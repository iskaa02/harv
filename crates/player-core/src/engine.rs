//! The playback controller: owns the queue and the mpv handle, resolves streams through `ytm-core`, and publishes [`Update`]s.
//! Front-ends call [`Engine::send`] (non-blocking, any thread) and await the update channel; position comes from the shared [`Clock`].
//!
//! Behaviour follows SimpMusic (`MediaServiceHandlerImpl`): endless radio refill, history trim, repeat/shuffle semantics; plus
//! "re-resolve the stream once on error, then skip" (SimpMusic's Android path just pauses).

use crate::audio::{Audio, AudioEvent, Clock};
use crate::fx::AudioFx;
use crate::queue::{Queue, QueueKind, QueueState, Removed, Repeat};
use anyhow::Result;
use std::f64::consts::FRAC_PI_2;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::{self, UnboundedSender, WeakUnboundedSender};
use std::collections::HashSet;
use ytm_core::{Quality, Segment, Stream, Track, Ytm};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status { Stopped, Loading, Playing, Paused }

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub queue: Vec<Track>,
    /// Index into `queue` of the current track.
    pub current: Option<usize>,
    pub status: Status,
    pub duration: f64,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub kind: QueueKind,
    pub volume: f64,
    pub endless: bool,
    /// Clicking a song in a loose list (recent, favourites, search…) starts its radio instead of queueing the whole list.
    pub auto_radio: bool,
    pub fx: AudioFx,
    /// 0 = cross-fade off.
    pub crossfade_ms: u64,
    pub sleep: Option<SleepInfo>,
    pub sponsor_categories: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SleepInfo {
    /// Unix ms when playback pauses; `None` for end-of-track.
    pub deadline_ms: Option<u64>,
    pub end_of_track: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepMode { After(Duration), EndOfTrack }

#[derive(Debug, Clone)]
pub enum Update {
    State(Snapshot),
    /// Human-readable, non-fatal (a track failed and was skipped, radio refill failed, …).
    Error(String),
}

#[derive(Debug)]
pub enum Command {
    PlayQueue { tracks: Vec<Track>, start: usize, kind: QueueKind },
    /// Load a saved queue paused at `position` seconds (app start).
    Restore { state: QueueState, position: f64 },
    Jump(usize),
    /// Warm a stream (a row was hovered) so the click that likely follows starts at once.
    Prefetch(String),
    Play,
    Pause,
    TogglePause,
    Next,
    Prev,
    Seek(f64),
    /// 0.0 – 1.0
    SetVolume(f64),
    SetShuffle(bool),
    SetRepeat(Repeat),
    SetEndless(bool),
    /// Only stored and echoed back so every toggle in the UI stays in sync; the front-end decides what a row click queues.
    SetAutoRadio(bool),
    SetQuality(Quality),
    Enqueue(Vec<Track>),
    PlayNext(Track),
    Remove(usize),
    Move { from: usize, to: usize },
    Stop,
    SetFx(AudioFx),
    /// Cross-fade length in ms; 0 = off, clamped to 15 s.
    SetCrossfade(u64),
    SetSleepTimer(Option<SleepMode>),
    /// SponsorBlock category names; empty = off.
    SetSponsorBlock(Vec<String>),
}

enum Msg {
    Cmd(Command),
    Audio(AudioEvent),
    Resolved { gen: u64, result: Result<Stream, String> },
    Radio { epoch: u64, then_advance: bool, result: Result<(Vec<Track>, Option<String>), String> },
    Segments { gen: u64, result: Result<Vec<Segment>, String> },
}

/// Where the next radio page comes from. `token = None` with `started` = the station ran out (re-seed from the queue's tail).
struct RadioSrc { seed: String, token: Option<String>, started: bool }

/// A refill is worth showing once it adds this many songs (a page is ~50, but dedupe against the queue can eat most of it).
const MIN_FRESH: usize = 10;
const MAX_PAGES: usize = 4;

/// Armed sleep timer. `At`: `at` drives the actor, `wall_ms` is what the UI shows.
enum SleepState { At { at: Instant, wall_ms: u64 }, EndOfTrack }

fn unix_ms() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64) }

#[derive(Clone)]
pub struct Engine { tx: UnboundedSender<Msg>, clock: Arc<Clock> }

impl Engine {
    /// Must be called from within (or given the handle of) a tokio runtime. The actor stops when every `Engine` clone is dropped.
    pub fn start(ytm: Arc<Ytm>, rt: &tokio::runtime::Handle) -> Result<(Engine, async_channel::Receiver<Update>)> {
        let (tx, rx) = mpsc::unbounded_channel();
        let weak = tx.downgrade();
        let audio = { let w = weak.clone(); Audio::new(move |e| { if let Some(tx) = w.upgrade() { let _ = tx.send(Msg::Audio(e)); } })? };
        let clock = audio.clock();
        let (out, updates) = async_channel::unbounded();
        let actor = Actor {
            ytm, audio, queue: Queue::default(), out, me: weak, status: Status::Stopped, duration: 0.0, volume: 1.0, quality: Quality::HighOpus, endless: true, auto_radio: false, radio: None, radio_epoch: 0,
            gen: 0, retried: false, failures: 0, start_at: None, start_paused: false, radio_inflight: false,
            fx: AudioFx::default(), crossfade_ms: 0, sleep: None, sleep_gain: 1.0, sponsor: vec![], segments: vec![], skipped: None, seg_gen: 0, pause_next: false, resolving: None,
        };
        rt.spawn(actor.run(rx));
        Ok((Engine { tx, clock }, updates))
    }

    pub fn send(&self, c: Command) { let _ = self.tx.send(Msg::Cmd(c)); }
    pub fn clock(&self) -> Arc<Clock> { self.clock.clone() }
}

struct Actor {
    ytm: Arc<Ytm>,
    audio: Audio,
    queue: Queue,
    out: async_channel::Sender<Update>,
    me: WeakUnboundedSender<Msg>,
    status: Status,
    duration: f64,
    volume: f64,
    quality: Quality,
    endless: bool,
    auto_radio: bool,
    /// The station feeding the queue: one seed and its continuation token, so refills stay on the same radio instead of drifting.
    radio: Option<RadioSrc>,
    /// Bumped whenever the queue is replaced; a radio page fetched for an older queue is dropped.
    radio_epoch: u64,
    /// Bumped on every load; stale async results carry an old value and are dropped.
    gen: u64,
    /// Already re-resolved the current track once after an error.
    retried: bool,
    /// Consecutive tracks that failed to play.
    failures: u32,
    start_at: Option<f64>,
    start_paused: bool,
    radio_inflight: bool,
    fx: AudioFx,
    crossfade_ms: u64,
    sleep: Option<SleepState>,
    /// Sleep-timer fade gain currently pushed to the audio (1.0 = none).
    sleep_gain: f64,
    sponsor: Vec<String>,
    segments: Vec<Segment>,
    /// Index into `segments` of the last one skipped, so a seek back into it isn't skipped again.
    skipped: Option<usize>,
    seg_gen: u64,
    /// Pause as soon as the next track loads (an end-of-track sleep fired).
    pause_next: bool,
    /// The in-flight stream resolve of the current load; aborted (killing its yt-dlp) when another load supersedes it.
    resolving: Option<tokio::task::AbortHandle>,
}

impl Actor {
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Msg>) {
        let _ = self.audio.set_volume(self.volume);
        let _ = self.audio.apply_fx(&self.fx);
        let mut ticker = tokio::time::interval(Duration::from_millis(100));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let playing = self.status == Status::Playing;
            tokio::select! {
                m = rx.recv() => match m {
                    Some(Msg::Cmd(c)) => self.command(c),
                    Some(Msg::Audio(e)) => self.audio_event(e),
                    Some(Msg::Resolved { gen, result }) if gen == self.gen => self.resolved(result),
                    Some(Msg::Radio { epoch, then_advance, result }) => self.radio(epoch, then_advance, result),
                    Some(Msg::Segments { gen, result }) => self.segments_ready(gen, result),
                    Some(Msg::Resolved { .. }) => {}
                    None => break,
                },
                _ = ticker.tick(), if playing => self.tick(),
            }
        }
    }

    fn emit(&self) {
        let sleep = self.sleep.as_ref().map(|s| match s {
            SleepState::At { wall_ms, .. } => SleepInfo { deadline_ms: Some(*wall_ms), end_of_track: false },
            SleepState::EndOfTrack => SleepInfo { deadline_ms: None, end_of_track: true },
        });
        let _ = self.out.try_send(Update::State(Snapshot {
            queue: self.queue.items().to_vec(), current: self.queue.current_index(), status: self.status, duration: self.duration,
            shuffle: self.queue.shuffle(), repeat: self.queue.repeat(), kind: self.queue.kind(), volume: self.volume, endless: self.endless, auto_radio: self.auto_radio,
            fx: self.fx.clone(), crossfade_ms: self.crossfade_ms, sleep, sponsor_categories: self.sponsor.clone(),
        }));
    }

    fn error(&self, s: String) { let _ = self.out.try_send(Update::Error(s)); }

    fn command(&mut self, c: Command) {
        match c {
            Command::PlayQueue { tracks, start, kind } => {
                // A radio queue is one station from its first song on; any other queue only starts one when it runs out.
                self.radio_epoch += 1;
                self.radio_inflight = false;
                self.radio = (kind == QueueKind::Radio).then(|| tracks.get(start).map(|t| RadioSrc { seed: t.video_id.clone(), token: None, started: false })).flatten();
                let loading_same = self.loading_id().is_some_and(|id| tracks.get(start).is_some_and(|t| t.video_id == id));
                self.queue.set(tracks, start, kind);
                // Clicking the song that is already loading just swaps the queue under it; a second resolve would only delay it.
                if loading_same { self.emit(); } else { self.fresh_load(); }
            }
            Command::Restore { state, position } => {
                self.radio_epoch += 1;
                self.radio_inflight = false;
                self.radio = None;
                self.queue.restore(state);
                self.start_at = (position > 1.0).then_some(position);
                self.start_paused = true;
                self.fresh_load_keep_start();
            }
            Command::Jump(i) => {
                let loading_same = self.loading_id().is_some_and(|id| self.queue.items().get(i).is_some_and(|t| t.video_id == id));
                if self.queue.jump(i) { if loading_same { self.emit(); } else { self.fresh_load(); } }
            }
            Command::Prefetch(id) => self.ytm.prefetch_stream(&id, self.quality),
            Command::Play => self.set_paused(false),
            Command::Pause => self.set_paused(true),
            Command::TogglePause => self.set_paused(match self.status { Status::Playing => true, Status::Loading => !self.start_paused, _ => false }),
            Command::Next => self.go_next(false),
            Command::Prev => {
                if self.audio_clock_pos() > 3.0 { let _ = self.audio.seek(0.0); } else { self.queue.back(); self.fresh_load(); }
            }
            // While the stream is still resolving there is nothing to seek in: remember it for the load.
            Command::Seek(s) => if self.status == Status::Loading { self.start_at = Some(s.max(0.0)); } else { let _ = self.audio.seek(s.max(0.0)); },
            Command::SetVolume(v) => { self.volume = v.clamp(0.0, 1.0); let _ = self.audio.set_volume(self.volume); self.emit(); }
            Command::SetShuffle(on) => { self.queue.set_shuffle(on); self.emit(); }
            Command::SetRepeat(r) => { self.queue.set_repeat(r); self.emit(); }
            Command::SetEndless(on) => { self.endless = on; self.emit(); }
            Command::SetAutoRadio(on) => { self.auto_radio = on; self.emit(); }
            Command::SetQuality(q) => self.quality = q,
            Command::Enqueue(tracks) => {
                let was_empty = self.queue.items().is_empty();
                self.queue.push(tracks);
                if was_empty { self.fresh_load(); } else { self.emit(); }
            }
            Command::PlayNext(t) => { let was_empty = self.queue.items().is_empty(); self.queue.play_next(t); if was_empty { self.fresh_load(); } else { self.emit(); } }
            Command::Remove(i) => match self.queue.remove(i) {
                Removed::Current => self.fresh_load(),
                _ => self.emit(),
            },
            Command::Move { from, to } => { self.queue.move_item(from, to); self.emit(); }
            Command::Stop => { if let Some(h) = self.resolving.take() { h.abort(); } let _ = self.audio.stop(); self.gen += 1; self.pause_next = false; self.status = Status::Stopped; self.duration = 0.0; self.emit(); }
            Command::SetFx(fx) => { self.fx = fx; if let Err(e) = self.audio.apply_fx(&self.fx) { self.error(format!("effects: {e}")); } self.emit(); }
            Command::SetCrossfade(ms) => { self.crossfade_ms = ms.min(15_000); self.emit(); }
            Command::SetSleepTimer(mode) => {
                self.sleep = mode.map(|m| match m {
                    SleepMode::After(d) => SleepState::At { at: Instant::now() + d, wall_ms: unix_ms() + d.as_millis() as u64 },
                    SleepMode::EndOfTrack => SleepState::EndOfTrack,
                });
                self.set_sleep_gain(1.0);
                self.emit();
            }
            Command::SetSponsorBlock(cats) => { self.sponsor = cats; self.fetch_segments(); self.emit(); }
        }
    }

    /// The video id being resolved right now, if the engine is loading a track (not retrying after an error).
    fn loading_id(&self) -> Option<String> {
        if self.status != Status::Loading || self.retried { return None; }
        self.queue.current().map(|t| t.video_id.clone())
    }

    fn audio_clock_pos(&self) -> f64 { self.audio.clock().position() }

    fn set_paused(&mut self, p: bool) {
        match self.status {
            Status::Playing | Status::Paused => { let _ = self.audio.set_pause(p); self.status = if p { Status::Paused } else { Status::Playing }; self.emit(); }
            Status::Stopped if !p && self.queue.current().is_some() => self.fresh_load(),
            // Loading: remember the intent so the track starts the way the user last asked.
            Status::Loading => { self.start_paused = p; if !p { self.pause_next = false; } }
            Status::Stopped => {}
        }
    }

    fn audio_event(&mut self, e: AudioEvent) {
        match e {
            AudioEvent::Loaded => { self.retried = false; self.failures = 0; }
            AudioEvent::Duration(d) => { self.duration = d; self.emit(); }
            AudioEvent::Paused(p) => {
                if matches!(self.status, Status::Playing | Status::Paused) {
                    let s = if p { Status::Paused } else { Status::Playing };
                    if s != self.status { self.status = s; self.emit(); }
                }
            }
            AudioEvent::Ended => if self.status == Status::Playing { self.go_next(true) },
            AudioEvent::Error(msg) => if self.status != Status::Stopped { self.track_failed(msg) },
        }
    }

    /// Start the current track from scratch (new track: no retry used, no saved position).
    fn fresh_load(&mut self) {
        self.retried = false;
        self.start_at = None;
        self.start_paused = false;
        self.load_current();
    }

    fn fresh_load_keep_start(&mut self) { self.retried = false; self.load_current(); }

    fn load_current(&mut self) {
        let Some(t) = self.queue.current().cloned() else {
            let _ = self.audio.stop();
            self.status = Status::Stopped;
            self.duration = 0.0;
            self.emit();
            return;
        };
        self.gen += 1;
        // Silence the previous track right away instead of letting it play until the new stream resolves; this also keeps its
        // late end-of-file / error events from skipping or retrying the track being loaded.
        if let Some(h) = self.resolving.take() { h.abort(); }
        if self.status == Status::Playing { self.audio.fade_out_stop(120); } else { let _ = self.audio.stop(); }
        self.status = Status::Loading;
        self.duration = t.duration_s.map_or(0.0, f64::from);
        self.emit();
        let (ytm, quality, gen, me) = (self.ytm.clone(), self.quality, self.gen, self.me.clone());
        self.resolving = Some(tokio::spawn(async move {
            let result = ytm.stream(&t.video_id, quality).await.map_err(|e| format!("{e:#}"));
            if let Some(tx) = me.upgrade() { let _ = tx.send(Msg::Resolved { gen, result }); }
        }).abort_handle());
        self.fetch_segments();
    }

    fn resolved(&mut self, result: Result<Stream, String>) {
        match result {
            Ok(s) => {
                let paused = std::mem::take(&mut self.start_paused) | std::mem::take(&mut self.pause_next);
                if let Err(e) = self.audio.load(&s.url, self.start_at.take(), paused) { return self.track_failed(e.to_string()); }
                self.status = if paused { Status::Paused } else { Status::Playing };
                self.emit();
                self.after_start();
            }
            Err(e) => self.track_failed(e),
        }
    }

    /// A track is now playing: warm the next stream and refill radio when the queue is nearly used up.
    fn after_start(&mut self) {
        for n in self.queue.peek_ahead(3) { self.ytm.prefetch_stream(&n.video_id, self.quality); }
        if self.queue.repeat() == Repeat::Off && self.queue.remaining() <= 1 { self.request_radio(false); }
    }

    /// Playback or resolution failed: re-resolve once (expired URL is the usual cause), then skip; give up after 3 in a row.
    fn track_failed(&mut self, msg: String) {
        let Some(t) = self.queue.current().cloned() else { return };
        self.ytm.invalidate_stream(&t.video_id);
        if !self.retried {
            self.retried = true;
            self.error(format!("{} — retrying…", t.title));
            self.start_at = (self.audio_clock_pos() > 1.0).then(|| self.audio_clock_pos());
            return self.load_current();
        }
        self.failures += 1;
        self.error(format!("{} — {}: {msg}", t.title, t.artist_line()));
        if self.failures >= 3 {
            self.pause_next = false;
            let _ = self.audio.stop();
            self.status = Status::Stopped;
            self.emit();
        } else {
            self.go_next(false);
        }
    }

    /// `auto` = the track ended by itself.
    fn go_next(&mut self, auto: bool) {
        // End-of-track sleep: the next track is loaded, but paused at its start.
        if auto && matches!(self.sleep, Some(SleepState::EndOfTrack)) { self.sleep = None; self.set_sleep_gain(1.0); self.pause_next = true; }
        if self.queue.advance(auto).is_some() {
            if self.queue.trim_history() > 0 { /* indices shifted; the emit in fresh_load carries the new list */ }
            self.fresh_load();
        } else if self.radio_wanted() && self.queue.current().is_some() {
            self.status = Status::Loading;
            self.emit();
            self.request_radio(true);
        } else {
            self.pause_next = false;
            let _ = self.audio.stop();
            self.status = Status::Stopped;
            self.emit();
        }
    }

    /// Sleep-timer fade gain; pushed to the audio only when it changes.
    fn set_sleep_gain(&mut self, g: f64) {
        if (g - self.sleep_gain).abs() > 1e-6 { self.sleep_gain = g; let _ = self.audio.set_sleep_gain(g); }
    }

    /// 100 ms while playing: sleep-timer fade and pause, SponsorBlock skips, cross-fade into the next track.
    fn tick(&mut self) {
        let (pos, dur) = (self.audio_clock_pos(), self.audio.clock().duration());
        if let Some(SleepState::At { at, .. }) = self.sleep {
            if at <= Instant::now() {
                let _ = self.audio.set_pause(true);
                self.status = Status::Paused;
                self.sleep = None;
                self.set_sleep_gain(1.0);
                self.emit();
                return;
            }
        }
        let left = match self.sleep {
            Some(SleepState::At { at, .. }) => Some(at.saturating_duration_since(Instant::now()).as_secs_f64()),
            Some(SleepState::EndOfTrack) if dur > 0.0 => Some(dur - pos),
            _ => None,
        };
        let gain = match left { Some(l) if l <= 5.0 => ((1.0 - l.max(0.0) / 5.0) * FRAC_PI_2).cos().clamp(0.0, 1.0), _ => 1.0 };
        self.set_sleep_gain(gain);

        // SponsorBlock: jump over a segment once; the 0.3 s margin keeps a seek landing inside it from re-triggering.
        if let Some(i) = self.segments.iter().position(|s| pos >= s.start && pos < s.end - 0.3) {
            if self.skipped != Some(i) { self.skipped = Some(i); let _ = self.audio.seek(self.segments[i].end + 0.05); }
        }

        let fade = self.crossfade_ms as f64 / 1000.0;
        let eot = matches!(self.sleep, Some(SleepState::EndOfTrack));
        if self.crossfade_ms > 0 && !eot && self.status == Status::Playing && !self.audio.is_crossfading() && self.queue.repeat() != Repeat::One
            && dur >= 20f64.max(3.0 * fade) && dur - pos <= fade + 0.3 { self.crossfade_next(); }
    }

    /// Start the next track on the idle deck while this one fades out. Needs the next stream already cached; otherwise the normal end-of-track path runs.
    fn crossfade_next(&mut self) {
        let Some(next) = self.queue.peek_next().cloned() else { return };
        let Some(stream) = self.ytm.cached_stream(&next.video_id, self.quality) else { return };
        self.queue.advance(true);
        self.queue.trim_history();
        self.gen += 1;
        self.retried = false;
        self.failures = 0;
        self.start_at = None;
        self.start_paused = false;
        if let Err(e) = self.audio.crossfade_to(&stream.url, self.crossfade_ms) { self.error(format!("crossfade: {e}")); return self.fresh_load(); }
        self.duration = next.duration_s.map_or(0.0, f64::from);
        self.status = Status::Playing;
        self.fetch_segments();
        self.emit();
        self.after_start();
    }

    /// SponsorBlock segments for the current track (no categories = none). Every call bumps `seg_gen` so an older fetch can't land.
    fn fetch_segments(&mut self) {
        self.seg_gen += 1;
        self.segments.clear();
        self.skipped = None;
        let Some(id) = self.queue.current().map(|t| t.video_id.clone()) else { return };
        if self.sponsor.is_empty() { return; }
        let (ytm, cats, gen, me) = (self.ytm.clone(), self.sponsor.clone(), self.seg_gen, self.me.clone());
        tokio::spawn(async move {
            let cats: Vec<&str> = cats.iter().map(String::as_str).collect();
            let result = ytm.skip_segments(&id, &cats).await.map_err(|e| format!("{e:#}"));
            if let Some(tx) = me.upgrade() { let _ = tx.send(Msg::Segments { gen, result }); }
        });
    }

    /// SponsorBlock is optional: a failed fetch just means no skips.
    fn segments_ready(&mut self, gen: u64, result: Result<Vec<Segment>, String>) {
        if gen == self.seg_gen { if let Ok(s) = result { self.segments = s; } }
    }

    /// A queue that was started as a radio always keeps going; any other queue only when "Endless radio" is on.
    fn radio_wanted(&self) -> bool { self.endless || self.queue.kind() == QueueKind::Radio }

    fn request_radio(&mut self, then_advance: bool) {
        if !self.radio_wanted() || self.radio_inflight { if then_advance && !self.radio_inflight { self.status = Status::Stopped; self.emit(); } return; }
        // No station yet (a playlist ran out) or the old one is exhausted: start one from the last song.
        if self.radio.as_ref().map_or(true, |r| r.started && r.token.is_none()) {
            let Some(seed) = self.queue.items().last().map(|t| t.video_id.clone()) else { return };
            self.radio = Some(RadioSrc { seed, token: None, started: false });
        }
        let Some(src) = self.radio.as_mut() else { return };
        src.started = true;
        let (seed, mut token) = (src.seed.clone(), src.token.take());
        let known: HashSet<String> = self.queue.items().iter().map(|t| t.video_id.clone()).collect();
        self.radio_inflight = true;
        let (ytm, epoch, me) = (self.ytm.clone(), self.radio_epoch, self.me.clone());
        tokio::spawn(async move {
            let result = async {
                let (mut fresh, mut seen) = (Vec::<Track>::new(), known);
                for _ in 0..MAX_PAGES {
                    let page = ytm.radio_page(&seed, token.as_deref()).await?;
                    token = page.continuation;
                    fresh.extend(page.tracks.into_iter().filter(|t| seen.insert(t.video_id.clone())));
                    if fresh.len() >= MIN_FRESH || token.is_none() { break; }
                }
                Ok::<_, anyhow::Error>((fresh, token))
            }.await.map_err(|e| format!("{e:#}"));
            if let Some(tx) = me.upgrade() { let _ = tx.send(Msg::Radio { epoch, then_advance, result }); }
        });
    }

    fn radio(&mut self, epoch: u64, then_advance: bool, result: Result<(Vec<Track>, Option<String>), String>) {
        if epoch != self.radio_epoch { return; }
        self.radio_inflight = false;
        let added = match result {
            Ok((tracks, token)) => {
                if let Some(src) = self.radio.as_mut() { src.token = token; }
                self.queue.push_new(tracks)
            }
            Err(e) => {
                // Keep the station so a retry resumes where it was; the token was taken for the request, so restart it from the seed.
                if let Some(src) = self.radio.as_mut() { src.started = false; }
                self.error(format!("radio: {e}"));
                0
            }
        };
        if then_advance {
            if added > 0 && self.queue.advance(true).is_some() { self.queue.trim_history(); self.fresh_load(); }
            else { self.pause_next = false; let _ = self.audio.stop(); self.status = Status::Stopped; self.emit(); }
        } else if added > 0 {
            self.emit();
            if let Some(n) = self.queue.peek_next() { self.ytm.prefetch_stream(&n.video_id, self.quality); }
        }
    }
}

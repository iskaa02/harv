//! Thin libmpv wrapper: two mpv handles ("decks") configured for audio only, so tracks can be cross-faded. Only the active deck drives the clock and events;
//! position is published through a lock-free [`Clock`] (read it every frame to drive lyrics); everything else is an [`AudioEvent`].

use crate::fx::AudioFx;
use anyhow::{anyhow, Result};
use libmpv2::events::{Event, PropertyData};
use libmpv2::{mpv_end_file_reason, Format, Mpv};
use std::f64::consts::FRAC_PI_2;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Playback position / duration in seconds, written by the mpv event thread, readable from anywhere.
#[derive(Default)]
pub struct Clock { pos: AtomicU64, dur: AtomicU64 }

impl Clock {
    pub fn position(&self) -> f64 { f64::from_bits(self.pos.load(Ordering::Relaxed)) }
    pub fn duration(&self) -> f64 { f64::from_bits(self.dur.load(Ordering::Relaxed)) }
    fn set_pos(&self, v: f64) { self.pos.store(v.to_bits(), Ordering::Relaxed) }
    fn set_dur(&self, v: f64) { self.dur.store(v.to_bits(), Ordering::Relaxed) }
}

#[derive(Debug, Clone)]
pub enum AudioEvent {
    Loaded,
    Duration(f64),
    Paused(bool),
    /// Reached the natural end of the file.
    Ended,
    Error(String),
}

type Emit = Arc<Mutex<Box<dyn Fn(AudioEvent) + Send>>>;

/// One mpv handle plus its event thread. `fade` is this deck's cross-fade gain (f64 bits, 0..1).
struct Deck { mpv: Arc<Mpv>, stop: Arc<AtomicBool>, thread: Option<JoinHandle<()>>, fade: Arc<AtomicU64> }

pub struct Audio {
    decks: [Deck; 2],
    /// Index (0 or 1) of the deck that owns the clock and forwards events.
    active: Arc<AtomicUsize>,
    clock: Arc<Clock>,
    /// f64 bits, 0..1: user volume.
    master: Arc<AtomicU64>,
    /// f64 bits, 0..1: sleep-timer fade gain.
    sleep: Arc<AtomicU64>,
    /// Bumped to abort a running cross-fade ramp.
    ramp_gen: Arc<AtomicU64>,
    crossfading: Arc<AtomicBool>,
}

/// libmpv refuses to start unless `LC_NUMERIC` is "C". GTK calls `setlocale(LC_ALL, "")`, so call this after GTK init and before [`Audio::new`].
pub fn fix_locale() {
    extern "C" { fn setlocale(category: i32, locale: *const std::ffi::c_char) -> *const std::ffi::c_char; }
    const LC_NUMERIC: i32 = 1;
    unsafe { setlocale(LC_NUMERIC, c"C".as_ptr()); }
}

fn f_get(a: &AtomicU64) -> f64 { f64::from_bits(a.load(Ordering::Relaxed)) }
fn f_set(a: &AtomicU64, v: f64) { a.store(v.to_bits(), Ordering::Relaxed) }

/// The one place that writes mpv's `volume`: master * sleep * fade, as 0..100.
fn push_volume(mpv: &Mpv, master: f64, sleep: f64, fade: f64) -> Result<()> {
    mpv.set_property("volume", (master * sleep * fade).clamp(0.0, 1.0) * 100.0).map_err(|e| anyhow!("{e}"))
}

fn new_mpv() -> Result<Mpv> {
    let mpv = Mpv::with_initializer(|i| {
        for (k, v) in [("video", "no"), ("vid", "no"), ("audio-display", "no"), ("force-window", "no"), ("ytdl", "no"), ("idle", "yes"), ("keep-open", "no"),
                       ("terminal", "no"), ("input-default-bindings", "no"), ("load-scripts", "no"), ("audio-client-name", "harv"), ("cache", "yes"), ("demuxer-max-bytes", "8MiB"), ("demuxer-max-back-bytes", "1MiB")] {
            i.set_option(k, v)?;
        }
        // mpv's bundled Lua scripts (stats, console, select, …) load even with `load-scripts=no`: one VM and thread each, per deck.
        // Older libmpv builds don't know every key, so these are best-effort.
        for k in ["load-stats-overlay", "load-osd-console", "load-console", "load-commands", "load-select", "load-context-menu", "load-positioning", "load-auto-profiles"] {
            let _ = i.set_option(k, "no");
        }
        // Escape hatch for headless runs (`HARV_AO=null`).
        if let Ok(ao) = std::env::var("HARV_AO") { i.set_option("ao", ao.as_str())?; }
        Ok(())
    }).map_err(|e| anyhow!("mpv init: {e}"))?;
    mpv.observe_property("time-pos", Format::Double, 1).map_err(|e| anyhow!("{e}"))?;
    mpv.observe_property("duration", Format::Double, 2).map_err(|e| anyhow!("{e}"))?;
    mpv.observe_property("pause", Format::Flag, 3).map_err(|e| anyhow!("{e}"))?;
    Ok(mpv)
}

/// Event loop for one deck. Only the active deck writes the clock or forwards events; the rest are dropped.
fn spawn_events(idx: usize, mpv: Arc<Mpv>, stop: Arc<AtomicBool>, active: Arc<AtomicUsize>, clock: Arc<Clock>, emit: Emit) -> Result<JoinHandle<()>> {
    Ok(std::thread::Builder::new().name(format!("mpv-events-{idx}")).spawn(move || {
        let on = |e: AudioEvent| if let Ok(f) = emit.lock() { (*f)(e) };
        let live = || active.load(Ordering::Relaxed) == idx;
        while !stop.load(Ordering::Relaxed) {
            match mpv.wait_event(0.2) {
                None => {}
                Some(Ok(Event::PropertyChange { name, change, .. })) => match (name, change) {
                    ("time-pos", PropertyData::Double(t)) => if live() { clock.set_pos(t) },
                    ("duration", PropertyData::Double(d)) => if live() { clock.set_dur(d); on(AudioEvent::Duration(d)) },
                    ("pause", PropertyData::Flag(p)) => if live() { on(AudioEvent::Paused(p)) },
                    _ => {}
                },
                Some(Ok(Event::FileLoaded)) => if live() { on(AudioEvent::Loaded) },
                Some(Ok(Event::EndFile(r))) if r == mpv_end_file_reason::Eof => if live() { on(AudioEvent::Ended) },
                Some(Ok(Event::Shutdown)) => break,
                Some(Ok(_)) => {}
                // libmpv2 reports a failed load / decode as an error from `wait_event` itself.
                Some(Err(e)) => if live() { on(AudioEvent::Error(format!("stream failed to load or play ({e})"))) },
            }
        }
    })?)
}

impl Deck {
    fn spawn(idx: usize, emit: Emit, clock: Arc<Clock>, active: Arc<AtomicUsize>) -> Result<Self> {
        let mpv = Arc::new(new_mpv()?);
        let stop = Arc::new(AtomicBool::new(false));
        let thread = spawn_events(idx, mpv.clone(), stop.clone(), active, clock, emit)?;
        Ok(Self { mpv, stop, thread: Some(thread), fade: Arc::new(AtomicU64::new(1f64.to_bits())) })
    }
}

impl Drop for Deck {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() { let _ = t.join(); }
    }
}

impl Audio {
    pub fn new(on_event: impl Fn(AudioEvent) + Send + 'static) -> Result<Self> {
        fix_locale();
        let emit: Emit = Arc::new(Mutex::new(Box::new(on_event) as Box<dyn Fn(AudioEvent) + Send>));
        let clock = Arc::new(Clock::default());
        let active = Arc::new(AtomicUsize::new(0));
        let d0 = Deck::spawn(0, emit.clone(), clock.clone(), active.clone())?;
        let d1 = Deck::spawn(1, emit, clock.clone(), active.clone())?;
        Ok(Self {
            decks: [d0, d1], active, clock,
            master: Arc::new(AtomicU64::new(1f64.to_bits())), sleep: Arc::new(AtomicU64::new(1f64.to_bits())),
            ramp_gen: Arc::new(AtomicU64::new(0)), crossfading: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn clock(&self) -> Arc<Clock> { self.clock.clone() }
    fn active_idx(&self) -> usize { self.active.load(Ordering::Relaxed) }

    /// Writes the volume of deck `i` from master, sleep gain and its own fade.
    fn apply_volume(&self, i: usize) -> Result<()> {
        push_volume(&self.decks[i].mpv, f_get(&self.master), f_get(&self.sleep), f_get(&self.decks[i].fade))
    }
    fn apply_all(&self) -> Result<()> { self.apply_volume(0).and(self.apply_volume(1)) }

    /// Replace whatever is playing. `start` seeks on load; `paused` starts the track paused.
    pub fn load(&self, url: &str, start: Option<f64>, paused: bool) -> Result<()> {
        self.cancel_crossfade();
        self.clock.set_pos(start.unwrap_or(0.0));
        self.clock.set_dur(0.0);
        self.set_pause(paused)?;
        let opts = start.map(|s| format!("start={s:.3}")).unwrap_or_default();
        self.decks[self.active_idx()].mpv.command("loadfile", &[url, "replace", "-1", &opts]).map_err(|e| anyhow!("loadfile: {e}"))
    }

    /// Fade out the current track on the active deck while `url` fades in on the other one over `fade_ms`.
    pub fn crossfade_to(&self, url: &str, fade_ms: u64) -> Result<()> {
        let old = self.active_idx();
        let new = 1 - old;
        let g = self.ramp_gen.fetch_add(1, Ordering::Relaxed) + 1;
        f_set(&self.decks[new].fade, 0.0);
        self.apply_all()?;
        self.decks[new].mpv.set_property("pause", false).map_err(|e| anyhow!("{e}"))?;
        self.decks[new].mpv.command("loadfile", &[url, "replace", "-1", ""]).map_err(|e| anyhow!("loadfile: {e}"))?;
        // Events and the clock follow the new deck from here on; reset after the switch so the outgoing deck can't overwrite the reset.
        self.active.store(new, Ordering::Relaxed);
        self.clock.set_pos(0.0);
        self.clock.set_dur(0.0);
        self.crossfading.store(true, Ordering::Relaxed);

        let (old_mpv, new_mpv) = (self.decks[old].mpv.clone(), self.decks[new].mpv.clone());
        let (fo, fnw) = (self.decks[old].fade.clone(), self.decks[new].fade.clone());
        let (master, sleep, ramp_gen, crossfading) = (self.master.clone(), self.sleep.clone(), self.ramp_gen.clone(), self.crossfading.clone());
        let step = (fade_ms / 50).max(20);
        std::thread::Builder::new().name("crossfade".into()).spawn(move || {
            let vol = |m: &Mpv, f: &AtomicU64| { let _ = push_volume(m, f_get(&master), f_get(&sleep), f_get(f)); };
            for i in 1..=50u32 {
                std::thread::sleep(Duration::from_millis(step));
                if ramp_gen.load(Ordering::Relaxed) != g { return; }
                let theta = i as f64 / 50.0 * FRAC_PI_2;
                f_set(&fo, theta.cos());
                f_set(&fnw, theta.sin());
                vol(&old_mpv, &fo);
                vol(&new_mpv, &fnw);
            }
            if ramp_gen.load(Ordering::Relaxed) != g { return; }
            let _ = old_mpv.command("stop", &[]);
            f_set(&fo, 1.0);
            vol(&old_mpv, &fo);
            crossfading.store(false, Ordering::Relaxed);
        }).map_err(|e| anyhow!("crossfade thread: {e}"))?;
        Ok(())
    }

    pub fn is_crossfading(&self) -> bool { self.crossfading.load(Ordering::Relaxed) }

    /// Abort a running cross-fade: the inactive deck is stopped, the active one keeps playing at full volume.
    pub fn cancel_crossfade(&self) {
        self.ramp_gen.fetch_add(1, Ordering::Relaxed);
        let _ = self.decks[1 - self.active_idx()].mpv.command("stop", &[]);
        f_set(&self.decks[0].fade, 1.0);
        f_set(&self.decks[1].fade, 1.0);
        self.crossfading.store(false, Ordering::Relaxed);
        let _ = self.apply_all();
    }

    /// Quick fade-out of the active deck, then stop it: a track switch ends the old song softly instead of cutting it.
    /// A `load` / `stop` / cross-fade that follows cancels the ramp (and the stop with it).
    pub fn fade_out_stop(&self, ms: u64) {
        self.cancel_crossfade();
        let g = self.ramp_gen.load(Ordering::Relaxed);
        let i = self.active_idx();
        let (mpv, fade, clock) = (self.decks[i].mpv.clone(), self.decks[i].fade.clone(), self.clock.clone());
        let (master, sleep, ramp_gen) = (self.master.clone(), self.sleep.clone(), self.ramp_gen.clone());
        clock.set_pos(0.0);
        clock.set_dur(0.0);
        let spawned = std::thread::Builder::new().name("fade-out".into()).spawn(move || {
            const STEPS: u64 = 6;
            for k in 1..=STEPS {
                std::thread::sleep(Duration::from_millis((ms / STEPS).max(1)));
                if ramp_gen.load(Ordering::Relaxed) != g { return; }
                f_set(&fade, 1.0 - k as f64 / STEPS as f64);
                let _ = push_volume(&mpv, f_get(&master), f_get(&sleep), f_get(&fade));
            }
            if ramp_gen.load(Ordering::Relaxed) != g { return; }
            let _ = mpv.command("stop", &[]);
            f_set(&fade, 1.0);
            let _ = push_volume(&mpv, f_get(&master), f_get(&sleep), f_get(&fade));
            clock.set_pos(0.0);
            clock.set_dur(0.0);
        });
        if spawned.is_err() { let _ = self.stop(); }
    }

    pub fn stop(&self) -> Result<()> {
        self.cancel_crossfade();
        self.clock.set_pos(0.0);
        self.clock.set_dur(0.0);
        self.decks[self.active_idx()].mpv.command("stop", &[]).map_err(|e| anyhow!("{e}"))
    }

    /// Applied to both decks so a cross-fade inherits the state; an error on the idle deck is ignored.
    pub fn set_pause(&self, p: bool) -> Result<()> {
        let a = self.active_idx();
        let _ = self.decks[1 - a].mpv.set_property("pause", p);
        self.decks[a].mpv.set_property("pause", p).map_err(|e| anyhow!("{e}"))
    }

    pub fn seek(&self, secs: f64) -> Result<()> { self.decks[self.active_idx()].mpv.command("seek", &[&format!("{secs:.3}"), "absolute"]).map_err(|e| anyhow!("{e}")) }

    /// 0.0 – 1.0
    pub fn set_volume(&self, v: f64) -> Result<()> {
        f_set(&self.master, v.clamp(0.0, 1.0));
        self.apply_all()
    }

    /// Sleep-timer fade gain, 0.0 – 1.0 (multiplies the user volume).
    pub fn set_sleep_gain(&self, g: f64) -> Result<()> {
        f_set(&self.sleep, g.clamp(0.0, 1.0));
        self.apply_all()
    }

    /// Sets the mpv `af` chain and `speed` on both decks. An `af` rejection is reported; the other deck and speed are still set.
    pub fn apply_fx(&self, fx: &AudioFx) -> Result<()> {
        let chain = fx.chain();
        let mut err: Option<anyhow::Error> = None;
        for d in &self.decks {
            if let Err(e) = d.mpv.set_property("af", chain.as_str()) { err.get_or_insert(anyhow!("audio filter rejected: {e}")); }
            if let Err(e) = d.mpv.set_property("speed", fx.speed) { err.get_or_insert(anyhow!("speed: {e}")); }
        }
        err.map_or(Ok(()), Err)
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        // Abort any running ramp; the decks' own `Drop` stops and joins their event threads.
        self.ramp_gen.fetch_add(1, Ordering::Relaxed);
    }
}

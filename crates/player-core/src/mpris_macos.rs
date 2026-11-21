//! macOS counterpart of the MPRIS service: media keys (F7–F9, AirPods, Touch Bar) via `MPRemoteCommandCenter`, and the
//! Control Centre "Now Playing" tile via `MPNowPlayingInfoCenter` (both through `souvlaki`). Same API as `mpris.rs`:
//! the front-end forwards each [`Snapshot`] with [`Mpris::update`]. Create it on the main thread — command handlers
//! are delivered on the main dispatch queue, which GTK's macOS backend drains from its main loop.

use crate::audio::Clock;
use crate::engine::{Command, Engine, Snapshot, Status};
use anyhow::{anyhow, Result};
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Seek step for the plain "seek forward/backward" command (no amount given).
const SEEK_STEP_S: f64 = 10.0;

pub struct Mpris { inner: Mutex<Inner>, clock: Arc<Clock> }

struct Inner {
    controls: MediaControls,
    /// (video id, duration) last published as metadata: the artwork is re-fetched on every `set_metadata`, so only
    /// republish when the track changes or its real duration arrives.
    meta_key: Option<(String, u64)>,
    shuffle: Option<bool>,
}

impl Mpris {
    pub async fn start(engine: Engine) -> Result<Mpris> {
        let clock = engine.clock();
        let mut controls = MediaControls::new(PlatformConfig { dbus_name: "harv", display_name: "Harv", hwnd: None })
            .map_err(|e| anyhow!("media controls: {e:?}"))?;
        let c = clock.clone();
        controls.attach(move |ev| {
            let cmd = match ev {
                MediaControlEvent::Play => Command::Play,
                MediaControlEvent::Pause => Command::Pause,
                MediaControlEvent::Toggle => Command::TogglePause,
                MediaControlEvent::Next => Command::Next,
                MediaControlEvent::Previous => Command::Prev,
                MediaControlEvent::Stop => Command::Stop,
                MediaControlEvent::Seek(d) => Command::Seek((c.position() + sign(d) * SEEK_STEP_S).max(0.0)),
                MediaControlEvent::SeekBy(d, by) => Command::Seek((c.position() + sign(d) * by.as_secs_f64()).max(0.0)),
                MediaControlEvent::SetPosition(MediaPosition(p)) => Command::Seek(p.as_secs_f64()),
                MediaControlEvent::SetVolume(v) => Command::SetVolume(v),
                MediaControlEvent::SetShuffle(on) => Command::SetShuffle(on),
                _ => return,
            };
            engine.send(cmd);
        }).map_err(|e| anyhow!("media controls: {e:?}"))?;
        Ok(Mpris { inner: Mutex::new(Inner { controls, meta_key: None, shuffle: None }), clock })
    }

    pub fn update(&self, s: &Snapshot) {
        let Ok(mut g) = self.inner.lock() else { return };
        let track = s.current.and_then(|i| s.queue.get(i));
        let key = track.map(|t| (t.video_id.clone(), s.duration.round() as u64));
        if key != g.meta_key {
            let md = match track {
                Some(t) => {
                    let artist = t.artist_line();
                    let dur = if s.duration > 0.0 { Some(s.duration) } else { t.duration_s.map(f64::from) };
                    g.controls.set_metadata(MediaMetadata {
                        title: Some(&t.title),
                        artist: (!artist.is_empty()).then_some(artist.as_str()),
                        album: t.album.as_deref(),
                        cover_url: t.thumb.as_deref(),
                        duration: dur.map(Duration::from_secs_f64),
                    })
                }
                None => g.controls.set_metadata(MediaMetadata::default()),
            };
            if md.is_ok() { g.meta_key = key; }
        }
        let progress = Some(MediaPosition(Duration::from_secs_f64(self.clock.position().max(0.0))));
        let playback = match (track, s.status) {
            (None, _) => MediaPlayback::Stopped,
            (_, Status::Playing | Status::Loading) => MediaPlayback::Playing { progress },
            (_, Status::Paused) => MediaPlayback::Paused { progress },
            _ => MediaPlayback::Stopped,
        };
        let _ = g.controls.set_playback(playback);
        let shuffle = track.map(|_| s.shuffle);
        if shuffle != g.shuffle { let _ = g.controls.set_shuffle(shuffle); g.shuffle = shuffle; }
    }
}

fn sign(d: SeekDirection) -> f64 { match d { SeekDirection::Forward => 1.0, SeekDirection::Backward => -1.0 } }

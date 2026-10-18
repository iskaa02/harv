//! MPRIS2 (`org.mpris.MediaPlayer2.harv`): media keys, GNOME/KDE media widgets, `playerctl`.
//! The front-end owns the update channel, so it forwards each [`Snapshot`] with [`Mpris::update`].

use crate::audio::Clock;
use crate::engine::{Command, Engine, Snapshot, Status};
use crate::queue::Repeat;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{interface, Connection};

const PATH: &str = "/org/mpris/MediaPlayer2";

struct Root;

#[interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {}
    fn quit(&self) {}
    #[zbus(property)] fn can_quit(&self) -> bool { false }
    #[zbus(property)] fn can_raise(&self) -> bool { false }
    #[zbus(property)] fn has_track_list(&self) -> bool { false }
    #[zbus(property)] fn identity(&self) -> &str { "Harv" }
    #[zbus(property)] fn supported_uri_schemes(&self) -> Vec<String> { vec![] }
    #[zbus(property)] fn supported_mime_types(&self) -> Vec<String> { vec![] }
}

struct Player { engine: Engine, clock: Arc<Clock>, snap: Option<Snapshot> }

impl Player {
    fn current(&self) -> Option<&ytm_core::Track> { self.snap.as_ref().and_then(|s| s.current.map(|i| &s.queue[i])) }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn next(&self) { self.engine.send(Command::Next) }
    fn previous(&self) { self.engine.send(Command::Prev) }
    fn pause(&self) { self.engine.send(Command::Pause) }
    fn play(&self) { self.engine.send(Command::Play) }
    fn play_pause(&self) { self.engine.send(Command::TogglePause) }
    fn stop(&self) { self.engine.send(Command::Stop) }
    fn seek(&self, offset_us: i64) { self.engine.send(Command::Seek(self.clock.position() + offset_us as f64 / 1e6)) }
    fn set_position(&self, _track: ObjectPath<'_>, pos_us: i64) { self.engine.send(Command::Seek(pos_us as f64 / 1e6)) }
    fn open_uri(&self, _uri: &str) {}

    #[zbus(property)]
    fn playback_status(&self) -> &str {
        match self.snap.as_ref().map(|s| s.status) { Some(Status::Playing | Status::Loading) => "Playing", Some(Status::Paused) => "Paused", _ => "Stopped" }
    }
    #[zbus(property)]
    fn loop_status(&self) -> &str { match self.snap.as_ref().map(|s| s.repeat) { Some(Repeat::All) => "Playlist", Some(Repeat::One) => "Track", _ => "None" } }
    #[zbus(property)]
    fn set_loop_status(&self, v: &str) { self.engine.send(Command::SetRepeat(match v { "Playlist" => Repeat::All, "Track" => Repeat::One, _ => Repeat::Off })) }
    #[zbus(property)] fn rate(&self) -> f64 { 1.0 }
    #[zbus(property)] fn set_rate(&self, _v: f64) {}
    #[zbus(property)] fn minimum_rate(&self) -> f64 { 1.0 }
    #[zbus(property)] fn maximum_rate(&self) -> f64 { 1.0 }
    #[zbus(property)] fn shuffle(&self) -> bool { self.snap.as_ref().is_some_and(|s| s.shuffle) }
    #[zbus(property)] fn set_shuffle(&self, on: bool) { self.engine.send(Command::SetShuffle(on)) }
    #[zbus(property)] fn volume(&self) -> f64 { self.snap.as_ref().map_or(1.0, |s| s.volume) }
    #[zbus(property)] fn set_volume(&self, v: f64) { self.engine.send(Command::SetVolume(v)) }
    #[zbus(property(emits_changed_signal = "false"))] fn position(&self) -> i64 { (self.clock.position() * 1e6) as i64 }
    #[zbus(property)] fn can_go_next(&self) -> bool { self.snap.as_ref().is_some_and(|s| s.current.is_some()) }
    #[zbus(property)] fn can_go_previous(&self) -> bool { self.snap.as_ref().is_some_and(|s| s.current.is_some()) }
    #[zbus(property)] fn can_play(&self) -> bool { self.snap.as_ref().is_some_and(|s| s.current.is_some()) }
    #[zbus(property)] fn can_pause(&self) -> bool { self.snap.as_ref().is_some_and(|s| s.current.is_some()) }
    #[zbus(property)] fn can_seek(&self) -> bool { self.snap.as_ref().is_some_and(|s| s.current.is_some()) }
    #[zbus(property)] fn can_control(&self) -> bool { true }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let mut m: HashMap<String, OwnedValue> = HashMap::new();
        let put = |m: &mut HashMap<String, OwnedValue>, k: &str, v: Value<'_>| { if let Ok(v) = v.try_to_owned() { m.insert(k.into(), v); } };
        let Some(t) = self.current() else {
            put(&mut m, "mpris:trackid", Value::from(ObjectPath::from_static_str_unchecked("/org/mpris/MediaPlayer2/TrackList/NoTrack")));
            return m;
        };
        let id = format!("/org/harv/track/{}", t.video_id.replace('-', "_"));
        if let Ok(p) = ObjectPath::try_from(id) { put(&mut m, "mpris:trackid", Value::from(p)); }
        put(&mut m, "xesam:title", Value::from(t.title.clone()));
        put(&mut m, "xesam:artist", Value::from(t.artists.clone()));
        if let Some(a) = &t.album { put(&mut m, "xesam:album", Value::from(a.clone())); }
        let dur = self.snap.as_ref().map_or(0.0, |s| s.duration);
        if dur > 0.0 { put(&mut m, "mpris:length", Value::from((dur * 1e6) as i64)); }
        if let Some(u) = &t.thumb { put(&mut m, "mpris:artUrl", Value::from(u.clone())); }
        m
    }
}

pub struct Mpris { tx: mpsc::UnboundedSender<Snapshot> }

impl Mpris {
    /// Claims `org.mpris.MediaPlayer2.harv` on the session bus. Needs a tokio runtime.
    pub async fn start(engine: Engine) -> Result<Mpris> {
        let clock = engine.clock();
        let conn = zbus::connection::Builder::session()?
            .name("org.mpris.MediaPlayer2.harv")?
            .serve_at(PATH, Root)?
            .serve_at(PATH, Player { engine, clock, snap: None })?
            .build().await?;
        let (tx, mut rx) = mpsc::unbounded_channel::<Snapshot>();
        tokio::spawn(async move {
            let conn: Connection = conn;
            let Ok(iface) = conn.object_server().interface::<_, Player>(PATH).await else { return };
            while let Some(mut s) = rx.recv().await {
                while let Ok(newer) = rx.try_recv() { s = newer; }
                let (old_track, old_status) = { let g = iface.get().await; (g.current().map(|t| t.video_id.clone()), g.snap.as_ref().map(|s| s.status)) };
                let new_track = s.current.map(|i| s.queue[i].video_id.clone());
                let new_status = s.status;
                iface.get_mut().await.snap = Some(s);
                let g = iface.get().await;
                let e: &SignalEmitter<'_> = iface.signal_emitter();
                let _ = g.playback_status_changed(e).await;
                let _ = g.loop_status_changed(e).await;
                let _ = g.shuffle_changed(e).await;
                let _ = g.volume_changed(e).await;
                let _ = g.can_go_next_changed(e).await;
                let _ = g.can_go_previous_changed(e).await;
                if old_track != new_track || old_status != Some(new_status) { let _ = g.metadata_changed(e).await; }
            }
        });
        Ok(Mpris { tx })
    }

    pub fn update(&self, s: &Snapshot) { let _ = self.tx.send(s.clone()); }
}

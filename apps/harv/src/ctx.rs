//! Shared application state (single-threaded, lives on the GTK main loop) and the glue to the tokio side.

use crate::images::Images;
use adw::prelude::*;
use gtk::glib;
use lyrics_net::LyricsService;
use player_core::{Command, Engine, QueueKind, QueueState, Snapshot, Status};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;
use store_core::Store;
use ytm_core::{Quality, Track, Ytm};

/// Where in-app navigation can go.
#[derive(Clone, Debug)]
pub enum Nav {
    Album(String),
    Artist(String),
    Playlist(String),
    Podcast(String),
    Local(i64),
    NowPlaying,
    CloseNowPlaying,
    Queue,
    Settings,
    SearchFor(String),
    Effects,
    /// (category params, title)
    Mood(String, String),
    MiniPlayer,
    /// A top-level page, as if picked in the sidebar.
    Root(crate::window::Root),
}

/// Widgets that mirror global state (liked heart, "playing" highlight) register here; dead ones are pruned.
struct RowRefs { video_id: String, title: glib::WeakRef<gtk::Label>, heart: glib::WeakRef<gtk::Button> }

pub struct Ctx {
    pub rt: tokio::runtime::Handle,
    pub ytm: Arc<Ytm>,
    pub store: Store,
    pub engine: Engine,
    pub lyrics: LyricsService,
    pub images: Images,
    pub snap: RefCell<Option<Snapshot>>,
    hooks: RefCell<Vec<Box<dyn Fn(&Snapshot) -> bool>>>,
    rows: RefCell<Vec<RowRefs>>,
    pub toasts: adw::ToastOverlay,
    /// `@define-color lib_accent …`, loaded after the static stylesheet.
    pub accent: gtk::CssProvider,
    last_cover: Cell<Option<[f32; 3]>>,
    nav: RefCell<Option<Rc<dyn Fn(Nav)>>>,
    /// Async jobs in flight (the screenshot mode waits for this to drain).
    pub pending: Cell<u32>,
    /// The active theme (tokens, vocabulary, metrics). Replaced live by [`Ctx::set_theme`].
    theme: RefCell<Rc<theme_core::Theme>>,
    theme_hooks: RefCell<Vec<Rc<dyn Fn(&theme_core::Theme)>>>,
    /// The accent colour last applied by [`Ctx::apply_accent`], and who wants to know when it changes.
    accent_rgb: Cell<[f32; 3]>,
    accent_hooks: RefCell<Vec<Rc<dyn Fn()>>>,
    /// The toast with an action button now on screen; replaced (not queued behind) by the next one.
    action_toast: RefCell<Option<adw::Toast>>,
    /// Bumped by [`Ctx::relayout`]: pages that fetch data (Home) rebuild only when a layout preference changed, not on every theme switch.
    pub layout_gen: Cell<u32>,
}

use std::sync::Arc;

impl Ctx {
    pub fn new(rt: tokio::runtime::Handle, ytm: Arc<Ytm>, store: Store, engine: Engine, lyrics: LyricsService, theme: Rc<theme_core::Theme>) -> Rc<Self> {
        let images = Images::new(rt.clone());
        Rc::new(Self { rt, ytm, store, engine, lyrics, images, snap: RefCell::new(None), hooks: RefCell::default(), rows: RefCell::default(), toasts: adw::ToastOverlay::new(), accent: gtk::CssProvider::new(), last_cover: Cell::new(None), nav: RefCell::default(), pending: Cell::new(0), theme: RefCell::new(theme), theme_hooks: RefCell::default(), accent_rgb: Cell::new([1.0, 0.22, 0.37]), accent_hooks: RefCell::default(), action_toast: RefCell::default(), layout_gen: Cell::new(0) })
    }

    // ── async glue ───────────────────────────────────────────────────────────────────────────
    /// Run `fut` on the tokio runtime, then call `then` with its result on the main loop.
    pub fn spawn<T: Send + 'static>(self: &Rc<Self>, fut: impl Future<Output = T> + Send + 'static, then: impl FnOnce(T) + 'static) {
        let (tx, rx) = async_channel::bounded(1);
        self.rt.spawn(async move { let _ = tx.send(fut.await).await; });
        self.pending.set(self.pending.get() + 1);
        let ctx = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(v) = rx.recv().await { then(v); }
            ctx.pending.set(ctx.pending.get().saturating_sub(1));
        });
    }

    /// Fixed accent choices for settings; index 0 = follow the artwork.
    pub const ACCENTS: [(&'static str, [f32; 3]); 6] = [
        ("From artwork", [1.0, 0.22, 0.37]), ("Rose", [1.0, 0.22, 0.37]), ("Blue", [0.25, 0.55, 1.0]),
        ("Green", [0.20, 0.78, 0.45]), ("Violet", [0.62, 0.42, 1.0]), ("Amber", [1.0, 0.70, 0.18]),
    ];

    // ── theme ────────────────────────────────────────────────────────────────────────────────
    pub fn theme(&self) -> Rc<theme_core::Theme> { self.theme.borrow().clone() }

    /// Vocabulary lookup in the active theme ("Playlists" / "Channels" / …).
    pub fn word(&self, key: &str) -> String { self.theme.borrow().word(key).to_string() }

    /// Swap the active theme: recompute the accent, then let widgets that cache theme values (sidebar labels/width) refresh.
    pub fn set_theme(&self, t: Rc<theme_core::Theme>) {
        *self.theme.borrow_mut() = t.clone();
        self.apply_accent(None);
        let hooks: Vec<_> = self.theme_hooks.borrow().clone();
        for h in hooks { h(&t); }
    }

    /// Re-run the theme hooks with the unchanged theme: sidebar, bar and the page on top rebuild (layout preferences changed).
    pub fn relayout(&self) { self.layout_gen.set(self.layout_gen.get() + 1); let t = self.theme(); self.set_theme(t); }

    /// Run `f` now and on every theme change.
    pub fn on_theme(&self, f: impl Fn(&theme_core::Theme) + 'static) {
        let f: Rc<dyn Fn(&theme_core::Theme)> = Rc::new(f);
        f(&self.theme());
        self.theme_hooks.borrow_mut().push(f);
    }

    /// Recompute the accent colour. Precedence: the user's explicit choice, then the theme's fixed accent, then the
    /// cover's dominant colour (lifted until readable on a dark UI).
    pub fn apply_accent(&self, cover: Option<[f32; 3]>) {
        if cover.is_some() { self.last_cover.set(cover); }
        let theme = self.theme();
        let mode: usize = self.store.get("accent").unwrap_or(0);
        let fixed = match theme.colors.accent { theme_core::Accent::Fixed(c) => Some(c.rgb_f32()), theme_core::Accent::Cover => None };
        let mut c = if mode != 0 { Self::ACCENTS[mode.min(5)].1 } else if let Some(f) = fixed { f } else { self.last_cover.get().unwrap_or(Self::ACCENTS[1].1) };
        // Keep it bright enough to read on the dark UI (never touch a theme's own fixed accent, nor on light themes).
        let lum = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        if !theme.light() && !(mode == 0 && fixed.is_some()) && lum < 0.45 { let k = (0.45 - lum) * 1.4; c = [c[0] + k, c[1] + k, c[2] + k].map(|v| v.min(1.0)); }
        self.accent_rgb.set(c);
        let b = |v: f32| (v * 255.0).round() as u8;
        let rgb = format!("rgb({}, {}, {})", b(c[0]), b(c[1]), b(c[2]));
        // libadwaita widgets (switches, suggested buttons, selection) read its own accent names too.
        self.accent.load_from_string(&format!("@define-color lib_accent {rgb}; @define-color accent_bg_color {rgb}; @define-color accent_color {rgb}; @define-color accent_standalone_color {rgb}; @define-color accent_fg_color {};", theme.colors.accent_on.css()));
        let hooks: Vec<_> = self.accent_hooks.borrow().clone();
        for h in hooks { h(); }
    }

    /// The accent in effect (user's choice, theme's fixed colour or the cover's), 0..1 RGB.
    pub fn accent_rgb(&self) -> [f32; 3] { self.accent_rgb.get() }

    /// Run `f` after every accent change (it lives as long as the app).
    pub fn on_accent(&self, f: impl Fn() + 'static) { self.accent_hooks.borrow_mut().push(Rc::new(f)); }

    pub fn toast(&self, msg: &str) { self.toasts.add_toast(adw::Toast::new(msg)); }

    /// A toast with one action button ("Undo", "Add to playlist…"). It replaces the previous action toast instead of queueing behind it.
    pub fn toast_action(&self, msg: &str, label: &str, f: impl Fn() + 'static) {
        if let Some(old) = self.action_toast.borrow_mut().take() { old.dismiss(); }
        let t = adw::Toast::new(msg);
        t.set_button_label(Some(label));
        t.connect_button_clicked(move |_| f());
        *self.action_toast.borrow_mut() = Some(t.clone());
        self.toasts.add_toast(t);
    }

    // ── navigation ───────────────────────────────────────────────────────────────────────────
    pub fn set_nav(&self, f: impl Fn(Nav) + 'static) { *self.nav.borrow_mut() = Some(Rc::new(f)); }
    pub fn navigate(&self, n: Nav) { let f = self.nav.borrow().clone(); if let Some(f) = f { f(n); } }

    // ── playback helpers ─────────────────────────────────────────────────────────────────────
    pub fn play(&self, tracks: Vec<Track>, start: usize, kind: QueueKind) {
        if tracks.is_empty() { return; }
        self.engine.send(Command::PlayQueue { tracks, start, kind });
    }

    /// A row/card click in a list. With "radio on click" on and a loose list (`QueueKind::Songs`), only that song is queued and its
    /// radio takes over; otherwise the list plays on from the clicked song.
    pub fn play_row(&self, tracks: Vec<Track>, i: usize, kind: QueueKind) {
        let radio = kind == QueueKind::Songs && self.snap.borrow().as_ref().map_or_else(|| self.store.get::<bool>("auto_radio").unwrap_or(false), |s| s.auto_radio);
        match tracks.get(i) {
            Some(t) if radio => self.play_one(t.clone()),
            _ => self.play(tracks, i, kind),
        }
    }

    /// "Radio on click": persist it and tell the engine (which echoes it to every toggle). The toast says what a click will do now.
    pub fn set_auto_radio(&self, on: bool) {
        let _ = self.store.set("auto_radio", &on);
        self.engine.send(Command::SetAutoRadio(on));
        self.toast(if on { "Radio on: playing a song from a list starts its radio" } else { "Radio off: playing a song from a list plays the list on" });
    }

    pub fn play_shuffled(&self, tracks: Vec<Track>, kind: QueueKind) {
        if tracks.is_empty() { return; }
        self.engine.send(Command::SetShuffle(true));
        let start = fastrand_index(tracks.len());
        self.engine.send(Command::PlayQueue { tracks, start, kind });
    }

    /// Play one track; with "endless" on, the engine keeps filling the queue from its radio.
    pub fn play_one(&self, t: Track) { self.engine.send(Command::PlayQueue { tracks: vec![t], start: 0, kind: QueueKind::Radio }); }

    pub fn current_track(&self) -> Option<Track> {
        let s = self.snap.borrow();
        let s = s.as_ref()?;
        s.current.and_then(|i| s.queue.get(i).cloned())
    }

    pub fn quality(&self) -> Quality { Quality::parse(&self.store.get::<String>("quality").unwrap_or_else(|| "high-opus".into())).unwrap_or(Quality::HighOpus) }

    // ── state fan-out ────────────────────────────────────────────────────────────────────────
    pub fn on_state(&self, f: impl Fn(&Snapshot) + 'static) { self.on_state_while(move |s| { f(s); true }); }

    /// Like [`Ctx::on_state`], but the hook is dropped once it returns `false` (for dialogs that can close).
    pub fn on_state_while(&self, f: impl Fn(&Snapshot) -> bool + 'static) {
        let cur = self.snap.borrow().clone();
        if let Some(s) = cur { if !f(&s) { return; } }
        self.hooks.borrow_mut().push(Box::new(f));
    }

    pub(crate) fn dispatch(&self, s: Snapshot) {
        *self.snap.borrow_mut() = Some(s.clone());
        self.run_hooks(&s);
        self.refresh_rows();
    }

    fn run_hooks(&self, s: &Snapshot) {
        // Hooks may register more hooks while running, so don't hold the borrow across the calls.
        let hooks = std::mem::take(&mut *self.hooks.borrow_mut());
        let mut kept: Vec<_> = hooks.into_iter().filter(|h| h(s)).collect();
        let mut cell = self.hooks.borrow_mut();
        kept.append(&mut cell);
        *cell = kept;
    }

    // ── per-row mirrors (heart / playing highlight) ──────────────────────────────────────────
    pub fn register_row(&self, video_id: &str, title: &gtk::Label, heart: &gtk::Button) {
        let mut rows = self.rows.borrow_mut();
        rows.retain(|r| r.title.upgrade().is_some());
        rows.push(RowRefs { video_id: video_id.to_string(), title: title.downgrade(), heart: heart.downgrade() });
        drop(rows);
        let playing = self.current_track().map(|t| t.video_id);
        set_row_state(title, heart, playing.as_deref() == Some(video_id), self.store.is_liked(video_id));
    }

    pub fn refresh_rows(&self) {
        let playing = self.current_track().map(|t| t.video_id);
        let liked = self.store.liked_ids();
        for r in self.rows.borrow().iter() {
            if let (Some(t), Some(h)) = (r.title.upgrade(), r.heart.upgrade()) { set_row_state(&t, &h, playing.as_deref() == Some(r.video_id.as_str()), liked.contains(&r.video_id)); }
        }
    }

    pub fn toggle_like(self: &Rc<Self>, t: &Track) {
        let now = !self.store.is_liked(&t.video_id);
        if !self.set_liked(t, now) { return; }
        let (me, t2) = (self.clone(), t.clone());
        if now { self.toast_action("Added to liked songs", "Add to playlist…", move || crate::rows::add_to_playlist(&me, vec![t2.clone()])); }
        else { self.toast_action("Removed from liked songs", "Undo", move || { me.set_liked(&t2, true); }); }
    }

    /// Store the like and refresh every heart. False (after a toast) when the store refused.
    pub fn set_liked(&self, t: &Track, liked: bool) -> bool {
        if let Err(e) = self.store.set_liked(t, liked) { self.toast(&format!("Couldn't save: {e}")); return false; }
        self.refresh_rows();
        self.hooks_like_changed();
        true
    }

    fn hooks_like_changed(&self) { let cur = self.snap.borrow().clone(); if let Some(s) = cur { self.run_hooks(&s); } }

    pub fn is_playing(&self) -> bool { self.snap.borrow().as_ref().is_some_and(|s| s.status == Status::Playing) }

    /// Persist what a fresh start needs: the queue (as a [`QueueState`]) and the position.
    pub fn save_queue(&self, position: f64) {
        let s = self.snap.borrow();
        let Some(s) = s.as_ref() else { return };
        let st = QueueState { items: s.queue.clone(), current: s.current, shuffle: s.shuffle, repeat: s.repeat, kind: s.kind };
        let _ = self.store.set("queue", &(st, position));
    }
}

fn set_row_state(title: &gtk::Label, heart: &gtk::Button, playing: bool, liked: bool) {
    if playing { title.add_css_class("playing"); } else { title.remove_css_class("playing"); }
    heart.set_icon_name(crate::icons::heart(liked));
    if liked { heart.add_css_class("liked"); } else { heart.remove_css_class("liked"); }
}

fn fastrand_index(n: usize) -> usize {
    // std-only: nanosecond clock is plenty for "pick a random start".
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() as usize % n).unwrap_or(0)
}

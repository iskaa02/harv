//! The themed bottom bar (`shell.bottom_bar.style = "parts"`) and its parts: `cover`, `title`, `transport`, `seek`,
//! `volume`, `link`. The same parts work inside `[page.*]` trees (any region).
//!
//! Every part that shows playback (seek, time labels) subscribes to one shared ticker (`HUB`): a single
//! 250 ms timeout that exists only while the engine is playing and only while a subscribed part is mapped.

use super::{build_all, Env};
use crate::ctx::{Ctx, Nav};
use crate::rows::{art, fmt_time, set_art};
use adw::prelude::*;
use gtk::{glib, pango};
use player_core::{Command, Repeat, Snapshot, Status};
use seek_widgets::{pseudo_peaks, Waveform};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;
use theme_core::Node;

// ── shared 250 ms ticker ─────────────────────────────────────────────────────────────────────
type Sub = (glib::WeakRef<gtk::Widget>, Rc<dyn Fn()>);

#[derive(Default)]
struct Hub {
    subs: RefCell<Vec<Sub>>,
    playing: Cell<bool>,
    timer: RefCell<Option<glib::SourceId>>,
}

thread_local! { static HUB: Hub = Hub::default(); }

fn subscribe(w: &gtk::Widget, tick: Rc<dyn Fn()>) {
    let r: glib::WeakRef<gtk::Widget> = glib::WeakRef::new();
    r.set(Some(w));
    HUB.with(|h| h.subs.borrow_mut().push((r, tick)));
}

/// Called on every engine state push: the ticker runs only while playing.
fn set_playing(on: bool) {
    HUB.with(|h| {
        h.playing.set(on);
        if on {
            if h.timer.borrow().is_none() {
                let id = glib::timeout_add_local(Duration::from_millis(250), || {
                    let any_mapped = HUB.with(|h| {
                        let mut subs = h.subs.borrow_mut();
                        subs.retain(|(w, _)| w.upgrade().is_some());
                        let live: Vec<Rc<dyn Fn()>> = subs.iter().filter(|(w, _)| w.upgrade().is_some_and(|w| w.is_mapped())).map(|(_, f)| f.clone()).collect();
                        drop(subs);
                        let any = !live.is_empty();
                        for f in live { f(); }
                        any
                    });
                    if any_mapped { glib::ControlFlow::Continue } else { HUB.with(|h| h.timer.borrow_mut().take()); glib::ControlFlow::Break }
                });
                *h.timer.borrow_mut() = Some(id);
            }
        } else if let Some(id) = h.timer.borrow_mut().take() {
            id.remove();
        }
    })
}

/// Wire a part to engine state: `on_state` runs on every push, `tick` on every push and on each hub tick while mapped.
/// The hook drops itself once the part's widget is gone (theme rebuilds).
fn wire(ctx: &Rc<Ctx>, w: &impl IsA<gtk::Widget>, on_state: impl Fn(&Snapshot) + 'static, tick: Rc<dyn Fn()>) {
    let w = w.upcast_ref::<gtk::Widget>();
    subscribe(w, tick.clone());
    let weak = w.downgrade();
    ctx.on_state_while(move |s| {
        if weak.upgrade().is_none() { return false; }
        on_state(s);
        tick();
        set_playing(s.status == Status::Playing);
        true
    });
}

fn current(s: &Snapshot) -> Option<&ytm_core::Track> { s.current.and_then(|i| s.queue.get(i)) }

/// Position / duration from the audio clock (`0.0` when unknown).
fn clock(ctx: &Ctx) -> (f64, f64) { let c = ctx.engine.clock(); (c.position().max(0.0), c.duration().max(0.0)) }

fn icon_btn(icon: &str, tip: &str, px: i32) -> gtk::Button {
    gtk::Button::builder().icon_name(icon).tooltip_text(tip).css_classes(["bar-btn"]).valign(gtk::Align::Center).build()
        .tap(|b| b.set_size_request(px, px))
}

trait Tap: Sized { fn tap(self, f: impl FnOnce(&Self)) -> Self { f(&self); self } }
impl<T> Tap for T {}

fn pointer_open(ctx: &Rc<Ctx>, w: &impl IsA<gtk::Widget>) {
    let open = gtk::GestureClick::new();
    { let ctx = ctx.clone(); open.connect_released(move |_, _, _, _| ctx.navigate(Nav::NowPlaying)); }
    w.add_controller(open);
    w.set_cursor_from_name(Some("pointer"));
}

// ── cover ────────────────────────────────────────────────────────────────────────────────────
/// `size` px; `shape` = `square` | `round`. Radius comes from CSS (`.bar-cover`).
fn cover(env: &Env, n: &Node) -> gtk::Widget {
    let ctx = env.ctx;
    let size = n.int_or("size", 48).max(8) as i32;
    let img = art(ctx, None, size);
    img.add_css_class("bar-cover");
    if n.str_or("shape", "square") != "square" { img.add_css_class("round"); }
    img.set_valign(gtk::Align::Center);
    img.set_halign(gtk::Align::Center);
    pointer_open(ctx, &img);

    let last = Rc::new(RefCell::new(String::new()));
    let (weak, ctx) = (img.downgrade(), ctx.clone());
    ctx.clone().on_state_while(move |s| {
        let Some(img) = weak.upgrade() else { return false };
        if let Some(t) = current(s) {
            if *last.borrow() != t.video_id {
                *last.borrow_mut() = t.video_id.clone();
                set_art(&ctx, &img, t.thumb.as_deref(), size);
            }
        }
        true
    });
    img.upcast()
}

/// Title + artist (ellipsised).
fn title(env: &Env, n: &Node) -> gtk::Widget {
    let ctx = env.ctx;
    let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
    root.set_valign(gtk::Align::Center);
    root.add_css_class("bar-meta");
    // max_width_chars(1): the natural width collapses, so a long title can't widen the part and shove the controls around.
    let title = gtk::Label::builder().xalign(0.0).max_width_chars(1).ellipsize(pango::EllipsizeMode::End).css_classes(["bar-title"]).build();
    let artist = gtk::Label::builder().xalign(0.0).max_width_chars(1).ellipsize(pango::EllipsizeMode::End).css_classes(["bar-artist"]).build();
    root.set_size_request(n.int_or("width", 200) as i32, -1);
    root.append(&title);
    root.append(&artist);
    pointer_open(ctx, &root);

    let last = Rc::new(RefCell::new(String::new()));
    let (weak, ctx) = (root.downgrade(), ctx.clone());
    ctx.on_state_while(move |s| {
        if weak.upgrade().is_none() { return false; }
        let Some(t) = current(s) else { return true };
        if *last.borrow() != t.video_id {
            *last.borrow_mut() = t.video_id.clone();
            title.set_label(&t.title);
            artist.set_label(&t.artist_line());
        }
        true
    });
    root.upcast()
}

// ── transport ────────────────────────────────────────────────────────────────────────────────
/// `buttons` in order (`shuffle prev play next repeat heart`); `play` = `circle` | `pill`; `play_size`, `size` in px.
fn transport(env: &Env, n: &Node) -> gtk::Widget {
    let ctx = env.ctx;
    let mut names = n.strs("buttons");
    if names.is_empty() { names = ["shuffle", "prev", "play", "next", "repeat", "radio"].map(String::from).to_vec(); }
    let size = n.int_or("size", 32).max(8) as i32;
    let play_size = n.int_or("play_size", 40).max(8) as i32;
    let pill = n.str_or("play", "circle") == "pill";
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.set_valign(gtk::Align::Center);
    row.add_css_class("bar-transport");

    let mut btns: Vec<(String, gtk::Button)> = vec![];
    for name in &names {
        let b = match name.as_str() {
            "shuffle" => icon_btn("media-playlist-shuffle-symbolic", "Shuffle", size),
            "prev" => icon_btn("media-skip-backward-symbolic", "Previous", size),
            "next" => icon_btn("media-skip-forward-symbolic", "Next", size),
            "repeat" => icon_btn("media-playlist-repeat-symbolic", "Repeat", size),
            "radio" => icon_btn(crate::icons::RADIO, "Radio: playing a song from a list starts its radio", size),
            "heart" => icon_btn(crate::icons::HEART_OFF, "Like", size),
            "play" => {
                let w = if pill { (play_size as f32 * 1.7) as i32 } else { play_size };
                let b = gtk::Button::builder().icon_name("media-playback-start-symbolic").tooltip_text("Play / pause (Space)").css_classes(["bar-play", if pill { "pill" } else { "circle" }]).valign(gtk::Align::Center).build();
                b.set_size_request(w, play_size);
                b
            }
            other => { eprintln!("theme: transport button `{other}` is unknown"); continue; }
        };
        row.append(&b);
        btns.push((name.clone(), b));
    }
    let find = |k: &str| btns.iter().find(|(n, _)| n == k).map(|(_, b)| b.clone());
    if let Some(b) = find("shuffle") { let c = ctx.clone(); b.connect_clicked(move |_| { let on = c.snap.borrow().as_ref().is_some_and(|s| s.shuffle); c.engine.send(Command::SetShuffle(!on)); }); }
    if let Some(b) = find("prev") { let c = ctx.clone(); b.connect_clicked(move |_| c.engine.send(Command::Prev)); }
    if let Some(b) = find("next") { let c = ctx.clone(); b.connect_clicked(move |_| c.engine.send(Command::Next)); }
    if let Some(b) = find("play") { let c = ctx.clone(); b.connect_clicked(move |_| c.engine.send(Command::TogglePause)); }
    if let Some(b) = find("repeat") { let c = ctx.clone(); b.connect_clicked(move |_| { let r = c.snap.borrow().as_ref().map_or(Repeat::Off, |s| s.repeat); c.engine.send(Command::SetRepeat(r.cycle())); }); }
    if let Some(b) = find("radio") { let c = ctx.clone(); b.connect_clicked(move |_| { let on = c.snap.borrow().as_ref().is_some_and(|s| s.auto_radio); c.set_auto_radio(!on); }); }
    if let Some(b) = find("heart") { let c = ctx.clone(); b.connect_clicked(move |_| { if let Some(t) = c.current_track() { c.toggle_like(&t); } }); }

    let btns = Rc::new(btns);
    let state_btns = btns.clone();
    let weak = row.downgrade();
    let ctx2 = ctx.clone();
    ctx.on_state_while(move |s| {
        let Some(row) = weak.upgrade() else { return false };
        for (name, b) in state_btns.iter() {
            match name.as_str() {
                "play" => b.set_icon_name(if matches!(s.status, Status::Playing | Status::Loading) { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" }),
                "shuffle" => set_on(b, s.shuffle),
                "radio" => set_on(b, s.auto_radio),
                "repeat" => { b.set_icon_name(if s.repeat == Repeat::One { "media-playlist-repeat-song-symbolic" } else { "media-playlist-repeat-symbolic" }); set_on(b, s.repeat != Repeat::Off); }
                "heart" => {
                    let liked = current(s).is_some_and(|t| ctx2.store.is_liked(&t.video_id));
                    b.set_icon_name(crate::icons::heart(liked));
                    set_on(b, liked);
                }
                _ => {}
            }
        }
        let _ = row;
        true
    });
    row.upcast()
}

fn set_on(b: &gtk::Button, on: bool) { if on { b.add_css_class("on"); } else { b.remove_css_class("on"); } }

// ── seek ─────────────────────────────────────────────────────────────────────────────────────
/// `style` = `bar` (6 px rounded, Mixer) | `thin` (3 px, Lagoon / Tape Deck) | `waveform` (pseudo-waveform,
/// `bar_width` / `bar_gap` / `wave_height` px). `times` = elapsed on the left, remaining on the right, mono.
fn seek(env: &Env, n: &Node) -> gtk::Widget {
    let ctx = env.ctx;
    let style = n.str_or("style", "bar");
    let times = n.bool_or("times", true);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_hexpand(true);
    row.set_valign(gtk::Align::Center);
    row.add_css_class("bar-seek-row");
    let (el, rem) = (gtk::Label::builder().label("0:00").width_chars(5).xalign(1.0).css_classes(["bar-time"]).build(), gtk::Label::builder().label("--:--").width_chars(6).xalign(0.0).css_classes(["bar-time"]).build());
    if times { row.append(&el); }

    let scale: Option<gtk::Scale> = (style != "waveform").then(|| {
        let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.001);
        s.set_draw_value(false);
        s.set_focusable(false);
        s.set_hexpand(true);
        s.set_valign(gtk::Align::Center);
        s.add_css_class("bar-seek");
        if style == "thin" { s.add_css_class("thin"); }
        { let ctx = ctx.clone(); s.connect_change_value(move |_, _, v| { let d = ctx.engine.clock().duration(); if d > 0.0 { ctx.engine.send(Command::Seek(v.clamp(0.0, 1.0) * d)); } glib::Propagation::Proceed }); }
        s
    });
    let wave: Option<Waveform> = (style == "waveform").then(|| {
        let w = Waveform::new();
        w.set_bar_width(n.float_or("bar_width", 3.0) as f32);
        w.set_bar_gap(n.float_or("bar_gap", 2.0) as f32);
        w.set_height(n.int_or("wave_height", 32) as i32);
        w.set_hexpand(true);
        w.set_valign(gtk::Align::Center);
        w.add_css_class("bar-wave");
        { let ctx = ctx.clone(); w.connect_seek(move |f| { let d = ctx.engine.clock().duration(); if d > 0.0 { ctx.engine.send(Command::Seek(f.clamp(0.0, 1.0) * d)); } }); }
        w
    });
    match (&scale, &wave) {
        (Some(s), _) => row.append(s),
        (_, Some(w)) => row.append(w),
        _ => {}
    }
    if times { row.append(&rem); }

    let tick: Rc<dyn Fn()> = {
        let (ctx, scale, wave, el, rem) = (ctx.clone(), scale.clone(), wave.clone(), el.clone(), rem.clone());
        Rc::new(move || {
            let (p, d) = clock(&ctx);
            let frac = if d > 0.0 { (p / d).min(1.0) } else { 0.0 };
            if let Some(s) = &scale { if !s.state_flags().contains(gtk::StateFlags::ACTIVE) { s.set_value(frac); } }
            if let Some(w) = &wave { w.set_progress(frac); }
            if times {
                el.set_label(&fmt_time(p as u32));
                rem.set_label(&if d > 0.0 { format!("-{}", fmt_time((d - p).max(0.0) as u32)) } else { "--:--".to_string() });
            }
        })
    };
    let peaks: Rc<RefCell<(String, Vec<f32>)>> = Rc::default();
    wire(ctx, &row, {
        let wave = wave.clone();
        move |s| {
            let Some(w) = &wave else { return };
            let Some(t) = current(s) else { return };
            if peaks.borrow().0 != t.video_id {
                *peaks.borrow_mut() = (t.video_id.clone(), pseudo_peaks(&t.video_id, 240));
                w.set_peaks(&peaks.borrow().1);
            }
        }
    }, tick);
    row.upcast()
}

// ── volume ───────────────────────────────────────────────────────────────────────────────────
/// `width` px (default 96). Persists to the store key `volume`, like the now-playing controls.
fn volume(env: &Env, n: &Node) -> gtk::Widget {
    let ctx = env.ctx;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_valign(gtk::Align::Center);
    row.set_size_request(n.int_or("width", 96) as i32, -1);
    row.add_css_class("bar-volume");
    if n.bool_or("label", false) { row.append(&gtk::Label::builder().label("VOL").css_classes(["bar-time"]).build()); }
    let vol = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.02);
    vol.set_draw_value(false);
    vol.set_focusable(false);
    vol.set_hexpand(false); // a child's hexpand would make the whole part expand; the slider is exactly `width`
    vol.set_size_request(n.int_or("width", 96) as i32, -1);
    vol.set_valign(gtk::Align::Center);
    vol.add_css_class("bar-vol");
    row.append(&vol);
    let guard = Rc::new(Cell::new(false));
    { let (c, g) = (ctx.clone(), guard.clone()); vol.connect_value_changed(move |v| { if !g.get() { c.engine.send(Command::SetVolume(v.value())); let _ = c.store.set("volume", &v.value()); } }); }
    wire(ctx, &row, {
        let vol = vol.clone();
        move |s| {
            if (vol.value() - s.volume).abs() > 0.01 { guard.set(true); vol.set_value(s.volume); guard.set(false); }
        }
    }, Rc::new(|| {}));
    row.upcast()
}

// ── link ─────────────────────────────────────────────────────────────────────────────────────
/// A text button that navigates: `target` = `now` | `queue` | `effects`; label from `word` (vocabulary) or `text`.
fn link(env: &Env, n: &Node) -> gtk::Widget {
    let ctx = env.ctx;
    let label = n.str("word").map(|k| ctx.word(k)).or_else(|| n.str("text").map(str::to_string)).unwrap_or_default();
    let b = gtk::Button::builder().label(label).css_classes(["bar-link"]).valign(gtk::Align::Center).build();
    let target = n.str_or("target", "now").to_string();
    let c = ctx.clone();
    b.connect_clicked(move |_| c.navigate(match target.as_str() { "queue" => Nav::Queue, "effects" => Nav::Effects, _ => Nav::NowPlaying }));
    b.upcast()
}

// ── entry points ─────────────────────────────────────────────────────────────────────────────
pub fn component(env: &Env, node: &Node) -> Option<gtk::Widget> {
    let w = match node.c.as_str() {
        "cover" => cover(env, node),
        "title" => title(env, node),
        "transport" => transport(env, node),
        "seek" => seek(env, node),
        "volume" => volume(env, node),
        "link" => link(env, node),
        "track-actions" => {
            let a = crate::actions::current_actions(env.ctx, &["bar-btn"]);
            let mut c = a.first_child();
            while let Some(w) = c { w.set_size_request(28, 28); c = w.next_sibling(); }
            a.upcast()
        }
        _ => return None,
    };
    Some(w)
}

/// The bottom bar from `shell.bottom_bar.parts` (built fresh on every theme change). Hidden until a track exists,
/// then slides up like the mini bar.
pub fn bottom_bar(ctx: &Rc<Ctx>) -> gtk::Widget {
    let theme = ctx.theme();
    let bb = &theme.shell.bottom_bar;
    let env = Env { ctx, region: super::Region::Bar, data: None };
    // Liking must never depend on the theme: without an explicit heart, add heart + ⋯ right after the title.
    let mut parts = bb.parts.clone();
    let has_heart = parts.iter().any(|p| p.c == "track-actions" || (p.c == "transport" && p.strs("buttons").iter().any(|b| b == "heart")));
    if !has_heart {
        let at = parts.iter().position(|p| p.c == "title").map_or(0, |i| i + 1);
        parts.insert(at, Node { c: "track-actions".into(), children: vec![], props: Default::default() });
    }
    let row = build_all(&env, &parts, gtk::Orientation::Horizontal, 14);
    row.add_css_class("themed-bar");
    row.set_size_request(-1, bb.height);
    let revealer = gtk::Revealer::builder().transition_type(gtk::RevealerTransitionType::SlideUp).hexpand(true).build();
    revealer.set_child(Some(&row));
    let weak = revealer.downgrade();
    ctx.on_state_while(move |s| {
        let Some(rev) = weak.upgrade() else { return false };
        rev.set_reveal_child(current(s).is_some());
        true
    });
    revealer.upcast()
}

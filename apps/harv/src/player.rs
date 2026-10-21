//! Playback UI: the mini player bar, the full now-playing screen (lyrics view + transport), and the queue dialog.

use crate::ctx::{Ctx, Nav};
use crate::rows::{art, fmt_time, set_art};
use adw::prelude::*;
use gtk::{glib, pango};
use lyrics_core::lrc::Lyrics;
use lyrics_view::{LyricsView, Meta};
use player_core::{Command, Repeat, Snapshot, SleepInfo, SleepMode, Status};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn icon_button(icon: &str, tip: &str, classes: &[&str]) -> gtk::Button {
    gtk::Button::builder().icon_name(icon).tooltip_text(tip).css_classes(classes.iter().map(|s| s.to_string()).collect::<Vec<_>>()).valign(gtk::Align::Center).build()
}

fn play_icon(s: &Snapshot) -> &'static str {
    match s.status { Status::Playing | Status::Loading => "media-playback-pause-symbolic", _ => "media-playback-start-symbolic" }
}

/// Play/pause button face: a spinner while the engine resolves the stream, else the play / pause icon.
pub fn set_play_button(b: &gtk::Button, s: &Snapshot) {
    if s.status == Status::Loading {
        if !b.child().is_some_and(|c| c.is::<gtk::Spinner>()) { b.set_child(Some(&gtk::Spinner::builder().spinning(true).build())); }
        b.add_css_class("loading");
    } else {
        b.set_icon_name(play_icon(s));
        b.remove_css_class("loading");
    }
}

/// Toggle the `loading` style class (dimmed times, pulsing progress) on a widget.
pub fn set_loading(w: &impl IsA<gtk::Widget>, loading: bool) { if loading { w.add_css_class("loading"); } else { w.remove_css_class("loading"); } }

fn repeat_icon(r: Repeat) -> &'static str { if r == Repeat::One { "media-playlist-repeat-song-symbolic" } else { "media-playlist-repeat-symbolic" } }

// ── mini bar ─────────────────────────────────────────────────────────────────────────────────
pub fn mini_bar(ctx: &Rc<Ctx>) -> gtk::Widget {
    let revealer = gtk::Revealer::builder().transition_type(gtk::RevealerTransitionType::SlideUp).build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("mini-bar");
    let progress = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.001);
    progress.add_css_class("mini-seek");
    progress.set_draw_value(false);
    progress.set_focusable(false);
    { let ctx = ctx.clone(); progress.connect_change_value(move |_, _, v| { let d = ctx.engine.clock().duration(); if d > 0.0 { ctx.engine.send(Command::Seek(v.clamp(0.0, 1.0) * d)); } gtk::glib::Propagation::Proceed }); }
    root.append(&progress);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_margin_start(14);
    row.set_margin_end(14);
    row.set_margin_top(8);
    row.set_margin_bottom(10);

    let cover = art(ctx, None, 52);
    let info = gtk::Box::new(gtk::Orientation::Vertical, 2);
    info.set_valign(gtk::Align::Center);
    info.set_hexpand(true);
    let title = gtk::Label::builder().xalign(0.0).ellipsize(pango::EllipsizeMode::End).css_classes(["song-title"]).build();
    let artist = gtk::Label::builder().xalign(0.0).ellipsize(pango::EllipsizeMode::End).css_classes(["song-artist"]).build();
    info.append(&title);
    info.append(&artist);

    // Clicking the cover or the text opens the full screen.
    let open = gtk::GestureClick::new();
    { let ctx = ctx.clone(); open.connect_released(move |_, _, _, _| ctx.navigate(Nav::NowPlaying)); }
    let left = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    left.set_hexpand(true);
    left.append(&cover);
    left.append(&info);
    left.add_controller(open);
    left.set_cursor_from_name(Some("pointer"));
    crate::actions::attach_context_menu(&left, ctx, crate::actions::current(ctx), Default::default());
    row.append(&left);
    row.append(&crate::actions::current_actions(ctx, &["flat", "circular"]));

    // Fixed width: "1:23 / 4:56" ↔ "" would otherwise shift the transport buttons.
    let time = gtk::Label::builder().css_classes(["mini-time"]).width_chars(13).xalign(1.0).build();
    let prev = icon_button("media-skip-backward-symbolic", "Previous", &["flat", "circular"]);
    let play = icon_button("media-playback-start-symbolic", "Play / pause", &["circular", "play-btn"]);
    let next = icon_button("media-skip-forward-symbolic", "Next", &["flat", "circular"]);
    let queue = icon_button("view-list-symbolic", "Queue", &["flat", "circular"]);
    let radio = gtk::ToggleButton::builder().icon_name(crate::icons::RADIO).tooltip_text("Radio: playing a song from a list starts its radio").css_classes(["flat", "circular"]).valign(gtk::Align::Center).build();
    let radio_guard = Rc::new(Cell::new(false));
    { let (ctx, g) = (ctx.clone(), radio_guard.clone()); radio.connect_toggled(move |b| if !g.get() { ctx.set_auto_radio(b.is_active()); }); }
    let expand = icon_button("go-up-symbolic", "Now playing", &["flat", "circular"]);
    let mini = icon_button("view-restore-symbolic", "Mini player window", &["flat", "circular"]);
    { let ctx = ctx.clone(); mini.connect_clicked(move |_| ctx.navigate(Nav::MiniPlayer)); }
    for (b, f) in [(&prev, Command::Prev), (&play, Command::TogglePause), (&next, Command::Next)] {
        let ctx = ctx.clone();
        let f = Rc::new(f);
        b.connect_clicked(move |_| ctx.engine.send(match &*f { Command::Prev => Command::Prev, Command::Next => Command::Next, _ => Command::TogglePause }));
    }
    { let ctx = ctx.clone(); queue.connect_clicked(move |_| ctx.navigate(Nav::Queue)); }
    { let ctx = ctx.clone(); expand.connect_clicked(move |_| ctx.navigate(Nav::NowPlaying)); }
    for w in [&time.clone().upcast::<gtk::Widget>(), prev.upcast_ref(), play.upcast_ref(), next.upcast_ref(), radio.upcast_ref(), queue.upcast_ref(), mini.upcast_ref(), expand.upcast_ref()] { row.append(w); }
    root.append(&row);
    revealer.set_child(Some(&root));

    // Position ticker: only while playing, so an idle/paused app costs nothing.
    let timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let tick = {
        let (ctx, progress, time) = (ctx.clone(), progress.clone(), time.clone());
        Rc::new(move || {
            let clock = ctx.engine.clock();
            let (pos, dur) = (clock.position(), clock.duration());
            if !progress.state_flags().contains(gtk::StateFlags::ACTIVE) { progress.set_value(if dur > 0.0 { (pos / dur).clamp(0.0, 1.0) } else { 0.0 }); }
            time.set_label(&if dur > 0.0 { format!("{} / {}", fmt_time(pos as u32), fmt_time(dur as u32)) } else { String::new() });
        })
    };
    let last = Rc::new(RefCell::new(String::new()));
    {
        let (ctx2, revealer, timer, tick) = (ctx.clone(), revealer.clone(), timer.clone(), tick.clone());
        ctx.on_state(move |s| {
            let cur = s.current.and_then(|i| s.queue.get(i));
            revealer.set_reveal_child(cur.is_some());
            if let Some(t) = cur {
                if *last.borrow() != t.video_id {
                    *last.borrow_mut() = t.video_id.clone();
                    title.set_label(&t.title);
                    artist.set_label(&t.artist_line());
                    set_art(&ctx2, &cover, t.thumb.as_deref(), 52);
                }
            }
            set_play_button(&play, s);
            set_loading(&progress, s.status == Status::Loading);
            if s.status == Status::Loading { progress.set_value(0.0); time.set_label(""); }
            radio_guard.set(true);
            radio.set_active(s.auto_radio);
            radio_guard.set(false);
            if s.status == Status::Playing {
                if timer.borrow().is_none() {
                    let tick = tick.clone();
                    *timer.borrow_mut() = Some(glib::timeout_add_local(Duration::from_millis(250), move || { tick(); glib::ControlFlow::Continue }));
                }
            } else if let Some(id) = timer.borrow_mut().take() { id.remove(); }
            tick();
        });
    }
    revealer.upcast()
}

// ── now playing ──────────────────────────────────────────────────────────────────────────────
pub struct NowPlaying {
    pub widget: gtk::Widget,
    pub view: LyricsView,
    /// The transport row and the header's icon buttons: the open / close animation fades them in after the artwork.
    /// The back button and the window controls (minimise / maximise / close) are left alone.
    pub chrome: Vec<gtk::Widget>,
}

/// The full-screen view's look from the active theme's `[lyrics]` section. Text, progress bar and backdrop colours come from the
/// theme's tokens (`np_fg`, `bg`) and the live accent, so light / dark mode and the accent setting carry through.
fn fullscreen_style(ctx: &Ctx) -> lyrics_core::LyricsStyle {
    use lyrics_core::{Align, BackdropKind, LayoutKind, LyricsStyle};
    let (t, c) = (ctx.theme(), ctx.theme().colors.clone());
    let l = &t.lyrics;
    let mut st = LyricsStyle::spotlight();
    st.font_family = Some(std::iter::once(if l.font == "ui" { t.fonts.ui.clone() } else { l.font.clone() }).chain(t.fonts.fallbacks.iter().cloned()).collect::<Vec<_>>().join(", ")); // theme fallbacks (e.g. its Arabic font) go before the view's built-in stack
    st.active_font_px = l.active_size;
    st.font_px = l.size;
    st.line_gap = l.line_gap;
    st.align = if l.align == "center" { Align::Center } else { Align::Start };
    (st.blur, st.glow, st.spring_stiffness, st.cascade_delay) = (l.blur, l.glow, l.spring, l.cascade);
    st.backdrop = l.backdrop != "none";
    st.backdrop_kind = match l.backdrop.as_str() { "gradient" => BackdropKind::Gradient, "solid" => BackdropKind::Solid, _ => BackdropKind::Cover };
    st.layout = match l.layout.as_str() { "centered" => LayoutKind::Centered, "lyrics-only" => LayoutKind::LyricsOnly, _ => LayoutKind::Split };
    st.cover_radius = l.cover_radius;
    st.cover_shadow = l.cover_shadow;
    st.tint = c.np_fg.rgb_f32();
    st.palette = [c.bg.rgb_f32(), ctx.accent_rgb()];
    st.scrim = c.bg.rgb_f32();
    st
}

pub fn now_playing(ctx: &Rc<Ctx>, on_accent: impl Fn([f32; 3]) + 'static, toggle_fullscreen: impl Fn() + 'static) -> NowPlaying {
    let view = LyricsView::new();
    view.set_controls_height(80.0);
    {
        let clock = ctx.engine.clock();
        view.set_position_source(move || clock.position() as f32);
        let ctx = ctx.clone();
        view.connect_seek(move |t| ctx.engine.send(Command::Seek(t as f64)));
    }
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&view));
    overlay.add_css_class("np-page");
    // The theme's [lyrics] section: look of the view now, and again whenever the theme, light / dark mode or accent changes.
    {
        let restyle = {
            let (ctx, view, overlay) = (ctx.clone(), view.downgrade(), overlay.downgrade());
            move || {
                let (Some(view), Some(overlay)) = (view.upgrade(), overlay.upgrade()) else { return };
                view.set_style(fullscreen_style(&ctx));
                if ctx.theme().lyrics.chrome == "capsule" { overlay.add_css_class("np-capsule"); } else { overlay.remove_css_class("np-capsule"); }
            }
        };
        restyle();
        ctx.on_accent(restyle);
    }

    // header (transparent, over the artwork)
    let header = adw::HeaderBar::new();
    header.add_css_class("flat");
    header.add_css_class("np-header");
    header.set_title_widget(Some(&gtk::Label::new(None)));
    let back = icon_button("go-down-symbolic", "Back (Esc)", &["flat"]);
    { let ctx = ctx.clone(); back.connect_clicked(move |_| ctx.navigate(Nav::CloseNowPlaying)); }
    header.pack_start(&back);
    let full = icon_button("view-fullscreen-symbolic", "Fullscreen (F)", &["flat"]);
    full.connect_clicked(move |_| toggle_fullscreen());
    let queue = icon_button("view-list-symbolic", "Queue", &["flat"]);
    { let ctx = ctx.clone(); queue.connect_clicked(move |_| ctx.navigate(Nav::Queue)); }
    let timing = lyrics_timing_button(ctx, &view);
    let effects = icon_button("multimedia-equalizer-symbolic", "Audio effects", &["flat"]);
    { let ctx = ctx.clone(); effects.connect_clicked(move |_| ctx.navigate(Nav::Effects)); }
    let sleep = sleep_button(ctx);
    header.pack_end(&full);
    header.pack_end(&queue);
    header.pack_end(&effects);
    header.pack_end(&sleep);
    header.pack_end(&timing);
    let header_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    header_box.set_valign(gtk::Align::Start);
    header_box.append(&header);
    let header_rev = gtk::Revealer::builder().transition_type(gtk::RevealerTransitionType::Crossfade).valign(gtk::Align::Start).reveal_child(true).build();
    header_rev.set_child(Some(&header_box));
    overlay.add_overlay(&header_rev);

    // Fullscreen: the header hides and comes back for a moment on real pointer motion. The window is only known once mapped.
    let hide_timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let attached = Cell::new(false);
    overlay.connect_map({
        let (rev, timer) = (header_rev.clone(), hide_timer.clone());
        move |o| {
            if attached.get() { return; }
            let Some(win) = o.root().and_downcast::<gtk::Window>() else { return };
            attached.set(true);
            rev.set_reveal_child(!win.is_fullscreen());
            let (rev, timer) = (rev.clone(), timer.clone());
            win.connect_fullscreened_notify(move |w| {
                if let Some(id) = timer.borrow_mut().take() { id.remove(); }
                rev.set_reveal_child(!w.is_fullscreen());
            });
        }
    });
    let motion = gtk::EventControllerMotion::new();
    let last_pos = Cell::new((0.0f64, 0.0f64));
    motion.connect_motion({
        let (rev, timer) = (header_rev.clone(), hide_timer.clone());
        move |_, x, y| {
            // GTK also emits motion while content animates under a still pointer: ignore small wobbles.
            let (lx, ly) = last_pos.replace((x, y));
            let Some(win) = rev.root().and_downcast::<gtk::Window>() else { return };
            if !win.is_fullscreen() || (x - lx).abs() + (y - ly).abs() < 3.0 { return; }
            rev.set_reveal_child(true);
            if let Some(id) = timer.borrow_mut().take() { id.remove(); }
            let (rev2, timer2) = (rev.clone(), timer.clone());
            *timer.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(2500), move || {
                timer2.borrow_mut().take();
                rev2.set_reveal_child(false);
            }));
        }
    });
    overlay.add_controller(motion);

    // transport controls, positioned under the progress bar by the view's layout callback
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    controls.set_halign(gtk::Align::Start);
    controls.set_valign(gtk::Align::Start);
    controls.add_css_class("np-controls");
    let shuffle = gtk::ToggleButton::builder().icon_name("media-playlist-shuffle-symbolic").tooltip_text("Shuffle").css_classes(["flat", "circular"]).valign(gtk::Align::Center).build();
    let prev = icon_button("media-skip-backward-symbolic", "Previous", &["flat", "circular"]);
    let play = icon_button("media-playback-start-symbolic", "Play / pause (Space)", &["circular", "play-btn", "big"]);
    let next = icon_button("media-skip-forward-symbolic", "Next", &["flat", "circular"]);
    let repeat = icon_button("media-playlist-repeat-symbolic", "Repeat", &["flat", "circular"]);
    let radio = gtk::ToggleButton::builder().icon_name(crate::icons::RADIO).tooltip_text("Radio: playing a song from a list starts its radio").css_classes(["flat", "circular"]).valign(gtk::Align::Center).build();
    let heart = icon_button(crate::icons::HEART_OFF, "Like", &["flat", "circular", "heart"]);
    let vol = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.02);
    vol.set_width_request(96);
    vol.set_valign(gtk::Align::Center);
    vol.add_css_class("vol");
    vol.set_value(1.0);
    let spacer = gtk::Box::builder().hexpand(true).build();
    let more = crate::actions::more_button(ctx, crate::actions::current(ctx), Default::default(), &["flat", "circular"]);
    for w in [shuffle.upcast_ref::<gtk::Widget>(), prev.upcast_ref(), play.upcast_ref(), next.upcast_ref(), repeat.upcast_ref(), radio.upcast_ref(), spacer.upcast_ref(), heart.upcast_ref(), more.upcast_ref(), vol.upcast_ref()] { controls.append(w); }
    // elapsed / remaining under the view's progress bar; one 250 ms timer, alive only while playing
    let times = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    times.add_css_class("np-times");
    let (elapsed, remaining) = (gtk::Label::builder().label("0:00").css_classes(["np-time"]).xalign(0.0).hexpand(true).build(), gtk::Label::builder().label("-0:00").css_classes(["np-time"]).xalign(1.0).build());
    times.append(&elapsed);
    times.append(&remaining);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.set_halign(gtk::Align::Start);
    column.set_valign(gtk::Align::Start);
    column.append(&times);
    column.append(&controls);
    controls.set_halign(gtk::Align::Fill);
    overlay.add_overlay(&column);
    let tick_times = {
        let (ctx, elapsed, remaining, column) = (ctx.clone(), elapsed.clone(), remaining.clone(), column.clone());
        Rc::new(move || {
            if !column.is_mapped() { return; }
            let c = ctx.engine.clock();
            let (p, d) = (c.position().max(0.0), c.duration());
            elapsed.set_label(&fmt_time(p as u32));
            remaining.set_label(&if d > 0.0 { format!("-{}", fmt_time((d - p).max(0.0) as u32)) } else { String::new() });
        })
    };
    let times_timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();

    view.connect_layout({
        let column = column.clone();
        move |l| {
            let [bx, by, bw, bh] = l.bar;
            column.set_margin_start(bx as i32);
            column.set_margin_top((by + bh + if l.compact { 4.0 } else { 8.0 }) as i32);
            column.set_width_request(bw as i32);
        }
    });

    { let ctx = ctx.clone(); prev.connect_clicked(move |_| ctx.engine.send(Command::Prev)); }
    { let ctx = ctx.clone(); play.connect_clicked(move |_| ctx.engine.send(Command::TogglePause)); }
    { let ctx = ctx.clone(); next.connect_clicked(move |_| ctx.engine.send(Command::Next)); }
    { let ctx = ctx.clone(); repeat.connect_clicked(move |_| { let r = ctx.snap.borrow().as_ref().map_or(Repeat::Off, |s| s.repeat); ctx.engine.send(Command::SetRepeat(r.cycle())); }); }
    let guard = Rc::new(Cell::new(false)); // ignore echoes of our own state pushes
    { let (ctx, g) = (ctx.clone(), guard.clone()); shuffle.connect_toggled(move |b| { if !g.get() { ctx.engine.send(Command::SetShuffle(b.is_active())); } }); }
    { let (ctx, g) = (ctx.clone(), guard.clone()); radio.connect_toggled(move |b| { if !g.get() { ctx.set_auto_radio(b.is_active()); } }); }
    { let (ctx, g) = (ctx.clone(), guard.clone()); vol.connect_value_changed(move |v| { if !g.get() { ctx.engine.send(Command::SetVolume(v.value())); let _ = ctx.store.set("volume", &v.value()); } }); }
    { let ctx = ctx.clone(); heart.connect_clicked(move |_| { if let Some(t) = ctx.current_track() { ctx.toggle_like(&t); } }); }

    // "No lyrics" note on the lyrics side
    let note = gtk::Label::builder().css_classes(["lyrics-note"]).halign(gtk::Align::End).valign(gtk::Align::Center).visible(false).build();
    note.set_margin_end(80);
    overlay.add_overlay(&note);
    note.set_can_target(false);

    // react to engine state
    let current: Rc<RefCell<String>> = Rc::default();
    // Songs played before the current one: returning to the last of them is "previous" (works for shuffle, wrap-around and MPRIS alike).
    let history: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let (ctx2, view2) = (ctx.clone(), view.clone());
        let (column, elapsed, remaining) = (column.clone(), elapsed.clone(), remaining.clone());
        let on_accent = Rc::new(on_accent);
        ctx.on_state(move |s| {
            guard.set(true);
            set_play_button(&play, s);
            set_loading(&column, s.status == Status::Loading);
            if s.status == Status::Loading { elapsed.set_label("0:00"); remaining.set_label(""); }
            shuffle.set_active(s.shuffle);
            radio.set_active(s.auto_radio);
            repeat.set_icon_name(repeat_icon(s.repeat));
            if s.repeat == Repeat::Off { repeat.remove_css_class("on"); } else { repeat.add_css_class("on"); }
            if (vol.value() - s.volume).abs() > 0.01 { vol.set_value(s.volume); }
            guard.set(false);
            if s.status == Status::Playing {
                if times_timer.borrow().is_none() { let t = tick_times.clone(); *times_timer.borrow_mut() = Some(glib::timeout_add_local(Duration::from_millis(250), move || { t(); glib::ControlFlow::Continue })); }
            } else if let Some(id) = times_timer.borrow_mut().take() { id.remove(); }
            tick_times();
            view2.set_playing(s.status == Status::Playing);
            if s.duration > 0.0 { view2.set_duration(s.duration as f32); }
            let Some(t) = s.current.and_then(|i| s.queue.get(i)) else { return };
            heart.set_icon_name(crate::icons::heart(ctx2.store.is_liked(&t.video_id)));
            if *current.borrow() == t.video_id { return; }
            let prev_id = std::mem::replace(&mut *current.borrow_mut(), t.video_id.clone());
            if !prev_id.is_empty() {
                let mut h = history.borrow_mut();
                let dir = if h.last() == Some(&t.video_id) { h.pop(); -1.0 } else { h.push(prev_id); if h.len() > 50 { h.remove(0); } 1.0 };
                view2.transition(dir);
            }
            view2.set_track(Meta { title: t.title.clone(), artist: t.artist_line(), album: t.album.clone().unwrap_or_default() }, None, None, s.duration.max(t.duration_s.unwrap_or(0) as f64) as f32);
            view2.set_offset(ctx2.lyrics.offset_ms(&t.video_id) as f32 / 1000.0);
            note.set_label("Loading lyrics…");
            note.set_visible(true);
            // cover → pixels (+ accent colour)
            if let Some(url) = &t.thumb {
                let (fut, id) = (ctx2.images.bytes(url, 640), t.video_id.clone());
                let (view3, cur, accent) = (view2.clone(), current.clone(), on_accent.clone());
                ctx2.spawn(async move {
                    let bytes = fut.await?;
                    tokio::task::spawn_blocking(move || image::load_from_memory(&bytes).ok().map(|i| i.to_rgba8())).await.ok().flatten()
                }, move |img| {
                    if *cur.borrow() != id { return; }
                    if let Some(img) = img {
                        accent(lyrics_core::palette::extract(&img)[0]);
                        view3.set_cover(Some(&img));
                    } else { view3.set_cover(None); }
                });
            } else { view2.set_cover(None); }
            // lyrics
            let (svc, tr, id) = (ctx2.lyrics.clone(), t.clone(), t.video_id.clone());
            let (view3, cur, note) = (view2.clone(), current.clone(), note.clone());
            ctx2.spawn(async move { svc.lyrics(&tr, false).await.map_err(|e| format!("{e:#}")) }, move |res| {
                if *cur.borrow() != id { return; }
                match res {
                    Ok(Some(l)) => { note.set_visible(false); view3.set_lyrics(Some(Lyrics { ..l })); }
                    Ok(None) => { note.set_label("No lyrics for this song"); view3.set_lyrics(None); }
                    Err(e) => { note.set_label(&format!("Couldn't load lyrics\n{e}")); view3.set_lyrics(None); }
                }
            });
        });
    }
    let chrome: Vec<gtk::Widget> = vec![column.clone().upcast(), timing.clone().upcast(), sleep.clone().upcast(), effects.clone().upcast(), queue.clone().upcast(), full.clone().upcast()];
    for w in &chrome { w.add_css_class("np-chrome"); }
    NowPlaying { widget: overlay.upcast(), view, chrome }
}

/// "Lyrics timing": nudge the offset for this song, ±100 ms, remembered per song.
fn lyrics_timing_button(ctx: &Rc<Ctx>, view: &LyricsView) -> gtk::MenuButton {
    let pop = gtk::Popover::new();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.set_margin_top(10);
    col.set_margin_bottom(10);
    col.set_margin_start(12);
    col.set_margin_end(12);
    col.append(&gtk::Label::builder().label("Lyrics timing").css_classes(["heading"]).build());
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let minus = gtk::Button::builder().label("−0.1 s").build();
    let plus = gtk::Button::builder().label("+0.1 s").build();
    let val = gtk::Label::builder().width_chars(9).css_classes(["song-time"]).build();
    row.append(&minus);
    row.append(&val);
    row.append(&plus);
    col.append(&row);
    let reset = gtk::Button::builder().label("Reset").css_classes(["flat"]).build();
    col.append(&reset);
    col.append(&gtk::Label::builder().label("Positive = lyrics appear later.\nSaved for this song.").css_classes(["dim"]).build());
    pop.set_child(Some(&col));
    let show = { let (v, val) = (view.clone(), val.clone()); Rc::new(move || val.set_label(&format!("{:+.1} s", v.offset()))) };
    let apply = {
        let (ctx, view, show) = (ctx.clone(), view.clone(), show.clone());
        Rc::new(move |delta: Option<f32>| {
            let new = match delta { Some(d) => view.offset() + d, None => 0.0 };
            view.set_offset((new * 10.0).round() / 10.0);
            if let Some(t) = ctx.current_track() { let _ = ctx.lyrics.set_offset_ms(&t.video_id, (view.offset() * 1000.0).round() as i64); }
            show();
        })
    };
    { let a = apply.clone(); minus.connect_clicked(move |_| a(Some(-0.1))); }
    { let a = apply.clone(); plus.connect_clicked(move |_| a(Some(0.1))); }
    { let a = apply.clone(); reset.connect_clicked(move |_| a(None)); }
    pop.connect_show(move |_| show());
    gtk::MenuButton::builder().icon_name("preferences-system-time-symbolic").tooltip_text("Lyrics timing").popover(&pop).css_classes(["flat"]).build()
}

// ── sleep timer ──────────────────────────────────────────────────────────────────────────────
fn sleep_button(ctx: &Rc<Ctx>) -> gtk::MenuButton {
    let pop = gtk::Popover::new();
    pop.add_css_class("menu");
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    col.set_margin_top(8);
    col.set_margin_bottom(8);
    col.set_margin_start(6);
    col.set_margin_end(6);
    col.append(&gtk::Label::builder().label("Sleep timer").xalign(0.0).margin_start(8).margin_bottom(2).css_classes(["heading"]).build());
    let status = gtk::Label::builder().xalign(0.0).margin_start(8).margin_bottom(6).css_classes(["dim"]).build();
    col.append(&status);
    let choices: [(&str, Option<SleepMode>); 8] = [
        ("5 min", Some(SleepMode::After(Duration::from_secs(300)))), ("10 min", Some(SleepMode::After(Duration::from_secs(600)))),
        ("15 min", Some(SleepMode::After(Duration::from_secs(900)))), ("30 min", Some(SleepMode::After(Duration::from_secs(1800)))),
        ("45 min", Some(SleepMode::After(Duration::from_secs(2700)))), ("1 hour", Some(SleepMode::After(Duration::from_secs(3600)))),
        ("End of this song", Some(SleepMode::EndOfTrack)), ("Turn off", None),
    ];
    for (label, mode) in choices {
        if mode.is_none() {
            let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
            sep.set_margin_top(4);
            sep.set_margin_bottom(4);
            col.append(&sep);
        }
        let b = gtk::Button::builder().label(label).css_classes(["flat", "menu-item"]).build();
        b.child().and_downcast::<gtk::Label>().inspect(|l| l.set_xalign(0.0));
        let (ctx, pop2) = (ctx.clone(), pop.downgrade());
        b.connect_clicked(move |_| {
            ctx.engine.send(Command::SetSleepTimer(mode));
            if let Some(p) = pop2.upgrade() { p.popdown(); }
        });
        col.append(&b);
    }
    pop.set_child(Some(&col));

    // The countdown ticks only while the popover is open and a deadline exists.
    let running = Rc::new(Cell::new(false));
    pop.connect_show({
        let (ctx, status, running) = (ctx.clone(), status.clone(), running.clone());
        move |p| {
            status.set_label(&sleep_text(ctx.snap.borrow().as_ref().and_then(|s| s.sleep.as_ref())));
            if running.replace(true) { return; }
            let (pw, ctx, status, running) = (p.downgrade(), ctx.clone(), status.clone(), running.clone());
            glib::timeout_add_local(Duration::from_secs(1), move || {
                let s = ctx.snap.borrow();
                let info = s.as_ref().and_then(|s| s.sleep.as_ref());
                let live = pw.upgrade().is_some_and(|p| p.is_visible()) && info.is_some_and(|i| i.deadline_ms.is_some());
                if !live { running.set(false); return glib::ControlFlow::Break; }
                status.set_label(&sleep_text(info));
                glib::ControlFlow::Continue
            });
        }
    });

    let btn = gtk::MenuButton::builder().icon_name("weather-clear-night-symbolic").tooltip_text("Sleep timer").popover(&pop).css_classes(["flat"]).build();
    {
        let (status, btn) = (status.clone(), btn.clone());
        ctx.on_state(move |s| {
            status.set_label(&sleep_text(s.sleep.as_ref()));
            if s.sleep.is_some() { btn.add_css_class("on"); } else { btn.remove_css_class("on"); }
        });
    }
    btn
}

fn sleep_text(sleep: Option<&SleepInfo>) -> String {
    match sleep {
        Some(i) if i.end_of_track => "Pausing after this song".into(),
        Some(SleepInfo { deadline_ms: Some(d), .. }) => format!("Pausing in {}", fmt_time(d.saturating_sub(unix_ms()).div_ceil(1000) as u32)),
        _ => "No timer set".into(),
    }
}

fn unix_ms() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64) }

// ── queue ────────────────────────────────────────────────────────────────────────────────────
pub fn queue_dialog(ctx: &Rc<Ctx>) {
    let dialog = adw::Dialog::builder().title(ctx.word("queue")).content_width(460).content_height(620).build();
    let tv = adw::ToolbarView::new();
    let hb = adw::HeaderBar::new();
    let shuffle = gtk::ToggleButton::builder().icon_name("media-playlist-shuffle-symbolic").tooltip_text("Shuffle").css_classes(["flat"]).build();
    let repeat = gtk::Button::builder().icon_name("media-playlist-repeat-symbolic").tooltip_text("Repeat").css_classes(["flat"]).build();
    let endless = gtk::ToggleButton::builder().icon_name("emblem-synchronizing-symbolic").tooltip_text("Endless radio: keep adding similar songs").css_classes(["flat"]).build();
    hb.pack_start(&shuffle);
    hb.pack_start(&repeat);
    hb.pack_start(&endless);
    tv.add_top_bar(&hb);
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["track-list"]).build();
    let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
    tv.set_content(Some(&scroll));
    dialog.set_child(Some(&tv));

    let guard = Rc::new(Cell::new(false));
    { let (c, g) = (ctx.clone(), guard.clone()); shuffle.connect_toggled(move |b| if !g.get() { c.engine.send(Command::SetShuffle(b.is_active())); }); }
    { let (c, g) = (ctx.clone(), guard.clone()); endless.connect_toggled(move |b| if !g.get() { c.engine.send(Command::SetEndless(b.is_active())); let _ = c.store.set("endless", &b.is_active()); }); }
    { let c = ctx.clone(); repeat.connect_clicked(move |_| { let r = c.snap.borrow().as_ref().map_or(Repeat::Off, |s| s.repeat); c.engine.send(Command::SetRepeat(r.cycle())); }); }
    { let c = ctx.clone(); list.connect_row_activated(move |_, row| c.engine.send(Command::Jump(row.index() as usize))); }

    // Rebuild only when the queue itself (or what is current) changes; position updates don't reach here.
    let shown: Rc<RefCell<(usize, Option<usize>, String)>> = Rc::new(RefCell::new((usize::MAX, None, String::new())));
    let weak = dialog.downgrade();
    let ctx2 = ctx.clone();
    ctx.on_state_while(move |s| {
        if weak.upgrade().is_none() { return false; }
        guard.set(true);
        shuffle.set_active(s.shuffle);
        endless.set_active(s.endless);
        repeat.set_icon_name(repeat_icon(s.repeat));
        if s.repeat == Repeat::Off { repeat.remove_css_class("on"); } else { repeat.add_css_class("on"); }
        guard.set(false);
        let sig = s.queue.iter().map(|t| t.video_id.as_str()).collect::<Vec<_>>().join(",");
        let key = (s.queue.len(), s.current, sig);
        {
            // Only the current song moved: rows (and the scroll position) stay; `refresh_rows` moves the .playing mark.
            let prev = shown.borrow();
            if prev.0 == key.0 && prev.2 == key.2 {
                let moved = prev.1 != key.1;
                drop(prev);
                if moved { shown.borrow_mut().1 = key.1; }
                return true;
            }
        }
        *shown.borrow_mut() = key;
        while let Some(c) = list.first_child() { list.remove(&c); }
        for (i, t) in s.queue.iter().enumerate() {
            let row = gtk::ListBoxRow::builder().css_classes(["track-row"]).build();
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            h.set_margin_start(8);
            h.set_margin_end(6);
            h.append(&art(&ctx2, t.thumb.as_deref(), 40));
            let col = gtk::Box::new(gtk::Orientation::Vertical, 1);
            col.set_hexpand(true);
            col.set_valign(gtk::Align::Center);
            let title = gtk::Label::builder().label(&t.title).xalign(0.0).ellipsize(pango::EllipsizeMode::End).css_classes(["song-title"]).build();
            if s.current == Some(i) { title.add_css_class("playing"); }
            col.append(&title);
            col.append(&gtk::Label::builder().label(&t.artist_line()).xalign(0.0).ellipsize(pango::EllipsizeMode::End).css_classes(["song-artist"]).build());
            h.append(&col);
            let heart = gtk::Button::builder().css_classes(["flat", "circular", "row-btn", "heart"]).valign(gtk::Align::Center).tooltip_text("Like").build();
            { let (c, t) = (ctx2.clone(), t.clone()); heart.connect_clicked(move |_| c.toggle_like(&t)); }
            ctx2.register_row(&t.video_id, &title, &heart);
            h.append(&heart);
            let rm = icon_button("list-remove-symbolic", "Remove from queue", &["flat", "circular", "row-btn"]);
            { let c = ctx2.clone(); rm.connect_clicked(move |_| crate::actions::remove_from_queue(&c, i)); }
            h.append(&rm);
            let qx = crate::actions::MenuExtras { queue_index: Some(i), ..Default::default() };
            crate::actions::attach_context_menu(&row, &ctx2, crate::actions::fixed(t.clone()), qx);
            { let c = ctx2.clone(); crate::actions::make_reorderable(&row, i, Rc::new(move |from, to| c.engine.send(Command::Move { from, to }))); }
            row.set_child(Some(&h));
            list.append(&row);
        }
        if let Some(i) = s.current { if let Some(row) = list.row_at_index(i as i32) { glib::idle_add_local_once(move || { row.grab_focus(); }); } }
        true
    });
    dialog.present(ctx.toasts.root().as_ref());
}

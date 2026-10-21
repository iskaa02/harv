//! Small detached player window: art, title / artist, prev / play-pause / next and a thin progress bar.
//! GTK4 cannot force "always on top" (Wayland has no client-side way to do it); keep it there from the compositor or the window menu.
use crate::ctx::Ctx;
use crate::rows::{art, set_art};
use adw::prelude::*;
use gtk::{glib, pango};
use player_core::{Command, Snapshot, Status};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

thread_local! {
    /// The open mini window, if any: a second `show` presents it instead of opening another.
    static WINDOW: RefCell<Option<glib::WeakRef<adw::Window>>> = RefCell::default();
}

fn icon_button(icon: &str, tip: &str, classes: &[&str]) -> gtk::Button {
    gtk::Button::builder().icon_name(icon).tooltip_text(tip).css_classes(classes.iter().map(|s| s.to_string()).collect::<Vec<_>>()).valign(gtk::Align::Center).build()
}

pub fn show(ctx: &Rc<Ctx>) {
    if let Some(w) = WINDOW.with(|w| w.borrow().as_ref().and_then(|w| w.upgrade())) { w.present(); return; }

    let win = adw::Window::builder().title("Harv").default_width(380).default_height(120).css_classes(["mini-window"]).build();
    win.set_size_request(320, 96);
    let tv = adw::ToolbarView::new();
    let hb = adw::HeaderBar::new();
    hb.add_css_class("flat");
    hb.set_show_title(false);
    tv.add_top_bar(&hb);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_margin_start(14);
    row.set_margin_end(14);
    row.set_margin_bottom(10);
    let cover = art(ctx, None, 56);
    let info = gtk::Box::new(gtk::Orientation::Vertical, 2);
    info.set_valign(gtk::Align::Center);
    info.set_hexpand(true);
    let title = gtk::Label::builder().xalign(0.0).ellipsize(pango::EllipsizeMode::End).css_classes(["song-title"]).build();
    let artist = gtk::Label::builder().xalign(0.0).ellipsize(pango::EllipsizeMode::End).css_classes(["song-artist"]).build();
    info.append(&title);
    info.append(&artist);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    controls.set_valign(gtk::Align::Center);
    let prev = icon_button("media-skip-backward-symbolic", "Previous", &["flat", "circular"]);
    let play = icon_button("media-playback-start-symbolic", "Play / pause", &["circular", "play-btn"]);
    let next = icon_button("media-skip-forward-symbolic", "Next", &["flat", "circular"]);
    { let c = ctx.clone(); prev.connect_clicked(move |_| c.engine.send(Command::Prev)); }
    { let c = ctx.clone(); play.connect_clicked(move |_| c.engine.send(Command::TogglePause)); }
    { let c = ctx.clone(); next.connect_clicked(move |_| c.engine.send(Command::Next)); }
    controls.append(&crate::actions::heart_button(ctx, &["flat", "circular"]));
    for b in [&prev, &play, &next] { controls.append(b); }
    row.append(&cover);
    row.append(&info);
    row.append(&controls);
    let progress = gtk::ProgressBar::builder().css_classes(["mini-progress"]).build();
    body.append(&row);
    body.append(&progress);
    tv.set_content(Some(&body));
    win.set_content(Some(&tv));

    // Progress ticker: 250 ms, only while playing and the window exists (and is visible). Stopped by the hook, the tick itself, or destroy.
    let timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let update = {
        let (c, progress) = (ctx.clone(), progress.clone());
        Rc::new(move || {
            let clock = c.engine.clock();
            let (pos, dur) = (clock.position(), clock.duration());
            progress.set_fraction(if dur > 0.0 { (pos / dur).clamp(0.0, 1.0) } else { 0.0 });
        })
    };
    let start = {
        let (c, weak, timer, update) = (ctx.clone(), win.downgrade(), timer.clone(), update.clone());
        Rc::new(move || {
            if timer.borrow().is_some() { return; }
            let (c, weak, t, update) = (c.clone(), weak.clone(), timer.clone(), update.clone());
            let id = glib::timeout_add_local(Duration::from_millis(250), move || {
                if !weak.upgrade().is_some_and(|w| w.is_visible()) || !c.is_playing() { t.borrow_mut().take(); return glib::ControlFlow::Break; }
                update();
                glib::ControlFlow::Continue
            });
            *timer.borrow_mut() = Some(id);
        })
    };
    let stop = {
        let timer = timer.clone();
        Rc::new(move || { if let Some(id) = timer.borrow_mut().take() { id.remove(); } })
    };
    { let stop = stop.clone(); win.connect_destroy(move |_| stop()); }

    // Track changes and play state come from the engine; the hook drops itself once the window is gone.
    let last = Rc::new(RefCell::new(String::new()));
    let weak = win.downgrade();
    let c = ctx.clone();
    ctx.on_state_while(move |s: &Snapshot| {
        let Some(_win) = weak.upgrade() else { return false };
        if let Some(t) = s.current.and_then(|i| s.queue.get(i)) {
            if *last.borrow() != t.video_id {
                *last.borrow_mut() = t.video_id.clone();
                title.set_label(&t.title);
                artist.set_label(&t.artist_line());
                set_art(&c, &cover, t.thumb.as_deref(), 56);
            }
        }
        crate::player::set_play_button(&play, s);
        crate::player::set_loading(&progress, s.status == Status::Loading);
        if s.status == Status::Playing { start() } else { stop() }
        update();
        true
    });

    // Space = play / pause here too (capture phase, so a focused button doesn't take it).
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    { let c = ctx.clone(); keys.connect_key_pressed(move |_, key, _, _| if key == gtk::gdk::Key::space { c.engine.send(Command::TogglePause); glib::Propagation::Stop } else { glib::Propagation::Proceed }); }
    win.add_controller(keys);

    WINDOW.with(|w| *w.borrow_mut() = Some(win.downgrade()));
    win.present();
}

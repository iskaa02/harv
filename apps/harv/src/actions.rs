//! The one track menu: the ⋯ button, the right-click / long-press / Menu-key popover, and the always-present heart.
//! Every place that shows a track builds its menu here, so the actions are the same everywhere.

use crate::ctx::{Ctx, Nav};
use crate::rows::{add_to_playlist, download_tracks, remove_download};
use adw::prelude::*;
use gtk::{gdk, glib};
use player_core::{Command, QueueKind};
use std::rc::Rc;
use ytm_core::Track;

/// Where the menu's track comes from: a fixed track (a row) or "whatever plays now" (bars, now-playing).
pub type TrackSrc = Rc<dyn Fn() -> Option<Track>>;

pub fn fixed(t: Track) -> TrackSrc { Rc::new(move || Some(t.clone())) }

pub fn current(ctx: &Rc<Ctx>) -> TrackSrc { let c = ctx.clone(); Rc::new(move || c.current_track()) }

/// Extra, context-specific entries (playlist rows, queue rows).
#[derive(Default, Clone)]
pub struct MenuExtras {
    pub remove_from_playlist: Option<Rc<dyn Fn()>>,
    pub move_up: Option<Rc<dyn Fn()>>,
    pub move_down: Option<Rc<dyn Fn()>>,
    /// Index in the engine queue (queue rows): adds "Remove from queue".
    pub queue_index: Option<usize>,
}

type Item = (String, Box<dyn Fn()>);

fn groups(ctx: &Rc<Ctx>, t: &Track, ex: &MenuExtras) -> Vec<Vec<Item>> {
    let item = |label: &str, f: Box<dyn Fn()>| -> Item { (label.to_string(), f) };
    let mut out: Vec<Vec<Item>> = vec![];

    let mut g = vec![];
    { let (c, t) = (ctx.clone(), t.clone()); g.push(item("Play next", Box::new(move || { c.engine.send(Command::PlayNext(t.clone())); c.toast("Playing next"); }))); }
    { let (c, t) = (ctx.clone(), t.clone()); g.push(item("Add to queue", Box::new(move || { c.engine.send(Command::Enqueue(vec![t.clone()])); c.toast("Added to queue"); }))); }
    { let (c, t) = (ctx.clone(), t.clone()); g.push(item("Add to playlist…", Box::new(move || add_to_playlist(&c, vec![t.clone()])))); }
    { let (c, t) = (ctx.clone(), t.clone()); g.push(item("Start radio", Box::new(move || c.play_one(t.clone())))); }
    out.push(g);

    let mut g = vec![];
    {
        let (c, t2) = (ctx.clone(), t.clone());
        let label = if ctx.store.is_liked(&t.video_id) { "Remove from liked" } else { "Like" };
        g.push(item(label, Box::new(move || c.toggle_like(&t2))));
    }
    {
        let (c, t2) = (ctx.clone(), t.clone());
        if ctx.store.download_path(&t.video_id).is_some() { g.push(item("Remove download", Box::new(move || remove_download(&c, &t2.video_id)))); }
        else { g.push(item("Download", Box::new(move || download_tracks(&c, vec![t2.clone()])))); }
    }
    out.push(g);

    let mut g = vec![];
    if let Some(a) = t.artists.first().cloned() { let c = ctx.clone(); g.push(item("Find artist", Box::new(move || c.navigate(Nav::SearchFor(a.clone()))))); }
    if let Some(a) = t.album.clone().filter(|a| !a.is_empty()) { let c = ctx.clone(); g.push(item("Find album", Box::new(move || c.navigate(Nav::SearchFor(a.clone()))))); }
    {
        let (c, id) = (ctx.clone(), t.video_id.clone());
        g.push(item("Copy link", Box::new(move || {
            if let Some(d) = gdk::Display::default() { d.clipboard().set_text(&format!("https://music.youtube.com/watch?v={id}")); }
            c.toast("Link copied");
        })));
    }
    out.push(g);

    let mut g = vec![];
    if let Some(f) = &ex.remove_from_playlist { let f = f.clone(); g.push(item("Remove from playlist", Box::new(move || f()))); }
    if let Some(f) = &ex.move_up { let f = f.clone(); g.push(item("Move up", Box::new(move || f()))); }
    if let Some(f) = &ex.move_down { let f = f.clone(); g.push(item("Move down", Box::new(move || f()))); }
    if let Some(i) = ex.queue_index { let c = ctx.clone(); g.push(item("Remove from queue", Box::new(move || remove_from_queue(&c, i)))); }
    if !g.is_empty() { out.push(g); }
    out
}

/// Remove queue entry `i`, with an Undo that puts the song back where it was.
pub fn remove_from_queue(ctx: &Rc<Ctx>, i: usize) {
    let (track, len) = { let s = ctx.snap.borrow(); match s.as_ref().and_then(|s| s.queue.get(i).map(|t| (t.clone(), s.queue.len()))) { Some(x) => x, None => return } };
    ctx.engine.send(Command::Remove(i));
    let c = ctx.clone();
    ctx.toast_action("Removed from queue", "Undo", move || {
        c.engine.send(Command::Enqueue(vec![track.clone()]));
        if i + 1 < len { c.engine.send(Command::Move { from: len - 1, to: i }); }
    });
}

/// Fill a popover with button groups separated by thin rules.
fn fill_groups(pop: &gtk::Popover, groups: Vec<Vec<Item>>) {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let weak = pop.downgrade();
    for (i, group) in groups.into_iter().enumerate() {
        if i > 0 { col.append(&gtk::Separator::builder().margin_top(4).margin_bottom(4).build()); }
        for (label, f) in group {
            let b = gtk::Button::builder().label(&label).css_classes(["flat", "menu-item"]).build();
            b.child().and_downcast::<gtk::Label>().inspect(|l| l.set_xalign(0.0));
            let weak = weak.clone();
            b.connect_clicked(move |_| { if let Some(p) = weak.upgrade() { p.popdown(); } f(); });
            col.append(&b);
        }
    }
    pop.set_child(Some(&col));
}

/// (Re)fill a popover with the menu for the current state of the track.
fn fill(pop: &gtk::Popover, ctx: &Rc<Ctx>, src: &TrackSrc, ex: &MenuExtras) {
    fill_groups(pop, src().map(|t| groups(ctx, &t, ex)).unwrap_or_default());
}

/// The ⋯ button. The menu is rebuilt on every open (labels depend on state).
pub fn more_button(ctx: &Rc<Ctx>, src: TrackSrc, extras: MenuExtras, classes: &[&str]) -> gtk::MenuButton {
    let pop = gtk::Popover::new();
    pop.add_css_class("menu");
    { let (c, s, e) = (ctx.clone(), src.clone(), extras.clone()); pop.connect_visible_notify(move |p| if p.is_visible() { fill(p, &c, &s, &e) }); }
    gtk::MenuButton::builder().icon_name("view-more-symbolic").popover(&pop).css_classes(classes.iter().map(|s| s.to_string()).collect::<Vec<_>>()).valign(gtk::Align::Center).tooltip_text("More").build()
}

fn popup_at(w: &gtk::Widget, ctx: &Rc<Ctx>, src: &TrackSrc, extras: &MenuExtras, at: Option<(f64, f64)>) {
    let (ctx, src, extras) = (ctx.clone(), src.clone(), extras.clone());
    popup_with(w, at, move |p| fill(p, &ctx, &src, &extras));
}

/// A popover on `w` (at the pointer, or the centre), filled by `fill_it`, removed again when closed.
fn popup_with(w: &gtk::Widget, at: Option<(f64, f64)>, fill_it: impl FnOnce(&gtk::Popover)) {
    let pop = gtk::Popover::new();
    pop.add_css_class("menu");
    pop.set_has_arrow(false);
    pop.set_halign(gtk::Align::Start);
    pop.set_parent(w);
    fill_it(&pop);
    let rect = match at { Some((x, y)) => gdk::Rectangle::new(x as i32, y as i32, 1, 1), None => gdk::Rectangle::new(w.width() / 2, w.height() / 2, 1, 1) };
    pop.set_pointing_to(Some(&rect));
    pop.connect_closed(|p| { let p = p.clone(); glib::idle_add_local_once(move || p.unparent()); });
    pop.popup();
}

/// Right-click, long-press and the Menu / Shift+F10 keys open the same menu on `w`.
pub fn attach_context_menu(w: &impl IsA<gtk::Widget>, ctx: &Rc<Ctx>, src: TrackSrc, extras: MenuExtras) {
    let w = w.upcast_ref::<gtk::Widget>();
    let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
    {
        let (ctx, src, extras, weak) = (ctx.clone(), src.clone(), extras.clone(), w.downgrade());
        click.connect_pressed(move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(w) = weak.upgrade() { popup_at(&w, &ctx, &src, &extras, Some((x, y))); }
        });
    }
    w.add_controller(click);
    let press = gtk::GestureLongPress::builder().touch_only(true).build();
    {
        let (ctx, src, extras, weak) = (ctx.clone(), src.clone(), extras.clone(), w.downgrade());
        press.connect_pressed(move |g, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(w) = weak.upgrade() { popup_at(&w, &ctx, &src, &extras, Some((x, y))); }
        });
    }
    w.add_controller(press);
    let keys = gtk::EventControllerKey::new();
    {
        let (ctx, weak) = (ctx.clone(), w.downgrade());
        keys.connect_key_pressed(move |_, key, _, state| {
            let menu = key == gdk::Key::Menu || (key == gdk::Key::F10 && state.contains(gdk::ModifierType::SHIFT_MASK));
            if !menu { return glib::Propagation::Proceed; }
            if let Some(w) = weak.upgrade() { popup_at(&w, &ctx, &src, &extras, None); }
            glib::Propagation::Stop
        });
    }
    w.add_controller(keys);
}

// ── albums, artists, playlists, podcasts (cards) ─────────────────────────────────────────────
#[derive(Clone)]
pub enum Collection {
    Album(String),
    Playlist(String),
    Podcast(String),
    Artist { id: String, name: String, thumb: Option<String> },
}

impl Collection {
    fn nav(&self) -> Nav {
        match self { Collection::Album(id) => Nav::Album(id.clone()), Collection::Playlist(id) => Nav::Playlist(id.clone()), Collection::Podcast(id) => Nav::Podcast(id.clone()), Collection::Artist { id, .. } => Nav::Artist(id.clone()) }
    }
    /// The collection a navigation target opens (None for pages that aren't one).
    pub fn from_nav(n: &Nav, title: &str, thumb: Option<&str>) -> Option<Collection> {
        Some(match n {
            Nav::Album(id) => Collection::Album(id.clone()),
            Nav::Playlist(id) => Collection::Playlist(id.clone()),
            Nav::Podcast(id) => Collection::Podcast(id.clone()),
            Nav::Artist(id) => Collection::Artist { id: id.clone(), name: title.to_string(), thumb: thumb.map(str::to_string) },
            _ => return None,
        })
    }
    fn kind(&self) -> QueueKind { if matches!(self, Collection::Album(_)) { QueueKind::Album } else { QueueKind::Playlist } }
}

/// Load a collection's tracks, then hand them to `then` (toasts on failure).
fn with_tracks(ctx: &Rc<Ctx>, c: &Collection, then: impl FnOnce(Vec<Track>) + 'static) {
    let (ytm, c2, c3) = (ctx.ytm.clone(), c.clone(), ctx.clone());
    ctx.toast("Loading…");
    ctx.spawn(async move {
        match c2 {
            Collection::Album(id) => ytm.album(&id).await.map(|a| { let mut t = a.tracks; for x in &mut t { x.album = Some(a.title.clone()); } t }),
            Collection::Playlist(id) => ytm.playlist(&id).await.map(|p| p.tracks),
            Collection::Podcast(id) => ytm.podcast(&id).await.map(|p| p.episodes),
            Collection::Artist { id, .. } => ytm.artist(&id).await.map(|a| { let mut t = a.songs; for x in &mut t { if x.thumb.is_none() { x.thumb = a.thumb.clone(); } } t }),
        }.map_err(|e| format!("{e:#}"))
    }, move |res| match res {
        Ok(t) if !t.is_empty() => then(t),
        Ok(_) => c3.toast("Nothing to play here"),
        Err(e) => c3.toast(&format!("Couldn't load: {e}")),
    });
}

/// Right-click / long-press / Menu key on an album, artist, playlist or podcast card.
pub fn attach_collection_menu(w: &impl IsA<gtk::Widget>, ctx: &Rc<Ctx>, c: Collection, title: String) {
    let w = w.upcast_ref::<gtk::Widget>();
    let build = {
        let (ctx, c, title) = (ctx.clone(), c.clone(), title.clone());
        move |pop: &gtk::Popover| {
            let item = |label: &str, f: Box<dyn Fn()>| -> Item { (label.to_string(), f) };
            let mut first = vec![];
            { let (x, c) = (ctx.clone(), c.clone()); first.push(item("Open", Box::new(move || x.navigate(c.nav())))); }
            { let (x, c) = (ctx.clone(), c.clone()); first.push(item("Play", Box::new(move || { let (x, k) = (x.clone(), c.kind()); with_tracks(&x.clone(), &c, move |t| { x.engine.send(Command::SetShuffle(false)); x.play(t, 0, k); }) }))); }
            { let (x, c) = (ctx.clone(), c.clone()); first.push(item("Shuffle", Box::new(move || { let (x, k) = (x.clone(), c.kind()); with_tracks(&x.clone(), &c, move |t| x.play_shuffled(t, k)) }))); }
            let mut second = vec![];
            { let (x, c) = (ctx.clone(), c.clone()); second.push(item("Play next", Box::new(move || { let x = x.clone(); with_tracks(&x.clone(), &c, move |t| { for tr in t.into_iter().rev() { x.engine.send(Command::PlayNext(tr)); } x.toast("Playing next"); }) }))); }
            { let (x, c) = (ctx.clone(), c.clone()); second.push(item("Add to queue", Box::new(move || { let x = x.clone(); with_tracks(&x.clone(), &c, move |t| { let n = t.len(); x.engine.send(Command::Enqueue(t)); x.toast(&format!("Added {n} songs to queue")); }) }))); }
            let mut groups = vec![first, second];
            match &c {
                Collection::Artist { id, name, thumb } => {
                    let (x, id, name, thumb) = (ctx.clone(), id.clone(), name.clone(), thumb.clone());
                    let on = ctx.store.is_followed(&id);
                    groups.push(vec![item(if on { "Unfollow" } else { "Follow" }, Box::new(move || { let _ = x.store.set_followed(&id, &name, thumb.as_deref(), !on); x.toast(if on { "Unfollowed" } else { "Following" }); }))]);
                }
                _ => {
                    let (x, c, title) = (ctx.clone(), c.clone(), title.clone());
                    groups.push(vec![item("Save as playlist", Box::new(move || {
                        let (x, title) = (x.clone(), title.clone());
                        with_tracks(&x.clone(), &c, move |t| match x.store.create_playlist(&title).and_then(|id| x.store.add_to_playlist(id, &t)) { Ok(n) => x.toast(&format!("Saved {n} songs to “{title}”")), Err(e) => x.toast(&format!("Couldn't save: {e}")) });
                    }))]);
                }
            }
            fill_groups(pop, groups);
        }
    };
    let build = Rc::new(build);
    let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
    { let (b, weak) = (build.clone(), w.downgrade()); click.connect_pressed(move |g, _, x, y| { g.set_state(gtk::EventSequenceState::Claimed); if let Some(w) = weak.upgrade() { let b = b.clone(); popup_with(&w, Some((x, y)), move |p| b(p)); } }); }
    w.add_controller(click);
    let press = gtk::GestureLongPress::builder().touch_only(true).build();
    { let (b, weak) = (build.clone(), w.downgrade()); press.connect_pressed(move |g, x, y| { g.set_state(gtk::EventSequenceState::Claimed); if let Some(w) = weak.upgrade() { let b = b.clone(); popup_with(&w, Some((x, y)), move |p| b(p)); } }); }
    w.add_controller(press);
    let keys = gtk::EventControllerKey::new();
    { let (b, weak) = (build, w.downgrade()); keys.connect_key_pressed(move |_, key, _, state| {
        if !(key == gdk::Key::Menu || (key == gdk::Key::F10 && state.contains(gdk::ModifierType::SHIFT_MASK))) { return glib::Propagation::Proceed; }
        if let Some(w) = weak.upgrade() { let b = b.clone(); popup_with(&w, None, move |p| b(p)); }
        glib::Propagation::Stop
    }); }
    w.add_controller(keys);
}

/// Drag this row onto another to reorder: `on_move(from, to)` gets the two indices (the row is dropped at the target's position).
pub fn make_reorderable(row: &impl IsA<gtk::Widget>, index: usize, on_move: Rc<dyn Fn(usize, usize)>) {
    let row = row.upcast_ref::<gtk::Widget>();
    let drag = gtk::DragSource::builder().actions(gdk::DragAction::MOVE).build();
    drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&(index as u32).to_value())));
    {
        let weak = row.downgrade();
        drag.connect_drag_begin(move |src, _| { if let Some(r) = weak.upgrade() { src.set_icon(Some(&gtk::WidgetPaintable::new(Some(&r))), 0, 0); } });
    }
    row.add_controller(drag);
    let drop = gtk::DropTarget::new(u32::static_type(), gdk::DragAction::MOVE);
    {
        let weak = row.downgrade();
        drop.connect_enter(move |_, _, _| { if let Some(r) = weak.upgrade() { r.add_css_class("drop-over"); } gdk::DragAction::MOVE });
        let weak = row.downgrade();
        drop.connect_leave(move |_| { if let Some(r) = weak.upgrade() { r.remove_css_class("drop-over"); } });
        let weak = row.downgrade();
        drop.connect_drop(move |_, v, _, _| {
            if let Some(r) = weak.upgrade() { r.remove_css_class("drop-over"); }
            match v.get::<u32>() { Ok(from) if from as usize != index => { on_move(from as usize, index); true } _ => false }
        });
    }
    row.add_controller(drop);
}

/// A heart for the track that is playing, kept in sync with the store (`Ctx::toggle_like` re-runs the state hooks).
pub fn heart_button(ctx: &Rc<Ctx>, classes: &[&str]) -> gtk::Button {
    let b = gtk::Button::builder().icon_name(crate::icons::HEART_OFF).tooltip_text("Like").css_classes(classes.iter().map(|s| s.to_string()).collect::<Vec<_>>()).valign(gtk::Align::Center).build();
    { let c = ctx.clone(); b.connect_clicked(move |_| { if let Some(t) = c.current_track() { c.toggle_like(&t); } }); }
    let weak = b.downgrade();
    let c = ctx.clone();
    ctx.on_state_while(move |s| {
        let Some(b) = weak.upgrade() else { return false };
        let t = s.current.and_then(|i| s.queue.get(i));
        let liked = t.is_some_and(|t| c.store.is_liked(&t.video_id));
        b.set_sensitive(t.is_some());
        b.set_icon_name(crate::icons::heart(liked));
        b.set_tooltip_text(Some(if liked { "Remove from liked" } else { "Like" }));
        if liked { b.add_css_class("on"); b.add_css_class("liked"); } else { b.remove_css_class("on"); b.remove_css_class("liked"); }
        true
    });
    b
}

/// Heart + ⋯ for the current track (the `track-actions` bar part, now-title, the mini bars).
pub fn current_actions(ctx: &Rc<Ctx>, classes: &[&str]) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    row.set_valign(gtk::Align::Center);
    row.add_css_class("track-actions");
    row.append(&heart_button(ctx, classes));
    let more = more_button(ctx, current(ctx), MenuExtras::default(), classes);
    row.append(&more);
    row
}

//! Reusable building blocks: cover art, track lists (rows, per-row menu, multi-select), cards, add-to-playlist dialog.

use crate::ctx::{Ctx, Nav};
use adw::prelude::*;
use gtk::{glib, pango};
use player_core::{Command, QueueKind};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use ytm_core::Track;

pub fn fmt_time(secs: u32) -> String {
    if secs >= 3600 { format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60) } else { format!("{}:{:02}", secs / 60, secs % 60) }
}

/// Fixed-size cover. (`gtk::Image` + `pixel_size` honours the size; `gtk::Picture` would not.)
pub fn art(ctx: &Rc<Ctx>, url: Option<&str>, px: i32) -> gtk::Image {
    let img = gtk::Image::builder().pixel_size(px).css_classes(["art"]).overflow(gtk::Overflow::Hidden).build();
    img.set_size_request(px, px);
    img.set_halign(gtk::Align::Center);
    img.set_valign(gtk::Align::Center);
    set_art(ctx, &img, url, px);
    img
}

/// Load `url` into `img`. The previous picture stays until the new one is ready (no blank flash on song changes); a load that
/// finishes after a newer request is dropped.
pub fn set_art(ctx: &Rc<Ctx>, img: &gtk::Image, url: Option<&str>, px: i32) {
    let want = url.unwrap_or("").to_string();
    // SAFETY: the key is private to this function and always holds a String.
    unsafe { img.set_data::<String>("art-url", want.clone()); }
    let Some(url) = url else { img.set_paintable(None::<&gtk::gdk::Paintable>); return };
    let img = img.downgrade();
    // Ask for 2x so HiDPI stays sharp; the server resizes, so it costs little.
    ctx.images.load(ctx, url, (px as u32 * 2).max(120), move |t| {
        let Some(img) = img.upgrade() else { return };
        let current = unsafe { img.data::<String>("art-url").map(|p| p.as_ref().clone()) };
        if current.as_deref() != Some(want.as_str()) { return; }
        img.set_paintable(t.as_ref());
    });
}

/// A button with an icon *and* a label (`gtk::Button` shows only the icon when both are set).
pub fn pill_button(icon: &str, label: &str, classes: &[&str]) -> gtk::Button {
    gtk::Button::builder().child(&adw::ButtonContent::builder().icon_name(icon).label(label).build()).css_classes(classes.iter().map(|s| s.to_string()).collect::<Vec<_>>()).build()
}

pub fn section_title(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).css_classes(["section-title"]).build()
}

/// How a track list is drawn. `Classic` is the original row; the others are the theme variants (`track-list style = …`).
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Look {
    #[default]
    Classic,
    /// Column header + table cells (index, title with cover, artist, album, time).
    Table,
    /// Header + flat rows with divider lines (no cover).
    Rows,
}

impl Look {
    pub fn parse(s: &str) -> Look {
        match s { "table" => Look::Table, "rows" => Look::Rows, _ => Look::Classic }
    }
    fn css(self) -> &'static str {
        match self { Look::Classic => "look-classic", Look::Table => "look-table", Look::Rows => "look-rows" }
    }
}

/// Per-row hooks beyond "play".
#[derive(Default, Clone)]
pub struct RowExtras {
    pub on_remove: Option<Rc<dyn Fn(usize)>>,
    pub on_move: Option<Rc<dyn Fn(usize, usize)>>,
    pub show_art: bool,
    /// Show the track number instead of cover art (album pages).
    pub numbered: bool,
    /// Theme variant (default `Classic` = the original rows, unchanged).
    pub look: Look,
    /// Cover size in px for the variants (0 = the variant's default).
    pub cover_px: i32,
    /// Table / rows columns: any of `index title artist album time` (empty = the variant's default).
    pub columns: Vec<String>,
}

fn column_names(look: Look, extras: &RowExtras) -> Vec<String> {
    if !extras.columns.is_empty() { return extras.columns.clone(); }
    let v: &[&str] = match look { Look::Table => &["index", "title", "artist", "album", "time"], _ => &["index", "title", "artist", "time"] };
    v.iter().map(|s| s.to_string()).collect()
}

/// Table / rows: the body cells of one row, appended to `h`. Returns the title label (for the playing highlight).
fn table_body(h: &gtk::Box, ctx: &Rc<Ctx>, t: &Track, i: usize, extras: &RowExtras, look: Look, px: i32) -> gtk::Label {
    let mut title_lbl = None;
    for c in column_names(look, extras) {
        match c.as_str() {
            "index" => h.append(&gtk::Label::builder().label((i + 1).to_string()).xalign(1.0).width_chars(3).valign(gtk::Align::Center).css_classes(["track-num"]).build()),
            "title" => {
                let cell = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                cell.set_hexpand(true);
                cell.set_valign(gtk::Align::Center);
                if look == Look::Table { cell.append(&art(ctx, t.thumb.as_deref(), px)); }
                let title = gtk::Label::builder().label(&t.title).xalign(0.0).hexpand(true).max_width_chars(1).ellipsize(pango::EllipsizeMode::End).css_classes(["song-title"]).build();
                cell.append(&title);
                h.append(&cell);
                title_lbl = Some(title);
            }
            "artist" => h.append(&gtk::Label::builder().label(t.artist_line()).xalign(0.0).width_request(120).max_width_chars(16).ellipsize(pango::EllipsizeMode::End).valign(gtk::Align::Center).css_classes(["song-artist"]).build()),
            "album" => h.append(&gtk::Label::builder().label(t.album.clone().unwrap_or_default()).xalign(0.0).width_request(130).max_width_chars(16).ellipsize(pango::EllipsizeMode::End).valign(gtk::Align::Center).css_classes(["song-artist"]).build()),
            "time" => h.append(&gtk::Label::builder().label(t.duration_s.map(fmt_time).unwrap_or_default()).xalign(1.0).width_request(56).valign(gtk::Align::Center).css_classes(["song-time"]).build()),
            _ => {}
        }
    }
    title_lbl.unwrap_or_else(|| gtk::Label::new(Some(&t.title)))
}

/// Column header for the table / rows looks (None for the others).
fn table_header(look: Look, extras: &RowExtras, px: i32) -> Option<gtk::Widget> {
    if !matches!(look, Look::Table | Look::Rows) { return None; }
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    h.add_css_class("track-thead");
    h.set_margin_start(8);
    h.set_margin_end(6);
    for c in column_names(look, extras) {
        let (text, xalign) = match c.as_str() { "index" => ("#", 1.0), "title" => ("TITLE", 0.0), "artist" => ("ARTIST", 0.0), "album" => ("ALBUM", 0.0), "time" => ("TIME", 1.0), _ => continue };
        let lbl = gtk::Label::builder().label(text).xalign(xalign).css_classes(["track-thead-cell"]).build();
        match c.as_str() {
            "index" => { lbl.set_width_chars(3); h.append(&lbl); }
            "title" => {
                if look == Look::Table { h.append(&gtk::Box::builder().width_request(px).build()); }
                lbl.set_hexpand(true);
                h.append(&lbl);
            }
            "artist" => { lbl.set_width_request(120); h.append(&lbl); }
            "album" => { lbl.set_width_request(130); h.append(&lbl); }
            _ => { lbl.set_width_request(56); h.append(&lbl); }
        }
    }
    // Room for the heart and the menu button at the end of each row.
    h.append(&gtk::Box::builder().width_request(78).build());
    Some(h.upcast())
}

pub fn track_list(ctx: &Rc<Ctx>, tracks: Vec<Track>, kind: QueueKind, extras: RowExtras) -> gtk::Widget {
    let tracks = Rc::new(tracks);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["track-list"]).activate_on_single_click(true).build();
    let look = extras.look;
    let px = if extras.cover_px > 0 { row_px(extras.cover_px) } else { row_px(match look { Look::Table => 36, _ => 40 }) };
    let checks: Rc<RefCell<Vec<gtk::CheckButton>>> = Rc::default();
    let selecting = Rc::new(std::cell::Cell::new(false));
    let select = gtk::ToggleButton::builder().label("Select").css_classes(["flat", "pill-btn"]).build();
    let last_pick = Rc::new(std::cell::Cell::new(0usize));
    // Activate = play from here (or toggle the checkbox while selecting).
    let activate: Rc<dyn Fn(usize)> = {
        let (ctx, tracks, checks, selecting) = (ctx.clone(), tracks.clone(), checks.clone(), selecting.clone());
        Rc::new(move |i: usize| {
            if selecting.get() { if let Some(c) = checks.borrow().get(i) { c.set_active(!c.is_active()); } return; }
            ctx.play_row(tracks.to_vec(), i, kind);
        })
    };

    for (i, t) in tracks.iter().enumerate() {
        let mut classes = vec!["track-row"];
        if look != Look::Classic { classes.push(look.css()); }
        let row = gtk::ListBoxRow::builder().css_classes(classes).build();
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        h.set_margin_start(8);
        h.set_margin_end(6);

        let check = gtk::CheckButton::builder().visible(false).valign(gtk::Align::Center).build();
        checks.borrow_mut().push(check.clone());
        h.append(&check);

        let title = if look != Look::Classic {
            table_body(&h, ctx, t, i, &extras, look, px)
        } else {
        if extras.numbered { h.append(&gtk::Label::builder().label((i + 1).to_string()).xalign(1.0).width_chars(2).valign(gtk::Align::Center).css_classes(["track-num"]).build()); }
        else { h.append(&art(ctx, t.thumb.as_deref(), row_px(40))); }
        let col = gtk::Box::new(gtk::Orientation::Vertical, 1);
        col.set_hexpand(true);
        col.set_valign(gtk::Align::Center);
        let title = gtk::Label::builder().label(&t.title).xalign(0.0).ellipsize(pango::EllipsizeMode::End).css_classes(["song-title"]).build();
        let sub = [t.artist_line(), t.album.clone().unwrap_or_default()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
        let artist = gtk::Label::builder().label(&sub).xalign(0.0).ellipsize(pango::EllipsizeMode::End).css_classes(["song-artist"]).build();
        col.append(&title);
        col.append(&artist);
        h.append(&col);

        let time = gtk::Label::builder().label(t.duration_s.map(fmt_time).unwrap_or_default()).css_classes(["song-time"]).width_chars(5).xalign(1.0).build();
        h.append(&time);
        title
        };

        let heart = gtk::Button::builder().css_classes(["flat", "circular", "row-btn", "heart"]).valign(gtk::Align::Center).tooltip_text("Favorite").build();
        {
            let (ctx, t) = (ctx.clone(), t.clone());
            heart.connect_clicked(move |_| ctx.toggle_like(&t));
        }
        ctx.register_row(&t.video_id, &title, &heart);
        h.append(&heart);

        let extras_menu = {
            let (rm, mv, n) = (extras.on_remove.clone(), extras.on_move.clone(), tracks.len());
            crate::actions::MenuExtras {
                remove_from_playlist: rm.map(|rm| Rc::new(move || rm(i)) as Rc<dyn Fn()>),
                move_up: mv.clone().filter(|_| i > 0).map(|mv| Rc::new(move || mv(i, i - 1)) as Rc<dyn Fn()>),
                move_down: mv.filter(|_| i + 1 < n).map(|mv| Rc::new(move || mv(i, i + 1)) as Rc<dyn Fn()>),
                queue_index: None,
            }
        };
        // Ctrl-click toggles a song and Shift-click selects a range; either one switches select mode on.
        {
            let g = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_PRIMARY).propagation_phase(gtk::PropagationPhase::Capture).build();
            let (select, checks, last_pick) = (select.clone(), checks.clone(), last_pick.clone());
            g.connect_pressed(move |g, _, _, _| {
                let st = g.current_event_state();
                let (ctrl, shift) = (st.contains(gtk::gdk::ModifierType::CONTROL_MASK), st.contains(gtk::gdk::ModifierType::SHIFT_MASK));
                if !ctrl && !shift { return; }
                g.set_state(gtk::EventSequenceState::Claimed);
                if !select.is_active() { select.set_active(true); }
                let checks = checks.borrow();
                if shift {
                    let (a, b) = (last_pick.get().min(i), last_pick.get().max(i));
                    for c in checks.iter().take(b + 1).skip(a) { c.set_active(true); }
                } else if let Some(c) = checks.get(i) { c.set_active(!c.is_active()); }
                last_pick.set(i);
            });
            row.add_controller(g);
        }
        let src = crate::actions::fixed(t.clone());
        h.append(&crate::actions::more_button(ctx, src.clone(), extras_menu.clone(), &["flat", "circular", "row-btn"]));
        crate::actions::attach_context_menu(&row, ctx, src, extras_menu);
        if let Some(mv) = &extras.on_move { crate::actions::make_reorderable(&row, i, mv.clone()); }

        row.set_child(Some(&h));
        list.append(&row);
    }

    {
        let act = activate.clone();
        list.connect_row_activated(move |_, row| act(row.index() as usize));
    }

    // header: count + select mode
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.add_css_class("list-head");
    let count = gtk::Label::builder().label(format!("{} songs", tracks.len())).xalign(0.0).hexpand(true).css_classes(["dim"]).build();
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    bar.set_visible(false);
    let chosen = {
        let (tracks, checks) = (tracks.clone(), checks.clone());
        Rc::new(move || tracks.iter().zip(checks.borrow().iter()).filter(|(_, c)| c.is_active()).map(|(t, _)| t.clone()).collect::<Vec<Track>>())
    };
    let action = |label: &str, f: Box<dyn Fn(Vec<Track>)>| {
        let b = gtk::Button::builder().label(label).css_classes(["flat", "pill-btn"]).build();
        let chosen = chosen.clone();
        b.connect_clicked(move |_| { let sel = chosen(); if !sel.is_empty() { f(sel); } });
        b
    };
    { let c = ctx.clone(); bar.append(&action("Play", Box::new(move |s| c.play(s, 0, QueueKind::Playlist)))); }
    { let c = ctx.clone(); bar.append(&action("Queue", Box::new(move |s| { let n = s.len(); c.engine.send(Command::Enqueue(s)); c.toast(&format!("Added {n} to queue")); }))); }
    { let c = ctx.clone(); bar.append(&action("Playlist…", Box::new(move |s| add_to_playlist(&c, s)))); }
    {
        let c = ctx.clone();
        bar.append(&action("Like", Box::new(move |s| { for t in &s { let _ = c.store.set_liked(t, true); } c.refresh_rows(); c.toast(&format!("Liked {}", s.len())); })));
    }
    {
        let c = ctx.clone();
        bar.append(&action("Unlike", Box::new(move |s| { for t in &s { let _ = c.store.set_liked(t, false); } c.refresh_rows(); c.toast(&format!("Removed {} from liked songs", s.len())); })));
    }
    { let c = ctx.clone(); bar.append(&action("Download", Box::new(move |s| download_tracks(&c, s)))); }
    {
        // Select all / none (not an `action`: it works with nothing selected).
        let all = gtk::Button::builder().label("All").css_classes(["flat", "pill-btn"]).build();
        let checks = checks.clone();
        all.connect_clicked(move |_| { let cs = checks.borrow(); let on = !cs.iter().all(|c| c.is_active()); for c in cs.iter() { c.set_active(on); } });
        bar.append(&all);
    }
    select.connect_toggled({
        let (checks, selecting, bar) = (checks.clone(), selecting.clone(), bar.clone());
        move |b| {
            selecting.set(b.is_active());
            bar.set_visible(b.is_active());
            for c in checks.borrow().iter() { c.set_visible(b.is_active()); if !b.is_active() { c.set_active(false); } }
        }
    });
    head.append(&count);
    head.append(&bar);
    head.append(&select);
    root.append(&head);
    if let Some(th) = table_header(look, &extras, px) { root.append(&th); }
    root.append(&list);
    root.upcast()
}

/// Download tracks for offline play (sequential, one yt-dlp at a time). Each finished file is recorded and registered with the engine.
pub fn download_tracks(ctx: &Rc<Ctx>, tracks: Vec<Track>) {
    let tracks: Vec<Track> = tracks.into_iter().filter(|t| ctx.store.download_path(&t.video_id).is_none()).collect();
    if tracks.is_empty() { return; }
    let total = tracks.len();
    ctx.toast(&if total == 1 { "Downloading…".to_string() } else { format!("Downloading {total} songs…") });
    let (ytm, dir, c) = (ctx.ytm.clone(), crate::downloads_dir(), ctx.clone());
    ctx.spawn(async move {
        let mut out = vec![];
        for t in tracks { let r = ytm.download(&t.video_id, &dir).await.map_err(|e| format!("{e:#}")); out.push((t, r)); }
        out
    }, move |res: Vec<(Track, Result<PathBuf, String>)>| {
        let (mut done, mut last, mut errs) = (0, String::new(), vec![]);
        for (t, r) in res {
            match r {
                Ok(p) => {
                    if let Err(e) = c.store.set_downloaded(&t, &p.to_string_lossy()) { errs.push(format!("couldn't save: {e}")); }
                    c.ytm.register_local(&t.video_id, p);
                    done += 1;
                    last = t.title.clone();
                }
                Err(e) => errs.push(e),
            }
        }
        let msg = match (total, errs.first()) {
            (1, None) => format!("Downloaded {last}"),
            (1, Some(e)) => format!("Download failed: {e}"),
            (_, None) => format!("Downloaded {done} songs"),
            (_, Some(e)) => format!("Downloaded {done} of {total} songs ({e})"),
        };
        c.toast(&msg);
    });
}

/// Delete a download's file (only inside the downloads folder) and forget it.
pub fn remove_download(ctx: &Rc<Ctx>, video_id: &str) {
    if let Some(p) = ctx.store.download_path(video_id).map(PathBuf::from) {
        if p.starts_with(crate::downloads_dir()) {
            if let Err(e) = std::fs::remove_file(&p) { if e.kind() != std::io::ErrorKind::NotFound { ctx.toast(&format!("Couldn't delete the file: {e}")); return; } }
        }
    }
    if let Err(e) = ctx.store.clear_download(video_id) { ctx.toast(&format!("Couldn't save: {e}")); return; }
    ctx.ytm.unregister_local(video_id);
    ctx.toast("Download removed");
}

/// "Download" for a whole list (skips what is already downloaded).
pub fn download_button(ctx: &Rc<Ctx>, tracks: Vec<Track>) -> gtk::Widget {
    let b = pill_button("folder-download-symbolic", "Download", &["pill"]);
    let c = ctx.clone();
    b.connect_clicked(move |_| download_tracks(&c, tracks.clone()));
    b.upcast()
}

/// "Add to playlist" chooser (with a "New playlist" row).
pub fn add_to_playlist(ctx: &Rc<Ctx>, tracks: Vec<Track>) {
    let dialog = adw::Dialog::builder().title("Add to playlist").content_width(360).content_height(420).build();
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    let col = gtk::Box::new(gtk::Orientation::Vertical, 10);
    col.set_margin_top(6);
    col.set_margin_bottom(14);
    col.set_margin_start(14);
    col.set_margin_end(14);

    let new_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = gtk::Entry::builder().placeholder_text("New playlist").hexpand(true).build();
    let create = gtk::Button::builder().label("Create").css_classes(["suggested-action"]).build();
    new_row.append(&entry);
    new_row.append(&create);
    col.append(&new_row);

    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
    let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
    col.append(&scroll);

    let ids: Rc<Vec<i64>> = Rc::new(ctx.store.playlists().unwrap_or_default().into_iter().map(|p| {
        let row = adw::ActionRow::builder().title(glib::markup_escape_text(&p.title)).subtitle(format!("{} songs", p.count)).activatable(true).build();
        list.append(&row);
        p.id
    }).collect());
    let tracks = Rc::new(tracks);
    let add_to = {
        let (ctx, tracks, dialog) = (ctx.clone(), tracks.clone(), dialog.clone());
        Rc::new(move |id: i64, name: String| {
            match ctx.store.add_to_playlist(id, &tracks) {
                Ok(n) => ctx.toast(&if n == tracks.len() { format!("Added to {name}") } else { format!("Added {n} (rest already in {name})") }),
                Err(e) => ctx.toast(&format!("Couldn't add: {e}")),
            }
            dialog.close();
        })
    };
    {
        let (ids, add_to) = (ids.clone(), add_to.clone());
        list.connect_row_activated(move |l, row| {
            let i = row.index() as usize;
            let name = row.downcast_ref::<adw::ActionRow>().map(|r| r.title().to_string()).unwrap_or_default();
            let _ = l;
            if let Some(&id) = ids.get(i) { add_to(id, name); }
        });
    }
    let make = {
        let (ctx, entry, add_to) = (ctx.clone(), entry.clone(), add_to.clone());
        Rc::new(move || {
            let name = entry.text().to_string();
            match ctx.store.create_playlist(&name) { Ok(id) => add_to(id, name.trim().to_string()), Err(_) => ctx.toast("Give the playlist a name") }
        })
    };
    { let make = make.clone(); create.connect_clicked(move |_| make()); }
    { let make = make.clone(); entry.connect_activate(move |_| make()); }
    tv.set_content(Some(&col));
    dialog.set_child(Some(&tv));
    dialog.present(ctx.toasts.root().as_ref());
}

/// A carousel card: cover (with a play badge that fades in on hover), title, subtitle. `on_click` fires on press.
thread_local! { static DENSITY: Cell<u32> = const { Cell::new(1) }; }

/// Preferences → Layout → Density (0 compact, 1 comfortable, 2 spacious): scales the cover in song rows along with the row height.
pub fn set_density(d: u32) { DENSITY.with(|c| c.set(d.min(2))); }
fn row_px(base: i32) -> i32 { match DENSITY.with(|c| c.get()) { 0 => base * 4 / 5, 2 => base * 6 / 5, _ => base } }

thread_local! { static CARD_PX: Cell<i32> = const { Cell::new(160) }; }

/// Preferences → Layout → Card size: the cover edge of shelf cards, px. Read when a card is built.
pub fn set_card_px(px: i32) { CARD_PX.with(|c| c.set(px.clamp(100, 260))); }
pub fn card_px() -> i32 { CARD_PX.with(|c| c.get()) }

pub fn card(ctx: &Rc<Ctx>, thumb: Option<&str>, title: &str, subtitle: &str, round: bool, on_click: impl Fn() + 'static) -> gtk::Widget {
    let b = gtk::Button::builder().css_classes(["flat", "card-btn"]).build();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let px = card_px();
    let chars = (px / 10).max(10);
    let a = art(ctx, thumb, px);
    a.set_halign(gtk::Align::Start);
    if round { a.add_css_class("round"); }
    let ov = gtk::Overlay::builder().child(&a).halign(gtk::Align::Start).build();
    let badge = gtk::Box::builder().css_classes(["card-play"]).halign(gtk::Align::End).valign(gtk::Align::End).can_target(false).build();
    let icon = gtk::Image::from_icon_name("media-playback-start-symbolic");
    icon.set_hexpand(true);
    icon.set_halign(gtk::Align::Center);
    badge.append(&icon);
    ov.add_overlay(&badge);
    col.append(&ov);
    col.append(&gtk::Label::builder().label(title).xalign(0.0).ellipsize(pango::EllipsizeMode::End).max_width_chars(chars).width_chars(chars).css_classes(["card-title"]).build());
    col.append(&gtk::Label::builder().label(subtitle).xalign(0.0).ellipsize(pango::EllipsizeMode::End).max_width_chars(chars).width_chars(chars).css_classes(["card-sub"]).build());
    b.set_child(Some(&col));
    b.connect_clicked(move |_| on_click());
    b.upcast()
}

/// A horizontally scrolling shelf of cards, with prev/next arrows (the scrollbar is hidden).
pub fn shelf(title: &str, cards: Vec<gtk::Widget>) -> gtk::Widget {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let t = section_title(title);
    t.set_hexpand(true);
    head.append(&t);
    let (left, right) = (arrow_button("go-previous-symbolic"), arrow_button("go-next-symbolic"));
    head.append(&left);
    head.append(&right);
    col.append(&head);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for c in cards { row.append(&c); }
    let sc = gtk::ScrolledWindow::builder().child(&row).vscrollbar_policy(gtk::PolicyType::Never).hscrollbar_policy(gtk::PolicyType::External).css_classes(["shelf"]).build();
    sc.set_propagate_natural_height(true);
    col.append(&sc);

    let adj = sc.hadjustment();
    let sync = { let (adj, l, r, head) = (adj.clone(), left.clone(), right.clone(), (left.clone(), right.clone())); let _ = head; move || {
        let overflow = adj.upper() > adj.page_size() + 1.0;
        l.set_visible(overflow);
        r.set_visible(overflow);
        l.set_sensitive(adj.value() > 1.0);
        r.set_sensitive(adj.value() + adj.page_size() < adj.upper() - 1.0);
    } };
    { let s = sync.clone(); adj.connect_changed(move |_| s()); }
    { let s = sync.clone(); adj.connect_value_changed(move |_| s()); }
    sync();
    let slide = |dir: f64| {
        let (adj, sc) = (adj.clone(), sc.clone());
        move |_: &gtk::Button| {
            let to = (adj.value() + dir * adj.page_size() * 0.85).clamp(0.0, (adj.upper() - adj.page_size()).max(0.0));
            let a = adj.clone();
            let anim = adw::TimedAnimation::builder().widget(&sc).value_from(adj.value()).value_to(to).duration(320).easing(adw::Easing::EaseOutCubic).target(&adw::CallbackAnimationTarget::new(move |v| a.set_value(v))).build();
            anim.play();
        }
    };
    left.connect_clicked(slide(-1.0));
    right.connect_clicked(slide(1.0));
    col.upcast()
}

fn arrow_button(icon: &str) -> gtk::Button {
    gtk::Button::builder().icon_name(icon).css_classes(["flat", "circular", "shelf-arrow"]).valign(gtk::Align::Center).build()
}

/// Empty / error state: a muted icon, a title and a line of explanation.
pub fn status(icon: &str, title: &str, desc: &str) -> gtk::Widget {
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).halign(gtk::Align::Center).valign(gtk::Align::Center).vexpand(true).css_classes(["state"]).build();
    b.append(&gtk::Image::builder().icon_name(icon).pixel_size(48).css_classes(["state-icon"]).margin_bottom(8).build());
    b.append(&gtk::Label::builder().label(title).css_classes(["state-title"]).wrap(true).justify(gtk::Justification::Center).build());
    if !desc.is_empty() { b.append(&gtk::Label::builder().label(desc).css_classes(["state-desc"]).wrap(true).max_width_chars(48).justify(gtk::Justification::Center).build()); }
    b.upcast()
}

/// Sorts a raw error into a human sentence: (icon, headline, hint).
fn classify_error(raw: &str) -> (&'static str, &'static str, &'static str) {
    let l = raw.to_lowercase();
    let has = |ws: &[&str]| ws.iter().any(|w| l.contains(w));
    if has(&["dns", "connect", "network", "timed out", "timeout", "resolve", "unreachable", "offline", "error sending request"]) {
        ("network-offline-symbolic", "You seem to be offline", "Check your connection and try again.")
    } else if has(&["429", "too many requests", "rate"]) {
        ("dialog-warning-symbolic", "YouTube Music is rate-limiting us", "Wait a minute, then try again.")
    } else if has(&["403", "401", "sign in", "login", "cookies", "forbidden"]) {
        ("dialog-password-symbolic", "YouTube Music refused the request", "It may need a sign-in or fresh cookies (Preferences).")
    } else if has(&["not found in api", "unable to parse", "expected", "parse", "missing field", "json"]) {
        ("dialog-warning-symbolic", "YouTube Music sent something unexpected", "That usually means the service changed. Try again, or try a different query.")
    } else {
        ("dialog-error-symbolic", "Something went wrong", "Try again in a moment.")
    }
}

/// Error state: what failed (`title`), a plain-language cause, a Retry button when `retry` is given, and the raw message behind "Details".
pub fn error_state(title: &str, raw: &str, retry: Option<Rc<dyn Fn()>>) -> gtk::Widget {
    let (icon, headline, hint) = classify_error(raw);
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).halign(gtk::Align::Center).valign(gtk::Align::Center).vexpand(true).css_classes(["state"]).build();
    b.append(&gtk::Image::builder().icon_name(icon).pixel_size(48).css_classes(["state-icon"]).margin_bottom(8).build());
    b.append(&gtk::Label::builder().label(title).css_classes(["state-title"]).wrap(true).justify(gtk::Justification::Center).build());
    b.append(&gtk::Label::builder().label(&format!("{headline}. {hint}")).css_classes(["state-desc"]).wrap(true).max_width_chars(48).justify(gtk::Justification::Center).build());
    if let Some(retry) = retry {
        let btn = gtk::Button::builder().label("Try again").halign(gtk::Align::Center).margin_top(10).css_classes(["suggested-action", "pill"]).build();
        btn.connect_clicked(move |_| retry());
        b.append(&btn);
    }
    if !raw.is_empty() {
        let detail = gtk::Label::builder().label(raw).selectable(true).wrap(true).wrap_mode(pango::WrapMode::WordChar).max_width_chars(64).xalign(0.0).css_classes(["state-detail"]).build();
        let ex = gtk::Expander::builder().label("Details").halign(gtk::Align::Center).margin_top(12).css_classes(["state-expander"]).child(&detail).build();
        b.append(&ex);
    }
    b.upcast()
}

fn bone(w: i32, h: i32, line: bool) -> gtk::Box {
    gtk::Box::builder().width_request(w).height_request(h).halign(gtk::Align::Start).css_classes(if line { vec!["skeleton", "line"] } else { vec!["skeleton"] }).build()
}

/// Loading placeholder shaped like a list of track rows.
pub fn spinner_page() -> gtk::Widget {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 10);
    for i in 0..7 {
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        h.set_margin_start(8);
        h.append(&bone(40, 40, false));
        let t = gtk::Box::new(gtk::Orientation::Vertical, 6);
        t.set_valign(gtk::Align::Center);
        t.append(&bone(150 + (i * 37 % 120), 11, true));
        t.append(&bone(90 + (i * 53 % 80), 9, true));
        h.append(&t);
        col.append(&h);
    }
    col.upcast()
}

/// Loading placeholder shaped like shelves of cards (home).
pub fn skeleton_shelves() -> gtk::Widget {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 22);
    for _ in 0..2 {
        let s = gtk::Box::new(gtk::Orientation::Vertical, 10);
        s.append(&bone(170, 18, true));
        let px = card_px();
        let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        for _ in 0..5 {
            let c = gtk::Box::new(gtk::Orientation::Vertical, 8);
            c.set_margin_start(8);
            c.append(&bone(px, px, false));
            c.append(&bone(120, 11, true));
            c.append(&bone(80, 9, true));
            r.append(&c);
        }
        s.append(&r);
        col.append(&s);
    }
    col.upcast()
}

pub fn open(ctx: &Rc<Ctx>, n: Nav) -> impl Fn() + 'static { let ctx = ctx.clone(); move || ctx.navigate(n.clone()) }

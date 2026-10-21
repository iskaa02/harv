//! Preferences dialog: playback, lyrics, appearance, library backup/import, storage.

use crate::ctx::Ctx;
use adw::prelude::*;
use gtk::{gio, glib};
use player_core::Command;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;
use ytm_core::Quality;

/// SponsorBlock categories: (API name, label, on by default). `music_offtopic` is the one that matters for music videos but is off by default.
const SB_CATS: [(&str, &str, bool); 7] = [
    ("sponsor", "Sponsor", true), ("selfpromo", "Self-promotion", true), ("interaction", "Interaction reminder", true), ("intro", "Intro", true),
    ("outro", "Outro", true), ("preview", "Preview / recap", true), ("music_offtopic", "Non-music section", false),
];

const QUALITIES: [(&str, &str, Quality); 4] = [("Low", "low", Quality::Low), ("Medium", "medium", Quality::Medium), ("High (Opus)", "high-opus", Quality::HighOpus), ("High (AAC)", "high-aac", Quality::HighAac)];

/// Shelf cover sizes (label, px); index 1 is the default.
const CARD_SIZES: [(&str, i32); 4] = [("Small", 128), ("Medium", 160), ("Large", 200), ("Huge", 240)];

fn apply_card_size(i: u32) { crate::rows::set_card_px(CARD_SIZES[(i as usize).min(3)].1); }

/// "Off" turns GTK's animations off (hover/press transitions, page and stack fades); "Follow system" restores what the desktop had at startup.
fn apply_ui_motion(i: u32) {
    thread_local! { static SYSTEM: Cell<Option<bool>> = const { Cell::new(None) }; }
    let Some(settings) = gtk::Settings::default() else { return };
    let system = SYSTEM.with(|s| { if s.get().is_none() { s.set(Some(settings.is_gtk_enable_animations())); } s.get().unwrap_or(true) });
    settings.set_gtk_enable_animations(if i == 1 { false } else { system });
}

/// One row per sidebar page: up / down within its section, and a switch to hide it.
fn fill_sidebar(ctx: &Rc<Ctx>, exp: &adw::ExpanderRow, rows: &Rc<RefCell<Vec<adw::ActionRow>>>) {
    for r in rows.borrow_mut().drain(..) { exp.remove(&r); }
    let items = crate::sidebar::page_items(&ctx.theme().shell.sidebar);
    let (order, hidden) = crate::sidebar::user_layout(ctx);
    let mut cur = items.clone();
    cur.sort_by_key(|(id, seg)| (*seg, order.iter().position(|o| o == id).unwrap_or(usize::MAX)));
    let ids: Vec<String> = cur.iter().map(|(id, _)| id.clone()).collect();
    for (i, (id, seg)) in cur.iter().enumerate() {
        let row = adw::ActionRow::builder().title(crate::sidebar::item_label(ctx, id)).build();
        let same = |j: usize| cur.get(j).is_some_and(|(_, s)| s == seg);
        let mover = |icon: &str, to: Option<usize>| {
            let b = gtk::Button::builder().icon_name(icon).css_classes(["flat"]).valign(gtk::Align::Center).sensitive(to.is_some_and(same)).build();
            if let Some(to) = to.filter(|t| same(*t)) {
                let (c, e, r, v) = (ctx.clone(), exp.clone(), rows.clone(), ids.clone());
                b.connect_clicked(move |_| { let mut v = v.clone(); v.swap(i, to); let _ = c.store.set("sidebar_order", &v); c.relayout(); fill_sidebar(&c, &e, &r); });
            }
            b
        };
        row.add_suffix(&mover("go-up-symbolic", i.checked_sub(1)));
        row.add_suffix(&mover("go-down-symbolic", Some(i + 1)));
        let sw = gtk::Switch::builder().valign(gtk::Align::Center).active(!hidden.contains(id)).build();
        { let (c, id) = (ctx.clone(), id.clone()); sw.connect_active_notify(move |s| {
            let mut h: Vec<String> = c.store.get("sidebar_hidden").unwrap_or_default();
            h.retain(|x| *x != id);
            if !s.is_active() { h.push(id.clone()); }
            let _ = c.store.set("sidebar_hidden", &h);
            c.relayout();
        }); }
        row.add_suffix(&sw);
        exp.add_row(&row);
        rows.borrow_mut().push(row);
    }
}

fn mb(bytes: u64) -> String { format!("{:.1} MB", bytes as f64 / 1_048_576.0) }

/// The theme gallery in a modal over Preferences.
fn open_theme_picker(ctx: &Rc<Ctx>, parent: &adw::PreferencesDialog) {
    let dialog = adw::Dialog::builder().title("Theme").content_width(900).content_height(640).build();
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.set_margin_start(20);
    holder.set_margin_end(20);
    holder.set_margin_bottom(20);
    crate::theme_gallery::fill(&holder, ctx, &dialog);
    let scroll = gtk::ScrolledWindow::builder().child(&holder).hscrollbar_policy(gtk::PolicyType::Never).build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&scroll));
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));
}

pub fn show(ctx: &Rc<Ctx>) {
    let dialog = adw::PreferencesDialog::builder().title("Preferences").build();
    let page = adw::PreferencesPage::new();
    // Appearance
    let g = adw::PreferencesGroup::builder().title("Appearance").description(format!("Your own themes live in {}", crate::theme_gallery::user_dir_label())).build();
    // Theme gallery lives in its own modal, opened from this row.
    if let Some(skin) = crate::theme::skin() {
        let style = adw::ComboRow::builder().title("Style").subtitle("Light, dark, or follow the system; every theme has both").model(&gtk::StringList::new(&crate::theme::Mode::ALL.map(|m| m.1))).build();
        style.set_selected(crate::theme::Mode::ALL.iter().position(|m| m.0 == skin.mode()).unwrap_or(0) as u32);
        { let c = ctx.clone(); style.connect_selected_notify(move |r| {
            let mode = crate::theme::Mode::ALL[(r.selected() as usize).min(2)].0;
            let Some(skin) = crate::theme::skin() else { return };
            if skin.mode() == mode { return; }
            match skin.set_mode(&c, mode) { Ok(()) => { let _ = c.store.set("color_mode", &mode.key().to_string()); } Err(e) => c.toast(&format!("Theme error: {e}")) }
        }); }
        let row = adw::ActionRow::builder().title("Theme").subtitle(ctx.theme().meta.name.as_str()).activatable(true).build();
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        { let w = row.downgrade(); ctx.on_theme(move |t| { if let Some(r) = w.upgrade() { r.set_subtitle(&t.meta.name); } }); }
        { let (c, d) = (ctx.clone(), dialog.clone()); row.connect_activated(move |_| open_theme_picker(&c, &d)); }
        g.add(&row);
        g.add(&style);
    }
    let acc = adw::ComboRow::builder().title("Accent colour").subtitle("Overrides the theme's accent unless set to From artwork").model(&gtk::StringList::new(&Ctx::ACCENTS.map(|a| a.0))).build();
    acc.set_selected(ctx.store.get::<u32>("accent").unwrap_or(0));
    { let c = ctx.clone(); acc.connect_selected_notify(move |r| { let _ = c.store.set("accent", &r.selected()); c.apply_accent(None); }); }
    g.add(&acc);
    let mo = adw::ComboRow::builder().title("Lyrics backdrop motion").subtitle("How alive the full-screen lyrics background is").model(&gtk::StringList::new(&["Off", "Calm", "Lively"])).build();
    mo.set_selected(ctx.store.get::<u32>("backdrop_motion").unwrap_or(2));
    { let c = ctx.clone(); mo.connect_selected_notify(move |r| { let _ = c.store.set("backdrop_motion", &r.selected()); apply_motion(r.selected()); }); }
    // Only the full-screen lyrics view (Blur) has the animated backdrop; page-hosted themes show plain lyrics.
    let animated = |t: &theme_core::Theme| t.shell.now_playing.host == "fullscreen" && t.lyrics.backdrop == "cover";
    mo.set_visible(animated(&ctx.theme()));
    { let w = mo.downgrade(); ctx.on_theme(move |t| { if let Some(r) = w.upgrade() { r.set_visible(animated(t)); } }); }
    g.add(&mo);
    // Page-hosted themes (Harv, Lagoon) show plain lyrics; these scale the theme's own text size and line spacing.
    for (title, subtitle, key, opts) in [("Lyrics text size", "Size of the lyrics on the now-playing page", "lyrics_size", crate::components::now::LYRICS_SIZES.map(|o| o.0)), ("Lyrics spacing", "Space between lyric lines", "lyrics_gap", crate::components::now::LYRICS_GAPS.map(|o| o.0))] {
        let row = adw::ComboRow::builder().title(title).subtitle(subtitle).model(&gtk::StringList::new(&opts)).build();
        row.set_selected(ctx.store.get::<u32>(key).unwrap_or(1));
        { let c = ctx.clone(); row.connect_selected_notify(move |r| { let _ = c.store.set(key, &r.selected()); crate::components::now::restyle_lyrics(); }); }
        row.set_visible(ctx.theme().shell.now_playing.host == "page");
        { let w = row.downgrade(); ctx.on_theme(move |t| { if let Some(r) = w.upgrade() { r.set_visible(t.shell.now_playing.host == "page"); } }); }
        g.add(&row);
    }
    page.add(&g);

    // Layout: how dense and how animated the app is, what Home and the sidebar show. Every row applies live.
    let g = adw::PreferencesGroup::builder().title("Layout").description("Each setting applies at once and overrides the theme's default").build();
    let combo = |title: &str, subtitle: &str, opts: &[&str], key: &str, default: u32| {
        let row = adw::ComboRow::builder().title(title).subtitle(subtitle).model(&gtk::StringList::new(opts)).build();
        row.set_selected(ctx.store.get::<u32>(key).unwrap_or(default));
        row
    };
    let density = combo("Density", "Height of song rows", &["Compact", "Comfortable", "Spacious"], "ui_density", 1);
    { let c = ctx.clone(); density.connect_selected_notify(move |r| {
        let _ = c.store.set("ui_density", &r.selected());
        let Some(skin) = crate::theme::skin() else { return };
        skin.set_density(r.selected());
        let spec = skin.current_id();
        if let Err(e) = skin.switch(&c, &spec) { c.toast(&format!("Theme error: {e}")); }
    }); }
    g.add(&density);
    let cards = combo("Card size", "Size of album and playlist covers on shelves", &CARD_SIZES.map(|c| c.0), "card_size", 1);
    { let c = ctx.clone(); cards.connect_selected_notify(move |r| { let _ = c.store.set("card_size", &r.selected()); apply_card_size(r.selected()); c.relayout(); }); }
    g.add(&cards);
    let motion = combo("Interface motion", "Hover, press and page transitions. Follow system respects your desktop's reduce-motion setting", &["Follow system", "Off"], "ui_motion", 0);
    { let c = ctx.clone(); motion.connect_selected_notify(move |r| { let _ = c.store.set("ui_motion", &r.selected()); apply_ui_motion(r.selected()); }); }
    g.add(&motion);
    let home = adw::ExpanderRow::builder().title("Home shelves").subtitle("Choose what the Home page shows").build();
    for (key, title) in [("home_recent", "Recently played"), ("home_favorites", "Favorites"), ("home_yt", "From YouTube Music")] {
        let row = adw::SwitchRow::builder().title(title).active(ctx.store.get::<bool>(key).unwrap_or(true)).build();
        { let c = ctx.clone(); row.connect_active_notify(move |r| { let _ = c.store.set(key, &r.is_active()); c.relayout(); }); }
        home.add_row(&row);
    }
    g.add(&home);
    let side = adw::ExpanderRow::builder().title("Sidebar pages").subtitle("Show, hide and reorder (within each section)").build();
    let side_rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::default();
    fill_sidebar(ctx, &side, &side_rows);
    g.add(&side);
    let reset = adw::ActionRow::builder().title("Reset layout").subtitle("Back to the theme's own density, covers, sidebar and Home").build();
    let b = gtk::Button::builder().label("Reset").valign(gtk::Align::Center).build();
    { let (c, d) = (ctx.clone(), dialog.clone()); b.connect_clicked(move |_| {
        for k in ["ui_density", "card_size"] { let _ = c.store.set(k, &1u32); }
        let _ = c.store.set("ui_motion", &0u32);
        for k in ["home_recent", "home_favorites", "home_yt"] { let _ = c.store.set(k, &true); }
        for k in ["sidebar_order", "sidebar_hidden"] { let _ = c.store.set(k, &Vec::<String>::new()); }
        apply_card_size(1);
        apply_ui_motion(0);
        if let Some(skin) = crate::theme::skin() { skin.set_density(1); let spec = skin.current_id(); let _ = skin.switch(&c, &spec); }
        c.relayout();
        d.close();
        let c = c.clone();
        glib::idle_add_local_once(move || show(&c));
    }); }
    reset.add_suffix(&b);
    g.add(&reset);
    page.add(&g);

    // Playback
    let g = adw::PreferencesGroup::builder().title("Playback").build();
    let q = adw::ComboRow::builder().title("Stream quality").subtitle("High needs a YouTube Music Premium login; otherwise the best free stream is used").model(&gtk::StringList::new(&QUALITIES.map(|q| q.0))).build();
    let cur = ctx.store.get::<String>("quality").unwrap_or_else(|| "high-opus".into());
    q.set_selected(QUALITIES.iter().position(|x| x.1 == cur).unwrap_or(2) as u32);
    { let c = ctx.clone(); q.connect_selected_notify(move |r| { let (_, key, quality) = QUALITIES[r.selected() as usize]; let _ = c.store.set("quality", &key); c.engine.send(Command::SetQuality(quality)); }); }
    g.add(&q);
    let endless = adw::SwitchRow::builder().title("Endless radio").subtitle("When the queue ends, keep adding songs similar to the last one").active(ctx.snap.borrow().as_ref().map_or(true, |s| s.endless)).build();
    { let c = ctx.clone(); endless.connect_active_notify(move |r| { let _ = c.store.set("endless", &r.is_active()); c.engine.send(Command::SetEndless(r.is_active())); }); }
    g.add(&endless);
    let ar = adw::SwitchRow::builder().title("Radio on click").subtitle("Playing a song from a list (recent, favourites, search, charts) starts its radio instead of queueing the rest of the list").active(ctx.snap.borrow().as_ref().map_or_else(|| ctx.store.get::<bool>("auto_radio").unwrap_or(false), |s| s.auto_radio)).build();
    { let c = ctx.clone(); ar.connect_active_notify(move |r| { if c.snap.borrow().as_ref().map_or(true, |s| s.auto_radio != r.is_active()) { c.set_auto_radio(r.is_active()); } }); }
    g.add(&ar);
    // Crossfade: `crossfade_ms` is what the engine gets (0 = off); `crossfade_len_ms` remembers the length while it is off.
    let on = ctx.store.get::<u64>("crossfade_ms").unwrap_or(0) > 0;
    let secs = ctx.store.get::<u64>("crossfade_len_ms").unwrap_or(5000) as f64 / 1000.0;
    let cf = adw::SwitchRow::builder().title("Crossfade").subtitle("Overlap the end of a song with the start of the next").active(on).build();
    let len = adw::SpinRow::builder().title("Fade length").subtitle("Seconds").adjustment(&gtk::Adjustment::new(secs, 1.0, 12.0, 1.0, 1.0, 0.0)).sensitive(on).build();
    let cf_apply: Rc<dyn Fn()> = {
        let (c, cf, len) = (ctx.clone(), cf.clone(), len.clone());
        Rc::new(move || {
            let ms = (len.value() * 1000.0).round() as u64;
            let eff = if cf.is_active() { ms } else { 0 };
            len.set_sensitive(cf.is_active());
            let _ = c.store.set("crossfade_ms", &eff);
            let _ = c.store.set("crossfade_len_ms", &ms);
            c.engine.send(Command::SetCrossfade(eff));
        })
    };
    { let f = cf_apply.clone(); cf.connect_active_notify(move |_| f()); }
    { let f = cf_apply.clone(); len.connect_value_notify(move |_| f()); }
    g.add(&cf);
    g.add(&len);
    page.add(&g);

    // SponsorBlock: the stored list is the enabled categories (empty = off); while the switch is off the boxes keep their state for this session.
    let g = adw::PreferencesGroup::builder().title("SponsorBlock").description("Skips segments that community members marked in YouTube videos. Not used for songs without a video.").build();
    let saved: Vec<String> = ctx.store.get("sponsorblock").unwrap_or_default();
    let sb_on = adw::SwitchRow::builder().title("Skip non-music segments (YouTube videos)").subtitle("Sponsors, intros, outros and similar").active(!saved.is_empty()).build();
    let cats: Rc<Vec<(&'static str, gtk::CheckButton)>> = Rc::new(SB_CATS.iter().map(|(k, _, def)| {
        let on = if saved.is_empty() { *def } else { saved.iter().any(|s| s == k) };
        let check = gtk::CheckButton::builder().valign(gtk::Align::Center).active(on).build();
        (*k, check)
    }).collect());
    let sb_apply: Rc<dyn Fn()> = {
        let (c, sb_on, cats) = (ctx.clone(), sb_on.clone(), cats.clone());
        Rc::new(move || {
            let list: Vec<String> = if sb_on.is_active() { cats.iter().filter(|(_, ch)| ch.is_active()).map(|(k, _)| k.to_string()).collect() } else { vec![] };
            let _ = c.store.set("sponsorblock", &list);
            c.engine.send(Command::SetSponsorBlock(list));
        })
    };
    { let f = sb_apply.clone(); sb_on.connect_active_notify(move |_| f()); }
    let exp = adw::ExpanderRow::builder().title("Categories").subtitle("What to skip").sensitive(!saved.is_empty()).build();
    for ((k, label, _), (_, check)) in SB_CATS.iter().zip(cats.iter()) {
        let row = adw::ActionRow::builder().title(*label).subtitle(*k).activatable_widget(check).build();
        row.add_prefix(check);
        { let f = sb_apply.clone(); check.connect_toggled(move |_| f()); }
        exp.add_row(&row);
    }
    { let exp = exp.clone(); sb_on.connect_active_notify(move |r| exp.set_sensitive(r.is_active())); }
    g.add(&sb_on);
    g.add(&exp);
    page.add(&g);

    // YouTube account: a cookies.txt lets yt-dlp act as the logged-in user.
    let g = adw::PreferencesGroup::builder().title("YouTube account").build();
    let row = adw::ActionRow::builder().title("Cookies file (cookies.txt)")
        .subtitle("Lets yt-dlp use your login (Premium audio quality, age-restricted songs). Library sync and Mix for You are not available.").build();
    let path_lbl = gtk::Label::builder().ellipsize(gtk::pango::EllipsizeMode::Start).max_width_chars(24).css_classes(["dim"]).valign(gtk::Align::Center).build();
    show_cookies(&path_lbl, ctx.store.get::<String>("cookies_file").as_deref());
    row.add_suffix(&path_lbl);
    let choose = gtk::Button::builder().label("Choose…").valign(gtk::Align::Center).build();
    let clear = gtk::Button::builder().label("Clear").valign(gtk::Align::Center).sensitive(ctx.store.get::<String>("cookies_file").is_some_and(|p| !p.is_empty())).build();
    { let (c, lbl, clear) = (ctx.clone(), path_lbl.clone(), clear.clone()); choose.connect_clicked(move |b| file_dialog(b, false, "", &c, {
        let (lbl, clear) = (lbl.clone(), clear.clone());
        move |c, path| { let s = path.to_string_lossy().into_owned(); let _ = c.store.set("cookies_file", &s); c.ytm.set_cookies_file(Some(path)); show_cookies(&lbl, Some(&s)); clear.set_sensitive(true); }
    })); }
    { let (c, lbl, clear2) = (ctx.clone(), path_lbl.clone(), clear.clone()); clear.connect_clicked(move |_| { let _ = c.store.set("cookies_file", &String::new()); c.ytm.set_cookies_file(None); show_cookies(&lbl, None); clear2.set_sensitive(false); }); }
    row.add_suffix(&choose);
    row.add_suffix(&clear);
    g.add(&row);
    page.add(&g);

    // Lyrics
    let g = adw::PreferencesGroup::builder().title("Lyrics").description("Sources: BetterLyrics (word-synced), then LRCLIB").build();
    let key = adw::PasswordEntryRow::builder().title("BetterLyrics API key (optional, applies after restart)").build();
    key.set_text(&ctx.store.get::<String>("betterlyrics_key").unwrap_or_default());
    { let c = ctx.clone(); key.connect_changed(move |e| { let _ = c.store.set("betterlyrics_key", &e.text().trim().to_string()); }); }
    g.add(&key);
    page.add(&g);

    // Library
    let g = adw::PreferencesGroup::builder().title("Library").description("Favorites, playlists, history, lyric timing and settings live in one file").build();
    let mk = |title: &str, sub: &str, label: &str| {
        let row = adw::ActionRow::builder().title(title).subtitle(sub).build();
        let b = gtk::Button::builder().label(label).valign(gtk::Align::Center).build();
        row.add_suffix(&b);
        (row, b)
    };
    let (row, b) = mk("Back up library", "Save a copy of your library", "Back up…");
    { let c = ctx.clone(); b.connect_clicked(move |b| file_dialog(b, true, "harv-backup.db", &c, |c, path| match c.store.backup_to(&path) { Ok(()) => c.toast("Backup saved"), Err(e) => c.toast(&format!("Backup failed: {e}")) })); }
    g.add(&row);
    let (row, b) = mk("Restore from backup", "Replaces everything in the library", "Restore…");
    { let c = ctx.clone(); b.connect_clicked(move |b| file_dialog(b, false, "", &c, |c, path| match c.store.restore_from(&path) { Ok(()) => { c.refresh_rows(); c.toast("Library restored — reopen the pages to see it") } Err(e) => c.toast(&format!("Restore failed: {e}")) })); }
    g.add(&row);
    let (row, b) = mk("Import from SimpMusic", "Playlists and songs from the web converter's JSON file", "Import…");
    { let c = ctx.clone(); b.connect_clicked(move |b| file_dialog(b, false, "", &c, |c, path| {
        match std::fs::read_to_string(&path).map_err(anyhow::Error::from).and_then(|j| c.store.import_simpmusic(&j)) {
            Ok(r) => c.toast(&format!("Imported {} songs and {} playlists ({} skipped)", r.songs_imported, r.playlists_created, r.skipped_entries)),
            Err(e) => c.toast(&format!("Import failed: {e}")),
        }
    })); }
    g.add(&row);
    page.add(&g);

    // Storage
    let g = adw::PreferencesGroup::builder().title("Storage").build();
    let (n, bytes) = ctx.images.disk_stats();
    let row = adw::ActionRow::builder().title("Thumbnail cache").subtitle(format!("{n} images · {}", mb(bytes))).build();
    let b = gtk::Button::builder().label("Clear").valign(gtk::Align::Center).build();
    { let (c, row) = (ctx.clone(), row.clone()); b.connect_clicked(move |_| { c.images.clear_disk(); row.set_subtitle("0 images · 0.0 MB"); }); }
    row.add_suffix(&b);
    g.add(&row);
    let (n, bytes) = ctx.store.lyrics_cache_stats();
    let row = adw::ActionRow::builder().title("Lyrics cache").subtitle(format!("{n} songs · {}", mb(bytes as u64))).build();
    let b = gtk::Button::builder().label("Clear").valign(gtk::Align::Center).build();
    { let (c, row) = (ctx.clone(), row.clone()); b.connect_clicked(move |_| { let _ = c.store.clear_lyrics_cache(); row.set_subtitle("0 songs · 0.0 MB"); }); }
    row.add_suffix(&b);
    g.add(&row);
    let row = adw::ActionRow::builder().title("Listening history").subtitle("Clears recently played and most played").build();
    let b = gtk::Button::builder().label("Clear").valign(gtk::Align::Center).css_classes(["destructive-action"]).build();
    { let c = ctx.clone(); b.connect_clicked(move |_| { let _ = c.store.clear_history(); c.toast("History cleared"); }); }
    row.add_suffix(&b);
    g.add(&row);
    page.add(&g);

    dialog.add(&page);
    dialog.present(ctx.toasts.root().as_ref());
}

fn file_dialog(from: &gtk::Button, save: bool, name: &str, ctx: &Rc<Ctx>, then: impl Fn(&Rc<Ctx>, std::path::PathBuf) + 'static) {
    let d = gtk::FileDialog::new();
    let win = from.root().and_downcast::<gtk::Window>();
    let ctx = ctx.clone();
    let done = move |r: Result<gio::File, gtk::glib::Error>| { if let Some(p) = r.ok().and_then(|f| f.path()) { then(&ctx, p); } };
    if save { d.set_initial_name(Some(name)); d.save(win.as_ref(), gio::Cancellable::NONE, done); } else { d.open(win.as_ref(), gio::Cancellable::NONE, done); }
}

fn show_cookies(lbl: &gtk::Label, path: Option<&str>) {
    match path.filter(|p| !p.is_empty()) {
        Some(p) => { lbl.set_label(p); lbl.set_tooltip_text(Some(p)); }
        None => { lbl.set_label("Not set"); lbl.set_tooltip_text(None); }
    }
}

fn apply_motion(i: u32) {
    lyrics_view::set_motion(match i { 0 => lyrics_view::Motion::Off, 1 => lyrics_view::Motion::Calm, _ => lyrics_view::Motion::Lively });
}

/// Re-apply saved crossfade, SponsorBlock and cookies settings at startup.
pub fn restore(ctx: &Rc<Ctx>) {
    apply_motion(ctx.store.get::<u32>("backdrop_motion").unwrap_or(2));
    apply_card_size(ctx.store.get::<u32>("card_size").unwrap_or(1));
    apply_ui_motion(ctx.store.get::<u32>("ui_motion").unwrap_or(0));
    let cats: Vec<String> = ctx.store.get("sponsorblock").unwrap_or_default();
    ctx.engine.send(Command::SetCrossfade(ctx.store.get::<u64>("crossfade_ms").unwrap_or(0)));
    ctx.engine.send(Command::SetSponsorBlock(cats));
    if let Some(p) = ctx.store.get::<String>("cookies_file").filter(|p| !p.is_empty()) { ctx.ytm.set_cookies_file(Some(p.into())); }
    // Dev hook: open the mini window shortly after start so it can be checked by hand (screenshots only capture the main window).
    if std::env::var("HARV_OPEN_MINI").as_deref() == Ok("1") {
        let c = ctx.clone();
        glib::timeout_add_local_once(Duration::from_millis(1500), move || crate::miniplayer::show(&c));
    }
}

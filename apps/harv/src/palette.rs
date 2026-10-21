//! The command palette (Ctrl+K / Ctrl+P): jump to a page, run an action, play something from your library, repeat a past search, or search for what was typed.
//! Local only (no request per keystroke), except `/ song`, which searches YouTube Music after a short pause.
//! Prefixes: `/` songs on YouTube Music · `>` commands only · `@` your playlists and followed artists.

use crate::ctx::{Ctx, Nav};
use crate::pages::Lib;
use crate::window::Root;
use adw::prelude::*;
use gtk::{gdk, glib};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use player_core::{Command, QueueKind, Repeat, SleepMode};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;
use store_core::PlaylistInfo;
use ytm_core::{SearchKind, Track};

thread_local! { static OPEN: RefCell<Option<adw::Dialog>> = RefCell::default(); }

/// Section order is also the order of an empty palette.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Sect { Recent, Current, Songs, Library, Pages, Playback, Settings, History, Search }

impl Sect {
    fn title(self) -> &'static str {
        match self { Sect::Recent => "RECENT", Sect::Current => "CURRENT SONG", Sect::Songs => "SONGS", Sect::Library => "LIBRARY", Sect::Pages => "GO TO", Sect::Playback => "PLAYBACK", Sect::Settings => "SETTINGS", Sect::History => "HISTORY", Sect::Search => "" }
    }
    /// Most rows a section shows while something is typed.
    fn cap(self) -> usize { match self { Sect::Songs => 5, Sect::Library => 6, _ => 8 } }
}

#[derive(Clone)]
enum Hint { None, Keys(&'static str), Text(String) }

#[derive(Clone)]
struct Item {
    /// Stable across renames, so "recent" survives; only `recent` items are remembered.
    id: String,
    sect: Sect,
    icon: &'static str,
    title: String,
    /// Extra words that match but are not shown (and not highlighted).
    keywords: String,
    hint: Hint,
    recent: bool,
    /// `None` = a row that only says something ("Searching…").
    run: Option<Rc<dyn Fn()>>,
    /// A song: Shift+Enter plays it next, Ctrl+Enter queues it, Alt+Enter starts its radio.
    track: Option<Track>,
}

impl Item {
    fn new(id: impl Into<String>, sect: Sect, icon: &'static str, title: impl Into<String>) -> Self {
        Item { id: id.into(), sect, icon, title: title.into(), keywords: String::new(), hint: Hint::None, recent: false, run: None, track: None }
    }
    fn run(mut self, f: impl Fn() + 'static) -> Self { self.run = Some(Rc::new(f)); self }
    fn keys(mut self, k: &'static str) -> Self { self.hint = Hint::Keys(k); self }
    fn text(mut self, t: impl Into<String>) -> Self { self.hint = Hint::Text(t.into()); self }
    fn kw(mut self, k: impl Into<String>) -> Self { self.keywords = k.into(); self }
    fn recent(mut self) -> Self { self.recent = true; self }
    fn track(mut self, t: Track) -> Self { self.track = Some(t); self }
}

enum Row { Head(&'static str), Item(Item, Vec<usize>) }

/// What the library holds, read once when the palette opens.
struct Library { songs: Vec<Track>, playlists: Vec<PlaylistInfo>, artists: Vec<(String, String, Option<String>)> }

fn load_library(ctx: &Ctx) -> Library {
    let mut songs: Vec<Track> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let rows = [ctx.store.liked_songs(2000, 0), ctx.store.recently_played(300, 0), ctx.store.most_played()];
    for r in rows.into_iter().flatten().flatten() { if seen.insert(r.track.video_id.clone()) { songs.push(r.track); } }
    Library { songs, playlists: ctx.store.playlists().unwrap_or_default(), artists: ctx.store.followed_artists().unwrap_or_default() }
}

fn page_icon(key: &str) -> &'static str {
    match key {
        "home" => "go-home-symbolic", "search" => "system-search-symbolic", "charts" => "view-sort-descending-symbolic", "favorites" => crate::icons::HEART_ON,
        "recent" => "document-open-recent-symbolic", "most_played" => "media-playlist-repeat-symbolic", "playlists" => "view-list-symbolic",
        "following" => "avatar-default-symbolic", _ => "folder-download-symbolic",
    }
}

fn on_off(b: bool) -> String { (if b { "On" } else { "Off" }).into() }

/// Pages, playback and settings: everything that is not about a particular song or playlist.
fn commands(ctx: &Rc<Ctx>, show: &Rc<dyn Fn(Root)>) -> Vec<Item> {
    let mut v = Vec::new();
    let (shuffle, repeat, endless, radio_click, sleeping, mode) = {
        let s = ctx.snap.borrow();
        let s = s.as_ref();
        (s.is_some_and(|s| s.shuffle), s.map_or(Repeat::Off, |s| s.repeat), s.is_some_and(|s| s.endless), s.is_some_and(|s| s.auto_radio), s.is_some_and(|s| s.sleep.is_some()), crate::theme::skin().map(|k| k.mode()))
    };
    let page = |key: &str, root: Root| { let show = show.clone(); Item::new(format!("page:{key}"), Sect::Pages, page_icon(key), ctx.word(key)).recent().run(move || show(root)) };
    v.push(page("home", Root::Home));
    v.push(page("search", Root::Search).keys("Ctrl + F"));
    v.push(page("charts", Root::Charts));
    for (key, lib) in [("favorites", Lib::Favorites), ("recent", Lib::Recent), ("most_played", Lib::MostPlayed), ("playlists", Lib::Playlists), ("following", Lib::Following), ("downloads", Lib::Downloads)] { v.push(page(key, Root::Lib(lib))); }
    let nav = |id: &str, icon: &'static str, title: &str, n: Nav| { let ctx = ctx.clone(); Item::new(id, Sect::Pages, icon, title).recent().run(move || ctx.navigate(n.clone())) };
    v.push(nav("page:now", "media-playback-start-symbolic", "Now playing", Nav::NowPlaying));
    v.push(nav("page:queue", "view-list-ordered-symbolic", "Queue", Nav::Queue));
    v.push(nav("page:effects", "audio-volume-high-symbolic", "Effects", Nav::Effects).kw("equalizer eq reverb speed pitch"));
    v.push(nav("page:mini", "window-new-symbolic", "Mini player", Nav::MiniPlayer));
    v.push(nav("page:prefs", "emblem-system-symbolic", "Preferences", Nav::Settings).keys("Ctrl + ,").kw("settings options"));
    { let ctx = ctx.clone(); v.push(Item::new("cmd:shortcuts", Sect::Settings, "input-keyboard-symbolic", "Keyboard shortcuts").keys("F1").recent().run(move || crate::shortcuts::show(&ctx))); }

    let send = |id: &str, icon: &'static str, title: &str, c: fn() -> Command| { let ctx = ctx.clone(); Item::new(id, Sect::Playback, icon, title).recent().run(move || ctx.engine.send(c())) };
    v.push(send("cmd:pause", "media-playback-start-symbolic", "Play / pause", || Command::TogglePause).keys("Space").kw("resume stop"));
    v.push(send("cmd:next", "media-skip-forward-symbolic", "Next song", || Command::Next).keys("Ctrl + →").kw("skip"));
    v.push(send("cmd:prev", "media-skip-backward-symbolic", "Previous song", || Command::Prev).keys("Ctrl + ←").kw("back"));
    { let ctx = ctx.clone(); v.push(Item::new("cmd:shuffle", Sect::Playback, "media-playlist-shuffle-symbolic", "Shuffle").text(on_off(shuffle)).recent().run(move || ctx.engine.send(Command::SetShuffle(!shuffle)))); }
    { let ctx = ctx.clone(); v.push(Item::new("cmd:repeat", Sect::Playback, "media-playlist-repeat-symbolic", "Repeat").text(match repeat { Repeat::Off => "Off", Repeat::All => "All", Repeat::One => "One" }).kw("loop").recent().run(move || ctx.engine.send(Command::SetRepeat(repeat.cycle())))); }
    { let ctx = ctx.clone(); v.push(Item::new("cmd:endless", Sect::Playback, crate::icons::RADIO, "Endless radio").text(on_off(endless)).kw("autoplay").recent().run(move || ctx.engine.send(Command::SetEndless(!endless)))); }
    { let ctx = ctx.clone(); v.push(Item::new("cmd:radio-click", Sect::Playback, crate::icons::RADIO, "Radio on click").text(on_off(radio_click)).recent().run(move || ctx.set_auto_radio(!radio_click))); }
    for (id, label, m) in [("sleep:15", "15 minutes", SleepMode::After(Duration::from_secs(900))), ("sleep:30", "30 minutes", SleepMode::After(Duration::from_secs(1800))), ("sleep:60", "1 hour", SleepMode::After(Duration::from_secs(3600))), ("sleep:end", "end of this song", SleepMode::EndOfTrack)] {
        let ctx = ctx.clone();
        v.push(Item::new(id, Sect::Playback, "alarm-symbolic", format!("Sleep timer: {label}")).kw("stop later bedtime").recent().run(move || { ctx.engine.send(Command::SetSleepTimer(Some(m))); ctx.toast(&format!("Playback will pause after {label}")); }));
    }
    if sleeping { let ctx = ctx.clone(); v.push(Item::new("sleep:off", Sect::Playback, "alarm-symbolic", "Sleep timer: turn off").kw("cancel").recent().run(move || ctx.engine.send(Command::SetSleepTimer(None)))); }
    for (m, name) in crate::theme::Mode::ALL {
        let ctx = ctx.clone();
        let mut it = Item::new(format!("style:{}", m.key()), Sect::Settings, "preferences-desktop-appearance-symbolic", format!("Style: {name}")).kw("theme appearance colour color light dark").recent();
        if mode == Some(m) { it = it.text("Current"); }
        v.push(it.run(move || {
            let Some(skin) = crate::theme::skin() else { return };
            if skin.mode() == m { return; }
            match skin.set_mode(&ctx, m) { Ok(()) => { let _ = ctx.store.set("color_mode", &m.key().to_string()); } Err(e) => ctx.toast(&format!("Theme error: {e}")) }
        }));
    }
    v
}

/// Actions on whatever is playing right now.
fn current(ctx: &Rc<Ctx>) -> Vec<Item> {
    let Some(t) = ctx.current_track() else { return Vec::new() };
    let mut v = Vec::new();
    let liked = ctx.store.is_liked(&t.video_id);
    { let (ctx, t) = (ctx.clone(), t.clone()); v.push(Item::new("cur:like", Sect::Current, crate::icons::heart(!liked), if liked { format!("Unlike “{}”", t.title) } else { format!("Like “{}”", t.title) }).keys("Ctrl + L").kw("favorite love heart").run(move || ctx.toggle_like(&t))); }
    if let Some(a) = t.artists.first().cloned() { let ctx = ctx.clone(); v.push(Item::new("cur:artist", Sect::Current, "avatar-default-symbolic", format!("Search for {a}")).kw("artist more by").run(move || ctx.navigate(Nav::SearchFor(a.clone())))); }
    if let Some(al) = t.album.clone() { let ctx = ctx.clone(); v.push(Item::new("cur:album", Sect::Current, "media-optical-symbolic", format!("Search for album {al}")).run(move || ctx.navigate(Nav::SearchFor(al.clone())))); }
    v
}

fn library_items(ctx: &Rc<Ctx>, lib: &Library, songs: bool) -> Vec<Item> {
    let mut v = Vec::new();
    if songs {
        let all: Rc<Vec<Track>> = Rc::new(lib.songs.clone());
        for (i, t) in lib.songs.iter().enumerate() {
            let (ctx, all) = (ctx.clone(), all.clone());
            v.push(Item::new(format!("song:{}", t.video_id), Sect::Songs, "audio-x-generic-symbolic", t.title.clone()).text(t.artists.join(", ")).kw(t.artists.join(" ")).track(t.clone())
                .run(move || ctx.play_row(vec![all[i].clone()], 0, QueueKind::Songs)));
        }
    }
    for p in &lib.playlists {
        let (ctx, id) = (ctx.clone(), p.id);
        v.push(Item::new(format!("pl:{id}"), Sect::Library, "view-list-symbolic", p.title.clone()).text(format!("Playlist · {}", p.count)).run(move || ctx.navigate(Nav::Local(id))));
    }
    for (cid, name, _) in &lib.artists {
        let (ctx, cid) = (ctx.clone(), cid.clone());
        v.push(Item::new(format!("artist:{cid}"), Sect::Library, "avatar-default-symbolic", name.clone()).text("Artist").run(move || ctx.navigate(Nav::Artist(cid.clone()))));
    }
    v
}

/// Fuzzy score of `it` for the pattern: the title (with the matched letters), else the hidden keywords; a title that starts with the typed text beats any fuzzy match.
fn score(it: &Item, pat: &Pattern, q: &str, m: &mut Matcher, buf: &mut Vec<char>) -> Option<(u32, Vec<usize>)> {
    let mut idx = Vec::new();
    if let Some(s) = pat.indices(Utf32Str::new(&it.title, buf), m, &mut idx) {
        let prefix = it.title.to_lowercase().starts_with(q);
        idx.sort_unstable();
        idx.dedup();
        return Some((s + if prefix { 10_000 } else { 0 }, idx.into_iter().map(|i| i as usize).collect()));
    }
    if it.keywords.is_empty() { return None; }
    pat.score(Utf32Str::new(&it.keywords, buf), m).map(|s| (s / 2, Vec::new()))
}

/// Which items a prefix mode keeps.
#[derive(Clone, Copy, PartialEq)]
enum Mode { All, Commands, Library }

fn local(ctx: &Rc<Ctx>, show: &Rc<dyn Fn(Root)>, lib: &Library, input: &str) -> Vec<Row> {
    let input = input.trim_start();
    let (mode, term) = match input.chars().next() { Some('>') => (Mode::Commands, &input[1..]), Some('@') => (Mode::Library, &input[1..]), _ => (Mode::All, input) };
    let term = term.trim();
    let empty = term.is_empty();
    let mut items = Vec::new();
    if mode != Mode::Library { items.extend(commands(ctx, show)); items.extend(current(ctx)); }
    if mode != Mode::Commands { items.extend(library_items(ctx, lib, mode == Mode::All && !empty)); }
    let past = if mode == Mode::All { ctx.store.search_history(8).unwrap_or_default() } else { Vec::new() };
    for h in past { let ctx = ctx.clone(); let q = h.clone(); items.push(Item::new(format!("hist:{h}"), Sect::History, "document-open-recent-symbolic", h).text("Search").run(move || ctx.navigate(Nav::SearchFor(q.clone())))); }

    // Recent commands lead an empty palette, and then no longer repeat in their own section.
    let recent_ids: Vec<String> = if empty && mode == Mode::All { ctx.store.get("palette_recent").unwrap_or_default() } else { Vec::new() };
    let mut scored: Vec<(Item, u32, Vec<usize>)> = Vec::new();
    if empty {
        for id in &recent_ids { if let Some(i) = items.iter().position(|it| it.recent && &it.id == id) { let mut it = items.remove(i); it.sect = Sect::Recent; scored.push((it, u32::MAX, Vec::new())); } }
        for it in items { scored.push((it, 0, Vec::new())); }
    } else {
        let pat = Pattern::parse(term, CaseMatching::Smart, Normalization::Smart);
        let (mut m, mut buf, q) = (Matcher::new(Config::DEFAULT), Vec::new(), term.to_lowercase());
        for it in items { if let Some((s, hl)) = score(&it, &pat, &q, &mut m, &mut buf) { scored.push((it, s, hl)); } }
    }

    // Group by section: an empty palette keeps the fixed order, a query puts the section with the best match first.
    let mut groups: Vec<(Sect, Vec<(Item, u32, Vec<usize>)>)> = Vec::new();
    for e in scored { match groups.iter_mut().find(|(s, _)| *s == e.0.sect) { Some((_, g)) => g.push(e), None => groups.push((e.0.sect, vec![e])) } }
    for (s, g) in &mut groups {
        if !empty { g.sort_by_key(|e| std::cmp::Reverse(e.1)); g.truncate(s.cap()); }
        else if *s == Sect::History { g.truncate(4); }
        else if *s == Sect::Recent { g.truncate(5); }
    }
    if empty { groups.sort_by_key(|(s, _)| *s); } else { groups.sort_by_key(|(s, g)| (std::cmp::Reverse(g.first().map_or(0, |e| e.1)), *s)); }

    let mut out = Vec::new();
    for (s, g) in groups {
        if !s.title().is_empty() { out.push(Row::Head(s.title())); }
        for (it, _, hl) in g { out.push(Row::Item(it, hl)); }
    }
    if !empty && mode == Mode::All {
        let (ctx, t) = (ctx.clone(), term.to_string());
        out.push(Row::Item(Item::new("search", Sect::Search, "system-search-symbolic", format!("Search for “{term}”")).keys("Enter").run(move || ctx.navigate(Nav::SearchFor(t.clone()))), Vec::new()));
    }
    if !out.iter().any(|r| matches!(r, Row::Item(..))) { out.push(Row::Item(Item::new("none", Sect::Search, "dialog-information-symbolic", "No matches"), Vec::new())); }
    out
}

/// A single row that only says something.
fn say(title: String, hint: &str) -> Vec<Row> { vec![Row::Item(Item::new("info", Sect::Search, "system-search-symbolic", title).text(hint), Vec::new())] }

/// Title with the letters the query matched in bold.
fn markup(title: &str, hl: &[usize]) -> String {
    if hl.is_empty() { return glib::markup_escape_text(title).to_string(); }
    let mut s = String::new();
    for (i, c) in title.chars().enumerate() {
        let esc = glib::markup_escape_text(&c.to_string());
        if hl.binary_search(&i).is_ok() { s.push_str(&format!("<b>{esc}</b>")); } else { s.push_str(&esc); }
    }
    s
}

fn hint_widget(h: &Hint) -> Option<gtk::Widget> {
    match h {
        Hint::None => None,
        Hint::Text(t) => Some(gtk::Label::builder().label(t).css_classes(["dim"]).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(28).build().upcast()),
        Hint::Keys(k) => {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            for part in k.split(" + ") { b.append(&gtk::Label::builder().label(part).css_classes(["keycap"]).build()); }
            Some(b.upcast())
        }
    }
}

const SONG_KEYS: &str = "↵ Play    ⇧↵ Play next    Ctrl ↵ Add to queue    Alt ↵ Radio";

/// Close and forget `d` at once: `closed` only fires after the close animation, and until then a re-open would find the old dialog and "toggle" it instead of opening.
fn close(d: &adw::Dialog) {
    OPEN.with(|o| { if o.borrow().as_ref() == Some(d) { o.borrow_mut().take(); } });
    d.close();
}

/// Open the palette, or close it when it is already open.
pub fn toggle(ctx: &Rc<Ctx>, show: &Rc<dyn Fn(Root)>, initial: &str) {
    // A dialog that is no longer on screen is stale (its `closed` never ran): open a fresh one.
    if let Some(d) = OPEN.with(|o| o.borrow_mut().take()) { if d.is_mapped() { d.close(); return; } }
    let dialog = adw::Dialog::builder().follows_content_size(true).build();
    dialog.add_css_class("palette");
    let search = gtk::SearchEntry::builder().placeholder_text("Search…    /  songs    >  commands    @  library").hexpand(true).build();
    search.set_margin_start(12);
    search.set_margin_end(12);
    search.set_margin_top(12);
    search.set_margin_bottom(8);
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::Single).css_classes(["navigation-sidebar"]).build();
    list.set_margin_start(6);
    list.set_margin_end(6);
    list.set_margin_bottom(8);
    list.update_property(&[gtk::accessible::Property::Label("Results")]);
    let scroll = gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).min_content_height(120).max_content_height(380).propagate_natural_height(true).build();
    let foot = gtk::Label::builder().label(SONG_KEYS).css_classes(["palette-foot"]).xalign(0.0).visible(false).build();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    col.set_width_request(560);
    col.append(&search);
    col.append(&scroll);
    col.append(&foot);
    dialog.set_child(Some(&col));
    dialog.set_focus(Some(&search));

    // One entry per list row (`None` for a section header); selectable rows are those with something to run.
    let shown: Rc<RefCell<Vec<Option<Item>>>> = Rc::default();
    let render = Rc::new({
        let (list, shown, search) = (list.clone(), shown.clone(), search.clone());
        move |rows: Vec<Row>| {
            while let Some(c) = list.first_child() { list.remove(&c); }
            let mut items = Vec::with_capacity(rows.len());
            let mut count = 0;
            for r in rows {
                match r {
                    Row::Head(t) => {
                        list.append(&gtk::ListBoxRow::builder().child(&gtk::Label::builder().label(t).xalign(0.0).css_classes(["palette-head"]).build()).selectable(false).activatable(false).build());
                        items.push(None);
                    }
                    Row::Item(e, hl) => {
                        let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                        line.set_margin_top(8);
                        line.set_margin_bottom(8);
                        line.set_margin_start(10);
                        line.set_margin_end(10);
                        line.append(&gtk::Image::builder().icon_name(e.icon).css_classes(["dim"]).build());
                        line.append(&gtk::Label::builder().use_markup(true).label(markup(&e.title, &hl)).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
                        if let Some(h) = hint_widget(&e.hint) { line.append(&h); }
                        let live = e.run.is_some();
                        list.append(&gtk::ListBoxRow::builder().child(&line).selectable(live).activatable(live).build());
                        if live { count += 1; }
                        items.push(Some(e));
                    }
                }
            }
            // A typed query lands on its best match.
            if let Some(r) = (0..).map_while(|i| list.row_at_index(i)).find(|r| r.is_selectable()) { list.select_row(Some(&r)); }
            *shown.borrow_mut() = items;
            search.announce(&format!("{count} results"), gtk::AccessibleAnnouncementPriority::Low);
        }
    });
    // The Enter-modifier hints show only on a song.
    list.connect_selected_rows_changed({
        let (shown, foot) = (shown.clone(), foot.clone());
        move |l| { let song = l.selected_row().and_then(|r| shown.borrow().get(r.index() as usize).cloned().flatten()).is_some_and(|e| e.track.is_some()); foot.set_visible(song); }
    });

    let lib = Rc::new(load_library(ctx));
    // `/ query` searches YouTube Music for songs (after a short pause, never per keystroke).
    let remote_seq = Rc::new(Cell::new(0u32));
    let refill = Rc::new({
        let (ctx, show, render, search, seq, lib) = (ctx.clone(), show.clone(), render.clone(), search.clone(), remote_seq.clone(), lib.clone());
        move || {
            let q = search.text().to_string();
            seq.set(seq.get().wrapping_add(1));
            let Some(rest) = q.trim_start().strip_prefix('/') else { render(local(&ctx, &show, &lib, &q)); return; };
            let term = rest.trim().to_string();
            if term.is_empty() { render(say("Type a song to search for".into(), "/ song")); return; }
            render(say(format!("Searching for “{term}”…"), ""));
            let (ctx, render, seq, mine) = (ctx.clone(), render.clone(), seq.clone(), seq.get());
            glib::timeout_add_local_once(Duration::from_millis(350), move || {
                if seq.get() != mine { return; }
                let (ytm, t2, ctx2) = (ctx.ytm.clone(), term.clone(), ctx.clone());
                ctx.spawn(async move { ytm.search(&t2, SearchKind::Songs).await.map(|r| r.songs).map_err(|e| format!("{e:#}")) }, move |res| {
                    if seq.get() != mine { return; }
                    render(match res {
                        Err(e) => say("Search failed".into(), &e),
                        Ok(songs) if songs.is_empty() => say(format!("No songs for “{term}”"), ""),
                        Ok(songs) => {
                            let songs: Rc<Vec<Track>> = Rc::new(songs.into_iter().take(10).collect());
                            (0..songs.len()).map(|i| { let (ctx, songs) = (ctx2.clone(), songs.clone()); Row::Item(Item::new(format!("rsong:{}", songs[i].video_id), Sect::Songs, "audio-x-generic-symbolic", songs[i].title.clone()).text(songs[i].artists.join(", ")).track(songs[i].clone()).run(move || ctx.play_row(songs.to_vec(), i, QueueKind::Songs)), Vec::new()) }).collect()
                        }
                    });
                });
            });
        }
    });
    search.set_text(initial);
    search.set_position(-1);
    refill();
    search.connect_search_changed({ let refill = refill.clone(); move |_| refill() });

    // Enter plays / opens the row; with Shift, Ctrl or Alt a song is queued next, at the end, or as a radio instead.
    let run_at: Rc<dyn Fn(usize, gdk::ModifierType)> = Rc::new({
        let (ctx, dialog, shown) = (ctx.clone(), dialog.clone(), shown.clone());
        move |i: usize, mods: gdk::ModifierType| {
            let Some(Some(item)) = shown.borrow().get(i).cloned() else { return };
            let Some(run) = item.run.clone() else { return };
            close(&dialog);
            let ctx = ctx.clone();
            glib::idle_add_local_once(move || {
                if item.recent {
                    let mut ids: Vec<String> = ctx.store.get("palette_recent").unwrap_or_default();
                    ids.retain(|x| x != &item.id);
                    ids.insert(0, item.id.clone());
                    ids.truncate(8);
                    let _ = ctx.store.set("palette_recent", &ids);
                }
                match item.track {
                    Some(t) if mods.contains(gdk::ModifierType::SHIFT_MASK) => { ctx.engine.send(Command::PlayNext(t.clone())); ctx.toast(&format!("“{}” plays next", t.title)); }
                    Some(t) if mods.contains(gdk::ModifierType::CONTROL_MASK) => { ctx.engine.send(Command::Enqueue(vec![t.clone()])); ctx.toast(&format!("Added “{}” to the queue", t.title)); }
                    Some(t) if mods.contains(gdk::ModifierType::ALT_MASK) => ctx.play_one(t),
                    _ => run(),
                }
            });
        }
    });
    list.connect_row_activated({ let run_at = run_at.clone(); move |_, r| run_at(r.index() as usize, gdk::ModifierType::empty()) });

    // The caret stays in the entry while the keys below move the highlight over the selectable rows.
    let select = Rc::new({
        let (list, search) = (list.clone(), search.clone());
        move |row: gtk::ListBoxRow| { list.select_row(Some(&row)); row.grab_focus(); search.grab_focus(); search.set_position(-1); }
    });
    let step = Rc::new({
        let (list, select) = (list.clone(), select.clone());
        move |steps: i32| {
            let n = { let mut c = 0; while list.row_at_index(c).is_some() { c += 1; } c };
            let dir = steps.signum();
            let live = |i: i32| list.row_at_index(i).is_some_and(|r| r.is_selectable());
            // The next selectable row from `from` in direction `dir`, if any.
            let find = |from: i32| { let mut i = from; loop { i += dir; if i < 0 || i >= n { return None; } if live(i) { return Some(i); } } };
            let mut cur = list.selected_row().map_or(if dir > 0 { -1 } else { n }, |r| r.index());
            for _ in 0..steps.abs() {
                match find(cur) {
                    Some(i) => cur = i,
                    // A single arrow wraps round; a page stops at the end.
                    None if steps.abs() == 1 => { if let Some(i) = find(if dir > 0 { -1 } else { n }) { cur = i; } }
                    None => break,
                }
            }
            if let Some(r) = list.row_at_index(cur).filter(|r| r.is_selectable()) { select(r); }
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let (list, dialog, run_at, step, select) = (list.clone(), dialog.clone(), run_at.clone(), step.clone(), select.clone());
        move |_, key, _, state| {
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            match key {
                gdk::Key::Escape => close(&dialog),
                gdk::Key::Return | gdk::Key::KP_Enter => { if let Some(r) = list.selected_row() { run_at(r.index() as usize, state); } }
                gdk::Key::Down | gdk::Key::Tab => step(1),
                gdk::Key::Up | gdk::Key::ISO_Left_Tab => step(-1),
                gdk::Key::Page_Down => step(5),
                gdk::Key::Page_Up => step(-5),
                gdk::Key::Home if ctrl => { if let Some(r) = (0..).map_while(|i| list.row_at_index(i)).find(|r| r.is_selectable()) { select(r); } }
                gdk::Key::End if ctrl => { if let Some(r) = (0..).map_while(|i| list.row_at_index(i)).filter(|r| r.is_selectable()).last() { select(r); } }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    });
    search.add_controller(keys);
    dialog.connect_closed(|d| OPEN.with(|o| { if o.borrow().as_ref() == Some(d) { o.borrow_mut().take(); } }));
    OPEN.with(|o| *o.borrow_mut() = Some(dialog.clone()));
    dialog.present(ctx.toasts.root().as_ref());
}

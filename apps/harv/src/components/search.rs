//! The themed search page (`page.search`). One component, configured by the node's props (read in [`Opts::from`]).
//! The built-in page (`pages::search_classic`) stays as it was; this module reuses its helpers (`shell`, `grid`, cards,
//! `MOODS`, search history, `Nav::SearchFor`) and adds lyric search ("Play from line") plus the themed result layouts.

use crate::ctx::{Ctx, Nav};
use crate::pages::{self, MOODS};
use crate::rows::*;
use adw::prelude::*;
use gtk::{gdk, glib, pango};
use player_core::{Command, QueueKind};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;
use store_core::LyricHit;
use theme_core::{Accent, Node};
use ytm_core::{Mood, SearchKind, SearchResults, Track};

/// Network results of one committed query: songs/albums/artists/playlists/videos plus podcasts.
type Net = (SearchResults, Vec<ytm_core::PodcastRef>);

/// Props read from the theme node (`[page.search] main = [ { c = "search", ... } ]`). Defaults = the built-in look.
#[derive(Clone)]
struct Opts {
    box_w: i32,
    /// `hint = "esc" | "none"`: ESC hint at the right of the field (Escape clears it either way).
    esc: bool,
    placeholder: String,
    /// `chips = ["all", "songs", "playlists=Tapes", …]` (`id=Label` overrides the label). Absent = the classic kinds; `[]` = none.
    chips: Vec<Chip>,
    /// `counts = true`: the lyrics chip reads "Lyrics · n" while typing.
    counts: bool,
    /// `lyrics = true | false`: lyric search on/off (gates the lyrics chip and lyric matches everywhere).
    lyrics: bool,
    /// `results = "list" | "columns"`.
    results: String,
    /// `side = "artists" | "playlists" | "albums" | "none"`: the right-hand column of `columns`.
    side: String,
    side_w: i32,
    /// `hit = "row" | "card"`: lyric matches as rows with a Play button, or cards with highlighted words.
    hit_card: bool,
    /// The fixed theme accent (colours the highlighted words in lyric matches); None when the accent follows the cover.
    accent: Option<[f32; 3]>,
}

#[derive(Clone)]
struct Chip { id: String, label: String, btn: Option<gtk::ToggleButton> }

impl Opts {
    fn from(ctx: &Rc<Ctx>, n: &Node) -> Opts {
        let theme = ctx.theme();
        let specs: Vec<String> = if n.props.contains_key("chips") { n.strs("chips") } else { ["songs", "albums", "artists", "playlists", "videos", "podcasts"].iter().map(|s| s.to_string()).collect() };
        let lyrics = n.bool_or("lyrics", true);
        let chips = specs.iter().filter_map(|spec| {
            let (id, label) = match spec.split_once('=') { Some((a, b)) => (a.trim().to_string(), Some(b.trim().to_string())), None => (spec.trim().to_string(), None) };
            if !matches!(id.as_str(), "all" | "songs" | "albums" | "artists" | "playlists" | "videos" | "podcasts" | "lyrics") { eprintln!("theme: search chip `{id}` is unknown"); return None; }
            if id == "lyrics" && !lyrics { return None; }
            let label = label.unwrap_or_else(|| match id.as_str() {
                "all" => "All".into(), "songs" => "Songs".into(), "albums" => "Albums".into(), "artists" => "Artists".into(),
                "playlists" => ctx.word("playlists"), "videos" => "Videos".into(), "podcasts" => "Podcasts".into(), _ => "Lyrics".into(),
            });
            Some(Chip { id, label, btn: None })
        }).collect();
        Opts {
            box_w: n.int_or("box_width", 720) as i32,
            esc: n.str_or("hint", "none") == "esc",
            placeholder: n.str_or("placeholder", "Songs, albums, artists, playlists").to_string(),
            chips,
            counts: n.bool_or("counts", false),
            lyrics,
            results: n.str_or("results", "list").to_string(),
            side: n.str_or("side", "artists").to_string(),
            side_w: n.int_or("side_w", 320) as i32,
            hit_card: n.str_or("hit", "row") == "card",
            accent: match theme.colors.accent { Accent::Fixed(c) => Some(c.rgb_f32()), Accent::Cover => None },
        }
    }
}

/// What a chip searches on YouTube Music. `all` = songs, albums, artists and playlists (lyrics are local, see `has_lyrics`).
/// `lyrics` also fetches the side column's kind, so the right-hand list has data beside the lyric matches.
fn net_kinds(o: &Opts, id: &str) -> Vec<SearchKind> {
    let side = match o.side.as_str() { "artists" => Some(SearchKind::Artists), "playlists" => Some(SearchKind::Playlists), "albums" => Some(SearchKind::Albums), _ => None };
    match id {
        "all" => vec![SearchKind::Songs, SearchKind::Albums, SearchKind::Artists, SearchKind::Playlists],
        "songs" => vec![SearchKind::Songs],
        "albums" => vec![SearchKind::Albums],
        "artists" => vec![SearchKind::Artists],
        "playlists" => vec![SearchKind::Playlists],
        "videos" => vec![SearchKind::Videos],
        "lyrics" => side.into_iter().collect(),
        _ => vec![],
    }
}

/// Chips whose results include lyric matches.
fn has_lyrics(id: &str) -> bool { id == "all" || id == "lyrics" }

/// Everything the page keeps between events.
struct St {
    ctx: Rc<Ctx>,
    o: Opts,
    entry: gtk::Entry,
    suggest: gtk::ListBox,
    results: gtk::Box,
    chips: RefCell<Vec<Chip>>,
    /// Active chip id ("all" when the theme has no chips).
    chip: RefCell<String>,
    /// The entry text as typed (trimmed), updated on every keystroke.
    live: RefCell<String>,
    /// The query last sent to YouTube Music (Enter, a chip, a suggestion or a history item).
    committed: RefCell<String>,
    /// Lyric matches for `live` (local index, refreshed after a short pause in typing).
    hits: RefCell<Vec<LyricHit>>,
    net: RefCell<Option<Rc<Net>>>,
    net_err: RefCell<Option<String>>,
    busy: Cell<bool>,
    /// Bumped on every keystroke: the debounced refresh only runs for the latest one.
    seq: Cell<u32>,
    /// Bumped per network request: an answer for an older request is dropped.
    net_seq: Cell<u32>,
}

/// Build the search page from the theme's `page.search.main[0]` node.
pub fn themed(ctx: &Rc<Ctx>, initial: Option<&str>, node: &Node) -> adw::NavigationPage {
    let o = Opts::from(ctx, node);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    col.set_margin_top(28);
    col.set_margin_start(36);
    col.set_margin_end(36);

    // ── the field: glyph · entry · optional ESC hint, a fixed width (box_width) so it doesn't stretch
    let field = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    field.add_css_class("s-box");
    field.set_size_request(o.box_w, -1);
    field.set_halign(gtk::Align::Start);
    field.set_hexpand(false);
    let glyph = gtk::Image::from_icon_name("system-search-symbolic");
    glyph.set_pixel_size(18);
    glyph.add_css_class("s-glyph");
    let entry = gtk::Entry::builder().placeholder_text(&o.placeholder).has_frame(false).hexpand(true).css_classes(["s-entry"]).build();
    field.append(&glyph);
    field.append(&entry);
    if o.esc { field.append(&gtk::Label::builder().label("ESC").css_classes(["s-hint"]).build()); }
    col.append(&field);

    let suggest = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["s-suggest", "suggest-list"]).visible(false).build();
    let results = gtk::Box::new(gtk::Orientation::Vertical, 22);
    results.set_margin_top(26);

    let chip0 = o.chips.first().map(|c| c.id.clone()).unwrap_or_else(|| "all".into());
    let st = Rc::new(St {
        ctx: ctx.clone(), o: o.clone(), entry: entry.clone(), suggest: suggest.clone(), results: results.clone(),
        chips: RefCell::new(vec![]), chip: RefCell::new(chip0.clone()), live: RefCell::default(), committed: RefCell::default(),
        hits: RefCell::default(), net: RefCell::default(), net_err: RefCell::default(), busy: Cell::new(false), seq: Cell::new(0), net_seq: Cell::new(0),
    });

    // ── filter chips (drawn per theme: `.s-chip`, `.s-chip:checked`)
    if !o.chips.is_empty() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("s-chips");
        row.set_margin_top(16);
        let mut first: Option<gtk::ToggleButton> = None;
        let mut made = vec![];
        for c in &o.chips {
            let b = gtk::ToggleButton::builder().label(&c.label).css_classes(["s-chip"]).build();
            match &first { Some(f) => b.set_group(Some(f)), None => { b.set_active(c.id == chip0); first = Some(b.clone()); } }
            row.append(&b);
            made.push(Chip { id: c.id.clone(), label: c.label.clone(), btn: Some(b) });
        }
        *st.chips.borrow_mut() = made;
        for c in st.chips.borrow().iter() {
            let (st2, id, b) = (st.clone(), c.id.clone(), c.btn.clone().unwrap());
            b.clone().connect_toggled(move |b| {
                if !b.is_active() { return; }
                *st2.chip.borrow_mut() = id.clone();
                if st2.committed.borrow().is_empty() { render(&st2); } else { run_net(&st2); }
            });
        }
        col.append(&row);
    }
    col.append(&suggest);
    col.append(&results);

    // ── typing: lyric matches and suggestions follow the keystrokes (after a short pause); YouTube Music waits for Enter
    entry.connect_changed({
        let st = st.clone();
        move |e| {
            *st.live.borrow_mut() = e.text().trim().to_string();
            let n = st.seq.get() + 1;
            st.seq.set(n);
            let st = st.clone();
            glib::timeout_add_local_once(Duration::from_millis(160), move || { if st.seq.get() == n { on_live(&st); } });
        }
    });
    entry.connect_activate({ let st = st.clone(); move |_| commit(&st) });
    {
        let keys = gtk::EventControllerKey::new();
        let (e, st) = (entry.clone(), st.clone());
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape && !e.text().is_empty() { e.set_text(""); let _ = &st; glib::Propagation::Stop } else { glib::Propagation::Proceed }
        });
        entry.add_controller(keys);
    }
    {
        let focus = gtk::EventControllerFocus::new();
        let field = field.clone();
        let field2 = field.clone();
        focus.connect_enter(move |_| field.add_css_class("focused"));
        focus.connect_leave(move |_| field2.remove_css_class("focused"));
        entry.add_controller(focus);
    }
    suggest.connect_row_activated({
        let (st, entry) = (st.clone(), entry.clone());
        move |l, row| { l.set_visible(false); entry.set_text(&row.widget_name()); commit(&st); }
    });

    // ── moods: fetched once per session, shared with the built-in page
    if MOODS.with(|m| m.borrow().is_none()) {
        let (ytm, st2) = (ctx.ytm.clone(), st.clone());
        ctx.spawn(async move { ytm.moods().await.unwrap_or_default() }, move |list| {
            MOODS.with(|m| *m.borrow_mut() = Some(list));
            if st2.live.borrow().is_empty() { render(&st2); }
        });
    }

    render(&st);
    if let Some(q) = initial { entry.set_text(q); entry.set_position(-1); commit(&st); }
    let page = pages::shell(&ctx.word("search"), &col, None);
    let e = entry.clone();
    page.connect_showing(move |_| { e.grab_focus(); e.set_position(-1); });
    page
}

/// Enter / chip / suggestion / history: send the query to YouTube Music and remember it.
fn commit(st: &Rc<St>) {
    let q = st.live.borrow().clone();
    st.suggest.set_visible(false);
    if q.is_empty() { *st.committed.borrow_mut() = String::new(); st.busy.set(false); render(st); return; }
    *st.committed.borrow_mut() = q.clone();
    let _ = st.ctx.store.add_search(&q);
    *st.hits.borrow_mut() = lyric_hits(st, &q);
    run_net(st);
}

fn lyric_hits(st: &St, q: &str) -> Vec<LyricHit> {
    if !st.o.lyrics { return vec![]; }
    st.ctx.store.search_lyrics(q, 12).unwrap_or_default()
}

/// Debounced keystroke handler: lyric matches for what is typed now, suggestions, redraw.
fn on_live(st: &Rc<St>) {
    let q = st.live.borrow().clone();
    if q.is_empty() { st.suggest.set_visible(false); render(st); return; }
    *st.hits.borrow_mut() = lyric_hits(st, &q);
    if q == *st.committed.borrow() { st.suggest.set_visible(false); } else { suggestions(st, q); }
    render(st);
}

fn suggestions(st: &Rc<St>, q: String) {
    let (ytm, st2, q2) = (st.ctx.ytm.clone(), st.clone(), q.clone());
    st.ctx.spawn(async move { ytm.suggest(&q2).await.unwrap_or_default() }, move |list| {
        if *st2.live.borrow() != q || *st2.committed.borrow() == q { return; } // superseded, or already searched
        while let Some(c) = st2.suggest.first_child() { st2.suggest.remove(&c); }
        for s in list.into_iter().take(6) {
            let row = gtk::ListBoxRow::builder().child(&gtk::Label::builder().label(&s).xalign(0.0).margin_top(8).margin_bottom(8).margin_start(12).build()).build();
            row.set_widget_name(&s);
            st2.suggest.append(&row);
        }
        st2.suggest.set_visible(st2.suggest.first_child().is_some());
    });
}

/// Search YouTube Music for the committed query with the active chip's kinds.
fn run_net(st: &Rc<St>) {
    let q = st.committed.borrow().clone();
    let chip = st.chip.borrow().clone();
    let kinds = net_kinds(&st.o, &chip);
    let podcasts = chip == "podcasts";
    *st.net.borrow_mut() = None;
    *st.net_err.borrow_mut() = None;
    if q.is_empty() || (kinds.is_empty() && !podcasts) { st.busy.set(false); render(st); return; }
    st.busy.set(true);
    let n = st.net_seq.get() + 1;
    st.net_seq.set(n);
    render(st);
    let ytm = st.ctx.ytm.clone();
    let st2 = st.clone();
    st.ctx.spawn(async move {
        let mut set = tokio::task::JoinSet::new();
        for k in kinds {
            let (y, q) = (ytm.clone(), q.clone());
            set.spawn(async move { (k, y.search(&q, k).await) });
        }
        let mut out = SearchResults::default();
        let mut err: Option<String> = None;
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((k, Ok(r))) => merge(&mut out, k, r),
                Ok((_, Err(e))) => { err.get_or_insert(format!("{e:#}")); }
                Err(e) => { err.get_or_insert(e.to_string()); }
            }
        }
        let pods = if podcasts {
            match ytm.search_podcasts(&q).await { Ok(p) => p, Err(e) => { err.get_or_insert(format!("{e:#}")); vec![] } }
        } else { vec![] };
        (out, pods, err)
    }, move |(out, pods, err)| {
        if st2.net_seq.get() != n { return; }
        st2.busy.set(false);
        let empty = out.songs.is_empty() && out.videos.is_empty() && out.albums.is_empty() && out.artists.is_empty() && out.playlists.is_empty() && pods.is_empty();
        if empty && err.is_some() { *st2.net_err.borrow_mut() = err; } else { *st2.net.borrow_mut() = Some(Rc::new((out, pods))); }
        render(&st2);
    });
}

fn merge(out: &mut SearchResults, k: SearchKind, r: SearchResults) {
    match k {
        SearchKind::Songs => out.songs = r.songs,
        SearchKind::Videos => out.videos = r.videos,
        SearchKind::Albums => out.albums = r.albums,
        SearchKind::Artists => out.artists = r.artists,
        SearchKind::Playlists => out.playlists = r.playlists,
    }
}

// ── rendering ────────────────────────────────────────────────────────────────────────────────

/// What the results area shows for the current state.
struct Data {
    hits: Vec<LyricHit>,
    net: Option<Rc<Net>>,
    busy: bool,
    err: Option<String>,
    /// A network chip is active but the query isn't committed yet: "press Enter".
    awaiting: bool,
    /// The active chip includes lyric matches (so an empty list there says so).
    lyric_chip: bool,
    query: String,
}

fn render(st: &Rc<St>) {
    let live = st.live.borrow().clone();
    let chip = st.chip.borrow().clone();
    let hit_count = st.hits.borrow().len();
    for c in st.chips.borrow().iter() {
        if let Some(b) = &c.btn {
            let text = if c.id == "lyrics" && st.o.counts && !live.is_empty() { format!("{} · {hit_count}", c.label) } else { c.label.clone() };
            b.set_label(&text);
        }
    }
    if live.is_empty() {
        swap(&st.results, &empty_view(st));
        return;
    }
    let committed = st.committed.borrow().clone();
    let net_chip = !net_kinds(&st.o, &chip).is_empty() || chip == "podcasts";
    let show_net = net_chip && !committed.is_empty() && committed == live;
    let d = Data {
        hits: if has_lyrics(&chip) && st.o.lyrics { st.hits.borrow().clone() } else { vec![] },
        net: if show_net { st.net.borrow().clone() } else { None },
        busy: show_net && st.busy.get(),
        err: if show_net { st.net_err.borrow().clone() } else { None },
        awaiting: net_chip && !show_net,
        lyric_chip: has_lyrics(&chip) && st.o.lyrics,
        query: live,
    };
    swap(&st.results, &layout(st, &d));
}

fn swap(holder: &gtk::Box, child: &gtk::Widget) {
    while let Some(c) = holder.first_child() { holder.remove(&c); }
    holder.append(child);
}

/// Children of a box before its list is filled (the section label only).
fn before_label_count(b: &gtk::Box) -> u32 { let mut n = 0; let mut c = b.first_child(); while let Some(w) = c { n += 1; c = w.next_sibling(); } n }

fn vbox(spacing: i32) -> gtk::Box { gtk::Box::new(gtk::Orientation::Vertical, spacing) }
fn hbox(spacing: i32) -> gtk::Box { gtk::Box::new(gtk::Orientation::Horizontal, spacing) }

/// A section label (upper-cased).
fn label(_st: &St, text: &str) -> gtk::Label {
    gtk::Label::builder().label(text.to_uppercase()).xalign(0.0).css_classes(["s-label"]).build()
}

fn plain(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).ellipsize(pango::EllipsizeMode::End).css_classes(classes.iter().map(|s| s.to_string()).collect::<Vec<_>>()).build()
}

/// Lays the data out per the theme's `results`, then adds the empty / busy / error states.
fn layout(st: &Rc<St>, d: &Data) -> gtk::Widget {
    let net_items = d.net.as_ref().map_or(0, |n| n.0.songs.len() + n.0.videos.len() + n.0.albums.len() + n.0.artists.len() + n.0.playlists.len() + n.1.len());
    if d.hits.is_empty() && net_items == 0 && !d.busy {
        if let Some(e) = &d.err { { let st = st.clone(); return error_state("Search failed", e, Some(Rc::new(move || run_net(&st)))); } }
        if d.awaiting { return status("system-search-symbolic", "Press Enter to search", &format!("“{}”", d.query)); }
        return status("edit-find-symbolic", "No results", &format!("Nothing found for “{}”.", d.query));
    }
    if d.busy && d.hits.is_empty() && net_items == 0 { return spinner_page(); }
    let body = match st.o.results.as_str() {
        "columns" => columns(st, d),
        _ => list(st, d),
    };
    if d.busy { body.append(&spinner_page()); }
    body.upcast()
}

fn list(st: &Rc<St>, d: &Data) -> gtk::Box {
    let b = vbox(22);
    if !d.hits.is_empty() {
        b.append(&label(st, "Lyric matches"));
        for h in &d.hits { b.append(&hit_widget(st, h)); }
    }
    if let Some(net) = &d.net {
        let (r, pods) = (&net.0, &net.1);
        if !r.songs.is_empty() { b.append(&track_rows(st, &r.songs, 40, usize::MAX)); }
        if !r.videos.is_empty() { b.append(&track_rows(st, &r.videos, 40, usize::MAX)); }
        if !r.albums.is_empty() { b.append(&pages::grid(r.albums.iter().map(|a| pages::album_card(&st.ctx, a)).collect())); }
        if !r.artists.is_empty() { b.append(&pages::grid(r.artists.iter().map(|a| pages::artist_card(&st.ctx, a)).collect())); }
        if !r.playlists.is_empty() { b.append(&pages::grid(r.playlists.iter().map(|p| pages::playlist_card(&st.ctx, p)).collect())); }
        if !pods.is_empty() { b.append(&pages::grid(pods.iter().map(|p| pages::podcast_card(&st.ctx, p)).collect())); }
    }
    b
}

/// Two columns: lyric matches (and songs) on the left, a side list (artists / channels / albums) on the right.
fn columns(st: &Rc<St>, d: &Data) -> gtk::Box {
    let row = hbox(32);
    let left = vbox(22);
    left.set_hexpand(true);
    if d.lyric_chip && d.hits.is_empty() {
        left.append(&label(st, "Lyric matches"));
        left.append(&plain("No lyric matches", &["s-row-sub"]));
    }
    if !d.hits.is_empty() {
        left.append(&label(st, "Lyric matches"));
        let list = vbox(0);
        for h in &d.hits { list.append(&hit_widget(st, h)); }
        left.append(&list);
    }
    if let Some(net) = &d.net {
        if !net.0.songs.is_empty() {
            left.append(&label(st, "Songs"));
            left.append(&track_rows(st, &net.0.songs, 44, 8));
        }
    }
    row.append(&left);

    if st.o.side != "none" {
        let right = vbox(0);
        right.set_size_request(st.o.side_w, -1);
        right.set_hexpand(false);
        let title = match st.o.side.as_str() { "playlists" => st.ctx.word("playlists"), "albums" => "Albums".into(), _ => "Artists".into() };
        right.append(&label(st, &title));
        if let Some(net) = &d.net {
            let ctx = &st.ctx;
            let before = before_label_count(&right);
            match st.o.side.as_str() {
                "playlists" => for p in net.0.playlists.iter().take(6) {
                    let (c, id) = (ctx.clone(), p.playlist_id.clone());
                    let b = item_row(ctx, p.thumb.as_deref(), 44, false, &p.title, &p.author, "Open →", "s-side", move || c.navigate(Nav::Playlist(id.clone())));
                    right.append(&b);
                },
                "albums" => for a in net.0.albums.iter().take(6) {
                    let (c, id) = (ctx.clone(), a.browse_id.clone());
                    let sub = a.artist.clone();
                    right.append(&item_row(ctx, a.thumb.as_deref(), 44, false, &a.title, &sub, "", "s-side", move || c.navigate(Nav::Album(id.clone()))));
                },
                _ => for a in net.0.artists.iter().take(6) {
                    let (c, id) = (ctx.clone(), a.channel_id.clone());
                    let n = d.hits.iter().filter(|h| h.artist.split(", ").any(|x| x == a.name)).count();
                    let sub = format!("Artist · {n} {}", if n == 1 { "match" } else { "matches" });
                    right.append(&item_row(ctx, a.thumb.as_deref(), 52, true, &a.name, &sub, "", "s-side", move || c.navigate(Nav::Artist(id.clone()))));
                },
            }
            if before_label_count(&right) == before {
                right.append(&plain("Nothing found", &["s-row-sub"]));
            }
        }
        row.append(&right);
    }
    row
}

/// Lyric match rows (`hit = "row"`) or cards (`hit = "card"`), for every layout.
fn hit_widget(st: &Rc<St>, h: &LyricHit) -> gtk::Widget {
    let ctx = st.ctx.clone();
    if st.o.hit_card {
        let row = hbox(16);
        row.add_css_class("s-hit-card");
        row.append(&art(&ctx, h.thumb.as_deref(), 64));
        let col = vbox(6);
        col.set_hexpand(true);
        let meta = format!("{} · {} · {}", h.title, h.artist, at_str(&ctx, h)).to_uppercase();
        col.append(&plain(&meta, &["s-hit-meta"]));
        let line = gtk::Label::builder().label(highlighted(&h.line, &h.snippet_ranges, st.o.accent)).use_markup(true).xalign(0.0).wrap(true).css_classes(["s-hit-line"]).build();
        col.append(&line);
        row.append(&col);
        row.append(&gtk::Label::builder().label("Play from line ▸").css_classes(["s-hit-cta"]).build());
        let b = gtk::Button::builder().child(&row).css_classes(["flat", "s-hit"]).build();
        let h2 = h.clone();
        b.connect_clicked(move |_| play_from_line(&ctx, &h2));
        b.upcast()
    } else {
        let row = hbox(14);
        row.add_css_class("s-hit-row");
        row.append(&art(&ctx, h.thumb.as_deref(), 56));
        let col = vbox(4);
        col.set_hexpand(true);
        let top = hbox(6);
        top.append(&plain(&h.title, &["s-hit-title"]));
        if !h.artist.is_empty() { top.append(&plain(&format!("· {}", h.artist), &["s-hit-artist"])); }
        col.append(&top);
        col.append(&gtk::Label::builder().label(format!("“{}”", h.line)).xalign(0.0).wrap(true).css_classes(["s-hit-line"]).build());
        row.append(&col);
        row.append(&plain(&format!("AT {}", at_str(&ctx, h)), &["s-hit-at"]));
        let play = gtk::Button::builder().label("▶ Play").valign(gtk::Align::Center).css_classes(["s-hit-play"]).build();
        let h2 = h.clone();
        play.connect_clicked(move |_| play_from_line(&ctx, &h2));
        row.append(&play);
        row.upcast()
    }
}

/// Lyric highlight as Pango markup: the matched words bold (and in the theme accent when it is fixed).
fn highlighted(line: &str, ranges: &[(usize, usize)], accent: Option<[f32; 3]>) -> String {
    let esc = |s: &str| glib::markup_escape_text(s).to_string();
    let color = accent.map(|a| format!(" foreground=\"#{:02x}{:02x}{:02x}\"", (a[0] * 255.0).round() as u8, (a[1] * 255.0).round() as u8, (a[2] * 255.0).round() as u8)).unwrap_or_default();
    let mut out = String::new();
    let mut at = 0;
    for &(s, e) in ranges {
        if s < at || e > line.len() || !line.is_char_boundary(s) || !line.is_char_boundary(e) { continue; }
        out += &esc(&line[at..s]);
        out += &format!("<span weight=\"bold\"{color}>{}</span>", esc(&line[s..e]));
        at = e;
    }
    out += &esc(&line[at..]);
    out
}

/// "AT m:ss" for a lyric line: its time plus the song's lyric offset (what the player will actually do).
fn at_str(ctx: &Ctx, h: &LyricHit) -> String { fmt_time(((h.start_ms as i64 + ctx.lyrics.offset_ms(&h.video_id)).max(0) / 1000) as u32) }

/// Start the song and seek to the line (`start_ms` + the song's lyric offset). `PlayQueue` puts the engine in `Loading`
/// synchronously, so a `Seek` sent right after is kept and applied when the track loads.
fn play_from_line(ctx: &Rc<Ctx>, h: &LyricHit) {
    let t = Track { video_id: h.video_id.clone(), title: h.title.clone(), artists: if h.artist.is_empty() { vec![] } else { vec![h.artist.clone()] }, album: None, duration_s: None, thumb: h.thumb.clone() };
    ctx.play_one(t);
    let secs = (h.start_ms as i64 + ctx.lyrics.offset_ms(&h.video_id)).max(0) as f64 / 1000.0;
    ctx.engine.send(Command::Seek(secs));
}

/// Song rows; a click plays the list from that row.
fn track_rows(st: &Rc<St>, songs: &[Track], px: i32, limit: usize) -> gtk::Box {
    let all = Rc::new(songs.to_vec());
    let b = vbox(0);
    for (i, t) in songs.iter().take(limit).enumerate() {
        let (c, all) = (st.ctx.clone(), all.clone());
        let dur = t.duration_s.map(fmt_time).unwrap_or_default();
        b.append(&item_row(&st.ctx, t.thumb.as_deref(), px, false, &t.title, &t.artist_line(), &dur, "s-track", move || c.play_row((*all).clone(), i, QueueKind::Songs)));
    }
    b
}

/// A generic clickable row: cover, title, subtitle, a right-hand text.
fn item_row(ctx: &Rc<Ctx>, thumb: Option<&str>, px: i32, round: bool, title: &str, sub: &str, right: &str, cls: &str, on_click: impl Fn() + 'static) -> gtk::Button {
    let row = hbox(12);
    let img = art(ctx, thumb, px);
    if round { img.add_css_class("round"); }
    row.append(&img);
    let col = vbox(2);
    col.set_hexpand(true);
    col.set_valign(gtk::Align::Center);
    col.append(&plain(title, &["s-row-title"]));
    if !sub.is_empty() { col.append(&plain(sub, &["s-row-sub"])); }
    row.append(&col);
    if !right.is_empty() { row.append(&gtk::Label::builder().label(right).css_classes(["s-row-right"]).build()); }
    let mut classes = vec!["flat", "s-row"];
    if !cls.is_empty() { classes.push(cls); }
    let b = gtk::Button::builder().child(&row).css_classes(classes).build();
    b.connect_clicked(move |_| on_click());
    b
}

/// The empty query: recent searches, then the moods & genres (in the theme's style).
fn empty_view(st: &Rc<St>) -> gtk::Widget {
    let col = vbox(28);
    let history = st.ctx.store.search_history(12).unwrap_or_default();
    if !history.is_empty() {
        let recent = vbox(4);
        let head = hbox(8);
        let title = label(st, "Recent searches");
        title.set_hexpand(true);
        head.append(&title);
        let clear = gtk::Button::builder().label("Clear").css_classes(["flat", "pill-btn"]).build();
        { let st = st.clone(); clear.connect_clicked(move |_| { let _ = st.ctx.store.clear_search_history(); render(&st); }); }
        head.append(&clear);
        recent.append(&head);
        for q in history {
            let btn = gtk::Button::builder().label(&q).halign(gtk::Align::Start).css_classes(["flat", "history-item", "s-history"]).build();
            btn.child().and_downcast::<gtk::Label>().inspect(|l| l.set_xalign(0.0));
            let (st2, q2) = (st.clone(), q.clone());
            btn.connect_clicked(move |_| { st2.entry.set_text(&q2); commit(&st2); });
            recent.append(&btn);
        }
        col.append(&recent);
    }
    match MOODS.with(|m| m.borrow().clone()) {
        None => col.append(&spinner_page()),
        Some(list) if list.is_empty() => {
            if col.first_child().is_none() { col.append(&status("system-search-symbolic", "Search YouTube Music", "Find songs, albums, artists and playlists.")); }
        }
        Some(list) => col.append(&moods(st, &list)),
    }
    col.upcast()
}

fn moods(st: &Rc<St>, list: &[Mood]) -> gtk::Box {
    let b = vbox(14);
    b.append(&label(st, "Moods & genres"));
    {
            let mut groups: Vec<(String, Vec<&Mood>)> = vec![];
            for m in list { match groups.iter_mut().find(|g| g.0 == m.section) { Some(g) => g.1.push(m), None => groups.push((m.section.clone(), vec![m])) } }
            for (sec, items) in groups {
                let col = vbox(8);
                if !sec.is_empty() { col.append(&plain(&sec, &["dim"])); }
                let chips = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).column_spacing(6).row_spacing(6).min_children_per_line(1).build();
                for m in items {
                    let btn = gtk::Button::builder().label(&m.title).css_classes(["chip", "s-mood-chip"]).build();
                    let (c, params, title) = (st.ctx.clone(), m.params.clone(), m.title.clone());
                    btn.connect_clicked(move |_| c.navigate(Nav::Mood(params.clone(), title.clone())));
                    chips.append(&btn);
                }
                col.append(&chips);
                b.append(&col);
            }
    }
    b
}

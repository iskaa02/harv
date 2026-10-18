//! Browse pages: home, search, library lists, album / artist / playlist / local-playlist details.

use crate::components::{self, DetailData, GridData, GridItem, PageData};
use crate::ctx::{Ctx, Nav};
use crate::rows::*;
use adw::prelude::*;
use gtk::{glib, pango};
use player_core::QueueKind;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use store_core::SongRow;
use ytm_core::{AlbumRef, ArtistRef, HomeItem, Mood, PlaylistRef, PodcastRef, SearchKind, SearchResults, Track};

thread_local! {
    /// Mood & genre chips for the Search page: fetched once per session; `None` until the answer arrives.
    pub(crate) static MOODS: RefCell<Option<Vec<Mood>>> = RefCell::default();
}

/// Header bar + clamped, scrollable content.
pub fn shell(title: &str, content: &impl IsA<gtk::Widget>, header_extra: Option<gtk::Widget>) -> adw::NavigationPage {
    let clamp = adw::Clamp::builder().maximum_size(1180).tightening_threshold(800).child(content).build();
    clamp.set_margin_bottom(24);
    let scroll = gtk::ScrolledWindow::builder().child(&clamp).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
    let tv = adw::ToolbarView::new();
    let hb = adw::HeaderBar::new();
    hb.add_css_class("flat");
    if let Some(w) = header_extra { hb.pack_end(&w); }
    tv.add_top_bar(&hb);
    tv.set_content(Some(&scroll));
    adw::NavigationPage::builder().title(title).child(&tv).build()
}

/// Mark a page with its theme page kind (`playlist`, `library`, `search`, `now`); the shell reads it for the right panel.
pub(crate) fn tagged(page: adw::NavigationPage, kind: &str) -> adw::NavigationPage { page.set_widget_name(kind); page }

fn column() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 22);
    b.set_margin_start(20);
    b.set_margin_end(20);
    b.set_margin_top(6);
    b
}

pub(crate) fn heading(text: &str, sub: &str) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.append(&gtk::Label::builder().label(text).xalign(0.0).css_classes(["page-title"]).build());
    if !sub.is_empty() { b.append(&gtk::Label::builder().label(sub).xalign(0.0).css_classes(["page-sub"]).build()); }
    b.upcast()
}

fn swap(holder: &gtk::Box, child: &gtk::Widget) {
    while let Some(c) = holder.first_child() { holder.remove(&c); }
    holder.append(child);
}

fn track_from_home(i: &HomeItem) -> Option<Track> {
    Some(Track { video_id: i.video_id.clone()?, title: i.title.clone(), artists: if i.subtitle.is_empty() { vec![] } else { vec![i.subtitle.clone()] }, album: None, duration_s: None, thumb: i.thumb.clone() })
}

fn rows_to_tracks(rows: Vec<SongRow>) -> Vec<Track> { rows.into_iter().map(|r| r.track).collect() }

fn nav_for_browse(id: &str) -> Option<Nav> {
    if id.starts_with("MPRE") { Some(Nav::Album(id.to_string())) }
    else if id.starts_with("UC") { Some(Nav::Artist(id.to_string())) }
    else if id.starts_with("VL") { Some(Nav::Playlist(id.to_string())) }
    else { None }
}

// ── home ─────────────────────────────────────────────────────────────────────────────────────
pub fn home(ctx: &Rc<Ctx>) -> adw::NavigationPage {
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let page = shell(&ctx.word("home"), &holder, None);
    let load = Rc::new({
        let (ctx, holder) = (ctx.clone(), holder.clone());
        move || {
            swap(&holder, &skeleton_shelves());
            let (c2, h2) = (ctx.clone(), holder.clone());
            let ytm = ctx.ytm.clone();
            ctx.spawn(async move { ytm.home().await.map_err(|e| format!("{e:#}")) }, move |res| {
                let col = column();
                col.append(&heading(&c2.word("home"), ""));
                let local = |title: &str, tracks: Vec<Track>, kind: QueueKind| -> Option<gtk::Widget> {
                    if tracks.is_empty() { return None; }
                    let all = Rc::new(tracks.clone());
                    let cards = tracks.iter().enumerate().map(|(i, t)| { let (c, all) = (c2.clone(), all.clone()); let w = card(&c2, t.thumb.as_deref(), &t.title, &t.artist_line(), false, move || c.play_row(all.to_vec(), i, kind)); crate::actions::attach_context_menu(&w, &c2, crate::actions::fixed(t.clone()), Default::default()); w }).collect();
                    Some(shelf(title, cards))
                };
                if let Some(s) = local(&c2.word("recent"), rows_to_tracks(c2.store.recently_played(15, 0).unwrap_or_default()), QueueKind::Songs) { col.append(&s); }
                if let Some(s) = local(&c2.word("favorites"), rows_to_tracks(c2.store.liked_songs(15, 0).unwrap_or_default()), QueueKind::Songs) { col.append(&s); }
                match res {
                    Ok(shelves) => for sh in shelves {
                        let cards: Vec<gtk::Widget> = sh.items.iter().filter_map(|i| {
                            let c = c2.clone();
                            let round = i.browse_id.as_deref().is_some_and(|b| b.starts_with("UC"));
                            if let Some(t) = track_from_home(i) {
                                let src = crate::actions::fixed(t.clone());
                                let w = card(&c2, i.thumb.as_deref(), &i.title, &i.subtitle, false, move || c.play_one(t.clone()));
                                crate::actions::attach_context_menu(&w, &c2, src, Default::default());
                                return Some(w);
                            }
                            let nav = i.browse_id.as_deref().and_then(nav_for_browse).or_else(|| i.playlist_id.clone().map(Nav::Playlist))?;
                            let coll = crate::actions::Collection::from_nav(&nav, &i.title, i.thumb.as_deref());
                            let w = card(&c2, i.thumb.as_deref(), &i.title, &i.subtitle, round, move || c.navigate(nav.clone()));
                            if let Some(coll) = coll { crate::actions::attach_collection_menu(&w, &c2, coll, i.title.clone()); }
                            Some(w)
                        }).collect();
                        if !cards.is_empty() { col.append(&shelf(&sh.title, cards)); }
                    },
                    Err(e) => {
                        let (ctx, h) = (c2.clone(), h2.clone());
                        let retry: Rc<dyn Fn()> = Rc::new(move || { let p = home(&ctx); if let Some(ch) = p.child() { swap(&h, &ch); } });
                        col.append(&error_state("Couldn't load YouTube Music", &e, Some(retry)));
                    }
                }
                swap(&h2, &col.upcast());
            });
        }
    });
    load();
    page
}

// ── search ───────────────────────────────────────────────────────────────────────────────────
pub fn search(ctx: &Rc<Ctx>, initial: Option<&str>) -> adw::NavigationPage {
    // A theme may lay the search page out itself (`[page.search]`); otherwise the built-in page.
    let theme = ctx.theme();
    if let Some(node) = theme.page.get("search").and_then(|p| p.main.first()) { return tagged(components::search::themed(ctx, initial, node), "search"); }
    tagged(search_classic(ctx, initial), "search")
}

/// The built-in search page.
pub fn search_classic(ctx: &Rc<Ctx>, initial: Option<&str>) -> adw::NavigationPage {
    let col = column();
    let entry = gtk::SearchEntry::builder().placeholder_text("Songs, albums, artists, playlists").hexpand(true).css_classes(["lib-search"]).build();
    col.append(&heading(&ctx.word("search"), ""));
    col.append(&entry);

    // `None` is the podcasts chip: podcasts have their own endpoint, so they are not a `SearchKind`.
    let kinds: [(&str, Option<SearchKind>); 6] = [("Songs", Some(SearchKind::Songs)), ("Albums", Some(SearchKind::Albums)), ("Artists", Some(SearchKind::Artists)), ("Playlists", Some(SearchKind::Playlists)), ("Videos", Some(SearchKind::Videos)), ("Podcasts", None)];
    let kind: Rc<RefCell<Option<SearchKind>>> = Rc::new(RefCell::new(Some(SearchKind::Songs)));
    let searched: Rc<RefCell<String>> = Rc::default();
    let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    chips.add_css_class("chips");
    let mut first: Option<gtk::ToggleButton> = None;
    let buttons: Vec<gtk::ToggleButton> = kinds.iter().map(|(l, _)| {
        let b = gtk::ToggleButton::builder().label(*l).css_classes(["chip"]).build();
        if let Some(f) = &first { b.set_group(Some(f)); } else { b.set_active(true); first = Some(b.clone()); }
        chips.append(&b);
        b
    }).collect();
    col.append(&chips);

    let suggest = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["suggest-list"]).visible(false).build();
    col.append(&suggest);
    let results = gtk::Box::new(gtk::Orientation::Vertical, 14);
    col.append(&results);

    let history = {
        let (ctx, results, entry) = (ctx.clone(), results.clone(), entry.clone());
        Rc::new(move || search_home(&ctx, &results, &entry))
    };
    history();
    // The mood list is fetched in the background; the empty-query view fills in when it arrives (unless a search has replaced it).
    if MOODS.with(|m| m.borrow().is_none()) {
        let (ytm, entry, history) = (ctx.ytm.clone(), entry.clone(), history.clone());
        ctx.spawn(async move { ytm.moods().await.unwrap_or_default() }, move |list| {
            MOODS.with(|m| *m.borrow_mut() = Some(list));
            if entry.text().trim().is_empty() { history(); }
        });
    }

    let run = Rc::new({
        let (ctx, results, entry, kind, suggest, searched) = (ctx.clone(), results.clone(), entry.clone(), kind.clone(), suggest.clone(), searched.clone());
        move || {
            let q = entry.text().trim().to_string();
            suggest.set_visible(false);
            if q.is_empty() { return; }
            *searched.borrow_mut() = q.clone();
            let _ = ctx.store.add_search(&q);
            swap(&results, &spinner_page());
            let k = *kind.borrow();
            let (ytm, q2) = (ctx.ytm.clone(), q.clone());
            let (ctx, results, entry2) = (ctx.clone(), results.clone(), entry.clone());
            ctx.clone().spawn(async move {
                let r = match k {
                    Some(k) => ytm.search(&q2, k).await.map(|r| (r, Vec::new())),
                    None => ytm.search_podcasts(&q2).await.map(|p| (SearchResults::default(), p)),
                };
                r.map_err(|e| format!("{e:#}"))
            }, move |res| {
                let out = gtk::Box::new(gtk::Orientation::Vertical, 14);
                match res {
                    Err(e) => { let en = entry2.clone(); out.append(&error_state("Search failed", &e, Some(Rc::new(move || { en.emit_activate(); })))) },
                    Ok((r, podcasts)) => {
                        let tracks: Vec<Track> = if k == Some(SearchKind::Videos) { r.videos } else { r.songs };
                        if !tracks.is_empty() { out.append(&track_list(&ctx, tracks, QueueKind::Songs, RowExtras::default())); }
                        if !r.albums.is_empty() { out.append(&grid(r.albums.iter().map(|a| album_card(&ctx, a)).collect())); }
                        if !r.artists.is_empty() { out.append(&grid(r.artists.iter().map(|a| artist_card(&ctx, a)).collect())); }
                        if !r.playlists.is_empty() { out.append(&grid(r.playlists.iter().map(|p| playlist_card(&ctx, p)).collect())); }
                        if !podcasts.is_empty() { out.append(&grid(podcasts.iter().map(|p| podcast_card(&ctx, p)).collect())); }
                        if out.first_child().is_none() { out.append(&status("edit-find-symbolic", "No results", &format!("Nothing found for “{q}”."))); }
                    }
                }
                swap(&results, &out.upcast());
            });
        }
    });
    entry.connect_activate({ let run = run.clone(); move |_| run() });
    for (b, (_, k)) in buttons.iter().zip(kinds.iter()) {
        let (kind, run, entry) = (kind.clone(), run.clone(), entry.clone());
        let k = *k;
        b.connect_toggled(move |b| { if b.is_active() { *kind.borrow_mut() = k; if !entry.text().is_empty() { run(); } } });
    }
    entry.connect_search_changed({
        let (ctx, suggest, entry, history, results, searched) = (ctx.clone(), suggest.clone(), entry.clone(), history.clone(), results.clone(), searched.clone());
        let seq = Rc::new(std::cell::Cell::new(0u32));
        move |e| {
            let q = e.text().trim().to_string();
            seq.set(seq.get() + 1);
            if q.is_empty() { suggest.set_visible(false); history(); return; }
            let (n, ytm) = (seq.get(), ctx.ytm.clone());
            let (seq, suggest, entry, _results, searched) = (seq.clone(), suggest.clone(), entry.clone(), results.clone(), searched.clone());
            ctx.spawn(async move { ytm.suggest(&q).await.unwrap_or_default() }, move |list| {
                if seq.get() != n || *searched.borrow() == entry.text().trim() { return; } // superseded by a newer keystroke, or already searched
                while let Some(c) = suggest.first_child() { suggest.remove(&c); }
                for s in list.into_iter().take(6) {
                    let row = gtk::ListBoxRow::builder().child(&gtk::Label::builder().label(&s).xalign(0.0).margin_top(8).margin_bottom(8).margin_start(12).build()).build();
                    row.set_widget_name(&s);
                    suggest.append(&row);
                }
                suggest.set_visible(suggest.first_child().is_some());
                let entry = entry.clone();
                suggest.connect_row_activated(move |l, row| { l.set_visible(false); entry.set_text(&row.widget_name()); entry.emit_by_name::<()>("activate", &[]); });
            });
        }
    });
    if let Some(q) = initial { entry.set_text(q); entry.emit_by_name::<()>("activate", &[]); }
    let page = shell("Search", &col, None);
    let e = entry.clone();
    page.connect_showing(move |_| { e.grab_focus(); });
    page
}

/// The Search page with an empty query: recent searches, then the mood & genre chips (grouped by section).
fn search_home(ctx: &Rc<Ctx>, results: &gtk::Box, entry: &gtk::SearchEntry) {
    let h = ctx.store.search_history(12).unwrap_or_default();
    let moods = MOODS.with(|m| m.borrow().clone());
    if h.is_empty() && moods.as_ref().map_or(true, |m| m.is_empty()) {
        match moods {
            None => swap(results, &spinner_page()),
            Some(_) => swap(results, &status("system-search-symbolic", "Search YouTube Music", "Find songs, albums, artists and playlists.")),
        }
        return;
    }
    let b = gtk::Box::new(gtk::Orientation::Vertical, 22);
    if !h.is_empty() {
        let recent = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&section_title("Recent searches"));
        head.first_child().unwrap().set_hexpand(true);
        let clear = gtk::Button::builder().label("Clear").css_classes(["flat", "pill-btn"]).build();
        { let (ctx, results, entry) = (ctx.clone(), results.clone(), entry.clone()); clear.connect_clicked(move |_| { let _ = ctx.store.clear_search_history(); search_home(&ctx, &results, &entry); }); }
        head.append(&clear);
        recent.append(&head);
        for q in h {
            let btn = gtk::Button::builder().label(&q).halign(gtk::Align::Start).css_classes(["flat", "history-item"]).build();
            btn.child().and_downcast::<gtk::Label>().inspect(|l| l.set_xalign(0.0));
            let entry = entry.clone();
            btn.connect_clicked(move |_| { entry.set_text(&q); entry.emit_by_name::<()>("activate", &[]); });
            recent.append(&btn);
        }
        b.append(&recent);
    }
    if let Some(list) = moods.filter(|m| !m.is_empty()) {
        let mut groups: Vec<(String, Vec<Mood>)> = vec![];
        for m in list {
            match groups.iter_mut().find(|g| g.0 == m.section) { Some(g) => g.1.push(m), None => groups.push((m.section.clone(), vec![m])) }
        }
        let mb = gtk::Box::new(gtk::Orientation::Vertical, 14);
        mb.append(&section_title("Moods & genres"));
        for (sec, items) in groups {
            let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
            if !sec.is_empty() { col.append(&gtk::Label::builder().label(&sec).xalign(0.0).css_classes(["dim"]).build()); }
            let chips = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).column_spacing(6).row_spacing(6).min_children_per_line(1).build();
            for m in items {
                let btn = gtk::Button::builder().label(&m.title).css_classes(["chip"]).build();
                let (c, params, title) = (ctx.clone(), m.params, m.title);
                btn.connect_clicked(move |_| c.navigate(Nav::Mood(params.clone(), title.clone())));
                chips.append(&btn);
            }
            col.append(&chips);
            mb.append(&col);
        }
        b.append(&mb);
    }
    swap(results, &b.upcast());
}

pub(crate) fn grid(cards: Vec<gtk::Widget>) -> gtk::Widget {
    let f = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).homogeneous(true).max_children_per_line(8).min_children_per_line(2).column_spacing(6).row_spacing(10).build();
    for c in cards { f.append(&c); }
    f.upcast()
}

pub(crate) fn album_card(ctx: &Rc<Ctx>, a: &AlbumRef) -> gtk::Widget {
    let (c, id) = (ctx.clone(), a.browse_id.clone());
    let w = card(ctx, a.thumb.as_deref(), &a.title, &[a.artist.as_str(), a.year.as_str()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" · "), false, move || c.navigate(Nav::Album(id.clone())));
    crate::actions::attach_collection_menu(&w, ctx, crate::actions::Collection::Album(a.browse_id.clone()), a.title.clone());
    w
}
pub(crate) fn artist_card(ctx: &Rc<Ctx>, a: &ArtistRef) -> gtk::Widget {
    let (c, id) = (ctx.clone(), a.channel_id.clone());
    let w = card(ctx, a.thumb.as_deref(), &a.name, a.subscribers.as_deref().unwrap_or("Artist"), true, move || c.navigate(Nav::Artist(id.clone())));
    crate::actions::attach_collection_menu(&w, ctx, crate::actions::Collection::Artist { id: a.channel_id.clone(), name: a.name.clone(), thumb: a.thumb.clone() }, a.name.clone());
    w
}
pub(crate) fn playlist_card(ctx: &Rc<Ctx>, p: &PlaylistRef) -> gtk::Widget {
    let (c, id) = (ctx.clone(), p.playlist_id.clone());
    let w = card(ctx, p.thumb.as_deref(), &p.title, &p.author, false, move || c.navigate(Nav::Playlist(id.clone())));
    crate::actions::attach_collection_menu(&w, ctx, crate::actions::Collection::Playlist(p.playlist_id.clone()), p.title.clone());
    w
}
pub(crate) fn podcast_card(ctx: &Rc<Ctx>, p: &PodcastRef) -> gtk::Widget {
    let (c, id) = (ctx.clone(), p.podcast_id.clone());
    let w = card(ctx, p.thumb.as_deref(), &p.title, &p.publisher, false, move || c.navigate(Nav::Podcast(id.clone())));
    crate::actions::attach_collection_menu(&w, ctx, crate::actions::Collection::Podcast(p.podcast_id.clone()), p.title.clone());
    w
}

// ── detail pages ─────────────────────────────────────────────────────────────────────────────
pub(crate) fn detail_header(ctx: &Rc<Ctx>, kind: &str, thumb: Option<&str>, title: &str, sub: &str, round: bool, buttons: Vec<gtk::Widget>) -> gtk::Widget {
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 28);
    h.add_css_class("detail-head");
    let a = art(ctx, thumb, 200);
    a.add_css_class("hero-art");
    a.set_valign(gtk::Align::End);
    if round { a.add_css_class("round"); }
    h.append(&a);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
    col.set_valign(gtk::Align::End);
    col.append(&gtk::Label::builder().label(kind.to_uppercase()).xalign(0.0).css_classes(["hero-kind"]).build());
    col.append(&gtk::Label::builder().label(title).xalign(0.0).wrap(true).wrap_mode(pango::WrapMode::WordChar).css_classes(["hero-title"]).build());
    if !sub.is_empty() { col.append(&gtk::Label::builder().label(sub).xalign(0.0).wrap(true).css_classes(["page-sub"]).build()); }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_margin_top(8);
    for b in buttons { row.append(&b); }
    col.append(&row);
    h.append(&col);
    h.upcast()
}

pub(crate) fn play_buttons(ctx: &Rc<Ctx>, tracks: Rc<Vec<Track>>, kind: QueueKind) -> Vec<gtk::Widget> {
    let play = pill_button("media-playback-start-symbolic", "Play", &["suggested-action", "pill"]);
    let shuffle = pill_button("media-playlist-shuffle-symbolic", "Shuffle", &["pill"]);
    { let (c, t) = (ctx.clone(), tracks.clone()); play.connect_clicked(move |_| { c.engine.send(player_core::Command::SetShuffle(false)); c.play(t.to_vec(), 0, kind); }); }
    { let (c, t) = (ctx.clone(), tracks); shuffle.connect_clicked(move |_| c.play_shuffled(t.to_vec(), kind)); }
    vec![play.upcast(), shuffle.upcast()]
}

fn loading_page(title: &str) -> (adw::NavigationPage, gtk::Box) {
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.append(&spinner_page());
    (shell(title, &holder, None), holder)
}

fn error_view(e: &str) -> gtk::Widget { error_state("Couldn't load this page", e, None) }

pub fn album(ctx: &Rc<Ctx>, id: &str) -> adw::NavigationPage {
    let (page, holder) = loading_page("Album");
    let page = tagged(page, "playlist");
    let (ytm, id2) = (ctx.ytm.clone(), id.to_string());
    let ctx = ctx.clone();
    ctx.clone().spawn(async move { ytm.album(&id2).await.map_err(|e| format!("{e:#}")) }, move |res| match res {
        Err(e) => swap(&holder, &error_view(&e)),
        Ok(mut a) => {
            // Album tracks arrive without cover/album: fill them in so queue rows look right.
            for t in &mut a.tracks { t.album = Some(a.title.clone()); }
            let col = column();
            let tracks = Rc::new(a.tracks.clone());
            let btn_ctx = ctx.clone();
            col.append(&components::detail::compose(&ctx, "playlist", PageData::Detail(DetailData {
                kind_label: "Album".into(), title: a.title.clone(), subtitle: format!("{} · {}", a.artists.join(", "), a.year), thumb: a.thumb.clone(), round: false, simple: false, description: None,
                tracks: tracks.clone(), queue_kind: QueueKind::Album, extras: RowExtras { numbered: true, ..Default::default() },
                buttons: Box::new(move || play_buttons(&btn_ctx, tracks.clone(), QueueKind::Album)), empty: None,
            })));
            swap(&holder, &col.upcast());
        }
    });
    page
}

pub fn artist(ctx: &Rc<Ctx>, id: &str) -> adw::NavigationPage {
    let (page, holder) = loading_page("Artist");
    let (ytm, id2) = (ctx.ytm.clone(), id.to_string());
    let ctx = ctx.clone();
    ctx.clone().spawn(async move { ytm.artist(&id2).await.map_err(|e| format!("{e:#}")) }, move |res| match res {
        Err(e) => swap(&holder, &error_view(&e)),
        Ok(a) => {
            let mut a = a;
            // The artist page lists songs without cover art; borrow the artist photo so rows aren't blank.
            for t in &mut a.songs { if t.thumb.is_none() { t.thumb = a.thumb.clone(); } }
            let col = column();
            let mut btns = play_buttons(&ctx, Rc::new(a.songs.clone()), QueueKind::Playlist);
            let follow = gtk::ToggleButton::builder().css_classes(["pill"]).build();
            let sync = |b: &gtk::ToggleButton| b.set_label(if b.is_active() { "Following" } else { "Follow" });
            follow.set_active(ctx.store.is_followed(&a.channel_id));
            sync(&follow);
            {
                let (c, id, name, thumb) = (ctx.clone(), a.channel_id.clone(), a.name.clone(), a.thumb.clone());
                follow.connect_toggled(move |b| { sync(b); let _ = c.store.set_followed(&id, &name, thumb.as_deref(), b.is_active()); });
            }
            btns.push(follow.upcast());
            col.append(&detail_header(&ctx, "Artist", a.thumb.as_deref(), &a.name, a.subscribers.as_deref().unwrap_or(""), true, btns));
            if !a.songs.is_empty() { col.append(&section_title("Top songs")); col.append(&track_list(&ctx, a.songs, QueueKind::Songs, RowExtras::default())); }
            if !a.albums.is_empty() { col.append(&shelf("Albums", a.albums.iter().map(|x| album_card(&ctx, x)).collect())); }
            if !a.singles.is_empty() { col.append(&shelf("Singles & EPs", a.singles.iter().map(|x| album_card(&ctx, x)).collect())); }
            swap(&holder, &col.upcast());
        }
    });
    page
}

pub fn playlist(ctx: &Rc<Ctx>, id: &str) -> adw::NavigationPage {
    let (page, holder) = loading_page("Playlist");
    let page = tagged(page, "playlist");
    let (ytm, id2) = (ctx.ytm.clone(), id.to_string());
    let ctx = ctx.clone();
    ctx.clone().spawn(async move { ytm.playlist(&id2).await.map_err(|e| format!("{e:#}")) }, move |res| match res {
        Err(e) => swap(&holder, &error_view(&e)),
        Ok(p) => {
            let col = column();
            let tracks = Rc::new(p.tracks.clone());
            let (btn_ctx, btn_tracks) = (ctx.clone(), tracks.clone());
            col.append(&components::detail::compose(&ctx, "playlist", PageData::Detail(DetailData {
                kind_label: "Playlist".into(), title: p.title.clone(), subtitle: p.author.clone(), thumb: p.thumb.clone(), round: false, simple: false, description: None,
                tracks, queue_kind: QueueKind::Playlist, extras: RowExtras::default(),
                buttons: Box::new(move || { let mut b = play_buttons(&btn_ctx, btn_tracks.clone(), QueueKind::Playlist); b.push(download_button(&btn_ctx, btn_tracks.to_vec())); b }), empty: None,
            })));
            swap(&holder, &col.upcast());
        }
    });
    page
}

/// A podcast: cover, episode count, description, then the episodes (each plays by its `video_id`).
pub fn podcast(ctx: &Rc<Ctx>, id: &str) -> adw::NavigationPage {
    let (page, holder) = loading_page("Podcast");
    let page = tagged(page, "playlist");
    let (ytm, id2) = (ctx.ytm.clone(), id.to_string());
    let ctx = ctx.clone();
    ctx.clone().spawn(async move { ytm.podcast(&id2).await.map_err(|e| format!("{e:#}")) }, move |res| match res {
        Err(e) => swap(&holder, &error_view(&e)),
        Ok(p) => {
            let col = column();
            let n = p.episodes.len();
            let tracks = Rc::new(p.episodes.clone());
            let (btn_ctx, btn_tracks) = (ctx.clone(), tracks.clone());
            col.append(&components::detail::compose(&ctx, "playlist", PageData::Detail(DetailData {
                kind_label: "Podcast".into(), title: p.title.clone(), subtitle: format!("{n} episodes"), thumb: p.thumb.clone(), round: false, simple: false, description: p.description.clone(),
                tracks, queue_kind: QueueKind::Playlist, extras: RowExtras::default(),
                buttons: Box::new(move || play_buttons(&btn_ctx, btn_tracks.clone(), QueueKind::Playlist)), empty: None,
            })));
            swap(&holder, &col.upcast());
        }
    });
    page
}

/// YouTube Music's regional "Trending" charts (`ytm.chart` codes).
const CHARTS: [(&str, &str); 8] = [("Global", "global"), ("Vietnam", "vn"), ("Italy", "it"), ("Indonesia", "id"), ("Brazil", "br"), ("Mexico", "mx"), ("United States", "us"), ("Spain", "es")];

pub fn charts(ctx: &Rc<Ctx>) -> adw::NavigationPage {
    let names: Vec<&str> = CHARTS.iter().map(|c| c.0).collect();
    let region = gtk::DropDown::from_strings(&names);
    region.set_valign(gtk::Align::Center);
    let col = column();
    col.append(&detail_header_simple("Charts", vec![region.clone().upcast()]));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    col.append(&body);
    let page = shell("Charts", &col, None);
    // Each load gets a number; a slow answer for a region we already left is dropped.
    let seq = Rc::new(Cell::new(0u32));
    let load = Rc::new({
        let (ctx, body, seq) = (ctx.clone(), body.clone(), seq.clone());
        move |code: &'static str| {
            swap(&body, &spinner_page());
            let n = seq.get() + 1;
            seq.set(n);
            let (ytm, c, body, seq) = (ctx.ytm.clone(), ctx.clone(), body.clone(), seq.clone());
            ctx.spawn(async move { ytm.chart(code).await.map_err(|e| format!("{e:#}")) }, move |res| {
                if seq.get() != n { return; }
                match res {
                    Err(e) => swap(&body, &error_view(&e)),
                    Ok(tracks) => {
                        let col = gtk::Box::new(gtk::Orientation::Vertical, 12);
                        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                        for b in play_buttons(&c, Rc::new(tracks.clone()), QueueKind::Playlist) { bar.append(&b); }
                        col.append(&bar);
                        col.append(&track_list(&c, tracks, QueueKind::Playlist, RowExtras::default()));
                        swap(&body, &col.upcast());
                    }
                }
            });
        }
    });
    {
        let load = load.clone();
        region.connect_selected_notify(move |d| load(CHARTS[d.selected() as usize].1));
    }
    load(CHARTS[0].1);
    page
}

/// A playlist stored locally: reorder, remove, rename, delete.
pub fn local_playlist(ctx: &Rc<Ctx>, id: i64) -> adw::NavigationPage {
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let menu = gtk::MenuButton::builder().icon_name("view-more-symbolic").css_classes(["flat"]).build();
    let page = tagged(shell("Playlist", &holder, Some(menu.clone().upcast())), "playlist");
    let refresh: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::default();
    let rebuild = Rc::new({
        let (ctx, holder, refresh, page) = (ctx.clone(), holder.clone(), refresh.clone(), page.clone());
        move || {
            let info = ctx.store.playlists().unwrap_or_default().into_iter().find(|p| p.id == id);
            let Some(info) = info else { swap(&holder, &status("folder-symbolic", "Playlist deleted", "")); return; };
            page.set_title(&info.title);
            let tracks = ctx.store.playlist_tracks(id).unwrap_or_default();
            let col = column();
            let again = refresh.clone();
            let call = move || if let Some(f) = again.borrow().as_ref() { f() };
            let (c1, c2, listed) = (ctx.clone(), ctx.clone(), Rc::new(tracks.clone()));
            let (call1, call2) = (call.clone(), call);
            let extras = RowExtras {
                on_remove: Some(Rc::new({
                    let listed = listed.clone();
                    move |i| {
                        let Some(t) = listed.get(i).cloned() else { return };
                        let _ = c1.store.remove_from_playlist(id, &t.video_id);
                        call1();
                        // Undo: append it again, then slide it back to where it was.
                        let (c, again, n) = (c1.clone(), call1.clone(), listed.len());
                        c1.toast_action("Removed from playlist", "Undo", move || { let _ = c.store.add_to_playlist(id, std::slice::from_ref(&t)); if i + 1 < n { let _ = c.store.move_in_playlist(id, n - 1, i); } again(); });
                    }
                })),
                on_move: Some(Rc::new(move |from, to| { let _ = c2.store.move_in_playlist(id, from, to); call2(); })),
                show_art: true,
                ..Default::default()
            };
            let all = Rc::new(tracks);
            let (btn_ctx, btn_tracks) = (ctx.clone(), all.clone());
            col.append(&components::detail::compose(&ctx, "playlist", PageData::Detail(DetailData {
                kind_label: "Playlist".into(), title: info.title.clone(), subtitle: format!("{} songs", all.len()), thumb: info.thumb.clone(), round: false, simple: false, description: None,
                tracks: all, queue_kind: QueueKind::LocalPlaylist, extras,
                buttons: Box::new(move || {
                    if btn_tracks.is_empty() { return vec![]; }
                    let mut b = play_buttons(&btn_ctx, btn_tracks.clone(), QueueKind::LocalPlaylist);
                    b.push(download_button(&btn_ctx, btn_tracks.to_vec()));
                    b
                }),
                empty: Some(("view-list-symbolic".into(), "Empty playlist".into(), "Add songs from any list with “Add to playlist…”.".into())),
            })));
            swap(&holder, &col.upcast());
        }
    });
    *refresh.borrow_mut() = Some(rebuild.clone() as Rc<dyn Fn()>);
    rebuild();

    // rename / delete menu
    let pop = gtk::Popover::new();
    pop.add_css_class("menu");
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for (label, rename) in [("Rename…", true), ("Delete playlist", false)] {
        let b = gtk::Button::builder().label(label).css_classes(["flat", "menu-item"]).build();
        b.child().and_downcast::<gtk::Label>().inspect(|l| l.set_xalign(0.0));
        let (ctx, rebuild, pop, menu) = (ctx.clone(), rebuild.clone(), pop.clone(), menu.clone());
        b.connect_clicked(move |_| {
            pop.popdown();
            if rename {
                let d = adw::AlertDialog::builder().heading("Rename playlist").build();
                let e = gtk::Entry::builder().text(ctx.store.playlists().unwrap_or_default().into_iter().find(|p| p.id == id).map(|p| p.title).unwrap_or_default()).build();
                d.set_extra_child(Some(&e));
                d.add_response("cancel", "Cancel");
                d.add_response("ok", "Rename");
                d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
                let (ctx, rebuild) = (ctx.clone(), rebuild.clone());
                d.connect_response(None, move |_, r| { if r == "ok" { let _ = ctx.store.rename_playlist(id, &e.text()); rebuild(); } });
                d.present(Some(&menu));
            } else {
                let d = adw::AlertDialog::builder().heading("Delete this playlist?").body("The songs stay in your library.").build();
                d.add_response("cancel", "Cancel");
                d.add_response("delete", "Delete");
                d.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                let (ctx, rebuild) = (ctx.clone(), rebuild.clone());
                d.connect_response(None, move |_, r| { if r == "delete" { let _ = ctx.store.delete_playlist(id); rebuild(); ctx.navigate(Nav::CloseNowPlaying); } });
                d.present(Some(&menu));
            }
        });
        box_.append(&b);
    }
    pop.set_child(Some(&box_));
    menu.set_popover(Some(&pop));
    page
}

// ── library ──────────────────────────────────────────────────────────────────────────────────
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lib { Favorites, Recent, MostPlayed, Downloads, Playlists, Following }

pub fn library(ctx: &Rc<Ctx>, which: Lib) -> adw::NavigationPage {
    let (title, empty_icon, empty_text) = match which {
        Lib::Favorites => (ctx.word("favorites"), "heart-filled-symbolic", "Tap the heart on any song to keep it here."),
        Lib::Recent => (ctx.word("recent"), "document-open-recent-symbolic", "Songs you listen to will show up here."),
        Lib::MostPlayed => (ctx.word("most_played"), "media-playlist-repeat-symbolic", "Your most listened-to songs appear here."),
        Lib::Downloads => (ctx.word("downloads"), "folder-download-symbolic", "Downloaded songs appear here."),
        Lib::Playlists => (ctx.word("playlists"), "view-list-symbolic", "Create a playlist to get started."),
        Lib::Following => (ctx.word("following"), "avatar-default-symbolic", "Follow artists from their page."),
    };
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let page = tagged(shell(&title, &holder, None), if matches!(which, Lib::Playlists | Lib::Following) { "library" } else { "playlist" });
    let build = Rc::new({
        let (ctx, holder, title) = (ctx.clone(), holder.clone(), title.clone());
        move || {
            let col = column();
            let no_items = (empty_icon.to_string(), format!("No {} yet", title.to_lowercase()), empty_text.to_string());
            match which {
                Lib::Playlists => {
                    let btn_ctx = ctx.clone();
                    let items = ctx.store.playlists().unwrap_or_default().iter().map(|p| { let c = ctx.clone(); let id = p.id; GridItem { thumb: p.thumb.clone(), title: p.title.clone(), subtitle: format!("{} songs", p.count), round: false, on_click: Rc::new(move || c.navigate(Nav::Local(id))) } }).collect();
                    col.append(&components::detail::compose(&ctx, "library", PageData::Grid(GridData {
                        title: title.clone(), items, empty: Some(no_items),
                        buttons: Box::new(move || {
                            let new = pill_button("list-add-symbolic", &btn_ctx.word("new_playlist"), &["pill"]);
                            new.set_halign(gtk::Align::Start);
                            { let ctx = btn_ctx.clone(); new.connect_clicked(move |_| new_playlist_dialog(&ctx)); }
                            vec![new.upcast()]
                        }),
                    })));
                }
                Lib::Following => {
                    let items = ctx.store.followed_artists().unwrap_or_default().into_iter().map(|(id, name, thumb)| { let c = ctx.clone(); GridItem { thumb, title: name, subtitle: "Artist".into(), round: true, on_click: Rc::new(move || c.navigate(Nav::Artist(id.clone()))) } }).collect();
                    col.append(&components::detail::compose(&ctx, "library", PageData::Grid(GridData { title: title.clone(), items, empty: Some(no_items), buttons: Box::new(Vec::new) })));
                }
                _ => {
                    let rows = match which {
                        Lib::Favorites => ctx.store.liked_songs(2000, 0),
                        Lib::Recent => ctx.store.recently_played(200, 0),
                        Lib::MostPlayed => ctx.store.most_played(),
                        _ => ctx.store.downloaded_songs(),
                    }.unwrap_or_default();
                    let tracks = Rc::new(rows_to_tracks(rows));
                    let (btn_ctx, btn_tracks) = (ctx.clone(), tracks.clone());
                    col.append(&components::detail::compose(&ctx, "playlist", PageData::Detail(DetailData {
                        kind_label: String::new(), title: title.clone(), subtitle: format!("{} songs", tracks.len()), thumb: None, round: false, simple: true, description: None,
                        tracks, queue_kind: QueueKind::Songs, extras: RowExtras::default(), empty: Some(no_items),
                        buttons: Box::new(move || {
                            if btn_tracks.is_empty() { return vec![]; }
                            let mut head = play_buttons(&btn_ctx, btn_tracks.clone(), QueueKind::Songs);
                            if which == Lib::Favorites { head.push(download_button(&btn_ctx, btn_tracks.to_vec())); }
                            head
                        }),
                    })));
                }
            }
            swap(&holder, &col.upcast());
        }
    });
    build();
    // Lists are cheap to rebuild, and likes/plays change them: refresh whenever the page comes back into view.
    page.connect_showing({ let b = build.clone(); move |_| b() });
    page
}

pub(crate) fn detail_header_simple(title: &str, buttons: Vec<gtk::Widget>) -> gtk::Widget {
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let t = heading(title, "");
    t.set_hexpand(true);
    h.append(&t);
    for b in buttons { h.append(&b); }
    h.upcast()
}

pub fn new_playlist_dialog(ctx: &Rc<Ctx>) {
    let d = adw::AlertDialog::builder().heading("New playlist").build();
    let e = gtk::Entry::builder().placeholder_text("Name").activates_default(true).build();
    d.set_extra_child(Some(&e));
    d.add_response("cancel", "Cancel");
    d.add_response("ok", "Create");
    d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    let c = ctx.clone();
    d.connect_response(None, move |_, r| {
        if r == "ok" { match c.store.create_playlist(&e.text()) { Ok(id) => c.navigate(Nav::Local(id)), Err(_) => c.toast("Give the playlist a name") } }
    });
    d.present(ctx.toasts.root().as_ref());
}

#[allow(dead_code)]
fn _unused(_: glib::Object) {}

/// Playlists for one mood/genre (`params` is the opaque token from `Ytm::moods`).
pub fn mood(ctx: &Rc<Ctx>, params: &str, title: &str) -> adw::NavigationPage {
    let (page, holder) = loading_page(title);
    let (ytm, p, title) = (ctx.ytm.clone(), params.to_string(), title.to_string());
    let ctx = ctx.clone();
    ctx.clone().spawn(async move { ytm.mood_playlists(&p).await.map_err(|e| format!("{e:#}")) }, move |res| match res {
        Err(e) => swap(&holder, &error_view(&e)),
        Ok(list) if list.is_empty() => swap(&holder, &status("folder-music-symbolic", "No playlists", "YouTube Music has nothing in this category right now.")),
        Ok(list) => {
            let col = column();
            col.append(&heading(&title, &format!("{} playlists", list.len())));
            col.append(&grid(list.iter().map(|p| playlist_card(&ctx, p)).collect()));
            swap(&holder, &col.upcast());
        }
    });
    page
}

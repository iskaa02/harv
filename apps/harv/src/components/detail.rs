//! Page composition for song-list and grid pages, plus the body components (`hero`, `track-list`, `cover-grid`).

use super::{DetailData, Env, GridData, GridItem, PageData, Region, Shelf, ShelfSet};
use crate::ctx::Ctx;
use crate::pages;
use crate::rows::*;
use gtk::prelude::*;
use std::rc::Rc;
use theme_core::Node;

/// Compose a page body. `kind` is the theme page kind (`playlist` for song lists, `library` for grids). With no layout in the
/// theme for that kind, the built-in composition runs — exactly what the pages built before themes existed.
pub fn compose(ctx: &Rc<Ctx>, kind: &str, data: PageData) -> gtk::Widget {
    let theme = ctx.theme();
    if let Some(page) = theme.page.get(kind).filter(|p| !p.main.is_empty()) {
        let env = Env { ctx, region: Region::Page, data: Some(&data) };
        return super::build_all(&env, &page.main, gtk::Orientation::Vertical, 22).upcast();
    }
    match &data { PageData::Detail(d) => builtin_detail(ctx, d), PageData::Grid(g) => builtin_grid(ctx, g), PageData::Home { title, shelves } => builtin_home(title, shelves) }
}

/// The original look: header, optional description, then the list (or the empty state).
fn builtin_detail(ctx: &Rc<Ctx>, d: &DetailData) -> gtk::Widget {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 22);
    let buttons = (d.buttons)();
    col.append(&if d.simple { pages::detail_header_simple(&d.title, buttons) } else { pages::detail_header(ctx, &d.kind_label, d.thumb.as_deref(), &d.title, &d.subtitle, d.round, buttons) });
    if let Some(text) = d.description.as_deref() {
        col.append(&gtk::Label::builder().label(text).xalign(0.0).wrap(true).lines(4).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["dim"]).build());
    }
    match (&d.empty, d.tracks.is_empty()) {
        (Some((icon, title, desc)), true) => col.append(&status(icon, title, desc)),
        _ => col.append(&track_list(ctx, d.tracks.to_vec(), d.queue_kind, d.extras.clone())),
    }
    col.upcast()
}

/// The original Home: the page title, then every shelf as a scrolling row.
fn builtin_home(title: &str, set: &ShelfSet) -> gtk::Widget {
    let col = pages::column();
    col.append(&pages::heading(title, ""));
    for sh in set.shelves.borrow_mut().iter_mut() { sh.used = true; col.append(&shelf(&sh.title, std::mem::take(&mut sh.cards))); }
    col.upcast()
}

fn builtin_grid(ctx: &Rc<Ctx>, g: &GridData) -> gtk::Widget {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 22);
    col.append(&pages::heading(&g.title, ""));
    for b in (g.buttons)() { col.append(&b); }
    if g.items.is_empty() {
        if let Some((icon, title, desc)) = &g.empty { col.append(&status(icon, title, desc)); }
    } else {
        col.append(&pages::grid(g.items.iter().map(|i| { let f = i.on_click.clone(); card(ctx, i.thumb.as_deref(), &i.title, &i.subtitle, i.round, move || f()) }).collect()));
    }
    col.upcast()
}

/// Body components: `hero`, `track-list`, `cover-grid`. Panel components are in `panel`.
pub fn component(env: &Env, node: &Node) -> Option<gtk::Widget> {
    match node.c.as_str() {
        "hero" => match env.data? { PageData::Detail(d) => Some(hero(env.ctx, d, node)), PageData::Grid(g) => Some(library_header(env.ctx, g, node)), PageData::Home { .. } => None },
        "heading" => Some(heading_node(env, node)),
        "shelf" | "shelves" => shelves_node(env, node),
        "track-list" => match env.data? { PageData::Detail(d) => Some(track_list_node(env.ctx, d, node)), _ => None },
        "cover-grid" => match env.data? { PageData::Grid(g) => Some(cover_grid(env.ctx, g, node)), _ => None },
        _ => None,
    }
}

/// `hero`: the header of a song-list page.
///   style = "cover" (default: cover art beside the text) | "plain" (text only)
///   cover = px, shape = "square" | "round" (round also for artist pages), kind_label = bool,
///   meta = ["crossfade", …] (chips), description = bool (podcast description under the header).
fn hero(ctx: &Rc<Ctx>, d: &DetailData, node: &Node) -> gtk::Widget {
    let style = node.str_or("style", "cover");
    let px = node.int_or("cover", 170) as i32;
    let round = node.str("shape").map_or(d.round, |s| s == "round");
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 28);
    h.add_css_class("detail-head");
    h.add_css_class("hero");
    h.add_css_class(&format!("hero-{style}"));

    match style {
        "plain" => {}
        // Library lists (Favorites, Recent…) have no cover: an accent-tinted icon tile instead of an empty square.
        _ if d.thumb.is_none() && d.simple => {
            let tile = gtk::Box::builder().css_classes(["hero-art", "hero-icon"]).halign(gtk::Align::Start).valign(gtk::Align::End).build();
            tile.set_size_request(px, px);
            tile.set_hexpand(false); // the centred icon asks to expand; the tile itself must keep its size
            tile.set_vexpand(false);
            let icon = gtk::Image::builder().icon_name(d.empty.as_ref().map(|e| e.0.as_str()).unwrap_or("audio-x-generic-symbolic")).pixel_size(px / 3).halign(gtk::Align::Center).valign(gtk::Align::Center).hexpand(true).vexpand(true).build();
            tile.append(&icon);
            h.append(&tile);
        }
        _ => {
            let a = art(ctx, d.thumb.as_deref(), px);
            a.add_css_class("hero-art");
            if round { a.add_css_class("round"); }
            a.set_valign(gtk::Align::End);
            h.append(&a);
        }
    }

    let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
    col.set_valign(gtk::Align::End);
    col.set_hexpand(true);
    if node.bool_or("kind_label", true) && !d.kind_label.is_empty() {
        col.append(&gtk::Label::builder().label(d.kind_label.to_uppercase()).xalign(0.0).css_classes(["hero-kind"]).build());
    }
    col.append(&gtk::Label::builder().label(&d.title).xalign(0.0).wrap(true).wrap_mode(gtk::pango::WrapMode::WordChar).css_classes(["hero-title"]).build());
    if !d.subtitle.is_empty() { col.append(&gtk::Label::builder().label(&d.subtitle).xalign(0.0).wrap(true).css_classes(["page-sub"]).build()); }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.add_css_class("hero-actions");
    actions.set_margin_top(10);
    for b in (d.buttons)() { actions.append(&b); }
    // Meta chips sit at the right end of the button row (Mixer: "CROSSFADE 4s").
    let meta = node.strs("meta");
    if !meta.is_empty() {
        actions.append(&gtk::Box::builder().hexpand(true).build());
        for m in &meta {
            if m == "crossfade" {
                let ms = ctx.snap.borrow().as_ref().map(|s| s.crossfade_ms).unwrap_or(0);
                let text = if ms == 0 { "CROSSFADE OFF".to_string() } else { format!("CROSSFADE {}s", (ms as f64 / 1000.0).round() as u64) };
                actions.append(&gtk::Label::builder().label(text).css_classes(["meta-chip"]).valign(gtk::Align::Center).build());
            }
        }
    }
    col.append(&actions);
    h.append(&col);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.append(&h);
    if node.bool_or("description", false) {
        if let Some(text) = d.description.as_deref() {
            root.append(&gtk::Label::builder().label(text).xalign(0.0).wrap(true).lines(4).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["dim"]).build());
        }
    }
    root.upcast()
}

/// Library header (Lagoon): title on the left, mono counters on the right. The counts are real: ALL = the list,
/// SAVED = liked songs, OFFLINE = downloaded songs; the sort label shows the current order.
fn library_header(ctx: &Rc<Ctx>, g: &GridData, node: &Node) -> gtk::Widget {
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    h.add_css_class("library-head");
    let title = gtk::Label::builder().label(&g.title).xalign(0.0).hexpand(true).css_classes(["library-title"]).build();
    h.append(&title);
    let filters = node.strs("filters");
    if !filters.is_empty() {
        let text = filters.iter().map(|f| match f.as_str() {
            "all" => format!("ALL {}", g.items.len()),
            "saved" => format!("SAVED {}", ctx.store.liked_ids().len()),
            "offline" => format!("OFFLINE {}", ctx.store.downloaded_paths().len()),
            "sort" => format!("SORT: {}", node.str_or("sort", "recent").to_uppercase()),
            other => other.to_uppercase(),
        }).collect::<Vec<_>>().join("  ·  ");
        h.append(&gtk::Label::builder().label(text).css_classes(["library-filters"]).valign(gtk::Align::Center).build());
    }
    h.upcast()
}

/// `track-list`: the song list. Props: style = "classic" | "table" | "rows", columns = [index,title,artist,album,time]
/// (table / rows), cover = px, numbered = bool.
fn track_list_node(ctx: &Rc<Ctx>, d: &DetailData, node: &Node) -> gtk::Widget {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if let Some(text) = d.description.as_deref().filter(|_| node.bool_or("description", false)) {
        col.append(&gtk::Label::builder().label(text).xalign(0.0).wrap(true).lines(4).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["dim"]).build());
    }
    match (&d.empty, d.tracks.is_empty()) {
        (Some((icon, title, desc)), true) => col.append(&status(icon, title, desc)),
        _ => {
            let mut ex = d.extras.clone();
            ex.look = Look::parse(node.str_or("style", "classic"));
            ex.cover_px = node.int_or("cover", 0) as i32;
            ex.columns = node.strs("columns");
            col.append(&track_list(ctx, d.tracks.to_vec(), d.queue_kind, ex));
        }
    }
    col.upcast()
}

/// `cover-grid`: a grid of cards. Props: shape = "square" | "round", columns = n, cover = px, badge = "index" | "none".
fn cover_grid(ctx: &Rc<Ctx>, g: &GridData, node: &Node) -> gtk::Widget {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 22);
    if node.bool_or("heading", false) { col.append(&pages::heading(&g.title, "")); }
    if g.items.is_empty() {
        if let Some((icon, title, desc)) = &g.empty { col.append(&status(icon, title, desc)); }
        return col.upcast();
    }
    let cols = node.int_or("columns", 4).max(1) as i32;
    let px = node.int_or("cover", 160) as i32;
    let round_all = node.str_or("shape", "square") == "round";
    let badge = node.str_or("badge", "none") == "index";
    let grid = gtk::Grid::builder().column_spacing(18).row_spacing(24).column_homogeneous(true).css_classes(["cover-grid"]).build();
    for (i, item) in g.items.iter().enumerate() {
        grid.attach(&grid_card(ctx, item, px, round_all || item.round, badge.then(|| i + 1), node), i as i32 % cols, i as i32 / cols, 1, 1);
    }
    col.append(&grid);
    col.upcast()
}

fn grid_card(ctx: &Rc<Ctx>, item: &GridItem, px: i32, round: bool, index: Option<usize>, node: &Node) -> gtk::Widget {
    let b = gtk::Button::builder().css_classes(["flat", "card-btn", "grid-card"]).build();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.set_size_request(px, -1);
    let a = art(ctx, item.thumb.as_deref(), px);
    a.set_halign(gtk::Align::Start);
    if round { a.add_css_class("round"); }
    let ov = gtk::Overlay::builder().child(&a).halign(gtk::Align::Start).build();
    if let Some(n) = index {
        let tag = gtk::Label::builder().label(format!("{n:02}")).css_classes(["grid-badge"]).halign(gtk::Align::Start).valign(gtk::Align::End).can_target(false).build();
        ov.add_overlay(&tag);
    }
    col.append(&ov);
    let sub_ok = node.bool_or("subtitle", true);
    col.append(&gtk::Label::builder().label(&item.title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(px / 9).css_classes(["card-title"]).build());
    if sub_ok && !item.subtitle.is_empty() {
        col.append(&gtk::Label::builder().label(&item.subtitle).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(px / 9).css_classes(["card-sub"]).build());
    }
    b.set_child(Some(&col));
    let f = item.on_click.clone();
    b.connect_clicked(move |_| f());
    b.upcast()
}

/// `heading`: a page title. `word` (vocabulary key) or `text`; defaults to the page's own title (Home).
fn heading_node(env: &Env, node: &Node) -> gtk::Widget {
    let own = match env.data { Some(PageData::Home { title, .. }) => title.clone(), Some(PageData::Detail(d)) => d.title.clone(), Some(PageData::Grid(g)) => g.title.clone(), None => String::new() };
    let text = node.str("word").map(|k| env.ctx.word(k)).or_else(|| node.str("text").map(str::to_string)).unwrap_or(own);
    pages::heading(&text, node.str_or("sub", ""))
}

fn shelf_set<'a>(env: &Env<'a>) -> Option<&'a ShelfSet> {
    match env.data? { PageData::Home { shelves, .. } => Some(shelves), PageData::Detail(d) => d.shelves.as_ref(), PageData::Grid(_) => None }
}

/// `shelf` places one shelf, picked by `key` (`recent`, `favorites`, `yt:0`, `albums`, `singles`), `title` or `index`;
/// `shelves` places every shelf not placed yet, in order, minus `skip` (keys), or only `only`.
/// Both take `style = "row"` (scrolling row, default) | `"grid"` (wrapping grid), `limit` (cards), `columns` (grid), `title = false`.
fn shelves_node(env: &Env, node: &Node) -> Option<gtk::Widget> {
    let set = shelf_set(env)?;
    let mut all = set.shelves.borrow_mut();
    let picked: Vec<usize> = if node.c == "shelf" {
        let found = all.iter().position(|s| !s.used && (node.str("key").is_some_and(|k| s.key == k) || node.str("title").is_some_and(|t| s.title == t) || node.int("index").is_some_and(|i| i >= 0 && all.iter().position(|x| std::ptr::eq(x, s)) == Some(i as usize))));
        found.into_iter().collect()
    } else {
        let (skip, only) = (node.strs("skip"), node.strs("only"));
        all.iter().enumerate().filter(|(_, s)| !s.used && !skip.contains(&s.key) && (only.is_empty() || only.contains(&s.key))).map(|(i, _)| i).collect()
    };
    let col = gtk::Box::new(gtk::Orientation::Vertical, node.int_or("gap", 22) as i32);
    for i in picked {
        let sh: &mut Shelf = &mut all[i];
        sh.used = true;
        let mut cards = std::mem::take(&mut sh.cards);
        if cards.is_empty() { continue; }
        if let Some(n) = node.int("limit") { cards.truncate(n.max(1) as usize); }
        let show_title = node.bool_or("title", true);
        match node.str_or("style", "row") {
            "grid" => {
                let g = gtk::Box::new(gtk::Orientation::Vertical, 8);
                if show_title { g.append(&section_title(&sh.title)); }
                let f = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).homogeneous(true).max_children_per_line(node.int_or("columns", 8).max(1) as u32).min_children_per_line(2).column_spacing(6).row_spacing(10).build();
                for c in cards { f.append(&c); }
                g.append(&f);
                col.append(&g);
            }
            _ => col.append(&shelf(if show_title { &sh.title } else { "" }, cards)),
        }
    }
    Some(col.upcast())
}

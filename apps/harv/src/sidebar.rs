//! The sidebar, built from `shell.sidebar` of the active theme. `style = "classic"` reproduces the built-in sidebar exactly.

use crate::ctx::{Ctx, Nav};
use crate::pages::Lib;
use crate::rows::art;
use crate::window::Root;
use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use theme_core::layout::Sidebar as Cfg;

/// The built-in item order (also what `classic` shows).
const CLASSIC_ITEMS: [&str; 10] = ["home", "search", "charts", "section:section_library", "favorites", "recent", "most_played", "playlists", "following", "downloads"];

/// Item id → (icon, vocabulary key, root).
fn item(id: &str) -> Option<(&'static str, &'static str, Root)> {
    Some(match id {
        "home" => ("go-home-symbolic", "home", Root::Home),
        "search" => ("system-search-symbolic", "search", Root::Search),
        "charts" => ("view-sort-descending-symbolic", "charts", Root::Charts),
        "favorites" => ("heart-filled-symbolic", "favorites", Root::Lib(Lib::Favorites)),
        "recent" => ("document-open-recent-symbolic", "recent", Root::Lib(Lib::Recent)),
        "most_played" => ("media-playlist-repeat-symbolic", "most_played", Root::Lib(Lib::MostPlayed)),
        "playlists" => ("view-list-symbolic", "playlists", Root::Lib(Lib::Playlists)),
        "following" => ("avatar-default-symbolic", "following", Root::Lib(Lib::Following)),
        "downloads" => ("folder-download-symbolic", "downloads", Root::Lib(Lib::Downloads)),
        "now" => ("media-playback-start-symbolic", "now", Root::Now),
        _ => return None,
    })
}

/// Narrow-window state shared by every sidebar instance (a theme switch rebuilds the sidebar, the window breakpoint stays).
#[derive(Default)]
pub struct Compact {
    pub on: Cell<bool>,
    hooks: RefCell<Vec<Box<dyn Fn(bool) -> bool>>>,
}

impl Compact {
    pub fn set(&self, on: bool) {
        self.on.set(on);
        self.hooks.borrow_mut().retain(|h| h(on));
    }
}

pub struct Built {
    pub widget: gtk::Widget,
    /// Highlight the row of a root (None clears the selection).
    pub select: Rc<dyn Fn(Option<Root>)>,
}

pub fn build(ctx: &Rc<Ctx>, cfg: &Cfg, on_root: Rc<dyn Fn(Root)>, compact: &Rc<Compact>) -> Built {
    let width = ctx.theme().metrics.sidebar_w;
    let classic = cfg.style == "classic";
    let items: Vec<String> = if classic || cfg.items.is_empty() { CLASSIC_ITEMS.iter().map(|s| s.to_string()).collect() } else { cfg.items.clone() };
    let (brand, prefs_row) = if classic { (true, true) } else { (cfg.brand, cfg.prefs) };

    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    sidebar.add_css_class("sidebar");
    sidebar.set_width_request(width);
    // Children (the search hint label) may ask to expand; the sidebar itself must keep its width and leave the extra space to the page.
    sidebar.set_hexpand(false);
    let mut collapsible: Vec<gtk::Widget> = vec![];
    if brand {
        let app_name = gtk::Label::builder().label("Harv").xalign(0.0).css_classes(["app-name"]).build();
        collapsible.push(app_name.clone().upcast());
        sidebar.append(&app_name);
    }
    if !classic && cfg.search_box {
        let b = gtk::Button::builder().css_classes(["sidebar-search"]).margin_bottom(12).build();
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        h.append(&gtk::Image::from_icon_name("system-search-symbolic"));
        let l = gtk::Label::builder().label(ctx.word("search")).xalign(0.0).hexpand(true).build();
        let hint = gtk::Label::builder().label("Ctrl F").css_classes(["dim", "kbd-hint"]).build();
        collapsible.push(l.clone().upcast());
        collapsible.push(hint.clone().upcast());
        h.append(&l);
        h.append(&hint);
        b.set_child(Some(&h));
        { let on_root = on_root.clone(); b.connect_clicked(move |_| on_root(Root::Search)); }
        sidebar.append(&b);
    }

    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::Single).css_classes(["nav-list"]).vexpand(true).build();
    let mut roots: Vec<Option<Root>> = vec![];
    for id in &items {
        if let Some(key) = id.strip_prefix("section:") {
            let section = gtk::Label::builder().label(ctx.word(key)).xalign(0.0).css_classes(["nav-section"]).build();
            let row = gtk::ListBoxRow::builder().selectable(false).activatable(false).child(&section).build();
            collapsible.push(section.upcast());
            list.append(&row);
            roots.push(None);
            continue;
        }
        let Some((icon, key, root)) = item(id) else { eprintln!("theme: unknown sidebar item `{id}`"); continue };
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        h.append(&gtk::Image::from_icon_name(icon));
        let l = gtk::Label::new(Some(&ctx.word(key)));
        collapsible.push(l.clone().upcast());
        h.append(&l);
        let row = gtk::ListBoxRow::builder().child(&h).css_classes(["nav-row"]).build();
        if !classic && cfg.item_shape == "pill" { row.add_css_class("pill"); }
        list.append(&row);
        roots.push(Some(root));
    }
    let roots = Rc::new(roots);
    // Selecting a row from code must not navigate again (the caller already did).
    let quiet = Rc::new(Cell::new(false));
    {
        let (on_root, roots, quiet) = (on_root.clone(), roots.clone(), quiet.clone());
        list.connect_row_selected(move |_, row| { if quiet.get() { return; } if let Some(Some(r)) = row.and_then(|r| roots.get(r.index() as usize)) { on_root(*r); } });
    }

    // Local playlists with covers (themes that list "channels" / "collections" / "orbits" in the sidebar).
    let mut playlists_list: Option<(gtk::ListBox, Rc<RefCell<Vec<i64>>>)> = None;
    if !classic && cfg.playlist_art != "none" {
        let scroll_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let title = gtk::Label::builder().label(ctx.word(&cfg.playlists_title)).xalign(0.0).css_classes(["nav-section"]).build();
        collapsible.push(title.clone().upcast());
        scroll_box.append(&title);
        let pl = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["nav-list", "nav-playlists"]).activate_on_single_click(true).build();
        scroll_box.append(&pl);
        if cfg.new_playlist {
            let b = gtk::Button::builder().css_classes(["flat", "nav-new"]).halign(gtk::Align::Start).build();
            let l = gtk::Label::new(Some(&format!("+ {}", ctx.word("new_playlist"))));
            collapsible.push(l.clone().upcast());
            b.set_child(Some(&l));
            { let ctx = ctx.clone(); b.connect_clicked(move |_| crate::pages::new_playlist_dialog(&ctx)); }
            scroll_box.append(&b);
        }
        // The nav list keeps `vexpand`; the playlists block scrolls under it so a long list can't push Preferences away.
        list.set_vexpand(false);
        let scroll = gtk::ScrolledWindow::builder().child(&scroll_box).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        sidebar.append(&list);
        sidebar.append(&scroll);
        let ids: Rc<RefCell<Vec<i64>>> = Rc::default();
        { let (ctx, ids) = (ctx.clone(), ids.clone()); pl.connect_row_activated(move |_, row| { if let Some(id) = ids.borrow().get(row.index() as usize) { ctx.navigate(Nav::Local(*id)); } }); }
        fill_playlists(ctx, &pl, cfg.playlist_art == "round", &ids);
        playlists_list = Some((pl, ids));
    } else {
        sidebar.append(&list);
    }

    if classic || prefs_row {
        let prefs = gtk::Button::builder().css_classes(["flat", "nav-prefs"]).build();
        let ph = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        ph.append(&gtk::Image::from_icon_name("emblem-system-symbolic"));
        let pl = gtk::Label::new(Some(&ctx.word("preferences")));
        collapsible.push(pl.clone().upcast());
        ph.append(&pl);
        prefs.set_child(Some(&ph));
        { let ctx = ctx.clone(); prefs.connect_clicked(move |_| ctx.navigate(Nav::Settings)); }
        sidebar.append(&prefs);
    }

    // Narrow window: labels hide, the sidebar shrinks to icons.
    {
        let apply = {
            let (sidebar, collapsible) = (sidebar.downgrade(), collapsible.iter().map(|w| w.downgrade()).collect::<Vec<_>>());
            move |on: bool| -> bool {
                let Some(sidebar) = sidebar.upgrade() else { return false };
                for w in &collapsible { if let Some(w) = w.upgrade() { w.set_visible(!on); } }
                sidebar.set_width_request(if on { 60 } else { width });
                true
            }
        };
        apply(compact.on.get());
        compact.hooks.borrow_mut().push(Box::new(apply));
    }

    // Keep the playlists block current: refill whenever the sidebar is shown again after navigation.
    if let Some((pl, ids)) = playlists_list {
        let (ctx, round) = (ctx.clone(), cfg.playlist_art == "round");
        sidebar.connect_map(move |_| fill_playlists(&ctx, &pl, round, &ids));
    }

    let select = {
        let (list, roots, quiet) = (list.clone(), roots.clone(), quiet.clone());
        Rc::new(move |r: Option<Root>| {
            quiet.set(true);
            match r.and_then(|r| roots.iter().position(|x| *x == Some(r))) {
                Some(i) => if let Some(row) = list.row_at_index(i as i32) { list.select_row(Some(&row)); },
                None => list.unselect_all(),
            }
            quiet.set(false);
        })
    };
    Built { widget: sidebar.upcast(), select }
}

/// (Re)fill the playlists list: cover + name + "N songs"; `ids` maps row index → playlist id for activation.
fn fill_playlists(ctx: &Rc<Ctx>, list: &gtk::ListBox, round: bool, ids: &Rc<RefCell<Vec<i64>>>) {
    while let Some(c) = list.first_child() { list.remove(&c); }
    let pls = ctx.store.playlists().unwrap_or_default();
    *ids.borrow_mut() = pls.iter().map(|p| p.id).collect();
    // Subtitle template from the theme vocabulary: `{n}` = song count, `{nn}` = list position (01, 02…); empty = no subtitle.
    let fmt = ctx.word("playlist_sub");
    let fmt = if fmt == "playlist_sub" { "{n} songs".to_string() } else { fmt };
    for (i, p) in pls.iter().enumerate() {
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let cover = art(ctx, p.thumb.as_deref(), 32);
        if round { cover.add_css_class("round"); }
        h.append(&cover);
        let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
        col.set_valign(gtk::Align::Center);
        col.set_hexpand(true);
        col.append(&gtk::Label::builder().label(&p.title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["nav-pl-title"]).build());
        if !fmt.is_empty() { col.append(&gtk::Label::builder().label(fmt.replace("{nn}", &format!("{:02}", i + 1)).replace("{n}", &p.count.to_string())).xalign(0.0).css_classes(["nav-pl-sub"]).build()); }
        h.append(&col);
        list.append(&gtk::ListBoxRow::builder().child(&h).css_classes(["nav-row", "nav-pl"]).build());
    }
}

//! The command palette (Ctrl+K): jump to a page, run an action, repeat a past search, or search for what was typed. Local only: no request per keystroke.

use crate::ctx::{Ctx, Nav};
use crate::pages::Lib;
use crate::window::Root;
use adw::prelude::*;
use gtk::{gdk, glib};
use player_core::Command;
use std::cell::RefCell;
use std::rc::Rc;

thread_local! { static OPEN: RefCell<Option<adw::Dialog>> = RefCell::default(); }

struct Entry { title: String, hint: String, run: Rc<dyn Fn()> }

fn entry(title: impl Into<String>, hint: &str, run: impl Fn() + 'static) -> Entry { Entry { title: title.into(), hint: hint.to_string(), run: Rc::new(run) } }

/// Every static entry; the history and "Search for …" rows are added per query.
fn entries(ctx: &Rc<Ctx>, show: &Rc<dyn Fn(Root)>) -> Vec<Entry> {
    let mut v = Vec::new();
    let go = |ctx: &Rc<Ctx>, key: &str, root: Root| { let (show, label) = (show.clone(), ctx.word(key)); entry(label, "Go to", move || show(root)) };
    v.push(go(ctx, "home", Root::Home));
    v.push(go(ctx, "search", Root::Search));
    v.push(go(ctx, "charts", Root::Charts));
    for (key, lib) in [("favorites", Lib::Favorites), ("recent", Lib::Recent), ("most_played", Lib::MostPlayed), ("playlists", Lib::Playlists), ("following", Lib::Following), ("downloads", Lib::Downloads)] { v.push(go(ctx, key, Root::Lib(lib))); }
    let nav = |ctx: &Rc<Ctx>, title: &str, hint: &str, n: Nav| { let ctx = ctx.clone(); entry(title, hint, move || ctx.navigate(n.clone())) };
    v.push(nav(ctx, "Now playing", "Open", Nav::NowPlaying));
    v.push(nav(ctx, "Queue", "Open", Nav::Queue));
    v.push(nav(ctx, "Effects", "Open", Nav::Effects));
    v.push(nav(ctx, "Mini player", "Open", Nav::MiniPlayer));
    v.push(nav(ctx, "Preferences", "Ctrl + ,", Nav::Settings));
    let cmd = |ctx: &Rc<Ctx>, title: &str, hint: &str, c: fn() -> Command| { let ctx = ctx.clone(); entry(title, hint, move || ctx.engine.send(c())) };
    v.push(cmd(ctx, "Play / pause", "Space", || Command::TogglePause));
    v.push(cmd(ctx, "Next song", "Ctrl + →", || Command::Next));
    v.push(cmd(ctx, "Previous song", "Ctrl + ←", || Command::Prev));
    { let ctx = ctx.clone(); v.push(entry("Like the current song", "Ctrl + L", move || if let Some(t) = ctx.current_track() { ctx.toggle_like(&t); })); }
    { let ctx = ctx.clone(); v.push(entry("Keyboard shortcuts", "F1", move || crate::shortcuts::show(&ctx))); }
    v
}

/// Words typed must all appear in the title (case-insensitive); prefix matches sort first.
fn matches(all: Vec<Entry>, q: &str) -> Vec<Entry> {
    let q = q.trim().to_lowercase();
    if q.is_empty() { return all; }
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut hit: Vec<(bool, Entry)> = all.into_iter().filter_map(|e| { let t = e.title.to_lowercase(); words.iter().all(|w| t.contains(w)).then(|| (t.starts_with(&q), e)) }).collect();
    hit.sort_by_key(|(prefix, _)| !*prefix);
    hit.into_iter().map(|(_, e)| e).collect()
}

/// Open the palette, or close it when it is already open.
pub fn toggle(ctx: &Rc<Ctx>, show: &Rc<dyn Fn(Root)>, initial: &str) {
    if let Some(d) = OPEN.with(|o| o.borrow_mut().take()) { d.close(); return; }
    let dialog = adw::Dialog::builder().follows_content_size(true).build();
    dialog.add_css_class("palette");
    let search = gtk::SearchEntry::builder().placeholder_text("Go to, play, search…").hexpand(true).build();
    search.set_margin_start(12);
    search.set_margin_end(12);
    search.set_margin_top(12);
    search.set_margin_bottom(8);
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::Single).css_classes(["navigation-sidebar"]).build();
    list.set_margin_start(6);
    list.set_margin_end(6);
    list.set_margin_bottom(8);
    let scroll = gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).min_content_height(120).max_content_height(380).propagate_natural_height(true).build();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    col.set_width_request(540);
    col.append(&search);
    col.append(&scroll);
    dialog.set_child(Some(&col));
    dialog.set_focus(Some(&search));

    let shown: Rc<RefCell<Vec<Rc<dyn Fn()>>>> = Rc::default();
    let refill = Rc::new({
        let (ctx, show, list, shown, search) = (ctx.clone(), show.clone(), list.clone(), shown.clone(), search.clone());
        move || {
            let q = search.text().to_string();
            let mut found = matches(entries(&ctx, &show), &q);
            let past: Vec<String> = ctx.store.search_history(8).unwrap_or_default();
            let typed = q.trim().to_string();
            for h in past.into_iter().filter(|h| typed.is_empty() || h.to_lowercase().contains(&typed.to_lowercase())).take(5) {
                let ctx = ctx.clone();
                let hq = h.clone();
                found.push(entry(format!("Search: {h}"), "History", move || ctx.navigate(Nav::SearchFor(hq.clone()))));
            }
            if !typed.is_empty() { let ctx = ctx.clone(); let t = typed.clone(); found.push(entry(format!("Search for “{typed}”"), "Enter", move || ctx.navigate(Nav::SearchFor(t.clone())))); }
            while let Some(c) = list.first_child() { list.remove(&c); }
            let mut runs = Vec::with_capacity(found.len());
            for e in found {
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                line.set_margin_top(9);
                line.set_margin_bottom(9);
                line.set_margin_start(10);
                line.set_margin_end(10);
                line.append(&gtk::Label::builder().label(&e.title).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
                line.append(&gtk::Label::builder().label(&e.hint).css_classes(["dim"]).build());
                let row = gtk::ListBoxRow::builder().child(&line).build();
                list.append(&row);
                runs.push(e.run);
            }
            // A typed query lands on its best match; with nothing typed nothing is pre-selected until the arrow keys move.
            if let Some(first) = list.row_at_index(0) { list.select_row(Some(&first)); }
            *shown.borrow_mut() = runs;
        }
    });
    search.set_text(initial);
    search.set_position(-1);
    refill();
    search.connect_search_changed({ let refill = refill.clone(); move |_| refill() });

    let run_at = Rc::new({
        let (dialog, shown) = (dialog.clone(), shown.clone());
        move |i: usize| {
            let Some(run) = shown.borrow().get(i).cloned() else { return };
            dialog.close();
            glib::idle_add_local_once(move || run());
        }
    });
    search.connect_activate({ let (list, run_at) = (list.clone(), run_at.clone()); move |_| if let Some(r) = list.selected_row() { run_at(r.index() as usize); } });
    list.connect_row_activated({ let run_at = run_at.clone(); move |_, r| run_at(r.index() as usize) });
    // Arrow keys move the highlight while the caret stays in the entry.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let (list, search) = (list.clone(), search.clone());
        move |_, key, _, _| {
            let step = match key { gdk::Key::Down => 1, gdk::Key::Up => -1, _ => return glib::Propagation::Proceed };
            let n = { let mut c = 0; while list.row_at_index(c).is_some() { c += 1; } c };
            if n == 0 { return glib::Propagation::Stop; }
            let cur = list.selected_row().map_or(if step > 0 { -1 } else { n }, |r| r.index());
            if let Some(next) = list.row_at_index((cur + step).clamp(0, n - 1)) {
                list.select_row(Some(&next));
                next.grab_focus();
                search.grab_focus();
                search.set_position(-1);
            }
            glib::Propagation::Stop
        }
    });
    search.add_controller(keys);
    dialog.connect_closed(|_| OPEN.with(|o| *o.borrow_mut() = None));
    OPEN.with(|o| *o.borrow_mut() = Some(dialog.clone()));
    dialog.present(ctx.toasts.root().as_ref());
}

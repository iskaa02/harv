//! Main window: themed shell (sidebar · pages · right panel · bottom bar), full-screen / in-window now-playing, keyboard.

use crate::components::{self, Env, Region};
use crate::ctx::{Ctx, Nav};
use crate::pages::{self, Lib};
use crate::sidebar::{self, Compact};
use crate::{player, settings};
use adw::prelude::*;
use gtk::gdk;
use player_core::Command;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Root { Home, Search, Charts, Lib(Lib), Now }

pub struct Window {
    pub win: adw::ApplicationWindow,
    pub show_root: Rc<dyn Fn(Root)>,
}

/// Page kind (see `pages::tagged`) of the page currently on top of the navigation stack.
fn visible_kind(nav: &adw::NavigationView) -> String { nav.visible_page().map(|p| p.widget_name().to_string()).unwrap_or_default() }

pub fn build(app: &adw::Application, ctx: &Rc<Ctx>, size: (i32, i32)) -> Window {
    let nav = adw::NavigationView::new();
    nav.set_hexpand(true);
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).transition_duration(380).build();

    let win = adw::ApplicationWindow::builder().application(app).title("Harv").default_width(size.0).default_height(size.1).width_request(760).height_request(520).build();
    win.add_css_class("harv-window");

    // ---- narrow window: the sidebar collapses to icons (state shared with every sidebar instance); breakpoints are wired below
    let compact = Rc::new(Compact::default());

    // ---- root pages (Home and Search keep their state)
    let home = Rc::new(RefCell::new(pages::home(ctx)));
    let search = Rc::new(RefCell::new(pages::search(ctx, None)));
    let select_cell: Rc<RefCell<Rc<dyn Fn(Option<Root>)>>> = Rc::new(RefCell::new(Rc::new(|_| {})));
    let current: Rc<Cell<Option<Root>>> = Rc::new(Cell::new(None));
    let show_root: Rc<dyn Fn(Root)> = Rc::new({
        let (nav, ctx, home, search, select_cell, current) = (nav.clone(), ctx.clone(), home.clone(), search.clone(), select_cell.clone(), current.clone());
        move |r| {
            let page = match r { Root::Home => home.borrow().clone(), Root::Search => search.borrow().clone(), Root::Charts => pages::charts(&ctx), Root::Lib(l) => pages::library(&ctx, l), Root::Now => components::now::page(&ctx) };
            current.set(Some(r));
            nav.replace(&[page]);
            let select = select_cell.borrow().clone();
            select(Some(r));
        }
    });

    // ---- sidebar (rebuilt when the theme changes)
    let sidebar_holder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    sidebar_holder.set_hexpand(false);
    let rebuild_sidebar = Rc::new({
        let (ctx, holder, compact, show, select_cell, current) = (ctx.clone(), sidebar_holder.clone(), compact.clone(), show_root.clone(), select_cell.clone(), current.clone());
        move || {
            while let Some(c) = holder.first_child() { holder.remove(&c); }
            let theme = ctx.theme();
            let built = sidebar::build(&ctx, &theme.shell.sidebar, show.clone(), &compact);
            holder.append(&built.widget);
            (built.select)(current.get());
            *select_cell.borrow_mut() = built.select;
        }
    });
    rebuild_sidebar();

    // ---- right panel: the theme's `page.<kind>.right` for the page on top
    let panel_holder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let panel_sep = gtk::Separator::new(gtk::Orientation::Vertical);
    panel_sep.set_visible(false);
    panel_holder.set_visible(false);
    let panel_for: Rc<RefCell<String>> = Rc::default();
    // The right panel has content for this page kind / the window is too narrow for it.
    let panel_has: Rc<Cell<bool>> = Rc::default();
    let panel_squeezed: Rc<Cell<bool>> = Rc::default();
    let apply_panel_visibility: Rc<dyn Fn()> = Rc::new({
        let (holder, sep, has, squeezed) = (panel_holder.clone(), panel_sep.clone(), panel_has.clone(), panel_squeezed.clone());
        move || { let show = has.get() && !squeezed.get(); holder.set_visible(show); sep.set_visible(show); }
    });
    {
        // Under ~1100 px the page body needs the room: the right panel steps aside; under 940 px the sidebar collapses to icons.
        // libadwaita applies only ONE matching breakpoint at a time (a narrow window matches both), so the panel rule counts
        // applied breakpoints instead of trusting a single apply/unapply pair, and the sidebar rule is its own flag.
        let applied = Rc::new(Cell::new(0i32));
        let wide = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 1100px").unwrap());
        let narrow = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 940px").unwrap());
        for (bp, is_narrow) in [(&wide, false), (&narrow, true)] {
            let (n, sq, apply, c) = (applied.clone(), panel_squeezed.clone(), apply_panel_visibility.clone(), compact.clone());
            bp.connect_apply(move |_| { n.set(n.get() + 1); sq.set(true); apply(); if is_narrow { c.set(true); } });
            let (n, sq, apply, c) = (applied.clone(), panel_squeezed.clone(), apply_panel_visibility.clone(), compact.clone());
            bp.connect_unapply(move |_| { n.set(n.get() - 1); sq.set(n.get() > 0); apply(); if is_narrow { c.set(false); } });
        }
        win.add_breakpoint(wide);
        win.add_breakpoint(narrow);
    }
    let update_panel = Rc::new({
        let (ctx, nav, holder, built_for, has, apply) = (ctx.clone(), nav.clone(), panel_holder.clone(), panel_for.clone(), panel_has.clone(), apply_panel_visibility.clone());
        move |force: bool| {
            let kind = visible_kind(&nav);
            if !force && *built_for.borrow() == kind { return; }
            *built_for.borrow_mut() = kind.clone();
            while let Some(c) = holder.first_child() { holder.remove(&c); }
            let theme = ctx.theme();
            let Some(page) = theme.page.get(&kind).filter(|p| !p.right.is_empty()) else { has.set(false); apply(); return };
            let env = Env { ctx: &ctx, region: Region::Panel, data: None };
            let col = components::build_all(&env, &page.right, gtk::Orientation::Vertical, 0);
            col.add_css_class("right-panel");
            col.set_width_request(if page.right_w > 0 { page.right_w } else { 290 });
            holder.append(&col);
            has.set(true);
            apply();
        }
    });

    // ---- bottom bar (rebuilt when the theme changes; hidden on the now-playing page for `except-now`)
    let bar_holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let update_bar_visibility = Rc::new({
        let (ctx, nav, holder) = (ctx.clone(), nav.clone(), bar_holder.clone());
        move || {
            let show = ctx.theme().shell.bottom_bar.show.clone();
            holder.set_visible(match show.as_str() { "never" => false, "except-now" => visible_kind(&nav) != "now", _ => true });
        }
    });
    let rebuild_bar = Rc::new({
        let (ctx, holder, vis) = (ctx.clone(), bar_holder.clone(), update_bar_visibility.clone());
        move || {
            while let Some(c) = holder.first_child() { holder.remove(&c); }
            let theme = ctx.theme();
            holder.append(&if theme.shell.bottom_bar.style == "classic" { player::mini_bar(&ctx) } else { components::bar::bottom_bar(&ctx) });
            vis();
        }
    });
    rebuild_bar();

    // ---- browse layout
    let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    split.append(&sidebar_holder);
    split.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    split.append(&nav);
    split.append(&panel_sep);
    split.append(&panel_holder);
    let browse = gtk::Box::new(gtk::Orientation::Vertical, 0);
    split.set_vexpand(true);
    browse.append(&split);
    browse.append(&bar_holder);
    stack.add_named(&browse, Some("browse"));

    // ---- now playing: the full-screen page (classic host) is built lazily; the in-window page is built on demand
    let np: Rc<RefCell<Option<player::NowPlaying>>> = Rc::default();
    let toggle_full = { let win = win.clone(); move || if win.is_fullscreen() { win.unfullscreen() } else { win.fullscreen() } };
    let ensure_np = Rc::new({
        let (ctx, np, stack, toggle) = (ctx.clone(), np.clone(), stack.clone(), toggle_full.clone());
        move || {
            if np.borrow().is_some() { return; }
            let ctx2 = ctx.clone();
            let built = player::now_playing(&ctx, move |c| ctx2.apply_accent(Some(c)), toggle.clone());
            stack.add_named(&built.widget, Some("playing"));
            *np.borrow_mut() = Some(built);
        }
    });
    if ctx.theme().shell.now_playing.host == "fullscreen" { ensure_np(); }
    stack.set_visible_child_name("browse");
    ctx.toasts.set_child(Some(&stack));
    win.set_content(Some(&ctx.toasts));

    // ---- theme changes: rebuild what depends on the shell/layout, refresh the current page
    {
        let (ctx2, rebuild_sidebar, rebuild_bar, update_panel, ensure_np, show, search, current, home) = (ctx.clone(), rebuild_sidebar.clone(), rebuild_bar.clone(), update_panel.clone(), ensure_np.clone(), show_root.clone(), search.clone(), current.clone(), home.clone());
        let first = Cell::new(true);
        let seen_gen = Cell::new(ctx.layout_gen.get());
        ctx.on_theme(move |t| {
            if first.replace(false) { return; } // the initial values were applied above
            rebuild_sidebar();
            rebuild_bar();
            if t.shell.now_playing.host == "fullscreen" { ensure_np(); }
            *search.borrow_mut() = pages::search(&ctx2, None);
            if seen_gen.replace(ctx2.layout_gen.get()) != ctx2.layout_gen.get() { *home.borrow_mut() = pages::home(&ctx2); }
            update_panel(true);
            if let Some(r) = current.get() { show(r); }
        });
    }
    {
        let (update_panel, vis) = (update_panel.clone(), update_bar_visibility.clone());
        nav.connect_visible_page_notify(move |_| { update_panel(false); vis(); });
    }

    // ---- navigation
    {
        let (nav, stack, ctx2, win2, np2, show) = (nav.clone(), stack.clone(), ctx.clone(), win.clone(), np.clone(), show_root.clone());
        ctx.set_nav(move |n| match n {
            Nav::Root(r) => show(r),
            Nav::Album(id) => nav.push(&pages::album(&ctx2, &id)),
            Nav::Artist(id) => nav.push(&pages::artist(&ctx2, &id)),
            Nav::Playlist(id) => nav.push(&pages::playlist(&ctx2, &id)),
            Nav::Podcast(id) => nav.push(&pages::podcast(&ctx2, &id)),
            Nav::Local(id) => nav.push(&pages::local_playlist(&ctx2, id)),
            Nav::NowPlaying => {
                if ctx2.theme().shell.now_playing.host == "page" {
                    if visible_kind(&nav) != "now" { nav.push(&components::now::page(&ctx2)); }
                } else if stack.visible_child_name().as_deref() != Some("playing") {
                    open_full(&stack, &np2);
                }
            }
            Nav::CloseNowPlaying => {
                if win2.is_fullscreen() { win2.unfullscreen(); }
                if visible_kind(&nav) == "now" { nav.pop(); }
                close_full(&stack, &np2);
            }
            Nav::Queue => player::queue_dialog(&ctx2),
            Nav::Settings => settings::show(&ctx2),
            Nav::Effects => crate::effects::show(&ctx2),
            Nav::Mood(p, t) => nav.push(&pages::mood(&ctx2, &p, &t)),
            Nav::MiniPlayer => crate::miniplayer::show(&ctx2),
            Nav::SearchFor(q) => { nav.replace(&[pages::search(&ctx2, Some(&q))]); }
        });
    }

    // ---- keyboard
    // Space and Ctrl+L run in the capture phase so a focused button, row or toggle can't take Space for itself:
    // Space is play/pause everywhere, except while typing or inside a dialog.
    let capture = gtk::EventControllerKey::new();
    capture.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (win2, stack, ctx2, show, np_f) = (win.clone(), stack.clone(), ctx.clone(), show_root.clone(), np.clone());
        capture.connect_key_pressed(move |_, key, _, state| {
            let focus = gtk::prelude::GtkWindowExt::focus(&win2);
            let typing = focus.as_ref().is_some_and(|w| w.is::<gtk::Text>() || w.is::<gtk::Entry>() || w.is::<gtk::TextView>());
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            if ctrl && key == gdk::Key::k { crate::palette::toggle(&ctx2, &show, ""); return gtk::glib::Propagation::Stop; }
            if ctrl && key == gdk::Key::f { close_full(&stack, &np_f); show(Root::Search); return gtk::glib::Propagation::Stop; }
            if typing { return gtk::glib::Propagation::Proceed; }
            if ctrl && key == gdk::Key::l {
                if let Some(t) = ctx2.current_track() { ctx2.toggle_like(&t); }
                return gtk::glib::Propagation::Stop;
            }
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
            let seek = |d: f64| { let p = ctx2.engine.clock().position(); ctx2.engine.send(Command::Seek((p + d).max(0.0))); };
            match key {
                gdk::Key::Left if ctrl => { ctx2.engine.send(Command::Prev); return gtk::glib::Propagation::Stop; }
                gdk::Key::Right if ctrl => { ctx2.engine.send(Command::Next); return gtk::glib::Propagation::Stop; }
                gdk::Key::Left if shift => { seek(-5.0); return gtk::glib::Propagation::Stop; }
                gdk::Key::Right if shift => { seek(5.0); return gtk::glib::Propagation::Stop; }
                gdk::Key::Up | gdk::Key::Down if ctrl => {
                    let v = ctx2.snap.borrow().as_ref().map_or(1.0, |s| s.volume);
                    let v = (v + if key == gdk::Key::Up { 0.05 } else { -0.05 }).clamp(0.0, 1.0);
                    ctx2.engine.send(Command::SetVolume(v));
                    let _ = ctx2.store.set("volume", &v);
                    return gtk::glib::Propagation::Stop;
                }
                gdk::Key::comma if ctrl => { ctx2.navigate(Nav::Settings); return gtk::glib::Propagation::Stop; }
                gdk::Key::F1 | gdk::Key::question => { crate::shortcuts::show(&ctx2); return gtk::glib::Propagation::Stop; }
                _ => {}
            }
            let in_dialog = focus.as_ref().is_some_and(|w| w.ancestor(adw::Dialog::static_type()).is_some());
            if key == gdk::Key::space && !ctrl && !in_dialog { ctx2.engine.send(Command::TogglePause); return gtk::glib::Propagation::Stop; }
            gtk::glib::Propagation::Proceed
        });
    }
    win.add_controller(capture);
    let keys = gtk::EventControllerKey::new();
    {
        let (win2, stack, ctx2, np2, nav2) = (win.clone(), stack.clone(), ctx.clone(), np.clone(), nav.clone());
        let t = toggle_full.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            let typing = gtk::prelude::GtkWindowExt::focus(&win2).is_some_and(|w| w.is::<gtk::Text>() || w.is::<gtk::Entry>() || w.is::<gtk::TextView>());
            if typing { return gtk::glib::Propagation::Proceed; }
            let fullscreen_view = stack.visible_child_name().as_deref() == Some("playing");
            let playing_view = fullscreen_view || visible_kind(&nav2) == "now";
            let poke = || if let Some(n) = np2.borrow().as_ref() { n.view.poke(); };
            match key {
                gdk::Key::Left if playing_view => { let p = ctx2.engine.clock().position(); ctx2.engine.send(Command::Seek((p - 5.0).max(0.0))); poke(); }
                gdk::Key::Right if playing_view => { let p = ctx2.engine.clock().position(); ctx2.engine.send(Command::Seek(p + 5.0)); poke(); }
                gdk::Key::f | gdk::Key::F11 if fullscreen_view => t(),
                gdk::Key::Escape if playing_view => ctx2.navigate(Nav::CloseNowPlaying),
                _ => return gtk::glib::Propagation::Proceed,
            }
            gtk::glib::Propagation::Stop
        });
    }
    win.add_controller(keys);

    // ---- accent provider (after the static sheet, so it overrides the default)
    gtk::style_context_add_provider_for_display(&gdk::Display::default().unwrap(), &ctx.accent, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    ctx.apply_accent(None);

    show_root(Root::Home);
    Window { win, show_root }
}


/// Open: the backdrop cross-fades in, then the view's own choreography plays (cover, title, progress bar, lyric lines) and the
/// transport row fades in last (CSS transition delay). Close: the same backwards and faster, then the backdrop cross-fades out.
fn open_full(stack: &gtk::Stack, np: &Rc<RefCell<Option<player::NowPlaying>>>) {
    let Some((view, chrome)) = np.borrow().as_ref().map(|n| (n.view.clone(), n.chrome.clone())) else { return };
    for c in &chrome { c.add_css_class("np-hidden"); }
    stack.set_transition_duration(300);
    view.reveal();
    stack.set_visible_child_name("playing");
    // Release the hidden state once GTK has drawn a frame with it (two ticks), so the CSS transition has something to animate from.
    let n = std::cell::Cell::new(0u8);
    let c = chrome.clone();
    chrome[0].add_tick_callback(move |_, _| {
        n.set(n.get() + 1);
        if n.get() < 3 { return gtk::glib::ControlFlow::Continue; }
        for w in &c { w.remove_css_class("np-hidden"); }
        gtk::glib::ControlFlow::Break
    });
}

fn close_full(stack: &gtk::Stack, np: &Rc<RefCell<Option<player::NowPlaying>>>) {
    let finish = |stack: &gtk::Stack| { stack.set_transition_duration(300); stack.set_visible_child_name("browse"); };
    let parts = np.borrow().as_ref().map(|n| (n.view.clone(), n.chrome.clone()));
    let Some((view, chrome)) = parts.filter(|_| stack.visible_child_name().as_deref() == Some("playing")) else { return finish(stack) };
    for c in &chrome { c.add_css_class("np-hidden"); }
    view.conceal();
    let stack = stack.clone();
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(lyrics_view::LyricsView::CONCEAL_MS as u64), move || finish(&stack));
}

//! Theme gallery (Preferences → Appearance): one card per theme, click to switch live. Your own themes live in
//! `~/.config/harv/themes/<id>/` (see docs/THEMING_DESIGN.md); "Open themes folder" jumps there.

use crate::ctx::Ctx;
use crate::theme;
use adw::prelude::*;
use gtk::{gdk, gio, glib, pango};
use std::cell::Cell;
use std::collections::HashSet;
use std::f64::consts::{FRAC_PI_2, PI};
use std::path::Path;
use std::rc::Rc;
use theme_core::{Accent, Color, Entry, Scheme, Theme};

const SW_W: i32 = 252;
const SW_H: i32 = 144;
/// Stand-in accent for swatches when a theme follows the artwork.
const ACCENT_FALLBACK: Color = Color { r: 1.0, g: 0.22, b: 0.37, a: 1.0 };

const CSS: &str = "
.theme-card { padding: 8px; border-radius: 16px; }
.theme-card:hover { background-color: alpha(currentColor, 0.05); }
.theme-card.current { background-color: alpha(currentColor, 0.08); outline: 2px solid @accent_color; outline-offset: -2px; }
";

thread_local! { static CSS_LOADED: Cell<bool> = const { Cell::new(false) }; }

fn load_css() {
    if CSS_LOADED.with(|c| c.replace(true)) { return; }
    let p = gtk::CssProvider::new();
    p.load_from_string(CSS);
    gtk::style_context_add_provider_for_display(&gdk::Display::default().unwrap(), &p, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
}

/// The user theme folder for display (`~/.config/harv/themes`).
pub fn user_dir_label() -> String {
    let dir = theme_core::user_theme_dir();
    match std::env::var("HOME").ok().and_then(|h| dir.strip_prefix(h.trim_end_matches('/')).ok().map(|rest| format!("~/{}", rest.display().to_string().trim_start_matches('/')))) {
        Some(short) => short,
        None => dir.display().to_string(),
    }
}

/// Lower-cased family names fontconfig knows.
fn installed(ctx: &Ctx) -> HashSet<String> { ctx.toasts.pango_context().list_families().iter().map(|f| f.name().to_lowercase()).collect() }

fn font_attrs(family: &str, size: i32) -> pango::AttrList {
    let list = pango::AttrList::new();
    list.insert(pango::AttrFontDesc::new(&pango::FontDescription::from_string(&format!("{family} {size}"))));
    list
}

fn open_path(path: &Path) -> Result<(), glib::Error> {
    gio::AppInfo::launch_default_for_uri(&gio::File::for_path(path).uri(), None::<&gio::AppLaunchContext>)
}
// ── gallery ──────────────────────────────────────────────────────────────────────────────────

/// (Re)build the gallery into `holder`: one card per theme, then the "Open themes folder" button.
pub fn fill(holder: &gtk::Box, ctx: &Rc<Ctx>, dialog: &adw::PreferencesDialog) {
    load_css();
    while let Some(child) = holder.first_child() { holder.remove(&child); }
    let Some(skin) = theme::skin() else { return };
    let (current, fonts) = (skin.current_id(), installed(ctx));
    let flow = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).homogeneous(true).min_children_per_line(1).max_children_per_line(3).row_spacing(10).column_spacing(10).build();
    for e in skin.list() {
        match skin.load(&e.id) {
            Ok(t) => flow.append(&card(ctx, holder, dialog, &e, Rc::new(t), e.id == current, &fonts)),
            Err(err) => eprintln!("theme {}: {err}", e.id),
        }
    }
    holder.append(&flow);
    let folder = gtk::Button::builder().label("Open themes folder").halign(gtk::Align::Start).margin_top(6).css_classes(["flat"]).build();
    { let c = ctx.clone(); folder.connect_clicked(move |_| {
        let dir = theme_core::user_theme_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Err(err) = open_path(&dir) { c.toast(&format!("Cannot open folder: {err}")); }
    }); }
    let rescan = gtk::Button::builder().label("Rescan").tooltip_text("Pick up themes added or edited on disk").halign(gtk::Align::Start).margin_top(6).css_classes(["flat"]).build();
    { let (h, c, d) = (holder.clone(), ctx.clone(), dialog.clone()); rescan.connect_clicked(move |_| refresh_later(&h, &c, &d)); }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&folder);
    row.append(&rescan);
    holder.append(&row);
}

/// Rebuild the gallery after the current idle work (a card can't be destroyed inside its own click handler).
fn refresh_later(holder: &gtk::Box, ctx: &Rc<Ctx>, dialog: &adw::PreferencesDialog) {
    let (h, c, d) = (holder.clone(), ctx.clone(), dialog.clone());
    glib::idle_add_local_once(move || fill(&h, &c, &d));
}

fn card(ctx: &Rc<Ctx>, holder: &gtk::Box, dialog: &adw::PreferencesDialog, e: &Entry, t: Rc<Theme>, current: bool, fonts: &HashSet<String>) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.add_css_class("theme-card");
    if current { card.add_css_class("current"); }
    card.set_tooltip_text(Some(&format!("Use {}", t.meta.name)));
    card.append(&swatch(t.clone()));

    // Name in the theme's own font, with a check on the current one.
    let stack = std::iter::once(t.fonts.ui.clone()).chain(t.fonts.fallbacks.iter().cloned()).collect::<Vec<_>>().join(", ");
    let title = gtk::Label::builder().label(&t.meta.name).xalign(0.0).ellipsize(pango::EllipsizeMode::End).hexpand(true).build();
    title.set_attributes(Some(&font_attrs(&stack, 13)));
    let caption = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    caption.append(&title);
    if current { caption.append(&gtk::Image::from_icon_name("object-select-symbolic")); }
    card.append(&caption);

    // A type specimen: Latin + Arabic, so the font pair is visible (the lyrics use the same stack).
    let specimen = gtk::Label::builder().label("Aa Bb 123  ·  أبجد هوز").xalign(0.0).ellipsize(pango::EllipsizeMode::End).build();
    specimen.set_attributes(Some(&font_attrs(&stack, 11)));
    card.append(&specimen);

    // Facts: scheme · lyrics page · origin, plus a warning when the UI font is missing.
    let installed_font = fonts.contains(&t.fonts.ui.to_lowercase());
    let scheme = match t.meta.scheme { Scheme::Light => "Light", Scheme::Dark => "Dark" };
    let lyrics = if t.shell.now_playing.host == "fullscreen" { "Full-screen lyrics" } else { "In-window player" };
    let facts = format!("{scheme} · {lyrics} · {}{}", if e.builtin { "Built-in" } else { "Yours" }, if installed_font { String::new() } else { format!(" · {} not installed", t.fonts.ui) });
    card.append(&gtk::Label::builder().label(&facts).xalign(0.0).ellipsize(pango::EllipsizeMode::End).css_classes(["dim-label", "caption"]).build());

    let click = gtk::GestureClick::new();
    { let (c, h, d, id) = (ctx.clone(), holder.clone(), dialog.clone(), e.id.clone()); click.connect_released(move |_, n, _, _| { if n == 1 { activate(&c, &h, &d, &id); } }); }
    card.add_controller(click);
    card.set_cursor_from_name(Some("pointer"));
    card
}

fn activate(ctx: &Rc<Ctx>, holder: &gtk::Box, dialog: &adw::PreferencesDialog, id: &str) {
    let Some(skin) = theme::skin() else { return };
    match skin.switch(ctx, id) {
        Ok(()) => { let _ = ctx.store.set("theme", &id.to_string()); }
        Err(e) => { ctx.toast(&format!("Theme error: {e}")); return; }
    }
    refresh_later(holder, ctx, dialog);
}

/// Swatch: a schematic of the app drawn from the theme's tokens (window, sidebar, rows, accent, bottom bar, radii).
fn swatch(t: Rc<Theme>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(SW_W);
    area.set_content_height(SW_H);
    area.set_draw_func(move |_, cr, w, h| paint_swatch(&t, cr, w as f64, h as f64));
    area
}

fn set_rgba(cr: &gtk::cairo::Context, c: Color, alpha: f64) { cr.set_source_rgba(c.r as f64, c.g as f64, c.b as f64, c.a as f64 * alpha); }

/// Rounded rectangle path, radius clamped to the box.
fn rrect(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.clamp(0.0, (w / 2.0).min(h / 2.0));
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

fn paint_swatch(t: &Theme, cr: &gtk::cairo::Context, w: f64, h: f64) {
    let (c, s) = (&t.colors, &t.shape);
    let accent = match c.accent { Accent::Fixed(a) => a, Accent::Cover => ACCENT_FALLBACK };
    rrect(cr, 0.0, 0.0, w, h, 14.0);
    cr.clip();
    set_rgba(cr, c.bg, 1.0);
    cr.paint().ok();
    let bar_h = h * 0.17;
    let body_h = h - bar_h;
    let side = w * 0.27;
    // Sidebar: five items, the first selected.
    set_rgba(cr, c.sidebar, 1.0);
    cr.rectangle(0.0, 0.0, side, body_h);
    cr.fill().ok();
    for i in 0..5 {
        let y = 14.0 + i as f64 * 14.0;
        if i == 0 { set_rgba(cr, accent, 1.0); } else { set_rgba(cr, c.fg, 0.32); }
        rrect(cr, 8.0, y, side - 16.0, 8.0, (s.md as f64).min(4.0));
        cr.fill().ok();
    }
    // Main: a heading and three rows (the first highlighted), each with a cover and two text bars.
    let (x0, x1) = (side + 10.0, w - 10.0);
    set_rgba(cr, c.fg, 0.9);
    rrect(cr, x0, 12.0, (x1 - x0) * 0.4, 7.0, (s.sm as f64).min(3.5));
    cr.fill().ok();
    for i in 0..3 {
        let y = 28.0 + i as f64 * 24.0;
        set_rgba(cr, if i == 0 { c.hover } else { c.raise }, 1.0);
        rrect(cr, x0, y, x1 - x0, 18.0, (s.lg as f64).min(9.0));
        cr.fill().ok();
        set_rgba(cr, if i == 0 { accent } else { c.dim }, 0.85);
        rrect(cr, x0 + 4.0, y + 3.0, 12.0, 12.0, (s.cover as f64).min(4.0));
        cr.fill().ok();
        set_rgba(cr, c.fg, 0.9);
        rrect(cr, x0 + 22.0, y + 4.5, (x1 - x0) * 0.42, 3.5, 1.5);
        cr.fill().ok();
        set_rgba(cr, c.dim, 0.9);
        rrect(cr, x0 + 22.0, y + 10.5, (x1 - x0) * 0.26, 3.0, 1.5);
        cr.fill().ok();
    }
    // Bottom bar: play button, progress line in the accent.
    set_rgba(cr, c.bar_bg, 1.0);
    cr.rectangle(0.0, body_h, w, bar_h);
    cr.fill().ok();
    set_rgba(cr, c.play_bg, 1.0);
    cr.arc(w * 0.5, body_h + bar_h / 2.0, 6.0, 0.0, 2.0 * PI);
    cr.fill().ok();
    set_rgba(cr, c.bar_fg, 0.25);
    rrect(cr, x0, h - 6.0, x1 - x0, 2.0, 1.0);
    cr.fill().ok();
    set_rgba(cr, accent, 1.0);
    rrect(cr, x0, h - 6.0, (x1 - x0) * 0.4, 2.0, 1.0);
    cr.fill().ok();
}

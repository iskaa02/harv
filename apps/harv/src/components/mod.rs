//! Layout engine: turns the theme's component trees (`theme_core::Node`) into widgets.
//!
//! * `data`   — what a page hands to its layout (`PageData`: song-list pages, grid pages).
//! * `detail` — page composition (`compose`), plus the page-body components `hero`, `track-list`, `cover-grid`.
//! * `panel`  — right-panel components: `queue-list`, `lyrics-teaser`.
//! * `bar`    — the bottom bar (`bottom_bar`) and its parts: `cover`, `title`, `transport`, `seek`, `volume`, `text`, `link`.
//! * `now`    — the in-window now-playing page (`page`) and its components.
//! * `search` — the themed search page.
//!
//! Each module exposes `component(env, node) -> Option<Widget>` that handles the component names it owns and returns `None`
//! for any other name; `build` tries them in turn. Containers (`row`, `column`, `scroll`, `spacer`, `text`) live here.
//! Unknown components are skipped with a warning on stderr (a theme typo must never blank the window).

use crate::ctx::Ctx;
use gtk::prelude::*;
use std::rc::Rc;
use theme_core::Node;

pub mod bar;
pub mod data;
pub mod detail;
pub mod now;
pub mod panel;
pub mod search;

pub use data::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Region { Page, Panel, Bar }

/// Everything a component may need while being built.
pub struct Env<'a> {
    pub ctx: &'a Rc<Ctx>,
    pub region: Region,
    /// The data of the page being composed (None in the shell: panel, bar, now-playing).
    pub data: Option<&'a PageData>,
}

/// Build one node (and its children), then apply the generic layout props.
pub fn build(env: &Env, node: &Node) -> Option<gtk::Widget> {
    let w = container(env, node)
        .or_else(|| detail::component(env, node))
        .or_else(|| panel::component(env, node))
        .or_else(|| bar::component(env, node))
        .or_else(|| now::component(env, node));
    match w {
        Some(w) => { decorate(&w, node); Some(w) }
        None => { eprintln!("theme: component `{}` is unknown or not available here ({:?})", node.c, env.region); None }
    }
}

/// Build `nodes` into a box (components that fail to build are skipped).
pub fn build_all(env: &Env, nodes: &[Node], orientation: gtk::Orientation, gap: i32) -> gtk::Box {
    let b = gtk::Box::new(orientation, gap);
    for n in nodes { if let Some(w) = build(env, n) { b.append(&w); } }
    b
}

fn orientation(s: &str) -> gtk::Orientation { if s == "row" { gtk::Orientation::Horizontal } else { gtk::Orientation::Vertical } }

fn container(env: &Env, node: &Node) -> Option<gtk::Widget> {
    match node.c.as_str() {
        "row" | "column" => Some(build_all(env, &node.children, orientation(&node.c), node.int_or("gap", 0) as i32).upcast()),
        "spacer" => Some(gtk::Box::builder().hexpand(true).vexpand(node.bool_or("vgrow", false)).build().upcast()),
        // A vertically scrollable column (right panels with long lists).
        "scroll" => {
            let inner = build_all(env, &node.children, gtk::Orientation::Vertical, node.int_or("gap", 0) as i32);
            Some(gtk::ScrolledWindow::builder().child(&inner).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build().upcast())
        }
        // Small text: `text = "literal"` or `word = "<vocabulary key>"`; `class` adds CSS classes (e.g. `dim`, `section-label`).
        "text" => {
            let label = node.str("word").map(|k| env.ctx.word(k)).or_else(|| node.str("text").map(str::to_string)).unwrap_or_default();
            Some(gtk::Label::builder().label(label).xalign(node.float_or("xalign", 0.0) as f32).wrap(node.bool_or("wrap", false)).build().upcast())
        }
        _ => None,
    }
}

fn align(s: &str) -> gtk::Align { match s { "start" => gtk::Align::Start, "center" => gtk::Align::Center, "end" => gtk::Align::End, _ => gtk::Align::Fill } }

/// Generic props every component accepts: `width`, `height`, `grow`, `vgrow`, `halign`, `valign`, `margin[_top|_bottom|_start|_end]`, `class`.
fn decorate(w: &gtk::Widget, n: &Node) {
    let (width, height) = (n.int_or("width", -1) as i32, n.int_or("height", -1) as i32);
    if width >= 0 || height >= 0 { w.set_size_request(width, height); }
    if let Some(g) = n.bool("grow") { w.set_hexpand(g); }
    if let Some(g) = n.bool("vgrow") { w.set_vexpand(g); }
    if let Some(a) = n.str("halign") { w.set_halign(align(a)); }
    if let Some(a) = n.str("valign") { w.set_valign(align(a)); }
    if let Some(m) = n.int("margin") { let m = m as i32; w.set_margin_top(m); w.set_margin_bottom(m); w.set_margin_start(m); w.set_margin_end(m); }
    if let Some(m) = n.int("margin_top") { w.set_margin_top(m as i32); }
    if let Some(m) = n.int("margin_bottom") { w.set_margin_bottom(m as i32); }
    if let Some(m) = n.int("margin_start") { w.set_margin_start(m as i32); }
    if let Some(m) = n.int("margin_end") { w.set_margin_end(m as i32); }
    if let Some(c) = n.str("class") { for c in c.split_whitespace() { w.add_css_class(c); } }
}

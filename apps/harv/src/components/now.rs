//! The in-window now-playing page (`shell.now_playing.host = "page"`), built from `page.now`, and its components.
//!
//! State-driven parts subscribe through [`on_key`]: the closure runs only when its key changes, and the hook removes itself
//! once the widget is gone, so a page left in the navigation stack costs nothing and a rebuilt page leaves no stale hooks.
//! Closures store widgets only as `WeakRef`s (a strong capture would keep the hook, and the widget, alive forever).
//! Progress-driven parts tick only while playing and mapped.

use super::{build, build_all, Env};
use crate::ctx::{Ctx, Nav};
use crate::pages;
use crate::rows::{fmt_time, set_art};
use adw::prelude::*;
use gtk::{gdk, pango};
use lyrics_view::{Align, LyricsStyle, LyricsView, Meta, TextColors};
use player_core::{Command, Snapshot, Status};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use theme_core::Node;
use ytm_core::Track;

/// The now-playing page as a navigation page (tagged `now`): the theme's `page.now.main` tree under a flat header.
pub fn page(ctx: &Rc<Ctx>) -> adw::NavigationPage {
    let theme = ctx.theme();
    let env = Env { ctx, region: super::Region::Page, data: None };
    let body: gtk::Widget = match theme.page.get("now") {
        Some(p) if !p.main.is_empty() => {
            let b = build_all(&env, &p.main, gtk::Orientation::Vertical, 0);
            b.set_hexpand(true);
            b.set_vexpand(true);
            b.add_css_class("nowp-page");
            b.upcast()
        }
        _ => gtk::Label::builder().label("Nothing to show: the theme has no page.now layout").css_classes(["dim"]).vexpand(true).build().upcast(),
    };
    let header = adw::HeaderBar::new();
    header.add_css_class("flat");
    header.add_css_class("nowp-header");
    // The ↓ button below is this page's back button; without this libadwaita adds a second ← one when a page is underneath.
    header.set_show_back_button(false);
    header.set_title_widget(Some(&gtk::Label::new(None)));
    let back = gtk::Button::builder().icon_name("go-down-symbolic").tooltip_text("Back (Esc)").css_classes(["flat"]).build();
    { let ctx = ctx.clone(); back.connect_clicked(move |_| ctx.navigate(Nav::CloseNowPlaying)); }
    header.pack_start(&back);
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&body));
    let page = adw::NavigationPage::builder().title("Now playing").child(&tv).build();
    pages::tagged(page, "now")
}

pub fn component(env: &Env, node: &Node) -> Option<gtk::Widget> {
    Some(match node.c.as_str() {
        "now-cover" => now_cover(env, node),
        "now-title" => now_title(env, node),
        "now-counter" => now_counter(env, node),
        "lyrics" => lyrics(env, node),
        "tabs" => tabs(env, node),
        "credits" => credits(env, node),
        "next-card" => next_card(env, node),
        _ => return None,
    })
}

// ── shared plumbing ──────────────────────────────────────────────────────────────────────────

fn cur_track(s: &Snapshot) -> Option<&Track> { s.current.and_then(|i| s.queue.get(i)) }

fn cur_id(s: &Snapshot) -> String { cur_track(s).map_or(String::new(), |t| t.video_id.clone()) }

/// Runs `f` once up front and again whenever `key(snapshot)` changes. Drops itself when `root` is gone.
fn on_key(ctx: &Rc<Ctx>, root: &impl IsA<gtk::Widget>, key: impl Fn(&Snapshot) -> String + 'static, f: impl Fn(&Snapshot) + 'static) {
    let weak = root.upcast_ref::<gtk::Widget>().downgrade();
    let last: RefCell<Option<String>> = RefCell::new(None);
    ctx.on_state_while(move |s| {
        if weak.upgrade().is_none() { return false; }
        let k = key(s);
        if last.borrow().as_deref() != Some(k.as_str()) {
            *last.borrow_mut() = Some(k);
            f(s);
        }
        true
    });
}

thread_local! { static RADII: RefCell<HashSet<i32>> = RefCell::new(HashSet::new()); }

/// A CSS class that rounds corners by `r` px (one provider per distinct radius, kept for the app's lifetime).
fn radius_class(r: i32) -> String {
    let name = format!("nowp-r{r}");
    RADII.with(|set| {
        if set.borrow_mut().insert(r) {
            let p = gtk::CssProvider::new();
            p.load_from_string(&format!(".{name} {{ border-radius: {r}px; }}"));
            gtk::style_context_add_provider_for_display(&gdk::Display::default().unwrap(), &p, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2);
        }
    });
    name
}

/// Label with a CSS class and an optional absolute font size (px), so a theme can size it inline.
fn text_label(class: &str, px: Option<i64>, xalign: f32) -> gtk::Label {
    let l = gtk::Label::builder().xalign(xalign).ellipsize(pango::EllipsizeMode::End).css_classes([class]).build();
    if let Some(px) = px {
        let attrs = pango::AttrList::new();
        attrs.insert(pango::AttrSize::new_size_absolute(px as i32 * pango::SCALE));
        l.set_attributes(Some(&attrs));
    }
    l
}

// ── now-cover ────────────────────────────────────────────────────────────────────────────────
/// `size` px square. `shape = "circle"`, `radius` (px), `shadow`.
fn now_cover(env: &Env, node: &Node) -> gtk::Widget {
    let ctx = env.ctx;
    let size = node.int_or("size", 300) as i32;
    let radius = if node.str("shape") == Some("circle") { size / 2 } else { node.int("radius").map_or(ctx.theme().shape.cover as i32, |r| r as i32) };
    let img = gtk::Image::new();
    img.set_pixel_size(size);
    img.set_size_request(size, size);
    img.set_overflow(gtk::Overflow::Hidden);
    img.set_halign(gtk::Align::Start); // a neighbour (the seek bar) may expand the column; the cover keeps its size
    img.add_css_class("nowp-art");
    img.add_css_class(&radius_class(radius));
    if node.bool_or("shadow", false) { img.add_css_class("nowp-shadow"); }

    let track_img = img.downgrade();
    let c3 = ctx.clone();
    on_key(ctx, &img, cur_id, move |s| {
        let Some(img) = track_img.upgrade() else { return };
        set_art(&c3, &img, cur_track(s).and_then(|t| t.thumb.as_deref()), size);
    });
    img.upcast()
}

// ── now-title ────────────────────────────────────────────────────────────────────────────────
/// Title + "artist · album" lines. `size` / `sub_size` in px, `align = "center"`.
fn now_title(env: &Env, node: &Node) -> gtk::Widget {
    let ctx = env.ctx;
    let xalign = if node.str_or("align", "start") == "center" { 0.5 } else { 0.0 };
    let title = text_label("nowp-title", node.int("size"), xalign);
    let sub = text_label("nowp-sub", node.int("sub_size"), xalign);
    let root = gtk::Box::new(gtk::Orientation::Vertical, node.int_or("gap", 4) as i32);
    title.set_hexpand(true);
    if node.bool_or("actions", true) {
        // Title with heart + ⋯ at its end, so the current song can always be liked from here.
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&title);
        head.append(&crate::actions::current_actions(ctx, &["flat", "circular", "nowp-act"]));
        root.append(&head);
    } else {
        root.append(&title);
    }
    root.append(&sub);
    let (t, s) = (title.downgrade(), sub.downgrade());
    on_key(ctx, &root, cur_id, move |snap| {
        let (Some(t), Some(s)) = (t.upgrade(), s.upgrade()) else { return };
        match cur_track(snap) {
            Some(tr) => {
                t.set_label(&tr.title);
                let mut parts = vec![tr.artist_line()];
                if let Some(al) = tr.album.as_deref().filter(|a| !a.is_empty()) { parts.push(al.to_string()); }
                s.set_label(&parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" · "));
            }
            None => { t.set_label(""); s.set_label(""); }
        }
    });
    root.upcast()
}

// ── now-counter ──────────────────────────────────────────────────────────────────────────────
/// `format` with `{n}` (1-based track index), `{m}` (queue length), `{next}` (tracks after this one), e.g.
/// "NOW PLAYING · TRACK {n} / {m}".
fn counter_text(format: &str, s: &Snapshot) -> String {
    let Some(i) = s.current else { return String::new() };
    let (n, m) = (i + 1, s.queue.len());
    format.replace("{n}", &format!("{n:02}")).replace("{m}", &format!("{m:02}")).replace("{next}", &format!("{}", m.saturating_sub(n)))
}

fn now_counter(env: &Env, node: &Node) -> gtk::Widget {
    let ctx = env.ctx;
    let format = node.str_or("format", "NOW PLAYING · TRACK {n} / {m}").to_string();
    let l = text_label("nowp-counter", None, 0.0);
    let w = l.downgrade();
    let f2 = format.clone();
    on_key(ctx, &l, move |s| counter_text(&format, s), move |s| { if let Some(l) = w.upgrade() { l.set_label(&counter_text(&f2, s)); } });
    l.upcast()
}

// ── lyrics ───────────────────────────────────────────────────────────────────────────────────
#[derive(Clone)]
struct LyricsOpts { font: f32, active_font: f32, gap: f32, pad: Option<f32>, align: Align, colors: String, accent_active: bool }

/// Preferences → Appearance: lyrics text size / line spacing (page-hosted themes), as multipliers on the theme's values.
pub const LYRICS_SIZES: [(&str, f32); 3] = [("Small", 0.85), ("Default", 1.0), ("Large", 1.2)];
pub const LYRICS_GAPS: [(&str, f32); 3] = [("Tight", 0.6), ("Default", 1.0), ("Roomy", 1.5)];

thread_local! { static RESTYLE: RefCell<Vec<Box<dyn Fn() -> bool>>> = const { RefCell::new(Vec::new()) }; }

/// Re-apply the lyrics text settings to every open lyrics view (dropped views unregister themselves).
pub fn restyle_lyrics() {
    let hooks = RESTYLE.with(|r| std::mem::take(&mut *r.borrow_mut()));
    let alive: Vec<_> = hooks.into_iter().filter(|h| h()).collect();
    RESTYLE.with(|r| r.borrow_mut().extend(alive));
}

#[allow(deprecated)] // StyleContext::lookup_color: the only way to read a named CSS colour from Rust
fn lyrics_style(ctx: &Ctx, view: &LyricsView, o: &LyricsOpts) -> LyricsStyle {
    let theme = ctx.theme();
    let mut st = LyricsStyle::plain();
    let (size_k, gap_k) = (LYRICS_SIZES[ctx.store.get::<usize>("lyrics_size").unwrap_or(1).min(2)].1, LYRICS_GAPS[ctx.store.get::<usize>("lyrics_gap").unwrap_or(1).min(2)].1);
    st.font_px = Some(o.font * size_k);
    st.active_font_px = Some(o.active_font * size_k);
    st.line_gap = Some(o.gap * gap_k);
    st.pad = o.pad;
    st.align = o.align;
    st.font_family = Some(theme.fonts.ui.clone());
    let rgba = |c: [f32; 3], a: f32| [c[0], c[1], c[2], a];
    // "np": the full-screen palette (light text on the dark now-playing page). "theme": the page's own fg / dim.
    let (fg, dim) = if o.colors == "np" { (theme.colors.np_fg.rgb_f32(), theme.colors.np_fg.rgb_f32()) } else { (theme.colors.fg.rgb_f32(), theme.colors.dim.rgb_f32()) };
    // The live accent is a named CSS colour (`lib_accent`), so it follows the user's and the cover's accent.
    let accent = if o.accent_active { view.style_context().lookup_color("lib_accent").map(|c| [c.red(), c.green(), c.blue()]) } else { None };
    // "np" steps the light text down by alpha (near ~ #8E9094, far ~ #4A4C51 on the dark page).
    let (near_alpha, far_alpha) = if o.colors == "np" { (0.55, 0.28) } else { (1.0, 0.55) };
    st.colors = TextColors { active: rgba(accent.unwrap_or(fg), 1.0), near: rgba(dim, near_alpha), far: rgba(dim, far_alpha) };
    st
}

/// Plain lyrics (no blur/glow/backdrop) following the audio clock. Click a line to seek. Options: `font`, `active_font`,
/// `gap`, `pad` (px), `align` (`start`|`center`), `colors` (`theme`|`np`), `accent_active` (bool).
fn lyrics(env: &Env, node: &Node) -> gtk::Widget {
    let ctx = env.ctx;
    let o = LyricsOpts {
        font: node.float_or("font", 20.0) as f32,
        active_font: node.float_or("active_font", 28.0) as f32,
        gap: node.float_or("gap", 16.0) as f32,
        pad: node.float("pad").map(|p| p as f32),
        align: if node.str_or("align", "start") == "center" { Align::Center } else { Align::Start },
        colors: node.str_or("colors", "theme").to_string(),
        accent_active: node.bool_or("accent_active", false),
    };
    let view = LyricsView::new();
    view.set_style(lyrics_style(ctx, &view, &o));
    {
        let clock = ctx.engine.clock();
        view.set_position_source(move || clock.position() as f32);
        let c = ctx.clone();
        view.connect_seek(move |t| c.engine.send(Command::Seek(t as f64)));
    }
    view.set_hexpand(true);
    view.set_vexpand(true);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&view));
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    let note = gtk::Label::builder().css_classes(["nowp-lyrics-note"]).halign(gtk::Align::Center).valign(gtk::Align::Center).visible(false).build();
    note.set_can_target(false);
    overlay.add_overlay(&note);
    let root: gtk::Widget = overlay.upcast();

    let (wv, wn) = (view.downgrade(), note.downgrade());
    let (c_track, o_track) = (ctx.clone(), o.clone());
    on_key(ctx, &root, cur_id, move |s| {
        let (Some(view), Some(note)) = (wv.upgrade(), wn.upgrade()) else { return };
        view.set_style(lyrics_style(&c_track, &view, &o_track));
        let Some(t) = cur_track(s) else {
            view.set_track(Meta::default(), None, None, 0.0);
            note.set_visible(false);
            return;
        };
        let duration = s.duration.max(t.duration_s.unwrap_or(0) as f64) as f32;
        view.set_track(Meta { title: t.title.clone(), artist: t.artist_line(), album: t.album.clone().unwrap_or_default() }, None, None, duration);
        view.set_offset(c_track.lyrics.offset_ms(&t.video_id) as f32 / 1000.0);
        note.set_label("Loading lyrics…");
        note.set_visible(true);
        let (svc, tr, id, c2) = (c_track.lyrics.clone(), t.clone(), t.video_id.clone(), c_track.clone());
        let (wv3, wn3) = (wv.clone(), wn.clone());
        c_track.spawn(async move { svc.lyrics(&tr, false).await.map_err(|e| format!("{e:#}")) }, move |res| {
            // Ignore the answer if the song changed meanwhile (or the page is gone).
            if c2.current_track().is_none_or(|c| c.video_id != id) { return; }
            let (Some(view), Some(note)) = (wv3.upgrade(), wn3.upgrade()) else { return };
            match res {
                Ok(Some(l)) => { note.set_visible(false); view.set_lyrics(Some(l)); }
                Ok(None) => { note.set_label("No lyrics for this song"); note.set_visible(true); view.set_lyrics(None); }
                Err(e) => { note.set_label(&format!("Couldn't load lyrics\n{e}")); note.set_visible(true); view.set_lyrics(None); }
            }
        });
    });
    {
        let (wv, c, o) = (view.downgrade(), ctx.clone(), o.clone());
        RESTYLE.with(|r| r.borrow_mut().push(Box::new(move || {
            let Some(view) = wv.upgrade() else { return false };
            view.set_style(lyrics_style(&c, &view, &o));
            true
        })));
    }
    let wv2 = view.downgrade();
    ctx.on_state_while(move |s| {
        let Some(view) = wv2.upgrade() else { return false };
        view.set_playing(s.status == Status::Playing);
        if s.duration > 0.0 { view.set_duration(s.duration as f32); }
        true
    });
    root
}

// ── tabs ─────────────────────────────────────────────────────────────────────────────────────
/// A tab strip over its children (one page each). `labels = ["Lyrics", "Credits", "Queue"]`, underline accent on the
/// active tab (CSS `.now-tab:checked`).
fn tabs(env: &Env, node: &Node) -> gtk::Widget {
    let labels = node.strs("labels");
    let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
    root.set_vexpand(true);
    root.set_hexpand(true);
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    strip.add_css_class("nowp-tabs");
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).transition_duration(160).vexpand(true).build();
    let mut first: Option<gtk::ToggleButton> = None;
    for (i, child) in node.children.iter().enumerate() {
        let Some(w) = build(env, child) else { continue };
        let name = format!("tab{i}");
        stack.add_named(&w, Some(&name));
        let label = labels.get(i).cloned().unwrap_or_else(|| format!("Tab {}", i + 1));
        let b = gtk::ToggleButton::builder().label(label).css_classes(["flat", "nowp-tab"]).build();
        match &first {
            Some(f) => b.set_group(Some(f)),
            None => { b.set_active(true); first = Some(b.clone()); }
        }
        { let (stack, name) = (stack.clone(), name.clone()); b.connect_toggled(move |t| if t.is_active() { stack.set_visible_child_name(&name) }); }
        strip.append(&b);
    }
    root.append(&strip);
    root.append(&stack);
    root.upcast()
}

// ── credits ──────────────────────────────────────────────────────────────────────────────────
/// Key/value grid for what the track really carries (artist, album, length). Rows without data are left out, and the
/// whole block hides when nothing is known. (ytm-core exposes no writer/producer credits.)
fn credits(env: &Env, _node: &Node) -> gtk::Widget {
    let ctx = env.ctx;
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("nowp-credits");
    root.set_vexpand(false);
    let w = root.downgrade();
    on_key(ctx, &root, cur_id, move |s| {
        let Some(root) = w.upgrade() else { return };
        while let Some(c) = root.first_child() { root.remove(&c); }
        let mut rows: Vec<(&str, String)> = vec![];
        if let Some(t) = cur_track(s) {
            let artist = t.artist_line();
            if !artist.is_empty() { rows.push(("Artist", artist)); }
            if let Some(al) = t.album.as_deref().filter(|a| !a.is_empty()) { rows.push(("Album", al.to_string())); }
            if let Some(d) = t.duration_s { rows.push(("Length", fmt_time(d))); }
        }
        root.set_visible(!rows.is_empty());
        if rows.is_empty() { return; }
        let grid = gtk::Grid::builder().column_spacing(16).row_spacing(8).build();
        for (i, (k, v)) in rows.into_iter().enumerate() {
            let kl = text_label("nowp-credit-key", None, 0.0);
            kl.set_label(k);
            let vl = text_label("nowp-credit-val", None, 0.0);
            vl.set_label(&v);
            vl.set_hexpand(true);
            grid.attach(&kl, 0, i as i32, 1, 1);
            grid.attach(&vl, 1, i as i32, 1, 1);
        }
        root.append(&grid);
    });
    root.upcast()
}

// ── next-card ────────────────────────────────────────────────────────────────────────────────
/// "NEXT · 2:26 / title — artist / Open queue →". `label` and `button` are the captions. Hidden when nothing is queued.
fn next_card(env: &Env, node: &Node) -> gtk::Widget {
    let ctx = env.ctx;
    let label = node.str_or("label", "NEXT").to_string();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("nowp-next");
    let img = gtk::Image::new();
    img.set_pixel_size(40);
    img.set_size_request(40, 40);
    img.set_overflow(gtk::Overflow::Hidden);
    img.add_css_class("nowp-art");
    img.add_css_class(&radius_class(4));
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    col.set_hexpand(true);
    col.set_valign(gtk::Align::Center);
    let head = text_label("nowp-next-label", None, 0.0);
    let title = text_label("nowp-next-title", None, 0.0);
    col.append(&head);
    col.append(&title);
    let open = gtk::Button::builder().label(node.str_or("button", "Open queue →")).css_classes(["flat", "nowp-next-open"]).valign(gtk::Align::Center).build();
    { let c = ctx.clone(); open.connect_clicked(move |_| c.navigate(Nav::Queue)); }
    row.append(&img);
    row.append(&col);
    row.append(&open);

    let (wr, wi, wh, wt) = (row.downgrade(), img.downgrade(), head.downgrade(), title.downgrade());
    let c2 = ctx.clone();
    on_key(ctx, &row, |s| format!("{}{}", s.endless, s.current.and_then(|i| s.queue.get(i + 1)).map_or(String::new(), |t| t.video_id.clone())), move |s| {
        let (Some(row), Some(img), Some(head), Some(title)) = (wr.upgrade(), wi.upgrade(), wh.upgrade(), wt.upgrade()) else { return };
        match s.current.and_then(|i| s.queue.get(i + 1)) {
            Some(n) => {
                row.set_visible(true);
                set_art(&c2, &img, n.thumb.as_deref(), 40);
                let d = n.duration_s.map(fmt_time).unwrap_or_default();
                head.set_label(&if d.is_empty() { label.clone() } else { format!("{label} · {d}") });
                title.set_label(&format!("{} — {}", n.title, n.artist_line()));
            }
            None => {
                // Keep the card (same height) so the lyrics area doesn't grow and shrink with the queue.
                row.set_visible(true);
                img.set_paintable(None::<&gdk::Paintable>);
                head.set_label(&label);
                title.set_label(if s.endless { "Endless radio will add more" } else { "End of the queue" });
            }
        }
    });
    row.upcast()
}

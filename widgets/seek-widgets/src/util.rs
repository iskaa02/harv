//! Shared colour and path helpers.

use gtk::{gdk, graphene, gsk, prelude::*};

/// `(accent, dim)`: the widget's CSS `color` (or the override), and the same colour at 25 % alpha (or the override).
pub fn palette(w: &impl IsA<gtk::Widget>, accent: Option<gdk::RGBA>, dim: Option<gdk::RGBA>) -> (gdk::RGBA, gdk::RGBA) {
    let a = accent.unwrap_or_else(|| w.color());
    let d = dim.unwrap_or_else(|| with_alpha(&a, a.alpha() * 0.25));
    (a, d)
}

pub fn with_alpha(c: &gdk::RGBA, a: f32) -> gdk::RGBA {
    gdk::RGBA::new(c.red(), c.green(), c.blue(), a)
}

pub fn rect(x: f32, y: f32, w: f32, h: f32) -> graphene::Rect {
    graphene::Rect::new(x, y, w, h)
}

pub fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> gsk::RoundedRect {
    let s = graphene::Size::new(r, r);
    gsk::RoundedRect::new(rect(x, y, w, h), s, s, s, s)
}

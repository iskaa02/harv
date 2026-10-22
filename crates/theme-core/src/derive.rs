//! Palette derivation on raw TOML tables: a theme with `derive = true` in `[colors]` only has to give `bg`, `fg` (and usually
//! `accent`); every colour it leaves out is computed from those, and the opposite-scheme palette is generated when missing.
//! Runs per theme file *before* `extends` is merged, so derived colours replace (not inherit) the parent's.

use crate::Color;
use toml::{Table, Value};

fn get(p: &Table, k: &str) -> Option<Color> { p.get(k)?.as_str().and_then(|s| Color::parse(s).ok()) }

/// Fill in every missing colour of `p` from its `bg` / `fg` / `accent`. Does nothing unless both `bg` and `fg` are set.
pub fn palette(p: &mut Table) {
    let (Some(bg), Some(fg)) = (get(p, "bg"), get(p, "fg")) else { return };
    let dark = bg.luma() < 0.18;
    let accent = get(p, "accent");
    let put = |p: &mut Table, k: &str, c: Color| -> Color {
        if let Some(have) = get(p, k) { return have; }
        p.insert(k.into(), Value::String(c.css()));
        c
    };
    let sidebar_default = if dark { bg.mix(&Color::BLACK, 0.35) } else { bg.mix(&fg, 0.05) };
    put(p, "sidebar", sidebar_default);
    let raise_default = if dark { bg.mix(&fg, 0.06) } else { bg.mix(&Color::WHITE, 0.75) };
    let raise = put(p, "raise", raise_default);
    put(p, "hover", bg.mix(&fg, 0.10));
    put(p, "dim", fg.mix(&bg, 0.42));
    let on_default = accent.map_or(if dark { Color::BLACK } else { Color::WHITE }, |a| a.readable_on());
    put(p, "accent_on", on_default);
    put(p, "play_bg", fg);
    put(p, "play_fg", bg);
    put(p, "bar_bg", raise);
    put(p, "bar_fg", fg);
    // Toasts are dark in light themes (like Blur's), a lifted surface in dark ones.
    let toast = if dark { bg.mix(&fg, 0.16) } else { Color::parse("#2a2a30").unwrap() };
    put(p, "toast_bg", toast);
    put(p, "toast_fg", if dark { fg } else { Color::WHITE });
    // The full-screen lyrics page sits on a dark cover backdrop by default.
    put(p, "np_fg", Color::WHITE);
    put(p, "np_on", Color::BLACK);
}

/// The opposite-scheme palette of `colors`: background and text flip lightness (hue kept), the accent stays, the rest derives.
pub fn auto_alt(colors: &Table) -> Table {
    let mut alt = Table::new();
    let (Some(bg), Some(fg)) = (get(colors, "bg"), get(colors, "fg")) else { return alt };
    let to_light = bg.luma() < 0.18;
    let (bg_l, fg_l) = if to_light { (0.965, 0.22) } else { (0.17, 0.96) };
    alt.insert("bg".into(), Value::String(bg.with_lightness(bg_l, 0.6).css()));
    alt.insert("fg".into(), Value::String(fg.with_lightness(fg_l, 0.4).css()));
    if let Some(a) = colors.get("accent") {
        // Keep the accent's hue, but nudge its lightness until it reads on the new background.
        let new_bg = bg.with_lightness(bg_l, 0.6);
        let fixed = a.as_str().and_then(|s| Color::parse(s).ok()).map(|mut c| {
            for _ in 0..12 { if c.contrast(&new_bg) >= 3.0 { break; } c = c.with_lightness((c.oklab()[0] + if to_light { -0.04 } else { 0.04 }).clamp(0.0, 1.0), 1.0); }
            c
        });
        alt.insert("accent".into(), fixed.map_or_else(|| a.clone(), |c| Value::String(c.css())));
    }
    palette(&mut alt);
    alt
}

/// Apply `derive = true` for one theme file's table (see module docs). Always strips the `derive` key.
pub fn apply(table: &mut Table) {
    let Some(Value::Table(c)) = table.get_mut("colors") else { return };
    if !matches!(c.remove("derive"), Some(Value::Boolean(true))) { return; }
    palette(c);
    let colors = c.clone();
    let alt = match table.remove("colors_alt") {
        Some(Value::Table(mut a)) if a.contains_key("bg") && a.contains_key("fg") => { palette(&mut a); a }
        Some(Value::Table(a)) => { let mut auto = auto_alt(&colors); auto.extend(a); auto }
        _ => auto_alt(&colors),
    };
    if !alt.is_empty() { table.insert("colors_alt".into(), Value::Table(alt)); }
}

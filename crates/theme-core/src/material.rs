//! Material 3 colour scheme generator. Given a seed colour it builds tonal palettes (primary / secondary / tertiary / neutral /
//! neutral-variant / error) and the M3 colour roles for light or dark, then maps them onto harv's tokens.
//!
//! This is an *approximation* of Material Color Utilities: tones are OKLCH lightness steps (tone 40 ≈ CIE L* 40) and chroma is
//! the M3 chroma scaled into OKLCH, gamut-mapped to sRGB. Hues and contrast match the spec closely, exact hex values do not.
//! No GTK in here.

use crate::Color;
use serde::Deserialize;
use toml::{Table, Value};

/// M3 dynamic-colour flavour.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Flavor { #[default] TonalSpot, Vibrant, Expressive, Neutral, Fidelity }

/// `[material]`: generate the palette from a seed. `seed = "cover"` follows the playing artwork (regenerated on every track).
#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct MaterialCfg {
    #[serde(default = "cover")]
    pub seed: String,
    #[serde(default)]
    pub variant: Flavor,
}

fn cover() -> String { "cover".into() }

/// Seed used for the static palette when `seed = "cover"` (M3's baseline purple).
pub const FALLBACK_SEED: &str = "#6750a4";

impl MaterialCfg {
    pub fn follows_cover(&self) -> bool { self.seed.trim().eq_ignore_ascii_case("cover") }

    /// The seed colour (the fallback while following the cover).
    pub fn seed_color(&self) -> Result<Color, String> { Color::parse(if self.follows_cover() { FALLBACK_SEED } else { &self.seed }) }
}

pub struct Roles(pub Vec<(&'static str, Color)>);

impl Roles {
    pub fn get(&self, name: &str) -> Color { self.0.iter().find(|(n, _)| *n == name).map(|(_, c)| *c).unwrap_or(Color::BLACK) }
}

/// OKLab lightness of a CIE L* tone (grey axis).
fn tone_l(tone: f32) -> f32 {
    let y = if tone > 8.0 { ((tone + 16.0) / 116.0).powi(3) } else { tone / 903.3 };
    y.cbrt()
}

/// M3 chroma units (HCT) to OKLCH chroma.
const CHROMA_K: f32 = 0.0035;

#[derive(Clone, Copy)]
struct Pal { hue: f32, chroma: f32 }

impl Pal {
    fn tone(&self, t: f32) -> Color { Color::from_oklch(tone_l(t), self.chroma * CHROMA_K, self.hue) }
}

fn palettes(seed: Color, flavor: Flavor) -> [Pal; 5] {
    let (_, seed_c, h) = seed.oklch();
    let hct_c = seed_c / CHROMA_K;
    let p = |dh: f32, c: f32| Pal { hue: (h + dh).rem_euclid(360.0), chroma: c };
    match flavor {
        Flavor::TonalSpot => [p(0.0, 36.0), p(0.0, 16.0), p(60.0, 24.0), p(0.0, 6.0), p(0.0, 8.0)],
        Flavor::Vibrant => [p(0.0, 200.0), p(20.0, 24.0), p(55.0, 32.0), p(0.0, 10.0), p(0.0, 12.0)],
        Flavor::Expressive => [p(240.0, 40.0), p(15.0, 24.0), p(120.0, 32.0), p(15.0, 8.0), p(15.0, 12.0)],
        Flavor::Neutral => [p(0.0, 12.0), p(0.0, 8.0), p(60.0, 16.0), p(0.0, 2.0), p(0.0, 2.0)],
        Flavor::Fidelity => [p(0.0, hct_c.max(48.0)), p(0.0, (hct_c - 32.0).max(8.0)), p(60.0, hct_c.max(24.0)), p(0.0, hct_c / 8.0), p(0.0, hct_c / 8.0 + 4.0)],
    }
}

/// The M3 colour roles for `seed` in light or dark.
pub fn generate(seed: Color, flavor: Flavor, dark: bool) -> Roles {
    let [primary, secondary, tertiary, neutral, nv] = palettes(seed, flavor);
    let error = Pal { hue: 29.0, chroma: 84.0 };
    let t = |light: f32, dk: f32| if dark { dk } else { light };
    let mut r: Vec<(&'static str, Color)> = vec![];
    for (name, pal) in [("primary", primary), ("secondary", secondary), ("tertiary", tertiary), ("error", error)] {
        let names = match name { "primary" => ["primary", "on_primary", "primary_container", "on_primary_container"], "secondary" => ["secondary", "on_secondary", "secondary_container", "on_secondary_container"], "tertiary" => ["tertiary", "on_tertiary", "tertiary_container", "on_tertiary_container"], _ => ["error", "on_error", "error_container", "on_error_container"] };
        r.push((names[0], pal.tone(t(40.0, 80.0))));
        r.push((names[1], pal.tone(t(100.0, 20.0))));
        r.push((names[2], pal.tone(t(90.0, 30.0))));
        r.push((names[3], pal.tone(t(10.0, 90.0))));
    }
    r.extend([
        ("surface", neutral.tone(t(98.0, 6.0))),
        ("surface_dim", neutral.tone(t(87.0, 6.0))),
        ("surface_bright", neutral.tone(t(98.0, 24.0))),
        ("surface_container_lowest", neutral.tone(t(100.0, 4.0))),
        ("surface_container_low", neutral.tone(t(96.0, 10.0))),
        ("surface_container", neutral.tone(t(94.0, 12.0))),
        ("surface_container_high", neutral.tone(t(92.0, 17.0))),
        ("surface_container_highest", neutral.tone(t(90.0, 22.0))),
        ("on_surface", neutral.tone(t(10.0, 90.0))),
        ("surface_variant", nv.tone(t(90.0, 30.0))),
        ("on_surface_variant", nv.tone(t(30.0, 80.0))),
        ("outline", nv.tone(t(50.0, 60.0))),
        ("outline_variant", nv.tone(t(80.0, 30.0))),
        ("inverse_surface", neutral.tone(t(20.0, 90.0))),
        ("inverse_on_surface", neutral.tone(t(95.0, 20.0))),
        ("inverse_primary", primary.tone(t(80.0, 40.0))),
    ]);
    Roles(r)
}

/// harv's colour tokens (TOML key, colour) from the roles. `accent` is included; the caller decides whether it is fixed.
pub fn token_colors(r: &Roles) -> Vec<(&'static str, Color)> {
    vec![
        ("bg", r.get("surface")), ("sidebar", r.get("surface_container_low")), ("raise", r.get("surface_container_high")), ("hover", r.get("surface_container_highest")),
        ("fg", r.get("on_surface")), ("dim", r.get("on_surface_variant")),
        ("accent", r.get("primary")), ("accent_on", r.get("on_primary")),
        ("play_bg", r.get("primary")), ("play_fg", r.get("on_primary")),
        ("bar_bg", r.get("surface_container")), ("bar_fg", r.get("on_surface")),
        ("toast_bg", r.get("inverse_surface")), ("toast_fg", r.get("inverse_on_surface")),
        ("np_fg", r.get("on_surface")), ("np_on", r.get("surface")),
        ("line", r.get("outline_variant")), ("selected", r.get("secondary_container")), ("focus", r.get("primary")),
        ("scrim", Color::BLACK.with_alpha(0.32)), ("danger", r.get("error")),
        ("surface_1", r.get("surface_container_lowest")), ("surface_2", r.get("surface_container_low")), ("surface_3", r.get("surface_container")), ("surface_4", r.get("surface_container_high")), ("surface_5", r.get("surface_container_highest")),
    ]
}

/// The GTK `@define-color` name of a token key.
pub fn css_name(key: &str) -> &str {
    match key { "sidebar" => "bg_side", "raise" => "bg_raise", "hover" => "bg_hover", "dim" => "fg_dim", "accent" => "lib_accent", k => k }
}

fn palette_table(seed: Color, flavor: Flavor, dark: bool, follows_cover: bool) -> Table {
    let mut t = Table::new();
    for (k, c) in token_colors(&generate(seed, flavor, dark)) {
        let v = if k == "accent" && follows_cover { "cover".to_string() } else { c.css() };
        t.insert(k.into(), Value::String(v));
    }
    t
}

/// The `[colors]` / `[colors_alt]` tables for `cfg`: `native_dark` is the scheme of `[colors]`.
pub fn palettes_for(cfg: &MaterialCfg, native_dark: bool) -> Option<(Table, Table)> {
    let seed = cfg.seed_color().ok()?;
    let follow = cfg.follows_cover();
    Some((palette_table(seed, cfg.variant, native_dark, follow), palette_table(seed, cfg.variant, !native_dark, follow)))
}

/// Per theme file: with `[material]`, generate `[colors]` and `[colors_alt]`; keys the file sets itself win.
pub fn apply(table: &mut Table) {
    let Some(Value::Table(m)) = table.get("material") else { return };
    let Ok(cfg) = Value::Table(m.clone()).try_into::<MaterialCfg>() else { return };
    let native_dark = table.get("meta").and_then(|m| m.get("scheme")).and_then(|v| v.as_str()) != Some("light");
    let Some((mut colors, mut alt)) = palettes_for(&cfg, native_dark) else { return };
    if let Some(Value::Table(own)) = table.remove("colors") { colors.extend(own); }
    if let Some(Value::Table(own)) = table.remove("colors_alt") { alt.extend(own); }
    table.insert("colors".into(), Value::Table(colors));
    table.insert("colors_alt".into(), Value::Table(alt));
}

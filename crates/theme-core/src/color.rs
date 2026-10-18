//! CSS-ish colour parsing (`#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()/rgba()`, `oklch()`), emitted as GTK-safe sRGB.
//! GTK CSS can't parse `oklch()`, so themes may use it and we convert here.

use serde::{Deserialize, Deserializer};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color { pub r: f32, pub g: f32, pub b: f32, pub a: f32 }

impl Color {
    pub fn parse(s: &str) -> Result<Color, String> {
        let t = s.trim();
        if let Some(h) = t.strip_prefix('#') { return hex(h).ok_or_else(|| format!("bad hex colour `{s}`")); }
        let lower = t.to_ascii_lowercase();
        if let Some(args) = func(&lower, "oklch") { return oklch(args).ok_or_else(|| format!("bad oklch() colour `{s}`")); }
        if let Some(args) = func(&lower, "rgba").or_else(|| func(&lower, "rgb")) { return rgb(args).ok_or_else(|| format!("bad rgb() colour `{s}`")); }
        Err(format!("unrecognised colour `{s}` (use #rrggbb, rgb() or oklch())"))
    }

    /// `#rrggbb` when opaque, `rgba(r, g, b, a)` otherwise.
    pub fn css(&self) -> String {
        let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        if self.a >= 0.999 { format!("#{:02x}{:02x}{:02x}", b(self.r), b(self.g), b(self.b)) } else { format!("rgba({}, {}, {}, {})", b(self.r), b(self.g), b(self.b), (self.a * 1000.0).round() / 1000.0) }
    }

    pub fn rgb_u8(&self) -> [u8; 3] { let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8; [b(self.r), b(self.g), b(self.b)] }
    pub fn rgb_f32(&self) -> [f32; 3] { [self.r, self.g, self.b] }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Color::parse(&s).map_err(serde::de::Error::custom)
    }
}

fn func<'a>(s: &'a str, name: &str) -> Option<&'a str> { s.strip_prefix(name)?.trim_start().strip_prefix('(')?.strip_suffix(')') }

fn hex(h: &str) -> Option<Color> {
    let d = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(|v| v as f32 / 255.0);
    let n = |c: char| c.to_digit(16).map(|v| (v * 17) as f32 / 255.0);
    let cs: Vec<char> = h.chars().collect();
    match h.len() {
        3 => Some(Color { r: n(cs[0])?, g: n(cs[1])?, b: n(cs[2])?, a: 1.0 }),
        6 => Some(Color { r: d(0)?, g: d(2)?, b: d(4)?, a: 1.0 }),
        8 => Some(Color { r: d(0)?, g: d(2)?, b: d(4)?, a: d(6)? }),
        _ => None,
    }
}

/// Splits `a, b, c / alpha` or `a b c / alpha` into numbers; percentages become fractions of `pct_scale`.
fn parts(args: &str, pct_scale: [f32; 4]) -> Option<Vec<f32>> {
    let norm = args.replace(['/', ','], " ");
    let toks: Vec<&str> = norm.split_whitespace().collect();
    if toks.len() < 3 || toks.len() > 4 { return None; }
    toks.iter().enumerate().map(|(i, t)| match t.strip_suffix('%') { Some(p) => p.parse::<f32>().ok().map(|v| v / 100.0 * pct_scale[i]), None => t.parse::<f32>().ok() }).collect()
}

fn rgb(args: &str) -> Option<Color> {
    let p = parts(args, [255.0, 255.0, 255.0, 1.0])?;
    Some(Color { r: p[0] / 255.0, g: p[1] / 255.0, b: p[2] / 255.0, a: p.get(3).copied().unwrap_or(1.0) })
}

/// `oklch(L C h [/ a])`: L is 0..1 (or %), C ≈ 0..0.4, h in degrees.
fn oklch(args: &str) -> Option<Color> {
    let p = parts(args, [1.0, 0.4, 360.0, 1.0])?;
    let (l, c, h) = (p[0], p[1], p[2].to_radians());
    let (a, b) = (c * h.cos(), c * h.sin());
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let lin = [4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_93 * s3, -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_4 * s3, -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3];
    let enc = |v: f32| { let v = v.clamp(0.0, 1.0); if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 } };
    Some(Color { r: enc(lin[0]), g: enc(lin[1]), b: enc(lin[2]), a: p.get(3).copied().unwrap_or(1.0) })
}

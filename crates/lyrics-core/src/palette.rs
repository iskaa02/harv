//! Colour extraction + the pre-blurred "backdrop" bitmap used for the animated background.

use image::{imageops, imageops::FilterType, RgbaImage};

fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let d = mx - mn;
    let h = if d == 0.0 {
        0.0
    } else if mx == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    (h, if mx == 0.0 { 0.0 } else { d / mx }, mx)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let f = |n: f32| {
        let k = (n + h * 6.0).rem_euclid(6.0);
        v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    [f(5.0), f(3.0), f(1.0)]
}

/// Four vivid colours that represent the artwork (sRGB 0..1).
pub fn extract(cover: &RgbaImage) -> [[f32; 3]; 4] {
    let small = imageops::resize(cover, 48, 48, FilterType::Triangle);
    const BINS: usize = 12;
    let mut acc = [[0.0f32; 4]; BINS]; // r,g,b,weight
    let mut avg = [0.0f32; 3];
    for p in small.pixels() {
        let (r, g, b) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        avg[0] += r;
        avg[1] += g;
        avg[2] += b;
        let (h, s, v) = rgb_to_hsv(r, g, b);
        let w = (s * s * v + 0.02) * (0.3 + v);
        let bin = ((h * BINS as f32) as usize).min(BINS - 1);
        acc[bin][0] += r * w;
        acc[bin][1] += g * w;
        acc[bin][2] += b * w;
        acc[bin][3] += w;
    }
    let n = small.pixels().len() as f32;
    let avg = [avg[0] / n, avg[1] / n, avg[2] / n];
    let mut bins: Vec<_> = acc.iter().filter(|a| a[3] > 0.0).collect();
    bins.sort_by(|a, b| b[3].total_cmp(&a[3]));

    let mut out = Vec::new();
    for b in bins.iter().take(4) {
        let (h, s, v) = rgb_to_hsv(b[0] / b[3], b[1] / b[3], b[2] / b[3]);
        out.push(hsv_to_rgb(h, (s * 1.15).clamp(0.35, 1.0), (v * 1.1).clamp(0.45, 0.95)));
    }
    // Fewer than four distinct hues (monochrome art): derive variations of the average.
    let (h, s, v) = rgb_to_hsv(avg[0], avg[1], avg[2]);
    let mut k = 0.0;
    while out.len() < 4 {
        out.push(hsv_to_rgb((h + k * 0.04).rem_euclid(1.0), (s + 0.25).clamp(0.3, 1.0), (v + 0.25 + k * 0.1).clamp(0.4, 0.95)));
        k += 1.0;
    }
    [out[0], out[1], out[2], out[3]]
}

/// A tiny, saturated, heavily blurred copy of the art, smoothly upscaled so
/// that bilinear magnification on the GPU never shows diamond artefacts.
pub fn backdrop(cover: &RgbaImage) -> RgbaImage {
    let mut small = imageops::resize(cover, 48, 48, FilterType::Triangle);
    for p in small.pixels_mut() {
        let (h, s, v) = rgb_to_hsv(p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        let c = hsv_to_rgb(h, (s * 1.35).min(1.0), (v * 1.15).min(1.0));
        p[0] = (c[0] * 255.0) as u8;
        p[1] = (c[1] * 255.0) as u8;
        p[2] = (c[2] * 255.0) as u8;
    }
    let blurred = imageops::blur(&small, 3.0);
    imageops::resize(&blurred, 256, 256, FilterType::CatmullRom)
}

/// Mean perceived brightness (0..1) of the backdrop bitmap: how light a screen it will paint.
pub fn luma(img: &RgbaImage) -> f32 {
    let n = img.pixels().len().max(1) as f32;
    img.pixels().map(|p| 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32).sum::<f32>() / (255.0 * n)
}

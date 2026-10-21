//! Colour extraction + the pre-blurred "backdrop" bitmap used for the animated background.
//!
//! Colours are picked in Oklab (perceptual): weighted k-means over the artwork, near-duplicate clusters merged,
//! then scored on population × chroma with a hue-separation penalty (the idea behind Material's `Score`), and
//! finally clamped in Oklch so every colour is usable on the dark backdrop whatever the art looks like.

use image::{imageops, imageops::FilterType, RgbaImage};

fn to_lin(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn from_lin(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// sRGB (0..1) → Oklab [L, a, b].
fn to_oklab(rgb: [f32; 3]) -> [f32; 3] {
    let (r, g, b) = (to_lin(rgb[0]), to_lin(rgb[1]), to_lin(rgb[2]));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

/// Oklab → linear sRGB, not clamped (so the caller can tell whether it is in gamut).
fn oklab_to_lin(lab: [f32; 3]) -> [f32; 3] {
    let l = (lab[0] + 0.3963377774 * lab[1] + 0.2158037573 * lab[2]).powi(3);
    let m = (lab[0] - 0.1055613458 * lab[1] - 0.0638541728 * lab[2]).powi(3);
    let s = (lab[0] - 0.0894841775 * lab[1] - 1.2914855480 * lab[2]).powi(3);
    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    ]
}

/// Oklch → sRGB (0..1); chroma is reduced until the colour fits the sRGB gamut, so hue and lightness hold.
fn oklch_to_srgb(l: f32, c: f32, h: f32) -> [f32; 3] {
    let mut c = c;
    for _ in 0..24 {
        let lin = oklab_to_lin([l, c * h.cos(), c * h.sin()]);
        if lin.iter().all(|v| (-0.0005..=1.0005).contains(v)) { break; }
        c *= 0.93;
    }
    let lin = oklab_to_lin([l, c * h.cos(), c * h.sin()]);
    [from_lin(lin[0]), from_lin(lin[1]), from_lin(lin[2])]
}

fn lch(lab: [f32; 3]) -> (f32, f32, f32) {
    (lab[0], lab[1].hypot(lab[2]), lab[2].atan2(lab[1]))
}

/// Distance that weighs hue/chroma twice as much as lightness (differences in colour read stronger than in brightness).
fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dl, da, db) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    (dl * dl + 4.0 * (da * da + db * db)).sqrt()
}

struct Cluster {
    lab: [f32; 3],
    pop: f32,
}

/// Weighted k-means in Oklab. Deterministic: seeds by farthest-point, so the same cover always gives the same colours.
fn cluster(px: &[([f32; 3], f32)], k: usize) -> Vec<Cluster> {
    let total: f32 = px.iter().map(|p| p.1).sum::<f32>().max(1e-6);
    let mut seeds: Vec<[f32; 3]> = Vec::with_capacity(k);
    let mean = px.iter().fold([0.0; 3], |a, (c, w)| [a[0] + c[0] * w, a[1] + c[1] * w, a[2] + c[2] * w]);
    seeds.push([mean[0] / total, mean[1] / total, mean[2] / total]);
    while seeds.len() < k + 1 {
        let far = px.iter().filter(|p| p.1 > 0.0).max_by(|a, b| {
            let da = seeds.iter().map(|s| dist(*s, a.0)).fold(f32::MAX, f32::min);
            let db = seeds.iter().map(|s| dist(*s, b.0)).fold(f32::MAX, f32::min);
            da.total_cmp(&db)
        });
        match far { Some(p) => seeds.push(p.0), None => break }
    }
    seeds.remove(0); // the mean was only there to start the spread
    let mut out: Vec<Cluster> = seeds.iter().map(|&lab| Cluster { lab, pop: 0.0 }).collect();
    for _ in 0..8 {
        let mut acc = vec![([0.0f32; 3], 0.0f32); out.len()];
        for (c, w) in px {
            let i = (0..out.len()).min_by(|&a, &b| dist(out[a].lab, *c).total_cmp(&dist(out[b].lab, *c))).unwrap_or(0);
            let e = &mut acc[i];
            e.0 = [e.0[0] + c[0] * w, e.0[1] + c[1] * w, e.0[2] + c[2] * w];
            e.1 += w;
        }
        for (cl, (sum, w)) in out.iter_mut().zip(acc) {
            cl.pop = w / total;
            if w > 0.0 { cl.lab = [sum[0] / w, sum[1] / w, sum[2] / w]; }
        }
    }
    out.retain(|c| c.pop > 0.0);
    out
}

/// Four vivid colours that represent the artwork (sRGB 0..1), most representative first.
pub fn extract(cover: &RgbaImage) -> [[f32; 3]; 4] {
    let small = imageops::resize(cover, 48, 48, FilterType::Triangle);
    let px: Vec<([f32; 3], f32)> = small.pixels().map(|p| (to_oklab([p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]), p[3] as f32 / 255.0)).collect();
    let mut cl = cluster(&px, 8);

    // Merge near-duplicates (heaviest first, so the survivor is the dominant shade).
    cl.sort_by(|a, b| b.pop.total_cmp(&a.pop));
    let mut kept: Vec<Cluster> = Vec::new();
    for c in cl {
        match kept.iter_mut().find(|k| dist(k.lab, c.lab) < 0.1) {
            Some(k) => {
                let t = k.pop + c.pop;
                for i in 0..3 { k.lab[i] = (k.lab[i] * k.pop + c.lab[i] * c.pop) / t; }
                k.pop = t;
            }
            None => kept.push(c),
        }
    }

    // Greedy pick: population × chroma, discounted when close to a colour already chosen.
    let mut picked: Vec<[f32; 3]> = Vec::new();
    while picked.len() < 4 && !kept.is_empty() {
        let score = |c: &Cluster| {
            let sep = picked.iter().map(|p| dist(*p, c.lab)).fold(f32::MAX, f32::min);
            c.pop.sqrt() * (0.04 + lch(c.lab).1.min(0.2)) * (sep / 0.2).min(1.0)
        };
        let i = (0..kept.len()).max_by(|&a, &b| score(&kept[a]).total_cmp(&score(&kept[b]))).unwrap();
        picked.push(kept.swap_remove(i).lab);
    }
    // Fewer than four distinct colours (flat or monochrome art): shifted variants of what there is.
    let base = picked.clone();
    let mut k = 0usize;
    while picked.len() < 4 {
        let (l, c, h) = lch(base[k % base.len()]);
        let (dl, dh) = if k % 2 == 0 { (0.09, 0.14) } else { (-0.09, -0.14) };
        let (l, h) = (l + dl * (1 + k / 2) as f32, h + dh);
        picked.push([l, c * h.cos(), c * h.sin()]);
        k += 1;
    }

    let mut out = [[0.0; 3]; 4];
    for (o, lab) in out.iter_mut().zip(picked) {
        let (l, c, h) = lch(lab);
        // Greys keep their (lack of) colour; everything else is lifted into a usable chroma band.
        let c = if c < 0.025 { c } else { (c * 1.15).clamp(0.07, 0.22) };
        *o = oklch_to_srgb(l.clamp(0.55, 0.85), c, h);
    }
    out
}

/// A tiny, saturated, heavily blurred copy of the art, smoothly upscaled so
/// that bilinear magnification on the GPU never shows diamond artefacts.
pub fn backdrop(cover: &RgbaImage) -> RgbaImage {
    let mut small = imageops::resize(cover, 48, 48, FilterType::Triangle);
    for p in small.pixels_mut() {
        let (l, c, h) = lch(to_oklab([p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]));
        let c = oklch_to_srgb((l * 1.08).min(0.98), c * 1.35, h);
        p[0] = (c[0] * 255.0).round() as u8;
        p[1] = (c[1] * 255.0).round() as u8;
        p[2] = (c[2] * 255.0).round() as u8;
    }
    let blurred = imageops::blur(&small, 3.0);
    imageops::resize(&blurred, 256, 256, FilterType::CatmullRom)
}

/// Mean perceived brightness (0..1) of the backdrop bitmap: how light a screen it will paint.
pub fn luma(img: &RgbaImage) -> f32 {
    let n = img.pixels().len().max(1) as f32;
    img.pixels().map(|p| 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32).sum::<f32>() / (255.0 * n)
}

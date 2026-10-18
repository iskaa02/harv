//! Decorative waveform for tracks that have no real peak data.
//!
//! This is NOT audio analysis: it is a deterministic fake that looks like music (a few seeded sines for body,
//! jitter for grain, a section envelope, a short ramp-in and a fade over the last 4 %). The same seed always gives
//! the same shape, so a track keeps its waveform between launches.

use std::f64::consts::TAU;

/// `n` bar heights in `0.12..=1.0`, deterministic for `seed` (use the track id).
pub fn pseudo_peaks(seed: &str, n: usize) -> Vec<f32> {
    if n == 0 {
        return Vec::new();
    }
    // FNV-1a over the seed gives the stream's starting state.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in seed.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let mut rng = Rng(h);

    // Two or three sines: a slow body, a mid wiggle and a fast grain.
    let nsin = 2 + (rng.next_f64() * 2.0) as usize;
    let mut waves = Vec::with_capacity(nsin);
    for i in 0..nsin {
        let cycles = [4.0, 11.0, 27.0][i] * (0.7 + 0.6 * rng.next_f64());
        let amp = [1.0, 0.55, 0.3][i] * (0.8 + 0.4 * rng.next_f64());
        let phase = TAU * rng.next_f64();
        waves.push((cycles, amp, phase));
    }
    let amp_sum: f64 = waves.iter().map(|w| w.1).sum();

    // Three to five sections, each with its own loudness.
    let nsec = 3 + (rng.next_f64() * 3.0) as usize;
    let gains: Vec<f64> = (0..nsec).map(|_| 0.55 + 0.45 * rng.next_f64()).collect();

    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let x = if n == 1 { 0.5 } else { i as f64 / (n - 1) as f64 };
        let s = waves.iter().map(|&(c, a, p)| a * (TAU * c * x + p).sin()).sum::<f64>() / amp_sum;
        let body = 0.25 + 0.75 * (0.5 + 0.5 * s);
        let grain = 1.0 + 0.12 * (rng.next_f64() - 0.5);
        let ramp = smooth((x / 0.03).min(1.0));
        let fade = 1.0 - 0.9 * smooth(((x - 0.96) / 0.04).clamp(0.0, 1.0));
        out.push(body * grain * section_env(&gains, x) * ramp * fade);
    }

    let lo = out.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = out.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    out.iter()
        .map(|&v| {
            if hi - lo < 1e-9 {
                0.5
            } else {
                (0.12 + 0.88 * (v - lo) / (hi - lo)) as f32
            }
        })
        .collect()
}

/// Section gains, cosine-smoothed between section centres.
fn section_env(gains: &[f64], x: f64) -> f64 {
    let k = gains.len();
    if k == 1 {
        return gains[0];
    }
    let p = (x * k as f64 - 0.5).clamp(0.0, (k - 1) as f64);
    let i = (p.floor() as usize).min(k - 2);
    let t = smooth(p - i as f64);
    gains[i] + (gains[i + 1] - gains[i]) * t
}

fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// splitmix64.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in `[0, 1)`.
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

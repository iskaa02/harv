//! Audio effect settings and the mpv `af` chain they compile to (shared by every deck).

use serde::{Deserialize, Serialize};

pub const EQ_BANDS_HZ: [u32; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Reverb { #[default] Off, Room, Hall, Cathedral }

/// One AutoEq / parametric filter: peaking (`Pk`), low shelf (`Lsc`) or high shelf (`Hsc`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parametric { pub kind: ParaKind, pub fc: f32, pub gain_db: f32, pub q: f32 }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParaKind { Pk, Lsc, Hsc }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioFx {
    pub eq_on: bool,
    pub preamp_db: f32,
    /// Gain in dB for each of [`EQ_BANDS_HZ`].
    pub eq_gains: [f32; 10],
    /// AutoEq headphone correction (applied after the graphic EQ); empty = none.
    pub parametric: Vec<Parametric>,
    /// Display name of the AutoEq profile in `parametric` (for the UI); `None` = no profile.
    pub autoeq_name: Option<String>,
    /// Preamp in dB applied in front of `parametric` (AutoEq headroom); 0 = none.
    pub autoeq_preamp_db: f32,
    /// Playback speed, pitch preserved (1.0 = normal).
    pub speed: f64,
    /// Independent pitch shift in semitones (0 = none).
    pub pitch_semitones: f32,
    pub reverb: Reverb,
    /// Echo delay in milliseconds; 0 = off.
    pub delay_ms: u32,
}

impl Default for AudioFx {
    fn default() -> Self { Self { eq_on: false, preamp_db: 0.0, eq_gains: [0.0; 10], parametric: vec![], autoeq_name: None, autoeq_preamp_db: 0.0, speed: 1.0, pitch_semitones: 0.0, reverb: Reverb::Off, delay_ms: 0 } }
}

impl AudioFx {
    /// Value for mpv's `af` property ("" = no filters).
    pub fn chain(&self) -> String {
        let mut parts: Vec<String> = vec![];
        let mut lavfi: Vec<String> = vec![];
        if self.eq_on {
            if self.preamp_db.abs() > 0.05 { lavfi.push(format!("volume={:.1}dB", self.preamp_db)); }
            for (f, g) in EQ_BANDS_HZ.iter().zip(self.eq_gains) {
                if g.abs() > 0.05 { lavfi.push(format!("equalizer=f={f}:t=q:w=1.41:g={g:.1}")); }
            }
        }
        if !self.parametric.is_empty() && self.autoeq_preamp_db.abs() > 0.05 { lavfi.push(format!("volume={:.1}dB", self.autoeq_preamp_db)); }
        for p in &self.parametric {
            let (name, extra) = match p.kind { ParaKind::Pk => ("equalizer", ""), ParaKind::Lsc => ("lowshelf", ""), ParaKind::Hsc => ("highshelf", "") };
            lavfi.push(format!("{name}=f={:.0}:t=q:w={:.2}:g={:.1}{extra}", p.fc, p.q.max(0.1), p.gain_db));
        }
        if !lavfi.is_empty() { parts.push(format!("lavfi=[{}]", lavfi.join(","))); }
        let mut echo: Vec<String> = vec![];
        match self.reverb {
            Reverb::Off => {}
            // Multi-tap echoes approximate a reverb tail (no impulse-response convolution).
            Reverb::Room => echo.push("aecho=0.85:0.55:25|43|61:0.35|0.25|0.18".into()),
            Reverb::Hall => echo.push("aecho=0.8:0.6:60|97|141|187:0.4|0.32|0.24|0.18".into()),
            Reverb::Cathedral => echo.push("aecho=0.75:0.65:110|173|251|331|443:0.45|0.38|0.3|0.24|0.18".into()),
        }
        if self.delay_ms > 0 { echo.push(format!("aecho=0.8:0.85:{}:0.45", self.delay_ms.min(2000))); }
        if !echo.is_empty() { parts.push(format!("lavfi=[{}]", echo.join(","))); }
        if self.pitch_semitones.abs() > 0.01 { parts.push(format!("rubberband=pitch-scale={:.5}", 2f32.powf(self.pitch_semitones / 12.0))); }
        parts.join(",")
    }

    pub fn is_neutral(&self) -> bool { self.chain().is_empty() && (self.speed - 1.0).abs() < 1e-3 }
}

/// Built-in graphic-EQ presets: (name, preamp dB, gains for [`EQ_BANDS_HZ`]).
pub const EQ_PRESETS: [(&str, f32, [f32; 10]); 8] = [
    ("Flat", 0.0, [0.0; 10]),
    ("Bass boost", -4.0, [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Treble boost", -4.0, [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 4.0, 5.0, 6.0]),
    ("Vocal", -2.0, [-2.0, -2.0, -1.0, 1.0, 3.0, 4.0, 3.0, 1.0, 0.0, -1.0]),
    ("Rock", -3.0, [5.0, 4.0, 2.0, -1.0, -2.0, -1.0, 2.0, 4.0, 5.0, 5.0]),
    ("Pop", -2.0, [-1.0, 1.0, 3.0, 4.0, 3.0, 0.0, -1.0, -1.0, 1.0, 2.0]),
    ("Jazz", -2.0, [3.0, 2.0, 1.0, 2.0, -2.0, -2.0, 0.0, 1.0, 2.0, 3.0]),
    ("Electronic", -3.0, [5.0, 4.0, 1.0, 0.0, -2.0, 2.0, 1.0, 1.0, 4.0, 5.0]),
];

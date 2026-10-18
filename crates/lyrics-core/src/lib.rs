//! Renderer-agnostic pieces of the lyrics screen: data, timing, animation, colour, layout.

pub mod anim;
pub mod layout;
pub mod lrc;
pub mod palette;
pub mod style;
pub mod ttml;

use anim::{Scroller, Spring};
pub use style::{Align, BackdropKind, LayoutKind, LyricsStyle, StyleKind, TextColors};

/// Simulated playback + all animation state. Front-ends call [`Scene::step`] once per frame.
pub struct Scene {
    pub pos: f32,
    pub playing: bool,
    pub duration: f32,
    pub scroller: Scroller,
    pub art_scale: Spring,
    /// Advances only while playing, so a paused app can stop rendering entirely.
    pub bg_time: f32,
    pub active: Option<usize>,
    /// True when `pos` is driven by the audio clock (set it before each [`Scene::step`]); false = simulated clock (demo).
    pub external_clock: bool,
    /// Backdrop motion strength: 0 = frozen, ~0.6 = calm, 1 = lively (set by the front-end each frame).
    pub motion: f32,
    /// Backdrop flow clock: like `bg_time` but sped up by `energy` and scaled by `motion`.
    pub flow: f32,
    /// 0..1 "how busy is this passage" (lyric density), smoothed. Drives flow speed, brightness and warp.
    pub energy: f32,
    /// Soft 0..1 swell after every new lyric line: rises over ~0.15 s and eases out over ~1.5 s (never snaps).
    pub pulse: f32,
    kick: f32,
    prev_active: Option<usize>,
    lines: Vec<f32>,
}

impl Scene {
    /// A scene with no lyrics yet (real playback; fill in with [`Scene::set_lines`]).
    pub fn empty() -> Self {
        let mut art_scale = Spring::new(0.9);
        art_scale.target = 0.9;
        Self { pos: 0.0, playing: false, duration: 1.0, scroller: Scroller::new(0), art_scale, bg_time: 0.0, active: None, external_clock: true, motion: 1.0, flow: 0.0, energy: 0.3, pulse: 0.0, kick: 0.0, prev_active: None, lines: Vec::new() }
    }

    /// Replace the lyric timeline. Plain (untimed) lyrics pass `f32::MAX` stamps so no line ever becomes "active".
    pub fn set_lines(&mut self, times: Vec<f32>) {
        self.scroller = Scroller::new(times.len());
        self.lines = times;
        self.active = None;
        self.prev_active = None;
    }

    pub fn toggle(&mut self) {
        self.playing = !self.playing;
    }

    pub fn seek_by(&mut self, d: f32) {
        self.pos = (self.pos + d).clamp(0.0, self.duration - 0.1);
    }

    pub fn seek_to_line(&mut self, i: usize) {
        self.pos = self.lines[i] + 0.01;
    }

    pub fn step(&mut self, dt: f32, tops: &[f32], anchor_y: f32) {
        let dt = dt.clamp(0.0, 1.0 / 20.0);
        if self.playing {
            if !self.external_clock {
                self.pos += dt;
                if self.pos >= self.duration {
                    self.pos = 0.0;
                }
            }
            self.bg_time += dt;
        }
        let t = self.pos;
        self.active = self.lines.partition_point(|&x| x <= t).checked_sub(1);
        self.step_fx(dt);
        self.scroller.update(dt, self.active, tops, anchor_y);
        self.art_scale.target = if self.playing { 1.0 } else { 0.9 };
        self.art_scale.step(dt, 180.0, 18.0);
    }

    /// Backdrop energy and line-change swell. Dense, fast passages run hotter; every new line gives a soft pulse.
    fn step_fx(&mut self, dt: f32) {
        let calm = if self.playing { 0.0 } else { 1.0 };
        let target = match self.active {
            Some(i) if self.playing => {
                // Seconds this line lasts: short lines = busy passage, long ones / gaps = calm.
                let span = self.lines.get(i + 1).map_or(6.0, |n| n - self.lines[i]).clamp(0.5, 12.0);
                0.1 + 0.9 * (1.0 - ((span - 1.5) / 6.0).clamp(0.0, 1.0))
            }
            _ => 0.2,
        };
        self.energy += (target - self.energy) * (1.0 - (-dt * (1.1 + calm)).exp());
        // Each new line also makes the backdrop surge forward for a moment (speed, not a position jump).
        let rate = self.motion * (0.3 + 1.5 * self.energy + 2.2 * self.pulse);
        if self.playing {
            self.flow += dt * rate;
        }
        if self.active != self.prev_active {
            if self.active.is_some() && self.playing && self.motion > 0.0 {
                self.kick = 1.0;
            }
            self.prev_active = self.active;
        }
        self.pulse += (self.kick - self.pulse) * (1.0 - (-dt * 9.0).exp());
        self.kick *= (-dt * 2.0).exp();
        if self.kick < 0.003 { self.kick = 0.0; }
        if self.kick == 0.0 && self.pulse < 0.003 { self.pulse = 0.0; }
    }

    /// Does the front-end need another frame?
    pub fn animating(&self) -> bool {
        self.playing || !self.scroller.settled() || !self.art_scale.settled(0.0005) || self.pulse > 0.0 || self.kick > 0.0
            || (self.energy - 0.2).abs() > 0.004
    }

    pub fn progress(&self) -> f32 {
        (self.pos / self.duration).clamp(0.0, 1.0)
    }

    /// 0..1 pulse for the interlude dots.
    pub fn breath(&self) -> f32 {
        0.5 + 0.5 * (self.pos * 2.4).sin()
    }
}

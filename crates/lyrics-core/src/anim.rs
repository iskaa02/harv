//! Spring animation + the staggered lyric scroll choreography.

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub x: f32,
    pub v: f32,
    pub target: f32,
}

impl Spring {
    pub fn new(x: f32) -> Self {
        Self { x, v: 0.0, target: x }
    }
    pub fn step(&mut self, dt: f32, k: f32, c: f32) {
        // Fixed sub-steps keep semi-implicit Euler stable at any frame rate.
        let n = (dt / (1.0 / 240.0)).ceil().max(1.0) as u32;
        let h = dt / n as f32;
        for _ in 0..n {
            let a = k * (self.target - self.x) - c * self.v;
            self.v += a * h;
            self.x += self.v * h;
        }
    }
    pub fn settled(&self, eps: f32) -> bool {
        (self.x - self.target).abs() < eps && self.v.abs() < eps * 4.0
    }
    pub fn snap(&mut self) {
        self.x = self.target;
        self.v = 0.0;
    }
}

#[derive(Clone, Copy)]
struct LineAnim {
    y: Spring,
    focus: Spring,
    dist: Spring,
    wait: f32,
}

/// What a renderer needs to draw one lyric line this frame.
#[derive(Clone, Copy, Debug)]
pub struct LineVis {
    /// Top of the line box, logical px.
    pub y: f32,
    pub scale: f32,
    pub alpha: f32,
    /// Blur radius, logical px.
    pub blur: f32,
    /// 0..1 glow amount (active line).
    pub glow: f32,
    pub focus: f32,
    /// Smoothed distance to the active line, in lines.
    pub dist: f32,
}

pub struct Scroller {
    lines: Vec<LineAnim>,
    active: Option<usize>,
    snap_next: bool,
    first: bool,
    /// Seconds of cascade delay per line below the active one.
    delay: f32,
    /// Spring stiffness multiplier (1.0 = the default feel).
    stiff: f32,
}

const DELAY_PER_LINE: f32 = 0.05;

impl Scroller {
    pub fn new(n: usize) -> Self {
        let l = LineAnim { y: Spring::new(0.0), focus: Spring::new(0.0), dist: Spring::new(0.0), wait: 0.0 };
        Self { lines: vec![l; n], active: None, snap_next: true, first: true, delay: DELAY_PER_LINE, stiff: 1.0 }
    }

    /// Cascade delay per line (seconds) and spring stiffness multiplier. Defaults: 0.05 and 1.0.
    pub fn set_tuning(&mut self, cascade_delay: f32, stiffness: f32) {
        self.delay = cascade_delay.max(0.0);
        self.stiff = stiffness.max(0.01);
    }

    /// Layout changed (resize, font change): jump instead of animating.
    pub fn snap(&mut self) {
        self.snap_next = true;
    }

    pub fn update(&mut self, dt: f32, active: Option<usize>, tops: &[f32], anchor_y: f32) {
        let base = active.unwrap_or(0);
        let base_top = tops.get(base).copied().unwrap_or(0.0);
        let changed = active != self.active;
        self.active = active;
        let (delay, k, c) = (self.delay, self.stiff, self.stiff.sqrt());

        for (i, l) in self.lines.iter_mut().enumerate() {
            let target_y = tops[i] - base_top + anchor_y;
            if changed && !self.first {
                // Lines at/below the new active line lag behind it: the cascade.
                let behind = (i as i32 - base as i32).max(0) as f32;
                l.wait = (behind * delay).min(0.6);
            }
            if l.wait > 0.0 {
                l.wait = (l.wait - dt).max(0.0);
            }
            if l.wait == 0.0 {
                l.y.target = target_y;
            }
            l.focus.target = if active == Some(i) { 1.0 } else { 0.0 };
            l.dist.target = (i as f32 - base as f32).abs().min(8.0);

            if self.snap_next || self.first {
                l.y.target = target_y;
                l.y.snap();
                l.focus.snap();
                l.dist.snap();
                l.wait = 0.0;
            } else {
                l.y.step(dt, 170.0 * k, 24.0 * c);
                l.focus.step(dt, 110.0 * k, 21.0 * c);
                l.dist.step(dt, 90.0 * k, 19.0 * c);
            }
        }
        self.snap_next = false;
        self.first = false;
    }

    pub fn settled(&self) -> bool {
        self.lines.iter().all(|l| {
            l.wait == 0.0 && l.y.settled(0.05) && l.focus.settled(0.002) && l.dist.settled(0.01)
        })
    }

    pub fn vis(&self, i: usize) -> LineVis {
        let l = &self.lines[i];
        let f = l.focus.x.clamp(0.0, 1.0);
        let d = l.dist.x.max(0.0);
        LineVis {
            y: l.y.x,
            scale: 0.9 + 0.1 * f,
            alpha: (0.30 + 0.70 * f) * (1.0 - ((d - 4.0) / 6.0).clamp(0.0, 0.5)),
            blur: (d * 1.0).min(6.0) * (1.0 - f),
            glow: f,
            focus: f,
            dist: d,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tops(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32 * 100.0).collect()
    }

    /// The signature effect: after the active line changes, lines further below start
    /// moving later than the ones nearest the active line.
    #[test]
    fn lines_below_the_active_one_follow_in_sequence() {
        let t = tops(10);
        let mut s = Scroller::new(10);
        s.update(0.0, Some(0), &t, 200.0); // first frame snaps
        let start: Vec<f32> = (0..10).map(|i| s.vis(i).y).collect();
        s.update(1.0 / 60.0, Some(1), &t, 200.0); // active line changes 0 -> 1
        for _ in 0..6 {
            s.update(1.0 / 60.0, Some(1), &t, 200.0); // ~117 ms in
        }
        let moved: Vec<f32> = (0..10).map(|i| start[i] - s.vis(i).y).collect();
        // Every line travels the same 100px in total, but nearer lines are further along...
        assert!(moved[1] > moved[3], "{moved:?}");
        assert!(moved[3] > moved[6], "{moved:?}");
        // ...and a line 8 rows down hasn't started yet.
        assert!(moved[9] < 1.0, "{moved:?}");
        // Everything lands on the new target and the scroller reports settled.
        for _ in 0..240 {
            s.update(1.0 / 60.0, Some(1), &t, 200.0);
        }
        assert!(s.settled());
        for i in 0..10 {
            assert!((s.vis(i).y - (t[i] - t[1] + 200.0)).abs() < 0.1);
        }
    }

    #[test]
    fn springs_are_framerate_independent() {
        let run = |fps: f32| {
            let mut sp = Spring::new(0.0);
            sp.target = 100.0;
            for _ in 0..(fps * 0.3) as u32 {
                sp.step(1.0 / fps, 170.0, 24.0);
            }
            sp.x
        };
        assert!((run(30.0) - run(144.0)).abs() < 1.5);
    }
}

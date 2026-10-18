//! Seek bar drawn as rounded vertical bars: played part in the accent colour, rest dimmed.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::{gdk, glib, gsk, prelude::*, subclass::prelude::*};

use crate::util::{palette, rect, rrect, with_alpha};

const MIN_BAR: f32 = 8.0;

mod imp {
    use super::*;

    pub struct Waveform {
        pub peaks: RefCell<Vec<f32>>,
        /// Resampled bar heights, 0..1, one per bar that fits the current width.
        pub bars: RefCell<Vec<f32>>,
        pub nbars: Cell<usize>,
        pub progress: Cell<f64>,
        /// Bars left of the playhead (drawn in the accent colour).
        pub played: Cell<usize>,
        /// Hover position as a fraction (0..1); resolved to a bar when drawing, so it can be set before layout.
        pub hover: Cell<Option<f64>>,
        pub bar_w: Cell<f32>,
        pub gap: Cell<f32>,
        pub height: Cell<i32>,
        pub accent: RefCell<Option<gdk::RGBA>>,
        pub dim: RefCell<Option<gdk::RGBA>>,
        pub key_step: Cell<f64>,
        pub drag_start: Cell<f64>,
        pub seek: RefCell<Option<Rc<dyn Fn(f64)>>>,
    }

    impl Default for Waveform {
        fn default() -> Self {
            Self {
                peaks: Default::default(),
                bars: Default::default(),
                nbars: Cell::new(0),
                progress: Cell::new(0.0),
                played: Cell::new(0),
                hover: Cell::new(None),
                bar_w: Cell::new(4.0),
                gap: Cell::new(3.0),
                height: Cell::new(48),
                accent: Default::default(),
                dim: Default::default(),
                key_step: Cell::new(0.05),
                drag_start: Cell::new(0.0),
                seek: Default::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Waveform {
        const NAME: &'static str = "SeekWaveform";
        type Type = super::Waveform;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("seek-waveform");
        }
    }

    impl ObjectImpl for Waveform {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_hexpand(true);
            obj.set_focusable(true);

            let drag = gtk::GestureDrag::new();
            let weak = obj.downgrade();
            drag.connect_drag_begin(move |_, x, _| {
                if let Some(o) = weak.upgrade() {
                    o.imp().drag_start.set(x);
                    o.seek_to_x(x);
                }
            });
            let weak = obj.downgrade();
            drag.connect_drag_update(move |_, dx, _| {
                if let Some(o) = weak.upgrade() {
                    o.seek_to_x(o.imp().drag_start.get() + dx);
                }
            });
            obj.add_controller(drag);

            let motion = gtk::EventControllerMotion::new();
            let weak = obj.downgrade();
            motion.connect_motion(move |_, x, _| {
                if let Some(o) = weak.upgrade() {
                    let n = o.imp().nbars.get();
                    let f = o.bar_at_x(x).map(|i| (i as f64 + 0.5) / n as f64);
                    o.set_hover(f);
                }
            });
            let weak = obj.downgrade();
            motion.connect_leave(move |_| {
                if let Some(o) = weak.upgrade() {
                    o.set_hover(None);
                }
            });
            obj.add_controller(motion);

            let keys = gtk::EventControllerKey::new();
            let weak = obj.downgrade();
            keys.connect_key_pressed(move |_, key, _, _| {
                let Some(o) = weak.upgrade() else { return glib::Propagation::Proceed };
                let step = o.imp().key_step.get();
                let p = o.imp().progress.get();
                match key {
                    gdk::Key::Left => o.seek_fraction((p - step).clamp(0.0, 1.0)),
                    gdk::Key::Right => o.seek_fraction((p + step).clamp(0.0, 1.0)),
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            });
            obj.add_controller(keys);
        }
    }

    impl WidgetImpl for Waveform {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => (40, 240, -1, -1),
                _ => (self.height.get(), self.height.get(), -1, -1),
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let n = self.obj().bar_count(width as f32);
            if n != self.nbars.get() {
                self.obj().resample(n);
            }
        }

        fn snapshot(&self, snap: &gtk::Snapshot) {
            self.obj().draw(snap);
        }
    }
}

glib::wrapper! {
    /// Seek bar of vertical rounded bars. See the crate docs for the colour and CSS conventions.
    pub struct Waveform(ObjectSubclass<imp::Waveform>) @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Waveform {
    fn default() -> Self {
        Self::new()
    }
}

impl Waveform {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Peaks in `0..1` (any length; they are resampled to the bars that fit the width).
    pub fn set_peaks(&self, peaks: &[f32]) {
        *self.imp().peaks.borrow_mut() = peaks.iter().map(|p| p.clamp(0.0, 1.0)).collect();
        self.resample(self.bar_count(self.width() as f32));
        self.queue_draw();
    }

    /// Playhead position, `0..1`. Redraws only when the played bar count changes.
    pub fn set_progress(&self, p: f64) {
        let p = if p.is_finite() { p.clamp(0.0, 1.0) } else { 0.0 };
        self.imp().progress.set(p);
        let played = ((p * self.imp().nbars.get() as f64).floor() as usize).min(self.imp().nbars.get());
        if played != self.imp().played.get() {
            self.imp().played.set(played);
            self.queue_draw();
        }
    }

    /// Called with a fraction `0..1` on click, drag, and Left/Right keys.
    pub fn connect_seek(&self, f: impl Fn(f64) + 'static) {
        *self.imp().seek.borrow_mut() = Some(Rc::new(f));
    }

    /// Width of one bar in px (default 4).
    pub fn set_bar_width(&self, px: f32) {
        self.imp().bar_w.set(px.max(1.0));
        self.imp().nbars.set(0);
        self.queue_resize();
    }

    /// Space between bars in px (default 3).
    pub fn set_bar_gap(&self, px: f32) {
        self.imp().gap.set(px.max(0.0));
        self.imp().nbars.set(0);
        self.queue_resize();
    }

    /// Height of the tallest bar in px (default 48). Shortest bars are 8 px.
    pub fn set_height(&self, px: i32) {
        self.imp().height.set(px.max(1));
        self.queue_resize();
    }

    /// Fraction moved by a Left/Right key press (default 0.05).
    pub fn set_key_step(&self, step: f64) {
        self.imp().key_step.set(step.clamp(0.001, 1.0));
    }

    /// Override the accent (played) colour. `None` returns to the CSS `color`.
    pub fn set_accent_color(&self, c: Option<gdk::RGBA>) {
        *self.imp().accent.borrow_mut() = c;
        self.queue_draw();
    }

    /// Override the dim (rest) colour. `None` returns to the CSS `color` at 25 % alpha.
    pub fn set_dim_color(&self, c: Option<gdk::RGBA>) {
        *self.imp().dim.borrow_mut() = c;
        self.queue_draw();
    }

    /// Show a hover cursor on the bar under `fraction` (0..1), or hide it. Pointer motion calls this itself; it is
    /// public so screenshots can show the hover state. Redraws only when the hovered bar changes.
    pub fn set_hover(&self, fraction: Option<f64>) {
        let fraction = fraction.filter(|f| f.is_finite()).map(|f| f.clamp(0.0, 1.0));
        let old = self.hover_bar(self.imp().hover.get());
        self.imp().hover.set(fraction);
        if self.hover_bar(fraction) != old {
            self.queue_draw();
        }
    }

    /// Bar index under a fraction, for the current bar count.
    fn hover_bar(&self, fraction: Option<f64>) -> Option<usize> {
        let n = self.imp().nbars.get();
        fraction.filter(|_| n > 0).map(|f| ((f * n as f64) as usize).min(n - 1))
    }

    /// Bars that fit in `width` px.
    fn bar_count(&self, width: f32) -> usize {
        let (bw, gap) = (self.imp().bar_w.get(), self.imp().gap.get());
        if width <= 0.0 {
            return 0;
        }
        (((width + gap) / (bw + gap)).floor() as usize).max(1)
    }

    fn resample(&self, n: usize) {
        let peaks = self.imp().peaks.borrow();
        let bars: Vec<f32> = (0..n)
            .map(|i| {
                if peaks.is_empty() {
                    return 0.5;
                }
                // Average the peaks that fall into this bar; with fewer peaks than bars, take the nearest one.
                let start = i * peaks.len() / n;
                let end = (((i + 1) * peaks.len()) / n).max(start + 1).min(peaks.len());
                peaks[start..end].iter().sum::<f32>() / (end - start) as f32
            })
            .collect();
        drop(peaks);
        *self.imp().bars.borrow_mut() = bars;
        self.imp().nbars.set(n);
        let played = ((self.imp().progress.get() * n as f64).floor() as usize).min(n);
        self.imp().played.set(played);
        self.queue_draw();
    }

    /// Geometry of the bar block: `(x of first bar, pitch, total width)`.
    fn layout(&self) -> (f32, f32, f32) {
        let (bw, gap) = (self.imp().bar_w.get(), self.imp().gap.get());
        let n = self.imp().nbars.get() as f32;
        let total = if n > 0.0 { n * bw + (n - 1.0) * gap } else { 0.0 };
        let w = self.width() as f32;
        // Whole-pixel origin keeps the bar edges crisp.
        (((w - total) / 2.0).floor(), bw + gap, total)
    }

    fn fraction_at_x(&self, x: f64) -> f64 {
        let (x0, _, total) = self.layout();
        if total <= 0.0 {
            return 0.0;
        }
        ((x as f32 - x0) / total).clamp(0.0, 1.0) as f64
    }

    fn bar_at_x(&self, x: f64) -> Option<usize> {
        let n = self.imp().nbars.get();
        if n == 0 {
            return None;
        }
        let (x0, pitch, _) = self.layout();
        let i = ((x as f32 - x0) / pitch).floor().max(0.0) as usize;
        Some(i.min(n - 1))
    }

    fn seek_to_x(&self, x: f64) {
        self.seek_fraction(self.fraction_at_x(x));
    }

    /// Move the local playhead and tell the host (the host seeks and calls `set_progress` again).
    fn seek_fraction(&self, f: f64) {
        let changed = (f - self.imp().progress.get()).abs() > 1e-9;
        self.set_progress(f);
        if changed {
            let cb = self.imp().seek.borrow().clone();
            if let Some(cb) = cb {
                cb(f);
            }
        }
    }

    fn draw(&self, snap: &gtk::Snapshot) {
        let h = self.height() as f32;
        let (accent, dim) = palette(self, self.imp().accent.borrow().clone(), self.imp().dim.borrow().clone());
        let bars = self.imp().bars.borrow();
        let (x0, pitch, _) = self.layout();
        let bw = self.imp().bar_w.get();
        let max_h = self.imp().height.get() as f32;
        let min_h = MIN_BAR.min(max_h);

        if !bars.is_empty() {
            let played = self.imp().played.get();
            let (pb, pr) = (gsk::PathBuilder::new(), gsk::PathBuilder::new());
            for (i, v) in bars.iter().enumerate() {
                let bh = min_h + (max_h - min_h) * v;
                let x = x0 + i as f32 * pitch;
                let r = rrect(x, (h - bh) / 2.0, bw, bh, (bw / 2.0).min(bh / 2.0));
                if i < played { pb.add_rounded_rect(&r) } else { pr.add_rounded_rect(&r) }
            }
            snap.append_fill(&pr.to_path(), gsk::FillRule::Winding, &dim);
            snap.append_fill(&pb.to_path(), gsk::FillRule::Winding, &accent);

            if let Some(i) = self.hover_bar(self.imp().hover.get()) {
                let xc = x0 + i as f32 * pitch + bw / 2.0;
                snap.append_color(&with_alpha(&accent, 0.6), &rect(xc - 0.75, 0.0, 1.5, h));
            }
        }
    }
}

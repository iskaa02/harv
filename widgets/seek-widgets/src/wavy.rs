//! Material 3 Expressive wavy seek bar: the played part is a travelling sine wave in the accent colour, the rest a straight dimmed
//! line, with a vertical handle at the playhead. The wave only moves while `set_playing(true)`: the frame callback is added then and
//! removed on pause, so a paused player costs nothing.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::{gdk, glib, gsk, prelude::*, subclass::prelude::*};

use crate::util::{palette, rrect};

mod imp {
    use super::*;

    pub struct Wavy {
        pub progress: Cell<f64>,
        pub phase: Cell<f32>,
        pub amp: Cell<f32>,
        pub wavelength: Cell<f32>,
        pub stroke: Cell<f32>,
        pub height: Cell<i32>,
        pub playing: Cell<bool>,
        pub tick: RefCell<Option<gtk::TickCallbackId>>,
        pub last: Cell<i64>,
        pub accent: RefCell<Option<gdk::RGBA>>,
        pub dim: RefCell<Option<gdk::RGBA>>,
        pub seek: RefCell<Option<Rc<dyn Fn(f64)>>>,
    }

    impl Default for Wavy {
        fn default() -> Self {
            Self { progress: Cell::new(0.0), phase: Cell::new(0.0), amp: Cell::new(3.0), wavelength: Cell::new(30.0), stroke: Cell::new(4.0), height: Cell::new(28), playing: Cell::new(false), tick: Default::default(), last: Cell::new(0), accent: Default::default(), dim: Default::default(), seek: Default::default() }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Wavy {
        const NAME: &'static str = "SeekWavy";
        type Type = super::Wavy;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) { klass.set_css_name("seek-wavy"); }
    }

    impl ObjectImpl for Wavy {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_hexpand(true);
            obj.set_focusable(true);
            let drag = gtk::GestureDrag::new();
            let start = Rc::new(Cell::new(0.0f64));
            let (weak, s) = (obj.downgrade(), start.clone());
            drag.connect_drag_begin(move |_, x, _| { if let Some(o) = weak.upgrade() { s.set(x); o.seek_to_x(x); } });
            let (weak, s) = (obj.downgrade(), start);
            drag.connect_drag_update(move |_, dx, _| { if let Some(o) = weak.upgrade() { o.seek_to_x(s.get() + dx); } });
            obj.add_controller(drag);
            let keys = gtk::EventControllerKey::new();
            let weak = obj.downgrade();
            keys.connect_key_pressed(move |_, key, _, _| {
                let Some(o) = weak.upgrade() else { return glib::Propagation::Proceed };
                let p = o.imp().progress.get();
                match key {
                    gdk::Key::Left => o.seek_fraction((p - 0.05).clamp(0.0, 1.0)),
                    gdk::Key::Right => o.seek_fraction((p + 0.05).clamp(0.0, 1.0)),
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            });
            obj.add_controller(keys);
        }

        fn dispose(&self) { if let Some(id) = self.tick.take() { id.remove(); } }
    }

    impl WidgetImpl for Wavy {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => (60, 240, -1, -1),
                _ => (self.height.get(), self.height.get(), -1, -1),
            }
        }

        fn snapshot(&self, snap: &gtk::Snapshot) { self.obj().draw(snap); }
    }
}

glib::wrapper! {
    /// Wavy seek bar. Accent = CSS `color`, rest = that colour at 25 % alpha (see the crate docs). CSS node: `seek-wavy`.
    pub struct Wavy(ObjectSubclass<imp::Wavy>) @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Wavy { fn default() -> Self { Self::new() } }

impl Wavy {
    pub fn new() -> Self { glib::Object::new() }

    /// Playhead, `0..1`. Redraws only when the playhead moves by at least a pixel.
    pub fn set_progress(&self, p: f64) {
        let p = if p.is_finite() { p.clamp(0.0, 1.0) } else { 0.0 };
        let old = self.imp().progress.replace(p);
        if ((p - old) * self.width() as f64).abs() >= 1.0 && !self.imp().playing.get() { self.queue_draw(); }
    }

    pub fn connect_seek(&self, f: impl Fn(f64) + 'static) { *self.imp().seek.borrow_mut() = Some(Rc::new(f)); }

    /// Wave height (peak to peak is twice this), wavelength and line width in px; widget height in px.
    pub fn set_shape(&self, amplitude: f32, wavelength: f32, stroke: f32, height: i32) {
        let i = self.imp();
        i.amp.set(amplitude.max(0.0));
        i.wavelength.set(wavelength.max(6.0));
        i.stroke.set(stroke.max(1.0));
        i.height.set(height.max(8));
        self.queue_resize();
    }

    pub fn set_accent_color(&self, c: Option<gdk::RGBA>) { *self.imp().accent.borrow_mut() = c; self.queue_draw(); }
    pub fn set_dim_color(&self, c: Option<gdk::RGBA>) { *self.imp().dim.borrow_mut() = c; self.queue_draw(); }

    /// Animate the wave (true) or hold it still (false). The frame callback exists only while playing.
    pub fn set_playing(&self, on: bool) {
        let i = self.imp();
        if i.playing.replace(on) == on { return; }
        if on {
            i.last.set(0);
            let id = self.add_tick_callback(|w, clock| {
                let i = w.imp();
                let now = clock.frame_time();
                let last = i.last.replace(now);
                if last != 0 { i.phase.set((i.phase.get() + (now - last) as f32 / 1_000_000.0 * 2.4) % (std::f32::consts::TAU * 1000.0)); }
                w.queue_draw();
                glib::ControlFlow::Continue
            });
            *i.tick.borrow_mut() = Some(id);
        } else if let Some(id) = i.tick.take() {
            id.remove();
            self.queue_draw();
        }
    }

    fn pad(&self) -> f32 { self.imp().stroke.get() / 2.0 + 2.0 }

    fn seek_to_x(&self, x: f64) {
        let (pad, w) = (self.pad() as f64, self.width() as f64);
        let span = (w - 2.0 * pad).max(1.0);
        self.seek_fraction(((x - pad) / span).clamp(0.0, 1.0));
    }

    fn seek_fraction(&self, f: f64) {
        self.imp().progress.set(f);
        self.queue_draw();
        let cb = self.imp().seek.borrow().clone();
        if let Some(cb) = cb { cb(f); }
    }

    fn draw(&self, snap: &gtk::Snapshot) {
        let i = self.imp();
        let (w, h) = (self.width() as f32, self.height() as f32);
        let (accent, dim) = palette(self, i.accent.borrow().clone(), i.dim.borrow().clone());
        let (pad, stroke, amp, wl) = (self.pad(), i.stroke.get(), i.amp.get(), i.wavelength.get());
        let span = (w - 2.0 * pad).max(1.0);
        let px = pad + span * i.progress.get() as f32;
        let cy = h / 2.0;
        let cap = |s: &gsk::Stroke| s.set_line_cap(gsk::LineCap::Round);
        // Rest of the track: a straight line.
        if px + stroke < w - pad {
            let b = gsk::PathBuilder::new();
            b.move_to(px + stroke, cy);
            b.line_to(w - pad, cy);
            let s = gsk::Stroke::new(stroke);
            cap(&s);
            snap.append_stroke(&b.to_path(), &s, &dim);
        }
        // Played part: the wave. It eases to flat over the first wavelength so it starts on the baseline.
        if px > pad {
            let b = gsk::PathBuilder::new();
            let k = std::f32::consts::TAU / wl;
            let mut x = pad;
            let y = |x: f32| cy + amp * ((x - pad) / wl).min(1.0) * (k * (x - pad) - i.phase.get()).sin();
            b.move_to(x, y(x));
            while x < px - 1.0 { x = (x + 2.0).min(px - 1.0); b.line_to(x, y(x)); if x >= px - 1.0 { break; } }
            let s = gsk::Stroke::new(stroke);
            cap(&s);
            s.set_line_join(gsk::LineJoin::Round);
            snap.append_stroke(&b.to_path(), &s, &accent);
        }
        // Handle.
        let hh = (h - 4.0).min(amp * 2.0 + stroke * 3.0).max(stroke * 2.0);
        let hw = 4.0;
        let r = rrect(px - hw / 2.0, cy - hh / 2.0, hw, hh, hw / 2.0);
        let b = gsk::PathBuilder::new();
        b.add_rounded_rect(&r);
        snap.append_fill(&b.to_path(), gsk::FillRule::Winding, &accent);
    }
}

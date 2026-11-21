//! macOS: stop drawing a window while it can't be seen (fully covered, minimized, on another Space, screen locked).
//!
//! On Wayland the compositor withholds frame callbacks from hidden windows, so the lyrics backdrop, the wavy seek bar
//! and the progress tickers cost nothing there. GTK's macOS backend keeps its display-link frame clock painting
//! regardless, so here we watch `NSWindowDidChangeOcclusionStateNotification` and pause our own animations while the
//! window is occluded: widgets registered with [`follow`] (the lyrics view, the wavy seek bar) drop their frame-clock
//! ticks, and periodic tickers skip their work when [`hidden`] says so. With nothing invalidating the window GTK renders
//! nothing. (GDK's `render` signal itself must not be blocked — see `imp::Watch`.)
//! It also works around a GTK macOS stall after Space switches (window stops repainting until resized) — see `imp::Watch`.
//! `HARV_DEBUG_OCCLUSION=1` logs; `HARV_NO_OCCLUSION=1` turns the animation pausing off, `HARV_NO_UNSTICK=1` the stall fix.

use gtk::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    /// Windows currently occluded (only ever filled on macOS).
    static HIDDEN: RefCell<Vec<glib::WeakRef<gtk::Window>>> = RefCell::default();
    static FOLLOWERS: RefCell<Vec<(glib::WeakRef<gtk::Widget>, Rc<dyn Fn(bool)>)>> = RefCell::default();
}

/// Whether `w`'s window is currently occluded. Always false off macOS.
pub fn hidden(w: &impl IsA<gtk::Widget>) -> bool {
    let Some(win) = w.as_ref().root().and_downcast::<gtk::Window>() else { return false };
    HIDDEN.with(|h| h.borrow().iter().any(|x| x.upgrade().as_ref() == Some(&win)))
}

/// Call `f(true)` when `w`'s window becomes occluded and `f(false)` when it shows again (never called off macOS).
pub fn follow<W: IsA<gtk::Widget>>(w: &W, f: impl Fn(&W, bool) + 'static) {
    let weak = w.downgrade();
    let cb: Rc<dyn Fn(bool)> = Rc::new(move |h| if let Some(w) = weak.upgrade() { f(&w, h) });
    FOLLOWERS.with(|v| v.borrow_mut().push((w.upcast_ref::<gtk::Widget>().downgrade(), cb)));
}

/// Record `win`'s new state and tell the followers inside it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn changed(win: &gtk::Window, hidden: bool) {
    HIDDEN.with(|h| {
        let mut h = h.borrow_mut();
        h.retain(|x| x.upgrade().is_some_and(|x| &x != win));
        if hidden { h.push(win.downgrade()); }
    });
    let due: Vec<Rc<dyn Fn(bool)>> = FOLLOWERS.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|(w, _)| w.upgrade().is_some());
        v.iter().filter(|(w, _)| w.upgrade().and_then(|w| w.root()).and_downcast::<gtk::Window>().as_ref() == Some(win)).map(|(_, f)| f.clone()).collect()
    });
    for f in due { f(hidden); }
}

#[cfg(target_os = "macos")]
pub fn watch(win: &impl IsA<gtk::Window>) {
    let win = win.upcast_ref::<gtk::Window>();
    win.connect_realize(imp::attach);
    if win.is_realized() { imp::attach(win); }
}

#[cfg(not(target_os = "macos"))]
pub fn watch(_win: &impl IsA<gtk::Window>) {}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use gtk::gdk;
    use gtk::glib::translate::ToGlibPtr;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, ProtocolObject};
    use objc2_app_kit::{NSWindow, NSWindowDidChangeOcclusionStateNotification, NSWindowOcclusionState, NSWorkspace, NSWorkspaceActiveSpaceDidChangeNotification};
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol};
    use std::cell::Cell;
    use std::ptr::NonNull;
    use std::time::Duration;

    unsafe extern "C" { fn gdk_macos_surface_get_native_window(surface: *mut gdk::ffi::GdkSurface) -> *mut std::ffi::c_void; }

    fn debug() -> bool { std::env::var_os("HARV_DEBUG_OCCLUSION").is_some() }

    /// Never block or swallow GDK's own `render` signal here: the macOS backend shows a new window (`showAndMakeKey`)
    /// only after its first buffer swap and paces its frame clock on frames it submitted, so suppressing renders left
    /// the window blank/unfocused at startup and froze it after being covered. We only pause our own animations and
    /// tickers; with nothing invalidating the window, GTK has nothing to render and idles by itself.
    ///
    /// Stall workaround (GTK 4.24 macOS backend): after every paint GDK freezes the surface's updates until the monitor's
    /// CVDisplayLink reports the frame presented. Around a Space switch that callback can get lost, and the window then
    /// never repaints again (input is still handled, so clicks "do nothing") until a resize runs
    /// `_gdk_macos_surface_monitor_changed`, which cancels the pending frame. We detect the stall with a probe (queue a
    /// draw, see whether the frame clock paints within [`PROBE`]) and fix it the same way a resize does, through the
    /// window's public delegate method `windowDidChangeScreen:`. `HARV_NO_UNSTICK=1` turns the workaround off.
    struct Watch {
        ns: Retained<NSWindow>,
        win: glib::WeakRef<gtk::Window>,
        hidden: Cell<bool>,
        seen: Cell<bool>,
        pausing: bool,
        unstick: bool,
        /// Bumped on every frame-clock `after-paint`.
        paints: Rc<Cell<u64>>,
        probing: Cell<bool>,
    }

    const PROBE: Duration = Duration::from_millis(500);

    impl Watch {
        fn visible(&self) -> bool { self.ns.occlusionState().contains(NSWindowOcclusionState::Visible) }

        fn set_hidden(self: &Rc<Self>, hidden: bool) {
            // Before the window has been on screen once it reports "not visible"; that isn't occlusion.
            if !hidden { self.seen.set(true); }
            if hidden && !self.seen.get() { return; }
            if self.hidden.replace(hidden) == hidden { return; }
            if debug() { eprintln!("occlusion: {}", if hidden { "hidden — animations paused" } else { "visible — animations resumed" }); }
            let Some(win) = self.win.upgrade() else { return };
            // Defer out of the AppKit notification: followers add/remove frame-clock ticks and queue draws, which
            // shouldn't run re-entrantly inside whatever AppKit/GDK call posted the notification.
            let me = self.clone();
            glib::idle_add_local_once(move || {
                if me.pausing { super::changed(&win, hidden); }
                if !hidden { win.queue_draw(); me.probe(); }
            });
        }

        /// Ask for a repaint; if the frame clock hasn't painted within [`PROBE`] while the window is visible, the surface
        /// is stuck waiting for a lost display-link frame — unstick it.
        fn probe(self: &Rc<Self>) {
            if !self.unstick || self.probing.replace(true) { return; }
            let Some(win) = self.win.upgrade() else { self.probing.set(false); return };
            let before = self.paints.get();
            win.queue_draw();
            let me = self.clone();
            glib::timeout_add_local_once(PROBE, move || {
                me.probing.set(false);
                if me.paints.get() != before || !me.visible() { return; }
                let Some(win) = me.win.upgrade() else { return };
                if !win.is_mapped() { return; }
                if debug() { eprintln!("occlusion: no frame painted in {PROBE:?} — unsticking the GDK surface"); }
                me.kick(&win);
            });
        }

        fn kick(&self, win: &gtk::Window) {
            let sel = objc2::sel!(windowDidChangeScreen:);
            if !self.ns.respondsToSelector(sel) { return; }
            unsafe { let _: () = objc2::msg_send![&*self.ns, windowDidChangeScreen: std::ptr::null::<AnyObject>()]; }
            win.queue_draw();
        }
    }

    pub fn attach(win: &gtk::Window) {
        let Some(surface) = win.surface() else { return };
        if !surface.type_().name().starts_with("GdkMacos") { return; }
        let ns = unsafe { gdk_macos_surface_get_native_window(surface.to_glib_none().0) } as *mut NSWindow;
        let Some(ns) = (unsafe { Retained::retain(ns) }) else { return };
        let pausing = std::env::var_os("HARV_NO_OCCLUSION").is_none();
        let unstick = std::env::var_os("HARV_NO_UNSTICK").is_none();
        if !pausing && !unstick { return; }

        let paints: Rc<Cell<u64>> = Rc::default();
        let clock = surface.frame_clock();
        let paint_id = { let p = paints.clone(); clock.connect_after_paint(move |_| p.set(p.get().wrapping_add(1))) };
        let watch = Rc::new(Watch { ns: ns.clone(), win: win.downgrade(), hidden: Cell::new(false), seen: Cell::new(false), pausing, unstick, paints, probing: Cell::new(false) });

        let occl = { let w = watch.clone(); block2::RcBlock::new(move |_n: NonNull<NSNotification>| w.set_hidden(!w.visible())) };
        // A Space switch: the display-link frame is what goes missing; probe once the switch animation is over.
        let space = {
            let w = Rc::downgrade(&watch);
            block2::RcBlock::new(move |_n: NonNull<NSNotification>| {
                let w = w.clone();
                glib::timeout_add_local_once(Duration::from_millis(400), move || if let Some(w) = w.upgrade() { if w.visible() { w.probe(); } });
            })
        };
        let center = NSNotificationCenter::defaultCenter();
        let ws_center = NSWorkspace::sharedWorkspace().notificationCenter();
        // queue = None: delivered synchronously on the posting (main) thread.
        let tokens: Vec<(Retained<NSNotificationCenter>, Retained<ProtocolObject<dyn NSObjectProtocol>>)> = unsafe { vec![
            (center.clone(), center.addObserverForName_object_queue_usingBlock(Some(NSWindowDidChangeOcclusionStateNotification), Some(&*(Retained::as_ptr(&ns) as *const AnyObject)), None, &occl)),
            (ws_center.clone(), ws_center.addObserverForName_object_queue_usingBlock(Some(NSWorkspaceActiveSpaceDidChangeNotification), None, None, &space)),
        ] };
        watch.set_hidden(!watch.visible());

        // Any click or key press that doesn't get a repaint means the surface is stuck: probe on input as well.
        let input = gtk::EventControllerLegacy::new();
        input.set_propagation_phase(gtk::PropagationPhase::Capture);
        { let w = Rc::downgrade(&watch); input.connect_event(move |_, ev| {
            if matches!(ev.event_type(), gdk::EventType::ButtonPress | gdk::EventType::KeyPress) { if let Some(w) = w.upgrade() { w.probe(); } }
            glib::Propagation::Proceed
        }); }
        win.add_controller(input.clone());

        // The surface goes away on unrealize (a re-realize attaches afresh).
        let state = RefCell::new(Some((tokens, paint_id, clock, input)));
        let id: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
        let id2 = id.clone();
        *id.borrow_mut() = Some(win.connect_unrealize(move |w| {
            if let Some((tokens, paint_id, clock, input)) = state.borrow_mut().take() {
                for (c, t) in tokens { unsafe { c.removeObserver(&*(Retained::as_ptr(&t) as *const AnyObject)) }; }
                clock.disconnect(paint_id);
                w.remove_controller(&input);
            }
            watch.set_hidden(false);
            if let Some(id) = id2.borrow_mut().take() { w.disconnect(id); }
        }));
    }

}

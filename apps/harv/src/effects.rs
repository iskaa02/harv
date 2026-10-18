//! Audio effects dialog (EQ, speed, pitch, reverb, delay, AutoEq). Edits a local copy of `AudioFx` that starts from the
//! engine's current one; changes reach the engine 120 ms after the last edit and are persisted under the "fx" setting.
use crate::ctx::Ctx;
use adw::prelude::*;
use gtk::glib;
use player_core::{AudioFx, Command, Reverb, EQ_PRESETS};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

const BAND_LABELS: [&str; 10] = ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
const REVERBS: [(&str, Reverb); 4] = [("Off", Reverb::Off), ("Room", Reverb::Room), ("Hall", Reverb::Hall), ("Cathedral", Reverb::Cathedral)];

struct Widgets {
    eq: adw::SwitchRow,
    preset: adw::ComboRow,
    preamp: adw::SpinRow,
    bands: gtk::Box,
    scales: Vec<gtk::Scale>,
    speed: gtk::Scale,
    speed_lbl: gtk::Label,
    pitch: gtk::Scale,
    pitch_lbl: gtk::Label,
    reverb: adw::ComboRow,
    delay: gtk::Scale,
    delay_lbl: gtk::Label,
    active: adw::ActionRow,
    remove: gtk::Button,
    entry: adw::EntryRow,
    spinner: gtk::Spinner,
    status: gtk::Label,
    results: gtk::ListBox,
    results_scroll: gtk::ScrolledWindow,
    reset_all: gtk::Button,
}

struct Dlg {
    ctx: Rc<Ctx>,
    fx: RefCell<AudioFx>,
    w: Widgets,
    /// Set while we write widget values ourselves, so their change handlers stay quiet.
    guard: Cell<bool>,
    timer: Cell<Option<glib::SourceId>>,
    /// The AutoEq headphone list, fetched once per opening (cached on disk by the crate).
    index: RefCell<Option<Vec<autoeq::Entry>>>,
    shown: RefCell<Vec<autoeq::Entry>>,
}

fn slider(lo: f64, hi: f64, step: f64, px: i32) -> gtk::Scale {
    let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, lo, hi, step);
    s.set_draw_value(false);
    s.set_size_request(px, -1);
    s.set_valign(gtk::Align::Center);
    s
}

fn value_label() -> gtk::Label { gtk::Label::builder().width_chars(6).xalign(1.0).css_classes(["song-time"]).build() }

fn reset_button() -> gtk::Button { gtk::Button::builder().icon_name("edit-undo-symbolic").tooltip_text("Reset").valign(gtk::Align::Center).css_classes(["flat", "circular"]).build() }

/// A preferences row whose right side holds controls.
fn suffix_row(title: &str, parts: &[&gtk::Widget]) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).build();
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    b.set_valign(gtk::Align::Center);
    for p in parts { b.append(*p); }
    row.add_suffix(&b);
    row
}

fn speed_text(v: f64) -> String { format!("{v:.2}×") }
fn pitch_text(v: f32) -> String { if v.round() == 0.0 { "0 st".into() } else { format!("{:+} st", v.round() as i32) } }
fn delay_text(ms: u32) -> String { if ms == 0 { "Off".into() } else { format!("{ms} ms") } }

/// Index of the built-in preset that matches `fx`, or the "Custom" slot after the presets.
fn match_preset(fx: &AudioFx) -> u32 {
    EQ_PRESETS.iter().position(|(_, pre, g)| (fx.preamp_db - pre).abs() < 0.05 && fx.eq_gains.iter().zip(g).all(|(a, b)| (a - b).abs() < 0.05)).map_or(EQ_PRESETS.len() as u32, |i| i as u32)
}

fn build() -> (Widgets, adw::PreferencesPage) {
    let page = adw::PreferencesPage::new();

    // ── equalizer
    let eq_group = adw::PreferencesGroup::builder().title("Equalizer").build();
    let eq = adw::SwitchRow::builder().title("Equalizer").build();
    let names: Vec<&str> = EQ_PRESETS.iter().map(|p| p.0).chain(std::iter::once("Custom")).collect();
    let preset = adw::ComboRow::builder().title("Preset").model(&gtk::StringList::new(&names)).build();
    let preamp = adw::SpinRow::with_range(-12.0, 12.0, 0.5);
    preamp.set_title("Preamp");
    preamp.set_subtitle("dB, applied before the bands");
    preamp.set_digits(1);
    let bands = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    bands.set_homogeneous(true);
    bands.set_margin_top(10);
    bands.set_margin_bottom(10);
    bands.set_margin_start(8);
    bands.set_margin_end(8);
    let mut scales = vec![];
    for label in BAND_LABELS {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let s = gtk::Scale::with_range(gtk::Orientation::Vertical, -12.0, 12.0, 0.5);
        s.set_draw_value(false);
        s.set_inverted(true);
        s.set_size_request(-1, 130);
        s.add_mark(0.0, gtk::PositionType::Right, None);
        col.append(&s);
        col.append(&gtk::Label::builder().label(label).css_classes(["caption", "dim"]).build());
        bands.append(&col);
        scales.push(s);
    }
    eq_group.add(&eq);
    eq_group.add(&preset);
    eq_group.add(&preamp);
    eq_group.add(&bands);
    page.add(&eq_group);

    // ── playback
    let play_group = adw::PreferencesGroup::builder().title("Playback").build();
    let speed = slider(0.5, 2.0, 0.05, 180);
    let speed_lbl = value_label();
    let speed_reset = reset_button();
    { let s = speed.clone(); speed_reset.connect_clicked(move |_| s.set_value(1.0)); }
    play_group.add(&suffix_row("Speed", &[speed.upcast_ref(), speed_lbl.upcast_ref(), speed_reset.upcast_ref()]));
    let pitch = slider(-12.0, 12.0, 1.0, 180);
    let pitch_lbl = value_label();
    let pitch_reset = reset_button();
    { let s = pitch.clone(); pitch_reset.connect_clicked(move |_| s.set_value(0.0)); }
    play_group.add(&suffix_row("Pitch", &[pitch.upcast_ref(), pitch_lbl.upcast_ref(), pitch_reset.upcast_ref()]));
    page.add(&play_group);

    // ── space
    let space_group = adw::PreferencesGroup::builder().title("Space").build();
    let reverb_names: Vec<&str> = REVERBS.iter().map(|r| r.0).collect();
    let reverb = adw::ComboRow::builder().title("Reverb").subtitle("Multi-tap echo approximation").model(&gtk::StringList::new(&reverb_names)).build();
    let delay = slider(0.0, 1000.0, 50.0, 180);
    let delay_lbl = value_label();
    space_group.add(&reverb);
    space_group.add(&suffix_row("Delay", &[delay.upcast_ref(), delay_lbl.upcast_ref()]));
    page.add(&space_group);

    // ── headphones (AutoEq)
    let hp_group = adw::PreferencesGroup::builder().title("Headphones (AutoEq)").description("Corrects the sound toward the AutoEq target for a headphone model.").build();
    let active = adw::ActionRow::builder().title("Active profile").subtitle("None").build();
    let remove = gtk::Button::builder().label("Remove").valign(gtk::Align::Center).css_classes(["flat"]).sensitive(false).build();
    active.add_suffix(&remove);
    hp_group.add(&active);
    let entry = adw::EntryRow::builder().title("Search headphones").build();
    hp_group.add(&entry);
    let spinner = gtk::Spinner::new();
    let status = gtk::Label::builder().xalign(0.0).hexpand(true).wrap(true).css_classes(["dim"]).build();
    let st = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    st.set_margin_top(6);
    st.set_margin_bottom(6);
    st.set_margin_start(12);
    st.set_margin_end(12);
    st.append(&spinner);
    st.append(&status);
    hp_group.add(&st);
    let results = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    let scroll = gtk::ScrolledWindow::builder().child(&results).max_content_height(260).propagate_natural_height(true).hscrollbar_policy(gtk::PolicyType::Never).build();
    hp_group.add(&scroll);
    scroll.set_visible(false);
    page.add(&hp_group);

    let reset_all = gtk::Button::builder().label("Reset all").css_classes(["flat"]).build();
    let w = Widgets { eq, preset, preamp, bands, scales, speed, speed_lbl, pitch, pitch_lbl, reverb, delay, delay_lbl, active, remove, entry, spinner, status, results, results_scroll: scroll, reset_all };
    (w, page)
}

impl Dlg {
    fn weak(self: &Rc<Self>) -> Weak<Self> { Rc::downgrade(self) }

    /// Debounced: push to the engine and persist 120 ms after the last edit.
    fn changed(self: &Rc<Self>) {
        if let Some(id) = self.timer.take() { id.remove(); }
        let d = self.clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(120), move || {
            d.timer.take();
            let fx = d.fx.borrow().clone();
            d.ctx.engine.send(Command::SetFx(fx.clone()));
            let _ = d.ctx.store.set("fx", &fx);
        });
        self.timer.set(Some(id));
    }

    /// Writes the whole local state into the widgets (with handlers muted).
    fn sync(&self) {
        self.guard.set(true);
        let fx = self.fx.borrow().clone();
        let w = &self.w;
        w.eq.set_active(fx.eq_on);
        w.preset.set_selected(match_preset(&fx));
        w.preamp.set_value(fx.preamp_db as f64);
        for (s, g) in w.scales.iter().zip(fx.eq_gains) { s.set_value(g as f64); }
        w.speed.set_value(fx.speed);
        w.speed_lbl.set_label(&speed_text(fx.speed));
        w.pitch.set_value(fx.pitch_semitones as f64);
        w.pitch_lbl.set_label(&pitch_text(fx.pitch_semitones));
        w.reverb.set_selected(REVERBS.iter().position(|r| r.1 == fx.reverb).unwrap_or(0) as u32);
        w.delay.set_value(fx.delay_ms as f64);
        w.delay_lbl.set_label(&delay_text(fx.delay_ms));
        w.active.set_subtitle(fx.autoeq_name.as_deref().unwrap_or("None"));
        w.remove.set_sensitive(fx.autoeq_name.is_some() || !fx.parametric.is_empty());
        self.guard.set(false);
        self.sensitivity();
    }

    /// The EQ controls only do something while the equalizer is on.
    fn sensitivity(&self) {
        let on = self.fx.borrow().eq_on;
        for w in [self.w.preset.upcast_ref::<gtk::Widget>(), self.w.preamp.upcast_ref(), self.w.bands.upcast_ref()] { w.set_sensitive(on); }
    }

    /// Moving a band or the preamp turns the preset into "Custom" (without re-applying anything).
    fn to_custom(&self) {
        self.guard.set(true);
        self.w.preset.set_selected(EQ_PRESETS.len() as u32);
        self.guard.set(false);
    }

    fn set_status(&self, msg: &str, busy: bool) {
        self.w.status.set_label(msg);
        self.w.status.set_visible(!msg.is_empty());
        self.w.spinner.set_spinning(busy);
        self.w.spinner.set_visible(busy);
    }

    fn search(&self) {
        let q = self.w.entry.text();
        let idx = self.index.borrow();
        let Some(list) = idx.as_ref() else { self.set_status("Loading list…", true); return };
        while let Some(c) = self.w.results.first_child() { self.w.results.remove(&c); }
        self.w.results_scroll.set_visible(false);
        if q.trim().is_empty() {
            self.shown.borrow_mut().clear();
            self.set_status("Type part of a headphone name, e.g. “HD 600”", false);
            return;
        }
        let hits: Vec<autoeq::Entry> = autoeq::search(list, q.as_str()).into_iter().cloned().collect();
        self.w.results_scroll.set_visible(!hits.is_empty());
        for e in &hits {
            let row = adw::ActionRow::builder().title(e.name.as_str()).subtitle(e.source.as_str()).activatable(true).build();
            self.w.results.append(&row);
        }
        let msg = match hits.len() { 0 => "No matches".to_string(), n if n >= 60 => "First 60 matches".to_string(), n => format!("{n} matches") };
        self.set_status(&msg, false);
        *self.shown.borrow_mut() = hits;
    }

    fn pick(self: &Rc<Self>, e: autoeq::Entry) {
        self.set_status(&format!("Fetching {}…", e.name), true);
        let (name, d) = (e.name.clone(), self.clone());
        self.ctx.spawn(async move { let http = reqwest::Client::new(); autoeq::profile(&http, &e).await.map_err(|err| format!("{err:#}")) }, move |res| {
            match res {
                Ok(p) => {
                    { let mut fx = d.fx.borrow_mut(); fx.parametric = p.filters; fx.autoeq_preamp_db = p.preamp_db; fx.autoeq_name = Some(name.clone()); }
                    d.sync();
                    d.changed();
                    d.ctx.toast(&format!("AutoEq profile: {name}"));
                }
                Err(err) => d.ctx.toast(&format!("Couldn't load {name}: {err}")),
            }
            d.search();
        });
    }

    /// Every handler: ignore while `guard` is set, otherwise edit `fx` and schedule a push.
    fn connect(self: &Rc<Self>) {
        let w = &self.w;
        { let d = self.weak(); w.eq.connect_active_notify(move |r| { let Some(d) = d.upgrade() else { return }; if d.guard.get() { return } d.fx.borrow_mut().eq_on = r.is_active(); d.sensitivity(); d.changed(); }); }
        { let d = self.weak(); w.preset.connect_selected_notify(move |r| {
            let Some(d) = d.upgrade() else { return };
            if d.guard.get() { return }
            let Some((_, pre, gains)) = EQ_PRESETS.get(r.selected() as usize) else { return };
            { let mut fx = d.fx.borrow_mut(); fx.eq_on = true; fx.preamp_db = *pre; fx.eq_gains = *gains; }
            d.sync();
            d.changed();
        }); }
        { let d = self.weak(); w.preamp.connect_value_notify(move |r| { let Some(d) = d.upgrade() else { return }; if d.guard.get() { return } d.fx.borrow_mut().preamp_db = r.value() as f32; d.to_custom(); d.changed(); }); }
        for (i, s) in w.scales.iter().enumerate() {
            let d = self.weak();
            s.connect_value_changed(move |s| { let Some(d) = d.upgrade() else { return }; if d.guard.get() { return } d.fx.borrow_mut().eq_gains[i] = s.value() as f32; d.to_custom(); d.changed(); });
        }
        { let d = self.weak(); w.speed.connect_value_changed(move |s| { let Some(d) = d.upgrade() else { return }; if d.guard.get() { return } d.fx.borrow_mut().speed = s.value(); d.w.speed_lbl.set_label(&speed_text(s.value())); d.changed(); }); }
        { let d = self.weak(); w.pitch.connect_value_changed(move |s| { let Some(d) = d.upgrade() else { return }; if d.guard.get() { return } let v = s.value() as f32; d.fx.borrow_mut().pitch_semitones = v; d.w.pitch_lbl.set_label(&pitch_text(v)); d.changed(); }); }
        { let d = self.weak(); w.reverb.connect_selected_notify(move |r| { let Some(d) = d.upgrade() else { return }; if d.guard.get() { return } if let Some((_, rv)) = REVERBS.get(r.selected() as usize) { d.fx.borrow_mut().reverb = *rv; d.changed(); } }); }
        { let d = self.weak(); w.delay.connect_value_changed(move |s| { let Some(d) = d.upgrade() else { return }; if d.guard.get() { return } let ms = s.value() as u32; d.fx.borrow_mut().delay_ms = ms; d.w.delay_lbl.set_label(&delay_text(ms)); d.changed(); }); }
        { let d = self.weak(); w.entry.connect_changed(move |_| { if let Some(d) = d.upgrade() { d.search(); } }); }
        { let d = self.weak(); w.results.connect_row_activated(move |_, row| {
            let Some(d) = d.upgrade() else { return };
            let e = d.shown.borrow().get(row.index() as usize).cloned();
            if let Some(e) = e { d.pick(e); }
        }); }
        { let d = self.weak(); w.remove.connect_clicked(move |_| {
            let Some(d) = d.upgrade() else { return };
            { let mut fx = d.fx.borrow_mut(); fx.parametric.clear(); fx.autoeq_name = None; fx.autoeq_preamp_db = 0.0; }
            d.sync();
            d.changed();
        }); }
        { let d = self.weak(); w.reset_all.connect_clicked(move |_| { let Some(d) = d.upgrade() else { return }; *d.fx.borrow_mut() = AudioFx::default(); d.sync(); d.changed(); }); }
    }
}

pub fn show(ctx: &Rc<Ctx>) {
    let fx = ctx.snap.borrow().as_ref().map(|s| s.fx.clone()).unwrap_or_default();
    let (w, page) = build();
    let tv = adw::ToolbarView::new();
    let hb = adw::HeaderBar::new();
    hb.pack_end(&w.reset_all);
    tv.add_top_bar(&hb);
    tv.set_content(Some(&page));
    let dialog = adw::Dialog::builder().title("Audio effects").content_width(520).content_height(680).build();
    dialog.set_child(Some(&tv));

    let d = Rc::new(Dlg { ctx: ctx.clone(), fx: RefCell::new(fx), w, guard: Cell::new(false), timer: Cell::new(None), index: RefCell::default(), shown: RefCell::default() });
    d.connect();
    d.sync();
    if d.index.borrow().is_none() {
        d.set_status("Loading list…", true);
        let d2 = d.clone();
        ctx.spawn(async move { let http = reqwest::Client::new(); autoeq::index(&http).await.map_err(|e| format!("{e:#}")) }, move |res| match res {
            Ok(list) => { *d2.index.borrow_mut() = Some(list); d2.search(); }
            Err(e) => d2.set_status(&format!("Couldn't load the headphone list: {e}"), false),
        });
    }
    // The dialog owns the state while it is open; dropping it on close releases the widgets.
    let keep = d.clone();
    dialog.connect_closed(move |_| { let _ = &keep; });
    dialog.present(ctx.toasts.root().as_ref());
}

/// Re-apply the saved effects at startup.
pub fn restore(ctx: &Rc<Ctx>) {
    if let Some(fx) = ctx.store.get::<AudioFx>("fx") { ctx.engine.send(Command::SetFx(fx)); }
    // Dev aid: open the dialog shortly after launch for screenshots (`HARV_OPEN_EFFECTS=1`).
    if std::env::var_os("HARV_OPEN_EFFECTS").is_some() {
        let c = ctx.clone();
        glib::timeout_add_local_once(Duration::from_millis(1500), move || show(&c));
    }
}

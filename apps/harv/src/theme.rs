//! Applies a `theme_core::Theme` to GTK: the stylesheet provider (template + tokens + the theme's own CSS), the colour
//! scheme, live switching, and (with `--watch`) hot reload of `style.css` and theme folders.

use crate::ctx::Ctx;
use adw::prelude::*;
use gtk::{gdk, gio};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use theme_core::{Loader, Theme};

/// The stylesheet template: `style.css` plus one file per component area (each owned by its component module).
const PARTS: [(&str, &str); 7] = [
    ("style.css", include_str!("style.css")),
    ("components/sidebar.css", include_str!("components/sidebar.css")),
    ("components/detail.css", include_str!("components/detail.css")),
    ("components/panel.css", include_str!("components/panel.css")),
    ("components/bar.css", include_str!("components/bar.css")),
    ("components/now.css", include_str!("components/now.css")),
    ("components/search.css", include_str!("components/search.css")),
];
const SRC_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
/// Stylesheet template and built-in themes in the source tree (only meaningful when running from the repo, like `--watch` always did).
const CSS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/style.css");
/// Template text: compiled-in parts, or (dev) the files on disk.
fn template(watch: bool) -> Result<String, String> {
    let mut out = String::new();
    for (name, built_in) in PARTS {
        if watch { out += &std::fs::read_to_string(Path::new(SRC_DIR).join(name)).map_err(|e| format!("cannot read {SRC_DIR}/{name}: {e}"))?; } else { out += built_in; }
        out.push('\n');
    }
    Ok(out)
}
const DEV_THEMES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/theme-core/themes");

/// Preferences → Appearance → Style: force a scheme, or follow the desktop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode { System, Light, Dark }

impl Mode {
    pub const ALL: [(Mode, &'static str); 3] = [(Mode::System, "System"), (Mode::Light, "Light"), (Mode::Dark, "Dark")];
    pub fn key(self) -> &'static str { match self { Mode::System => "system", Mode::Light => "light", Mode::Dark => "dark" } }
    /// Store value → mode; the default is Dark (the look harv shipped with).
    pub fn parse(s: &str) -> Mode { match s { "system" => Mode::System, "light" => Mode::Light, _ => Mode::Dark } }
}

pub struct Skin {
    provider: gtk::CssProvider,
    loader: Loader,
    /// Dev mode: read the template and built-in themes from the source tree instead of the compiled-in copies.
    watch: bool,
    /// The spec (id or path) of the active theme, for reloads.
    spec: RefCell<String>,
    mode: Cell<Mode>,
    /// Preferences → Layout → Density: 0 compact, 1 comfortable (the theme's own row height), 2 spacious.
    density: Cell<u32>,
    /// Chosen palette variant per theme spec (`[variants.*]`); absent = the theme's own palette.
    variants: RefCell<HashMap<String, String>>,
}

thread_local! { static SKIN: RefCell<Option<Rc<Skin>>> = const { RefCell::new(None) }; }

/// The skin installed by [`Skin::install`] (main thread only).
pub fn skin() -> Option<Rc<Skin>> { SKIN.with(|s| s.borrow().clone()) }

impl Skin {
    /// Create the stylesheet provider and register it with the display.
    pub fn install(watch: bool) -> Rc<Skin> {
        let provider = gtk::CssProvider::new();
        provider.connect_parsing_error(|_, section, err| eprintln!("css: line {}: {err}", section.start_location().lines() + 1));
        gtk::style_context_add_provider_for_display(&gdk::Display::default().unwrap(), &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        let mut dirs = vec![theme_core::user_theme_dir()];
        if watch { dirs.insert(0, PathBuf::from(DEV_THEMES)); }
        let skin = Rc::new(Skin { provider, loader: Loader::new(dirs), watch, spec: RefCell::new(theme_core::DEFAULT.into()), mode: Cell::new(Mode::Dark), density: Cell::new(1), variants: RefCell::new(HashMap::new()) });
        SKIN.with(|s| *s.borrow_mut() = Some(skin.clone()));
        skin
    }

    pub fn mode(&self) -> Mode { self.mode.get() }

    /// Startup: the saved variant choices (theme id → variant id).
    pub fn set_variants(&self, v: HashMap<String, String>) { *self.variants.borrow_mut() = v; }

    pub fn variants(&self) -> HashMap<String, String> { self.variants.borrow().clone() }

    /// The variant chosen for theme `spec`, if any.
    pub fn variant_of(&self, spec: &str) -> Option<String> { self.variants.borrow().get(spec).cloned() }

    /// Remember (or clear) the variant of `spec`; applies on the next load / [`Skin::switch`].
    pub fn set_variant(&self, spec: &str, variant: Option<&str>) {
        let mut m = self.variants.borrow_mut();
        match variant { Some(v) => { m.insert(spec.into(), v.into()); } None => { m.remove(spec); } }
    }

    pub fn density(&self) -> u32 { self.density.get() }

    /// Set the row density; takes effect on the next [`Skin::switch`] (or at startup, before the first load).
    pub fn set_density(&self, d: u32) { self.density.set(d.min(2)); crate::rows::set_density(d); }

    /// Startup: set the mode before the first [`Skin::load_or_default`].
    pub fn set_initial_mode(&self, mode: Mode) { self.mode.set(mode); }

    /// Tell libadwaita which scheme to use and report whether the UI is light (System follows the desktop).
    fn sync_scheme(&self) -> bool {
        let sm = adw::StyleManager::default();
        sm.set_color_scheme(match self.mode.get() { Mode::System => adw::ColorScheme::Default, Mode::Light => adw::ColorScheme::ForceLight, Mode::Dark => adw::ColorScheme::ForceDark });
        !sm.is_dark()
    }

    /// Load a theme in the current scheme.
    fn load_resolved(&self, spec: &str) -> Result<Theme, String> {
        let light = self.sync_scheme();
        let variant = self.variant_of(spec);
        self.loader.load_with(spec, variant.as_deref()).map(|t| {
            let mut t = t.resolved(light);
            // Density scales the theme's own row height; the middle setting leaves the theme untouched.
            t.metrics.track_row_h = match self.density.get() { 0 => (t.metrics.track_row_h * 4 + 2) / 5, 2 => t.metrics.track_row_h * 5 / 4, _ => t.metrics.track_row_h };
            t
        })
    }

    /// Change the mode and repaint the active theme in it.
    pub fn set_mode(&self, ctx: &Rc<Ctx>, mode: Mode) -> Result<(), String> {
        self.mode.set(mode);
        let spec = self.current_id();
        self.switch(ctx, &spec)
    }

    /// In System mode, repaint when the desktop flips between light and dark.
    pub fn follow_system(self: &Rc<Self>, ctx: &Rc<Ctx>) {
        let (skin, ctx) = (self.clone(), ctx.clone());
        adw::StyleManager::default().connect_dark_notify(move |sm| {
            if skin.mode.get() == Mode::System && ctx.theme().light() == sm.is_dark() {
                let spec = skin.current_id();
                if let Err(e) = skin.switch(&ctx, &spec) { eprintln!("theme: {e}"); }
            }
        });
    }

    pub fn list(&self) -> Vec<theme_core::Entry> { self.loader.list() }

    pub fn current_id(&self) -> String { self.spec.borrow().clone() }

    /// Load a theme without activating it (gallery swatches).
    pub fn load(&self, spec: &str) -> Result<Theme, String> { self.load_resolved(spec) }

    /// Load `spec`; on any error report it and fall back to the built-in default (startup: never a blank window).
    pub fn load_or_default(&self, spec: &str) -> Rc<Theme> {
        match self.load_resolved(spec) {
            Ok(t) => { *self.spec.borrow_mut() = spec.into(); Rc::new(t) }
            Err(e) => {
                eprintln!("theme: {e} — using {}", theme_core::DEFAULT);
                *self.spec.borrow_mut() = theme_core::DEFAULT.into();
                Rc::new(self.load_resolved(theme_core::DEFAULT).expect("built-in default theme"))
            }
        }
    }

    /// Stylesheet text for `theme`; a template error leaves the old stylesheet in place.
    fn paint(&self, theme: &Theme) -> Result<(), String> {
        self.provider.load_from_string(&theme.stylesheet(&template(self.watch)?)?);
        Ok(())
    }

    /// Startup: paint the theme chosen by [`Skin::load_or_default`].
    pub fn apply_initial(&self, theme: &Theme) {
        if let Err(e) = self.paint(theme) { eprintln!("theme: {e}"); }
    }

    /// Switch live (Preferences, hot reload). On error nothing changes and the message is returned.
    pub fn switch(&self, ctx: &Rc<Ctx>, spec: &str) -> Result<(), String> {
        let theme = Rc::new(self.load_resolved(spec)?);
        self.paint(&theme)?;
        *self.spec.borrow_mut() = spec.into();
        ctx.set_theme(theme);
        Ok(())
    }

    /// Dev: re-apply whenever `style.css` or a `theme.toml`/`style.css` in a theme folder changes.
    pub fn watch(self: &Rc<Self>, ctx: &Rc<Ctx>) {
        let mut dirs = vec![Path::new(CSS_PATH).parent().unwrap().to_path_buf(), Path::new(SRC_DIR).join("components"), theme_core::user_theme_dir()];
        for root in [PathBuf::from(DEV_THEMES), theme_core::user_theme_dir()] {
            if let Ok(rd) = std::fs::read_dir(&root) { dirs.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir())); }
        }
        for dir in dirs {
            if !dir.is_dir() { continue; }
            let Ok(monitor) = gio::File::for_path(&dir).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) else { continue };
            let (skin, ctx) = (self.clone(), ctx.clone());
            monitor.connect_changed(move |_, file, other, event| {
                use gio::FileMonitorEvent::*;
                let touched = |f: &gio::File| f.basename().is_some_and(|n| n.to_str().is_some_and(|n| n == "theme.toml" || n.ends_with(".css")));
                if (touched(file) || other.is_some_and(touched)) && matches!(event, ChangesDoneHint | Created | Renamed | MovedIn) {
                    let spec = skin.current_id();
                    match skin.switch(&ctx, &spec) { Ok(()) => eprintln!("theme reloaded ({spec})"), Err(e) => { eprintln!("theme: {e}"); ctx.toast(&format!("Theme error: {e}")); } }
                }
            });
            eprintln!("watching {}", dir.display());
            std::mem::forget(monitor);
        }
    }
}

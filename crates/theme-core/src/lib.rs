//! GUI-free theme model: parse `theme.toml` (+ optional `style.css`), resolve `extends`, validate, and turn tokens into
//! a GTK CSS prelude (`@define-color`) plus `{{placeholder}}` substitution for the app's stylesheet template.
//! See docs/THEMING_DESIGN.md. No GTK in here.

mod color;
pub mod fonts;
pub mod layout;

pub use color::Color;
pub use layout::{Node, Page, Shell};
use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, String>;

/// Themes compiled into the binary. A user folder with the same id overrides the built-in one.
/// (id, theme.toml, style.css)
pub const BUILTIN: &[(&str, &str, &str)] = &[
    ("blur", include_str!("../themes/blur/theme.toml"), include_str!("../themes/blur/style.css")),
    ("harv", include_str!("../themes/harv/theme.toml"), include_str!("../themes/harv/style.css")),
];
pub const DEFAULT: &str = "blur";

/// Old theme ids (saved preferences, user themes that `extends` them) and what they were renamed to.
const RENAMED: &[(&str, &str)] = &[("classic", "blur"), ("mixer", "harv"), ("rawi", "harv"), ("night-console", "blur"), ("lagoon", "blur")];
fn current_id(id: &str) -> &str { RENAMED.iter().find(|(old, _)| *old == id).map_or(id, |(_, new)| new) }

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Scheme { Light, Dark }

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    #[serde(default)] pub id: String,
    pub name: String,
    #[serde(default)] pub extends: Option<String>,
    pub scheme: Scheme,
}

/// Accent source: follow the playing cover, or a fixed colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Accent { Cover, Fixed(Color) }

impl<'de> Deserialize<'de> for Accent {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s.trim().eq_ignore_ascii_case("cover") { Ok(Accent::Cover) } else { Color::parse(&s).map(Accent::Fixed).map_err(serde::de::Error::custom) }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Colors {
    /// Window / page background.
    pub bg: Color,
    pub sidebar: Color,
    /// Raised surfaces: popups, suggestion list, dialogs.
    pub raise: Color,
    pub hover: Color,
    pub fg: Color,
    pub dim: Color,
    pub accent: Accent,
    /// Text/icons drawn on top of the accent.
    pub accent_on: Color,
    pub play_bg: Color,
    pub play_fg: Color,
    /// The bottom (mini) player bar can differ from the page (Mixer's is dark on a light page).
    pub bar_bg: Color,
    pub bar_fg: Color,
    pub toast_bg: Color,
    pub toast_fg: Color,
    /// The full-screen now-playing view always sits on a dark cover backdrop: its own foreground pair.
    pub np_fg: Color,
    pub np_on: Color,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Fonts {
    pub ui: String,
    pub mono: String,
    pub fallbacks: Vec<String>,
}

/// Corner radii in px. `control` is the radius of chips / pill buttons / toasts (999 = pill).
#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Shape { pub xs: u32, pub sm: u32, pub md: u32, pub lg: u32, pub input: u32, pub xl: u32, pub control: u32, pub cover: u32, pub hero: u32 }

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Metrics { pub sidebar_w: i32, pub track_row_h: i32, pub label_tracking: f32 }

/// 0..1 multipliers for the ambient decoration that depends on the cover accent.
#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Effects { pub ambient: f64, pub bar_tint: f64 }

/// `[lyrics]`: the full-screen lyrics page (`shell.now_playing.host = "fullscreen"`). Every key is optional; the defaults are
/// Blur's look. Text, progress bar and controls are drawn in the theme's `np_fg` colour (`np_on` on the play button), so a
/// theme that changes `backdrop` must also give `np_fg` / `np_on` (in both palettes) that contrast with it.
/// See docs/THEMING_DESIGN.md §"Full-screen lyrics page".
#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields, default)]
pub struct LyricsTheme {
    /// Font family for lines and titles; `"ui"` = the theme's UI font.
    pub font: String,
    /// Active / inactive line size and the gap between lines, px; absent = scaled from the window height.
    pub active_size: Option<f32>,
    pub size: Option<f32>,
    pub line_gap: Option<f32>,
    /// `start` | `center`.
    pub align: String,
    /// Multipliers: blur of far lines, glow of the active one, spring stiffness; seconds each line waits in the cascade.
    pub blur: f32,
    pub glow: f32,
    pub spring: f32,
    pub cascade: f32,
    /// `cover` (animated artwork), `gradient` (accent blobs on the theme background), `solid` (flat + one glow), `none` (the page background).
    pub backdrop: String,
    /// `split` (cover left, lyrics right) | `centered` (small cover in a header row, lyrics centred) | `lyrics-only`.
    pub layout: String,
    pub cover_radius: Option<f32>,
    pub cover_shadow: bool,
    /// Transport row and header buttons: `plain` | `capsule` (a translucent pill behind the controls).
    pub chrome: String,
}

impl Default for LyricsTheme {
    fn default() -> Self {
        Self { font: "ui".into(), active_size: None, size: None, line_gap: None, align: "start".into(), blur: 1.0, glow: 1.0, spring: 1.0, cascade: 0.05, backdrop: "cover".into(), layout: "split".into(), cover_radius: None, cover_shadow: true, chrome: "plain".into() }
    }
}

impl LyricsTheme {
    fn validate(&self) -> Vec<String> {
        let mut out = vec![];
        for (what, v, ok) in [("lyrics.align", &self.align, &["start", "center"][..]), ("lyrics.backdrop", &self.backdrop, &["cover", "gradient", "solid", "none"]), ("lyrics.layout", &self.layout, &["split", "centered", "lyrics-only"]), ("lyrics.chrome", &self.chrome, &["plain", "capsule"])] {
            if !ok.contains(&v.as_str()) { out.push(format!("{what} = `{v}` (expected one of: {})", ok.join(", "))); }
        }
        out
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    pub meta: Meta,
    pub colors: Colors,
    /// The palette for the opposite scheme (`[colors_alt]`, overlaid on `[colors]`); see [`Theme::resolved`].
    #[serde(default)]
    pub colors_alt: Option<Colors>,
    pub fonts: Fonts,
    pub shape: Shape,
    pub metrics: Metrics,
    pub effects: Effects,
    /// Vocabulary: every user-visible noun a theme may rename (channels / orbits / tapes…). See [`Theme::word`].
    pub words: BTreeMap<String, String>,
    /// Window shell (sidebar / bottom bar / now-playing host). Defaults = the Blur built-in shell.
    #[serde(default)]
    pub shell: Shell,
    /// The full-screen lyrics page (see [`LyricsTheme`]).
    #[serde(default)]
    pub lyrics: LyricsTheme,
    /// Per page-kind layouts (`[page.playlist]`, `[page.library]`, `[page.search]`, `[page.now]`); absent = built-in composition.
    #[serde(default)]
    pub page: BTreeMap<String, Page>,
    /// Concatenated `style.css` of the theme chain (parents first); appended after the app stylesheet.
    #[serde(skip)] pub css: String,
    /// Folder of the theme (None for built-ins) — what the dev watcher monitors.
    #[serde(skip)] pub dir: Option<PathBuf>,
}

impl Theme {
    /// Vocabulary lookup; an unknown key shows as itself so a missing entry is visible but harmless.
    pub fn word<'a>(&'a self, key: &'a str) -> &'a str { self.words.get(key).map(String::as_str).unwrap_or(key) }

    pub fn light(&self) -> bool { self.meta.scheme == Scheme::Light }

    /// The theme in the requested scheme: `[colors]` is its native palette, `[colors_alt]` the other one. A theme
    /// without an alternate palette stays as it is.
    pub fn resolved(mut self, light: bool) -> Theme {
        if light != self.light() {
            if let Some(alt) = self.colors_alt.take() {
                self.colors = alt;
                self.meta.scheme = if light { Scheme::Light } else { Scheme::Dark };
            }
        }
        self.colors_alt = None;
        self
    }

    fn font_stack(&self, first: &str) -> String {
        let mut v = vec![format!("\"{first}\"")];
        v.extend(self.fonts.fallbacks.iter().map(|f| format!("\"{f}\"")));
        v.push("sans-serif".into());
        v.join(", ")
    }

    /// `@define-color` lines for every colour token. Loaded first; `lib_accent` is only the default until the app
    /// loads its accent provider (cover colour or fixed accent).
    pub fn prelude(&self) -> String {
        let c = &self.colors;
        let mut s = String::new();
        for (name, col) in [("bg", c.bg), ("bg_side", c.sidebar), ("bg_raise", c.raise), ("bg_hover", c.hover), ("fg", c.fg), ("fg_dim", c.dim), ("accent_on", c.accent_on), ("play_bg", c.play_bg), ("play_fg", c.play_fg), ("bar_bg", c.bar_bg), ("bar_fg", c.bar_fg), ("toast_bg", c.toast_bg), ("toast_fg", c.toast_fg), ("np_fg", c.np_fg), ("np_on", c.np_on)] {
            s += &format!("@define-color {name} {};\n", col.css());
        }
        let accent = match c.accent { Accent::Fixed(a) => a.css(), Accent::Cover => "#ff375f".into() };
        s + &format!("@define-color lib_accent {accent};\n")
    }

    /// Numeric/text tokens usable as `{{name}}` in the stylesheet template.
    fn tokens(&self) -> BTreeMap<&'static str, String> {
        let (s, m, e) = (&self.shape, &self.metrics, &self.effects);
        let mut t = BTreeMap::new();
        t.insert("font_ui", self.font_stack(&self.fonts.ui));
        t.insert("font_mono", self.font_stack(&self.fonts.mono));
        for (k, v) in [("r_xs", s.xs), ("r_sm", s.sm), ("r_md", s.md), ("r_lg", s.lg), ("r_input", s.input), ("r_xl", s.xl), ("r_control", s.control), ("r_cover", s.cover), ("r_hero", s.hero)] { t.insert(k, v.to_string()); }
        t.insert("track_row_h", m.track_row_h.to_string());
        t.insert("label_tracking", m.label_tracking.to_string());
        t.insert("wash1", (0.22 * e.ambient).to_string());
        t.insert("wash2", (0.06 * e.ambient).to_string());
        t.insert("bar_tint1", (0.18 * e.bar_tint).to_string());
        t.insert("bar_tint2", (0.04 * e.bar_tint).to_string());
        t
    }

    /// Substitute `{{token}}` in `template`; an unknown token is an error naming its line.
    pub fn render(&self, template: &str) -> Result<String> {
        let tokens = self.tokens();
        let mut out = String::with_capacity(template.len() + 256);
        let mut rest = template;
        let mut line = 1;
        while let Some(i) = rest.find("{{") {
            let (head, tail) = rest.split_at(i);
            line += head.matches('\n').count();
            out.push_str(head);
            let end = tail.find("}}").ok_or_else(|| format!("style template line {line}: unterminated `{{{{`"))?;
            let key = tail[2..end].trim();
            out.push_str(tokens.get(key).ok_or_else(|| format!("style template line {line}: unknown token `{key}`"))?);
            rest = &tail[end + 2..];
        }
        out.push_str(rest);
        Ok(out)
    }

    /// The whole stylesheet: prelude, rendered template, then the theme's own CSS.
    pub fn stylesheet(&self, template: &str) -> Result<String> { Ok(format!("{}{}\n{}", self.prelude(), self.render(template)?, self.css)) }
}

#[derive(Clone, Debug)]
pub struct Entry { pub id: String, pub name: String, pub builtin: bool }

/// Finds themes by id: user directories first (first wins), then the built-ins.
pub struct Loader { dirs: Vec<PathBuf> }

impl Loader {
    pub fn new(dirs: Vec<PathBuf>) -> Self { Self { dirs } }

    /// `spec` is a theme id, or a path to a theme folder / `theme.toml`.
    pub fn load(&self, spec: &str) -> Result<Theme> {
        let path = Path::new(spec);
        let (id, dir) = if spec.contains('/') || path.exists() {
            let dir = if path.is_dir() { path.to_path_buf() } else { path.parent().unwrap_or(Path::new(".")).to_path_buf() };
            (dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| spec.into()), Some(dir))
        } else { (current_id(spec).to_string(), None) };
        let mut chain = vec![];
        let (table, css, dir) = self.resolve(&id, dir, &mut chain)?;
        let mut table = table;
        if let Some(toml::Value::Table(alt)) = table.remove("colors_alt") {
            let mut full = match table.get("colors") { Some(toml::Value::Table(c)) => c.clone(), _ => toml::Table::new() };
            merge(&mut full, alt);
            table.insert("colors_alt".into(), toml::Value::Table(full));
        }
        let mut theme: Theme = table.try_into().map_err(|e| format!("theme `{id}`: {e}"))?;
        let mut problems = layout::validate(&theme.shell, &theme.page);
        problems.extend(theme.lyrics.validate());
        if !problems.is_empty() { return Err(format!("theme `{id}`: {}", problems.join("; "))); }
        theme.meta.id = id;
        theme.css = css;
        theme.dir = dir;
        Ok(theme)
    }

    pub fn list(&self) -> Vec<Entry> {
        let mut out: Vec<Entry> = vec![];
        for dir in &self.dirs {
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            for e in rd.flatten() {
                let id = e.file_name().to_string_lossy().into_owned();
                if out.iter().any(|x| x.id == id) { continue; }
                if let Ok(t) = self.load(&id) { out.push(Entry { id, name: t.meta.name, builtin: false }); }
            }
        }
        for (id, _, _) in BUILTIN {
            if out.iter().any(|x| x.id == *id) { continue; }
            if let Ok(t) = self.load(id) { out.push(Entry { id: (*id).into(), name: t.meta.name, builtin: true }); }
        }
        // Built-ins first, Harv at the very top, then by name.
        out.sort_by(|a, b| b.builtin.cmp(&a.builtin).then((a.id != "harv").cmp(&(b.id != "harv"))).then(a.name.cmp(&b.name)));
        out
    }

    /// Folder of a theme id in the user dirs, if any.
    fn find_dir(&self, id: &str) -> Option<PathBuf> { self.dirs.iter().map(|d| d.join(id)).find(|d| d.join("theme.toml").is_file()) }

    /// Read `id` (from `dir`, a user dir, or the built-ins), merge it over its `extends` parent, and gather CSS.
    fn resolve(&self, id: &str, dir: Option<PathBuf>, chain: &mut Vec<String>) -> Result<(toml::Table, String, Option<PathBuf>)> {
        if chain.iter().any(|c| c == id) || chain.len() > 8 { return Err(format!("theme `{id}`: `extends` cycle ({})", chain.join(" → "))); }
        let id = current_id(id);
        chain.push(id.to_string());
        let dir = dir.or_else(|| self.find_dir(id));
        let (text, css) = match &dir {
            Some(d) => (std::fs::read_to_string(d.join("theme.toml")).map_err(|e| format!("theme `{id}`: cannot read {}: {e}", d.join("theme.toml").display()))?, std::fs::read_to_string(d.join("style.css")).unwrap_or_default()),
            None => BUILTIN.iter().find(|(b, _, _)| *b == id).map(|(_, t, c)| (t.to_string(), c.to_string())).ok_or_else(|| format!("unknown theme `{id}`"))?,
        };
        let table: toml::Table = text.parse().map_err(|e| format!("theme `{id}`: {e}"))?;
        let parent = table.get("meta").and_then(|m| m.get("extends")).and_then(|v| v.as_str()).map(str::to_string);
        match parent {
            Some(p) => {
                let (mut base, base_css, base_dir) = self.resolve(&p, None, chain)?;
                merge(&mut base, table);
                Ok((base, format!("{base_css}\n{css}"), dir.or(base_dir)))
            }
            None => Ok((table, css, dir)),
        }
    }
}

/// Deep merge: tables merge key by key; any other value (including arrays) is replaced by the child's.
fn merge(base: &mut toml::Table, child: toml::Table) {
    for (k, v) in child {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(c)) => merge(b, c),
            (_, v) => { base.insert(k, v); }
        }
    }
}

/// Directories searched for user themes: `$XDG_CONFIG_HOME/harv/themes` (default `~/.config/harv/themes`).
pub fn user_theme_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))).unwrap_or_else(|| PathBuf::from(".")).join("harv/themes")
}

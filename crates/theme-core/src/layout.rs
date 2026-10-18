//! Layout schema: the window shell (sidebar / right panel / bottom bar / now-playing host) and per-page component trees.
//! A theme with no `[shell]` / `[page.*]` entries gets the app's built-in (classic) layouts — see docs/THEMING_DESIGN.md §5.
//! The component *catalog* lives in the app; this crate only parses and validates the structure.

use serde::Deserialize;
use std::collections::BTreeMap;

/// One component in a layout tree: `{ c = "track-list", style = "table", ... }` plus optional `children` for containers.
/// Every key other than `c` / `children` is a prop; typed accessors below.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct Node {
    pub c: String,
    #[serde(default)]
    pub children: Vec<Node>,
    #[serde(flatten)]
    pub props: BTreeMap<String, toml::Value>,
}

impl Node {
    pub fn str(&self, key: &str) -> Option<&str> { self.props.get(key)?.as_str() }
    pub fn str_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str { self.str(key).unwrap_or(default) }
    pub fn int(&self, key: &str) -> Option<i64> { self.props.get(key)?.as_integer() }
    pub fn int_or(&self, key: &str, default: i64) -> i64 { self.int(key).unwrap_or(default) }
    /// Floats may be written as integers in TOML (`gap = 8`).
    pub fn float(&self, key: &str) -> Option<f64> { let v = self.props.get(key)?; v.as_float().or_else(|| v.as_integer().map(|i| i as f64)) }
    pub fn float_or(&self, key: &str, default: f64) -> f64 { self.float(key).unwrap_or(default) }
    pub fn bool(&self, key: &str) -> Option<bool> { self.props.get(key)?.as_bool() }
    pub fn bool_or(&self, key: &str, default: bool) -> bool { self.bool(key).unwrap_or(default) }
    pub fn strs(&self, key: &str) -> Vec<String> { self.props.get(key).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields, default)]
pub struct Sidebar {
    /// `classic` = the built-in sidebar (ignores everything else here); `custom` = built from `items`.
    pub style: String,
    /// Ordered entries: a root id (`home search charts favorites recent most_played playlists following downloads now`),
    /// `section:<words key>` for a small section label, `playlists` entries can also be expanded with `[sidebar.playlists]`.
    pub items: Vec<String>,
    /// `rounded` | `pill` — shape of the selected row.
    pub item_shape: String,
    /// Show the app name at the top.
    pub brand: bool,
    /// A search field at the top that opens the Search page (Mixer).
    pub search_box: bool,
    /// List the user's local playlists under the items, with covers (`none` | `square` | `round`).
    pub playlist_art: String,
    /// Title key (vocabulary) of the playlists section, e.g. `section_playlists`.
    pub playlists_title: String,
    /// Show a "+ New …" row under the playlists list.
    pub new_playlist: bool,
    /// Show "Preferences" at the bottom.
    pub prefs: bool,
}

impl Default for Sidebar {
    fn default() -> Self { Self { style: "classic".into(), items: vec![], item_shape: "rounded".into(), brand: true, search_box: false, playlist_art: "none".into(), playlists_title: "section_playlists".into(), new_playlist: false, prefs: true } }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields, default)]
pub struct BottomBar {
    /// `classic` = the built-in mini bar (ignores `parts`); `parts` = built from `parts`.
    pub style: String,
    /// `always` | `except-now` (hidden while the in-shell now-playing page is showing) | `never`.
    pub show: String,
    /// Height in px.
    pub height: i32,
    pub parts: Vec<Node>,
}

impl Default for BottomBar {
    fn default() -> Self { Self { style: "classic".into(), show: "always".into(), height: 80, parts: vec![] } }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields, default)]
pub struct NowPlaying {
    /// `fullscreen` = the built-in full-screen lyrics page; `page` = the `[page.now]` tree inside the window.
    pub host: String,
}

impl Default for NowPlaying { fn default() -> Self { Self { host: "fullscreen".into() } } }

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Shell {
    pub sidebar: Sidebar,
    pub bottom_bar: BottomBar,
    pub now_playing: NowPlaying,
}

/// What a page shows. Missing `main` = the app's built-in composition for that page.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Page {
    pub main: Vec<Node>,
    /// Optional right panel next to this kind of page (hidden when empty).
    pub right: Vec<Node>,
    pub right_w: i32,
}

/// Page kinds themes may lay out.
pub const PAGE_KINDS: &[&str] = &["playlist", "library", "search", "now"];

/// Collect problems in a layout: unknown page kinds, empty component names. (Unknown *components* are checked by the app,
/// which owns the catalog.)
pub fn validate(shell: &Shell, pages: &BTreeMap<String, Page>) -> Vec<String> {
    let mut out = vec![];
    for k in pages.keys() { if !PAGE_KINDS.contains(&k.as_str()) { out.push(format!("page.{k}: unknown page kind (known: {})", PAGE_KINDS.join(", "))); } }
    fn walk(n: &Node, path: &str, out: &mut Vec<String>) {
        if n.c.trim().is_empty() { out.push(format!("{path}: component without a name (`c = \"…\"`)")); }
        for (i, c) in n.children.iter().enumerate() { walk(c, &format!("{path}.children[{i}]"), out); }
    }
    for (k, p) in pages {
        for (i, n) in p.main.iter().enumerate() { walk(n, &format!("page.{k}.main[{i}]"), &mut out); }
        for (i, n) in p.right.iter().enumerate() { walk(n, &format!("page.{k}.right[{i}]"), &mut out); }
    }
    for (i, n) in shell.bottom_bar.parts.iter().enumerate() { walk(n, &format!("shell.bottom_bar.parts[{i}]"), &mut out); }
    for (what, v, ok) in [("shell.sidebar.style", &shell.sidebar.style, &["classic", "custom"][..]), ("shell.sidebar.item_shape", &shell.sidebar.item_shape, &["rounded", "pill"]), ("shell.sidebar.playlist_art", &shell.sidebar.playlist_art, &["none", "square", "round"]), ("shell.bottom_bar.style", &shell.bottom_bar.style, &["classic", "parts"]), ("shell.bottom_bar.show", &shell.bottom_bar.show, &["always", "except-now", "never"]), ("shell.now_playing.host", &shell.now_playing.host, &["fullscreen", "page"])] {
        if !ok.contains(&v.as_str()) { out.push(format!("{what}: `{v}` is not one of {}", ok.join(" | "))); }
    }
    out
}

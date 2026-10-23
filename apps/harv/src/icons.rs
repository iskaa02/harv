//! Bundled symbolic icons (the Adwaita set has no heart). Written to `$XDG_CACHE_HOME/harv/icons/hicolor/scalable/actions/`
//! and added to the icon theme's search path. Never fails the app: problems go to stderr.

use std::path::PathBuf;

const ICONS: [(&str, &str); 3] = [
    ("heart-filled-symbolic.svg", include_str!("../icons/heart-filled-symbolic.svg")),
    ("heart-outline-symbolic.svg", include_str!("../icons/heart-outline-symbolic.svg")),
    ("station-radio-symbolic.svg", include_str!("../icons/station-radio-symbolic.svg")),
];

/// Window-control glyphs. Some icon themes (ePapirus) wrap them in `<g>`, which GTK 4.22's symbolic loader skips, leaving blank circles.
/// They live in a tiny theme that inherits the user's own, so only these four icons change.
const WINDOW_ICONS: [(&str, &str); 4] = [
    ("window-close-symbolic.svg", include_str!("../icons/window-close-symbolic.svg")),
    ("window-minimize-symbolic.svg", include_str!("../icons/window-minimize-symbolic.svg")),
    ("window-maximize-symbolic.svg", include_str!("../icons/window-maximize-symbolic.svg")),
    ("window-restore-symbolic.svg", include_str!("../icons/window-restore-symbolic.svg")),
];
const OVERLAY_THEME: &str = "harv-icons";

pub const HEART_ON: &str = "heart-filled-symbolic";
pub const HEART_OFF: &str = "heart-outline-symbolic";
/// "Radio on click" toggle.
pub const RADIO: &str = "station-radio-symbolic";

/// The icon for a liked / not-liked song.
pub fn heart(liked: bool) -> &'static str { if liked { HEART_ON } else { HEART_OFF } }

pub fn register() {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))).unwrap_or_else(|| PathBuf::from("/tmp"));
    let root = base.join("harv").join("icons");
    let dir = root.join("hicolor/scalable/actions");
    if let Err(e) = std::fs::create_dir_all(&dir) { eprintln!("icons: {e}"); return; }
    for (name, svg) in ICONS {
        let p = dir.join(name);
        if std::fs::read_to_string(&p).ok().as_deref() != Some(svg) { if let Err(e) = std::fs::write(&p, svg) { eprintln!("icons: {e}"); } }
    }
    if let Some(d) = gtk::gdk::Display::default() { gtk::IconTheme::for_display(&d).add_search_path(&root); }
    overlay_window_icons(&root);
}

/// Points GTK at `harv-icons`, a theme holding only the window-control icons and inheriting the user's theme for everything else.
fn overlay_window_icons(root: &std::path::Path) {
    let Some(settings) = gtk::Settings::default() else { return };
    let base = settings.gtk_icon_theme_name().map(|s| s.to_string()).filter(|n| n != OVERLAY_THEME).unwrap_or_else(|| "Adwaita".into());
    let dir = root.join(OVERLAY_THEME);
    let icons = dir.join("scalable/actions");
    if let Err(e) = std::fs::create_dir_all(&icons) { eprintln!("icons: {e}"); return; }
    let index = format!("[Icon Theme]\nName={OVERLAY_THEME}\nComment=harv window-control icons\nInherits={base},hicolor\nDirectories=scalable/actions\n\n[scalable/actions]\nSize=16\nMinSize=8\nMaxSize=512\nType=Scalable\nContext=Actions\n");
    let mut files: Vec<(PathBuf, &str)> = WINDOW_ICONS.iter().map(|(n, svg)| (icons.join(n), *svg)).collect();
    files.push((dir.join("index.theme"), &index));
    for (p, body) in files {
        if std::fs::read_to_string(&p).ok().as_deref() != Some(body) { if let Err(e) = std::fs::write(&p, body) { eprintln!("icons: {e}"); return; } }
    }
    settings.set_gtk_icon_theme_name(Some(OVERLAY_THEME));
}

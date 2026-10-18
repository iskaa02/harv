//! Bundled theme fonts. The built-in themes name Figtree, Lexend, Outfit and Red Hat, which many systems lack, so the
//! files are compiled in (`theme_core::fonts`). At startup, each family that fontconfig can't already match is written
//! to `$XDG_CACHE_HOME/harv/fonts/` (skipped when a file of the same size is there) and registered with fontconfig
//! for this process; then Pango's font map is told to rescan. A system copy under the same family name always wins.
//! Never fails the app: problems go to stderr.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use gtk::glib::translate::FromGlibPtrNone;
use gtk::pango::prelude::{FontExt, FontFamilyExt, FontMapExt};

enum FcConfig {}
enum FcPattern {}

const FC_FAMILY: &[u8] = b"family\0";
const FC_MATCH_PATTERN: c_int = 0;
const FC_RESULT_MATCH: c_int = 0;

#[link(name = "fontconfig")]
unsafe extern "C" {
    fn FcConfigGetCurrent() -> *mut FcConfig;
    fn FcConfigAppFontAddFile(config: *mut FcConfig, file: *const u8) -> c_int;
    fn FcPatternCreate() -> *mut FcPattern;
    fn FcPatternDestroy(p: *mut FcPattern);
    fn FcPatternAddString(p: *mut FcPattern, object: *const c_char, s: *const u8) -> c_int;
    fn FcPatternGetString(p: *const FcPattern, object: *const c_char, n: c_int, s: *mut *mut u8) -> c_int;
    fn FcConfigSubstitute(config: *mut FcConfig, p: *mut FcPattern, kind: c_int) -> c_int;
    fn FcDefaultSubstitute(p: *mut FcPattern);
    fn FcFontMatch(config: *mut FcConfig, p: *mut FcPattern, result: *mut c_int) -> *mut FcPattern;
}

#[link(name = "pangocairo-1.0")]
unsafe extern "C" {
    fn pango_cairo_font_map_get_default() -> *mut c_void;
}

#[link(name = "pangoft2-1.0")] // pango_fc_font_map_* live in libpangoft2 (Pango 1.57)
unsafe extern "C" {
    fn pango_fc_font_map_config_changed(fcmap: *mut c_void);
}

/// Does fontconfig resolve `family` to a font of that same family (not a fallback such as Noto Sans)?
fn system_knows(config: *mut FcConfig, family: &str) -> bool {
    let Ok(name) = CString::new(family) else { return false };
    unsafe {
        let pat = FcPatternCreate();
        if pat.is_null() {
            return false;
        }
        FcPatternAddString(pat, FC_FAMILY.as_ptr() as *const c_char, name.as_ptr() as *const u8);
        FcConfigSubstitute(config, pat, FC_MATCH_PATTERN);
        FcDefaultSubstitute(pat);
        let mut result: c_int = 0;
        let m = FcFontMatch(config, pat, &mut result);
        let mut known = false;
        if !m.is_null() {
            let mut s: *mut u8 = null_mut();
            if result == FC_RESULT_MATCH && FcPatternGetString(m, FC_FAMILY.as_ptr() as *const c_char, 0, &mut s) == FC_RESULT_MATCH && !s.is_null() {
                known = CStr::from_ptr(s as *const c_char).to_string_lossy().eq_ignore_ascii_case(family);
            }
            FcPatternDestroy(m);
        }
        FcPatternDestroy(pat);
        known
    }
}

/// `$XDG_CACHE_HOME/harv/fonts`, or `~/.cache/harv/fonts`.
fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("harv").join("fonts"))
}

/// Writes `bytes` to `dir/<Family>.ttf` unless a file of that size is already there.
fn ensure_file(dir: &Path, family: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let path = dir.join(format!("{}.ttf", family.replace(' ', "")));
    if std::fs::metadata(&path).is_ok_and(|m| m.len() == bytes.len() as u64) {
        return Ok(path);
    }
    let tmp = dir.join(format!(".{}.{}.tmp", family.replace(' ', ""), std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Call once, before the first window is built (GTK's Pango font map is created lazily and is rescanned here anyway).
pub fn register() {
    if let Err(e) = try_register() {
        eprintln!("fonts: bundled fonts unavailable: {e}");
    }
}

fn try_register() -> Result<(), String> {
    let debug = std::env::var_os("HARV_DEBUG_FONTS").is_some_and(|v| v == "1");
    let config = unsafe { FcConfigGetCurrent() };
    if config.is_null() {
        return Err("no fontconfig configuration".into());
    }
    let missing: Vec<_> = theme_core::fonts::BUNDLED.iter().filter(|(family, _)| !system_knows(config, family)).collect();
    let mut registered = 0;
    if !missing.is_empty() {
        let dir = cache_dir().ok_or("no cache directory")?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for (family, bytes) in missing {
            let step = ensure_file(&dir, family, bytes).map_err(|e| format!("{family}: {e}")).and_then(|path| {
                let c = CString::new(path.as_os_str().as_bytes()).map_err(|_| format!("{}: NUL in path", path.display()))?;
                if unsafe { FcConfigAppFontAddFile(config, c.as_ptr() as *const u8) } == 0 {
                    return Err(format!("fontconfig refused {}", path.display()));
                }
                Ok(())
            });
            match step {
                Ok(()) => registered += 1,
                Err(e) => eprintln!("fonts: {e}"),
            }
        }
    }
    if registered > 0 {
        // Pango caches its family list per font map; invalidate it so the new families are found.
        let map = unsafe { pango_cairo_font_map_get_default() };
        if !map.is_null() {
            unsafe { pango_fc_font_map_config_changed(map) };
        }
    }
    if debug {
        let map = unsafe { pango_cairo_font_map_get_default() };
        let fm = unsafe { gtk::pango::FontMap::from_glib_none(map as *mut gtk::pango::ffi::PangoFontMap) };
        let names: Vec<String> = fm.list_families().iter().map(|f| f.name().to_string()).collect();
        eprintln!("fonts: registered {registered} file(s); {} families visible to Pango", names.len());
        for (family, _) in theme_core::fonts::BUNDLED {
            // What Pango actually resolves for this name (a fallback shows up here as a different family).
            let ctx = fm.create_context();
            let resolved = ctx.load_font(&gtk::pango::FontDescription::from_string(&format!("{family} 14"))).map(|f| f.describe().family().map(|s| s.to_string()).unwrap_or_default());
            eprintln!("fonts: {family:<16} pango-lists={} resolves-to={:?}", names.iter().any(|n| n.eq_ignore_ascii_case(family)), resolved);
        }
    }
    Ok(())
}

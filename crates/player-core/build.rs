//! libmpv2-sys emits `-lmpv` without a search path. On Linux libmpv sits in a default linker dir; on macOS Homebrew installs it under
//! its own prefix (/opt/homebrew on Apple Silicon, /usr/local on Intel), so point the linker there. `pkg-config` wins when it knows mpv.

fn main() {
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") { return; }
    let from_pkg_config = std::process::Command::new("pkg-config").args(["--libs-only-L", "mpv"]).output().ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().filter_map(|f| f.strip_prefix("-L").map(str::to_owned)).collect::<Vec<_>>())
        .unwrap_or_default();
    let dirs = if from_pkg_config.is_empty() { vec!["/opt/homebrew/lib".to_owned(), "/usr/local/lib".to_owned()] } else { from_pkg_config };
    for d in dirs.iter().filter(|d| std::path::Path::new(d).join("libmpv.dylib").exists()) { println!("cargo:rustc-link-search=native={d}"); }
}

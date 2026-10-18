#!/usr/bin/env bash
# Build harv and install it for the current user:
#   binary   -> ~/.local/bin/harv
#   icon     -> ~/.local/share/icons/hicolor/{scalable,48x48,…}/apps/dev.harv.Harv.*
#   launcher -> ~/.local/share/applications/dev.harv.Harv.desktop
#
#   ./install.sh               build (release) + install
#   ./install.sh --no-build    install the already built target/release/harv
#   ./install.sh --uninstall   remove everything this script installed (your library/settings in ~/.local/share/harv stay)
set -euo pipefail

APP_ID=dev.harv.Harv
ROOT="$(cd "$(dirname "$0")" && pwd)"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
ICON_ROOT="$DATA_DIR/icons/hicolor"
APPS_DIR="$DATA_DIR/applications"
DESKTOP="$APPS_DIR/$APP_ID.desktop"
SIZES=(32 48 64 128 256 512)

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

refresh_caches() {
  command -v gtk4-update-icon-cache >/dev/null && gtk4-update-icon-cache -q -f -t "$ICON_ROOT" 2>/dev/null || true
  command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -f -t "$ICON_ROOT" 2>/dev/null || true
  command -v update-desktop-database >/dev/null && update-desktop-database -q "$APPS_DIR" 2>/dev/null || true
}

if [[ "${1:-}" == "--uninstall" ]]; then
  say "Removing harv"
  rm -f "$BIN_DIR/harv" "$DESKTOP" "$ICON_ROOT/scalable/apps/$APP_ID.svg"
  for s in "${SIZES[@]}"; do rm -f "$ICON_ROOT/${s}x${s}/apps/$APP_ID.png"; done
  refresh_caches
  say "Done. Your library and settings are untouched (~/.local/share/harv)."
  exit 0
fi

BUILD=1
[[ "${1:-}" == "--no-build" ]] && BUILD=0

if (( BUILD )); then
  # rustup's cargo must come first: /usr/local/sbin/cargo (1.84) shadows it and fails on edition 2024 crates.
  export PATH="$HOME/.cargo/bin:$PATH"
  command -v cargo >/dev/null || die "cargo not found — install Rust with rustup (https://rustup.rs)"
  command -v pkg-config >/dev/null || die "pkg-config not found (Fedora: sudo dnf install pkgconf-pkg-config)"
  missing=()
  for lib in gtk4 libadwaita-1 mpv; do pkg-config --exists "$lib" || missing+=("$lib"); done
  if (( ${#missing[@]} )); then
    die "missing development files for: ${missing[*]}
       Fedora: sudo dnf install gtk4-devel libadwaita-devel mpv-devel
       Debian/Ubuntu: sudo apt install libgtk-4-dev libadwaita-1-dev libmpv-dev"
  fi
  say "Building (release) — the first build takes a few minutes"
  (cd "$ROOT" && cargo build --release -p harv)
fi

SRC="$ROOT/target/release/harv"
[[ -x "$SRC" ]] || die "$SRC not found — run without --no-build"

# Clean up the pre-rename (musssic, rawi) installs, if any.
for pair in musssic:dev.musssic.Musssic rawi:dev.rawi.Rawi; do
  OLD_BIN=${pair%%:*}; OLD_ID=${pair##*:}
  rm -f "$BIN_DIR/$OLD_BIN" "$APPS_DIR/$OLD_ID.desktop" "$ICON_ROOT/scalable/apps/$OLD_ID.svg"
  for s in "${SIZES[@]}"; do rm -f "$ICON_ROOT/${s}x${s}/apps/$OLD_ID.png"; done
done

say "Installing the binary to $BIN_DIR/harv"
install -Dm755 "$SRC" "$BIN_DIR/harv"

say "Installing the icon"
install -Dm644 "$ROOT/packaging/$APP_ID.svg" "$ICON_ROOT/scalable/apps/$APP_ID.svg"
# Render PNGs with whichever converter actually runs (a broken `magick` shim is common, so test each one).
render=""
if command -v rsvg-convert >/dev/null && rsvg-convert --version >/dev/null 2>&1; then render=rsvg
elif command -v inkscape >/dev/null && inkscape --version >/dev/null 2>&1; then render=inkscape
elif command -v magick >/dev/null && magick -version >/dev/null 2>&1; then render=magick
elif command -v convert >/dev/null && convert -version >/dev/null 2>&1; then render=convert
fi
if [[ -n "$render" ]]; then
  for s in "${SIZES[@]}"; do
    out="$ICON_ROOT/${s}x${s}/apps/$APP_ID.png"; mkdir -p "$(dirname "$out")"
    case "$render" in
      rsvg) rsvg-convert -w "$s" -h "$s" "$ROOT/packaging/$APP_ID.svg" -o "$out" ;;
      inkscape) inkscape -w "$s" -h "$s" "$ROOT/packaging/$APP_ID.svg" -o "$out" >/dev/null 2>&1 ;;
      magick) magick -background none -density 768 "$ROOT/packaging/$APP_ID.svg" -resize "${s}x${s}" "$out" ;;
      convert) convert -background none -density 768 "$ROOT/packaging/$APP_ID.svg" -resize "${s}x${s}" "$out" 2>/dev/null ;;
    esac || warn "could not render the ${s}px icon (the scalable SVG is still installed)"
  done
else
  warn "no working SVG converter (rsvg-convert, inkscape, ImageMagick): installed only the scalable SVG icon"
fi

say "Writing the launcher $DESKTOP"
mkdir -p "$APPS_DIR"
cat > "$DESKTOP" <<DESKTOP_ENTRY
[Desktop Entry]
Type=Application
Version=1.0
Name=Harv
GenericName=Music Player
Comment=GTK4 YouTube Music player with synced lyrics
Exec=$BIN_DIR/harv
Icon=$APP_ID
Terminal=false
Categories=AudioVideo;Audio;Player;Music;GTK;
Keywords=music;lyrics;youtube;player;
StartupNotify=true
StartupWMClass=$APP_ID
DESKTOP_ENTRY
command -v desktop-file-validate >/dev/null && { desktop-file-validate "$DESKTOP" || warn "desktop-file-validate reported problems"; }
refresh_caches

# Runtime needs (not build-time): mpv's library, yt-dlp, and the Inter font.
command -v yt-dlp >/dev/null || warn "yt-dlp not found — playback needs it (Fedora: sudo dnf install yt-dlp)"
fonts="$(fc-list : family 2>/dev/null || true)"
grep -qi "^inter" <<<"$fonts" || warn "the Inter font isn't installed; the UI falls back to a default font"
case ":$PATH:" in *":$BIN_DIR:"*) ;; *) warn "$BIN_DIR is not on your PATH (the launcher entry still works)";; esac

say "Installed. Launch it from your app menu, or run: harv"

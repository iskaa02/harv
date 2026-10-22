<p align="center"><img src="packaging/dev.harv.Harv.svg" width="112" alt="Harv icon"></p>

# Harv

_Every word, lit._

Harv is a Linux-native music player for YouTube Music, written in Rust with GTK4 and libadwaita. It has an Apple-Music-style lyrics screen (blurred cover backdrop, glow, word-synced fill) and a clean library UI.

## Features

Search (suggestions, history, moods and genres), home feed, charts, album / artist / playlist / podcast pages, favorites, local playlists, history, most played, queue with endless radio, word-synced lyrics with a per-song offset, equalizer (+ AutoEq headphone profiles), speed / pitch / reverb / delay, crossfade, sleep timer, SponsorBlock, offline downloads, MPRIS media keys, mini-player, session restore, and themes (classic, mixer, night-console).

## Disclaimer

Harv is an **unofficial** client. It is not affiliated with, endorsed by or sponsored by YouTube, Google. It ships no music, video, artwork or lyrics; it fetches metadata and streams on your device through `ytmapi-rs` and `yt-dlp`, and lyrics from BetterLyrics and LRCLIB. Accessing YouTube Music this way may go against YouTube's Terms of Service; you are responsible for complying with the terms of the services you use. If you use a cookies file, prefer a throwaway account.

## Install

Build needs `gtk4-devel libadwaita-devel mpv-devel` and a Rust toolchain (edition 2024). Running needs `yt-dlp`, and the Inter font for the intended look.

## Packages (.deb and .rpm)

Pushing a `vX.Y.Z` tag runs the [Packages workflow](.github/workflows/packages.yml), which builds a `.deb` (on Ubuntu 24.04, via `cargo-deb`) and an `.rpm` (in a Fedora 42 container, via `cargo-generate-rpm`) and attaches both to a GitHub pre-release. Download them from the repository's Releases page:

```sh
sudo apt install ./harv_*.deb     # Debian / Ubuntu
sudo dnf install ./harv-*.rpm     # Fedora
```

The tag must match `version` in `apps/harv/Cargo.toml`, or the release step fails. You still need `yt-dlp` installed to play anything, and the Inter font for the intended look.

## Install (current user)

```sh
./install.sh              # build --release, install ~/.local/bin/harv, icon + desktop entry (app menu)
./install.sh --no-build   # reuse target/release/harv
./install.sh --uninstall  # remove what it installed (your library in ~/.local/share/harv stays)
```

Build needs `gtk4-devel libadwaita-devel mpv-devel`; running needs `yt-dlp` (and the Inter font for the intended look).
Icon source: `packaging/dev.harv.Harv.svg`.

## Extra themes

Community/custom themes live in [harv-themes](https://github.com/iskaa02/harv-themes). Install them all with one command, then pick one in Preferences → Appearance → Style:

```sh
git clone https://github.com/iskaa02/harv-themes ~/.config/harv/themes
```

(If `~/.config/harv/themes` already exists, see the repo's README for a merge-in variant.)

## Run

```sh
export PATH=$HOME/.cargo/bin:$PATH     # if a distro cargo shadows rustup's
cargo run --release -p harv
```

Flag `--screenshot out.png` renders one frame and exits. For a fast edit loop, `./dev.sh harv` hot-reloads CSS and rebuilds on `.rs` changes (needs `inotifywait`). `GTK_DEBUG=interactive` opens the GTK Inspector.

## Data and licensing

Harv is licensed under the **GNU GPL v3 or later** (see [`LICENSE`](LICENSE)). It draws design ideas from the GPL-3.0 project [SimpMusic](https://github.com/maxrave-dev/SimpMusic).

`assets/cover.jpg` is an original generated placeholder (used by the `seek-widgets` gallery example). Lyrics, covers and metadata come from the services listed above at runtime.

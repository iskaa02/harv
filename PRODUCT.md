# Product

<!-- impeccable:product-schema 1 -->

## Platform

web
<!-- INFERRED: harv is a native Linux desktop app (Rust + GTK4 + libadwaita), not web/iOS/Android. The schema has no desktop value; "web" is the closest (CSS-styled, mouse/keyboard, resizable window). Confirm. -->

## Stack
Rust, GTK4, libadwaita (gtk4-rs). Chosen after measuring alternatives (docs/UI_STACK_COMPARISON.md). UI styling is GTK CSS (no :has(), no overflow, limited transitions) plus theme.toml tokens. GSK nodes for the lyrics screen; no custom shaders.

## Users
Linux desktop listeners who stream from YouTube Music and found SimpMusic (Kotlin/Compose) too heavy. Long sessions, music in the background, frequent glances, keyboard and mouse, many with lyrics visible. Often multilingual libraries (Arabic and Latin text appear side by side).

## Product Purpose
A fast, native music player for Linux: browse and search YouTube Music, keep a local library, play with effects, and watch word-synced lyrics on a beautiful full-screen view. Success: it feels instant, stays light on CPU/GPU, and is pleasant enough to leave open all day.

## Positioning
The Apple-Music-style word-synced lyrics screen and a fully themeable, native Linux UI, with no account lock-in. SimpMusic's server and YouTube sync are deliberately not used.

## Operating Context
Runs beside other windows, often in the background. MPRIS media keys, mini-player window, session restore. Needs mpv, yt-dlp. Nothing is exercised on real audio hardware, HiDPI or keyboard-only in tests yet (see check.txt).

## Capabilities and Constraints
- Home, search, charts, library lists, album/artist/playlist/podcast pages, queue, effects, sleep timer, downloads, preferences.
- Low CPU/GPU/RAM is a product goal; paused means zero animation work. Never call the app "lightweight" (about 400 MB RAM today).
- Everything visual is themeable (theme-core); the Blur theme must stay pixel-identical.
- No tray icon or always-on-top (not possible on GTK4/Wayland). No SimpMusic lyrics server. No tests are written by the maintainer; they test manually.

## Brand Commitments
Name "Harv". Inter for UI (the harv theme uses Red Hat Display). Voice: plain, warm, short. Never imply affiliation with YouTube, Google or SimpMusic. Icon: warm yellow rounded tile, not to be recoloured.

## Evidence on Hand
No user research. Screenshots via `--screenshot`. Third-party lyrics, artwork and metadata must never be committed.

## Product Principles
1. Light first: motion and effects pay for themselves or go.
2. Keyboard and mouse are equals; every frequent action has a shortcut that is shown.
3. The music and its artwork lead; the chrome recedes.
4. Customizable by default, good without touching a setting.
5. Be honest about state: loading, failing, empty states say what to do.

## Accessibility & Inclusion
RTL and non-Latin text must render correctly (Arabic appears in real feeds); respect the system reduced-motion preference; keep contrast AA on every theme in both schemes.

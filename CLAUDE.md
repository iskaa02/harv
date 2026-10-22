# CLAUDE.md — harv (Rust + GTK4 rewrite of SimpMusic's desktop UI)

## What this is

A Linux-native GTK4 music player being prototyped to replace the **SimpMusic** desktop app. The user ran
SimpMusic (Kotlin / Compose Multiplatform) on Linux and found it too heavy (self-reported ~20 % GPU, ~1 GB RAM). Goal:
**low CPU/GPU/RAM use** (currently ~400 MB RAM, so never describe the app as "lightweight" in docs, metadata or UI), with a beautiful Apple-Music-style lyrics screen (blur, glow, image manipulation,
animation) and a clean library UI.

- Stack: **Rust + GTK4 + libadwaita (gtk4-rs)**. Decided after building and measuring alternatives —
  read `docs/UI_STACK_COMPARISON.md` before proposing a different toolkit.
- `SimpMusic/` is the **upstream Kotlin project we will migrate features from**. It has its own `CLAUDE.md` (1000+ lines).
  Treat it as read-only reference: don't edit it, and don't apply its Gradle/Kotlin conventions here.
- Not a git repository. **There is no undo for deletes/overwrites** — look first, and ask before removing anything
  the user didn't name.

## Repo map (the repo root is the Cargo workspace)

```
crates/          GUI-free logic (no GTK anywhere in here)
  lyrics-core/   GUI-free: LRC parser, springs + staggered scroll (anim.rs), palette + backdrop (palette.rs),
                 shared layout (layout.rs), Scene (playback clock + animation state, lib.rs)
  ytm-core/      GUI-free: YouTube Music via ytmapi-rs (search/browse/radio/moods) + own home-feed query + yt-dlp stream resolver
                 with URL cache. Own plain types (Track, Album…); `ytm` CLI is the manual test harness
  player-core/   GUI-free: libmpv audio (audio.rs, `Clock` = lock-free position for lyrics), Queue (queue.rs: shuffle order,
                 repeat, radio trim), Engine actor (engine.rs: resolve→play, retry-once-then-skip, endless radio = one station per queue: seed + continuation token via ytm-core `radio_page`, never re-seeded from its own tail; `Ctx::play_row` + `QueueKind::Songs` + setting `auto_radio` make a row click in a loose list start that song's radio), MPRIS (mpris.rs);
                 `pcli` CLI is the harness (`HARV_AO=null` for headless runs)
widgets/         GTK widget crates shared by the apps
  lyrics-view/   the lyrics screen widget (GSK nodes, springs, word-level fill); real audio clock
apps/            GTK front-ends
  harv/       THE app: sidebar + pages (home, search/moods, charts, library lists, album/artist/playlist/podcast), mini bar,
                 now-playing (lyrics view + transport), queue, effects dialog, preferences, mini-player window, MPRIS, session
                 restore. `--screenshot` mode takes `--page/--search/--album/--artist/--playlist/--podcast/--play-search/--seek/
                 --now-playing/--queue/--settings/--palette Q ("-" = empty query)/--wait`, `--anim-frames` (with `--screenshot PREFIX`: saves `PREFIX-open-NN.png` / `-close-NN.png` frames of the full-screen open/close transition), `--then-play Q` (plays a second song ~12 s later to exercise the track-change animation; `LYRICS_FREEZE_T=0.3` pins its timers for a mid-motion shot; a reused `HARV_DB` with a saved queue makes screenshots panic "empty snapshot" — use a fresh one); use a scratch `HARV_DB=` and `HARV_AO=null`
  lyrics-net/    GUI-free: LyricsService — store cache → BetterLyrics (word-synced TTML; works for cached tracks without a key, `BETTERLYRICS_KEY`
                 / setting `betterlyrics_key` for the rest) → LRCLIB (exact /get, then fuzzy /search). NOT SimpMusic's lyrics server (403
                 "only for SimpMusic clients" — we don't impersonate it). `lyr` CLI is the harness. lyrics-core now parses enhanced LRC + TTML
                 into `Line{words}` / `Lyrics{sync}`
  theme-core/    GUI-free: themes — theme.toml schema + `extends`, colour parsing (oklch → sRGB), `{{token}}` CSS templating, loader
                 (user dir `~/.config/harv/themes/<id>/` over built-ins in `themes/<id>/theme.toml`). Design + status: docs/THEMING_DESIGN.md
  widgets/seek-widgets: Waveform (+ GUI-free `pseudo_peaks`) — themed seek widget (no timers; host calls set_progress)
  autoeq/        GUI-free: AutoEq headphone index + ParametricEQ profiles (cached in ~/.cache/harv)
  store-core/    GUI-free: SQLite (rusqlite, bundled) — songs/likes, local playlists, play history, most played, followed artists,
                 downloads state, lyrics cache + per-song offsets, typed settings kv (saved queue goes here), search history,
                 backup/restore (whole DB file), SimpMusic JSON import. `scli` CLI is the harness
assets/          cover.jpg (generated placeholder, used by the seek-widgets gallery example)
docs/            UI_STACK_COMPARISON.md, THEMING_DESIGN.md, mockups/
dev.sh           dev loop (CSS hot-reload + rebuild-and-swap)
SimpMusic/       upstream Kotlin reference clone (own git repo, git-ignored, read-only)
```

Keep UI-toolkit-free logic in the `*-core` crates (so the front-end stays swappable and the logic reusable); put
GTK-specific code in `apps/`. New logic crates go in `crates/`, new front-ends in `apps/`.

## Commands

**Always** put rustup's cargo first — `/usr/local/sbin/cargo` is 1.84 and shadows it (fails on edition 2024 deps):

```sh
export PATH=$HOME/.cargo/bin:$PATH     # cargo 1.99
cargo run --release -p harv         # the real app (./install.sh installs it + icon + desktop entry for the user)
./dev.sh harv                      # dev loop: CSS hot-reload + rebuild-and-swap on .rs changes
```

- `--screenshot out.png` renders one frame through GTK's own renderer and exits — the way to *see* a change
  without a human. Always look at the image (Read tool) after UI work. Window sizes get clamped to the 1366×768 screen.
- `dev.sh` details: saving `style.css` restyles live (needs `--watch`, which `dev.sh` passes); a failed Rust build leaves
  the old window running. Don't `pkill -f dev.sh` from your own shell command — the pattern matches your own command line
  and kills your shell; kill by PID.
- Debug builds are tuned in the workspace `Cargo.toml` (`opt-level` 1 own code / 3 deps); first debug build ≈ 4 min.

## Releases

Pushing a `vX.Y.Z` tag runs the Packages workflow, which fails at "Check the tag matches the crate version" unless `apps/harv/Cargo.toml`
has `version = "X.Y.Z"`. Before tagging: bump that version (and refresh `Cargo.lock`, e.g. `cargo update -p harv --offline`), commit, push
`main`, then tag that commit and push the tag. Never tag first — fixing it means force-moving a pushed tag.

## Working agreements (from the user)

- **Don't write or run tests.** The user tests manually. (Two legacy unit tests exist in `crates/lyrics-core/src/anim.rs`; leave
  them, don't add more, don't run them unprompted.)
- **Don't profile unless asked.** When a comparison is requested, build the thing; measure only on request.
- **Quality bar: "do it very well".** Polish spacing, typography and states; verify with screenshots, not assumptions.
- **Be honest about what wasn't exercised** (keyboard/hover/HiDPI/audio have never been tested live). State it plainly.
- Quick feedback loops matter to them — prefer changes that work with `dev.sh` hot reload.
- Match the surrounding code style. There is no rustfmt config and the code uses long lines; don't mass-reformat.

## Architecture notes

**Lyrics screen (`widgets/lyrics-view`)**
- `Scene::step(dt, tops, anchor_y)` is called once per frame; the frame-clock tick callback is added only while
  `Scene::animating()` and removes itself when settled (paused ⇒ 0 CPU/GPU). Keep that property.
- `Scroller` gives each line y/focus/dist springs; on active-line change lines at/below it start after
  `0.05 s × distance` (the cascade). `LineVis` (y, scale, alpha, blur, glow) is what the view draws.
- Drawing: backdrop = 4 rotating/mirrored copies of `palette::backdrop()` + 3 palette radial gradients; lines use
  `push_blur` / `push_opacity` / scale about the left-centre; active line gets a second blurred copy as glow.
- `GSK_RENDERER` is forced to `gl` unless the user sets it (GTK 4.22's Vulkan default cost ~2× CPU here).
  The renderer is named `gl` — `ngl` is a deprecated alias that logs a warning.
- Simulated clock only: **when real playback arrives, drive `Scene.pos` from the audio clock**, not from frame dt, or
  lyrics drift whenever frames drop or the window is hidden.
- Backdrop (full-screen, `widgets/lyrics-view`, `bg_group`): GSK nodes only — GTK 4.22's renderer rejects `GskGLShader` ("does not support gl shaders"), so no custom
  shaders. Blurred-cover layers + 6 palette blobs + conic fans + streaks + bokeh + line-change bloom + vignette + grain tile. `Scene` carries `flow` (clock sped up by `energy`),
  `energy` (lyric-line density), `pulse` (soft swell per line change). Strength is the global `lyrics_view::set_motion` (Preferences → "Lyrics backdrop motion",
  store key `backdrop_motion`: 0 off / 1 calm / 2 lively).
- Known cost: ~44 % GPU (relative, iGPU) — the lyrics screen is GTK's weak spot; mitigation ideas are in
  `docs/UI_STACK_COMPARISON.md` §6 (untried).


## GTK / gtk4-rs gotchas (all hit and fixed here)

- Features needed: `gtk4` `v4_14` (append_scaled_texture, ColumnViewCell), `libadwaita` `v1_4` (ToolbarView, Breakpoint).
  Crates are renamed in `Cargo.toml` (`gtk = { package = "gtk4" }`, `adw = { package = "libadwaita" }`).
- Custom widget wrappers need `@implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget`.
- ColumnView cell factories receive **`gtk::ColumnViewCell`**, not `ListItem` (downcasting wrongly panics).
- Fixed-size cover art: use `gtk::Image` + `pixel_size`. `gtk::Picture` takes its natural size and ignores
  `size_request` as a cap (covers rendered 60 px instead of 44).
- GTK CSS has **no `overflow` property and no `:has()`**; round-clip in code (`set_overflow(Hidden)`). Parse errors are
  silent unless you hook `CssProvider::connect_parsing_error` (we do). Useful nodes:
  `columnview > header > button`, `columnview > listview > row > cell`, `sort-indicator`.
- Use the GTK Inspector (`GTK_DEBUG=interactive`) to find node names and try CSS live.
- `SearchEntry::set_key_capture_widget(window)` gives type-anywhere search; grab focus on the list at startup or the
  entry shows a focus ring immediately.
- GTK synthesises pointer-motion events while content animates under a still pointer; ignore motion below a small
  distance threshold (fullscreen header auto-hide relied on this).
- Fonts: Pango asks fontconfig for **"Inter"**, so it must be installed (`~/.local/share/fonts`). Not bundled.
- Screenshots: `WidgetPaintable::new(window)` → `Snapshot` → `renderer.render_texture` → `save_to_png`.

## SimpMusic migration

Reference source: `SimpMusic/core/` (git submodule, cloned shallow — it was empty originally). Feature picks, design decisions and
the build order live in `~/.claude/plans/great-plan-all-this-parsed-crystal.md`. Needs `mpv-devel` + `yt-dlp` installed.
Decisions: ytmapi-rs + yt-dlp (not a scraper port); one now-playing style (the Apple-style lyrics screen); lyrics chain cache →
SimpMusic → BetterLyrics → LRCLIB; per-song lyric offset; endless radio is a setting (default on); plain equal-power crossfade only.
When the GTK UI hosts `Engine`: call `player_core::fix_locale()` after GTK init (libmpv needs LC_NUMERIC=C), run one tokio runtime
on a background thread, and forward `Update`s to the main loop via the `async_channel` receiver (`glib::spawn_future_local`).

## Themes and layouts

Everything visual is themeable: `crates/theme-core` (tokens, layouts, loader) + `apps/harv/src/{theme.rs,sidebar.rs,window.rs,components/}`.
Built-ins: blur (formerly classic; the original look — keep it pixel-identical: compare screenshots against a build before your change), harv (formerly mixer; the app-palette theme). Every theme has a `[colors]` palette for its native scheme plus a full `[colors_alt]` for the other; Preferences → Appearance → Style picks System / Light / Dark, store key `color_mode`, `HARV_MODE` env overrides; Orbit and Tape Deck were removed to cut surface; mockups still in `docs/mockups/`; there is no theme editor — the gallery in
Preferences only switches themes and opens `~/.config/harv/themes`). Design, status and gotchas: `docs/THEMING_DESIGN.md`. User-facing reference (derive, variants, `[vars]`, templated theme CSS, `{{shadow:N}}`/`{{ms:N}}`, `--validate-theme`, `--install-theme`): `docs/THEME_REFERENCE.md`; example in `docs/examples/catppuccin/`. Pages with no theme
layout use the built-in composition. CSS: `style.css` + `components/*.css` are a template (`{{token}}`, `@colors`); a theme's own
`style.css` is templated too (comments stripped first). libadwaita applies one breakpoint at a time. `--theme`, `--watch`, `--switch-to` for dev. The full-screen lyrics page is styled by an optional `[lyrics]` table (keys in `docs/THEMING_DESIGN.md`, "Full-screen lyrics page ([lyrics])"). Its text, progress bar and controls use the `np_fg` token (`np_on` on the play button), so a theme that changes `backdrop` must pick both to contrast with it, in `[colors]` and `[colors_alt]`.

## Data and licensing

`assets/cover.jpg` is an original generated placeholder. Never commit third-party lyrics, artwork or metadata (`assets/library/` and `docs/img/` are git-ignored). The project is GPL-3.0-or-later.

## Status / not done yet

Done and wired into `apps/harv` (only exercised headlessly — see `check.txt` for everything no human has tried: audio through
speakers, clicks, keyboard, HiDPI, real BetterLyrics key, login): home / search (+ suggestions, history, moods & genres) / charts /
library lists (favorites, recent, most played, following, playlists, downloads) / album, artist, playlist, podcast pages, mini bar,
now-playing with word-synced lyrics + per-song offset, queue, effects (EQ, speed, pitch, reverb, delay, AutoEq), crossfade, sleep timer,
SponsorBlock, downloads with offline playback, preferences (quality, endless radio, accent, backup/restore/import, caches, yt-dlp
cookies file), mini-player window, MPRIS, session restore. The old `demo-gtk` / `list-gtk` / `library-core` prototypes were removed (see git history).

Deliberately not done: SimpMusic's lyrics server (403 "only for SimpMusic clients" — we don't impersonate it), Listen Together, tray icon
(GTK4 has no API; always-on-top is also impossible on Wayland), Mix for You and YouTube library/like sync (need a logged-in
ytmapi-rs session — only a yt-dlp cookies file is supported), convolution reverb (multi-tap `aecho` instead).
Not done: collapsed/narrow layout (sidebar is fixed, min window 760 px), plain-lyrics scrolling, fallback fonts for CJK/Arabic/emoji
lyrics, HiDPI/fractional-scale verification, long-list virtualisation (track lists are plain ListBoxes).

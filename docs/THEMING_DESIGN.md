# Themes & layouts — design

## Implementation status

**Trimmed (surface reduction):** the Orbit and Tape Deck themes, the custom-theme editor, and everything only they used (ring/orbit-cover widgets, `now-card`, stat hero,
pill track list / pill queue, search `pills` / `top-result` results and bubble / tile moods, Lexend + Outfit fonts, `save.rs`) were deleted. The sections below still
describe them as the original design; only Classic, Mixer and Night Console exist now. Recover the rest from git history (commit `ba3b8f4` and earlier).

**Done and wired into `apps/harv`** (checked headlessly with screenshots, never clicked/typed/heard — see "Not exercised"):
- `crates/theme-core`: schema, `extends`, colours (oklch/hex/rgb), validation, templating, loader (user dir `~/.config/harv/themes/<id>/`
  over built-ins in `themes/<id>/{theme.toml,style.css}`), layout schema (`layout.rs`), bundled fonts table.
- Built-in themes: `classic` (the previous look, pixel-identical to the pre-theme build), `mixer`, `night-console`, `orbit`, `tape-deck`
  (all four mockups: tokens, sidebar, bottom bar, now-playing page, playlist/library/search layouts; sources in `docs/mockups/`).
- Layout engine `apps/harv/src/components/` (`mod.rs` registry + generic props; `detail` hero/track-list/cover-grid; `panel` queue-list/
  lyrics-teaser/now-card; `bar` cover/title/transport/seek(bar|thin|waveform)/volume/link + bottom bar; `now` now-cover/now-title/
  now-counter/lyrics/tabs/credits/next-card; `search` themed search page with lyric search). Shell (`window.rs`, `sidebar.rs`): sidebar
  items/playlists/search box, right panel per page kind (hidden under 1100 px), bottom bar modes, in-window now-playing host.
  Everything rebuilds live on theme change.
- `widgets/seek-widgets` (Waveform + `pseudo_peaks`, Ring, OrbitCover), `LyricsStyle::plain()` in lyrics-core/lyrics-view, FTS5 lyric
  search in store-core (`Store::search_lyrics`), runtime font registration (`fonts.rs`; Figtree/Lexend/Outfit/Red Hat bundled, OFL).
- Preferences → Appearance: theme gallery (live swatches) + Open folder.
- Dev: `--theme <id|path>`, `--watch` hot reload (style/component CSS + theme files), `--switch-to <theme>` (screenshot aid).

**Deliberately not done:** Mixer's output meters (no cheap level source); credits other than artist/album/length (ytm-core has none);
liner-note/"CH 01" style data that the app doesn't have; the Night Console "Credits" search chip.

**Not exercised** (no input automation here): clicks, drags, keyboard, hover, colour/font pickers, Preferences card activation, audio,
RTL lyrics in plain mode, HiDPI, release build. The Mixer table omits the ALBUM column (title truncates at 1280).

**Deviations from the sketch below:** tokens are `colors.{bg,sidebar,raise,hover,fg,dim,accent,accent_on,play_bg,play_fg,bar_bg,bar_fg,
toast_bg,toast_fg,np_fg,np_on}`, radii `shape.{xs,sm,md,lg,input,xl,control,cover,hero}` (`control` 9999 = fully round),
`effects.{ambient,bar_tint}`. Accent precedence: the user's explicit accent, then a theme's fixed accent, then the artwork colour.
Pages without a theme layout use the built-in composition, which is how classic stays unchanged. libadwaita applies only ONE matching
breakpoint at a time (window.rs counts applied breakpoints). Theme-specific CSS is raw (no `{{tokens}}`); area CSS files under
`components/*.css` are templated; never put a literal double-brace or a nested comment marker inside a CSS comment.
**Since then (token layer):** `docs/THEME_REFERENCE.md` is the user-facing reference. Added: `derive = true` palettes, `[variants]`, templated theme CSS with `[vars]`,
`[material]` (Material 3 palette generator, `crates/theme-core/src/material.rs`, regenerated per cover through `Ctx::apply_accent`), the surface ramp,
`{{st:}}` / `{{fs:}}` / `{{fw:}}` / `{{type:}}` / `{{ease:}}` / `{{shadow:}}` / `{{ms:}}` template functions, per-component and pressed shapes, `item_style = "rail"`,
`seek style = "wavy"` (`widgets/seek-widgets/src/wavy.rs`), `play = "rounded"` and `group = "connected"`. The app CSS was converted to these functions with
`[shell] panel_side`, `sidebar.position` / `collapse`, `bottom_bar.position`, `[page.home]` / `[page.artist]` with `heading` / `shelf` / `shelves` (`PageData::Home`, `DetailData.shelves`), cover-driven Material colours kept in the stored theme (`Theme::reseeded`), defaults that reproduce Blur exactly (check with `HARV_DUMP_CSS=<path>`, which writes the final stylesheet, and diff before/after).

## Decisions already made

- **No level meters** (Mixer's output meters). mpv has no cheap level output; skipped entirely.
- **Waveform seekbar is a pseudo-waveform** (Night Console) — see §7. It is decoration shaped like a waveform, not audio data.
- Everything else in the mockups is built from data the app already has, except **liner notes** (credits) — §9 open question.

## 1. What the mockups share, and where they differ

Shared: a shell of **title bar · sidebar (210–240 px) · main · optional right panel (280–330 px) · bottom bar (72–80 px)**, and
the same data (track, queue, lyrics, progress, search results, library lists).

| | Mixer | Night Console | Orbit | Tape Deck |
|---|---|---|---|---|
| Palette | light grey, red `oklch(.62 .17 25)` | dark, cyan `#7FD6E8` | light, mint `oklch(.72 .12 165)` | cream, orange `#B5552B` / `#D9825A` |
| Font | Red Hat Display + Red Hat Mono | Figtree | Lexend | Outfit |
| Radii | 4 / 6 / 8 | 10–18 | pills (999) + circles (50 %) | 5–16, pills |
| Right panel | queue + lyrics (playlist page) | lyrics + queue (Now page) | queue + one lyric line (Now page) | now playing + up next (playlist page) |
| Now playing | big cover left, big lyrics right, tabs Lyrics/Credits/Queue | cover + **waveform seek** + tracklist; lyrics in side panel | **circular cover with progress ring** + decorative satellites | big cover, big lyrics, tabs Lyrics/Liner notes/Queue, credits grid |
| Library | track table + hero | **cover grid**, index badges | **circular covers**, ring stat, hero | table + hero |
| Search | lyric matches first | lyric matches, query highlighted, "Play from line" | round mood bubbles + pill results | top-result card + songs + image tiles |
| Bottom bar | every screen | not on Now page | ring around cover | dark bar, every screen |
| Vocabulary | channels | collections | orbits | tapes |

Consequence: tokens alone cover maybe 40 %. Page composition differs too, so layouts are required.

## 2. Today's architecture (what we're building on)

From `apps/harv/src`: `window.rs` builds sidebar + `adw::NavigationView` + `mini_bar` inside a `gtk::Stack` with a second
page `"playing"` (full-bleed `LyricsView`, `player::now_playing`). Pages are functions in `pages.rs`; row/card helpers in
`rows.rs` (`track_list`, `card`, `shelf`, `detail_header`); styling is one `style.css` (hot-reloaded in `--watch`) plus the
accent `CssProvider` in `ctx.rs`. Sidebar items are a hard-coded `SIDEBAR` array.

Implications:
- Sidebar, mini bar and page headers are already separate builders — easy to turn into catalog components.
- "Now playing" is a separate full-screen stack page. Three of the four mockups want it **inside the shell** (sidebar visible);
  the classic theme keeps the full-screen page. So now-playing needs two hosting modes (§5).
- Pages are imperative Rust today; they must become *layout-driven*, which is most of the cost.

## 3. Theme package

```
~/.config/harv/themes/<id>/
  theme.toml      metadata, tokens, vocabulary, lyrics params, shell + page layouts
  style.css       optional: extra GTK CSS appended last (escape hatch)
  fonts/          optional: *.ttf/*.otf registered with fontconfig at load
```

Built-in themes (`classic` + the four mockups once built) ship embedded via `include_str!`; a user folder with the same `<id>`
overrides it. Selected in Preferences (`theme` setting, typed kv in `store-core`). Switching applies live.

Loader lives in a GUI-free crate: **`crates/theme-core`** (serde + `toml`): parse, validate, resolve `extends`, produce a
`Theme` struct. The GTK side (`apps/harv/src/theme.rs`) turns that into CSS providers and builds layouts.
A theme may `extends = "classic"` and override only some keys.

## 4. Tokens

```toml
[meta]
name = "Mixer"
id = "mixer"
extends = "classic"
scheme = "light"                    # light | dark → adw::StyleManager color scheme

[colors]                            # hex or rgb()/oklch() — theme-core converts to sRGB hex
bg = "#F4F4F2"        surface = "#FFFFFF"     sidebar = "#ECECEA"
text = "#111214"      text_dim = "#55575A"    text_faint = "#6E7074"
line = "#D2D3CF"      bar_bg = "#111214"      bar_text = "#ECECEA"
accent = "oklch(0.62 0.17 25)"      accent_on = "#FFFFFF"
selected = "oklch(0.93 0.035 25)"

[fonts]
ui = "Red Hat Display"   mono = "Red Hat Mono"   label_tracking = 0.06      # em

[shape]
radius_sm = 4  radius_md = 6  radius_lg = 8  cover_radius = 6  control = "rounded"   # rounded | pill | circle
[metrics]
sidebar_w = 240  panel_w = 290  bar_h = 80  row_h = 52  gap = 14
```

How tokens reach GTK:
- GTK CSS has `@define-color` but I'm **not relying on `var()`** (unverified on the installed GTK). Colours are emitted as
  `@define-color` lines; numeric tokens are substituted into the base stylesheet as `{{radius_md}}` placeholders before it's
  loaded. Load order: base sheet (templated) → accent provider (existing) → theme `style.css`.
- `oklch()` is converted in `theme-core` (GTK CSS can't parse it).
- Sizes that CSS can't drive (sidebar width, bar height, cover px) are read by the layout builder from `[metrics]`.
- Dynamic cover accent (`ctx.apply_accent`) stays; `accent = "cover"` makes the theme follow it instead of a fixed colour.

**Fonts.** Name only is the base case (user installs the font, as with Inter today). If `fonts/` exists, register files with
fontconfig's app-font API at theme load; if that proves awkward from Rust, fall back to documenting a manual install.

**Vocabulary** (the channels/collections/orbits/tapes difference):

```toml
[words]
playlist = "Channel"  playlists = "Channels"  queue = "Up next"  library = "Library"  lyrics = "Lyrics"
```
Every user-visible noun in the app is looked up through `words` (default = current English text). Section labels can use
`text-transform`-style casing via `label_case = "upper"`.

**Lyrics parameters** (`[lyrics]`): the Apple-style view stays one widget; its constants move into a struct fed from the
theme (today in `lyrics-core/layout.rs` and `anim.rs`):

```toml
[lyrics]
style = "spotlight"      # spotlight = current blur/glow/cascade; "plain" = size/colour by distance, no blur/glow/backdrop
active_scale = 1.0   dim_alpha = [0.30, 0.45, 0.70]     # by distance 3+, 2, 1
blur = 0.0   glow = 0.0   backdrop = "none"             # none | cover-blur | gradient
font_px = 30  active_font_px = 38  line_gap = 18  align = "start"
spring_stiffness = 1.0  cascade_delay = 0.05
```
`plain` is what Mixer/Tape Deck show; `spotlight` is what exists. Both keep the "tick only while animating" rule.

### Full-screen lyrics page ([lyrics])

Themes whose now-playing host is the full-screen page (`[shell.now_playing]` with `host = "fullscreen"`, e.g. Blur and the user theme
Sorbet) can restyle that page through an optional `[lyrics]` table. Every key is optional and the defaults reproduce Blur's look exactly.
The table supersedes the older sketch above, whose keys (`style`, `font_px`, …) were never built. Source of truth: `LyricsTheme` in
`crates/theme-core/src/lib.rs`; the mapping to the widget is `fullscreen_style()` in `apps/harv/src/player.rs`.

| Key | Values | Default | Effect |
|---|---|---|---|
| `font` | `"ui"` or a font family | `"ui"` | Font for lines and titles; `"ui"` is the theme's UI font. |
| `active_size` | px | absent | Size of the active line. Absent = scaled from the window height. |
| `size` | px | absent | Size of the other lines. Absent = scaled from the window height. |
| `line_gap` | px | absent | Gap between lines. Absent = scaled from the window height. |
| `align` | `start`, `center` | `start` | Horizontal alignment of the lines. |
| `blur` | multiplier | `1.0` | Blur applied to lines away from the active one. |
| `glow` | multiplier | `1.0` | Glow behind the active line. |
| `spring` | multiplier | `1.0` | Spring stiffness of line motion. |
| `cascade` | seconds | `0.05` | Delay per line of distance from the active line before it moves (the cascade). |
| `backdrop` | `cover`, `gradient`, `solid`, `none` | `cover` | `cover`: animated artwork. `gradient`: accent blobs on the theme `bg`. `solid`: flat `bg` with one glow. `none`: the page colour (`@bg`). |
| `layout` | `split`, `centered`, `lyrics-only` | `split` | `split`: cover left, lyrics right. `centered`: small cover in a header row, lyrics centred. `lyrics-only`: no cover. Windows too narrow for either use the stacked compact layout. |
| `cover_radius` | px | absent | Corner radius of the cover art. Absent = widget default. |
| `cover_shadow` | bool | `true` | Drop shadow under the cover. |
| `chrome` | `plain`, `capsule` | `plain` | Transport row and header buttons. `capsule` draws a translucent pill behind them. |

Colours: the text, progress bar and controls are drawn in the `np_fg` token (`np_on` on the play button), in both `[colors]` and
`[colors_alt]`. A theme that changes `backdrop` must pick `np_fg` / `np_on` that contrast with it. The `gradient` and `solid` backdrops
are built from the theme `bg` and the live accent, so they follow the Accent setting and light/dark mode.

Preferences shows the "Lyrics backdrop motion" row only for full-screen themes whose `backdrop` is `cover`.

Example (the Sorbet user theme):

```toml
# (inherited from blur) [shell.now_playing] host = "fullscreen"

[lyrics]
backdrop = "gradient"
layout = "centered"
align = "center"
chrome = "capsule"
cover_radius = 18
blur = 0.8
glow = 0.6
```

## 5. Shell and layout

The shell is fixed; themes choose what goes in each region.

```toml
[shell]
sidebar = { show = true, width = 240, items = ["home","search","liked","recent", "section:channels", "playlists", "action:new-playlist"], footer = [] }
titlebar = { style = "plain", right = "samplerate" }       # decoration dots are header-bar styling, not real controls
right_panel = { show = "per-page" }                        # show | hide | per-page
bottom_bar = { show = "always" }                           # always | except:now | hide
now_playing = { host = "page" }                            # page (in-shell) | fullscreen (classic stack page)
```

Pages are component trees, one per page kind:

```toml
[page.playlist]
layout = [
  { c = "hero",        cover = 170, kind_label = true, actions = ["play","shuffle","menu"], meta = ["crossfade","gain"] },
  { c = "track-list",  style = "table", columns = ["index","title","artist","album","time"], cover = 36 },
]
right_panel = [
  { c = "queue-list",  style = "rows", limit = 4, title = "words.queue" },
  { c = "lyrics-teaser", lines = 2, align = "bottom" },
]
```

Rules that keep this from becoming a UI framework:
- **Closed catalog** (§6). Unknown component or prop → ignored with a logged warning, never a crash.
- Containers are only `row`, `column`, `stack`, `grid(cols)`, `split(left,right,ratio)`; props are simple scalars/enums.
- Page kinds are fixed: `home, search, charts, library, playlist(album/playlist/podcast), artist, now-playing, queue, settings`.
  Themes override the ones they care about; the rest fall back to `classic`'s layout.
- Components bind to the shared `Ctx` (state hooks, engine, store) exactly as today; a layout never touches data.

**Now-playing hosting.** `host = "page"` renders the `[page.now-playing]` tree as a normal page in the navigation view (bottom
bar per `bottom_bar`). `host = "fullscreen"` keeps today's stack page. Either way the lyrics component is the same `LyricsView`.

## 6. Component catalog (v1)

| Component | Variants / props | Needed by |
|---|---|---|
| `sidebar` | items, footer, section labels, item shape (`pill`/`rounded`/`flat`), show collection art | all |
| `titlebar` | `plain`, right text slot (`samplerate`, `track`, `none`) | Mixer |
| `bottom-bar` | parts list: `cover, title, transport, seek, volume, time, ring-cover, lyrics-link`; `seek = bar/ring/none`; 72–80 px | all |
| `hero` | cover px + shape (`square`/`round`/`ring-stat`), kind label, title size, actions, meta chips | playlist, library |
| `track-list` | `style = table / rows / pills`, columns, cover px, index/equalizer, highlight playing row | all |
| `cover-grid` | `shape = square/round`, cols, cover px, badge (`index`/`none`), caption lines | Night Console, Orbit |
| `shelf` | existing horizontal shelf | home |
| `queue-list` | `rows / pills`, limit, reorder handle | panels |
| `lyrics` | `lyrics.*` params from theme; optional tabs (`lyrics/credits/queue`) | now playing, panels |
| `lyrics-teaser` | N lines around the active one | panels |
| `now-cover` | size, radius, `ring` (progress ring around it), `satellites` (decorative thumbs from queue) | now playing |
| `seek` | `bar / ring / waveform` (pseudo, §7) | now playing, bars |
| `transport` | buttons list, play button `circle/pill`, sizes | now playing, bars |
| `search-box` | `rect / pill`, shortcut hint, history | search, sidebar |
| `filter-chips` | `rect / pill`, items | search |
| `lyric-hits` | cards/rows with highlighted match + "Play from line" | search |
| `top-result` | big card with art + play | Tape Deck search |
| `mood-tiles` | `bubble` (round) / `image-tile` (text over art) | search |
| `credits` | key/value grid (Written/Produced/…) | Mixer tab, Tape Deck |
| `meta-chips` | crossfade, gain, sample rate | Mixer |

Existing builders (`track_list`, `card`, `shelf`, `detail_header`, `mini_bar`, sidebar) get re-expressed as these components with
variant props; the classic theme = today's layouts expressed in the new format.

## 7. Pseudo-waveform (`seek = "waveform"`)

A bar-style seekbar whose bar heights *look* like a waveform but are generated, not decoded. Not presented as real audio data.

- **Shape generation** (`player-core` or `theme-core`, GUI-free, pure function): `peaks(video_id: &str, n: usize) -> Vec<f32>`
  in 0.12..1.0. Seed an RNG from a hash of `video_id` (stable per track, so it doesn't change between plays). Build smooth noise
  (sum of 2–3 sines with seeded phases/frequencies plus small jitter), multiply by an envelope (short ramp-in, a few
  "sections" of different loudness, fade-out in the last ~4 %), then normalise. Bar count `n` follows widget width
  (≈ one bar per 7 px) and is recomputed only on resize.
- **Widget**: a custom `gtk::Widget` snapshotting rounded rects; bars before the playhead use the accent, the rest the dim colour.
  Hover shows a thin cursor + time tooltip; click/drag seeks via `Command::Seek` (same path as the existing `Scale`).
- **Updates**: position comes from the audio `Clock`; redraw only when the playhead crosses a bar boundary, driven by the
  existing 250 ms ticker that already runs only while playing. Paused ⇒ no ticks.
- **Cost**: no decoding, no network, no allocation after the first build per track (cache the `Vec` for the current track).
- **Future (out of scope)**: real peaks for downloaded tracks (decode once, store in the DB). The widget takes `&[f32]`, so
  swapping the source later changes nothing else.

Progress **ring** (Orbit) is the same idea: a custom widget stroking an arc with GSK paths (needs `gtk4` `v4_14` — already enabled).

## 8. Validation, errors, hot reload

- Validation on load in `theme-core`; errors carry file + key path. On any error the app logs it, shows a toast, and keeps the
  previous (or `classic`) theme — never a blank window.
- Hot reload (extends today's `--watch`): watch `theme.toml` and `style.css`; CSS-only changes swap the provider; `theme.toml`
  changes rebuild the shell and visible page (state like playback is untouched; scroll position may reset).
- `harv --theme <path>` for development; `--screenshot` works per theme so every theme can be checked headlessly
  (`--page`, `--now-playing`, `--search` flags already exist).
- A schema doc generated from the catalog (`docs/THEME_REFERENCE.md`) is the user-facing reference — written once the catalog settles.

## 9. Open questions / risks

- **Liner notes / credits** (Tape Deck, Mixer tab): I haven't checked whether ytmapi-rs exposes credits for most tracks. Until
  verified, `credits` renders only when data exists and hides otherwise.
- **Lyric search** (Mixer, Night Console): only covers songs with lyrics already in the store cache. Needs a store query
  (FTS5 virtual table over cached lyrics is the likely route; not designed here).
- **Font loading from the theme folder** may need `fontconfig` FFI; fallback is manual install.
- **Layout rebuild on live reload** can lose transient UI state; acceptable for a dev feature.
- **Test matrix**: 5 themes × pages. Per CLAUDE.md we don't write tests, so checking is screenshots per theme — budget for it.
- **Scope creep**: the catalog is the control. New component = Rust change; resist adding generic containers/expressions.
- Nothing here has been prototyped; sizes in the mockups assume 1280×800 and the app's min window is 760 px — responsive rules
  per theme (e.g. hide right panel under N px) are not designed yet.

## 10. Build order

1. `crates/theme-core`: schema, TOML parse, colour conversion (oklch), validation, `extends`. Embed `classic`.
2. Token pipeline in `apps/harv`: templated base CSS, theme provider, `[words]` lookups, `[lyrics]` params into `LyricsView`.
   *Checkpoint: re-skin the app to Mixer's palette/fonts with no layout change.*
3. Componentise: turn sidebar / mini bar / track list / hero / cards into catalog components with variants; classic layouts
   re-expressed as data. Must render identically to today (screenshot compare).
4. Layout engine: shell regions + page trees + now-playing `page` host.
5. New components: `seek` waveform + ring, `now-cover` ring/satellites, `lyric-hits`, `top-result`, `credits`.
6. Themes in order: **Mixer** (closest to today's structure) → **Tape Deck** (same shell, different now-playing) →
   **Night Console** (waveform, cover grid) → **Orbit** (round everything, ring, most custom widgets).
7. Preferences theme picker, `--theme`, hot reload for `theme.toml`, `docs/THEME_REFERENCE.md`.

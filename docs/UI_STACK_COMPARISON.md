# UI stack comparison and why we picked GTK

Decision: **Rust + GTK4 + libadwaita (gtk4-rs)**, with the lyrics screen drawn through GSK in a custom widget.

> **Note:** the prototypes this comparison refers to (`demo-gtk`, `list-gtk`, `library-core`, `LIST_DESIGN.md`) were removed from the tree once their ideas moved into the real app; they remain in git history.

This records what was actually built, measured and seen, so the choice can be revisited with facts.
Everything marked **built** was implemented and run on this machine; everything marked **researched** is web
research only (not tried). The non-GTK prototypes were deleted after the decision; the lessons from them are in
[§5](#5-what-the-deleted-prototypes-taught-us).

- Machine: Intel UHD 620 (Whiskey Lake) + NVIDIA MX110 (nouveau), Fedora 44, GNOME on Wayland, 1366×768 @ 60 Hz,
  GTK 4.22 / libadwaita 1.9, Rust 1.99.
- Why leave SimpMusic's stack at all: the user reported ~20 % GPU and ~1 GB RAM for the Kotlin/Compose desktop app
  (user-reported, not measured by us).

## 1. Candidates

| Stack | Status | Notes |
|---|---|---|
| **GTK4 + libadwaita** (gtk4-rs) | **built** (lyrics screen + library list) | Chosen. |
| winit + wgpu + cosmic-text, hand-rolled | **built** (lyrics screen only) | Fastest, but no widgets at all. |
| iced 0.14 (+ its `Shader` widget for wgpu) | **built** (lyrics screen + library list) | Best "in between" candidate. |
| Slint | researched | Retained, own markup, free desktop licence but commercial otherwise; can import wgpu textures (from memory, unverified). |
| egui | researched | Immediate mode; default backend moved to wgpu in 0.34; reads as a dev tool; idling needs care. |
| GPUI (Zed) | researched | Pre-1.0; crates.io has third-party snapshots, frequent breaking changes; Linux renderer is now wgpu. |
| Makepad | researched | Shader-first DSL, strong on animation; Wayland support not confirmed; different programming model. |
| Xilem | researched | Described as not production-ready. |
| Dioxus / Tauri | researched | Webview-based — the weight we are trying to escape. |
| Tessera / Operad / Blinc | researched | Young wgpu UI libs (custom shaders / canvas / backdrop blur); no evidence of stability. |

Sources for the researched rows (search results were thin; treat as directional):
[2026 survey](https://daily.dev/posts/a-2026-survey-of-rust-gui-libraries-e93cfc0b7) ·
[landscape 2026](https://wrenlearnsrust.com/posts/2026-03-11-rust-gui-landscape-2026.html) ·
[LogRocket](https://blog.logrocket.com/state-rust-gui-libraries/) ·
[Windows GUI guide](https://rust-pc.github.io/rust-windows-gui.html) ·
[COSMIC 1.5 uses iced](https://borncity.com/news/cosmic-1-5-0-system76-bringt-ersten-rust-desktop-stabil/) ·
[iced shader widget](https://docs.iced.rs/iced/widget/shader/index.html) ·
[gpui-pre](https://docs.rs/crate/gpui-pre/0.3.1) ·
[Tessera](https://docs.rs/crate/tessera-ui/latest) ·
[Makepad docs](https://publish.obsidian.md/makepad-docs).

## 2. Measurements — Apple-Music-style lyrics screen

Same scene in all three (blurred animated backdrop, lyric lines with blur-by-distance and glow, springs, cover art).
Release builds, simulated clock, 60 Hz, ~1366×701 windowed; averaged over 10 s after warm-up
(measurement script since removed). GPU = i915 render-engine busy time, which is inflated by the iGPU idling at 300 MHz — **compare
relatively, not absolutely**. The machine was not idle during measurement.

| | CPU (1 core) | GPU | RSS | Notes |
|---|---|---|---|---|
| winit + wgpu (hand-rolled) | ~8–10 % | ~14 % | ~130 MB | fullscreen ~9 % / ~15 % |
| iced (`Shader` widget, same pipeline) | ~13.5 % | ~17 % | ~127 MB | +3–5 pts CPU, +3 pts GPU vs raw |
| **GTK4, GSK `gl` renderer** | ~19 % | **~44 %** | ~167 MB | fullscreen ~21 % / **~54 %** |
| GTK4, GSK Vulkan (GTK 4.22 default) | ~39 % | ~44 % | ~113 MB | 2× the CPU of `gl` |
| any of them, paused | 0 % | 0 % | same | no frames are produced |

Private heap is ~43 MB (wgpu) / ~49 MB (GTK); ~65 MB of every RSS is shared Mesa/Vulkan driver mappings.

**This is GTK's weak spot and we chose it anyway** — see §4. GTK uses roughly 3× the GPU time of the hand-rolled
renderer for this blur-heavy screen. It is a known, accepted cost with untried mitigations (§6).

Two environment traps found along the way:
- iced/wgpu default to the *high-performance* adapter, which here is the discrete NVIDIA GPU. A music player must not
  wake it; set `WGPU_POWER_PREF=low`. (It also made our GPU counter read a false 0 %.)
- GTK's default Vulkan renderer cost 2× the CPU of `GSK_RENDERER=gl` for this scene. The renderer's name in GTK 4.22
  is `gl` (`ngl` is a deprecated alias that logs a warning).

## 3. Library list — GTK vs iced

Same design spec (`LIST_DESIGN.md`, removed), same data (`library-core`, 84 songs / 43 albums), same
features: search, sortable headers, selection, keyboard navigation, play + animated equalizer, empty state, narrow
layout.

| | GTK (`list-gtk`) | iced (`list-iced`, deleted) |
|---|---|---|
| Size | 669 lines (356 main + 258 widgets + 55 CSS) | 501 lines, one file |
| Virtualisation | ColumnView creates only visible rows | `scrollable` lays out all 84 rows |
| Sorting / selection / keyboard nav / double-click | built in | hand-built |
| Scroll-to-selection | `scroll_to` | hand-built from viewport tracking |
| Text ellipsis | yes | **no** — hard clip |
| Tabular digits, letter-spacing | CSS | not available |
| Narrow-width layout | `adw::Breakpoint` | `responsive` + manual |
| Animation only while visible | tick callback on bound row, removed on `unbind` | subscription gated on viewport |
| Styling | CSS (slowest part: ColumnView node names found by trial) | in code, typed |
| Visual result | **user's verdict: "looks much better"** | — |

Effort: iced was less code and compiled almost first time; GTK needed ~150 lines of GObject boilerplate and a CSS
session. But iced re-implements, by hand, behaviour GTK provides — and a real library adds album grids, context menus,
drag-and-drop, thousands of rows. In GTK those are existing widgets; in iced each is new custom work.

## 4. Why GTK

1. **Looks better** in side-by-side use (user's judgement on the library list; the lyrics screen looks equivalent
   across all three — the difference there is cost, not appearance).
2. **Cost of the rest of the app.** SimpMusic is mostly lists, search, queues, menus and settings. GTK/libadwaita ship
   those; every other option makes us build them. This outweighs the lyrics screen being the cheapest part to write
   in any stack.
3. **Maturity and stability.** GTK 4 / libadwaita are stable; iced is pre-1.0 and fast-moving (we already straddled
   two wgpu majors, 27 in iced vs 30 standalone).
4. **Platform fit.** Linux-first app: Wayland, fractional scaling, theming, portals, fullscreen/headerbar behaviour
   all come from the toolkit.
5. **Idle behaviour is excellent when done right** — GTK's frame clock lets the lyrics view stop ticking entirely when
   paused and settled (measured 0 %).

### What we accepted
- ~3× the GPU of a hand-rolled renderer on the lyrics screen (§2).
- gtk4-rs verbosity (GObject subclassing) and CSS-node spelunking (use the GTK Inspector).
- GTK-flavoured, Linux-only UI; fonts come from fontconfig (Inter must be installed — not bundled).

### What would change the decision
- If the lyrics screen cannot reach an acceptable budget after the mitigations in §6, embed the custom renderer
  rather than abandon GTK (hybrid, below).
- If a non-Linux target becomes a requirement, GTK stops being a good fit; reconsider iced or Slint.

## 5. What the deleted prototypes taught us

The standalone wgpu renderer (and its iced wrapper) are gone. The techniques that mattered, in case a hybrid
lyrics view is ever built:

- **Rasterise lyric lines once** (cosmic-text) into small R8 coverage masks with a CPU-built mip chain; per-frame GPU
  work is then just sampling. Blur = sample a mip level (+ 4 taps); glow = 8-tap ring. Jittered/random taps looked grainy.
- **Render the backdrop at ¼ resolution** into an offscreen texture and stretch it (+ per-pixel dither at full res).
  With **cropping each line's quad to its current blur margin**, this halved GPU: 29 % → 14 %.
- **Backdrop recipe:** 4 mirrored copies of a heavily blurred 96-px version of the cover, slowly rotating/drifting,
  plus 3 palette-colour radial blobs, darkened ~24 %. Palette = hue-binned, saturation-weighted (now in `lyrics-core`).
- **Animation:** per-line springs with a start delay of 50 ms × distance below the active line (the cascade),
  `focus`/`dist` springs driving alpha/scale/blur. Frame-rate independent via fixed 1/240 s sub-steps (now in
  `lyrics-core::anim`).
- **Idle:** stop requesting frames when `Scene::animating()` is false.
- **Pitfalls:** pick the low-power adapter; non-sRGB vs sRGB surface changes how blends look; GTK emits synthetic motion
  events while content animates under a still pointer (don't treat them as user activity).

## 6. Untried mitigations for GTK's lyrics GPU cost

Ideas only — none implemented or measured:
- Replace per-line `push_blur` (an offscreen pass per blurred line, per frame) with pre-blurred line textures
  (CPU-rendered once per layout) cross-faded by distance.
- Render the animated backdrop once at low resolution into a texture and animate only its transform, or update it at
  30 fps.
- Blur fewer lines (distance > N → fade out instead of blur).
- Try `GtkGraphicsOffload`, or a `GtkGLArea` hosting the standalone renderer described in §5 (hybrid).
- Re-measure GL vs Vulkan GSK renderers after each change; the winner has flipped between scenes before.

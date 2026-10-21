# Design system (harv) — draft, recorded from current code

Source of truth: crates/theme-core (tokens, layouts), apps/harv/src/style.css + components/*.css (templates with {{token}}), themes/{blur,harv}/theme.toml.

## Tokens
- Colours: bg, sidebar, raise, hover, fg, dim, accent (artwork by default), accent_on, play_bg/fg, bar_bg/fg, toast_bg/fg, np_fg/np_on (lyrics screen). Every theme has a native scheme plus `colors_alt`.
- Shape: xs/sm 4, md 6, lg/xl 8, input 8, control 6, cover 4, hero 6 (harv theme).
- Metrics: sidebar_w 240, track_row_h 52, label_tracking 0.7.
- Fonts: ui "Red Hat Display" (harv) / Inter (blur), mono "Red Hat Mono".

## Composition
Fixed sidebar (brand, nav, Library section, Preferences) + content with large H1 and artwork-tinted gradient. Shelves: 160 px covers with title + subtitle and prev/next buttons. Lists: 52–62 px rows (cover, title, artist/album, duration, heart, menu). Search: chips for Songs, Albums, Artists, Playlists, Videos, Podcasts. Mini bar at bottom; full-screen lyrics page with palette backdrop.

## Known gaps (the redesign works on these)
No motion tokens; no density control; header-bar title duplicates the page H1; no global command palette (Ctrl+F opens the Search page); empty states have no action and the subtitle's first glyph is clipped; Charts rows have no rank; no hover play affordance on rows (untested live).

Already present (do not rebuild): shortcuts sheet (F1 / ?), Recent + Favorites shelves on Home, skeleton shelves, error state with retry, like-undo toast, playing-row highlight, sidebar collapse under 940 px.

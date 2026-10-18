# Library list – design spec (implemented by `list-gtk`)

Data: `library-core` (84 songs, 43 albums). Default order = artist (ignoring "The "), year, album, track.

**Layout** (content clamped to 1040 px wide, centred)
- Header block: "Library" large title (30 px, 800) + summary line "84 songs · 5 hr 12 min" (13 px, 50 % white) on the left; search field (260 px) on the right. Summary follows the filter.
- Column header row (32 px): `Title` · `Album` · `Time`; 12 px, 600, 45 % white; the active sort column is white with a sort arrow. Click toggles asc/desc.
- Rows 60 px high, 10 px corner radius, 12 px side inset. Cover 44 px, radius 6.
  - Title 14 px/600 white; artist 12.5 px/500 58 % white; album 13 px 58 %; time 13 px 50 %, tabular digits. Long text ellipsised.
- Below 640 px width: Album column hides.

**Colour** – window `#121214`; row hover 5.5 % white; selected 10 % white.
**Playing row** – title coloured with the artwork accent; the cover gets a 45 % black scrim with a 3-bar equalizer (accent coloured). Animation only runs while a playing row is on screen.
**Empty state** – "No Results" / "Try a different search."
**Input** – type anywhere to search · Esc clears · ↑/↓ move selection · Enter or double-click plays.
**Font** – Inter (must be installed system-wide; fontconfig resolves it).

CLI: `--size WxH --search Q --select N --play N --screenshot out.png [--watch]` (N = row index in the current view; `--watch` hot-reloads the CSS).

This spec was originally written so a GTK and an iced build could be compared like-for-like; see `docs/UI_STACK_COMPARISON.md`.

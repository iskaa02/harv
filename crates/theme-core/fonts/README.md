# Bundled fonts

Font files compiled into harv (`include_bytes!` in `src/fonts.rs`) so the built-in themes work without installing
anything. All are licensed under the SIL Open Font License 1.1; the licence text for each family is next to its font
(`OFL-<family>.txt`) and must travel with it.

| File | Family | Source | Copyright |
|---|---|---|---|
| `Figtree-Variable.ttf` | Figtree (lagoon) | https://github.com/google/fonts/tree/main/ofl/figtree | 2022 The Figtree Project Authors |
| `RedHatDisplay-Variable.ttf` | Red Hat Display (harv) | https://github.com/google/fonts/tree/main/ofl/redhatdisplay | 2024 The Red Hat Project Authors |
| `RedHatMono-Variable.ttf` | Red Hat Mono (harv) | https://github.com/google/fonts/tree/main/ofl/redhatmono | 2024 The Red Hat Project Authors |

Files are the upright variable (`wght` axis) builds from the Google Fonts repository, renamed without brackets. Italics
are not bundled. Inter is not bundled (the blur theme uses the system copy).

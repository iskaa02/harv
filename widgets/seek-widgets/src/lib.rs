//! Themed seek and cover widgets for the player's themes.
//!
//! Colours come from CSS. For every widget the **accent** (played / progress part) is the widget's CSS `color`,
//! and the **dim** part (track, rest) is that colour at 25 % alpha. `set_accent_color` / `set_dim_color` override
//! either one when a theme cannot express it in CSS. Widgets never run timers: the host calls `set_progress`, and a
//! widget only redraws when the visible result changes. (`Wavy` animates its wave only while `set_playing(true)`.)
//!
//! CSS node names: `seek-waveform`, `seek-wavy`.

mod peaks;
mod util;
mod wavy;
mod waveform;

pub use peaks::pseudo_peaks;
pub use waveform::Waveform;
pub use wavy::Wavy;

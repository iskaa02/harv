//! GUI-free playback: libmpv audio, play queue (shuffle/repeat/radio), and the engine that ties them to `ytm-core`.

mod audio;
mod engine;
mod fx;
mod mpris;
mod queue;

pub use audio::{fix_locale, Clock};
pub use fx::{AudioFx, ParaKind, Parametric, Reverb, EQ_BANDS_HZ, EQ_PRESETS};
pub use engine::{Command, Engine, Snapshot, SleepInfo, SleepMode, Status, Update};
pub use mpris::Mpris;
pub use queue::{Queue, QueueKind, QueueState, Repeat};

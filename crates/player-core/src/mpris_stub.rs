//! Stand-in for the MPRIS service on platforms other than Linux (mpris.rs) and macOS (mpris_macos.rs): same API, does nothing.

use crate::engine::{Engine, Snapshot};
use anyhow::Result;

pub struct Mpris;

impl Mpris {
    pub async fn start(_engine: Engine) -> Result<Mpris> { anyhow::bail!("media-key integration is only implemented on Linux (MPRIS)") }
    pub fn update(&self, _s: &Snapshot) {}
}

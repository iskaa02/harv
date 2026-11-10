//! Non-Linux stand-in for the MPRIS service (D-Bus): same API, does nothing. A macOS Now Playing / media-key backend would replace this.

use crate::engine::{Engine, Snapshot};
use anyhow::Result;

pub struct Mpris;

impl Mpris {
    pub async fn start(_engine: Engine) -> Result<Mpris> { anyhow::bail!("media-key integration is only implemented on Linux (MPRIS)") }
    pub fn update(&self, _s: &Snapshot) {}
}

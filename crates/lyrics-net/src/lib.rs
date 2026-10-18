//! Lyrics fetching: local cache → BetterLyrics (word-synced TTML; without an API key only songs cached on their server) → LRCLIB (exact match, then fuzzy search).
//!
//! SimpMusic's own lyrics server is deliberately *not* used: it answers 403 "only available to SimpMusic clients" and only
//! opens up to requests signed with a key embedded in their app — this app isn't one, so we don't impersonate it.

mod providers;

use anyhow::Result;
use lyrics_core::lrc::{Lyrics, Sync};
use std::time::Duration;
use store_core::Store;
use ytm_core::Track;

/// Re-ask the providers about a "nobody has these lyrics" answer after a week.
const NEGATIVE_TTL_MS: i64 = 7 * 24 * 3600 * 1000;

#[derive(Clone)]
pub struct LyricsService { http: reqwest::Client, store: Store, betterlyrics_key: Option<String> }

pub(crate) struct Found { pub source: &'static str, pub kind: &'static str, pub body: String }

impl LyricsService {
    /// The BetterLyrics key comes from the `betterlyrics_key` setting, or the `BETTERLYRICS_KEY` env var.
    pub fn new(store: Store) -> Result<Self> {
        let key = store.get::<String>("betterlyrics_key").or_else(|| std::env::var("BETTERLYRICS_KEY").ok()).filter(|k| !k.is_empty());
        let http = reqwest::Client::builder().user_agent("harv/0.1").timeout(Duration::from_secs(15)).build()?;
        Ok(Self { http, store, betterlyrics_key: key })
    }

    pub fn store(&self) -> &Store { &self.store }

    /// `None` = no provider has lyrics for this track. `force` ignores the cache.
    pub async fn lyrics(&self, t: &Track, force: bool) -> Result<Option<Lyrics>> {
        if !force {
            if let Some(c) = self.store.cached_lyrics(&t.video_id) {
                if c.kind != "none" { return Ok(Some(Lyrics::from_text(&c.body, &c.source))); }
                if store_core::now_ms() - c.fetched_at < NEGATIVE_TTL_MS { return Ok(None); }
            }
        }
        let mut last_err = None;
        let mut found = None;
        match providers::betterlyrics(&self.http, self.betterlyrics_key.as_deref(), t).await { Ok(f) => found = f, Err(e) => last_err = Some(e) }
        if found.is_none() {
            match providers::lrclib(&self.http, t).await { Ok(f) => found = f, Err(e) => last_err = Some(e) }
        }
        match found {
            Some(f) => {
                self.store.cache_lyrics(&t.video_id, f.source, f.kind, &f.body)?;
                Ok(Some(Lyrics::from_text(&f.body, f.source)))
            }
            // Only remember "none" when the providers actually answered; a network failure must not poison the cache.
            None => match last_err { Some(e) => Err(e), None => { self.store.cache_lyrics(&t.video_id, "none", "none", "")?; Ok(None) } },
        }
    }

    /// Per-song timing offset in ms (positive = lyrics later).
    pub fn offset_ms(&self, video_id: &str) -> i64 { self.store.lyric_offset_ms(video_id) }
    pub fn set_offset_ms(&self, video_id: &str, ms: i64) -> Result<()> { self.store.set_lyric_offset_ms(video_id, ms) }
}

pub fn describe(l: &Lyrics) -> &'static str { match l.sync { Sync::Rich => "word-synced", Sync::Line => "line-synced", Sync::Plain => "unsynced" } }

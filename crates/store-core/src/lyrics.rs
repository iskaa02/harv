//! Lyrics cache (raw provider text, parsed by `lyrics-core`) and per-song timing offsets.
//! The offset lives in its own table so it survives a lyrics re-fetch or provider switch.

use crate::{lyrsearch, now_ms, Store};
use anyhow::Result;
use rusqlite::params;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedLyrics {
    /// `simpmusic` | `betterlyrics` | `lrclib`
    pub source: String,
    /// `rich` | `synced` | `plain` | `none` (negative cache: every provider came up empty)
    pub kind: String,
    pub body: String,
    pub fetched_at: i64,
}

impl Store {
    pub fn cached_lyrics(&self, video_id: &str) -> Option<CachedLyrics> {
        self.db().query_row("SELECT source, kind, body, fetched_at FROM lyrics_cache WHERE video_id = ?1", [video_id],
            |r| Ok(CachedLyrics { source: r.get(0)?, kind: r.get(1)?, body: r.get(2)?, fetched_at: r.get(3)? })).ok()
    }

    /// Caches the lyrics of a song (replacing any previous entry) and re-indexes its lines for [`Store::search_lyrics`].
    pub fn cache_lyrics(&self, video_id: &str, source: &str, kind: &str, body: &str) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute("INSERT INTO lyrics_cache(video_id, source, kind, body, fetched_at) VALUES(?1, ?2, ?3, ?4, ?5)
                    ON CONFLICT(video_id) DO UPDATE SET source = excluded.source, kind = excluded.kind, body = excluded.body, fetched_at = excluded.fetched_at",
                   params![video_id, source, kind, body, now_ms()])?;
        lyrsearch::reindex(&tx, video_id, body, source)?;
        tx.commit()?;
        Ok(())
    }

    pub fn forget_lyrics(&self, video_id: &str) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        lyrsearch::unindex(&tx, video_id)?;
        tx.execute("DELETE FROM lyrics_cache WHERE video_id = ?1", [video_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Number of cached lyric entries and their total size in bytes (for the storage settings).
    pub fn lyrics_cache_stats(&self) -> (usize, usize) {
        self.db().query_row("SELECT COUNT(*), COALESCE(SUM(LENGTH(body)), 0) FROM lyrics_cache", [], |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)? as usize))).unwrap_or((0, 0))
    }

    pub fn clear_lyrics_cache(&self) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        lyrsearch::clear(&tx)?;
        tx.execute("DELETE FROM lyrics_cache", [])?;
        tx.commit()?;
        Ok(())
    }

    pub fn lyric_offset_ms(&self, video_id: &str) -> i64 {
        self.db().query_row("SELECT offset_ms FROM lyric_offset WHERE video_id = ?1", [video_id], |r| r.get(0)).unwrap_or(0)
    }

    /// Positive = lyrics appear later. Zero removes the row.
    pub fn set_lyric_offset_ms(&self, video_id: &str, ms: i64) -> Result<()> {
        if ms == 0 { self.db().execute("DELETE FROM lyric_offset WHERE video_id = ?1", [video_id])?; }
        else { self.db().execute("INSERT INTO lyric_offset(video_id, offset_ms) VALUES(?1, ?2) ON CONFLICT(video_id) DO UPDATE SET offset_ms = excluded.offset_ms", params![video_id, ms])?; }
        Ok(())
    }
}

//! GUI-free persistence: one SQLite file for songs/likes, local playlists, play history, queue, lyrics cache + per-song offsets,
//! and settings. Schema is our own (real foreign keys, no JSON id lists) — SimpMusic's Room schema is only a reference.

mod backup;
mod library;
mod lyrics;
mod lyrsearch;

pub use backup::{ImportReport, SimpMusicImport};
pub use library::{PlaylistInfo, SongRow};
pub use lyrics::CachedLyrics;
pub use lyrsearch::LyricHit;

use anyhow::Result;
use rusqlite::Connection;
use serde::{de::DeserializeOwned, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

/// 2 = lyric line index (`lyric_line` + `lyric_fts`), backfilled from `lyrics_cache` once by `migrate`.
const SCHEMA_VERSION: i64 = 2;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS song (
    video_id        TEXT PRIMARY KEY,
    title           TEXT NOT NULL,
    artists         TEXT NOT NULL DEFAULT '[]',   -- JSON array of names
    album           TEXT,
    duration_s      INTEGER,
    thumb           TEXT,
    liked           INTEGER NOT NULL DEFAULT 0,
    favorite_at     INTEGER,
    total_play_time INTEGER NOT NULL DEFAULT 0,   -- seconds, lifetime
    download_state  INTEGER NOT NULL DEFAULT 0,   -- 0 none, 1 preparing, 2 downloading, 3 downloaded
    downloaded_at   INTEGER,
    download_path   TEXT,                         -- file inside <data dir>/harv/downloads
    added_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS song_liked ON song(liked, favorite_at DESC);
CREATE INDEX IF NOT EXISTS song_played ON song(total_play_time DESC);

CREATE TABLE IF NOT EXISTS artist (
    channel_id  TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    thumb       TEXT,
    followed    INTEGER NOT NULL DEFAULT 0,
    followed_at INTEGER
);

CREATE TABLE IF NOT EXISTS local_playlist (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    title      TEXT NOT NULL,
    thumb      TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS playlist_song (
    playlist_id INTEGER NOT NULL REFERENCES local_playlist(id) ON DELETE CASCADE,
    video_id    TEXT    NOT NULL REFERENCES song(video_id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    added_at    INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, video_id)
);
CREATE INDEX IF NOT EXISTS playlist_song_pos ON playlist_song(playlist_id, position);

CREATE TABLE IF NOT EXISTS playback_event (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    ts         INTEGER NOT NULL,
    video_id   TEXT NOT NULL REFERENCES song(video_id) ON DELETE CASCADE,
    listened_s INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS playback_event_ts ON playback_event(ts DESC);
CREATE INDEX IF NOT EXISTS playback_event_video ON playback_event(video_id, ts);

CREATE TABLE IF NOT EXISTS search_history (
    query TEXT PRIMARY KEY,
    ts    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS lyrics_cache (
    video_id   TEXT PRIMARY KEY,
    source     TEXT NOT NULL,      -- simpmusic | betterlyrics | lrclib
    kind       TEXT NOT NULL,      -- rich | synced | plain | none
    body       TEXT NOT NULL,
    fetched_at INTEGER NOT NULL
);

-- One row per non-empty lyric line of every cached song (kept in sync by lyrics.rs in the same transaction).
-- `text` is the folded line (lower-case, no diacritics/harakat) that the FTS index reads; `line` is the line as displayed.
-- `lyric_fts` is an external-content FTS5 index over `lyric_line.text` (rowid = lyric_line.id).
CREATE TABLE IF NOT EXISTS lyric_line (
    id         INTEGER PRIMARY KEY,
    video_id   TEXT    NOT NULL,
    line_index INTEGER NOT NULL,     -- position in the parsed lyrics (instrumental/empty lines skipped, index kept)
    start_ms   INTEGER NOT NULL,     -- 0 for unsynced lyrics
    text       TEXT    NOT NULL,
    line       TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS lyric_line_video ON lyric_line(video_id);
CREATE VIRTUAL TABLE IF NOT EXISTS lyric_fts USING fts5(
    text, content='lyric_line', content_rowid='id', tokenize='unicode61 remove_diacritics 2'
);

CREATE TABLE IF NOT EXISTS lyric_offset (
    video_id  TEXT PRIMARY KEY,
    offset_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS setting (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL            -- JSON
);
";

/// Schema plus additive migrations. Idempotent: runs on every open and after a restore from an older backup.
pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    // Columns added after the first schema: fresh DBs already got them from SCHEMA, so "duplicate column" is expected there.
    if let Err(e) = conn.execute("ALTER TABLE song ADD COLUMN download_path TEXT", []) {
        if !e.to_string().contains("duplicate column") { return Err(e.into()); }
    }
    // One-time: index lyrics cached before the FTS index existed. Gated on user_version so it never runs on a normal start.
    // A restored backup carries its own user_version, so an older backup is backfilled here too.
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version < 2 {
        let tx = conn.unchecked_transaction()?;
        lyrsearch::rebuild_index(&tx)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
    }
    Ok(())
}

/// Cheap to clone; all clones share one connection behind a mutex (queries are tiny, called from UI or async tasks alike).
#[derive(Clone)]
pub struct Store { conn: Arc<Mutex<Connection>> }

pub fn now_ms() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0) }

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(dir) = path.as_ref().parent() { std::fs::create_dir_all(dir)?; }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> { Self::init(Connection::open_in_memory()?) }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        migrate(&conn)?;
        let v: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if v < SCHEMA_VERSION { conn.pragma_update(None, "user_version", SCHEMA_VERSION)?; }
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    pub(crate) fn db(&self) -> MutexGuard<'_, Connection> { self.conn.lock().unwrap() }

    /// Typed key/value settings (volume, quality, endless, saved queue, …), stored as JSON.
    pub fn set<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.db().execute("INSERT INTO setting(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", (key, serde_json::to_string(value)?))?;
        Ok(())
    }

    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let json: String = self.db().query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| r.get(0)).ok()?;
        serde_json::from_str(&json).ok()
    }

    pub fn search_history(&self, limit: usize) -> Result<Vec<String>> {
        let db = self.db();
        let mut st = db.prepare("SELECT query FROM search_history ORDER BY ts DESC LIMIT ?1")?;
        let rows = st.query_map([limit as i64], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn add_search(&self, q: &str) -> Result<()> {
        let q = q.trim();
        if q.is_empty() { return Ok(()); }
        self.db().execute("INSERT INTO search_history(query, ts) VALUES(?1, ?2) ON CONFLICT(query) DO UPDATE SET ts = excluded.ts", (q, now_ms()))?;
        Ok(())
    }

    pub fn clear_search_history(&self) -> Result<()> { self.db().execute("DELETE FROM search_history", [])?; Ok(()) }
}

//! Songs, likes, local playlists, play history, followed artists, downloads — and the "dynamic" library lists built from them.
//! Queries mirror SimpMusic's `DatabaseDao` (favorites by `favorite_at`, most played = lifetime `total_play_time`, …).

use crate::{now_ms, Store};
use anyhow::{bail, Result};
use rusqlite::{params, Row};
use std::collections::HashSet;
use ytm_core::Track;

/// A song as stored (a `Track` plus library state).
#[derive(Debug, Clone)]
pub struct SongRow { pub track: Track, pub liked: bool, pub play_time_s: i64, pub download_state: i64 }

#[derive(Debug, Clone)]
pub struct PlaylistInfo { pub id: i64, pub title: String, pub thumb: Option<String>, pub count: usize }

const SONG_COLS: &str = "video_id, title, artists, album, duration_s, thumb, liked, total_play_time, download_state";

fn song_row(r: &Row) -> rusqlite::Result<SongRow> {
    let artists: String = r.get(2)?;
    Ok(SongRow {
        track: Track {
            video_id: r.get(0)?, title: r.get(1)?, artists: serde_json::from_str(&artists).unwrap_or_default(), album: r.get(3)?,
            duration_s: r.get::<_, Option<i64>>(4)?.map(|d| d as u32), thumb: r.get(5)?,
        },
        liked: r.get::<_, i64>(6)? != 0, play_time_s: r.get(7)?, download_state: r.get(8)?,
    })
}

impl Store {
    /// Insert or refresh metadata; never touches likes / play time / download state.
    pub fn upsert_song(&self, t: &Track) -> Result<()> { upsert(&self.db(), t) }

    pub fn song(&self, video_id: &str) -> Option<SongRow> {
        self.db().query_row(&format!("SELECT {SONG_COLS} FROM song WHERE video_id = ?1"), [video_id], song_row).ok()
    }

    // ── likes ────────────────────────────────────────────────────────────────────────────────
    pub fn set_liked(&self, t: &Track, liked: bool) -> Result<()> {
        let db = self.db();
        upsert(&db, t)?;
        db.execute("UPDATE song SET liked = ?2, favorite_at = ?3 WHERE video_id = ?1", params![t.video_id, liked as i64, liked.then(now_ms)])?;
        Ok(())
    }

    pub fn is_liked(&self, video_id: &str) -> bool {
        self.db().query_row("SELECT liked FROM song WHERE video_id = ?1", [video_id], |r| r.get::<_, i64>(0)).map(|v| v != 0).unwrap_or(false)
    }

    pub fn liked_ids(&self) -> HashSet<String> {
        let db = self.db();
        let Ok(mut st) = db.prepare("SELECT video_id FROM song WHERE liked = 1") else { return HashSet::new() };
        st.query_map([], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    /// Favorites, newest like first.
    pub fn liked_songs(&self, limit: usize, offset: usize) -> Result<Vec<SongRow>> {
        self.songs(&format!("SELECT {SONG_COLS} FROM song WHERE liked = 1 ORDER BY favorite_at DESC LIMIT ?1 OFFSET ?2"), limit, offset)
    }

    // ── history & most played ────────────────────────────────────────────────────────────────
    /// Call when a track stops: records a history event (if it was actually listened to) and adds to the lifetime counter.
    /// "Listened" = ≥ 30 s or ≥ half the track, like most scrobblers.
    pub fn record_play(&self, t: &Track, listened_s: f64) -> Result<bool> {
        let secs = listened_s.max(0.0) as i64;
        let counts = secs >= 30 || t.duration_s.is_some_and(|d| d > 0 && secs * 2 >= d as i64);
        if !counts { return Ok(false); }
        let db = self.db();
        upsert(&db, t)?;
        db.execute("INSERT INTO playback_event(ts, video_id, listened_s) VALUES(?1, ?2, ?3)", params![now_ms(), t.video_id, secs])?;
        db.execute("UPDATE song SET total_play_time = total_play_time + ?2 WHERE video_id = ?1", params![t.video_id, secs])?;
        Ok(true)
    }

    /// Distinct tracks, most recently played first.
    pub fn recently_played(&self, limit: usize, offset: usize) -> Result<Vec<SongRow>> {
        self.songs("SELECT s.video_id, s.title, s.artists, s.album, s.duration_s, s.thumb, s.liked, s.total_play_time, s.download_state
                    FROM song s JOIN (SELECT video_id, MAX(ts) AS last FROM playback_event GROUP BY video_id) e ON e.video_id = s.video_id
                    ORDER BY e.last DESC LIMIT ?1 OFFSET ?2", limit, offset)
    }

    /// SimpMusic's "Most Played": lifetime listening time, top 50.
    pub fn most_played(&self) -> Result<Vec<SongRow>> {
        self.songs(&format!("SELECT {SONG_COLS} FROM song WHERE total_play_time > 1 ORDER BY total_play_time DESC LIMIT ?1 OFFSET ?2"), 50, 0)
    }

    pub fn clear_history(&self) -> Result<()> {
        let db = self.db();
        db.execute("DELETE FROM playback_event", [])?;
        db.execute("UPDATE song SET total_play_time = 0", [])?;
        Ok(())
    }

    // ── followed artists ─────────────────────────────────────────────────────────────────────
    pub fn set_followed(&self, channel_id: &str, name: &str, thumb: Option<&str>, followed: bool) -> Result<()> {
        self.db().execute("INSERT INTO artist(channel_id, name, thumb, followed, followed_at) VALUES(?1, ?2, ?3, ?4, ?5)
                           ON CONFLICT(channel_id) DO UPDATE SET name = excluded.name, thumb = COALESCE(excluded.thumb, thumb), followed = excluded.followed, followed_at = excluded.followed_at",
                          params![channel_id, name, thumb, followed as i64, followed.then(now_ms)])?;
        Ok(())
    }

    pub fn followed_artists(&self) -> Result<Vec<(String, String, Option<String>)>> {
        let db = self.db();
        let mut st = db.prepare("SELECT channel_id, name, thumb FROM artist WHERE followed = 1 ORDER BY followed_at DESC")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn is_followed(&self, channel_id: &str) -> bool {
        self.db().query_row("SELECT followed FROM artist WHERE channel_id = ?1", [channel_id], |r| r.get::<_, i64>(0)).map(|v| v != 0).unwrap_or(false)
    }

    // ── downloads ────────────────────────────────────────────────────────────────────────────
    pub fn set_download_state(&self, t: &Track, state: i64) -> Result<()> {
        let db = self.db();
        upsert(&db, t)?;
        db.execute("UPDATE song SET download_state = ?2, downloaded_at = ?3 WHERE video_id = ?1", params![t.video_id, state, (state == 3).then(now_ms)])?;
        Ok(())
    }

    /// Record a finished download: the file at `path` is what plays offline.
    pub fn set_downloaded(&self, t: &Track, path: &str) -> Result<()> {
        let db = self.db();
        upsert(&db, t)?;
        db.execute("UPDATE song SET download_state = 3, downloaded_at = ?2, download_path = ?3 WHERE video_id = ?1", params![t.video_id, now_ms(), path])?;
        Ok(())
    }

    /// Forget the download (the caller deletes the file).
    pub fn clear_download(&self, video_id: &str) -> Result<()> {
        self.db().execute("UPDATE song SET download_state = 0, downloaded_at = NULL, download_path = NULL WHERE video_id = ?1", [video_id])?;
        Ok(())
    }

    /// After the data folder moved: point every recorded download from `from` to `to` (paths are stored absolute).
    pub fn rebase_downloads(&self, from: &str, to: &str) {
        let _ = self.db().execute("UPDATE song SET download_path = ?2 || substr(download_path, length(?1) + 1) WHERE substr(download_path, 1, length(?1)) = ?1", [from, to]);
    }

    pub fn download_path(&self, video_id: &str) -> Option<String> {
        self.db().query_row("SELECT download_path FROM song WHERE video_id = ?1", [video_id], |r| r.get::<_, Option<String>>(0)).ok().flatten()
    }

    /// Every recorded download as (video_id, path); the app registers the ones whose file still exists.
    pub fn downloaded_paths(&self) -> Vec<(String, String)> {
        let db = self.db();
        let Ok(mut st) = db.prepare("SELECT video_id, download_path FROM song WHERE download_path IS NOT NULL") else { return vec![] };
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    pub fn downloaded_songs(&self) -> Result<Vec<SongRow>> {
        self.songs(&format!("SELECT {SONG_COLS} FROM song WHERE download_state = 3 ORDER BY downloaded_at DESC LIMIT ?1 OFFSET ?2"), 10_000, 0)
    }

    // ── local playlists ──────────────────────────────────────────────────────────────────────
    pub fn create_playlist(&self, title: &str) -> Result<i64> {
        let title = title.trim();
        if title.is_empty() { bail!("playlist needs a title"); }
        let now = now_ms();
        let db = self.db();
        db.execute("INSERT INTO local_playlist(title, created_at, updated_at) VALUES(?1, ?2, ?2)", params![title, now])?;
        Ok(db.last_insert_rowid())
    }

    pub fn rename_playlist(&self, id: i64, title: &str) -> Result<()> {
        self.db().execute("UPDATE local_playlist SET title = ?2, updated_at = ?3 WHERE id = ?1", params![id, title.trim(), now_ms()])?;
        Ok(())
    }

    pub fn set_playlist_thumb(&self, id: i64, thumb: Option<&str>) -> Result<()> {
        self.db().execute("UPDATE local_playlist SET thumb = ?2 WHERE id = ?1", params![id, thumb])?;
        Ok(())
    }

    pub fn delete_playlist(&self, id: i64) -> Result<()> { self.db().execute("DELETE FROM local_playlist WHERE id = ?1", [id])?; Ok(()) }

    /// Newest first. A playlist without its own thumb borrows its first track's.
    pub fn playlists(&self) -> Result<Vec<PlaylistInfo>> {
        let db = self.db();
        let mut st = db.prepare("SELECT p.id, p.title,
                COALESCE(p.thumb, (SELECT s.thumb FROM playlist_song ps JOIN song s ON s.video_id = ps.video_id WHERE ps.playlist_id = p.id ORDER BY ps.position LIMIT 1)),
                (SELECT COUNT(*) FROM playlist_song WHERE playlist_id = p.id)
            FROM local_playlist p ORDER BY p.updated_at DESC")?;
        let rows = st.query_map([], |r| Ok(PlaylistInfo { id: r.get(0)?, title: r.get(1)?, thumb: r.get(2)?, count: r.get::<_, i64>(3)? as usize }))?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn playlist_tracks(&self, id: i64) -> Result<Vec<Track>> {
        let rows = self.songs_with("SELECT s.video_id, s.title, s.artists, s.album, s.duration_s, s.thumb, s.liked, s.total_play_time, s.download_state
                               FROM playlist_song ps JOIN song s ON s.video_id = ps.video_id WHERE ps.playlist_id = ?3 ORDER BY ps.position LIMIT ?1 OFFSET ?2", i64::MAX as usize, 0, Some(id))?;
        Ok(rows.into_iter().map(|r| r.track).collect())
    }

    pub fn playlist_contains(&self, id: i64, video_id: &str) -> bool {
        self.db().query_row("SELECT 1 FROM playlist_song WHERE playlist_id = ?1 AND video_id = ?2", params![id, video_id], |_| Ok(())).is_ok()
    }

    /// Appends, skipping tracks already in the playlist. Returns how many were added.
    pub fn add_to_playlist(&self, id: i64, tracks: &[Track]) -> Result<usize> {
        let db = self.db();
        let tx = db.unchecked_transaction()?;
        let mut pos: i64 = tx.query_row("SELECT COALESCE(MAX(position) + 1, 0) FROM playlist_song WHERE playlist_id = ?1", [id], |r| r.get(0))?;
        let mut added = 0;
        for t in tracks {
            upsert(&tx, t)?;
            let n = tx.execute("INSERT OR IGNORE INTO playlist_song(playlist_id, video_id, position, added_at) VALUES(?1, ?2, ?3, ?4)", params![id, t.video_id, pos, now_ms()])?;
            if n > 0 { pos += 1; added += 1; }
        }
        tx.execute("UPDATE local_playlist SET updated_at = ?2 WHERE id = ?1", params![id, now_ms()])?;
        tx.commit()?;
        Ok(added)
    }

    pub fn remove_from_playlist(&self, id: i64, video_id: &str) -> Result<()> {
        let db = self.db();
        let tx = db.unchecked_transaction()?;
        tx.execute("DELETE FROM playlist_song WHERE playlist_id = ?1 AND video_id = ?2", params![id, video_id])?;
        renumber(&tx, id)?;
        tx.commit()?;
        Ok(())
    }

    /// Move the track at index `from` to index `to` (both 0-based, in current order).
    pub fn move_in_playlist(&self, id: i64, from: usize, to: usize) -> Result<()> {
        let db = self.db();
        let tx = db.unchecked_transaction()?;
        let mut ids: Vec<String> = {
            let mut st = tx.prepare("SELECT video_id FROM playlist_song WHERE playlist_id = ?1 ORDER BY position")?;
            let rows = st.query_map([id], |r| r.get(0))?.collect::<Result<_, _>>()?;
            rows
        };
        if from >= ids.len() || to >= ids.len() { return Ok(()); }
        let v = ids.remove(from);
        ids.insert(to, v);
        for (i, v) in ids.iter().enumerate() { tx.execute("UPDATE playlist_song SET position = ?3 WHERE playlist_id = ?1 AND video_id = ?2", params![id, v, i as i64])?; }
        tx.commit()?;
        Ok(())
    }

    fn songs(&self, sql: &str, limit: usize, offset: usize) -> Result<Vec<SongRow>> { self.songs_with(sql, limit, offset, None) }
    fn songs_with(&self, sql: &str, limit: usize, offset: usize, extra: Option<i64>) -> Result<Vec<SongRow>> {
        let db = self.db();
        let mut st = db.prepare(sql)?;
        let (limit, offset) = (limit.min(i64::MAX as usize) as i64, offset as i64);
        let rows = match extra {
            Some(e) => st.query_map(params![limit, offset, e], song_row)?.collect::<Result<_, _>>()?,
            None => st.query_map(params![limit, offset], song_row)?.collect::<Result<_, _>>()?,
        };
        Ok(rows)
    }
}

fn renumber(db: &rusqlite::Connection, id: i64) -> Result<()> {
    let ids: Vec<String> = {
        let mut st = db.prepare("SELECT video_id FROM playlist_song WHERE playlist_id = ?1 ORDER BY position")?;
        let rows = st.query_map([id], |r| r.get(0))?.collect::<Result<_, _>>()?;
        rows
    };
    for (i, v) in ids.iter().enumerate() { db.execute("UPDATE playlist_song SET position = ?3 WHERE playlist_id = ?1 AND video_id = ?2", params![id, v, i as i64])?; }
    Ok(())
}

pub(crate) fn upsert(db: &rusqlite::Connection, t: &Track) -> Result<()> {
    db.execute("INSERT INTO song(video_id, title, artists, album, duration_s, thumb, added_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(video_id) DO UPDATE SET title = excluded.title, artists = excluded.artists, album = COALESCE(excluded.album, album),
                    duration_s = COALESCE(excluded.duration_s, duration_s), thumb = COALESCE(excluded.thumb, thumb)",
               params![t.video_id, t.title, serde_json::to_string(&t.artists)?, t.album, t.duration_s, t.thumb, now_ms()])?;
    Ok(())
}

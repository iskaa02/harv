//! Backup / restore of the whole database file (settings included) and import of SimpMusic's "web converter" JSON.

use crate::library::upsert;
use crate::{migrate, now_ms, Store};
use anyhow::{bail, Context, Result};
use rusqlite::backup::Backup;
use rusqlite::{params, Connection, OpenFlags};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;
use ytm_core::Track;

/// SimpMusic's `ImportData` (`core/domain/.../importdata/ImportData.kt`); unknown keys are ignored like upstream.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct SimpMusicImport { pub songs: Vec<ImportSong>, pub playlists: Vec<ImportPlaylist> }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSong {
    pub video_id: String,
    pub title: String,
    #[serde(default)] pub artist_name: Option<Vec<String>>,
    #[serde(default)] pub album_name: Option<String>,
    #[serde(default)] pub duration_seconds: u32,
    #[serde(default)] pub thumbnails: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPlaylist {
    pub title: String,
    #[serde(default)] pub thumbnail: Option<String>,
    #[serde(default)] pub video_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportReport { pub playlists_created: usize, pub songs_imported: usize, pub skipped_entries: usize }

impl Store {
    /// Consistent snapshot of the live database into `path` (overwritten).
    pub fn backup_to(&self, path: impl AsRef<Path>) -> Result<()> {
        let mut dst = Connection::open(path.as_ref()).context("cannot create backup file")?;
        let src = self.db();
        let b = Backup::new(&src, &mut dst)?;
        b.run_to_completion(256, Duration::from_millis(0), None)?;
        Ok(())
    }

    /// Replace everything with the contents of a backup made by [`Store::backup_to`]. The file is validated first.
    pub fn restore_from(&self, path: impl AsRef<Path>) -> Result<()> {
        let src = Connection::open_with_flags(path.as_ref(), OpenFlags::SQLITE_OPEN_READ_ONLY).context("cannot open backup")?;
        let ok: i64 = src.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('song', 'local_playlist', 'setting')", [], |r| r.get(0))?;
        if ok < 3 { bail!("not a harv backup"); }
        let mut db = self.db();
        {
            let b = Backup::new(&src, &mut db)?;
            b.run_to_completion(256, Duration::from_millis(0), None)?;
        }
        db.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&db)?; // backups from an older schema get any newer tables and columns
        Ok(())
    }

    /// Songs first, then playlists; playlist entries whose song isn't in the file are dropped and counted (as upstream does).
    pub fn import_simpmusic(&self, json: &str) -> Result<ImportReport> {
        let data: SimpMusicImport = serde_json::from_str(json).context("not a SimpMusic export")?;
        if data.songs.is_empty() && data.playlists.is_empty() { bail!("the file has no songs and no playlists"); }
        let db = self.db();
        let tx = db.unchecked_transaction()?;
        let mut known = std::collections::HashSet::new();
        for s in &data.songs {
            let t = Track { video_id: s.video_id.clone(), title: s.title.clone(), artists: s.artist_name.clone().unwrap_or_default(), album: s.album_name.clone(),
                            duration_s: (s.duration_seconds > 0).then_some(s.duration_seconds), thumb: s.thumbnails.clone() };
            upsert(&tx, &t)?;
            known.insert(s.video_id.as_str());
        }
        let (mut created, mut skipped) = (0, 0);
        for p in &data.playlists {
            let ids: Vec<&String> = p.video_ids.iter().filter(|id| known.contains(id.as_str())).collect();
            skipped += p.video_ids.len() - ids.len();
            let now = now_ms();
            tx.execute("INSERT INTO local_playlist(title, thumb, created_at, updated_at) VALUES(?1, ?2, ?3, ?3)", params![p.title, p.thumbnail, now])?;
            let pid = tx.last_insert_rowid();
            for (i, id) in ids.iter().enumerate() {
                tx.execute("INSERT OR IGNORE INTO playlist_song(playlist_id, video_id, position, added_at) VALUES(?1, ?2, ?3, ?4)", params![pid, id, i as i64, now])?;
            }
            created += 1;
        }
        tx.commit()?;
        Ok(ImportReport { playlists_created: created, songs_imported: data.songs.len(), skipped_entries: skipped })
    }
}

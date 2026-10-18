//! Lyric search: an FTS5 index over the lines of every cached lyric ("find the song whose lyrics contain these words").
//! `lyric_line` holds the lines (source of truth for hits), `lyric_fts` is its external-content FTS5 index. Both are written
//! only through [`reindex`] / [`unindex`] from `lyrics.rs`, inside the same transaction as the `lyrics_cache` write.
//! Lines come from `lyrics-core`'s parser (LRC, enhanced LRC, TTML, plain), so the index matches what the lyrics screen shows.

use crate::Store;
use anyhow::Result;
use lyrics_core::lrc::Lyrics;
use rusqlite::{params, Connection};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// One song, its best-matching lyric line, and where to highlight inside that line.
#[derive(Debug, Clone, PartialEq)]
pub struct LyricHit {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub thumb: Option<String>,
    pub line: String,
    /// Line start in ms (0 for unsynced lyrics). The UI applies the per-song offset itself.
    pub start_ms: u64,
    /// Byte ranges inside `line` of the words that matched (case- and diacritic-insensitive, prefix on the last word).
    pub snippet_ranges: Vec<(usize, usize)>,
}

/// A "word" for both indexing and querying: letters, digits and combining marks (so Arabic harakat stay attached).
fn is_word(c: char) -> bool { c.is_alphanumeric() || is_combining_mark(c) }

/// Same folding as FTS5 `unicode61 remove_diacritics 2` (close enough for highlighting): NFD, drop marks, lower-case.
fn fold(s: &str) -> String { s.nfd().filter(|c| !is_combining_mark(*c)).flat_map(char::to_lowercase).collect() }

/// The query's words, as typed (runs of word characters that contain at least one letter or digit).
fn query_words(query: &str) -> Vec<&str> {
    query.split(|c: char| !is_word(c)).filter(|w| w.chars().any(|c| c.is_alphanumeric())).collect()
}

/// Byte ranges of the words in `line` that match the query: whole-word equality for every word except the last,
/// prefix match for the last (search-as-you-type).
fn highlight(line: &str, words: &[&str]) -> Vec<(usize, usize)> {
    let folded: Vec<String> = words.iter().map(|w| fold(w)).filter(|w| !w.is_empty()).collect();
    if folded.is_empty() { return vec![]; }
    let last = folded.len() - 1;
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in line.char_indices().chain(std::iter::once((line.len(), ' '))) {
        match (is_word(c), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                start = None;
                let token = fold(&line[s..i]);
                if !token.is_empty() && folded.iter().enumerate().any(|(k, q)| if k == last { token.starts_with(q.as_str()) } else { token == *q }) {
                    out.push((s, i));
                }
            }
            _ => {}
        }
    }
    out
}

/// FTS5 MATCH expression: every word folded and quoted (so user input can't be operators or syntax), AND-ed,
/// last word as a prefix.
fn match_expr(words: &[&str]) -> String {
    let folded: Vec<String> = words.iter().map(|w| fold(w)).filter(|w| !w.is_empty()).collect();
    let mut out = String::new();
    for (i, w) in folded.iter().enumerate() {
        if i > 0 { out.push(' '); }
        out.push('"');
        out.push_str(&w.replace('"', "\"\""));
        out.push('"');
        if i + 1 == folded.len() { out.push('*'); }
    }
    out
}

/// Remove every indexed line of `video_id` (FTS entries first, while the old text is still there).
pub(crate) fn unindex(db: &Connection, video_id: &str) -> Result<()> {
    db.execute("INSERT INTO lyric_fts(lyric_fts, rowid, text) SELECT 'delete', id, text FROM lyric_line WHERE video_id = ?1", [video_id])?;
    db.execute("DELETE FROM lyric_line WHERE video_id = ?1", [video_id])?;
    Ok(())
}

/// Replace the indexed lines of `video_id` with the lines of `body` (raw provider text; empty body = no lines).
pub(crate) fn reindex(db: &Connection, video_id: &str, body: &str, source: &str) -> Result<()> {
    unindex(db, video_id)?;
    if body.trim().is_empty() { return Ok(()); }
    for (i, l) in Lyrics::from_text(body, source).lines.iter().enumerate() {
        if l.instrumental || l.text.trim().is_empty() { continue; }
        let start_ms = (l.time.max(0.0) * 1000.0).round() as i64;
        let folded = fold(&l.text);
        db.execute("INSERT INTO lyric_line(video_id, line_index, start_ms, text, line) VALUES(?1, ?2, ?3, ?4, ?5)", params![video_id, i as i64, start_ms, folded, l.text])?;
        db.execute("INSERT INTO lyric_fts(rowid, text) VALUES(?1, ?2)", params![db.last_insert_rowid(), folded])?;
    }
    Ok(())
}

/// Drop the whole index (used when the lyrics cache is cleared).
pub(crate) fn clear(db: &Connection) -> Result<()> {
    db.execute("DELETE FROM lyric_line", [])?;
    db.execute("INSERT INTO lyric_fts(lyric_fts) VALUES('rebuild')", [])?; // external content is now empty, so this empties the index
    Ok(())
}

/// Index everything in `lyrics_cache` from scratch (migration / restore of an older backup).
pub(crate) fn rebuild_index(db: &Connection) -> Result<()> {
    clear(db)?;
    let rows: Vec<(String, String, String)> = {
        let mut st = db.prepare("SELECT video_id, source, body FROM lyrics_cache WHERE kind != 'none'")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        rows
    };
    for (vid, source, body) in rows { reindex(db, &vid, &body, &source)?; }
    Ok(())
}

impl Store {
    /// Full-text search over cached lyric lines: one hit per song (its best line), ranked by bm25, then most recently played.
    /// Empty or symbol-only queries return nothing without touching the database. Prefix match on the last word.
    pub fn search_lyrics(&self, query: &str, limit: usize) -> Result<Vec<LyricHit>> {
        let words = query_words(query);
        if words.is_empty() || limit == 0 { return Ok(vec![]); }
        let db = self.db();
        let mut st = db.prepare("WITH hits AS (
                SELECT l.video_id AS vid, l.line_index AS li, l.start_ms AS ms, l.line AS txt, bm25(lyric_fts) AS score
                FROM lyric_fts JOIN lyric_line l ON l.id = lyric_fts.rowid WHERE lyric_fts MATCH ?1
            ), best AS (
                SELECT vid, ms, txt, score, ROW_NUMBER() OVER (PARTITION BY vid ORDER BY score, li) AS rn FROM hits
            )
            SELECT b.vid, COALESCE(s.title, ''), COALESCE(s.artists, '[]'), s.thumb, b.txt, b.ms
            FROM best b LEFT JOIN song s ON s.video_id = b.vid
            WHERE b.rn = 1
            ORDER BY b.score, COALESCE((SELECT MAX(p.ts) FROM playback_event p WHERE p.video_id = b.vid), 0) DESC
            LIMIT ?2")?;
        let rows = st.query_map(params![match_expr(&words), limit as i64], |r| Ok((
            r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, String>(4)?, r.get::<_, i64>(5)?,
        )))?;
        let mut out = Vec::new();
        for row in rows {
            let (video_id, title, artists, thumb, line, start_ms) = row?;
            let artists: Vec<String> = serde_json::from_str(&artists).unwrap_or_default();
            let snippet_ranges = highlight(&line, &words);
            let title = if title.is_empty() { video_id.clone() } else { title };
            let artist = artists.join(", ");
            out.push(LyricHit { video_id, title, artist, thumb, line, start_ms: start_ms.max(0) as u64, snippet_ranges });
        }
        Ok(out)
    }
}

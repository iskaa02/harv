//! AutoEq headphone correction profiles (github.com/jaakkopasanen/AutoEq): index search + ParametricEQ fetch → `player_core::Parametric`.
//! GUI-free. The index is cached under `~/.cache/harv/autoeq-index.md` for 14 days.

use anyhow::{anyhow, bail, Context, Result};
use player_core::{ParaKind, Parametric};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

const RAW: &str = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results";
const INDEX_URL: &str = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results/INDEX.md";
const CACHE_TTL: Duration = Duration::from_secs(14 * 24 * 3600);
const SEARCH_CAP: usize = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    /// Results-relative directory, URL-decoded, e.g. "crinacle/711 in-ear/1Custom SA02".
    pub path: String,
    /// Text after " by ", e.g. "crinacle on 711" (empty if absent).
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub preamp_db: f32,
    pub filters: Vec<Parametric>,
}

/// All entries from INDEX.md (cached copy if fresher than 14 days, otherwise downloaded and cached).
pub async fn index(http: &reqwest::Client) -> Result<Vec<Entry>> {
    let path = cache_path();
    if let Ok(meta) = tokio::fs::metadata(&path).await {
        let fresh = meta.modified().ok().and_then(|m| m.elapsed().ok()).is_some_and(|age| age < CACHE_TTL);
        if fresh {
            if let Ok(text) = tokio::fs::read_to_string(&path).await { return Ok(parse_index(&text)); }
        }
    }
    let text = http.get(INDEX_URL).send().await?.error_for_status()?.text().await?;
    let entries = parse_index(&text);
    if entries.is_empty() { bail!("AutoEq index had no entries"); }
    if let Some(dir) = path.parent() { let _ = tokio::fs::create_dir_all(dir).await; }
    let _ = tokio::fs::write(&path, &text).await;
    Ok(entries)
}

/// Case-insensitive: every whitespace-separated word must occur in the name. Names starting with the query come first; at most 60 results.
pub fn search<'a>(entries: &'a [Entry], q: &str) -> Vec<&'a Entry> {
    let query = q.split_whitespace().map(str::to_lowercase).collect::<Vec<_>>().join(" ");
    let words: Vec<&str> = query.split(' ').filter(|w| !w.is_empty()).collect();
    let mut prefix = vec![];
    let mut rest = vec![];
    for e in entries {
        let name = e.name.to_lowercase();
        if !words.iter().all(|w| name.contains(w)) { continue; }
        if name.starts_with(&query) { prefix.push(e) } else { rest.push(e) }
    }
    prefix.into_iter().chain(rest).take(SEARCH_CAP).collect()
}

pub fn cache_path() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("harv").join("autoeq-index.md")
}

/// Fetches the ParametricEQ.txt of one entry. 404 = "no profile for <name>".
pub async fn profile(http: &reqwest::Client, e: &Entry) -> Result<Profile> {
    let dir: Vec<String> = e.path.split('/').map(encode_segment).collect();
    let file = encode_segment(&format!("{} ParametricEQ.txt", e.name));
    let url = format!("{RAW}/{}/{file}", dir.join("/"));
    let resp = http.get(&url).send().await.with_context(|| format!("fetching profile for {}", e.name))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND { bail!("no profile for {}", e.name); }
    let text = resp.error_for_status()?.text().await?;
    parse_profile(&text).with_context(|| format!("parsing profile for {}", e.name))
}

fn parse_index(text: &str) -> Vec<Entry> { text.lines().filter_map(parse_line).collect() }

/// `- [Name](./dir%20encoded/Name) by source`: the path ends at the last ")" followed by end-of-line or " by ".
fn parse_line(line: &str) -> Option<Entry> {
    let line = line.trim_end();
    let rest = line.strip_prefix("- [")?;
    let mid = rest.find("](./")?;
    let name = &rest[..mid];
    let after = &rest[mid + 4..];
    let end = after.match_indices(')').filter(|(i, _)| { let tail = &after[i + 1..]; tail.is_empty() || tail.starts_with(" by ") }).map(|(i, _)| i).last()?;
    let path = percent_decode(&after[..end]);
    let source = after[end + 1..].strip_prefix(" by ").unwrap_or("").trim().to_string();
    if name.is_empty() || path.is_empty() { return None; }
    Some(Entry { name: name.to_string(), path, source })
}

fn parse_profile(text: &str) -> Result<Profile> {
    let mut preamp_db = None;
    let mut filters = vec![];
    for line in text.lines().map(str::trim) {
        if let Some(v) = line.strip_prefix("Preamp:") {
            preamp_db = v.split_whitespace().next().and_then(|t| t.parse::<f32>().ok());
            continue;
        }
        let Some((_, rest)) = line.strip_prefix("Filter ").and_then(|r| r.split_once(':')) else { continue };
        let t: Vec<&str> = rest.split_whitespace().collect();
        if t.first() != Some(&"ON") { continue; }
        let kind = match t.get(1).copied() {
            Some("PK") => ParaKind::Pk,
            Some("LS") | Some("LSC") => ParaKind::Lsc,
            Some("HS") | Some("HSC") => ParaKind::Hsc,
            _ => continue,
        };
        let val = |key: &str| t.iter().position(|x| *x == key).and_then(|i| t.get(i + 1)).and_then(|v| v.parse::<f32>().ok());
        let (Some(fc), Some(gain_db), Some(q)) = (val("Fc"), val("Gain"), val("Q")) else { continue };
        filters.push(Parametric { kind, fc, gain_db, q });
    }
    if preamp_db.is_none() && filters.is_empty() { return Err(anyhow!("no Preamp or Filter lines")); }
    Ok(Profile { preamp_db: preamp_db.unwrap_or(0.0), filters })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%' && i + 2 < b.len()).then(|| std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())).flatten();
        match hex {
            Some(v) => { out.push(v); i += 3; }
            None => { out.push(b[i]); i += 1; }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~()".contains(&byte) { out.push(byte as char) } else { out.push_str(&format!("%{byte:02X}")) }
    }
    out
}

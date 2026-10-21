//! Stream URL resolution through `yt-dlp -J`, with an in-memory cache keyed by video id.
//! Quality ↔ itag mapping mirrors SimpMusic (`Config.kt`): 250 low, 251 medium, 774 high opus, 141 high aac.

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Quality { Low, Medium, HighOpus, HighAac }

impl Quality {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s { "low" => Self::Low, "medium" => Self::Medium, "high-opus" | "high" => Self::HighOpus, "high-aac" => Self::HighAac, _ => return None })
    }

    /// yt-dlp format selector: chosen itag, its 256 kbps twin (774↔141), then progressively more generic fallbacks.
    fn selector(self) -> &'static str {
        match self {
            Self::Low => "250/251/140/bestaudio",
            Self::Medium => "251/250/140/bestaudio",
            Self::HighOpus => "774/141/251/250/140/bestaudio",
            Self::HighAac => "141/774/251/250/140/bestaudio",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Stream {
    pub video_id: String,
    pub url: String,
    pub itag: String,
    pub codec: String,
    pub ext: String,
    pub bitrate_kbps: Option<f64>,
    pub duration_s: Option<f64>,
    /// Unix seconds after which the URL stops working.
    pub expires_at: u64,
}

#[derive(Deserialize)]
struct Info {
    url: Option<String>,
    format_id: Option<String>,
    acodec: Option<String>,
    ext: Option<String>,
    abr: Option<f64>,
    duration: Option<f64>,
}

pub(crate) struct StreamResolver {
    cache: Mutex<HashMap<(String, Quality), Stream>>,
    /// Downloaded copies: video id → file. Played from disk while the file exists; never touches yt-dlp.
    local: Mutex<HashMap<String, PathBuf>>,
    /// Netscape cookies.txt passed to yt-dlp as `--cookies` (YouTube Music login: Premium quality, age-gated songs).
    cookies: Mutex<Option<PathBuf>>,
    ytdlp: String,
}

fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

/// yt-dlp binary: `YTM_YTDLP` env override, else `yt-dlp` on PATH.
pub(crate) fn ytdlp_bin() -> String { std::env::var("YTM_YTDLP").unwrap_or_else(|_| "yt-dlp".into()) }

fn expiry_of(url: &str) -> u64 {
    url.split(['?', '&']).find_map(|kv| kv.strip_prefix("expire=")).and_then(|v| v.parse().ok()).unwrap_or_else(|| now() + 5 * 3600)
}

impl StreamResolver {
    pub(crate) fn new() -> Self {
        Self { cache: Mutex::new(HashMap::new()), local: Mutex::new(HashMap::new()), cookies: Mutex::new(None), ytdlp: ytdlp_bin() }
    }

    pub(crate) fn set_cookies(&self, path: Option<PathBuf>) { *self.cookies.lock().unwrap() = path; }
    pub(crate) fn cookies(&self) -> Option<PathBuf> { self.cookies.lock().unwrap().clone() }

    pub(crate) fn register_local(&self, video_id: &str, path: PathBuf) { self.local.lock().unwrap().insert(video_id.to_string(), path); }
    pub(crate) fn unregister_local(&self, video_id: &str) { self.local.lock().unwrap().remove(video_id); }

    /// A registered download, as a `Stream` whose url is the file path (mpv plays plain paths). `None` once the file is gone.
    fn local(&self, video_id: &str) -> Option<Stream> {
        let path = self.local.lock().unwrap().get(video_id)?.clone();
        if !path.is_file() { return None; }
        Some(Stream {
            video_id: video_id.to_string(), url: path.to_string_lossy().into_owned(), itag: "local".into(), codec: String::new(),
            ext: path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_string(), bitrate_kbps: None, duration_s: None, expires_at: u64::MAX,
        })
    }

    /// Non-blocking: a downloaded copy, else the cached stream if its URL stays valid for at least another minute.
    pub(crate) fn cached(&self, video_id: &str, quality: Quality) -> Option<Stream> {
        if let Some(s) = self.local(video_id) { return Some(s); }
        self.cache.lock().unwrap().get(&(video_id.to_string(), quality)).filter(|s| s.expires_at > now() + 60).cloned()
    }

    pub(crate) async fn resolve(&self, video_id: &str, quality: Quality) -> Result<Stream> {
        if let Some(s) = self.cached(video_id, quality) { return Ok(s); }
        let s = self.run(video_id, quality).await?;
        self.cache.lock().unwrap().insert((video_id.to_string(), quality), s.clone());
        Ok(s)
    }

    pub(crate) fn invalidate(&self, video_id: &str) { self.cache.lock().unwrap().retain(|(id, _), _| id != video_id); }

    async fn run(&self, video_id: &str, quality: Quality) -> Result<Stream> {
        let mut cmd = Command::new(&self.ytdlp);
        cmd.kill_on_drop(true);
        cmd.args(["-J", "--no-playlist", "--no-warnings", "-f", quality.selector()]);
        if let Some(c) = self.cookies() { cmd.arg("--cookies").arg(c); }
        let out = cmd.arg(format!("https://music.youtube.com/watch?v={video_id}"))
            .output().await.with_context(|| format!("failed to run `{}` (set YTM_YTDLP to override)", self.ytdlp))?;
        if !out.status.success() { bail!("yt-dlp failed: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("unknown error")); }
        let info: Info = serde_json::from_slice(&out.stdout).context("unreadable yt-dlp output")?;
        let url = info.url.ok_or_else(|| anyhow!("yt-dlp returned no direct URL"))?;
        Ok(Stream {
            video_id: video_id.to_string(),
            itag: info.format_id.unwrap_or_default(),
            codec: info.acodec.unwrap_or_default(),
            ext: info.ext.unwrap_or_default(),
            bitrate_kbps: info.abr,
            duration_s: info.duration,
            expires_at: expiry_of(&url),
            url,
        })
    }
}

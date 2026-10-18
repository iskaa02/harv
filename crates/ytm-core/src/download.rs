//! Audio download through `yt-dlp` for offline copies. Files land at `<dest_dir>/<videoId>.<ext>`; an existing one is reused.

use crate::*;
use crate::stream::ytdlp_bin;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;

impl Ytm {
    pub async fn download(&self, video_id: &str, dest_dir: &Path) -> Result<PathBuf> {
        std::fs::create_dir_all(dest_dir).with_context(|| format!("cannot create {}", dest_dir.display()))?;
        if let Some(existing) = std::fs::read_dir(dest_dir)?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.file_stem().and_then(|s| s.to_str()) == Some(video_id)) { return Ok(existing); }
        let bin = ytdlp_bin();
        let mut cmd = Command::new(&bin);
        cmd.args(["-f", "bestaudio[ext=m4a]/bestaudio", "--no-playlist", "--no-warnings", "-o"]).arg(dest_dir.join("%(id)s.%(ext)s")).args(["--print", "after_move:filepath"]);
        if let Some(c) = self.streams.cookies() { cmd.arg("--cookies").arg(c); }
        let out = cmd.arg(format!("https://music.youtube.com/watch?v={video_id}"))
            .output().await.with_context(|| format!("failed to run `{bin}` (set YTM_YTDLP to override)"))?;
        if !out.status.success() { bail!("yt-dlp failed: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("unknown error")); }
        let path = String::from_utf8_lossy(&out.stdout).lines().last().map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from);
        path.ok_or_else(|| anyhow::anyhow!("yt-dlp printed no output path"))
    }
}

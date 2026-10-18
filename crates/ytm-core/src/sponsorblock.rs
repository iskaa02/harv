//! SponsorBlock skip segments (`sponsor.ajay.app`): time ranges to skip inside a video (e.g. `music_offtopic`, `sponsor`, `intro`).

use crate::*;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub category: String,
}

#[derive(Deserialize)]
struct RawSegment {
    category: String,
    segment: [f64; 2],
}

impl Ytm {
    /// Segments to skip for a video, sorted by start; ranges under 1 s are dropped. No submissions (HTTP 404) is an empty list.
    pub async fn skip_segments(&self, video_id: &str, categories: &[&str]) -> Result<Vec<Segment>> {
        let mut q: Vec<(&str, &str)> = vec![("videoID", video_id), ("actionType", "skip")];
        q.extend(categories.iter().map(|c| ("category", *c)));
        let res = self.http.get("https://sponsor.ajay.app/api/skipSegments").query(&q).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND { return Ok(vec![]); }
        let raw: Vec<RawSegment> = res.error_for_status()?.json().await.context("unreadable SponsorBlock response")?;
        let mut out: Vec<Segment> = raw.into_iter().map(|r| Segment { start: r.segment[0], end: r.segment[1], category: r.category }).filter(|s| s.end - s.start >= 1.0).collect();
        out.sort_by(|a, b| a.start.total_cmp(&b.start));
        Ok(out)
    }
}

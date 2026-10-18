//! Radio ("Start radio" / endless queue): InnerTube `next` for `RDAMVM<videoId>`. `ytmapi-rs` returns only the first page and hides the
//! continuation token, which is what lets a radio stay on one coherent station instead of re-seeding from its own tail.
//! Reference: SimpMusic `YouTube.next()` / `SongRepositoryImpl.getContinueTrack`.

use crate::{parse_duration, Track, Ytm};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

/// One page of a radio (~50 tracks). Pass `continuation` back to [`Ytm::radio_page`] for the next one; `None` = the station is exhausted.
#[derive(Debug, Clone, Default)]
pub struct RadioPage {
    pub tracks: Vec<Track>,
    pub continuation: Option<String>,
}

fn runs_text(v: &Value) -> String { v["runs"].as_array().map(|r| r.iter().filter_map(|x| x["text"].as_str()).collect()).unwrap_or_default() }

fn page_type(run: &Value) -> Option<&str> { run.pointer("/navigationEndpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType").and_then(Value::as_str) }

/// Random uploads (covers, mashups, slowed edits) — the radio is better without them; official videos and audio tracks stay.
fn is_ugc(r: &Value) -> bool { r.pointer("/navigationEndpoint/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType").and_then(Value::as_str) == Some("MUSIC_VIDEO_TYPE_UGC") }

fn track(r: &Value) -> Option<Track> {
    let video_id = r["videoId"].as_str()?.to_string();
    let runs = r.pointer("/longBylineText/runs").and_then(Value::as_array);
    // Artist and album are the runs that link somewhere; the rest is " • ", view counts, years.
    let named = |kind: &str| -> Vec<String> { runs.map(|rs| rs.iter().filter(|x| page_type(x).is_some_and(|t| t.ends_with(kind))).filter_map(|x| x["text"].as_str().map(String::from)).collect()).unwrap_or_default() };
    let mut artists = named("_ARTIST");
    if artists.is_empty() { artists = named("_USER_CHANNEL"); }
    if artists.is_empty() { artists = vec![runs_text(&r["shortBylineText"])].into_iter().filter(|s| !s.is_empty()).collect(); }
    let thumbs = r.pointer("/thumbnail/thumbnails").and_then(Value::as_array);
    Some(Track {
        video_id, title: runs_text(&r["title"]), artists, album: named("_ALBUM").into_iter().next(),
        duration_s: parse_duration(&runs_text(&r["lengthText"])),
        thumb: thumbs.and_then(|t| t.iter().max_by_key(|x| x["width"].as_u64().unwrap_or(0))).and_then(|x| x["url"].as_str().map(String::from)),
    })
}

impl Ytm {
    /// First page of the radio of `video_id` (`token = None`), or the page after `token`.
    pub async fn radio_page(&self, video_id: &str, token: Option<&str>) -> Result<RadioPage> {
        let context = json!({ "client": { "clientName": "WEB_REMIX", "clientVersion": "1.20250210.01.00", "hl": "en", "gl": "US" } });
        let body = match token {
            Some(t) => json!({ "context": context, "continuation": t }),
            None => json!({ "context": context, "videoId": video_id, "playlistId": format!("RDAMVM{video_id}"), "isAudioOnly": true, "tunerSettingValue": "AUTOMIX_SETTING_NORMAL" }),
        };
        let v: Value = self.http.post("https://music.youtube.com/youtubei/v1/next?prettyPrint=false")
            .header("Origin", "https://music.youtube.com").json(&body).send().await?.error_for_status()?.json().await.context("radio response")?;
        let panel = v.pointer("/continuationContents/playlistPanelContinuation")
            .or_else(|| v.pointer("/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs/0/tabRenderer/content/musicQueueRenderer/content/playlistPanelRenderer"))
            .ok_or_else(|| anyhow!("unexpected radio layout"))?;
        let tracks = panel["contents"].as_array().map(|c| c.iter().filter_map(|i| {
            let r = i.get("playlistPanelVideoRenderer").or_else(|| i.pointer("/playlistPanelVideoWrapperRenderer/primaryRenderer/playlistPanelVideoRenderer"))?;
            if is_ugc(r) && !(token.is_none() && r["videoId"] == video_id) { return None; }
            track(r)
        }).collect()).unwrap_or_default();
        let continuation = panel.pointer("/continuations/0/nextRadioContinuationData/continuation").or_else(|| panel.pointer("/continuations/0/nextContinuationData/continuation")).and_then(Value::as_str).map(String::from);
        Ok(RadioPage { tracks, continuation })
    }

    /// First page of the radio, tracks only (the seed itself comes first).
    pub async fn radio(&self, video_id: &str) -> Result<Vec<Track>> { Ok(self.radio_page(video_id, None).await?.tracks) }
}

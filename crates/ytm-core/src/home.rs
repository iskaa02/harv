//! Home feed (`FEmusic_home`) — `ytmapi-rs` has no query for it, so this posts to InnerTube directly with the WEB_REMIX client
//! and keeps only what a shelf UI needs. Reference: SimpMusic `YouTube.kt` `browse()`.

use crate::Ytm;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HomeItem {
    pub title: String,
    pub subtitle: String,
    pub video_id: Option<String>,
    pub playlist_id: Option<String>,
    /// Album / artist / playlist browse id (`MPRE…`, `UC…`, `VL…`).
    pub browse_id: Option<String>,
    pub thumb: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HomeShelf {
    pub title: String,
    pub items: Vec<HomeItem>,
}

fn text(v: &Value) -> String {
    v["runs"].as_array().map(|r| r.iter().filter_map(|x| x["text"].as_str()).collect::<String>()).unwrap_or_default()
}

fn thumb(v: &Value) -> Option<String> {
    let t = v.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails").or_else(|| v.pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails"))?;
    t.as_array()?.last()?["url"].as_str().map(String::from)
}

fn item(v: &Value) -> Option<HomeItem> {
    if let Some(r) = v.get("musicTwoRowItemRenderer") {
        let nav = &r["navigationEndpoint"];
        return Some(HomeItem {
            title: text(&r["title"]),
            subtitle: text(&r["subtitle"]),
            video_id: nav.pointer("/watchEndpoint/videoId").and_then(Value::as_str).map(String::from),
            playlist_id: nav.pointer("/watchEndpoint/playlistId").or_else(|| nav.pointer("/watchPlaylistEndpoint/playlistId")).and_then(Value::as_str).map(String::from),
            browse_id: nav.pointer("/browseEndpoint/browseId").and_then(Value::as_str).map(String::from),
            thumb: thumb(r),
        });
    }
    let r = v.get("musicResponsiveListItemRenderer")?;
    let col = |i: usize| -> String { r.pointer(&format!("/flexColumns/{i}/musicResponsiveListItemFlexColumnRenderer/text")).map(text).unwrap_or_default() };
    Some(HomeItem {
        title: col(0),
        subtitle: col(1),
        video_id: r.pointer("/playlistItemData/videoId").and_then(Value::as_str).map(String::from),
        playlist_id: None,
        browse_id: None,
        thumb: thumb(r),
    })
}

impl Ytm {
    pub async fn home(&self) -> Result<Vec<HomeShelf>> {
        let body = json!({
            "context": { "client": { "clientName": "WEB_REMIX", "clientVersion": "1.20250210.01.00", "hl": "en", "gl": "US" } },
            "browseId": "FEmusic_home",
        });
        let v: Value = self.http.post("https://music.youtube.com/youtubei/v1/browse?prettyPrint=false")
            .header("Origin", "https://music.youtube.com").json(&body).send().await?.error_for_status()?.json().await.context("home response")?;
        let sections = v.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents").and_then(Value::as_array).context("unexpected home layout")?;
        Ok(sections.iter().filter_map(|s| {
            let shelf = s.get("musicCarouselShelfRenderer")?;
            let title = text(shelf.pointer("/header/musicCarouselShelfBasicHeaderRenderer/title")?);
            let items: Vec<HomeItem> = shelf["contents"].as_array()?.iter().filter_map(item).collect();
            (!items.is_empty()).then_some(HomeShelf { title, items })
        }).collect())
    }
}

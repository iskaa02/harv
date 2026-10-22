//! GUI-free YouTube Music access: browse/search through `ytmapi-rs`, stream URLs through a `yt-dlp` subprocess.
//! The public API uses our own plain types so front-ends never depend on `ytmapi-rs`.

mod browse;
mod download;
mod home;
mod radio;
mod sponsorblock;
mod stream;

pub use browse::{Podcast, PodcastRef};
pub use home::{HomeItem, HomeShelf};
pub use radio::RadioPage;
pub use sponsorblock::Segment;
pub use stream::{Quality, Stream};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Track {
    pub video_id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_s: Option<u32>,
    pub thumb: Option<String>,
}

impl Track {
    pub fn artist_line(&self) -> String { self.artists.join(", ") }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AlbumRef {
    pub browse_id: String,
    pub title: String,
    pub artist: String,
    pub year: String,
    pub thumb: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArtistRef {
    pub channel_id: String,
    pub name: String,
    pub subscribers: Option<String>,
    pub thumb: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaylistRef {
    pub playlist_id: String,
    pub title: String,
    pub author: String,
    pub thumb: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchResults {
    pub songs: Vec<Track>,
    pub videos: Vec<Track>,
    pub albums: Vec<AlbumRef>,
    pub artists: Vec<ArtistRef>,
    pub playlists: Vec<PlaylistRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind { Songs, Videos, Albums, Artists, Playlists }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Album {
    pub title: String,
    pub artists: Vec<String>,
    pub year: String,
    pub thumb: Option<String>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Artist {
    pub channel_id: String,
    pub name: String,
    pub subscribers: Option<String>,
    pub description: Option<String>,
    pub thumb: Option<String>,
    pub songs: Vec<Track>,
    pub albums: Vec<AlbumRef>,
    pub singles: Vec<AlbumRef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaylistPage {
    pub playlist_id: String,
    pub title: String,
    pub author: String,
    pub thumb: Option<String>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mood {
    pub section: String,
    pub title: String,
    /// Opaque; pass back to [`Ytm::mood_playlists`].
    pub params: String,
}

/// Upper bounds for every request: a dead or crawling connection must turn into an error (with a Retry button) rather than a spinner.
pub(crate) const NET_CONNECT: std::time::Duration = std::time::Duration::from_secs(8);
pub(crate) const NET_TOTAL: std::time::Duration = std::time::Duration::from_secs(20);

/// Run a `ytmapi-rs` call under [`NET_TOTAL`] (that crate builds its own HTTP client, so there is no timeout to configure).
pub(crate) async fn net<T, E: Into<anyhow::Error>>(f: impl std::future::Future<Output = std::result::Result<T, E>>) -> Result<T> {
    match tokio::time::timeout(NET_TOTAL, f).await {
        Ok(r) => r.map_err(Into::into),
        Err(_) => Err(anyhow::anyhow!("request timed out")),
    }
}

pub struct Ytm {
    /// Created on first use: building it makes a network request, and the app must still start offline.
    api: tokio::sync::OnceCell<ytmapi_rs::YtMusic<ytmapi_rs::auth::noauth::NoAuthToken>>,
    pub(crate) http: reqwest::Client,
    pub(crate) streams: Arc<stream::StreamResolver>,
}

impl Ytm {
    pub async fn new() -> Result<Self> {
        Ok(Self {
            api: tokio::sync::OnceCell::new(),
            http: reqwest::Client::builder().user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0").connect_timeout(NET_CONNECT).timeout(NET_TOTAL).build()?,
            streams: Arc::new(stream::StreamResolver::new()),
        })
    }

    pub(crate) async fn api(&self) -> Result<&ytmapi_rs::YtMusic<ytmapi_rs::auth::noauth::NoAuthToken>> {
        Ok(self.api.get_or_try_init(|| net(ytmapi_rs::YtMusic::new_unauthenticated())).await?)
    }

    /// Resolve a playable audio URL (cached until shortly before it expires).
    pub async fn stream(&self, video_id: &str, quality: Quality) -> Result<Stream> { self.streams.resolve(video_id, quality).await }
    /// Non-blocking lookup: the cached stream if it is still valid for over a minute, else `None` (never hits the network).
    pub fn cached_stream(&self, video_id: &str, quality: Quality) -> Option<Stream> { self.streams.cached(video_id, quality) }
    /// Play `video_id` from a downloaded file from now on (`stream`/`cached_stream` return the path while it exists).
    pub fn register_local(&self, video_id: &str, path: std::path::PathBuf) { self.streams.register_local(video_id, path) }
    pub fn unregister_local(&self, video_id: &str) { self.streams.unregister_local(video_id) }
    /// Netscape cookies.txt for yt-dlp (`--cookies`) from the next resolve/download on; `None` = anonymous. Cached URLs are kept.
    pub fn set_cookies_file(&self, path: Option<std::path::PathBuf>) { self.streams.set_cookies(path) }
    /// Forget a cached URL (call after a playback error, then `stream` again).
    pub fn invalidate_stream(&self, video_id: &str) { self.streams.invalidate(video_id) }
    /// Resolve in the background so the next track starts instantly.
    pub fn prefetch_stream(&self, video_id: &str, quality: Quality) {
        let (streams, id) = (self.streams.clone(), video_id.to_string());
        tokio::spawn(async move { let _ = streams.resolve(&id, quality).await; });
    }
    /// Like [`Self::prefetch_stream`] but after `delay`, so the track that just started gets the bandwidth to fill its buffer first.
    pub fn prefetch_stream_after(&self, video_id: &str, quality: Quality, delay: std::time::Duration) {
        let (streams, id) = (self.streams.clone(), video_id.to_string());
        tokio::spawn(async move { tokio::time::sleep(delay).await; let _ = streams.resolve(&id, quality).await; });
    }
}

pub(crate) fn parse_duration(s: &str) -> Option<u32> {
    let mut total = 0u32;
    for part in s.trim().split(':') { total = total * 60 + part.trim().parse::<u32>().ok()?; }
    Some(total)
}

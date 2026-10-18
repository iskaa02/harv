//! Search / browse through `ytmapi-rs`, mapped into our own types.

use crate::*;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use ytmapi_rs::common::{AlbumID, ArtistChannelID, PlaylistID, PodcastID, Thumbnail, YoutubeID};
use ytmapi_rs::parse as p;

fn best(t: &[Thumbnail]) -> Option<String> { t.iter().max_by_key(|t| t.width).map(|t| t.url.clone()) }

fn split_artists(s: &str) -> Vec<String> { s.split(" & ").flat_map(|x| x.split(", ")).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect() }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PodcastRef { pub podcast_id: String, pub title: String, pub publisher: String, pub thumb: Option<String> }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Podcast { pub podcast_id: String, pub title: String, pub description: Option<String>, pub thumb: Option<String>, pub episodes: Vec<Track> }

#[derive(Deserialize)]
struct ChartList { data: Vec<ChartPlaylist> }
#[derive(Deserialize)]
struct ChartPlaylist { country: String, #[serde(rename = "youtubePlaylistId")] playlist_id: String }

/// Episode durations read "2 hr 2 min" / "30 min" (not `h:mm:ss`).
fn parse_words_duration(s: &str) -> Option<u32> {
    let (mut total, mut num) = (0u32, None);
    for w in s.split_whitespace() {
        if let Ok(n) = w.parse::<u32>() { num = Some(n); continue; }
        total += num.take()? * match w.trim_end_matches('s') { "hr" | "hour" => 3600, "min" | "minute" => 60, "sec" | "second" => 1, _ => return None };
    }
    (total > 0).then_some(total)
}

fn album_ref(a: &p::AlbumResult) -> AlbumRef {
    AlbumRef { browse_id: a.album_id.get_raw().to_string(), title: a.title.clone(), artist: String::new(), year: a.year.clone(), thumb: best(&a.thumbnails) }
}

impl Ytm {
    pub async fn suggest(&self, q: &str) -> Result<Vec<String>> {
        let api = self.api().await?;
        Ok(api.get_search_suggestions(q).await?.into_iter().map(|s| s.runs.into_iter().map(|r| r.take_text()).collect()).collect())
    }

    pub async fn search(&self, q: &str, kind: SearchKind) -> Result<SearchResults> {
        let api = self.api().await?;
        let mut out = SearchResults::default();
        match kind {
            SearchKind::Songs => out.songs = api.search_songs(q).await?.into_iter().map(|s| Track {
                video_id: s.video_id.get_raw().to_string(), title: s.title, artists: split_artists(&s.artist), album: s.album.map(|a| a.name),
                duration_s: parse_duration(&s.duration), thumb: best(&s.thumbnails),
            }).collect(),
            SearchKind::Videos => out.videos = api.search_videos(q).await?.into_iter().filter_map(|v| match v {
                p::SearchResultVideo::Video { title, channel_name, video_id, length, thumbnails, .. } => Some(Track {
                    video_id: video_id.get_raw().to_string(), title, artists: vec![channel_name], album: None, duration_s: parse_duration(&length), thumb: best(&thumbnails),
                }),
                _ => None,
            }).collect(),
            SearchKind::Albums => out.albums = api.search_albums(q).await?.into_iter().map(|a| AlbumRef {
                browse_id: a.album_id.get_raw().to_string(), title: a.title, artist: a.artist, year: a.year, thumb: best(&a.thumbnails),
            }).collect(),
            SearchKind::Artists => out.artists = api.search_artists(q).await?.into_iter().map(|a| ArtistRef {
                channel_id: a.browse_id.get_raw().to_string(), name: a.artist, subscribers: a.subscribers, thumb: best(&a.thumbnails),
            }).collect(),
            SearchKind::Playlists => {
                out.playlists = api.search_featured_playlists(q).await?.into_iter().map(|a| PlaylistRef {
                    playlist_id: a.playlist_id.get_raw().to_string(), title: a.title, author: a.author, thumb: best(&a.thumbnails),
                }).collect();
                out.playlists.extend(api.search_community_playlists(q).await?.into_iter().filter_map(|a| match a {
                    p::SearchResultPlaylist::Community(a) => Some(PlaylistRef { playlist_id: a.playlist_id.get_raw().to_string(), title: a.title, author: a.author, thumb: best(&a.thumbnails) }),
                    p::SearchResultPlaylist::Featured(a) => Some(PlaylistRef { playlist_id: a.playlist_id.get_raw().to_string(), title: a.title, author: a.author, thumb: best(&a.thumbnails) }),
                    _ => None,
                }));
            }
        }
        Ok(out)
    }

    pub async fn album(&self, browse_id: &str) -> Result<Album> {
        let api = self.api().await?;
        let a = api.get_album(AlbumID::from_raw(browse_id)).await?;
        let artists: Vec<String> = a.artists.iter().map(|x| x.name.clone()).collect();
        let thumb = best(&a.thumbnails);
        Ok(Album {
            title: a.title, year: a.year, thumb: thumb.clone(), artists: artists.clone(),
            tracks: a.tracks.into_iter().map(|t| Track {
                video_id: t.video_id.get_raw().to_string(), title: t.title, artists: artists.clone(), album: None, duration_s: parse_duration(&t.duration), thumb: thumb.clone(),
            }).collect(),
        })
    }

    pub async fn artist(&self, channel_id: &str) -> Result<Artist> {
        let api = self.api().await?;
        let a = api.get_artist(ArtistChannelID::from_raw(channel_id)).await?;
        let r = a.top_releases;
        Ok(Artist {
            channel_id: a.channel_id.get_raw().to_string(), name: a.name, subscribers: a.subscribers, description: a.description, thumb: best(&a.thumbnails),
            songs: r.songs.map(|s| s.results.into_iter().map(|t| Track {
                video_id: t.video_id.get_raw().to_string(), title: t.title, artists: t.artists.into_iter().map(|x| x.name).collect(), album: Some(t.album.name), duration_s: None, thumb: None,
            }).collect()).unwrap_or_default(),
            albums: r.albums.map(|a| a.results.iter().map(album_ref).collect()).unwrap_or_default(),
            singles: r.singles.map(|a| a.results.iter().map(album_ref).collect()).unwrap_or_default(),
        })
    }

    pub async fn playlist(&self, playlist_id: &str) -> Result<PlaylistPage> {
        let api = self.api().await?;
        // Browse ids need the `VL` prefix; the home/mood lists already carry it, watch-page ids (`RDCLAK…`, `PL…`) do not.
        let playlist_id = if playlist_id.starts_with("VL") { playlist_id.to_string() } else { format!("VL{playlist_id}") };
        let id = || PlaylistID::from_raw(playlist_id.clone());
        let (d, items) = tokio::try_join!(api.get_playlist_details(id()), api.get_playlist_tracks(id()))?;
        Ok(PlaylistPage {
            playlist_id: playlist_id.trim_start_matches("VL").to_string(), title: d.title, author: d.author, thumb: best(&d.thumbnails),
            tracks: items.into_iter().filter_map(|i| match i {
                p::PlaylistItem::Song(s) => Some(Track {
                    video_id: s.video_id.get_raw().to_string(), title: s.title, artists: s.artists.into_iter().map(|a| a.name).collect(), album: Some(s.album.name),
                    duration_s: parse_duration(&s.duration), thumb: best(&s.thumbnails),
                }),
                p::PlaylistItem::Video(v) => Some(Track {
                    video_id: v.video_id.get_raw().to_string(), title: v.title, artists: vec![v.channel_name], album: None, duration_s: parse_duration(&v.duration), thumb: best(&v.thumbnails),
                }),
                _ => None,
            }).collect(),
        })
    }

    pub async fn search_podcasts(&self, q: &str) -> Result<Vec<PodcastRef>> {
        let api = self.api().await?;
        Ok(api.search_podcasts(q).await?.into_iter().map(|p| PodcastRef {
            podcast_id: p.podcast_id.get_raw().to_string(), title: p.title, publisher: p.publisher, thumb: best(&p.thumbnails),
        }).collect())
    }

    /// Episodes come back as `Track`s; `video_id` is the playable id (plays through `stream` like any video).
    pub async fn podcast(&self, podcast_id: &str) -> Result<Podcast> {
        let api = self.api().await?;
        let p = api.get_podcast(PodcastID::from_raw(podcast_id)).await?;
        // The podcast page has no cover of its own; the first episode's art stands in for it.
        let thumb = p.episodes.first().and_then(|e| best(&e.thumbnails));
        Ok(Podcast {
            podcast_id: podcast_id.to_string(), title: p.title.clone(), description: Some(p.description).filter(|d| !d.is_empty()), thumb,
            episodes: p.episodes.into_iter().map(|e| Track {
                // Episode ids are `MPED` + the playable video id; only the video id resolves in yt-dlp.
                video_id: e.episode_id.get_raw().strip_prefix("MPED").unwrap_or(e.episode_id.get_raw()).to_string(), title: e.title, artists: vec![p.title.clone()],
                album: None, duration_s: parse_words_duration(&e.total_duration), thumb: best(&e.thumbnails),
            }).collect(),
        })
    }

    /// SimpMusic's "Trending Weekly" chart for a country code (`global`, `us`, `vn`, `it`, `id`, `br`, `mx`, `es`).
    pub async fn chart(&self, region: &str) -> Result<Vec<Track>> {
        let list: ChartList = self.http.get("https://chart.simpmusic.org/api/playlists").send().await?.error_for_status()?.json().await?;
        let pl = list.data.into_iter().find(|p| p.country.eq_ignore_ascii_case(region)).ok_or_else(|| anyhow!("no chart for region {region:?}"))?;
        Ok(self.playlist(&pl.playlist_id).await?.tracks)
    }

    pub async fn moods(&self) -> Result<Vec<Mood>> {
        let api = self.api().await?;
        Ok(api.get_mood_categories().await?.into_iter().flat_map(|s| {
            let section = s.section_name;
            s.mood_categories.into_iter().map(move |m| Mood { section: section.clone(), title: m.title, params: m.params.get_raw().to_string() })
        }).collect())
    }

    pub async fn mood_playlists(&self, params: &str) -> Result<Vec<PlaylistRef>> {
        let api = self.api().await?;
        let params = ytmapi_rs::common::MoodCategoryParams::from_raw(params.to_string());
        Ok(api.get_mood_playlists(params).await?.into_iter().flat_map(|c| c.playlists).map(|a| PlaylistRef {
            playlist_id: a.playlist_id.get_raw().to_string(), title: a.title, author: a.author, thumb: best(&a.thumbnails),
        }).collect())
    }
}

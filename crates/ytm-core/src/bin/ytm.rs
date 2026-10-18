//! Manual test harness for `ytm-core`: `ytm <command> …` (run without arguments for the list).

use anyhow::{bail, Result};
use ytm_core::*;

const USAGE: &str = "usage: ytm <command>
  search <q> [--kind songs|videos|albums|artists|playlists]   suggest <q>
  album <browseId>   artist <channelId>   playlist <id>   radio <videoId>
  moods   mood <params>   home   chart <region>
  podcasts <q>   podcast <podcastId>
  stream <videoId> [--quality low|medium|high-opus|high-aac]   play <q>   download <videoId> <dir>";

fn mins(s: Option<u32>) -> String { s.map(|s| format!("{}:{:02}", s / 60, s % 60)).unwrap_or_else(|| "-:--".into()) }

fn tracks(t: &[Track]) { for (i, t) in t.iter().enumerate() { println!("{:>3}  {:<40.40}  {:<28.28}  {}  {}", i + 1, t.title, t.artist_line(), mins(t.duration_s), t.video_id); } }
fn albums(a: &[AlbumRef]) { for a in a { println!("  {:<40.40}  {:<24.24}  {:<5}  {}", a.title, a.artist, a.year, a.browse_id); } }

fn flag(args: &[String], name: &str) -> Option<String> { args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned() }

fn positional(args: &[String]) -> String {
    let mut out = vec![]; let mut skip = false;
    for a in args { if skip { skip = false } else if a.starts_with("--") { skip = true } else { out.push(a.as_str()) } }
    out.join(" ")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else { bail!("{USAGE}") };
    let rest = &args[1..];
    let arg = positional(rest);
    let yt = Ytm::new().await?;
    let quality = flag(rest, "--quality").map(|q| Quality::parse(&q).ok_or_else(|| anyhow::anyhow!("unknown quality {q}"))).transpose()?.unwrap_or(Quality::HighOpus);

    match cmd {
        "search" => {
            let kind = match flag(rest, "--kind").as_deref().unwrap_or("songs") {
                "songs" => SearchKind::Songs, "videos" => SearchKind::Videos, "albums" => SearchKind::Albums, "artists" => SearchKind::Artists, "playlists" => SearchKind::Playlists,
                k => bail!("unknown kind {k}"),
            };
            let r = yt.search(&arg, kind).await?;
            tracks(&r.songs); tracks(&r.videos); albums(&r.albums);
            for a in &r.artists { println!("  {:<36.36}  {:<18}  {}", a.name, a.subscribers.as_deref().unwrap_or(""), a.channel_id); }
            for p in &r.playlists { println!("  {:<40.40}  {:<24.24}  {}", p.title, p.author, p.playlist_id); }
        }
        "suggest" => for s in yt.suggest(&arg).await? { println!("{s}") },
        "album" => { let a = yt.album(&arg).await?; println!("{} — {} ({})", a.title, a.artists.join(", "), a.year); tracks(&a.tracks); }
        "artist" => {
            let a = yt.artist(&arg).await?;
            println!("{}  {}\n{}", a.name, a.subscribers.as_deref().unwrap_or(""), a.description.as_deref().unwrap_or("").chars().take(200).collect::<String>());
            println!("\nSongs"); tracks(&a.songs); println!("\nAlbums"); albums(&a.albums); println!("\nSingles"); albums(&a.singles);
        }
        "playlist" => { let p = yt.playlist(&arg).await?; println!("{} — {}", p.title, p.author); tracks(&p.tracks); }
        "radio" => tracks(&yt.radio(&arg).await?),
        "moods" => for m in yt.moods().await? { println!("{:<18}  {:<26}  {}", m.section, m.title, m.params) },
        "mood" => for p in yt.mood_playlists(&arg).await? { println!("  {:<40.40}  {:<24.24}  {}", p.title, p.author, p.playlist_id) },
        "home" => for s in yt.home().await? {
            println!("\n== {} ==", s.title);
            for i in &s.items { println!("  {:<36.36}  {:<30.30}  {}", i.title, i.subtitle, i.video_id.as_deref().or(i.browse_id.as_deref()).or(i.playlist_id.as_deref()).unwrap_or("")); }
        },
        "chart" => tracks(&yt.chart(&arg).await?),
        "podcasts" => for p in yt.search_podcasts(&arg).await? { println!("  {:<40.40}  {:<24.24}  {}", p.title, p.publisher, p.podcast_id) },
        "podcast" => {
            let p = yt.podcast(&arg).await?;
            println!("{}\n{}", p.title, p.description.as_deref().unwrap_or("").chars().take(200).collect::<String>());
            tracks(&p.episodes);
        }
        "stream" => print_stream(&yt.stream(&arg, quality).await?),
        "download" => {
            let [id, dir] = &rest[..] else { bail!("usage: ytm download <videoId> <dir>") };
            println!("{}", yt.download(id, std::path::Path::new(dir)).await?.display());
        }
        "play" => {
            let Some(t) = yt.search(&arg, SearchKind::Songs).await?.songs.into_iter().next() else { bail!("no results for {arg:?}") };
            println!("▶ {} — {}", t.title, t.artist_line());
            let s = yt.stream(&t.video_id, quality).await?;
            print_stream(&s);
            std::process::Command::new("mpv").args(["--no-video", "--force-window=no", &s.url]).status()?;
        }
        _ => bail!("{USAGE}"),
    }
    Ok(())
}

fn print_stream(s: &Stream) {
    let left = s.expires_at.saturating_sub(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
    println!("itag {}  {}  {}  {} kbps  {:.0}s  expires in {}h{:02}m\n{}", s.itag, s.codec, s.ext, s.bitrate_kbps.map(|b| format!("{b:.0}")).unwrap_or_else(|| "?".into()), s.duration_s.unwrap_or(0.0), left / 3600, left % 3600 / 60, s.url);
}

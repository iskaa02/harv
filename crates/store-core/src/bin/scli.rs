//! Manual harness: `scli <db> <command> …` (tracks are given as `id:Title:Artist[:secs]`).
//!   like T | unlike T | liked | play T SECS | recent | top | follow ID NAME | followed
//!   pl-new TITLE | pl-add ID T… | pl-show ID | pl-mv ID FROM TO | pl-rm ID VIDEO | pls | del-pl ID
//!   lyrics-put VID KIND BODY | lyrics-get VID | lyrics-forget VID | lyrics-clear | lyrsearch WORDS… | lyrseed (fake songs + lyrics) | offset VID [MS] | set KEY JSON | get KEY | search-add Q | search-hist
//!   backup FILE | restore FILE | import FILE.json

use store_core::Store;
use ytm_core::Track;

fn track(s: &str) -> Track {
    let mut p = s.splitn(4, ':');
    Track { video_id: p.next().unwrap_or("").into(), title: p.next().unwrap_or("?").into(), artists: p.next().map(|a| vec![a.into()]).unwrap_or_default(), album: None, duration_s: p.next().and_then(|d| d.parse().ok()), thumb: None }
}

fn show(rows: &[store_core::SongRow]) { for r in rows { println!("  {} {:<10} {:<28} {:>5}s played  {}", if r.liked { "♥" } else { " " }, r.track.video_id, r.track.title, r.play_time_s, r.track.artist_line()); } }

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (Some(db), Some(cmd)) = (a.first(), a.get(1)) else { anyhow::bail!("usage: scli <db> <command> …") };
    let s = Store::open(db)?;
    let r = &a[2..];
    match cmd.as_str() {
        "like" => s.set_liked(&track(&r[0]), true)?,
        "unlike" => s.set_liked(&track(&r[0]), false)?,
        "liked" => show(&s.liked_songs(100, 0)?),
        "play" => println!("counted: {}", s.record_play(&track(&r[0]), r[1].parse()?)?),
        "recent" => show(&s.recently_played(50, 0)?),
        "top" => show(&s.most_played()?),
        "follow" => s.set_followed(&r[0], &r[1], None, true)?,
        "followed" => println!("{:?}", s.followed_artists()?),
        "pl-new" => println!("playlist id {}", s.create_playlist(&r[0])?),
        "pl-add" => println!("added {}", s.add_to_playlist(r[0].parse()?, &r[1..].iter().map(|t| track(t)).collect::<Vec<_>>())?),
        "pl-show" => for (i, t) in s.playlist_tracks(r[0].parse()?)?.iter().enumerate() { println!("  {i} {} {}", t.video_id, t.title); },
        "pl-mv" => s.move_in_playlist(r[0].parse()?, r[1].parse()?, r[2].parse()?)?,
        "pl-rm" => s.remove_from_playlist(r[0].parse()?, &r[1])?,
        "pls" => for p in s.playlists()? { println!("  #{} {} ({} songs)", p.id, p.title, p.count); },
        "del-pl" => s.delete_playlist(r[0].parse()?)?,
        "lyrics-put" => s.cache_lyrics(&r[0], "lrclib", &r[1], &r[2])?,
        "lyrics-get" => println!("{:?}", s.cached_lyrics(&r[0])),
        "lyrics-forget" => s.forget_lyrics(&r[0])?,
        "lyrics-clear" => s.clear_lyrics_cache()?,
        "lyrsearch" => {
            let hits = s.search_lyrics(&r.join(" "), 20)?;
            if hits.is_empty() { println!("  (no results)"); }
            for h in &hits {
                println!("  {} @{}ms  {} — {}", h.video_id, h.start_ms, h.title, h.artist);
                let mut shown = String::new();
                let mut at = 0;
                for &(a, b) in &h.snippet_ranges { shown.push_str(&h.line[at..a]); shown.push('['); shown.push_str(&h.line[a..b]); shown.push(']'); at = b; }
                shown.push_str(&h.line[at..]);
                println!("      {shown}");
            }
        }
        "lyrseed" => {
            let songs = [
                ("seed-static", "Static Dreams", "Test Artist", "[00:12.00] Counting static in the dark\n[00:17.50] the radio hums a lullaby\n[00:23.00] café lights and rainy streets\n[00:30.00] counting stars again tonight", "lrclib", "synced"),
                ("seed-night", "Night Drive", "Other Band", "Driving through the city\nNeon signs and counting stars\nnothing else matters now", "lrclib", "plain"),
                ("seed-arabic", "ليلة هادئة", "مغني", "[00:05.00] أحبك يا حبيبي\n[00:09.00] كَلِمَاتُ الحُبِّ تَمْلَأُ قَلْبِي", "betterlyrics", "synced"),
                ("seed-words", "Word Sync", "Enhanced", "[00:01.00] <00:01.00>counting <00:01.50>stars <00:02.00>tonight", "betterlyrics", "synced"),
            ];
            for (vid, title, artist, body, src, kind) in songs {
                s.upsert_song(&track(&format!("{vid}:{title}:{artist}:200")))?;
                s.cache_lyrics(vid, src, kind, body)?;
            }
            println!("seeded {} songs", songs.len());
        }
        "offset" => { if let Some(ms) = r.get(1) { s.set_lyric_offset_ms(&r[0], ms.parse()?)?; } println!("offset {} ms", s.lyric_offset_ms(&r[0])); }
        "set" => s.set(&r[0], &serde_json::from_str::<serde_json::Value>(&r[1])?)?,
        "get" => println!("{:?}", s.get::<serde_json::Value>(&r[0])),
        "search-add" => s.add_search(&r[0])?,
        "search-hist" => println!("{:?}", s.search_history(10)?),
        "backup" => s.backup_to(&r[0])?,
        "restore" => s.restore_from(&r[0])?,
        "import" => println!("{:?}", s.import_simpmusic(&std::fs::read_to_string(&r[0])?)?),
        c => anyhow::bail!("unknown command {c}"),
    }
    Ok(())
}

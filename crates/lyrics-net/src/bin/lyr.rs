//! Manual harness: `lyr <title> <artist> [duration_s] [--force]` — fetches (or reads the cache of) a throw-away store.
use lyrics_net::{describe, LyricsService};
use store_core::Store;
use ytm_core::Track;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut a: Vec<String> = std::env::args().skip(1).collect();
    let force = a.iter().position(|x| x == "--force").map(|i| a.remove(i)).is_some();
    anyhow::ensure!(a.len() >= 2, "usage: lyr <title> <artist> [duration_s] [--force]");
    let db = std::env::var("LYR_DB").unwrap_or_else(|_| "/tmp/lyr.db".into());
    let svc = LyricsService::new(Store::open(db)?)?;
    let t = Track { video_id: format!("{}-{}", a[0], a[1]), title: a[0].clone(), artists: vec![a[1].clone()], duration_s: a.get(2).and_then(|d| d.parse().ok()), ..Default::default() };
    match svc.lyrics(&t, force).await? {
        None => println!("no lyrics found"),
        Some(l) => {
            println!("{} from {} — {} lines", describe(&l), l.source, l.lines.len());
            for line in l.lines.iter().take(8) { println!("  {:>7.2}s  {}{}", line.time, line.text, if line.words.is_empty() { String::new() } else { format!("   [{} words]", line.words.len()) }); }
        }
    }
    Ok(())
}

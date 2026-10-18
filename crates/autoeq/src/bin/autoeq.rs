//! `autoeq search <query>` lists index matches; `autoeq show <exact name>` prints the profile and the mpv chain it compiles to.

use anyhow::{anyhow, Result};
use autoeq::{index, profile, search};
use player_core::AudioFx;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let http = reqwest::Client::builder().user_agent("harv/0.1").timeout(Duration::from_secs(60)).build()?;
    match args.first().map(String::as_str) {
        Some("search") => {
            let entries = index(&http).await?;
            for e in search(&entries, &args[1..].join(" ")) { println!("{}\t{}", e.name, e.source); }
        }
        Some("show") => {
            let name = args[1..].join(" ");
            let entries = index(&http).await?;
            let e = entries.iter().find(|e| e.name == name).ok_or_else(|| anyhow!("no entry named {name:?}; try `autoeq search`"))?;
            let p = profile(&http, e).await?;
            println!("{}  ({})  [{}]", e.name, e.path, e.source);
            println!("preamp {:.1} dB, {} filters", p.preamp_db, p.filters.len());
            for f in &p.filters { println!("  {:?} fc {:.0} Hz gain {:+.1} dB q {:.2}", f.kind, f.fc, f.gain_db, f.q); }
            let fx = AudioFx { parametric: p.filters.clone(), autoeq_preamp_db: p.preamp_db, ..Default::default() };
            println!("chain: {}", fx.chain());
        }
        _ => eprintln!("usage: autoeq search <query> | autoeq show <exact name>"),
    }
    Ok(())
}

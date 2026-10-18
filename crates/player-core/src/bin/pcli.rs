//! Manual test harness for `player-core`: `pcli <query>` queues song results, `pcli --radio <query>` queues a radio from the first hit.
//! `--state FILE`: on quit write the queue + position to FILE; if FILE exists at start, restore it (paused) instead of searching.
//! Commands on stdin: p pause/play · n next · b prev · j N jump · s shuffle · r repeat · e endless · + - volume · g SECS seek · x stop · w SECS wait · q quit
//! Audio: cf MS cross-fade · sleep SECS|end|off · sb cat1,cat2|off (SponsorBlock) · fx-eq N|off · speed X · pitch SEMIS · reverb 0..3 · delay MS

use player_core::*;
use std::sync::Arc;
use std::time::Duration;
use ytm_core::{SearchKind, Ytm};

fn now_ms() -> f64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64) }

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let state_file = args.iter().position(|a| a == "--state").map(|i| { args.remove(i); args.remove(i) });
    let radio = args.first().is_some_and(|a| a == "--radio");
    if radio { args.remove(0); }
    let query = args.join(" ");
    let saved: Option<(QueueState, f64)> = state_file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|j| serde_json::from_str(&j).ok());
    anyhow::ensure!(!query.is_empty() || saved.is_some(), "usage: pcli [--state FILE] [--radio] <query>");

    let ytm = Arc::new(Ytm::new().await?);
    let (engine, updates) = Engine::start(ytm.clone(), &tokio::runtime::Handle::current())?;
    if let Some((state, position)) = saved {
        println!("restoring {} tracks at {:.1}s", state.items.len(), position);
        engine.send(Command::Restore { state, position });
    } else {
        let songs = ytm.search(&query, SearchKind::Songs).await?.songs;
        let tracks = if radio { ytm.radio(&songs.first().ok_or_else(|| anyhow::anyhow!("no results"))?.video_id).await? } else { songs };
        println!("queued {} tracks", tracks.len());
        engine.send(Command::PlayQueue { tracks, start: 0, kind: if radio { QueueKind::Radio } else { QueueKind::Playlist } });
    }
    let latest: Arc<std::sync::Mutex<Option<Snapshot>>> = Arc::default();
    let latest2 = latest.clone();

    let mpris = match Mpris::start(engine.clone()).await { Ok(m) => Some(m), Err(e) => { println!("mpris unavailable: {e}"); None } };
    let clock = engine.clock();
    tokio::spawn(async move {
        let mut last = (None, Status::Stopped);
        while let Ok(u) = updates.recv().await {
            match u {
                Update::State(s) => {
                    if let Some(m) = &mpris { m.update(&s); }
                    *latest2.lock().unwrap() = Some(s.clone());
                    if (s.current, s.status) != last {
                        last = (s.current, s.status);
                        let t = s.current.map(|i| &s.queue[i]);
                        let mut extra = String::new();
                        if s.crossfade_ms > 0 { extra += &format!("  xfade={}ms", s.crossfade_ms); }
                        if let Some(sl) = &s.sleep { extra += &match sl.deadline_ms { Some(d) => format!("  sleep={:.0}s", (d as f64 - now_ms()) / 1000.0), None => "  sleep=end-of-track".to_string() }; }
                        if !s.sponsor_categories.is_empty() { extra += &format!("  sponsor={}", s.sponsor_categories.join(",")); }
                        if !s.fx.is_neutral() { extra += &format!("  fx=[{} speed={}]", s.fx.chain(), s.fx.speed); }
                        println!("[{:?}] {}/{}  {} — {}  dur {:.0}s  shuffle={} repeat={:?} vol={:.0}%{extra}", s.status, s.current.map_or(0, |i| i + 1), s.queue.len(),
                                 t.map_or("-", |t| t.title.as_str()), t.map_or(String::new(), |t| t.artist_line()), s.duration, s.shuffle, s.repeat, s.volume * 100.0);
                    }
                }
                Update::Error(e) => println!("error: {e}"),
            }
        }
    });
    let c2 = clock.clone();
    tokio::spawn(async move { loop { tokio::time::sleep(Duration::from_secs(5)).await; println!("  position {:.1} / {:.1}", c2.position(), c2.duration()); } });

    let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || { for l in std::io::stdin().lines().map_while(Result::ok) { let _ = line_tx.send(l); } let _ = line_tx.send("q".into()); });
    let (mut shuffle, mut repeat, mut endless, mut vol) = (false, Repeat::Off, true, 1.0f64);
    let mut fx = AudioFx::default();
    while let Some(l) = line_rx.recv().await {
        let mut it = l.split_whitespace();
        let arg: Option<f64> = it.clone().nth(1).and_then(|a| a.parse().ok());
        let word: &str = it.clone().nth(1).unwrap_or("");
        match it.next().unwrap_or("") {
            "p" => engine.send(Command::TogglePause),
            "n" => engine.send(Command::Next),
            "b" => engine.send(Command::Prev),
            "j" => engine.send(Command::Jump(arg.unwrap_or(1.0) as usize - 1)),
            "s" => { shuffle = !shuffle; engine.send(Command::SetShuffle(shuffle)) }
            "r" => { repeat = repeat.cycle(); engine.send(Command::SetRepeat(repeat)) }
            "e" => { endless = !endless; engine.send(Command::SetEndless(endless)) }
            "+" => { vol = (vol + 0.1).min(1.0); engine.send(Command::SetVolume(vol)) }
            "-" => { vol = (vol - 0.1).max(0.0); engine.send(Command::SetVolume(vol)) }
            "g" => engine.send(Command::Seek(arg.unwrap_or(0.0))),
            "x" => engine.send(Command::Stop),
            "w" => tokio::time::sleep(Duration::from_secs(arg.unwrap_or(1.0) as u64)).await,
            "cf" => engine.send(Command::SetCrossfade(arg.unwrap_or(0.0).max(0.0) as u64)),
            "sleep" => engine.send(Command::SetSleepTimer(match word {
                "end" => Some(SleepMode::EndOfTrack),
                "off" => None,
                _ => Some(SleepMode::After(Duration::from_secs_f64(arg.unwrap_or(0.0).max(0.0)))),
            })),
            "sb" => engine.send(Command::SetSponsorBlock(word.split(',').map(str::trim).filter(|c| !c.is_empty() && *c != "off").map(String::from).collect())),
            "fx-eq" => {
                if word == "off" { fx.eq_on = false; }
                else if let Some((_, pre, gains)) = word.parse::<usize>().ok().and_then(|i| EQ_PRESETS.get(i)) { fx.eq_on = true; fx.preamp_db = *pre; fx.eq_gains = *gains; }
                engine.send(Command::SetFx(fx.clone()))
            }
            "speed" => { fx.speed = arg.unwrap_or(1.0); engine.send(Command::SetFx(fx.clone())) }
            "pitch" => { fx.pitch_semitones = arg.unwrap_or(0.0) as f32; engine.send(Command::SetFx(fx.clone())) }
            "reverb" => { fx.reverb = match arg.unwrap_or(0.0) as u32 { 1 => Reverb::Room, 2 => Reverb::Hall, 3 => Reverb::Cathedral, _ => Reverb::Off }; engine.send(Command::SetFx(fx.clone())) }
            "delay" => { fx.delay_ms = arg.unwrap_or(0.0).max(0.0) as u32; engine.send(Command::SetFx(fx.clone())) }
            "q" => {
                if let (Some(f), Some(s)) = (&state_file, latest.lock().unwrap().clone()) {
                    let st = QueueState { items: s.queue, current: s.current, shuffle: s.shuffle, repeat: s.repeat, kind: s.kind };
                    std::fs::write(f, serde_json::to_string(&(st, clock.position()))?)?;
                    println!("saved state at {:.1}s", clock.position());
                }
                break;
            }
            _ => {}
        }
    }
    Ok(())
}

use crate::Found;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use ytm_core::Track;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct LrclibItem {
    #[serde(default)] track_name: String,
    #[serde(default)] duration: Option<f64>,
    #[serde(default)] instrumental: bool,
    #[serde(default)] plain_lyrics: Option<String>,
    #[serde(default)] synced_lyrics: Option<String>,
}

fn nonempty(s: &Option<String>) -> Option<&str> { s.as_deref().map(str::trim).filter(|s| !s.is_empty()) }

/// Drops `(feat. …)`, `[Remastered]` and similar bracketed noise, and a trailing ` - 2011 Remaster` style suffix.
pub(crate) fn clean_title(t: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in t.chars() {
        match c { '(' | '[' => depth += 1, ')' | ']' if depth > 0 => depth -= 1, c if depth == 0 => out.push(c), _ => {} }
    }
    let out = out.trim();
    let lower = out.to_lowercase();
    let cut = lower.rfind(" - ").filter(|&i| ["remaster", "version", "edit", "mono", "stereo", "live", "radio"].iter().any(|k| lower[i..].contains(k)));
    out[..cut.unwrap_or(out.len())].trim().to_string()
}

/// LRCLIB answers 503 now and then; one retry after a short pause covers it.
async fn get_retry(http: &reqwest::Client, url: &str, q: &[(&str, String)]) -> Result<reqwest::Response> {
    let r = http.get(url).query(q).send().await.context("lrclib unreachable")?;
    if !r.status().is_server_error() { return Ok(r); }
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    http.get(url).query(q).send().await.context("lrclib unreachable")
}

fn first_artist(t: &Track) -> String { t.artists.first().cloned().unwrap_or_default() }

fn found_from(i: &LrclibItem) -> Option<Found> {
    if i.instrumental { return None; }
    if let Some(s) = nonempty(&i.synced_lyrics) { return Some(Found { source: "lrclib", kind: "synced", body: s.to_string() }); }
    nonempty(&i.plain_lyrics).map(|p| Found { source: "lrclib", kind: "plain", body: p.to_string() })
}

/// Exact `/api/get` (needs duration; LRCLIB accepts ±2 s), then fuzzy `/api/search` picking the closest duration.
pub(crate) async fn lrclib(http: &reqwest::Client, t: &Track) -> Result<Option<Found>> {
    let artist = first_artist(t);
    let titles = { let c = clean_title(&t.title); if c != t.title && !c.is_empty() { vec![c, t.title.clone()] } else { vec![t.title.clone()] } };
    if let Some(d) = t.duration_s {
        for title in &titles {
            let mut q = vec![("track_name", title.clone()), ("artist_name", artist.clone()), ("duration", d.to_string())];
            if let Some(a) = &t.album { q.push(("album_name", a.clone())); }
            let r = get_retry(http, "https://lrclib.net/api/get", &q).await?;
            match r.status().as_u16() {
                200 => { if let Some(f) = found_from(&r.json::<LrclibItem>().await?) { return Ok(Some(f)); } }
                404 => {}
                s => bail!("lrclib answered {s}"),
            }
        }
    }
    let r = get_retry(http, "https://lrclib.net/api/search", &[("track_name", titles[0].clone()), ("artist_name", artist.clone())]).await?;
    if !r.status().is_success() { bail!("lrclib search answered {}", r.status()); }
    let items: Vec<LrclibItem> = r.json().await?;
    Ok(pick(&items, t, &titles[0]).and_then(found_from))
}

/// Closest duration (within 10 s) wins; synced beats plain; without a duration the first synced hit is taken.
fn pick<'a>(items: &'a [LrclibItem], t: &Track, title: &str) -> Option<&'a LrclibItem> {
    let want = title.to_lowercase();
    let mut best: Option<(&LrclibItem, f64)> = None;
    for i in items.iter().filter(|i| !i.instrumental && (nonempty(&i.synced_lyrics).is_some() || nonempty(&i.plain_lyrics).is_some())) {
        let dd = match (t.duration_s, i.duration) { (Some(a), Some(b)) => (a as f64 - b).abs(), _ => 5.0 };
        if dd > 10.0 { continue; }
        // Prefer the same title (not a remix), then synced, then closer duration.
        let same = i.track_name.to_lowercase() == want;
        let score = dd + if nonempty(&i.synced_lyrics).is_some() { 0.0 } else { 20.0 } + if same { 0.0 } else { 40.0 };
        if best.is_none_or(|(_, s)| score < s) { best = Some((i, score)); }
    }
    best.map(|(i, _)| i)
}

#[derive(Deserialize)]
struct BetterResponse { #[serde(default)] ttml: Option<String> }

/// Word-synced TTML. The service refuses uncached queries without a key (`X-API-Key`).
pub(crate) async fn betterlyrics(http: &reqwest::Client, key: Option<&str>, t: &Track) -> Result<Option<Found>> {
    let mut q = vec![("s", clean_title(&t.title)), ("a", first_artist(t))];
    if let Some(d) = t.duration_s { q.push(("d", d.to_string())); }
    let mut req = http.get("https://lyrics-api.boidu.dev/getLyrics").query(&q);
    if let Some(k) = key { req = req.header("X-API-Key", k); }
    let r = req.send().await.context("betterlyrics unreachable")?;
    match r.status().as_u16() {
        200 => {}
        404 => return Ok(None),
        // Without a key only songs already cached on their server are served; the rest answer 401 — that is just a miss.
        401 | 403 if key.is_none() => return Ok(None),
        401 | 403 => bail!("betterlyrics rejected the API key"),
        s => bail!("betterlyrics answered {s}"),
    }
    let text = r.text().await?;
    let body = serde_json::from_str::<BetterResponse>(&text).ok().and_then(|b| b.ttml).unwrap_or(text);
    Ok(body.trim_start().starts_with('<').then(|| Found { source: "betterlyrics", kind: "rich", body }))
}

//! Minimal TTML reader for word-synced lyrics (BetterLyrics, Apple-style): `<p begin end>` lines with `<span begin end>` words.
//! Hand-rolled scanner — the dialect is tiny and fixed, and it keeps this crate dependency-free of XML parsers.

use crate::lrc::{Line, Word};

pub fn parse(src: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(i) = find_tag(rest, "p") {
        let tag_end = rest[i..].find('>').map_or(rest.len(), |e| i + e);
        let attrs = &rest[i..tag_end];
        let body_start = (tag_end + 1).min(rest.len());
        let Some(close) = rest[body_start..].find("</p>") else { break };
        let body = &rest[body_start..body_start + close];
        rest = &rest[body_start + close + 4..];
        let Some(begin) = attr(attrs, "begin").and_then(parse_time) else { continue };
        let end = attr(attrs, "end").and_then(parse_time).unwrap_or(0.0);
        let (text, words) = spans(body);
        let instrumental = text.chars().all(|c| matches!(c, '♪' | '♫' | ' ' | '-'));
        out.push(Line { time: begin, end, text: if instrumental { crate::lrc::INTERLUDE_TEXT.into() } else { text }, instrumental, words: if instrumental { vec![] } else { words } });
    }
    out.sort_by(|a, b| a.time.total_cmp(&b.time));
    out
}

/// Position of `<name` followed by whitespace or `>` (so `<p` doesn't match `<param`).
fn find_tag(s: &str, name: &str) -> Option<usize> {
    let pat = format!("<{name}");
    let mut from = 0;
    while let Some(i) = s[from..].find(&pat) {
        let at = from + i;
        match s[at + pat.len()..].chars().next() { Some(c) if c.is_whitespace() || c == '>' => return Some(at), _ => from = at + pat.len() }
    }
    None
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(i) = tag[from..].find(name) {
        let at = from + i;
        let after = &tag[at + name.len()..];
        let prev_ok = at == 0 || !tag[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '-');
        if prev_ok { if let Some(v) = after.strip_prefix("=\"") { return v.split('"').next(); } }
        from = at + name.len();
    }
    None
}

/// `1:02:03.500`, `2:03.5`, `3.25`, `3.25s`.
fn parse_time(s: &str) -> Option<f32> {
    let s = s.trim().trim_end_matches('s');
    let mut secs = 0.0f32;
    for part in s.split(':') { secs = secs * 60.0 + part.parse::<f32>().ok()?; }
    Some(secs)
}

/// Words from `<span>`s inside one `<p>`. Whitespace between spans ends a word; spans glued together are syllables of one word.
fn spans(body: &str) -> (String, Vec<Word>) {
    let mut words: Vec<Word> = Vec::new();
    let mut rest = body;
    let mut gap = false;
    while let Some(i) = find_tag(rest, "span") {
        if rest[..i].chars().any(char::is_whitespace) { gap = true; }
        let tag_end = rest[i..].find('>').map_or(rest.len(), |e| i + e);
        let attrs = &rest[i..tag_end];
        let after = &rest[(tag_end + 1).min(rest.len())..];
        // A wrapper span without timing (e.g. background vocals `ttm:role="x-bg"`): step into it and read the nested spans.
        if attr(attrs, "begin").is_none() { rest = after; continue; }
        let Some(close) = after.find("</span>") else { break };
        let text = decode(&strip_tags(&after[..close]));
        rest = &after[close + 7..];
        let core = text.trim();
        if core.is_empty() { gap = true; continue; }
        let begin = attr(attrs, "begin").and_then(parse_time);
        let end = attr(attrs, "end").and_then(parse_time).unwrap_or(0.0);
        if words.is_empty() || gap || text.starts_with(char::is_whitespace) {
            let Some(b) = begin else { continue };
            if let Some(w) = words.last_mut() { if !w.text.ends_with(' ') { w.text.push(' '); } }
            words.push(Word { time: b, end, text: core.to_string() });
        } else if let Some(w) = words.last_mut() {
            w.text.push_str(core);
            if end > 0.0 { w.end = end; }
        }
        gap = text.ends_with(char::is_whitespace);
    }
    if words.is_empty() { return (decode(strip_tags(body).trim()), words); }
    let text: String = words.iter().map(|w| w.text.as_str()).collect::<String>().trim().to_string();
    (text, words)
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() { match c { '<' => in_tag = true, '>' => in_tag = false, c if !in_tag => out.push(c), _ => {} } }
    out
}

fn decode(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#39;", "'").replace("&amp;", "&")
}

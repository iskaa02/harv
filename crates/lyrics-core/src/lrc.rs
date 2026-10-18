//! `.lrc` parser: `[mm:ss.xx] text`, multiple stamps per line allowed, plus "enhanced" word stamps `<mm:ss.xx>word`.

/// One sung word (or syllable group). `text` carries its trailing space, so `words.concat() == line.text`.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub time: f32,
    /// 0.0 = unknown (enhanced LRC has no end stamps; derive it from the next word / line).
    pub end: f32,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub time: f32,
    /// 0.0 = unknown.
    pub end: f32,
    pub text: String,
    /// `♪` / empty lines: rendered as an animated "• • •" interlude.
    pub instrumental: bool,
    /// Empty for line-synced lyrics.
    pub words: Vec<Word>,
}

impl Line {
    /// Word-level progress 0..1 of word `i` at time `t` (for a left-to-right fill / glow); 0 before, 1 after.
    pub fn word_end(&self, i: usize) -> f32 {
        let w = &self.words[i];
        if w.end > w.time { w.end } else { self.words.get(i + 1).map_or(self.end.max(w.time + 0.3), |n| n.time) }
    }

    pub fn word_progress(&self, i: usize, t: f32) -> f32 {
        self.span_progress(i, i, t)
    }

    /// Progress through words `a..=b` taken as one unit (a whole word sung as several syllables).
    pub fn span_progress(&self, a: usize, b: usize, t: f32) -> f32 {
        let start = self.words[a].time;
        ((t - start) / (self.word_end(b) - start).max(0.05)).clamp(0.0, 1.0)
    }
}

/// How precisely the lyrics are timed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sync { Rich, Line, Plain }

/// Parsed lyrics ready for display.
#[derive(Clone, Debug)]
pub struct Lyrics { pub lines: Vec<Line>, pub sync: Sync, pub source: String }

impl Lyrics {
    pub fn is_empty(&self) -> bool { self.lines.is_empty() }

    /// From provider text: TTML if it looks like XML, otherwise (enhanced) LRC; unsynced text becomes [`Sync::Plain`].
    pub fn from_text(body: &str, source: &str) -> Lyrics {
        let b = body.trim_start();
        let (lines, sync) = if b.starts_with('<') {
            let l = crate::ttml::parse(body);
            let s = if l.iter().any(|l| !l.words.is_empty()) { Sync::Rich } else { Sync::Line };
            (l, s)
        } else {
            let l = parse(body);
            if l.is_empty() { (parse_plain(body), Sync::Plain) }
            else if l.iter().any(|l| !l.words.is_empty()) { (l, Sync::Rich) } else { (l, Sync::Line) }
        };
        Lyrics { lines, sync, source: source.into() }
    }
}

/// Unsynced text: one line per row, no timing.
pub fn parse_plain(src: &str) -> Vec<Line> {
    src.lines().map(str::trim).map(|t| Line { time: 0.0, end: 0.0, text: t.into(), instrumental: t.is_empty(), words: vec![] }).collect()
}

pub const INTERLUDE_TEXT: &str = "• • •";

pub fn parse(src: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for raw in src.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            if let Some(t) = parse_stamp(&stripped[..end]) {
                stamps.push(t);
            }
            rest = stripped[end + 1..].trim_start();
        }
        if stamps.is_empty() { continue; }
        let (text, words) = split_words(rest.trim());
        let instrumental = text.chars().all(|c| matches!(c, '♪' | '♫' | ' ' | '-'));
        for t in stamps {
            out.push(Line {
                time: t,
                end: 0.0,
                text: if instrumental { INTERLUDE_TEXT.into() } else { text.clone() },
                instrumental,
                words: if instrumental { vec![] } else { words.clone() },
            });
        }
    }
    out.sort_by(|a, b| a.time.total_cmp(&b.time));
    for i in 0..out.len().saturating_sub(1) { out[i].end = out[i + 1].time; }
    // Collapse consecutive interludes and drop a trailing one.
    out.dedup_by(|b, a| a.instrumental && b.instrumental);
    if out.last().is_some_and(|l| l.instrumental) {
        out.pop();
    }
    out
}

fn parse_stamp(s: &str) -> Option<f32> {
    let (m, rest) = s.split_once(':')?;
    let m: f32 = m.parse().ok()?;
    let sec: f32 = rest.parse().ok()?;
    Some(m * 60.0 + sec)
}

/// Index of the line being sung at time `t`.
pub fn active(lines: &[Line], t: f32) -> Option<usize> {
    lines.partition_point(|l| l.time <= t).checked_sub(1)
}

/// `<00:12.30>Hel<00:12.55>lo <00:13.00>world` → ("Hello world", words). Text without word stamps → no words.
fn split_words(s: &str) -> (String, Vec<Word>) {
    if !s.contains('<') { return (s.to_string(), vec![]); }
    let mut words: Vec<Word> = Vec::new();
    let mut plain = String::new();
    let mut rest = s;
    let mut cur: Option<(f32, String)> = None;
    let flush = |cur: &mut Option<(f32, String)>, words: &mut Vec<Word>| {
        if let Some((t, text)) = cur.take() { if !text.is_empty() { words.push(Word { time: t, end: 0.0, text }); } }
    };
    while let Some(open) = rest.find('<') {
        let (before, after) = rest.split_at(open);
        if let Some((_, c)) = cur.as_mut() { c.push_str(before); }
        plain.push_str(before);
        let Some(close) = after.find('>') else { rest = after; break };
        match parse_stamp(&after[1..close]) {
            Some(t) => {
                // A stamp that follows whitespace starts a new word; one glued to text continues the same word (syllables).
                let glued = cur.as_ref().is_some_and(|(_, c)| !c.is_empty() && !c.ends_with(' '));
                if !glued { flush(&mut cur, &mut words); cur = Some((t, String::new())); }
            }
            None => { if let Some((_, c)) = cur.as_mut() { c.push_str(&after[..=close]); } plain.push_str(&after[..=close]); }
        }
        rest = &after[close + 1..];
    }
    if let Some((_, c)) = cur.as_mut() { c.push_str(rest); }
    plain.push_str(rest);
    flush(&mut cur, &mut words);
    for i in 0..words.len().saturating_sub(1) { words[i].end = words[i + 1].time; }
    (plain.trim().to_string(), words)
}

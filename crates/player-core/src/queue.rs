//! Play queue: item list + play-order permutation (so shuffle never reorders what the user sees), repeat, and kind.
//! Pure logic, no audio. Mirrors SimpMusic's `QueueData` / `PlaylistType` semantics.

use serde::{Deserialize, Serialize};
use ytm_core::Track;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Repeat { #[default] Off, All, One }

impl Repeat {
    pub fn cycle(self) -> Self { match self { Self::Off => Self::All, Self::All => Self::One, Self::One => Self::Off } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
/// `Songs` = a loose list (recent, favourites, search results, charts…) as opposed to a curated `Playlist`/`Album`: the kind that "radio on click" applies to.
pub enum QueueKind { #[default] Playlist, Album, LocalPlaylist, Radio, Songs }

/// What gets persisted / restored (the shuffle order itself is not kept).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueueState {
    pub items: Vec<Track>,
    pub current: Option<usize>,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub kind: QueueKind,
}

/// SimpMusic keeps 100 played tracks and trims once 120 have piled up (`RadioQueueTrim`).
const KEEP_PLAYED: usize = 100;
const TRIM_AT: usize = 120;

#[derive(Debug, Default)]
pub struct Queue {
    items: Vec<Track>,
    /// Play order: indices into `items`.
    order: Vec<usize>,
    /// Position in `order`.
    cursor: Option<usize>,
    shuffle: bool,
    repeat: Repeat,
    kind: QueueKind,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Removed { Current, Other, Nothing }

impl Queue {
    pub fn items(&self) -> &[Track] { &self.items }
    pub fn shuffle(&self) -> bool { self.shuffle }
    pub fn repeat(&self) -> Repeat { self.repeat }
    pub fn kind(&self) -> QueueKind { self.kind }
    pub fn set_repeat(&mut self, r: Repeat) { self.repeat = r }
    pub fn current_index(&self) -> Option<usize> { self.cursor.map(|c| self.order[c]) }
    pub fn current(&self) -> Option<&Track> { self.current_index().map(|i| &self.items[i]) }
    /// Tracks still to come in play order (not counting wrap-around).
    pub fn remaining(&self) -> usize { self.cursor.map_or(0, |c| self.order.len() - c - 1) }

    pub fn state(&self) -> QueueState { QueueState { items: self.items.clone(), current: self.current_index(), shuffle: self.shuffle, repeat: self.repeat, kind: self.kind } }

    pub fn restore(&mut self, s: QueueState) {
        self.items = s.items;
        self.shuffle = s.shuffle;
        self.repeat = s.repeat;
        self.kind = s.kind;
        self.order = (0..self.items.len()).collect();
        self.cursor = s.current.filter(|&i| i < self.items.len());
        if self.shuffle { self.reshuffle(self.cursor); }
    }

    pub fn set(&mut self, items: Vec<Track>, start: usize, kind: QueueKind) {
        self.items = items;
        self.kind = kind;
        self.order = (0..self.items.len()).collect();
        self.cursor = (!self.items.is_empty()).then(|| start.min(self.items.len() - 1));
        if self.shuffle { self.reshuffle(self.cursor); }
    }

    fn reshuffle(&mut self, keep: Option<usize>) {
        let mut o: Vec<usize> = (0..self.items.len()).collect();
        fastrand::shuffle(&mut o);
        match keep {
            Some(k) => { let p = o.iter().position(|&x| x == k).unwrap(); o.swap(0, p); self.cursor = Some(0); }
            None => self.cursor = (!o.is_empty()).then_some(0),
        }
        self.order = o;
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if on == self.shuffle { return; }
        let cur = self.current_index();
        self.shuffle = on;
        if on { self.reshuffle(cur); } else { self.order = (0..self.items.len()).collect(); self.cursor = cur; }
    }

    /// Move to the next track. `auto` = the track ended by itself (repeat-one replays it); a manual skip always moves on.
    /// `None` = end of queue (cursor unchanged).
    pub fn advance(&mut self, auto: bool) -> Option<usize> {
        let c = self.cursor?;
        if auto && self.repeat == Repeat::One { return self.current_index(); }
        if c + 1 < self.order.len() { self.cursor = Some(c + 1); }
        else if self.repeat != Repeat::Off && !self.items.is_empty() {
            if self.shuffle { self.reshuffle(None); } else { self.cursor = Some(0); }
        } else { return None; }
        self.current_index()
    }

    /// The track `advance(true)` would pick, without moving (for prefetching).
    pub fn peek_next(&self) -> Option<&Track> {
        let c = self.cursor?;
        if self.repeat == Repeat::One { return None; }
        match self.order.get(c + 1) {
            Some(&i) => Some(&self.items[i]),
            None if self.repeat == Repeat::All && !self.shuffle => self.items.first(),
            None => None,
        }
    }

    pub fn back(&mut self) -> Option<usize> {
        let c = self.cursor?;
        if c > 0 { self.cursor = Some(c - 1); }
        else if self.repeat == Repeat::All { self.cursor = Some(self.order.len() - 1); }
        self.current_index()
    }

    pub fn jump(&mut self, item: usize) -> bool {
        match self.order.iter().position(|&i| i == item) { Some(p) => { self.cursor = Some(p); true } None => false }
    }

    pub fn push(&mut self, tracks: Vec<Track>) {
        for t in tracks { self.order.push(self.items.len()); self.items.push(t); }
        if self.cursor.is_none() && !self.items.is_empty() { self.cursor = Some(0); }
    }

    /// Radio refill: only tracks not already queued.
    pub fn push_new(&mut self, tracks: Vec<Track>) -> usize {
        let fresh: Vec<Track> = tracks.into_iter().filter(|t| !self.items.iter().any(|x| x.video_id == t.video_id)).collect();
        let n = fresh.len();
        self.push(fresh);
        n
    }

    pub fn play_next(&mut self, track: Track) {
        let idx = self.items.len();
        self.items.push(track);
        let at = self.cursor.map_or(0, |c| c + 1);
        self.order.insert(at, idx);
        if self.cursor.is_none() { self.cursor = Some(0); }
    }

    pub fn remove(&mut self, item: usize) -> Removed {
        let Some(p) = self.order.iter().position(|&i| i == item) else { return Removed::Nothing };
        let was_current = self.cursor == Some(p);
        self.items.remove(item);
        self.order.remove(p);
        for o in &mut self.order { if *o > item { *o -= 1; } }
        self.cursor = match self.cursor {
            _ if self.order.is_empty() => None,
            Some(c) if p < c => Some(c - 1),
            Some(c) => Some(c.min(self.order.len() - 1)),
            None => None,
        };
        if was_current { Removed::Current } else { Removed::Other }
    }

    pub fn move_item(&mut self, from: usize, to: usize) {
        if from == to || from >= self.items.len() || to >= self.items.len() { return; }
        let t = self.items.remove(from);
        self.items.insert(to, t);
        let map = |i: usize| if i == from { to } else if from < to && i > from && i <= to { i - 1 } else if from > to && i >= to && i < from { i + 1 } else { i };
        for o in &mut self.order { *o = map(*o); }
    }

    /// Drop old history from long (radio) queues so memory stays flat. Returns how many items were dropped.
    pub fn trim_history(&mut self) -> usize {
        let Some(c) = self.cursor else { return 0 };
        if self.shuffle || c < TRIM_AT { return 0; }
        let drop = c - KEEP_PLAYED;
        self.items.drain(..drop);
        self.order = (0..self.items.len()).collect();
        self.cursor = Some(c - drop);
        drop
    }
}

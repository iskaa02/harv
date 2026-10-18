//! Right-panel components: `queue-list`, `lyrics-teaser`. They have no page data; they follow playback through
//! `ctx.on_state` (queue / track changes) and, for the lyrics, the 250 ms ticker that only runs while playing.

use super::Env;
use crate::ctx::Ctx;
use crate::rows::art;
use gtk::{glib, prelude::*};
use lyrics_core::lrc::{self, Line};
use player_core::{Command, Snapshot, Status};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use theme_core::Node;
use ytm_core::Track;

pub fn component(env: &Env, node: &Node) -> Option<gtk::Widget> {
    match node.c.as_str() {
        "queue-list" => Some(queue_list(env.ctx, node)),
        "lyrics-teaser" => Some(lyrics_teaser(env.ctx, node)),
        _ => None,
    }
}

/// The track playing now.
fn current(s: &Snapshot) -> Option<&Track> { s.current.and_then(|i| s.queue.get(i)) }

/// `queue-list`: the upcoming tracks. Props: limit = n, title = <vocabulary key> (section label).
/// A click plays that queue item (`Command::Jump`).
fn queue_list(ctx: &Rc<Ctx>, node: &Node) -> gtk::Widget {
    let limit = node.int_or("limit", 6).max(1) as usize;
    let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
    col.add_css_class("queue-list");
    if let Some(key) = node.str("title") { col.append(&gtk::Label::builder().label(ctx.word(key)).xalign(0.0).css_classes(["panel-label"]).build()); }
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["queue-items", "rows"]).build();
    col.append(&list);

    // Queue index of each shown row (the list may also hold the "nothing up next" note).
    let indices: Rc<RefCell<Vec<usize>>> = Rc::default();
    {
        let (indices, ctx2) = (indices.clone(), ctx.clone());
        list.connect_row_activated(move |_, row| {
            let j = indices.borrow().get(row.index() as usize).copied();
            if let Some(j) = j { ctx2.engine.send(Command::Jump(j)); }
        });
    }

    // Rebuild only when the visible part of the queue changes; position updates never reach here.
    let sig: Rc<RefCell<String>> = Rc::default();
    let weak = list.downgrade();
    ctx.on_state_while({
        let ctx = ctx.clone();
        move |s| {
            let Some(list) = weak.upgrade() else { return false };
            let start = s.current.map_or(0, |i| i + 1);
            let shown: Vec<(usize, &Track)> = s.queue.iter().enumerate().skip(start).take(limit).collect();
            let key = format!("{start}|{}|{}", s.queue.len(), shown.iter().map(|(_, t)| t.video_id.as_str()).collect::<Vec<_>>().join(","));
            if *sig.borrow() == key { return true; }
            *sig.borrow_mut() = key;
            while let Some(c) = list.first_child() { list.remove(&c); }
            indices.borrow_mut().clear();
            for (j, t) in &shown {
                indices.borrow_mut().push(*j);
                list.append(&queue_row(&ctx, t));
            }
            if shown.is_empty() {
                list.append(&gtk::Label::builder().label("Nothing up next").xalign(0.0).css_classes(["queue-empty"]).build());
            }
            true
        }
    });
    col.upcast()
}

fn queue_row(ctx: &Rc<Ctx>, t: &Track) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("queue-row");
    let a = art(ctx, t.thumb.as_deref(), 36);
    row.append(&a);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(&gtk::Label::builder().label(&t.title).xalign(0.0).max_width_chars(1).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["song-title"]).build());
    text.append(&gtk::Label::builder().label(t.artist_line()).xalign(0.0).max_width_chars(1).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["song-artist"]).build());
    row.append(&text);
    row.append(&gtk::Label::builder().label(t.duration_s.map(crate::rows::fmt_time).unwrap_or_default()).valign(gtk::Align::Center).css_classes(["song-time"]).build());
    row.upcast()
}

/// Lyrics state of the current track (fetched once per track; `shown` = last painted line, -2 = nothing painted yet).
#[derive(Default)]
struct Teaser { id: String, lines: Vec<Line>, status: LyrState, shown: i64, running: bool, generation: u64 }

#[derive(Default, PartialEq, Clone, Copy)]
enum LyrState { #[default] Idle, Loading, Ready, Missing }

/// `lyrics-teaser`: the active lyric line in full, then the next `lines` lines dimmed. Follows the playback clock.
fn lyrics_teaser(ctx: &Rc<Ctx>, node: &Node) -> gtk::Widget {
    let n = node.int_or("lines", 2).clamp(0, 6) as usize;
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.add_css_class("lyrics-teaser");
    if let Some(a) = node.str("align") { col.add_css_class(&format!("align-{a}")); }
    let active = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["lyric-active"]).build();
    col.append(&active);
    let next: Vec<gtk::Label> = (0..n).map(|_| gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["lyric-next"]).build()).collect();
    for l in &next { col.append(l); }
    let note = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["lyric-note"]).visible(false).build();
    col.append(&note);

    let st = Rc::new(RefCell::new(Teaser { shown: -2, ..Default::default() }));
    let paint = {
        let (st, active, next, note) = (st.clone(), active.clone(), next.clone(), note.clone());
        Rc::new(move |pos: f64| {
            let mut t = st.borrow_mut();
            let (status, shown_now) = (t.status, lrc::active(&t.lines, pos as f32).map_or(-1, |i| i as i64));
            let text_of = |l: &Line| if l.instrumental || l.text.trim().is_empty() { "•••".to_string() } else { l.text.clone() };
            match status {
                LyrState::Ready => {
                    note.set_visible(false);
                    active.set_visible(true);
                    if shown_now == t.shown { return; }
                    t.shown = shown_now;
                    let from = if shown_now < 0 { 0 } else { shown_now as usize };
                    let (a, rest_from) = if shown_now < 0 { (String::new(), 0) } else { (text_of(&t.lines[from]), from + 1) };
                    active.set_label(&a);
                    for (k, l) in next.iter().enumerate() {
                        l.set_label(&t.lines.get(rest_from + k).map(|x| text_of(x)).unwrap_or_default());
                    }
                }
                LyrState::Loading | LyrState::Missing => {
                    active.set_visible(false);
                    for l in &next { l.set_visible(false); }
                    note.set_visible(true);
                    note.set_label(if status == LyrState::Loading { "Loading lyrics…" } else { "No lyrics for this song" });
                }
                LyrState::Idle => { active.set_label(""); for l in &next { l.set_label(""); } note.set_visible(false); }
            }
            for l in &next { l.set_visible(status == LyrState::Ready); }
        })
    };

    // Ticker: only while playing (the timer ends itself when paused or when the teaser is gone).
    let start_timer = {
        let (st, paint, weak, ctx) = (st.clone(), paint.clone(), col.downgrade(), ctx.clone());
        Rc::new(move || {
            let my = {
                let mut t = st.borrow_mut();
                if t.running { return; }
                t.running = true;
                t.generation += 1;
                t.generation
            };
            let (st, paint, weak, ctx) = (st.clone(), paint.clone(), weak.clone(), ctx.clone());
            glib::timeout_add_local(Duration::from_millis(250), move || {
                let alive = weak.upgrade().is_some();
                let mut t = st.borrow_mut();
                if !alive || t.generation != my || !matches!(ctx.snap.borrow().as_ref().map(|s| s.status), Some(Status::Playing)) {
                    if t.generation == my { t.running = false; }
                    return glib::ControlFlow::Break;
                }
                drop(t);
                paint(ctx.engine.clock().position());
                glib::ControlFlow::Continue
            });
        })
    };

    let weak = col.downgrade();
    ctx.on_state_while({
        let (st, paint, start_timer, ctx) = (st.clone(), paint.clone(), start_timer.clone(), ctx.clone());
        move |s| {
            if weak.upgrade().is_none() { return false; }
            let cur = current(s).cloned();
            let id = cur.as_ref().map(|t| t.video_id.clone()).unwrap_or_default();
            let changed = st.borrow().id != id;
            if changed {
                {
                    let mut t = st.borrow_mut();
                    t.id = id.clone();
                    t.lines.clear();
                    t.shown = -2;
                    t.status = if cur.is_some() { LyrState::Loading } else { LyrState::Idle };
                }
                if let Some(tr) = cur {
                    let svc = ctx.lyrics.clone();
                    let (st, paint, ctx3) = (st.clone(), paint.clone(), ctx.clone());
                    ctx.spawn(async move { svc.lyrics(&tr, false).await.map_err(|e| format!("{e:#}")) }, move |res| {
                        let mut t = st.borrow_mut();
                        if t.id != id { return; }
                        match res {
                            Ok(Some(l)) if !l.is_empty() => { t.lines = l.lines; t.status = LyrState::Ready; }
                            _ => t.status = LyrState::Missing,
                        }
                        drop(t);
                        paint(ctx3.engine.clock().position());
                    });
                }
            }
            paint(ctx.engine.clock().position());
            if s.status == Status::Playing { start_timer(); }
            true
        }
    });
    col.upcast()
}

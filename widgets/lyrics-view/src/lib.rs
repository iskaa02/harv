//! The lyrics screen as a custom GTK widget. All drawing goes through GSK render nodes
//! (blur / opacity / transforms are GPU-side), and the frame clock only ticks while
//! something is animating.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU8, Ordering};

use gtk::prelude::*;
use gtk::{gdk, glib, graphene, gsk, pango, subclass::prelude::*};
use lyrics_core::lrc::{Line, Lyrics, Sync};
pub use lyrics_core::{Align, BackdropKind, LayoutKind, LyricsStyle, StyleKind, TextColors};
use lyrics_core::{layout, palette, Scene};

/// What the screen shows besides the lyrics.
#[derive(Clone, Debug, Default)]
pub struct Meta {
    pub title: String,
    pub artist: String,
    pub album: String,
}

/// How alive the full-screen backdrop is. Global (one setting for every lyrics view); read every frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Off,
    Calm,
    Lively,
}

static MOTION: AtomicU8 = AtomicU8::new(2);

pub fn set_motion(m: Motion) { MOTION.store(m as u8, Ordering::Relaxed); }

pub fn motion() -> Motion {
    match MOTION.load(Ordering::Relaxed) { 0 => Motion::Off, 1 => Motion::Calm, _ => Motion::Lively }
}

fn motion_strength() -> f32 {
    match motion() { Motion::Off => 0.0, Motion::Calm => 0.6, Motion::Lively => 1.0 }
}

/// Full-screen open / close choreography (not a track change): the pieces of the view enter one after another on a shared
/// timeline. Opening plays it forward over [`INTRO_TOTAL`] s; closing plays it backwards, faster.
#[derive(Clone, Copy)]
struct Intro { t: f32, closing: bool }
const INTRO_TOTAL: f32 = 1.0;
const INTRO_CLOSE: f32 = 0.26;

/// A lyric line from the previous song, frozen where it was and animated out.
struct Leaving { layout: pango::Layout, y: f32, h: f32, sc: f32, alpha: f32, blur: f32, rtl: bool, rank: f32 }

/// Track-change choreography. Armed by [`LyricsView::transition`]; the old song's pieces stay on screen until the new ones land.
#[derive(Default)]
struct Trans {
    /// +1 = forward, -1 = back; 0 = nothing armed.
    dir: f32,
    /// Seconds since `transition()`.
    t: f32,
    old_cover: Option<gdk::Texture>,
    old_backdrop: Option<gdk::Texture>,
    old_colors: [[f32; 3]; 4],
    old_luma: f32,
    old_title: Option<pango::Layout>,
    old_artist: Option<pango::Layout>,
    /// Seconds since the new cover / lyrics landed.
    cover_t: Option<f32>,
    lyr_t: Option<f32>,
    leaving: Vec<Leaving>,
}

const TITLE_D: f32 = 0.7;
const LEAVE_D: f32 = 0.35;
const COVER_D: f32 = 0.7;
const BG_D: f32 = 1.0;
const ENTER_BASE: f32 = 0.28;
const ENTER_D: f32 = 0.7;
const ENTER_STAGGER: f32 = 0.045;
const SLIDE: f32 = 90.0;

/// 0..1 progress of one piece of the open / close choreography (`delay` and `dur` in timeline seconds); 1 when none is running.
fn intro_k(s: &State, delay: f32, dur: f32) -> f32 {
    match s.intro {
        None => 1.0,
        Some(i) => {
            let tl = if i.closing { (INTRO_TOTAL * (1.0 - i.t / INTRO_CLOSE)).max(0.0) } else { i.t };
            ease_out((tl - delay) / dur)
        }
    }
}

fn ease_out(x: f32) -> f32 { 1.0 - (1.0 - x.clamp(0.0, 1.0)).powi(3) }
fn ease_in(x: f32) -> f32 { x.clamp(0.0, 1.0).powi(2) }
fn smooth(x: f32) -> f32 { let x = x.clamp(0.0, 1.0); x * x * (3.0 - 2.0 * x) }

impl Trans {
    fn advance(&mut self, dt: f32) {
        // Debug aid for screenshots: LYRICS_FREEZE_T=0.35 pins every timer at that many seconds.
        let cap = std::env::var("LYRICS_FREEZE_T").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(f32::MAX);
        let up = |t: &mut f32| *t = (*t + dt).min(cap);
        up(&mut self.t);
        if let Some(c) = &mut self.cover_t { up(c); }
        if let Some(c) = &mut self.lyr_t { up(c); }
        // The cover never arrived: stop waiting (old and new are then the same texture).
        if self.cover_t.is_none() && self.t > 6.0 { self.cover_t = Some(0.0); }
        if self.t > 0.75 { self.old_title = None; self.old_artist = None; }
        if self.t > LEAVE_D + 0.5 { self.leaving.clear(); }
        if let Some(c) = self.cover_t {
            if c > COVER_D { self.old_cover = None; }
            if c > BG_D { self.old_backdrop = None; }
        }
    }
    fn live(&self) -> bool {
        self.old_cover.is_some() || self.old_backdrop.is_some() || self.old_title.is_some() || !self.leaving.is_empty()
            || self.lyr_t.is_some_and(|t| t < ENTER_BASE + ENTER_D + 0.6) || self.cover_t.is_some_and(|t| t < BG_D)
    }
}

pub struct State {
    meta: Meta,
    lines: Vec<Line>,
    sync: Sync,
    /// Byte ranges of each word in `lines[i].text`, only for lines whose words reassemble the text exactly.
    word_ranges: Vec<Vec<(u32, u32)>>,
    /// Line currently carrying word-alpha attributes (cleared when it stops being active).
    attr_line: Option<usize>,
    scene: Scene,
    /// Audio position in seconds; `None` = simulated clock (demo).
    position: Option<Box<dyn Fn() -> f32>>,
    /// Lyric timing offset in seconds (positive = lyrics later).
    offset: f32,
    on_seek: Option<Rc<dyn Fn(f32)>>,
    controls_h: f32,
    on_layout: Option<Rc<dyn Fn(&layout::Layout)>>,
    colors: [[f32; 3]; 4],
    /// Mean brightness of the backdrop bitmap; light covers get a darker overlay so the lyrics stay readable.
    luma: f32,
    cover: gdk::Texture,
    backdrop: gdk::Texture,
    lay: layout::Layout,
    key: (i32, i32),
    layouts: Vec<pango::Layout>,
    /// Plain mode: each line as a white alpha mask, rendered once and scaled by the GPU while its size animates (see `Paint::line_tex`).
    line_tex: Vec<Option<LineTex>>,
    /// Each line's scale in the previous frame (plain mode), to tell which lines are mid-animation.
    prev_sc: Vec<f32>,
    heights: Vec<f32>,
    tops: Vec<f32>,
    title: Option<pango::Layout>,
    artist: Option<pango::Layout>,
    hit: Vec<[f32; 4]>,
    tick: Option<gtk::TickCallbackId>,
    last: Option<i64>,
    tr: Trans,
    intro: Option<Intro>,
    hover: Option<usize>,
    style: LyricsStyle,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct LyricsView {
        pub st: RefCell<Option<State>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LyricsView {
        const NAME: &'static str = "HarvLyricsView";
        type Type = super::LyricsView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for LyricsView {}

    impl WidgetImpl for LyricsView {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }
    }
}

glib::wrapper! {
    pub struct LyricsView(ObjectSubclass<imp::LyricsView>) @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

fn texture(img: &image::RgbaImage) -> gdk::Texture {
    let bytes = glib::Bytes::from(img.as_raw().as_slice());
    gdk::MemoryTexture::new(
        img.width() as i32,
        img.height() as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        img.width() as usize * 4,
    )
    .upcast()
}

fn rgba(c: [f32; 3], a: f32) -> gdk::RGBA {
    gdk::RGBA::new(c[0], c[1], c[2], a)
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> graphene::Rect {
    graphene::Rect::new(x, y, w, h)
}

fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> gsk::RoundedRect {
    let s = graphene::Size::new(r, r);
    gsk::RoundedRect::new(rect(x, y, w, h), s, s, s, s)
}

impl LyricsView {
    /// An empty screen (no track yet); fill it with [`LyricsView::set_track`].
    pub fn new() -> Self {
        let obj: Self = glib::Object::new();
        obj.set_hexpand(true);
        obj.set_vexpand(true);
        let cover = placeholder_cover();
        *obj.imp().st.borrow_mut() = Some(State {
            meta: Meta::default(),
            lines: Vec::new(),
            sync: Sync::Line,
            word_ranges: Vec::new(),
            attr_line: None,
            scene: Scene::empty(),
            position: None,
            offset: 0.0,
            on_seek: None,
            controls_h: 0.0,
            on_layout: None,
            colors: palette::extract(&cover),
            luma: 0.1,
            cover: texture(&cover),
            backdrop: texture(&palette::backdrop(&cover)),
            lay: layout::compute(1000.0, 700.0, 0.0),
            key: (0, 0),
            layouts: Vec::new(),
            line_tex: Vec::new(),
            prev_sc: Vec::new(),
            heights: Vec::new(),
            tops: Vec::new(),
            title: None,
            artist: None,
            hit: Vec::new(),
            tick: None,
            last: None,
            tr: Trans::default(),
            intro: None,
            hover: None,
            style: LyricsStyle::default(),
        });

        let click = gtk::GestureClick::new();
        let v = obj.clone();
        click.connect_pressed(move |_, _, x, y| v.click(x as f32, y as f32));
        obj.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        let v = obj.clone();
        motion.connect_motion(move |_, x, y| v.hover_at(Some((x as f32, y as f32))));
        let v = obj.clone();
        motion.connect_leave(move |_| v.hover_at(None));
        obj.add_controller(motion);
        obj.kick();
        obj
    }

    /// Replace everything on screen. `cover: None` keeps a neutral placeholder.
    pub fn set_track(&self, meta: Meta, cover: Option<&image::RgbaImage>, lyrics: Option<Lyrics>, duration: f32) {
        // `None` keeps the current cover so a track change can cross-fade to the real one when it arrives.
        if cover.is_some() { self.set_cover(cover); }
        self.with(|s| {
            s.meta = meta;
            s.scene.duration = duration.max(1.0);
            s.key = (0, 0); // force text layouts to rebuild
        });
        self.set_lyrics(lyrics);
    }

    pub fn set_cover(&self, cover: Option<&image::RgbaImage>) {
        let ph;
        let img = match cover { Some(c) => c, None => { ph = placeholder_cover(); &ph } };
        let bd = palette::backdrop(img);
        let (tex, back, colors, luma) = (texture(img), texture(&bd), palette::extract(img), palette::luma(&bd));
        self.with(|s| {
            s.cover = tex; s.backdrop = back; s.colors = colors; s.luma = luma;
            if s.tr.dir != 0.0 && s.tr.cover_t.is_none() { s.tr.cover_t = Some(0.0); }
        });
        self.kick();
        self.queue_draw();
    }

    /// Switch look (Spotlight / Plain), sizes, colours and animation tuning. Takes effect on the next frame.
    pub fn set_style(&self, style: LyricsStyle) {
        self.with(|s| {
            s.scene.scroller.set_tuning(style.cascade_delay, style.spring_stiffness);
            s.style = style;
            s.key = (0, 0); // relayout: sizes, wrapping and line positions change
        });
        self.kick();
        self.queue_draw();
    }

    pub fn style(&self) -> LyricsStyle {
        self.with(|s| s.style.clone())
    }

    pub fn set_lyrics(&self, lyrics: Option<Lyrics>) {
        self.with(|s| {
            let (lines, sync) = match lyrics { Some(l) if !l.is_empty() => (l.lines, l.sync), _ => (Vec::new(), Sync::Line) };
            let times: Vec<f32> = lines.iter().map(|l| if sync == Sync::Plain { f32::MAX } else { l.time }).collect();
            s.word_ranges = lines.iter().map(word_ranges).collect();
            s.attr_line = None;
            s.hit = vec![[0.0; 4]; lines.len()];
            s.heights = vec![0.0; lines.len()];
            s.tops = vec![0.0; lines.len()];
            s.layouts = Vec::new();
            s.line_tex = Vec::new();
            s.prev_sc = Vec::new();
            s.lines = lines;
            s.sync = sync;
            s.scene.set_lines(times);
            s.key = (0, 0);
            if s.tr.dir != 0.0 && s.tr.lyr_t.is_none() && !s.lines.is_empty() { s.tr.lyr_t = Some(0.0); }
        });
        self.kick();
        self.queue_draw();
    }

    /// Call right before [`LyricsView::set_track`] when the song changes (`dir`: +1 next, -1 previous).
    /// The outgoing cover, backdrop, title and lyric lines are kept and animated out while the new ones animate in.
    pub fn transition(&self, dir: f32) {
        if !self.is_mapped() || self.with(|s| s.layouts.is_empty() && s.title.is_none()) { return; }
        self.with(|s| {
            let [_, vh] = s.lay.viewport;
            let base = s.scene.active.unwrap_or(0) as f32;
            let mut leaving = Vec::new();
            // Plain mode has no leave-out choreography: the old lines simply go.
            for (i, layout) in s.layouts.iter().enumerate().filter(|_| !s.style.is_plain()) {
                let v = s.scene.scroller.vis(i);
                let (lh, y) = (s.heights[i], v.y);
                if s.lines[i].instrumental || y > vh + 20.0 || y + lh < -20.0 { continue; }
                let alpha = v.alpha * edge_fade(&s.lay, y + lh * v.scale / 2.0);
                if alpha < 0.02 { continue; }
                let rtl = pango::find_base_dir(&s.lines[i].text) == pango::Direction::Rtl;
                leaving.push(Leaving { layout: layout.clone(), y, h: lh, sc: v.scale, alpha, blur: v.blur, rtl, rank: (i as f32 - base).abs() });
            }
            s.tr = Trans {
                dir: if dir < 0.0 { -1.0 } else { 1.0 },
                old_cover: Some(s.cover.clone()),
                old_backdrop: Some(s.backdrop.clone()),
                old_colors: s.colors,
                old_luma: s.luma,
                old_title: s.title.take(),
                old_artist: s.artist.take(),
                leaving,
                ..Trans::default()
            };
        });
        self.kick();
    }

    /// The view was just brought on screen (not a track change): the cover, title, progress bar and lyric lines enter one after another.
    pub fn reveal(&self) {
        self.with(|s| {
            s.intro = Some(Intro { t: 0.0, closing: false });
            if s.lines.is_empty() || s.style.is_plain() { return; }
            s.tr.dir = 1.0;
            s.tr.lyr_t = Some(0.0);
        });
        self.kick();
    }

    /// Play the staged choreography backwards (the view is about to be closed). Takes [`LyricsView::CONCEAL_MS`].
    pub fn conceal(&self) {
        self.with(|s| s.intro = Some(Intro { t: 0.0, closing: true }));
        self.kick();
    }

    pub const CONCEAL_MS: u32 = (INTRO_CLOSE * 1000.0) as u32;

    fn hover_at(&self, p: Option<(f32, f32)>) {
        let changed = self.with(|s| {
            let h = p.and_then(|(x, y)| (s.sync != Sync::Plain).then(|| s.hit.iter().position(|r| x >= r[0] && x <= r[0] + r[2] && y >= r[1] && y <= r[1] + r[3])).flatten());
            let changed = h != s.hover;
            s.hover = h;
            changed.then_some(h.is_some())
        });
        if let Some(on) = changed {
            self.set_cursor_from_name(on.then_some("pointer"));
            self.queue_draw();
        }
    }

    /// Drive the lyric position from the audio clock (seconds). Without it the view runs its own simulated clock.
    pub fn set_position_source(&self, f: impl Fn() -> f32 + 'static) {
        self.with(|s| { s.position = Some(Box::new(f)); s.scene.external_clock = true; });
    }

    pub fn set_offset(&self, secs: f32) { self.with(|s| s.offset = secs); self.kick(); }
    pub fn offset(&self) -> f32 { self.with(|s| s.offset) }

    /// Playing state (drives the artwork "breathing", the backdrop drift and whether the frame clock runs).
    pub fn set_playing(&self, playing: bool) {
        self.with(|s| s.scene.playing = playing);
        self.kick();
    }

    pub fn set_duration(&self, secs: f32) { self.with(|s| s.scene.duration = secs.max(1.0)); self.kick(); }

    /// Called with a position in seconds when a lyric line or the progress bar is clicked (external clock only).
    pub fn connect_seek(&self, f: impl Fn(f32) + 'static) { self.with(|s| s.on_seek = Some(Rc::new(f))); }

    /// Reserve a strip below the progress bar for host widgets (overlay them using [`LyricsView::connect_layout`]).
    pub fn set_controls_height(&self, h: f32) { self.with(|s| { s.controls_h = h; s.key = (0, 0); }); self.queue_draw(); }

    /// Called whenever the layout is recomputed (resize): position overlaid controls from `lay.bar` etc.
    pub fn connect_layout(&self, f: impl Fn(&layout::Layout) + 'static) { self.with(|s| s.on_layout = Some(Rc::new(f))); }

    /// Tell the view the audio position changed while paused (a seek): redraw once.
    pub fn poke(&self) { self.kick(); }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(self.imp().st.borrow_mut().as_mut().unwrap())
    }

    pub fn playing(&self) -> bool {
        self.with(|s| s.scene.playing)
    }
    pub fn toggle(&self) {
        self.with(|s| s.scene.toggle());
        self.kick();
    }
    pub fn seek_by(&self, d: f32) {
        self.with(|s| s.scene.seek_by(d));
        self.kick();
    }
    /// Jump without animating the scroll (used for screenshots).
    pub fn jump(&self, t: f32) {
        self.with(|s| {
            s.scene.pos = t;
            s.scene.scroller.snap();
        });
        self.kick();
    }
    fn click(&self, x: f32, y: f32) {
        // Progress bar (generous vertical hit area).
        let bar = self.with(|s| {
            let [bx, by, bw, bh] = s.lay.bar;
            (x >= bx && x <= bx + bw && y >= by - 12.0 && y <= by + bh + 12.0).then(|| (((x - bx) / bw).clamp(0.0, 1.0) * s.scene.duration, s.on_seek.clone(), s.scene.external_clock))
        });
        if let Some((t, cb, external)) = bar {
            if external { if let Some(cb) = cb { cb(t); } } else { self.with(|s| s.scene.pos = t); }
            self.kick();
            return;
        }
        let hit = self.with(|s| s.hit.iter().position(|r| x >= r[0] && x <= r[0] + r[2] && y >= r[1] && y <= r[1] + r[3]));
        let Some(i) = hit else { return };
        let (t, cb, external, plain) = self.with(|s| (s.lines[i].time + 0.01, s.on_seek.clone(), s.scene.external_clock, s.sync == Sync::Plain));
        if plain { return; }
        if external { if let Some(cb) = cb { cb(t + self.offset()); } } else { self.with(|s| s.scene.seek_to_line(i)); }
        self.kick();
    }

    /// (Re)start the frame-clock tick. It stops itself once everything settles.
    pub fn kick(&self) {
        if self.with(|s| s.tick.is_some()) {
            return;
        }
        let id = self.add_tick_callback(|w, clock| {
            let view = w.downcast_ref::<LyricsView>().unwrap();
            view.prepare();
            let t = clock.frame_time();
            let go = view.with(|s| {
                let dt = s.last.map_or(0.0, |l| (t - l) as f32 / 1e6);
                s.last = Some(t);
                s.scene.motion = motion_strength();
                if let Some(pos) = &s.position { s.scene.pos = (pos() - s.offset).max(0.0); }
                s.scene.step(dt, &s.tops, s.lay.anchor_y);
                s.tr.advance(dt.min(0.05));
                if let Some(i) = &mut s.intro { i.t += dt.min(0.05); }
                let go = s.scene.animating() || s.tr.live() || s.intro.is_some_and(|i| i.t < if i.closing { INTRO_CLOSE } else { INTRO_TOTAL });
                if !go {
                    s.tick = None;
                    s.last = None;
                }
                go
            });
            view.queue_draw();
            if go { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
        });
        self.with(|s| s.tick = Some(id));
    }

    /// `plain`: `Some(align)` for a Plain-mode lyric line (no tracking, optional centring); `None` for Spotlight lines and labels.
    fn text(&self, s: &str, px: f32, weight: pango::Weight, width: Option<f32>, plain: Option<Align>) -> pango::Layout {
        let l = self.create_pango_layout(Some(s));
        let mut d = pango::FontDescription::new();
        let stack = "Inter, SF Arabic, Noto Sans Arabic, IBM Plex Sans Arabic, Vazirmatn, sans-serif";
        match self.with(|s| s.style.font_family.clone()) { Some(f) => d.set_family(&format!("{f}, {stack}")), None => d.set_family(stack) }
        d.set_weight(weight);
        d.set_absolute_size(px as f64 * pango::SCALE as f64);
        l.set_font_description(Some(&d));
        if let Some(w) = width {
            l.set_width((w * pango::SCALE as f32) as i32);
            l.set_wrap(pango::WrapMode::WordChar);
            // Arabic fonts have tall line boxes; tighten wrapped lines.
            if pango::find_base_dir(s) == pango::Direction::Rtl { l.set_line_spacing(0.8); }
            if plain.is_none() { l.set_attributes(line_attrs(s, px).as_ref()); }
            // Plain passes its alignment in; Spotlight lines follow the style's too (centred layouts).
            if plain.unwrap_or_else(|| self.with(|st| st.style.align)) == Align::Center { l.set_alignment(pango::Alignment::Center); }
        }
        l
    }

    /// Rebuild text layouts when the size changes.
    fn prepare(&self) {
        let (w, h) = (self.width(), self.height());
        if w == 0 || h == 0 || self.with(|s| s.key == (w, h)) {
            return;
        }
        let style = self.with(|s| s.style.clone());
        let plain = style.is_plain();
        let lay = if plain {
            layout::plain(w as f32, h as f32, &style)
        } else {
            let mut l = layout::compute_with(w as f32, h as f32, 48.0, self.with(|s| s.controls_h), style.layout);
            if let Some(r) = style.cover_radius { l.art_radius = r; }
            if let Some(a) = style.active_font_px { l.lyrics_font = a; l.line_gap = a * 0.55; }
            if let Some(g) = style.line_gap { l.line_gap = g; }
            l
        };
        let (texts, ttl, art): (Vec<String>, String, String) =
            self.with(|s| (s.lines.iter().map(|l| l.text.clone()).collect(), s.meta.title.clone(), s.meta.artist.clone()));
        let lines: Vec<_> = texts.iter().map(|t| if plain {
            self.text(t, lay.lyrics_font, pango::Weight::Semibold, Some(lay.lyrics_w), Some(style.align))
        } else {
            self.text(t, lay.lyrics_font, pango::Weight::Ultrabold, Some(lay.lyrics_w), None)
        }).collect();
        let (title, artist) = if plain { (None, None) } else {
            let title = self.text(&ttl, lay.title_font, pango::Weight::Bold, None, None);
            let artist = self.text(&art, lay.artist_font, pango::Weight::Medium, None, None);
            for l in [&title, &artist] {
                l.set_width((lay.meta_w * pango::SCALE as f32) as i32);
                l.set_ellipsize(pango::EllipsizeMode::End);
            }
            (Some(title), Some(artist))
        };
        self.with(|s| {
            s.key = (w, h);
            s.lay = lay;
            s.heights = lines.iter().map(|l| l.pixel_size().1 as f32).collect();
            // Plain: far lines are drawn smaller, so their slots shrink too (the active line grows into the gap).
            let k = if plain { style.far_ratio() } else { 1.0 };
            let mut y = 0.0;
            s.tops = s.heights.iter().map(|h| { let t = y; y += h * k + lay.line_gap; t }).collect();
            s.line_tex = vec![None; lines.len()];
            s.prev_sc = vec![f32::NAN; lines.len()];
            s.layouts = lines;
            s.attr_line = None;
            s.title = title;
            s.artist = artist;
            s.scene.scroller.snap();
            s.scene.step(0.0, &s.tops, lay.anchor_y);
        });
        if let Some(cb) = self.with(|s| s.on_layout.clone()) { cb(&lay); }
    }

    fn draw(&self, snap: &gtk::Snapshot) {
        self.prepare();
        let (w, h) = (self.width() as f32, self.height() as f32);
        if w == 0.0 {
            return;
        }
        let gpu = self.native().and_then(|n| n.renderer().map(|r| (r, n.surface().map_or(1.0, |sf| sf.scale() as f32))));
        self.with(|s| {
            if s.style.backdrop { draw_background(snap, s, w, h); }
            if !s.style.is_plain() { draw_meta(snap, s); }
            draw_lyrics(snap, s, gpu.as_ref());
        });
    }
}

fn draw_background(snap: &gtk::Snapshot, s: &State, w: f32, h: f32) {
    let t = s.scene.flow;
    if s.style.backdrop_kind != BackdropKind::Cover { return theme_backdrop(snap, s, w, h); }
    let tr = &s.tr;
    match (&tr.old_backdrop, tr.cover_t) {
        // Cross-fade between the two (opaque) backdrops once the new cover has landed; before that the old one stays.
        (Some(old), k) => {
            bg_group(snap, s, old, &tr.old_colors, tr.old_luma, t, w, h);
            if let Some(k) = k {
                snap.push_opacity(smooth(k / BG_D) as f64);
                bg_group(snap, s, &s.backdrop, &s.colors, s.luma, t, w, h);
                snap.pop();
            }
        }
        (None, _) => bg_group(snap, s, &s.backdrop, &s.colors, s.luma, t, w, h),
    }
}

/// `Gradient` / `Solid` backdrops: built from the theme's colours, independent of the artwork.
fn theme_backdrop(snap: &gtk::Snapshot, s: &State, w: f32, h: f32) {
    let [bg, accent] = s.style.palette;
    let mix = |a: [f32; 3], b: [f32; 3], k: f32| [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k];
    let full = rect(0.0, 0.0, w, h);
    let (t, sc) = (s.scene.flow, &s.scene);
    let (energy, pulse, motion) = (sc.energy, sc.pulse, sc.motion);
    snap.append_color(&rgba(bg, 1.0), &full);
    let diag = w.hypot(h);
    if s.style.backdrop_kind == BackdropKind::Solid {
        // One soft glow of the accent behind the lyrics column; nothing moves.
        let at = graphene::Point::new(s.lay.lyrics_x + s.lay.lyrics_w * 0.4, s.lay.anchor_y);
        let r = diag * 0.5;
        snap.append_radial_gradient(&rect(at.x() - r, at.y() - r, r * 2.0, r * 2.0), &at, r, r * 0.8, 0.0, 1.0,
            &[gsk::ColorStop::new(0.0, rgba(accent, 0.20)), gsk::ColorStop::new(1.0, rgba(accent, 0.0))]);
        return;
    }
    // Gradient: a diagonal wash from the background into a tint of the accent, then a few slow blobs that swell with the music.
    snap.append_linear_gradient(&full, &graphene::Point::new(0.0, 0.0), &graphene::Point::new(w, h),
        &[gsk::ColorStop::new(0.0, rgba(mix(bg, accent, 0.10), 1.0)), gsk::ColorStop::new(1.0, rgba(mix(bg, accent, 0.32), 1.0))]);
    let blobs = [accent, mix(accent, bg, 0.45), mix(accent, [1.0, 1.0, 1.0], 0.25), mix(accent, bg, 0.7)];
    let a0 = (0.30 + 0.18 * energy + 0.2 * pulse) * motion.clamp(0.35, 1.0);
    for (i, c) in blobs.into_iter().enumerate() {
        let a = i as f32 * 1.9;
        let (fx, fy) = (0.13 + 0.04 * i as f32, 0.11 + 0.03 * ((i * 3) % 5) as f32);
        let (cx, cy) = (w * (0.5 + 0.42 * (t * fx + a).sin()), h * (0.5 + 0.42 * (t * fy + a * 1.5).cos()));
        let r = w.max(h) * (0.38 + 0.08 * (t * 0.3 + a).sin());
        snap.append_radial_gradient(&rect(cx - r, cy - r, r * 2.0, r * 2.0), &graphene::Point::new(cx, cy), r, r, 0.0, 1.0,
            &[gsk::ColorStop::new(0.0, rgba(c, a0)), gsk::ColorStop::new(0.6, rgba(c, a0 * 0.35)), gsk::ColorStop::new(1.0, rgba(c, 0.0))]);
    }
    // A light veil in the background colour keeps text contrast steady as the blobs drift under it.
    snap.append_color(&rgba(s.style.scrim, 0.14), &full);
}

/// Deterministic 0..1 hash (bokeh placement).
fn hash01(i: u32, k: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B1) ^ k.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// A small tiling noise texture for film grain (made once).
fn grain_texture() -> gdk::Texture {
    thread_local!(static GRAIN: gdk::Texture = {
        let n = 160u32;
        let mut px = Vec::with_capacity((n * n * 4) as usize);
        for i in 0..n * n {
            // Straight-alpha specks, light or dark, each only a few percent opaque.
            let v = if hash01(i, 7) > 0.5 { 255 } else { 0 };
            px.extend_from_slice(&[v, v, v, (hash01(i, 8) * 14.0) as u8]);
        }
        let bytes = glib::Bytes::from_owned(px);
        gdk::MemoryTexture::new(n as i32, n as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, n as usize * 4).upcast()
    });
    GRAIN.with(|g| g.clone())
}

/// The living backdrop, built only from GSK nodes (GTK 4.22's renderer has no GL-shader support):
/// blurred-cover layers, big palette blobs on Lissajous paths, sweeping conic light, light streaks, bokeh,
/// a soft bloom behind the active line, a vignette and grain. `energy` / `pulse` come from the Scene.
fn bg_group(snap: &gtk::Snapshot, s: &State, backdrop: &gdk::Texture, colors: &[[f32; 3]; 4], luma: f32, t: f32, w: f32, h: f32) {
    let sc = &s.scene;
    let (energy, pulse, motion) = (sc.energy, sc.pulse, sc.motion.max(0.35));
    // 0 for a dark cover .. 1 for a light one: light covers dim more and add less light on top, so white lyrics keep their contrast.
    let light = ((luma - 0.18) / 0.30).clamp(0.0, 1.0);
    let add = 1.0 - 0.65 * light;
    let diag = w.hypot(h);
    let full = rect(0.0, 0.0, w, h);
    let tint = |c: [f32; 3], k: f32| [c[0] + (1.0 - c[0]) * k, c[1] + (1.0 - c[1]) * k, c[2] + (1.0 - c[2]) * k];
    snap.append_color(&rgba([0.04, 0.04, 0.05], 1.0), &full);

    // Blurred cover, four layers turning against each other; the zoom breathes with the energy.
    // (deg/s, drift, size, opacity, mirror, phase)
    const LAYERS: [(f32, f32, f32, f32, f32, f32); 4] = [
        (4.0, 0.12, 1.15, 1.0, 1.0, 0.0),
        (-6.0, 0.18, 1.5, 0.55, -1.0, 1.7),
        (9.0, 0.22, 1.9, 0.45, 1.0, 3.1),
        (-3.0, 0.15, 1.3, 0.35, -1.0, 4.6),
    ];
    for (rot, drift, zoom, op, mirror, ph) in LAYERS {
        let size = diag * zoom * (1.0 + 0.05 * (t * 0.5 + ph).sin() * (0.4 + energy));
        snap.save();
        snap.translate(&graphene::Point::new(
            w / 2.0 + (t * 0.31 + ph).sin() * drift * w,
            h / 2.0 + (t * 0.26 + ph * 1.3).cos() * drift * h,
        ));
        snap.rotate(ph * 40.0 + rot * t * 1.6);
        snap.scale(mirror, 1.0);
        snap.push_opacity(op as f64);
        snap.append_scaled_texture(backdrop, gsk::ScalingFilter::Linear, &rect(-size / 2.0, -size / 2.0, size, size));
        snap.pop();
        snap.restore();
    }

    // Palette blobs: big soft ellipses sliding and turning on Lissajous paths. Alpha and size swell with the music.
    let blob_a = (0.58 + 0.28 * energy + 0.25 * pulse) * motion.min(1.0) * (1.0 - 0.3 * light);
    for i in 0..6usize {
        let c = colors[i % 4];
        let a = i as f32 * 1.37;
        let (fx, fy) = (0.21 + 0.05 * i as f32, 0.17 + 0.04 * ((i * 5) % 7) as f32);
        let cx = w * (0.5 + 0.46 * (t * fx + a).sin());
        let cy = h * (0.5 + 0.46 * (t * fy + a * 1.6).cos());
        let r = w.max(h) * (0.34 + 0.12 * (t * 0.4 + a).sin() + 0.07 * energy);
        snap.save();
        snap.translate(&graphene::Point::new(cx, cy));
        snap.rotate(t * (6.0 + 3.0 * i as f32) * if i % 2 == 0 { 1.0 } else { -1.0 });
        snap.scale(1.0, 0.55);
        snap.append_radial_gradient(
            &rect(-r, -r, r * 2.0, r * 2.0),
            &graphene::Point::new(0.0, 0.0),
            r,
            r,
            0.0,
            1.0,
            &[gsk::ColorStop::new(0.0, rgba(c, blob_a)), gsk::ColorStop::new(0.55, rgba(c, blob_a * 0.45)), gsk::ColorStop::new(1.0, rgba(c, 0.0))],
        );
        snap.restore();
    }

    // Sweeping conic light: two counter-rotating fans of palette colour, brighter with energy.
    let conic_a = (0.16 + 0.16 * energy + 0.12 * pulse) * motion.min(1.0) * add;
    for (k, (speed, cxk, cyk)) in [(14.0f32, 0.35f32, 0.4f32), (-9.0, 0.65, 0.6)].into_iter().enumerate() {
        let (c0, c1) = (colors[(k * 2) % 4], colors[(k * 2 + 1) % 4]);
        let center = graphene::Point::new(w * (cxk + 0.08 * (t * 0.23 + k as f32).sin()), h * (cyk + 0.08 * (t * 0.19 + k as f32).cos()));
        let st = |o: f32, c: [f32; 3], a: f32| gsk::ColorStop::new(o, rgba(c, a));
        snap.append_conic_gradient(
            &full,
            &center,
            speed * t,
            &[st(0.0, c0, conic_a), st(0.18, c0, 0.0), st(0.4, c1, conic_a * 0.8), st(0.58, c1, 0.0), st(0.8, tint(c0, 0.5), conic_a * 0.6), st(1.0, c0, conic_a)],
        );
    }

    // Light streaks: wide diagonal bands gliding across.
    for i in 0..3 {
        let x = (((t * (0.020 + 0.009 * i as f32) + i as f32 * 0.37).rem_euclid(1.0)) * 1.8 - 0.4) * w;
        let band = w * (0.22 + 0.06 * i as f32);
        let a = (0.04 + 0.07 * energy + 0.06 * pulse) * motion.min(1.0) * add;
        let c = tint(colors[(i + 1) % 4], 0.55);
        snap.save();
        snap.translate(&graphene::Point::new(x, h / 2.0));
        snap.rotate(-24.0 + 6.0 * i as f32);
        snap.append_linear_gradient(
            &rect(-band, -diag, band * 2.0, diag * 2.0),
            &graphene::Point::new(-band, 0.0),
            &graphene::Point::new(band, 0.0),
            &[gsk::ColorStop::new(0.0, rgba(c, 0.0)), gsk::ColorStop::new(0.5, rgba(c, a)), gsk::ColorStop::new(1.0, rgba(c, 0.0))],
        );
        snap.restore();
    }

    // Bokeh: soft discs rising at three depths (far = small + slow, near = big + quick), twinkling.
    for i in 0..30u32 {
        let depth = (i % 3) as f32;
        let r = (10.0 + 16.0 * hash01(i, 1)) * (1.0 + depth * 0.9) * (w / 1366.0).max(0.6);
        let speed = (0.010 + 0.006 * depth) * (0.8 + energy);
        let y = 1.15 - ((hash01(i, 2) + t * speed).rem_euclid(1.0)) * 1.3;
        let x = hash01(i, 3) + 0.03 * (t * 0.3 + hash01(i, 4) * 6.0).sin();
        let tw = 0.55 + 0.45 * (t * 0.8 + hash01(i, 5) * 40.0).sin();
        let a = (0.14 + 0.06 * depth + 0.05 * pulse) * tw * motion.min(1.0) * add;
        let c = tint(colors[(i % 4) as usize], 0.5);
        snap.append_radial_gradient(
            &rect(x * w - r, y * h - r, r * 2.0, r * 2.0),
            &graphene::Point::new(x * w, y * h),
            r,
            r,
            0.0,
            1.0,
            &[gsk::ColorStop::new(0.0, rgba(c, a * 0.5)), gsk::ColorStop::new(0.78, rgba(c, a)), gsk::ColorStop::new(1.0, rgba(c, 0.0))],
        );
    }

    // Line change: a broad, soft bloom of light behind the active line that swells and fades with `pulse` (no edges).
    if pulse > 0.0 {
        let at = graphene::Point::new(s.lay.lyrics_x + s.lay.lyrics_w * 0.35, s.lay.anchor_y);
        let r = diag * 0.55;
        let c = tint(colors[0], 0.35);
        let a = 0.22 * pulse * motion.min(1.0) * add;
        snap.append_radial_gradient(
            &rect(at.x() - r, at.y() - r, r * 2.0, r * 2.0),
            &at,
            r,
            r * 0.7,
            0.0,
            1.0,
            &[gsk::ColorStop::new(0.0, rgba(c, a)), gsk::ColorStop::new(0.5, rgba(c, a * 0.4)), gsk::ColorStop::new(1.0, rgba(c, 0.0))],
        );
    }

    // Dimming that eases off on each line change, plus a vignette so the text keeps its contrast.
    snap.append_color(&rgba([0.0, 0.0, 0.0], (0.28 - 0.05 * pulse - 0.06 * energy).max(0.1) + 0.46 * light), &full);
    snap.append_radial_gradient(
        &full,
        &graphene::Point::new(w / 2.0, h / 2.0),
        diag * 0.62,
        diag * 0.62,
        0.0,
        1.0,
        &[gsk::ColorStop::new(0.0, rgba([0.0; 3], 0.0)), gsk::ColorStop::new(0.55, rgba([0.0; 3], 0.0)), gsk::ColorStop::new(1.0, rgba([0.0; 3], 0.45 + 0.15 * light))],
    );

    // Film grain (jumps to a new offset ~24 times a second) hides banding in the dark gradients.
    if sc.motion > 0.0 {
        let f = (t * 24.0).floor() as u32;
        let (ox, oy) = (hash01(f, 11) * 160.0, hash01(f, 12) * 160.0);
        snap.push_repeat(&full, Some(&rect(ox, oy, 160.0, 160.0)));
        snap.append_texture(&grain_texture(), &rect(ox, oy, 160.0, 160.0));
        snap.pop();
    }
}

fn draw_meta(snap: &gtk::Snapshot, s: &State) {
    let l = &s.lay;
    let tr = &s.tr;
    let tint = s.style.tint;
    let white = |a: f32| gdk::RGBA::new(tint[0], tint[1], tint[2], a);

    // Artwork: springs between full size (playing) and 90% (paused).
    let sc = s.scene.art_scale.x;
    let size = l.art[2] * sc;
    let (x, y) = (l.art[0] + (l.art[2] - size) / 2.0, l.art[1] + (l.art[2] - size) / 2.0);
    let rr = rrect(x, y, size, size, l.art_radius);
    // (Lyrics-only layout: no artwork at all.)
    let kc = if l.art[2] <= 0.0 { 0.0 } else { intro_k(s, 0.10, 0.55) };
    // Open / close choreography: the artwork group rises, scales up and fades in first.
    snap.save();
    snap.translate(&graphene::Point::new(x + size / 2.0, y + size / 2.0 + (1.0 - kc) * 22.0));
    snap.scale(0.96 + 0.04 * kc, 0.96 + 0.04 * kc);
    snap.translate(&graphene::Point::new(-(x + size / 2.0), -(y + size / 2.0)));
    snap.push_opacity(kc as f64);
    // Breathing glow in the cover's lead colour behind the artwork (stronger with the music, flaring on each line).
    if s.style.backdrop && s.scene.motion > 0.0 {
        let (cx, cy, rad) = (x + size / 2.0, y + size / 2.0, size * 0.95);
        let a = (0.14 + 0.18 * s.scene.energy + 0.12 * s.scene.pulse) * s.scene.motion;
        snap.append_radial_gradient(
            &rect(cx - rad, cy - rad, rad * 2.0, rad * 2.0),
            &graphene::Point::new(cx, cy),
            rad,
            rad,
            0.45,
            1.0,
            &[gsk::ColorStop::new(0.0, rgba(s.colors[0], a)), gsk::ColorStop::new(1.0, rgba(s.colors[0], 0.0))],
        );
    }
    if s.style.cover_shadow { snap.append_outset_shadow(&rr, &gdk::RGBA::new(0.0, 0.0, 0.0, 0.45), 0.0, size * 0.06, 0.0, size * 0.12); }
    // Track change: the old cover slides away against the direction of travel while the new one slides in and settles.
    let cover = |tex: &gdk::Texture, dx: f32, k: f32, op: f32| {
        let sz = size * k;
        let r = rrect(x + (size - sz) / 2.0 + dx, y + (size - sz) / 2.0, sz, sz, l.art_radius * k);
        snap.push_opacity(op as f64);
        snap.push_rounded_clip(&r);
        snap.append_scaled_texture(tex, gsk::ScalingFilter::Linear, &r.bounds());
        snap.pop();
        snap.pop();
    };
    match (&tr.old_cover, tr.cover_t) {
        (Some(old), None) => cover(old, 0.0, 1.0, 1.0),
        (Some(old), Some(c)) => {
            let (o, n) = (ease_in(c / (COVER_D * 0.6)), ease_out(c / COVER_D));
            if o < 1.0 { cover(old, -tr.dir * size * 0.16 * o, 1.0 - 0.06 * o, 1.0 - o); }
            cover(&s.cover, tr.dir * size * 0.16 * (1.0 - n), 0.94 + 0.06 * n, n);
        }
        (None, _) => cover(&s.cover, 0.0, 1.0, 1.0),
    }
    snap.pop();
    snap.restore();

    // Title / artist: old text slides out, new text slides in (a little behind it).
    let (to, tn) = (ease_in(tr.t / 0.25), ease_out((tr.t - 0.2) / 0.5));
    let text = |lay: &Option<pango::Layout>, pos: [f32; 2], a: f32, dx: f32, k: f32| {
        if let Some(lay) = lay {
            if a * k < 0.01 { return; }
            snap.save();
            snap.translate(&graphene::Point::new(pos[0] + dx, pos[1]));
            snap.append_layout(lay, &white(a * k));
            snap.restore();
        }
    };
    let (d, armed) = (tr.dir, tr.dir != 0.0);
    for (i, (old, new, pos, a)) in [(&tr.old_title, &s.title, l.title_pos, 0.95), (&tr.old_artist, &s.artist, l.artist_pos, 0.6)].into_iter().enumerate() {
        let ki = intro_k(s, 0.18 + 0.06 * i as f32, 0.5);
        snap.save();
        snap.translate(&graphene::Point::new(0.0, (1.0 - ki) * 14.0));
        snap.push_opacity(ki as f64);
        text(old, pos, a, -d * 28.0 * to, 1.0 - to);
        text(new, pos, a, if armed { d * 28.0 * (1.0 - tn) } else { 0.0 }, if armed { tn } else { 1.0 });
        snap.pop();
        snap.restore();
    }

    // Progress bar.
    let [bx, by, bw, bh] = l.bar;
    let bar = rrect(bx, by, bw, bh, bh / 2.0);
    snap.push_opacity(intro_k(s, 0.30, 0.45) as f64);
    snap.push_rounded_clip(&bar);
    snap.append_color(&white(0.22), &rect(bx, by, bw, bh));
    snap.append_color(&white(0.85), &rect(bx, by, bw * s.scene.progress(), bh));
    snap.pop();
    snap.pop();
}

/// Fade lines out towards the top / bottom edge of the lyrics area instead of cutting them off.
fn edge_fade(l: &layout::Layout, cy: f32) -> f32 {
    let [_, vh] = l.viewport;
    let (top, bottom) = (l.lyrics_top, l.lyrics_bottom);
    let f = (bottom - top) * 0.14;
    smooth((cy - top) / f) * smooth((bottom - cy) / f)
}

/// Everything that positions one line: scale about the anchored edge, blurred glow copy, then the line itself.
struct Paint<'a> { l: &'a layout::Layout, center: bool }

impl Paint<'_> {
    fn line(&self, snap: &gtk::Snapshot, layout: &pango::Layout, y: f32, lh: f32, sc: f32, rtl: bool, alpha: f32, blur: f32, glow: f32, col: [f32; 4]) {
        let l = self.l;
        snap.save();
        // Scale about the edge the text is anchored to (RTL: right edge; centred plain lines: middle), so lines keep a common anchor.
        let ox = if self.center { l.lyrics_w * 0.5 } else if rtl { l.lyrics_w } else { 0.0 };
        snap.translate(&graphene::Point::new(l.lyrics_x + ox, y + lh / 2.0));
        snap.scale(sc, sc);
        snap.translate(&graphene::Point::new(-ox, -lh / 2.0));
        let draw = |op: f32, blur: f32| {
            snap.push_opacity(op as f64);
            if blur > 0.4 { snap.push_blur(blur as f64); }
            snap.append_layout(layout, &gdk::RGBA::new(col[0], col[1], col[2], col[3]));
            if blur > 0.4 { snap.pop(); }
            snap.pop();
        };
        if glow > 0.02 { draw(glow * 0.55, l.lyrics_font * 0.22); }
        draw(alpha, blur);
        snap.restore();
    }

    /// Same placement as [`Paint::line`], from a cached mask texture tinted with `col`.
    fn line_tex(&self, snap: &gtk::Snapshot, t: &LineTex, y: f32, lh: f32, sc: f32, rtl: bool, alpha: f32, col: [f32; 4]) {
        let l = self.l;
        snap.save();
        let ox = if self.center { l.lyrics_w * 0.5 } else if rtl { l.lyrics_w } else { 0.0 };
        snap.translate(&graphene::Point::new(l.lyrics_x + ox, y + lh / 2.0));
        snap.scale(sc, sc);
        snap.translate(&graphene::Point::new(-ox, -lh / 2.0));
        // White mask → `col`: rgb comes from the offset, alpha (line alpha × colour alpha) scales the mask.
        snap.push_color_matrix(&graphene::Matrix::from_float([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, col[3] * alpha]), &graphene::Vec4::new(col[0], col[1], col[2], 0.0));
        snap.append_texture(&t.tex, &rect(-LineTex::PAD, -LineTex::PAD, t.w, t.h));
        snap.pop();
        snap.restore();
    }

    /// Interlude / intro marker: three dots that fill one after another as `progress` goes 0 → 1.
    fn dots(&self, snap: &gtk::Snapshot, cy: f32, progress: f32, alpha: f32, breath: f32, col: [f32; 4]) {
        let l = self.l;
        let d = l.lyrics_font * 0.30;
        let pulse = 1.0 + 0.08 * breath;
        // Centred plain lines: centre the three-dot group (its first dot sits at `lyrics_x + d`, spacing 1.9d).
        let shift = if self.center { (l.lyrics_w - 3.8 * d) / 2.0 - d } else { 0.0 };
        for k in 0..3 {
            let fill = smooth(progress * 3.0 - k as f32);
            let a = alpha * col[3] * (0.32 + 0.68 * fill);
            let sz = d * pulse * (0.85 + 0.15 * fill);
            let x = l.lyrics_x + shift + d * 0.5 + k as f32 * d * 1.9 - sz / 2.0 + d * 0.5;
            snap.push_rounded_clip(&rrect(x, cy - sz / 2.0, sz, sz, sz / 2.0));
            snap.append_color(&gdk::RGBA::new(col[0], col[1], col[2], a), &rect(x, cy - sz / 2.0, sz, sz));
            snap.pop();
        }
    }
}

/// One lyric line rendered white-on-transparent at its own size, padded by [`LineTex::PAD`] for glyph overhang.
#[derive(Clone)]
struct LineTex { tex: gdk::Texture, w: f32, h: f32 }

impl LineTex {
    const PAD: f32 = 6.0;

    fn render(renderer: &gsk::Renderer, dev: f32, layout: &pango::Layout, w: f32, lh: f32) -> Option<Self> {
        let (tw, th) = (w + 2.0 * Self::PAD, lh + 2.0 * Self::PAD);
        let snap = gtk::Snapshot::new();
        snap.scale(dev, dev);
        snap.translate(&graphene::Point::new(Self::PAD, Self::PAD));
        snap.append_layout(layout, &gdk::RGBA::WHITE);
        let node = snap.to_node()?;
        let tex = renderer.render_texture(&node, Some(&rect(0.0, 0.0, tw * dev, th * dev)));
        Some(Self { tex, w: tw, h: th })
    }
}

fn draw_lyrics(snap: &gtk::Snapshot, s: &mut State, gpu: Option<&(gsk::Renderer, f32)>) {
    let l = s.lay;
    let plain = s.style.is_plain();
    let paint = Paint { l: &l, center: s.style.align == Align::Center };
    let [vw, vh] = l.viewport;
    let breath = s.scene.breath();
    let (clip_top, clip_bottom) = (l.lyrics_top, l.lyrics_bottom);
    // Room for scaled / glowing lines on both sides; the right edge must not depend on `lyrics_x` (pad = 0 would shave RTL text).
    snap.push_clip(&rect(l.lyrics_x - 40.0, clip_top, vw + 40.0 - (l.lyrics_x - 40.0), clip_bottom - clip_top));
    apply_word_alpha(s);
    let (dir, base, pos) = (s.tr.dir, s.scene.active.unwrap_or(0) as i32, s.scene.pos);
    let white = [s.style.tint[0], s.style.tint[1], s.style.tint[2], 1.0f32];
    let (blur_k, glow_k) = (s.style.blur, s.style.glow);
    let intro_col = if plain { s.style.colors.active } else { white };
    let hover_col = if plain { let a = s.style.colors.active; [a[0], a[1], a[2], 0.08] } else { [s.style.tint[0], s.style.tint[1], s.style.tint[2], 0.07] };

    // Plain mode: render one missing line mask per frame, nearest the active line first, so they are ready before a line moves.
    if plain {
        if let Some((r, dev)) = gpu {
            let at = s.scene.active.unwrap_or(0) as i32;
            let next = (-2..=8).map(|d| at + d).filter(|&i| i >= 0 && s.scene.active != Some(i as usize) && (i as usize) < s.layouts.len() && s.line_tex[i as usize].is_none() && !s.lines[i as usize].instrumental).min_by_key(|&i| (i - at).abs());
            if let Some(i) = next { s.line_tex[i as usize] = LineTex::render(r, *dev, &s.layouts[i as usize], l.lyrics_w, s.heights[i as usize]); }
        }
    }

    // Previous song's lines leave first, nearest the active line first.
    for g in &s.tr.leaving {
        let p = ease_in((s.tr.t - g.rank * 0.025) / LEAVE_D);
        let alpha = g.alpha * (1.0 - p);
        if alpha < 0.01 { continue; }
        paint.line(snap, &g.layout, g.y - dir * SLIDE * p, g.h, g.sc, g.rtl, alpha, (g.blur + 8.0 * p) * blur_k, 0.0, white);
    }

    for (i, layout) in s.layouts.iter().enumerate() {
        let v = s.scene.scroller.vis(i);
        let lh = s.heights[i];
        // Spotlight: the scroller's look scaled by the style. Plain: size and colour by distance from the active line.
        let (sc, mut alpha, mut glow, mut blur, col) = if plain {
            let (sc, col) = s.style.plain_look(v.dist, v.focus);
            (sc, 1.0, 0.0, 0.0, col)
        } else {
            (v.scale, v.alpha, v.glow * glow_k, v.blur * blur_k, white)
        };
        let mut y = v.y;
        s.hit[i] = [l.lyrics_x, y, l.lyrics_w, lh * sc];
        // Entrance after a track change: lines rise in with a cascade, soft-focus to sharp.
        if let Some(t) = s.tr.lyr_t {
            let rel = (i as i32 - base).clamp(0, 12) as f32;
            let p = ease_out((t - ENTER_BASE - rel * ENTER_STAGGER) / ENTER_D);
            y += dir * SLIDE * (1.0 - p);
            alpha *= p;
            glow *= p;
            blur += 8.0 * (1.0 - p) * blur_k;
        }
        if s.intro.is_some_and(|i| i.closing) { alpha *= intro_k(s, 0.0, 0.45); glow *= intro_k(s, 0.0, 0.45); }
        let fade = edge_fade(&l, y + lh * sc / 2.0);
        alpha *= fade;
        glow *= fade;
        if y > vh + 20.0 || y + lh < -20.0 || alpha < 0.01 {
            continue;
        }
        if s.hover == Some(i) && s.tr.lyr_t.is_none_or(|t| t > ENTER_BASE + ENTER_D) {
            let r = rrect(l.lyrics_x - 18.0, y - 6.0, l.lyrics_w + 36.0, lh * sc + 12.0, 16.0);
            snap.push_rounded_clip(&r);
            snap.append_color(&gdk::RGBA::new(hover_col[0], hover_col[1], hover_col[2], hover_col[3] * fade), &r.bounds());
            snap.pop();
        }
        if s.lines[i].instrumental {
            let line = &s.lines[i];
            let span = (line.end - line.time).max(0.5);
            let progress = if pos >= line.time { ((pos - line.time) / span).clamp(0.0, 1.0) } else { 0.0 };
            let b = if s.scene.active == Some(i) { breath } else { 0.0 };
            paint.dots(snap, y + lh * sc / 2.0, progress, alpha.max(0.0), b * v.focus, col);
        } else {
            let rtl = pango::find_base_dir(&s.lines[i].text) == pango::Direction::Rtl;
            // Plain mode: while playing (or while a line's size is animating) every line but the active one is drawn from a cached
            // mask instead of live text. GTK re-rasterises a text node's glyphs whenever its scale changes, which cost ~4 ms
            // per frame in each line change; the mask is just a textured quad. Paused and settled lines stay crisp live text.
            let textured = plain && gpu.is_some() && s.scene.active != Some(i) && (s.scene.playing || s.prev_sc.get(i).is_some_and(|p| (p - sc).abs() > 1e-4));
            if textured && s.line_tex[i].is_none() {
                let (r, dev) = gpu.unwrap();
                s.line_tex[i] = LineTex::render(r, *dev, layout, l.lyrics_w, lh);
            }
            match s.line_tex.get(i).and_then(|t| t.as_ref()).filter(|_| textured) {
                Some(t) => paint.line_tex(snap, t, y, lh, sc, rtl, alpha, col),
                None => paint.line(snap, layout, y, lh, sc, rtl, alpha, blur, glow, col),
            }
        }
        if plain { if let Some(p) = s.prev_sc.get_mut(i) { *p = sc; } }
    }

    // Intro: dots above the first line while the song has not started singing.
    if s.sync != Sync::Plain && !s.layouts.is_empty() && !s.lines[0].instrumental {
        let first = s.lines[0].time;
        if first > 3.0 && pos < first {
            let a = ((first - pos) / 0.6).clamp(0.0, 1.0);
            let v = s.scene.scroller.vis(0);
            let gate = s.tr.lyr_t.map_or(1.0, |t| ease_out((t - ENTER_BASE) / ENTER_D));
            paint.dots(snap, v.y - l.lyrics_font * 0.55, pos / first, a * gate * 0.9, breath, intro_col);
        }
    }
    snap.pop();
}

/// Opacity of a sung / not-yet-sung word inside the active line.
const WORD_DIM: f32 = 0.36;

/// Slight negative tracking for big bold lyrics (skipped for RTL: letter-spacing breaks Arabic joining).
fn line_attrs(text: &str, px: f32) -> Option<pango::AttrList> {
    if pango::find_base_dir(text) == pango::Direction::Rtl { return None; }
    let list = pango::AttrList::new();
    list.insert(pango::AttrInt::new_letter_spacing((-px * 0.012 * pango::SCALE as f32) as i32));
    Some(list)
}

fn placeholder_cover() -> image::RgbaImage { image::RgbaImage::from_pixel(32, 32, image::Rgba([46, 46, 54, 255])) }

/// Byte range of every word in `line.text`; empty when the words don't reassemble the text.
fn word_ranges(line: &Line) -> Vec<(u32, u32)> {
    if line.words.is_empty() { return Vec::new(); }
    let joined: String = line.words.iter().map(|w| w.text.as_str()).collect();
    if joined.trim() != line.text.trim() { return Vec::new(); }
    let lead = joined.len() - joined.trim_start().len() + (line.text.len() - line.text.trim_start().len());
    let mut at = 0usize;
    line.words.iter().map(|w| {
        let start = at.saturating_sub(lead).min(line.text.len());
        at += w.text.len();
        (start as u32, at.saturating_sub(lead).min(line.text.len()) as u32)
    }).collect()
}

/// Word-synced lyrics: dim the words of the active line that haven't been sung yet (fading each in as it is reached).
/// Plain mode has no tracking and colours the words with the active colour.
fn base_attrs(s: &State, i: usize) -> Option<pango::AttrList> {
    if s.style.is_plain() { None } else { line_attrs(&s.lines[i].text, s.lay.lyrics_font) }
}

fn apply_word_alpha(s: &mut State) {
    let plain = s.style.is_plain();
    let active = if s.sync == Sync::Rich { s.scene.active } else { None };
    if s.attr_line != active {
        if let Some(prev) = s.attr_line { if s.layouts.get(prev).is_some() { let a = base_attrs(s, prev); s.layouts[prev].set_attributes(a.as_ref()); } }
        s.attr_line = None;
    }
    let Some(i) = active else { return };
    if s.word_ranges.get(i).is_none_or(|r| r.is_empty()) { return; }
    let (line, ranges, t) = (&s.lines[i], &s.word_ranges[i], s.scene.pos);
    let list = if plain { pango::AttrList::new() } else { line_attrs(&line.text, s.lay.lyrics_font).unwrap_or_else(pango::AttrList::new) };
    let c = s.style.colors.active;
    let rgb: [u16; 3] = if plain { std::array::from_fn(|k| (c[k].clamp(0.0, 1.0) * 65535.0) as u16) } else { std::array::from_fn(|k| (s.style.tint[k].clamp(0.0, 1.0) * 65535.0) as u16) };
    // Arabic letters join across syllable boundaries, so a per-syllable alpha would show as different shades inside one word:
    // RTL lines are filled one whole (space-delimited) word at a time.
    let rtl = pango::find_base_dir(&line.text) == pango::Direction::Rtl;
    let mut groups: Vec<(usize, usize, u32, u32)> = Vec::new(); // (first word, last word, start byte, end byte)
    for (w, &(a, b)) in ranges.iter().enumerate() {
        match groups.last_mut() {
            Some(g) if rtl && !line.words[g.1].text.ends_with(char::is_whitespace) => { g.1 = w; g.3 = b; }
            _ => groups.push((w, w, a, b)),
        }
    }
    for &(first, last, a, b) in &groups {
        let p = line.span_progress(first, last, t);
        let p = p * p * (3.0 - 2.0 * p); // smoothstep
        let alpha = WORD_DIM + (1.0 - WORD_DIM) * p;
        // GTK's text renderer only honours the alpha attribute when a foreground colour attribute is present too.
        let mut col = pango::AttrColor::new_foreground(rgb[0], rgb[1], rgb[2]);
        col.set_start_index(a);
        col.set_end_index(b);
        list.insert(col);
        let mut attr = pango::AttrInt::new_foreground_alpha((alpha * 65535.0) as u16);
        attr.set_start_index(a);
        attr.set_end_index(b);
        list.insert(attr);
    }
    s.layouts[i].set_attributes(Some(&list));
    s.attr_line = Some(i);
}

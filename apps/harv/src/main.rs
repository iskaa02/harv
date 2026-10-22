mod actions;
mod components;
mod ctx;
mod palette;
mod effects;
mod fonts;
mod miniplayer;
mod icons;
mod images;
mod pages;
mod player;
mod rows;
mod settings;
mod shortcuts;
mod sidebar;
mod theme;
mod theme_gallery;
mod window;

use adw::prelude::*;
use ctx::{Ctx, Nav};
use gtk::{gio, glib, graphene};
use player_core::{Command, Engine, QueueState, Status, Update};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use store_core::Store;
use ytm_core::{SearchKind, Track, Ytm};

#[derive(Default, Clone)]
struct Opts {
    size: (i32, i32),
    screenshot: Option<String>,
    /// Minimum seconds before a screenshot (network pages need time); it also waits for in-flight loads.
    wait: f32,
    watch: bool,
    /// Theme id or path (overrides the saved preference and `HARV_THEME`).
    theme: Option<String>,
    fullscreen: bool,
    page: Option<String>,
    search: Option<String>,
    album: Option<String>,
    artist: Option<String>,
    playlist: Option<String>,
    podcast: Option<String>,
    play_search: Option<String>,
    /// Screenshot aid: ~12 s after `--play-search`, play this search's first hit too (exercises the track-change animation).
    then_play: Option<String>,
    seek: Option<f64>,
    now_playing: bool,
    /// Dev aid with `--screenshot PREFIX`: instead of one picture, save frames of the full-screen open and close animation (`PREFIX-open-NN.png`, `PREFIX-close-NN.png`).
    anim_frames: bool,
    queue: bool,
    settings: bool,
    /// Open the command palette with this text typed.
    palette: Option<String>,
    /// Screenshot aid: switch to this theme ~2.5 s after startup (exercises live theme switching).
    switch_to: Option<String>,
    /// Screenshot aid: play this album (browse id) as the queue, so queue-driven UI has data.
    play_album: Option<String>,
}

fn parse_args() -> Opts {
    let mut o = Opts { size: (1180, 760), wait: 1.5, ..Default::default() };
    let mut a = std::env::args().skip(1);
    while let Some(k) = a.next() {
        match k.as_str() {
            "--size" => if let Some((w, h)) = a.next().as_deref().and_then(|v| v.split_once('x')) { o.size = (w.parse().unwrap_or(1180), h.parse().unwrap_or(760)); },
            "--screenshot" => o.screenshot = a.next(),
            "--wait" => o.wait = a.next().and_then(|v| v.parse().ok()).unwrap_or(1.5),
            "--watch" => o.watch = true,
            "--theme" => o.theme = a.next(),
            "--fullscreen" => o.fullscreen = true,
            "--page" => o.page = a.next(),
            "--search" => o.search = a.next(),
            "--album" => o.album = a.next(),
            "--artist" => o.artist = a.next(),
            "--playlist" => o.playlist = a.next(),
            "--podcast" => o.podcast = a.next(),
            "--play-search" => o.play_search = a.next(),
            "--seek" => o.seek = a.next().and_then(|v| v.parse().ok()),
            "--now-playing" => o.now_playing = true,
            "--anim-frames" => o.anim_frames = true,
            "--then-play" => o.then_play = a.next(),
            "--queue" => o.queue = true,
            "--settings" => o.settings = true,
            "--palette" => o.palette = a.next(),
            "--switch-to" => o.switch_to = a.next(),
            "--play-album" => o.play_album = a.next(),
            _ => {}
        }
    }
    o
}

fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share"))).unwrap_or_else(|| PathBuf::from(".")).join("harv")
}

/// One-time move from the pre-rename `musssic` / `rawi` folders (data + user themes) so nothing is lost. Returns (old, new) data dirs when it moved them.
fn migrate_legacy() -> Option<(PathBuf, PathBuf)> {
    let xdg = |var: &str, rel: &str| std::env::var_os(var).map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rel))).unwrap_or_else(|| PathBuf::from("."));
    let (cfg, share) = (xdg("XDG_CONFIG_HOME", ".config"), xdg("XDG_DATA_HOME", ".local/share"));
    for old in ["rawi", "musssic"] { if !cfg.join("harv").exists() { let _ = std::fs::rename(cfg.join(old), cfg.join("harv")); } }
    let new = data_dir();
    if new.exists() { return None; }
    for old in ["rawi", "musssic"] {
        let from = share.join(old);
        if !from.is_dir() || std::fs::rename(&from, &new).is_err() { continue; }
        for ext in ["", "-wal", "-shm"] { let _ = std::fs::rename(new.join(format!("{old}.db{ext}")), new.join(format!("harv.db{ext}"))); }
        return Some((from, new));
    }
    None
}

/// Offline copies live here (`<videoId>.<ext>`); removing a download only ever deletes inside this folder.
pub fn downloads_dir() -> PathBuf { data_dir().join("downloads") }

fn main() -> glib::ExitCode {
    // GTK 4.22's Vulkan default costs ~2x the CPU of `gl` on this hardware (see CLAUDE.md); respect an explicit override.
    if std::env::var_os("GSK_RENDERER").is_none() {
        // SAFETY: first thing in main, before any threads exist.
        unsafe { std::env::set_var("GSK_RENDERER", "gl") };
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--validate-theme") => return validate_themes(&args[1..]),
        Some("--install-theme") => return install_theme(&args[1..]),
        _ => {}
    }
    // glibc gives every thread that allocates its own arena (GTK, mpv, tokio, ffmpeg: ~40 here) and keeps freed pages in them;
    // two arenas plus an eager trim cut ~20 MB of resident memory with no measurable cost for a UI app.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: mallopt only sets allocator tunables; called before any thread exists.
    unsafe { libc::mallopt(libc::M_ARENA_MAX, 2); libc::mallopt(libc::M_TRIM_THRESHOLD, 256 * 1024); }
    let opts = parse_args();
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().thread_name("harv-io").build().expect("tokio runtime");
    let handle = rt.handle().clone();
    let app = adw::Application::builder().application_id("dev.harv.Harv").flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_activate(move |app| {
        if let Err(e) = build(app, &opts, &handle) { eprintln!("harv: {e:#}"); std::process::exit(1); }
    });
    let code = app.run_with_args::<&str>(&[]);
    drop(rt);
    code
}

/// `harv --validate-theme <id|folder>…`: load each theme (and every variant, in both schemes), print problems and readability warnings.
fn validate_themes(specs: &[String]) -> glib::ExitCode {
    if specs.is_empty() { eprintln!("usage: harv --validate-theme <theme id | theme folder | theme.toml>…"); return glib::ExitCode::from(2); }
    let loader = theme_core::Loader::new(vec![theme_core::user_theme_dir()]);
    let mut failed = false;
    for spec in specs {
        let base = match loader.load(spec) { Ok(t) => t, Err(e) => { println!("✗ {spec}: {e}"); failed = true; continue; } };
        let mut runs: Vec<Option<String>> = vec![None];
        runs.extend(base.variants.keys().cloned().map(Some));
        for v in runs {
            let label = v.as_deref().map_or(spec.clone(), |v| format!("{spec} [{v}]"));
            match loader.load_with(spec, v.as_deref()) {
                Ok(t) => {
                    let light = t.light();
                    let mut notes = t.clone().lint();
                    // The other scheme as well: that is what a user in the opposite mode sees.
                    notes.extend(t.clone().resolved(!light).lint().into_iter().map(|n| format!("(other scheme) {n}")));
                    notes.extend(t.lint_seeds());
                    if notes.is_empty() { println!("✓ {label}"); } else { println!("✓ {label} — {} warning(s)", notes.len()); for n in notes { println!("    ! {n}"); } }
                }
                Err(e) => { println!("✗ {label}: {e}"); failed = true; }
            }
        }
    }
    if failed { glib::ExitCode::FAILURE } else { glib::ExitCode::SUCCESS }
}

/// `harv --install-theme <folder|theme.toml|.zip>`: same as Preferences → Theme → Install.
fn install_theme(args: &[String]) -> glib::ExitCode {
    let Some(src) = args.first() else { eprintln!("usage: harv --install-theme <folder | theme.toml | .zip>"); return glib::ExitCode::from(2) };
    match theme_core::install(std::path::Path::new(src), &theme_core::user_theme_dir()) {
        Ok((id, warn)) => { println!("installed `{id}` into {}", theme_core::user_theme_dir().display()); for w in warn { println!("    ! {w}"); } glib::ExitCode::SUCCESS }
        Err(e) => { eprintln!("install failed: {e}"); glib::ExitCode::FAILURE }
    }
}

fn build(app: &adw::Application, opts: &Opts, rt: &tokio::runtime::Handle) -> anyhow::Result<()> {
    // Matches the installed icon / desktop entry (install.sh); harmless when not installed.
    gtk::Window::set_default_icon_name("dev.harv.Harv");
    // Bundled theme fonts (Figtree, Lexend, Outfit, Red Hat) become visible to Pango before any window exists.
    fonts::register();
    icons::register();

    let db = std::env::var_os("HARV_DB").map(PathBuf::from).unwrap_or_else(|| data_dir().join("harv.db"));
    let moved = migrate_legacy();
    let store = Store::open(&db)?;
    if let Some((old, new)) = moved { store.rebase_downloads(&old.join("downloads").to_string_lossy(), &new.join("downloads").to_string_lossy()); }
    // Theme: --theme, then HARV_THEME (HARV_MODE likewise overrides the saved light/dark/system style), then the saved preference, then the built-in default.
    let skin = theme::Skin::install(opts.watch);
    skin.set_initial_mode(theme::Mode::parse(&std::env::var("HARV_MODE").ok().or_else(|| store.get::<String>("color_mode")).unwrap_or_default()));
    let spec = opts.theme.clone().or_else(|| std::env::var("HARV_THEME").ok()).or_else(|| store.get::<String>("theme")).unwrap_or_else(|| theme_core::DEFAULT.into());
    skin.set_density(store.get::<u32>("ui_density").unwrap_or(1));
    skin.set_variants(store.get::<std::collections::HashMap<String, String>>("theme_variants").unwrap_or_default());
    let active = skin.load_or_default(&spec);
    skin.apply_initial(&active);
    let ytm = Arc::new(rt.block_on(Ytm::new())?);
    let lyrics = lyrics_net::LyricsService::new(store.clone())?;
    // libmpv needs LC_NUMERIC=C and GTK has already initialised by now: Engine::start (via Audio::new) fixes the locale.
    let (engine, updates) = Engine::start(ytm.clone(), rt)?;
    let ctx = Ctx::new(rt.clone(), ytm, store, engine, lyrics, active);
    skin.follow_system(&ctx);
    if opts.watch { skin.watch(&ctx); }
    // Offline copies: every recorded download whose file still exists plays from disk.
    for (vid, path) in ctx.store.downloaded_paths() { let p = PathBuf::from(&path); if p.exists() { ctx.ytm.register_local(&vid, p); } }

    // engine → UI
    {
        let ctx = ctx.clone();
        glib::spawn_future_local(async move {
            while let Ok(u) = updates.recv().await {
                match u { Update::State(s) => ctx.dispatch(s), Update::Error(e) => ctx.toast(&format!("Playback problem: {e}")) }
            }
        });
    }

    let w = window::build(app, &ctx, opts.size);
    let win = w.win.clone();
    restore_session(&ctx);
    effects::restore(&ctx);
    settings::restore(&ctx);
    wire_history_and_persistence(&ctx);
    match rt.block_on(player_core::Mpris::start(ctx.engine.clone())) {
        Ok(m) => ctx.on_state(move |s| m.update(s)),
        Err(e) => eprintln!("mpris unavailable: {e}"),
    }
    win.present();
    if opts.fullscreen { win.fullscreen(); }
    script(&ctx, &w, opts);
    if let Some(path) = opts.screenshot.clone() { screenshot_when_idle(&ctx, &win, path, opts.wait, opts.anim_frames); }
    Ok(())
}

/// Volume / quality / endless / the saved queue (loaded paused at its saved position).
fn restore_session(ctx: &Rc<Ctx>) {
    if let Some(v) = ctx.store.get::<f64>("volume") { ctx.engine.send(Command::SetVolume(v)); }
    if let Some(e) = ctx.store.get::<bool>("endless") { ctx.engine.send(Command::SetEndless(e)); }
    if let Some(e) = ctx.store.get::<bool>("auto_radio") { ctx.engine.send(Command::SetAutoRadio(e)); }
    ctx.engine.send(Command::SetQuality(ctx.quality()));
    if let Some((state, position)) = ctx.store.get::<(QueueState, f64)>("queue") {
        if !state.items.is_empty() { ctx.engine.send(Command::Restore { state, position }); }
    }
}

/// Record plays into the library when a track stops, and keep the saved queue current.
fn wire_history_and_persistence(ctx: &Rc<Ctx>) {
    let last: Rc<RefCell<Option<Track>>> = Rc::default();
    let listened = Rc::new(std::cell::Cell::new(0.0f64));
    {
        let (ctx2, last, listened) = (ctx.clone(), last.clone(), listened.clone());
        ctx.on_state(move |s| {
            let cur = s.current.and_then(|i| s.queue.get(i)).cloned();
            let changed = last.borrow().as_ref().map(|t| &t.video_id) != cur.as_ref().map(|t| &t.video_id);
            if changed {
                if let Some(prev) = last.borrow().as_ref() { let _ = ctx2.store.record_play(prev, listened.get()); }
                *last.borrow_mut() = cur;
                listened.set(0.0);
            }
        });
    }
    // 1 s ticker while playing: remember how far the current track got, and save the queue every 5 s.
    let n = Rc::new(std::cell::Cell::new(0u32));
    let ctx2 = ctx.clone();
    glib::timeout_add_seconds_local(1, move || {
        if ctx2.is_playing() {
            let pos = ctx2.engine.clock().position();
            listened.set(listened.get().max(pos));
            n.set(n.get() + 1);
            if n.get() % 5 == 0 { ctx2.save_queue(pos); }
        }
        glib::ControlFlow::Continue
    });
    // Save on structural changes (queue edits, track changes) too.
    let ctx3 = ctx.clone();
    let key = Rc::new(RefCell::new((0usize, None::<usize>)));
    ctx.on_state(move |s| {
        let k = (s.queue.len(), s.current);
        if *key.borrow() != k { *key.borrow_mut() = k; if s.status != Status::Loading { ctx3.save_queue(0.0); } }
    });
}

fn script(ctx: &Rc<Ctx>, w: &window::Window, o: &Opts) {
    use window::Root;
    use pages::Lib;
    match o.page.as_deref() {
        Some("search") => (w.show_root)(Root::Search),
        Some("favorites") => (w.show_root)(Root::Lib(Lib::Favorites)),
        Some("recent") => (w.show_root)(Root::Lib(Lib::Recent)),
        Some("top") => (w.show_root)(Root::Lib(Lib::MostPlayed)),
        Some("playlists") => (w.show_root)(Root::Lib(Lib::Playlists)),
        Some("following") => (w.show_root)(Root::Lib(Lib::Following)),
        Some("downloads") => (w.show_root)(Root::Lib(Lib::Downloads)),
        Some("charts") => (w.show_root)(Root::Charts),
        _ => {}
    }
    if let Some(q) = &o.search { ctx.navigate(Nav::SearchFor(q.clone())); }
    if let Some(id) = &o.album { ctx.navigate(Nav::Album(id.clone())); }
    if let Some(id) = &o.artist { ctx.navigate(Nav::Artist(id.clone())); }
    if let Some(id) = &o.playlist { ctx.navigate(Nav::Playlist(id.clone())); }
    if let Some(id) = &o.podcast { ctx.navigate(Nav::Podcast(id.clone())); }
    if let Some(q) = o.play_search.clone() {
        let (ytm, c) = (ctx.ytm.clone(), ctx.clone());
        let seek = o.seek;
        ctx.spawn(async move { ytm.search(&q, SearchKind::Songs).await.ok().and_then(|r| r.songs.into_iter().next()) }, move |t| if let Some(t) = t { c.play_one(t); if let Some(s) = seek { c.engine.send(Command::Seek(s)); } });
    }
    if let Some(q) = o.then_play.clone() {
        let c = ctx.clone();
        glib::timeout_add_local_once(Duration::from_secs(12), move || {
            let (ytm, c2) = (c.ytm.clone(), c.clone());
            c.spawn(async move { ytm.search(&q, SearchKind::Songs).await.ok().and_then(|r| r.songs.into_iter().next()) }, move |t| if let Some(t) = t { c2.play_one(t); });
        });
    }
    if o.now_playing { ctx.navigate(Nav::NowPlaying); }
    if o.queue { ctx.navigate(Nav::Queue); }
    if o.settings { ctx.navigate(Nav::Settings); }
    if let Some(q) = &o.palette { palette::toggle(ctx, &w.show_root, if q == "-" { "" } else { q }); }
    if let Some(id) = o.play_album.clone() {
        let (ytm, c) = (ctx.ytm.clone(), ctx.clone());
        let seek = o.seek;
        ctx.spawn(async move { ytm.album(&id).await.ok() }, move |a| if let Some(a) = a { c.play(a.tracks, 0, player_core::QueueKind::Album); if let Some(s) = seek { c.engine.send(Command::Seek(s)); } });
    }
    if let Some(t) = o.switch_to.clone() {
        let ctx = ctx.clone();
        glib::timeout_add_local_once(Duration::from_millis(2500), move || { if let Some(skin) = theme::skin() { if let Err(e) = skin.switch(&ctx, &t) { eprintln!("switch: {e}"); } } });
    }
}

/// Screenshot once the scripted state has loaded: at least `min` seconds, and no async job in flight for ~1 s.
fn screenshot_when_idle(ctx: &Rc<Ctx>, win: &adw::ApplicationWindow, path: String, min: f32, anim: bool) {
    let (ctx, win) = (ctx.clone(), win.clone());
    let (mut elapsed, mut quiet) = (0.0f32, 0u32);
    glib::timeout_add_local(Duration::from_millis(250), move || {
        elapsed += 0.25;
        quiet = if ctx.pending.get() == 0 { quiet + 1 } else { 0 };
        if (quiet >= 4 && elapsed >= min) || elapsed >= 40.0 {
            if anim { anim_frames(&ctx, &win, path.trim_end_matches(".png").to_string()); return glib::ControlFlow::Break; }
            screenshot(&win, &path);
            std::process::exit(0);
        }
        glib::ControlFlow::Continue
    });
}

/// Open the full-screen view, grab a frame every ~33 ms for ~1 s, then close it and do the same.
fn anim_frames(ctx: &Rc<Ctx>, win: &adw::ApplicationWindow, prefix: String) {
    let (ctx, win) = (ctx.clone(), win.clone());
    let n = Cell::new(0u32);
    ctx.navigate(Nav::NowPlaying);
    glib::timeout_add_local(Duration::from_millis(33), move || {
        let i = n.get();
        n.set(i + 1);
        match i {
            0..=29 => screenshot(&win, &format!("{prefix}-open-{i:02}.png")),
            45 => ctx.navigate(Nav::CloseNowPlaying),
            46..=75 => screenshot(&win, &format!("{prefix}-close-{:02}.png", i - 46)),
            76.. => std::process::exit(0),
            _ => {}
        }
        glib::ControlFlow::Continue
    });
}

/// Render the whole window through its own GSK renderer into a PNG.
fn screenshot(win: &adw::ApplicationWindow, path: &str) {
    let (w, h) = (win.width() as f32, win.height() as f32);
    let paintable = gtk::WidgetPaintable::new(Some(win));
    let snap = gtk::Snapshot::new();
    paintable.snapshot(&snap, w as f64, h as f64);
    let Some(node) = snap.to_node() else { eprintln!("skipped {path}: empty snapshot"); return };
    let renderer = gtk::prelude::NativeExt::renderer(win).expect("renderer");
    let tex = renderer.render_texture(node, Some(&graphene::Rect::new(0.0, 0.0, w, h)));
    tex.save_to_png(path).expect("save png");
    eprintln!("saved {path} ({w}x{h})");
}

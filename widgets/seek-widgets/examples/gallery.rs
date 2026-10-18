//! Visual check for the seek widgets: the same three widgets on a light and a dark panel.
//!
//!   cargo run -p seek-widgets --example gallery
//!   cargo run -p seek-widgets --example gallery -- --screenshot /tmp/x.png [--hover 0.4] [--progress 0.38]

use gtk::{gdk, glib, graphene, prelude::*};
use seek_widgets::{pseudo_peaks, OrbitCover, Ring, Waveform};

const CSS: &str = r#"
.panel { padding: 24px 28px; }
.light { background-color: #f4f1ea; color: #1d1b18; }
.dark { background-color: #14161b; color: #e8e6e1; }
.light seek-waveform, .light seek-ring, .light seek-orbit-cover { color: #0f9470; }
.dark seek-waveform, .dark seek-ring, .dark seek-orbit-cover { color: #3ee0a6; }
.light seek-orbit-cover { background-color: #f4f1ea; }
.dark seek-orbit-cover { background-color: #14161b; }
"#;

struct Args {
    screenshot: Option<String>,
    hover: Option<f64>,
    progress: f64,
}

fn parse_args() -> Args {
    let mut a = Args { screenshot: None, hover: None, progress: 0.38 };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--screenshot" => a.screenshot = it.next(),
            "--hover" => a.hover = it.next().and_then(|v| v.parse().ok()),
            "--progress" => a.progress = it.next().and_then(|v| v.parse().ok()).unwrap_or(a.progress),
            _ => {}
        }
    }
    a
}

fn gradient(c1: [u8; 3], c2: [u8; 3]) -> gdk::Texture {
    let n = 96usize;
    let mut px = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let t = (x + y) as f32 / (2 * n) as f32;
            let i = (y * n + x) * 4;
            for c in 0..3 {
                px[i + c] = (c1[c] as f32 + (c2[c] as f32 - c1[c] as f32) * t) as u8;
            }
            px[i + 3] = 255;
        }
    }
    gdk::MemoryTexture::new(n as i32, n as i32, gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from_owned(px), n * 4).upcast()
}

fn panel(class: &str, cover: &gdk::Paintable, args: &Args) -> gtk::Box {
    let peaks = pseudo_peaks("gallery-track-1", 240);

    let orbit = OrbitCover::new();
    orbit.set_cover(Some(cover));
    orbit.set_satellites(vec![
        gradient([255, 120, 90], [250, 200, 80]).upcast(),
        gradient([90, 140, 255], [160, 90, 230]).upcast(),
        gradient([60, 200, 170], [30, 110, 120]).upcast(),
    ]);
    orbit.set_progress(args.progress);
    orbit.set_halign(gtk::Align::Center);

    let wave = Waveform::new();
    wave.set_peaks(&peaks);
    wave.set_progress(args.progress);
    wave.set_hover(args.hover);
    wave.connect_seek(|f| eprintln!("seek {f:.3}"));

    let ring = Ring::new();
    let pic = gtk::Picture::for_paintable(cover);
    pic.set_content_fit(gtk::ContentFit::Cover);
    ring.set_child(Some(&pic));
    ring.set_size(132);
    ring.set_thickness(5.0);
    ring.set_gap(6.0);
    ring.set_progress(args.progress);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    row.set_halign(gtk::Align::Center);
    row.append(&ring);
    let info = gtk::Box::new(gtk::Orientation::Vertical, 4);
    info.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some("Blue in Green"));
    title.add_css_class("title-3");
    title.set_halign(gtk::Align::Start);
    let sub = gtk::Label::new(Some("Miles Davis"));
    sub.set_halign(gtk::Align::Start);
    sub.set_opacity(0.7);
    info.append(&title);
    info.append(&sub);
    row.append(&info);

    let col = gtk::Box::new(gtk::Orientation::Vertical, 22);
    col.add_css_class("panel");
    col.add_css_class(class);
    col.set_size_request(560, -1);
    col.append(&orbit);
    col.append(&wave);
    col.append(&row);
    col
}

fn main() -> glib::ExitCode {
    let args = parse_args();
    let app = gtk::Application::builder().application_id("dev.harv.SeekWidgetsGallery").build();
    let shot = args.screenshot.clone();
    app.connect_activate(move |app| {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("display"),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let cover: gdk::Paintable = gdk::Texture::from_filename(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/cover.jpg")).expect("cover.jpg").upcast();
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.append(&panel("light", &cover, &args));
        root.append(&panel("dark", &cover, &args));

        let win = gtk::ApplicationWindow::builder().application(app).title("Seek widgets").child(&root).build();
        win.set_default_size(1120, 740);
        win.present();

        if let Some(path) = shot.clone() {
            let win = win.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(700), move || {
                let (w, h) = (win.width() as f32, win.height() as f32);
                let paintable = gtk::WidgetPaintable::new(Some(&win));
                let snap = gtk::Snapshot::new();
                paintable.snapshot(&snap, w as f64, h as f64);
                let node = snap.to_node().expect("empty snapshot");
                let renderer = gtk::prelude::NativeExt::renderer(&win).expect("renderer");
                let tex = renderer.render_texture(node, Some(&graphene::Rect::new(0.0, 0.0, w, h)));
                tex.save_to_png(&path).expect("save png");
                eprintln!("saved {path} ({w}x{h})");
                std::process::exit(0);
            });
        }
    });
    app.run_with_args::<&str>(&[])
}

//! Remote thumbnails: memory cache of `gdk::Texture`s + on-disk cache of the downloaded bytes.
//! Downloads run on the tokio runtime; textures are built on the main thread.

use gtk::{gdk, glib, prelude::TextureExt};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

type Callback = Box<dyn FnOnce(Option<gdk::Texture>)>;

/// Decoded thumbnails kept for instant re-display, least recently used dropped first once their pixels pass `BUDGET`.
/// Evicting only drops the cache's reference: a texture still shown by a widget stays alive until that widget lets go.
const BUDGET: usize = 24 << 20;

#[derive(Default)]
struct TextureCache { map: HashMap<String, gdk::Texture>, order: Vec<String>, bytes: usize }

fn cost(t: &gdk::Texture) -> usize { (t.width() * t.height() * 4) as usize }

impl TextureCache {
    fn get(&mut self, url: &str) -> Option<gdk::Texture> {
        let t = self.map.get(url)?.clone();
        if let Some(i) = self.order.iter().position(|u| u == url) { let u = self.order.remove(i); self.order.push(u); }
        Some(t)
    }

    fn insert(&mut self, url: String, t: gdk::Texture) {
        self.bytes += cost(&t);
        if let Some(old) = self.map.insert(url.clone(), t) { self.bytes -= cost(&old); self.order.retain(|u| *u != url); }
        self.order.push(url);
        while self.bytes > BUDGET && self.order.len() > 1 {
            let oldest = self.order.remove(0);
            if let Some(t) = self.map.remove(&oldest) { self.bytes -= cost(&t); }
        }
    }

    fn clear(&mut self) { *self = Self::default(); }
}

#[derive(Clone)]
pub struct Images {
    rt: tokio::runtime::Handle,
    http: reqwest::Client,
    /// Caps parallel downloads so a page of covers can't crowd out the audio stream on a slow link.
    gate: std::sync::Arc<tokio::sync::Semaphore>,
    dir: PathBuf,
    mem: Rc<RefCell<TextureCache>>,
    waiting: Rc<RefCell<HashMap<String, Vec<Callback>>>>,
}

/// Google/YouTube art URLs end in `=w120-h120-…`; ask for the size we actually draw.
pub fn sized(url: &str, px: u32) -> String {
    let Some(i) = url.rfind("=w") else { return url.to_string() };
    let tail = &url[i + 2..];
    let digits = |s: &str| s.chars().take_while(char::is_ascii_digit).count();
    let w = digits(tail);
    if w == 0 { return url.to_string(); }
    let rest = &tail[w..];
    let Some(rest) = rest.strip_prefix("-h") else { return url.to_string() };
    let h = digits(rest);
    if h == 0 { return url.to_string(); }
    format!("{}=w{px}-h{px}{}", &url[..i], &rest[h..])
}

fn hash(s: &str) -> u64 { s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3)) }

impl Images {
    pub fn new(rt: tokio::runtime::Handle) -> Self {
        let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))).unwrap_or_else(|| PathBuf::from("/tmp"));
        let dir = base.join("harv").join("thumbs");
        let _ = std::fs::create_dir_all(&dir);
        let http = reqwest::Client::builder().user_agent("harv/0.1").connect_timeout(std::time::Duration::from_secs(8)).timeout(std::time::Duration::from_secs(20)).build().expect("http client");
        Self { rt, http, gate: std::sync::Arc::new(tokio::sync::Semaphore::new(6)), dir, mem: Rc::default(), waiting: Rc::default() }
    }

    /// Texture for `url` at roughly `px` pixels (callback runs on the main loop, `None` on failure).
    pub fn load(&self, ctx: &Rc<crate::ctx::Ctx>, url: &str, px: u32, cb: impl FnOnce(Option<gdk::Texture>) + 'static) {
        let url = sized(url, px);
        if let Some(t) = self.mem.borrow_mut().get(&url) { cb(Some(t)); return; }
        {
            let mut w = self.waiting.borrow_mut();
            if let Some(list) = w.get_mut(&url) { list.push(Box::new(cb)); return; }
            w.insert(url.clone(), vec![Box::new(cb)]);
        }
        let (http, gate, path) = (self.http.clone(), self.gate.clone(), self.dir.join(format!("{:016x}", hash(&url))));
        let u = url.clone();
        let me = self.clone();
        ctx.spawn(async move { fetch(&http, &gate, &u, &path).await }, move |bytes| {
            let tex = bytes.and_then(|b| gdk::Texture::from_bytes(&glib::Bytes::from_owned(b)).ok());
            if let Some(t) = &tex { me.mem.borrow_mut().insert(url.clone(), t.clone()); }
            for cb in me.waiting.borrow_mut().remove(&url).unwrap_or_default() { cb(tex.clone()); }
        });
    }

    /// Raw bytes (disk-cached) for the now-playing cover, which needs pixels rather than a texture.
    pub fn bytes(&self, url: &str, px: u32) -> impl std::future::Future<Output = Option<Vec<u8>>> + Send + 'static {
        let url = sized(url, px);
        let (http, gate, path) = (self.http.clone(), self.gate.clone(), self.dir.join(format!("{:016x}", hash(&url))));
        async move { fetch(&http, &gate, &url, &path).await }
    }

    /// (files, bytes) of the on-disk thumbnail cache.
    pub fn disk_stats(&self) -> (usize, u64) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return (0, 0) };
        rd.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).fold((0, 0), |(n, b), m| (n + 1, b + m.len()))
    }

    /// Delete the downloaded thumbnails (only files in our own cache directory).
    pub fn clear_disk(&self) {
        if let Ok(rd) = std::fs::read_dir(&self.dir) { for e in rd.flatten() { if e.metadata().is_ok_and(|m| m.is_file()) { let _ = std::fs::remove_file(e.path()); } } }
        self.mem.borrow_mut().clear();
    }

    pub fn rt(&self) -> &tokio::runtime::Handle { &self.rt }
}

async fn fetch(http: &reqwest::Client, gate: &tokio::sync::Semaphore, url: &str, path: &PathBuf) -> Option<Vec<u8>> {
    if let Ok(b) = tokio::fs::read(path).await { if !b.is_empty() { return Some(b); } }
    let _permit = gate.acquire().await.ok()?;
    let r = http.get(url).send().await.ok()?.error_for_status().ok()?;
    let b = r.bytes().await.ok()?.to_vec();
    let _ = tokio::fs::write(path, &b).await;
    Some(b)
}

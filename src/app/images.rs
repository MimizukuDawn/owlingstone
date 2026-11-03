//! Card images, like HDT's banners.
//!
//! - HearthstoneJSON publishes a small banner (tile) per card, and the full card picture (shown when the
//!   mouse is over a tile). `Images::tile(id)` / `Images::card(id)` return the texture if it is ready,
//!   else None and the picture is fetched in the background (saved in the user folder `tiles/` and
//!   `cards/`). Offline: no picture, nothing breaks.
//! - Downloads run on a few worker threads; the window is asked to repaint when one arrives.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

pub const TILE_URL: &str = "https://art.hearthstonejson.com/v1/tiles/{}.png";
/// Full picture: the Battlegrounds one (with the tier), else the normal card (heroes, spells...).
pub const CARD_URLS: [&str; 2] = [
    "https://art.hearthstonejson.com/v1/bgs/latest/enUS/256x/{}.png",
    "https://art.hearthstonejson.com/v1/render/latest/enUS/256x/{}.png",
];
const WORKERS: usize = 4;
const MAX_BYTES: u64 = 2_000_000;
/// Pictures that arrive within this time are drawn by the same repaint.
const REPAINT_BATCH: std::time::Duration = std::time::Duration::from_millis(60);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Tile,
    Card,
}

impl Kind {
    fn folder(self) -> PathBuf {
        owlingstone::paths::user_dir().join(match self {
            Kind::Tile => "tiles",
            Kind::Card => "cards",
        })
    }
    fn urls(self) -> Vec<&'static str> {
        match self {
            Kind::Tile => vec![TILE_URL],
            Kind::Card => CARD_URLS.to_vec(),
        }
    }
}

/// Decoded picture from a worker.
type Done = (Kind, String, Option<ColorImage>);

pub struct Images {
    textures: HashMap<(Kind, String), TextureHandle>,
    pending: HashSet<(Kind, String)>,
    failed: HashSet<(Kind, String)>,
    jobs: Sender<(Kind, String)>,
    done: Receiver<Done>,
}

/// Only plain card ids go into a URL or a file name.
fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn decode(bytes: &[u8]) -> Option<ColorImage> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

fn fetch(kind: Kind, id: &str) -> Option<ColorImage> {
    let path = kind.folder().join(format!("{id}.png"));
    if let Some(img) = std::fs::read(&path).ok().and_then(|b| decode(&b)) {
        return Some(img);
    }
    if std::env::var_os(owlingstone::cards::NO_DOWNLOAD).is_some() {
        return None;
    }
    for url in kind.urls() {
        let Ok(bytes) = owlingstone::net::get(&url.replace("{}", id), MAX_BYTES, 20) else { continue };
        if let Some(img) = decode(&bytes) {
            let _ = std::fs::create_dir_all(kind.folder());
            let _ = std::fs::write(&path, &bytes);
            return Some(img);
        }
    }
    None
}

impl Images {
    pub fn new(ctx: &egui::Context) -> Images {
        let (jobs, rx) = channel::<(Kind, String)>();
        let (tx, done) = channel::<Done>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..WORKERS {
            let (rx, tx, ctx) = (rx.clone(), tx.clone(), ctx.clone());
            let _ = std::thread::Builder::new().name(format!("images {i}")).spawn(move || {
                owlingstone::background::lower_priority();
                loop {
                    let job = { rx.lock().unwrap_or_else(|e| e.into_inner()).recv() };
                    let Ok((kind, id)) = job else { return };
                    let img = fetch(kind, &id);
                    if tx.send((kind, id, img)).is_err() {
                        return;
                    }
                    // A new tier shows ~20 pictures at once: one repaint for the lot, not one each.
                    ctx.request_repaint_after(REPAINT_BATCH);
                }
            });
        }
        Images { textures: HashMap::new(), pending: HashSet::new(), failed: HashSet::new(), jobs, done }
    }

    /// Takes the pictures that arrived (once per frame).
    pub fn receive(&mut self, ctx: &egui::Context) {
        while let Ok((kind, id, img)) = self.done.try_recv() {
            self.pending.remove(&(kind, id.clone()));
            match img {
                Some(img) => {
                    let tex = ctx.load_texture(format!("{kind:?}/{id}"), img, TextureOptions::LINEAR);
                    self.textures.insert((kind, id), tex);
                }
                None => {
                    self.failed.insert((kind, id));
                }
            }
        }
    }

    pub fn get(&mut self, kind: Kind, id: &str) -> Option<TextureHandle> {
        let key = (kind, id.to_string());
        if let Some(t) = self.textures.get(&key) {
            return Some(t.clone());
        }
        if safe_id(id) && !self.pending.contains(&key) && !self.failed.contains(&key) {
            self.pending.insert(key.clone());
            let _ = self.jobs.send(key);
        }
        None
    }

    pub fn tile(&mut self, id: &str) -> Option<TextureHandle> {
        self.get(Kind::Tile, id)
    }

    pub fn card(&mut self, id: &str) -> Option<TextureHandle> {
        self.get(Kind::Card, id)
    }
}

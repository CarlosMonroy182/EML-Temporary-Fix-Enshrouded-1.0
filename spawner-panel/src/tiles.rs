use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use eframe::egui::{self, TextureHandle};
use serde_json::Value;

use crate::icons::{fetch_bytes_retry, icons_dir};

pub const ORIGIN: f64 = 32512.0 * 256.0;
pub const PX_PER_METER: f64 = 2.82253;
const OFFSET_X: f64 = 19022.26;
const OFFSET_Y: f64 = 42863.21;
const CALIBRATION_FILE: &str = "calibration.json";
pub const MIN_ZOOM: u8 = 8;
pub const MAX_ZOOM: u8 = 16;
pub const TILE_URL: &str = "https://tiles.mapgenie.io/games/enshrouded/embervale/default-v7";
const PAGE_URL: &str = "https://mapgenie.io/enshrouded/maps/embervale";
const DATA_URL: &str = "https://mapgenie.io/api/v1/maps/581/data";
const MARKERS_FILE: &str = "markers.json";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    pub scale: f64,
    pub off_x: f64,
    pub off_y: f64,
}

pub const DEFAULT_CALIBRATION: Calibration = Calibration { scale: PX_PER_METER, off_x: OFFSET_X, off_y: OFFSET_Y };

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub x: f64,
    pub z: f64,
    pub px: f64,
    pub py: f64,
}

static CALIBRATION: Mutex<Calibration> = Mutex::new(DEFAULT_CALIBRATION);

pub fn calibration() -> Calibration {
    CALIBRATION.lock().map(|c| *c).unwrap_or(DEFAULT_CALIBRATION)
}

pub fn set_calibration(c: Calibration) {
    if let Ok(mut current) = CALIBRATION.lock() {
        *current = c;
    }
}

pub fn map_world_to_px(c: Calibration, x: f64, z: f64) -> (f64, f64) {
    (ORIGIN + c.scale * x + c.off_x, ORIGIN - c.scale * z + c.off_y)
}

pub fn map_px_to_world(c: Calibration, px: f64, py: f64) -> (f64, f64) {
    ((px - ORIGIN - c.off_x) / c.scale, -(py - ORIGIN - c.off_y) / c.scale)
}

pub fn world_to_px(x: f64, z: f64) -> (f64, f64) {
    map_world_to_px(calibration(), x, z)
}

pub fn px_to_world(px: f64, py: f64) -> (f64, f64) {
    map_px_to_world(calibration(), px, py)
}

pub fn fit_calibration(anchors: &[Anchor]) -> Calibration {
    let Some(last) = anchors.last() else { return DEFAULT_CALIBRATION };
    let single = Calibration {
        scale: PX_PER_METER,
        off_x: last.px - ORIGIN - PX_PER_METER * last.x,
        off_y: last.py - ORIGIN + PX_PER_METER * last.z,
    };
    if anchors.len() < 2 {
        return single;
    }
    let n = anchors.len() as f64;
    let (mut sx, mut sz) = (0.0, 0.0);
    for a in anchors {
        sx += a.x;
        sz += a.z;
    }
    let (mx, mz) = (sx / n, sz / n);
    let (mut spx, mut spy) = (0.0, 0.0);
    for a in anchors {
        spx += a.px - ORIGIN;
        spy += a.py - ORIGIN;
    }
    let (mpx, mpy) = (spx / n, spy / n);
    let (mut num, mut den) = (0.0, 0.0);
    for a in anchors {
        let (dx, dz) = (a.x - mx, a.z - mz);
        num += dx * (a.px - ORIGIN - mpx) - dz * (a.py - ORIGIN - mpy);
        den += dx * dx + dz * dz;
    }
    if den < 400.0 * n {
        return single;
    }
    let scale = (num / den).clamp(2.0, 4.0);
    Calibration { scale, off_x: mpx - scale * mx, off_y: mpy + scale * mz }
}

pub fn calibration_path(dir: &Path) -> PathBuf {
    icons_dir(dir).join("map").join(CALIBRATION_FILE)
}

pub fn load_anchors(dir: &Path) -> Vec<Anchor> {
    let Some(v) = std::fs::read_to_string(calibration_path(dir)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return Vec::new() };
    let Some(list) = v.get("anchors").and_then(|a| a.as_array()) else { return Vec::new() };
    list.iter()
        .filter_map(|a| {
            let a = a.as_array()?;
            Some(Anchor { x: a.first()?.as_f64()?, z: a.get(1)?.as_f64()?, px: a.get(2)?.as_f64()?, py: a.get(3)?.as_f64()? })
        })
        .collect()
}

pub fn save_anchors(dir: &Path, anchors: &[Anchor]) -> Result<(), String> {
    let path = calibration_path(dir);
    if anchors.is_empty() {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let list: Vec<Value> = anchors.iter().map(|a| serde_json::json!([a.x, a.z, a.px, a.py])).collect();
    std::fs::write(path, serde_json::json!({ "anchors": list }).to_string()).map_err(|e| e.to_string())
}

pub fn latlng_to_px(lat: f64, lng: f64) -> (f64, f64) {
    let size = 256.0 * 65536.0;
    let x = (lng + 180.0) / 360.0 * size;
    let phi = lat.to_radians();
    let y = (0.5 - ((std::f64::consts::FRAC_PI_4 + phi / 2.0).tan()).ln() / (2.0 * std::f64::consts::PI)) * size;
    (x, y)
}

pub fn tile_path(dir: &Path, z: u8, x: u32, y: u32) -> PathBuf {
    icons_dir(dir).join("map_tiles").join(z.to_string()).join(format!("{y}_{x}.jpg"))
}

pub type TileKey = (u8, u32, u32);

pub enum Slot {
    Pending,
    Ready(TextureHandle),
    Missing,
}

pub struct Decoded {
    pub key: TileKey,
    pub image: Option<(usize, usize, Vec<u8>)>,
}

struct Queue {
    items: Mutex<Vec<TileKey>>,
    wake: Condvar,
}

pub struct TileCache {
    pub slots: HashMap<TileKey, Slot>,
    queue: Arc<Queue>,
    results: Receiver<Decoded>,
    pub downloaded: Arc<std::sync::atomic::AtomicUsize>,
}

impl TileCache {
    pub fn start(dir: &Path) -> Self {
        let queue = Arc::new(Queue { items: Mutex::new(Vec::new()), wake: Condvar::new() });
        let (tx, results) = channel::<Decoded>();
        let downloaded = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for n in 0..4 {
            let queue = queue.clone();
            let tx = tx.clone();
            let dir = dir.to_path_buf();
            let counter = downloaded.clone();
            let _ = std::thread::Builder::new().name(format!("m182-tiles-{n}")).spawn(move || loop {
                let key = {
                    let mut items = queue.items.lock().unwrap();
                    while items.is_empty() {
                        items = queue.wake.wait(items).unwrap();
                    }
                    items.pop().unwrap()
                };
                let (z, x, y) = key;
                let path = tile_path(&dir, z, x, y);
                let mut bytes = std::fs::read(&path).ok().filter(|b| b.len() > 100);
                if bytes.is_none() {
                    let url = format!("{TILE_URL}/{z}/{y}/{x}.jpg");
                    if let Some(b) = fetch_bytes_retry(&url, 2).filter(|b| b.len() > 100 && b.starts_with(&[0xFF, 0xD8])) {
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let _ = std::fs::write(&path, &b);
                        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        bytes = Some(b);
                    }
                }
                let image = bytes.and_then(|b| image::load_from_memory(&b).ok()).map(|img| {
                    let rgba = img.to_rgba8();
                    (rgba.width() as usize, rgba.height() as usize, rgba.into_raw())
                });
                let _ = tx.send(Decoded { key, image });
            });
        }
        Self { slots: HashMap::new(), queue, results, downloaded }
    }

    pub fn request(&mut self, key: TileKey) {
        if self.slots.contains_key(&key) {
            return;
        }
        self.slots.insert(key, Slot::Pending);
        self.queue.items.lock().unwrap().push(key);
        self.queue.wake.notify_one();
    }

    pub fn poll(&mut self, ctx: &egui::Context) -> bool {
        let mut any = false;
        while let Ok(done) = self.results.try_recv() {
            any = true;
            let slot = match done.image {
                Some((w, h, rgba)) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                    Slot::Ready(ctx.load_texture(format!("tile-{}-{}-{}", done.key.0, done.key.1, done.key.2), image, egui::TextureOptions::LINEAR))
                }
                None => Slot::Missing,
            };
            self.slots.insert(done.key, slot);
        }
        any
    }

    pub fn pending(&self) -> usize {
        self.slots.values().filter(|s| matches!(s, Slot::Pending)).count()
    }

    pub fn trim(&mut self, keep_zoom: u8) {
        if self.slots.len() > 700 {
            self.slots.retain(|k, s| k.0 <= 10 || k.0 == keep_zoom || matches!(s, Slot::Pending));
        }
    }
}

#[derive(Clone, Debug)]
pub struct Category {
    pub id: u64,
    pub group: String,
}

#[derive(Clone, Debug)]
pub struct Marker {
    pub category: u64,
    pub title: String,
    pub x: f64,
    pub z: f64,
}

#[derive(Default)]
pub struct Markers {
    pub categories: Vec<Category>,
    pub markers: Vec<Marker>,
}

pub fn parse_markers_file(text: &str) -> Markers {
    let mut out = Markers::default();
    let Ok(v) = serde_json::from_str::<Value>(text) else { return out };
    if let Some(cats) = v.get("cats").and_then(|c| c.as_array()) {
        for c in cats {
            let Some(a) = c.as_array() else { continue };
            if a.len() < 3 {
                continue;
            }
            out.categories.push(Category { id: a[0].as_u64().unwrap_or(0), group: a[2].as_str().unwrap_or("").to_string() });
        }
    }
    if let Some(locs) = v.get("locs").and_then(|c| c.as_array()) {
        for l in locs {
            let Some(a) = l.as_array() else { continue };
            if a.len() < 4 {
                continue;
            }
            let (Some(x), Some(z)) = (a[2].as_f64(), a[3].as_f64()) else { continue };
            out.markers.push(Marker { category: a[0].as_u64().unwrap_or(0), title: a[1].as_str().unwrap_or("").to_string(), x, z });
        }
    }
    out
}

fn as_f64(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub fn build_markers(page: &str, data: &Value) -> Option<Value> {
    let start = page.find("window.mapData")?;
    let rest = &page[start..];
    let brace = rest.find('{')?;
    let mut stream = serde_json::Deserializer::from_str(&rest[brace..]).into_iter::<Value>();
    let map_data = stream.next()?.ok()?;
    let mut cats = Vec::new();
    let groups = map_data.get("groups").and_then(|g| g.as_array());
    let mut group_titles: HashMap<u64, String> = HashMap::new();
    if let Some(groups) = groups {
        for g in groups {
            if let (Some(id), Some(t)) = (g.get("id").and_then(|i| i.as_u64()), g.get("title").and_then(|t| t.as_str())) {
                group_titles.insert(id, t.to_string());
            }
        }
    }
    let list: Vec<&Value> = match map_data.get("categories") {
        Some(Value::Array(a)) => a.iter().collect(),
        Some(Value::Object(o)) => o.values().collect(),
        _ => Vec::new(),
    };
    let mut seen = std::collections::HashSet::new();
    for c in list {
        let Some(id) = c.get("id").and_then(|i| i.as_u64()) else { continue };
        if !seen.insert(id) {
            continue;
        }
        let title = c.get("title").and_then(|t| t.as_str()).unwrap_or("");
        let group = c.get("group_id").and_then(|g| g.as_u64()).and_then(|g| group_titles.get(&g)).cloned().unwrap_or_default();
        cats.push(serde_json::json!([id, title, group]));
    }
    let mut locs = Vec::new();
    for l in data.get("locations")?.as_array()? {
        let (Some(lat), Some(lng)) = (as_f64(l.get("latitude")), as_f64(l.get("longitude"))) else { continue };
        let (px, py) = latlng_to_px(lat, lng);
        let (x, z) = map_px_to_world(DEFAULT_CALIBRATION, px, py);
        let category = l.get("category_id").and_then(|c| c.as_u64()).unwrap_or(0);
        let title = l.get("title").and_then(|t| t.as_str()).unwrap_or("");
        locs.push(serde_json::json!([category, title, (x * 10.0).round() / 10.0, (z * 10.0).round() / 10.0]));
    }
    if cats.is_empty() || locs.is_empty() {
        return None;
    }
    Some(serde_json::json!({"cats": cats, "locs": locs}))
}

pub fn markers_path(dir: &Path) -> PathBuf {
    icons_dir(dir).join("map").join(MARKERS_FILE)
}

pub fn load_markers(dir: &Path) -> Markers {
    std::fs::read_to_string(markers_path(dir)).map(|t| parse_markers_file(&t)).unwrap_or_default()
}

pub fn spawn_marker_download(dir: &Path) -> Receiver<bool> {
    let (tx, rx) = channel();
    let dir = dir.to_path_buf();
    let _ = std::thread::Builder::new().name("m182-markers".into()).spawn(move || {
        let ok = (|| {
            let page = String::from_utf8_lossy(&fetch_bytes_retry(PAGE_URL, 3)?).to_string();
            std::thread::sleep(Duration::from_millis(600));
            let data: Value = serde_json::from_slice(&fetch_bytes_retry(DATA_URL, 3)?).ok()?;
            let built = build_markers(&page, &data)?;
            let path = markers_path(&dir);
            std::fs::create_dir_all(path.parent()?).ok()?;
            std::fs::write(path, built.to_string()).ok()?;
            Some(())
        })()
        .is_some();
        let _ = tx.send(ok);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_pixel_roundtrip_and_anchor() {
        let (px, py) = world_to_px(5870.0, 2638.0);
        assert!((px - ORIGIN - 35590.5).abs() < 1.0, "{px}");
        assert!((py - ORIGIN - 35417.0).abs() < 1.5, "{py}");
        let (x, z) = px_to_world(px, py);
        assert!((x - 5870.0).abs() < 1e-6 && (z - 2638.0).abs() < 1e-6);
        assert_eq!(((px / 256.0).floor() as u32, (py / 256.0).floor() as u32), (32651, 32650));
    }

    #[test]
    fn one_anchor_shifts_the_map_and_keeps_the_scale() {
        let truth = Calibration { scale: PX_PER_METER, off_x: OFFSET_X + 120.0, off_y: OFFSET_Y - 80.0 };
        let (px, py) = map_world_to_px(truth, 3508.0, 1982.0);
        let fitted = fit_calibration(&[Anchor { x: 3508.0, z: 1982.0, px, py }]);
        let (qx, qy) = map_world_to_px(fitted, 3508.0, 1982.0);
        assert!((qx - px).abs() < 1e-6 && (qy - py).abs() < 1e-6);
        assert_eq!(fitted.scale, PX_PER_METER);
        let (wx, wz) = map_px_to_world(fitted, px, py);
        assert!((wx - 3508.0).abs() < 1e-6 && (wz - 1982.0).abs() < 1e-6);
    }

    #[test]
    fn two_far_anchors_recover_scale_and_offset() {
        let truth = Calibration { scale: 3.1, off_x: 18000.0, off_y: 41000.0 };
        let points = [(3508.0, 1982.0), (5870.0, 2638.0), (900.0, 6000.0)];
        let anchors: Vec<Anchor> = points.iter().map(|(x, z)| {
            let (px, py) = map_world_to_px(truth, *x, *z);
            Anchor { x: *x, z: *z, px, py }
        }).collect();
        let fitted = fit_calibration(&anchors);
        assert!((fitted.scale - 3.1).abs() < 1e-6, "{fitted:?}");
        assert!((fitted.off_x - 18000.0).abs() < 1e-3 && (fitted.off_y - 41000.0).abs() < 1e-3);
        let near = [anchors[0], Anchor { x: 3510.0, z: 1985.0, ..anchors[0] }];
        assert_eq!(fit_calibration(&near).scale, PX_PER_METER, "anchors that are too close keep the default scale");
        assert_eq!(fit_calibration(&[]), DEFAULT_CALIBRATION);
    }

    #[test]
    fn anchors_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("m182_cal_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let anchors = vec![Anchor { x: 1.0, z: 2.0, px: 3.0, py: 4.0 }];
        save_anchors(&dir, &anchors).unwrap();
        assert_eq!(load_anchors(&dir), anchors);
        save_anchors(&dir, &[]).unwrap();
        assert!(load_anchors(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mercator_origin_tile() {
        let (px, py) = latlng_to_px(1.40625, -1.40625);
        assert!((px / 256.0 - 32512.0).abs() < 0.01 && (py / 256.0 - 32512.0).abs() < 0.05, "{px} {py}");
    }

    #[test]
    fn marker_build_and_parse() {
        let page = r#"<script>window.mapData = {"groups":[{"id":1,"title":"Locations"}],"categories":[{"id":10,"group_id":1,"title":"Cave"}],"x":1};</script>"#;
        let data: Value = serde_json::from_str(r#"{"locations":[{"category_id":10,"title":"Dark Cave","latitude":"0.5351","longitude":-0.8902}]}"#).unwrap();
        let built = build_markers(page, &data).unwrap();
        let parsed = parse_markers_file(&built.to_string());
        assert_eq!(parsed.categories[0].group, "Locations");
        assert_eq!(parsed.markers.len(), 1);
        let m = &parsed.markers[0];
        assert!(m.x > 0.0 && m.x < 8300.0 && m.z > 0.0 && m.z < 8300.0, "{} {}", m.x, m.z);
    }
}

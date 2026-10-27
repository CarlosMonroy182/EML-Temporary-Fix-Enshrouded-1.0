//! M182 Eternal Pillar (stage 1, observe only): writes what the server is given when the Eternal
//! Pillar Of Flames creates its fire zone, plus who is connected, so the caster can be recognised.
//! Nothing is changed in the game.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use super::pillar_cfg::{self, PillarConfig};
use super::spawn_probe::{hex, read_bytes, u32_at, u64_at};

static WANTED: AtomicBool = AtomicBool::new(false);
static DUMPS: AtomicU32 = AtomicU32::new(0);
static DECISIONS: AtomicU32 = AtomicU32::new(0);
static CONFIG: Mutex<Option<PillarConfig>> = Mutex::new(None);
static ZONE: Mutex<Option<[u8; 16]>> = Mutex::new(None);

fn load() -> PillarConfig {
    let Ok(dir) = std::env::current_dir() else { return PillarConfig::default() };
    std::fs::read_to_string(dir.join(pillar_cfg::CONFIG_FILE)).map(|t| PillarConfig::parse(&t)).unwrap_or_default()
}

/// Reads the settings once; true when the module has to see entity creations.
pub fn wanted() -> bool {
    let cfg = load();
    let wanted = cfg.wanted();
    WANTED.store(wanted, Ordering::SeqCst);
    if let Ok(mut c) = CONFIG.lock() {
        *c = Some(cfg);
    }
    if let Ok(mut z) = ZONE.lock() {
        *z = super::config::parse_guid(pillar_cfg::ETERNAL_ZONE_TEMPLATE);
    }
    wanted
}

pub(super) fn log(text: &str) {
    let Ok(dir) = std::env::current_dir() else { return };
    let dir = dir.join("m182_pillar");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("pillar.log")) {
        let _ = f.write_all(text.as_bytes());
    }
}

const HASH_CURRENT_TRANSFORM: u32 = 0x027a_c564;
const HASH_LIFE_TIME: u32 = 0xf22c_253c;
const TRANSFORM_SIZE: usize = 56;
const SCALE_AT: usize = 40;
const METERS: f64 = 4_294_967_296.0;
const MAX_CASTER_DISTANCE_M: f64 = 45.0;

pub(super) fn config() -> PillarConfig {
    CONFIG.lock().ok().and_then(|c| c.clone()).unwrap_or_default()
}

/// True when the pillar at (x, z) was cast by an administrator. Only players within casting range
/// count; when administrators and other players are mixed in that range the caster cannot be told
/// apart, so the pillar stays normal unless `nearest_decides` is set.
fn cast_by_admin(manager: usize, x: f64, z: f64, nearest_decides: bool) -> (bool, String) {
    let table = super::table::entries();
    if table.is_empty() {
        return (false, "player table not available".to_string());
    }
    let mut near: Vec<(f64, bool, String)> = Vec::new();
    let mut far: Option<(f64, String)> = None;
    let mut unknown = 0usize;
    for e in &table {
        let name = e.name.clone().unwrap_or_default();
        let Some(t) = super::spawner::transform_of(manager, e.entity()) else {
            unknown += 1;
            continue;
        };
        let (px, pz) = (u64_at(&t, 0) as i64 as f64 / METERS, u64_at(&t, 16) as i64 as f64 / METERS);
        let d = ((px - x).powi(2) + (pz - z).powi(2)).sqrt();
        let admin = super::roster::is_admin(e.permissions);
        if d <= MAX_CASTER_DISTANCE_M {
            near.push((d, admin, name));
        } else if far.as_ref().is_none_or(|(best, _)| d < *best) {
            far = Some((d, name));
        }
    }
    near.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let list = near.iter().map(|(d, a, n)| format!("{n} {d:.1} m admin={a}")).collect::<Vec<_>>().join(", ");
    if near.is_empty() {
        if unknown > 0 && table.len() == 1 {
            let admin = super::roster::is_admin(table[0].permissions);
            return (admin, format!("only player {} (position unknown, admin: {admin})", table[0].name.clone().unwrap_or_default()));
        }
        return match far {
            Some((d, name)) => (false, format!("nearest player {name} is {d:.1} m away, too far")),
            None => (false, "player positions unknown".to_string()),
        };
    }
    let admins = near.iter().filter(|p| p.1).count();
    if admins == near.len() {
        return (true, format!("near the cast: {list}"));
    }
    if admins == 0 {
        return (false, format!("near the cast: {list}"));
    }
    if nearest_decides {
        return (near[0].1, format!("administrators and players mixed, nearest decides: {list}"));
    }
    (false, format!("administrators and players mixed, caster unknown, left normal: {list}"))
}

/// Called before the game creates an entity from `desc`. Returns the descriptor the game must use.
pub fn before(manager: usize, desc: usize) -> usize {
    if !WANTED.load(Ordering::Relaxed) || desc < 0x10000 {
        return desc;
    }
    let Some(head) = read_bytes(desc, 0x30) else { return desc };
    if head.len() < 0x28 {
        return desc;
    }
    let Ok(zone) = ZONE.lock() else { return desc };
    let Some(zone) = *zone else { return desc };
    if head[4..20] != zone {
        return desc;
    }
    let cfg = config();
    if cfg.active() {
        return enlarge(manager, desc, &cfg);
    }
    let max = cfg.max_dumps;
    if DUMPS.fetch_add(1, Ordering::SeqCst) >= max {
        return desc;
    }
    let array = u64_at(&head, 0x18) as usize;
    let count = (u64_at(&head, 0x20) as usize).min(96);
    let entries = read_bytes(array, count * 24).unwrap_or_default();
    let count = (entries.len() / 24).min(count);
    let mut text = format!("pillar zone created: head={} components={count}\n", hex(&head));
    for i in 0..count {
        let hash = u32_at(&entries, i * 24);
        let data = u64_at(&entries, i * 24 + 8) as usize;
        let size = u64_at(&entries, i * 24 + 16) as usize;
        let dump = read_bytes(data, size.min(256)).unwrap_or_default();
        text.push_str(&format!("  c{i} hash={hash:08x} size={size} data={}\n", hex(&dump)));
    }
    for e in super::table::entries() {
        text.push_str(&format!("  player entity={} client={} permissions={:#04x} name={}\n", e.entity(), e.client, e.permissions, e.name.clone().unwrap_or_default()));
    }
    log(&text);
    desc
}

fn enlarge(manager: usize, desc: usize, cfg: &PillarConfig) -> usize {
    let head = read_bytes(desc, 0x100).or_else(|| read_bytes(desc, 0x30).map(|mut h| { h.resize(0x100, 0); h }));
    let Some(mut head) = head else { return desc };
    head.resize(0x100, 0);
    let array = u64_at(&head, 0x18) as usize;
    let count = (u64_at(&head, 0x20) as usize).min(96);
    let Some(entries) = read_bytes(array, count * 24) else { return desc };
    let count = (entries.len() / 24).min(count);

    let mut transform_at = None;
    for i in 0..count {
        if u32_at(&entries, i * 24) == HASH_CURRENT_TRANSFORM && u64_at(&entries, i * 24 + 16) as usize >= TRANSFORM_SIZE {
            transform_at = Some(u64_at(&entries, i * 24 + 8) as usize);
        }
    }
    let Some(transform_at) = transform_at else { return desc };
    let Some(t) = read_bytes(transform_at, TRANSFORM_SIZE) else { return desc };
    if t.len() < TRANSFORM_SIZE {
        return desc;
    }
    let (x, z) = (u64_at(&t, 0) as i64 as f64 / METERS, u64_at(&t, 16) as i64 as f64 / METERS);

    if cfg.admin_only {
        let (admin, why) = cast_by_admin(manager, x, z, cfg.nearest_decides);
        if DECISIONS.fetch_add(1, Ordering::SeqCst) < 500 {
            log(&format!("pillar at x={x:.1} z={z:.1}: {why} -> {}\n", if admin { "enlarged" } else { "left as it is" }));
        }
        if !admin {
            return desc;
        }
    }

    let scale = cfg.scale();
    let mut bytes = [0u8; 12];
    for part in bytes.chunks_mut(4) {
        part.copy_from_slice(&scale.to_le_bytes());
    }
    if !super::spawner::write_bytes(transform_at + SCALE_AT, &bytes) {
        log("pillar: could not write the scale\n");
        return desc;
    }

    if cfg.visual {
        visuals(&t, cfg);
    }

    if !cfg.set_duration {
        return desc;
    }
    let mut new_entries = Vec::with_capacity((count + 1) * 24);
    new_entries.extend_from_slice(&entries[..count * 24]);
    let mut life = [0u8; 24];
    life[..8].copy_from_slice(&cfg.duration_ns().to_le_bytes());
    let life: &'static mut [u8; 24] = Box::leak(Box::new(life));
    let mut extra = [0u8; 24];
    extra[..4].copy_from_slice(&HASH_LIFE_TIME.to_le_bytes());
    extra[8..16].copy_from_slice(&(life.as_ptr() as u64).to_le_bytes());
    extra[16..24].copy_from_slice(&24u64.to_le_bytes());
    new_entries.extend_from_slice(&extra);
    let new_entries: &'static mut [u8] = Box::leak(new_entries.into_boxed_slice());
    head[0x18..0x20].copy_from_slice(&(new_entries.as_ptr() as u64).to_le_bytes());
    head[0x20..0x28].copy_from_slice(&((count + 1) as u64).to_le_bytes());
    let head: &'static mut [u8] = Box::leak(head.into_boxed_slice());
    head.as_ptr() as usize
}

static LAST_VISUAL: Mutex<Option<(i64, i64, std::time::Instant)>> = Mutex::new(None);

/// The game creates two entities per cast; the effects are queued once per cast.
fn visuals(t: &[u8], cfg: &PillarConfig) {
    let (x, y, z) = (u64_at(t, 0) as i64, u64_at(t, 8) as i64, u64_at(t, 16) as i64);
    if let Ok(mut last) = LAST_VISUAL.lock() {
        if let Some((lx, lz, at)) = *last {
            let near = ((lx - x) as f64).abs() < METERS && ((lz - z) as f64).abs() < METERS;
            if near && at.elapsed().as_secs() < 3 {
                return;
            }
        }
        *last = Some((x, z, std::time::Instant::now()));
    }
    let life_ms = (cfg.duration_s.clamp(1.0, 120.0) * 1000.0) as u64;
    let templates: Vec<[u8; 16]> = cfg.visual_templates.iter().filter_map(|g| super::config::parse_guid(g)).collect();
    if templates.is_empty() {
        return;
    }
    let ring = cfg.visual_ring.min(24);
    let radius = cfg.ring_radius_m() as f64;
    for template in &templates {
        super::vfx::queue_effect(*template, (x, y, z), cfg.visual_scale, life_ms);
        for i in 0..ring {
            let a = std::f64::consts::TAU * i as f64 / ring as f64;
            let at = (x + (a.cos() * radius * METERS) as i64, y, z + (a.sin() * radius * METERS) as i64);
            super::vfx::queue_effect(*template, at, cfg.visual_scale, life_ms);
        }
    }
    log(&format!("pillar visual: {} effect(s) x {} around x={:.1} z={:.1}\n", templates.len(), ring + 1, x as f64 / METERS, z as f64 / METERS));
}

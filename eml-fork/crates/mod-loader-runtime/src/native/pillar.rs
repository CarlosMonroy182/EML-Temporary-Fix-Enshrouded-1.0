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

fn log(text: &str) {
    let Ok(dir) = std::env::current_dir() else { return };
    let dir = dir.join("m182_pillar");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("pillar.log")) {
        let _ = f.write_all(text.as_bytes());
    }
}

/// Called before the game creates an entity from `desc`.
pub fn before(desc: usize) {
    if !WANTED.load(Ordering::Relaxed) || desc < 0x10000 {
        return;
    }
    let Some(head) = read_bytes(desc, 0x30) else { return };
    if head.len() < 0x28 {
        return;
    }
    let Ok(zone) = ZONE.lock() else { return };
    let Some(zone) = *zone else { return };
    if head[4..20] != zone {
        return;
    }
    let max = CONFIG.lock().ok().and_then(|c| c.as_ref().map(|c| c.max_dumps)).unwrap_or(12);
    if DUMPS.fetch_add(1, Ordering::SeqCst) >= max {
        return;
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
}

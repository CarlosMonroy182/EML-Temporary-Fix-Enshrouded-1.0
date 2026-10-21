//! M182 VFX: big magic effects while a player charges an attack and a burst when the charged
//! attack is released. Fully separate from the admin panel: it only reads `m182_vfx.json` and
//! does nothing when that file is missing or `"enabled": false`.
//!
//! Effects are created by the server (or the host) with the same entity-create call the
//! spawner uses, so every player sees them without installing anything.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;

use super::spawn_probe::{original_create, read_bytes, u32_at, u64_at};
use super::spawner;

pub static KEY_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
unsafe fn GetAsyncKeyState(_vk: i32) -> i16 { if KEY_DOWN.load(Ordering::SeqCst) { 0x8000u16 as i16 } else { 0 } }

pub const CONFIG_FILE: &str = "m182_vfx.json";
pub const LOG_FILE: &str = "m182_vfx.log";

const CATALOG: &[(&str, &str)] = &[
    ("VFX_Skill_Aura_SunFire_Trigger_OnPlayer", "4aa64338-ae74-4647-a501-d40cd73af2b6"),
    ("VFX_Spell_LightningChannel_SourceTarget_NoTarget", "4c99af18-8db3-4ea3-95c4-a71dfc98321a"),
    ("VFX_Trap_Electro_Impact", "5493e543-a71e-46ec-b17b-d0a5e4d1d0a5"),
    ("VFX_Enemy_Vukah_Shaman_Lightning_Sky_Explosion", "bf944b00-58cc-45a1-a8da-c30a59d8c146"),
    ("VFX_Player_Projectile_Wand_Heavy_Piercing_Impact_Lightning", "f8afd903-8a98-4bfe-bab5-0decbc0da0c6"),
    ("VFX_Skill_MassDestruction_Impact", "b24e0244-7e12-44d6-8be2-6db92ac3ab8e"),
    ("VFX_Skill_BloodMagic_Trigger", "dca5c97a-dfa5-4a9b-aa68-ac653f690e00"),
    ("VFX_Skill_ChainHit_Impact", "fbdb3570-e74f-4673-8ea1-aa0e2b6fdf41"),
    ("VFX_Spell_ChainLightning_SourceTarget_NoTarget", "9c9932c9-2317-4260-8bb9-6e505456e9c3"),
    ("VFX_Prop_Enemy_Vukah_Shaman_Elite_Totem_Lightning_Projectile_Impact", "8bb2e2c3-8ef3-4ba3-a451-5ebf9dbd66ed"),
    ("VFX_Player_Projectile_Wand_EvadeAttack_Impact_Lightning", "1e786b62-1a3d-4565-918e-ad3d62144aa5"),
    ("VFX_Player_Projectile_Wand_Heavy_Piercing_Impact_Fire", "c3086f77-3d76-478c-abaf-fe364599b3b8"),
    ("VFX_Player_Projectile_Wand_Heavy_Piercing_Impact_Ice", "b567533b-89a4-439b-a405-67f68b3034d1"),
    ("VFX_Player_Projectile_Wand_Heavy_Piercing_Impact_Fog", "a3bddbcd-0e19-47dd-9b69-dadd7d294c4e"),
    ("VFX_Trap_Fireball_Impact", "10716ed0-9806-4535-bf3b-776d826a381a"),
    ("VFX_Spell_Icicle_Explosion", "13c71898-b4ad-4783-aadd-aeae447b9c3f"),
    ("VFX_WeaponGem_Corruption_Explosion_Tier5", "b5adbb7a-69f7-4565-9356-72b59337130f"),
    ("VFX_Player_Explosive_Arrow_Explosion_SKILL", "0cae3805-f924-42f6-a4cc-45f7d7bf2fcc"),
    ("VFX_Resource_Increase_FogResistance", "7ee4717b-71f5-4838-8757-9b1eb7fbfec7"),
    ("VFX_Enemy_Boss_FogDragon_Germinate_Explosion_Player_Hit", "82526e7f-b4fe-485f-83e3-4e8274624d9e"),
    ("VFX_Spell_Totem_Projectile_Shoot_Lightning", "9ba3cb11-08f7-4d19-a20b-087848d7e536"),
    ("VFX_Player_Companion_Shock_Wisp_Shoot", "53293378-8c9d-4277-ba22-f3c607f1025f"),
    ("VFX_Weapon_Arrow_Stun_Impact", "e2f935c2-5687-4d8d-a111-117fd0f6bf19"),
];


const HASH_CURRENT_TRANSFORM: u32 = 0x027a_c564;
const TRANSFORM_SIZE: usize = 56;
const COMBAT_SIZE: usize = 24;
const METERS: f64 = 4_294_967_296.0;
const MAX_DELAY_MS: u64 = 600;

pub const FLAG_HEAVY_ATTACK: u32 = 29;
pub const FLAG_RANGED_PREPARING: u32 = 26;
pub const FLAG_DEAD: u32 = 7;
pub const FLAG_READY: u32 = 50;

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct Burst {
    pub effects: Vec<String>,
    pub scale: f32,
    pub height: f32,
    pub distance: f32,
    pub ring: u32,
    pub radius: f32,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct VfxConfig {
    pub enabled: bool,
    pub charge: Burst,
    pub charge_every_ms: u64,
    pub release: Burst,
    pub min_charge_ms: u64,
    pub test_key: String,
}

impl Default for Burst {
    fn default() -> Self {
        Self { effects: Vec::new(), scale: 1.0, height: 0.0, distance: 0.0, ring: 1, radius: 0.0 }
    }
}

impl Default for VfxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            charge: Burst {
                effects: vec![
                    "VFX_Skill_Aura_SunFire_Trigger_OnPlayer".into(),
                    "VFX_Trap_Electro_Impact".into(),
                ],
                scale: 2.0,
                height: 0.9,
                distance: 0.0,
                ring: 1,
                radius: 0.0,
            },
            charge_every_ms: 250,
            release: Burst {
                effects: vec![
                    "VFX_Enemy_Vukah_Shaman_Lightning_Sky_Explosion".into(),
                    "VFX_Skill_MassDestruction_Impact".into(),
                ],
                scale: 2.0,
                height: 0.0,
                distance: 1.5,
                ring: 1,
                radius: 0.0,
            },
            min_charge_ms: 300,
            test_key: "F9".into(),
        }
    }
}

pub fn parse_config(text: &str) -> Result<VfxConfig, String> {
    let mut cfg: VfxConfig = serde_json::from_str(text).map_err(|e| e.to_string())?;
    for b in [&mut cfg.charge, &mut cfg.release] {
        b.scale = if b.scale.is_finite() { b.scale.clamp(0.1, 20.0) } else { 1.0 };
        b.ring = b.ring.clamp(1, 8);
        b.radius = if b.radius.is_finite() { b.radius.clamp(0.0, 10.0) } else { 0.0 };
        b.height = if b.height.is_finite() { b.height.clamp(-5.0, 20.0) } else { 0.0 };
        b.distance = if b.distance.is_finite() { b.distance.clamp(0.0, 20.0) } else { 0.0 };
        b.effects.truncate(6);
    }
    cfg.charge_every_ms = cfg.charge_every_ms.clamp(100, 5000);
    Ok(cfg)
}

/// State flags of a `CombatTargetActor` component (`activeAbilityMask` u32, padding,
/// `combinedState` u64, `lastRangedWeaponUsage`), or `None` when the bytes do not look like one.
pub fn combat_flags(b: &[u8]) -> Option<u64> {
    if b.len() < COMBAT_SIZE {
        return None;
    }
    let abilities = u32_at(b, 0);
    let padding = u32_at(b, 4);
    let state = u64_at(b, 8);
    if abilities >= 1 << 29 || padding != 0 || state >> 63 != 0 || state & (1 << FLAG_READY) == 0 {
        return None;
    }
    Some(state)
}

pub fn is_charging(flags: u64) -> bool {
    flags & (1 << FLAG_DEAD) == 0 && (flags & (1 << FLAG_HEAVY_ATTACK) != 0 || flags & (1 << FLAG_RANGED_PREPARING) != 0)
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct PlayerCharge {
    pub since: Option<u64>,
    pub last_burst: Option<u64>,
    pub heavy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    None,
    Charge,
    Release,
}

/// One step of the per-player charge tracker (`now` in milliseconds).
pub fn step(st: &mut PlayerCharge, flags: u64, now: u64, cfg: &VfxConfig) -> Event {
    if is_charging(flags) {
        if flags & (1 << FLAG_HEAVY_ATTACK) != 0 {
            st.heavy = true;
        }
        let since = *st.since.get_or_insert(now);
        let due = st.last_burst.map(|t| now.saturating_sub(t) >= cfg.charge_every_ms).unwrap_or(true);
        if due && now.saturating_sub(since) >= 120 && !cfg.charge.effects.is_empty() {
            st.last_burst = Some(now);
            return Event::Charge;
        }
        return Event::None;
    }
    let Some(since) = st.since.take() else { return Event::None };
    st.last_burst = None;
    st.heavy = false;
    let long_enough = now.saturating_sub(since) >= cfg.min_charge_ms;
    if long_enough && flags & (1 << FLAG_DEAD) == 0 && !cfg.release.effects.is_empty() {
        Event::Release
    } else {
        Event::None
    }
}

/// Points of a burst around `(x, y, z)`: `ring` points on a circle of `radius` metres,
/// pushed `distance` metres along `(fx, fz)` and lifted by `height` metres.
pub fn burst_points(x: i64, y: i64, z: i64, facing: (f64, f64), b: &Burst) -> Vec<(i64, i64, i64)> {
    let cx = x as f64 / METERS + facing.0 * b.distance as f64;
    let cz = z as f64 / METERS + facing.1 * b.distance as f64;
    let cy = y as f64 / METERS + b.height as f64;
    let n = b.ring.max(1);
    (0..n)
        .map(|i| {
            let (dx, dz) = if n == 1 || b.radius <= 0.0 {
                (0.0, 0.0)
            } else {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                (a.cos() * b.radius as f64, a.sin() * b.radius as f64)
            };
            (((cx + dx) * METERS) as i64, (cy * METERS) as i64, ((cz + dz) * METERS) as i64)
        })
        .collect()
}

struct Spawn {
    queued: u64,
    template: [u8; 16],
    at: (i64, i64, i64),
    scale: f32,
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
static CONFIG: Mutex<Option<VfxConfig>> = Mutex::new(None);
static QUEUE: Mutex<Vec<Spawn>> = Mutex::new(Vec::new());
static QUEUED: AtomicU32 = AtomicU32::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);
static COMBAT_INDEX: AtomicI32 = AtomicI32::new(-1);
static COMBAT_MGR: AtomicUsize = AtomicUsize::new(0);
static SPAWNED: AtomicU32 = AtomicU32::new(0);
static FAILED: AtomicU32 = AtomicU32::new(0);
static GUIDS: Mutex<Vec<(String, Option<[u8; 16]>)>> = Mutex::new(Vec::new());
static CLOCK: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn log(message: &str) {
    use std::io::Write;
    let ms = now_ms();
    let Ok(mut guard) = LOG.lock() else { return };
    if guard.is_none() {
        if let Ok(dir) = std::env::current_dir() {
            if let Ok(mut f) = std::fs::File::create(dir.join(LOG_FILE)) {
                let _ = writeln!(f, "# M182 VFX log");
                *guard = Some(f);
            }
        }
    }
    if let Some(f) = guard.as_mut() {
        let _ = writeln!(f, "[{ms}ms] {message}");
        let _ = f.flush();
    }
}

fn now_ms() -> u64 {
    CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64
}

pub fn pending() -> bool {
    QUEUED.load(Ordering::Relaxed) != 0
}

fn template_guid(name: &str) -> Option<[u8; 16]> {
    if let Ok(known) = GUIDS.lock() {
        if let Some((_, g)) = known.iter().find(|(n, _)| n == name) {
            return *g;
        }
    }
    let builtin = CATALOG.iter().find(|(n, _)| *n == name).and_then(|(_, g)| super::config::parse_guid(g));
    let found = if let Some(g) = super::config::parse_guid(name).or(builtin) {
        Some(g)
    } else {
        let path = std::env::current_dir().ok().map(|d| d.join("export").join("m182_objects.csv"));
        let content = path.and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        content.lines().skip(1).find_map(|line| {
            let mut parts = line.split(',');
            match (parts.next(), parts.next()) {
                (Some(n), Some(g)) if n == name => super::config::parse_guid(g),
                _ => None,
            }
        })
    };
    log(&match found {
        Some(_) => format!("vfx: effect {name} found"),
        None => format!("vfx: effect {name} is not in export/m182_objects.csv"),
    });
    if let Ok(mut known) = GUIDS.lock() {
        known.push((name.to_string(), found));
    }
    found
}

pub fn queue_burst(at: (i64, i64, i64), facing: (f64, f64), b: &Burst) {
    let points = burst_points(at.0, at.1, at.2, facing, b);
    let templates: Vec<[u8; 16]> = b.effects.iter().filter_map(|n| template_guid(n)).collect();
    if templates.is_empty() {
        return;
    }
    let Ok(mut q) = QUEUE.lock() else { return };
    if q.len() > 48 {
        return;
    }
    for (i, p) in points.into_iter().enumerate() {
        for (j, t) in templates.iter().enumerate() {
            if i > 0 && j > 0 {
                continue;
            }
            q.push(Spawn { queued: now_ms(), template: *t, at: p, scale: b.scale });
        }
    }
    QUEUED.store(1, Ordering::SeqCst);
}

/// Creates the queued effects. Called by the spawner from the game's own entity-create call
/// on the server, the same place every other M182 spawn happens.
pub fn run(manager: usize) {
    if QUEUED.load(Ordering::Relaxed) == 0 || RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    run_queue(manager);
    RUNNING.store(false, Ordering::SeqCst);
}

/// Called on every server tick (the server's player-table pump).
pub fn server_tick() {
    if QUEUED.load(Ordering::Relaxed) == 0 {
        return;
    }
    let manager = spawner::server_manager();
    if manager != 0 {
        run(manager);
    }
}

fn run_queue(manager: usize) {
    if QUEUED.swap(0, Ordering::SeqCst) == 0 {
        return;
    }
    let work: Vec<Spawn> = match QUEUE.lock() {
        Ok(mut q) => std::mem::take(&mut *q),
        Err(_) => return,
    };
    let now = now_ms();
    let fresh: Vec<Spawn> = work.into_iter().filter(|s| now.saturating_sub(s.queued) <= MAX_DELAY_MS).collect();
    for s in fresh.into_iter().take(24) {
        let id = create_effect(manager, s.template, s.at, s.scale);
        if id == 0 {
            if FAILED.fetch_add(1, Ordering::Relaxed) < 3 {
                log("vfx: the game did not create an effect");
            }
        } else if SPAWNED.fetch_add(1, Ordering::Relaxed) < 5 {
            log(&format!("vfx: effect created id={id} scale={:.1}", s.scale));
        }
    }
}

pub fn create_effect(manager: usize, template: [u8; 16], at: (i64, i64, i64), scale: f32) -> u32 {
    let mut transform = [0u8; TRANSFORM_SIZE];
    transform[0..8].copy_from_slice(&(at.0 as u64).to_le_bytes());
    transform[8..16].copy_from_slice(&(at.1 as u64).to_le_bytes());
    transform[16..24].copy_from_slice(&(at.2 as u64).to_le_bytes());
    transform[36..40].copy_from_slice(&1.0f32.to_le_bytes());
    for off in [40, 44, 48] {
        transform[off..off + 4].copy_from_slice(&scale.to_le_bytes());
    }
    let mut entries = [0u8; 0x100];
    entries[0..4].copy_from_slice(&HASH_CURRENT_TRANSFORM.to_le_bytes());
    entries[8..16].copy_from_slice(&(transform.as_ptr() as u64).to_le_bytes());
    entries[16..24].copy_from_slice(&(TRANSFORM_SIZE as u64).to_le_bytes());
    let mut desc = [0u8; 0x100];
    desc[4..20].copy_from_slice(&template);
    desc[0x18..0x20].copy_from_slice(&(entries.as_ptr() as u64).to_le_bytes());
    desc[0x20..0x28].copy_from_slice(&1u64.to_le_bytes());
    original_create(manager, 0, desc.as_ptr() as usize, 0)
}

/// Finds the `CombatTargetActor` component of a player and returns its flags.
pub fn read_flags(manager: usize, id: u32) -> Option<u64> {
    let record = spawner::find_record(manager, id)?;
    let p = spawner::placement(record)?;
    if COMBAT_MGR.load(Ordering::SeqCst) == manager {
        let index = COMBAT_INDEX.load(Ordering::SeqCst);
        if index >= 0 {
            if let Some((addr, stride)) = spawner::component_address(&p, index as usize) {
                if stride == COMBAT_SIZE {
                    if let Some(f) = read_bytes(addr, COMBAT_SIZE).as_deref().and_then(combat_flags) {
                        return Some(f);
                    }
                }
            }
        }
    }
    let mut found: Vec<(usize, u64)> = Vec::new();
    for index in 0..spawner::COMPONENT_SLOTS {
        let Some((addr, stride)) = spawner::component_address(&p, index) else { continue };
        if stride != COMBAT_SIZE {
            continue;
        }
        if let Some(f) = read_bytes(addr, COMBAT_SIZE).as_deref().and_then(combat_flags) {
            found.push((index, f));
        }
    }
    let (index, flags) = *found.first()?;
    COMBAT_MGR.store(manager, Ordering::SeqCst);
    COMBAT_INDEX.store(index as i32, Ordering::SeqCst);
    log(&format!(
        "vfx: combat state found on entity {id}: index={index} flags={flags:#x} ({} candidate(s): {:?})",
        found.len(),
        found.iter().map(|(i, _)| *i).collect::<Vec<_>>()
    ));
    Some(flags)
}

fn position_of(manager: usize, id: u32) -> Option<((i64, i64, i64), (f64, f64))> {
    let t = spawner::transform_of(manager, id)?;
    if t.len() < TRANSFORM_SIZE {
        return None;
    }
    let at = (u64_at(&t, 0) as i64, u64_at(&t, 8) as i64, u64_at(&t, 16) as i64);
    Some((at, spawner::facing_direction(&t)))
}

fn load(dir: &std::path::Path) -> Option<(VfxConfig, Option<SystemTime>)> {
    let path = dir.join(CONFIG_FILE);
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let text = std::fs::read_to_string(&path).ok()?;
    match parse_config(&text) {
        Ok(cfg) => Some((cfg, modified)),
        Err(e) => {
            log(&format!("vfx: {CONFIG_FILE} is not valid ({e}); effects off"));
            Some((VfxConfig { enabled: false, ..VfxConfig::default() }, modified))
        }
    }
}

/// Starts the effect thread when `m182_vfx.json` exists next to the game.
pub fn start() {
    let Ok(dir) = std::env::current_dir() else { return };
    if !dir.join(CONFIG_FILE).exists() || STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = CLOCK.get_or_init(Instant::now);
    log("vfx: started");
    if let Err(e) = std::thread::Builder::new().name("m182-vfx".into()).spawn(move || run_loop(dir)) {
        tracing::warn!(target: "m182::vfx", error = %e, "could not start vfx thread");
    }
}

fn run_loop(dir: std::path::PathBuf) {
    let mut modified = None;
    let mut checked = Instant::now() - Duration::from_secs(10);
    let mut players: Vec<u32> = Vec::new();
    let mut scanned = Instant::now() - Duration::from_secs(10);
    let mut states: HashMap<u32, PlayerCharge> = HashMap::new();
    let mut key_was_down = false;
    let mut reported_players = usize::MAX;
    let mut waiting_logged = false;
    loop {
        std::thread::sleep(Duration::from_millis(60));
        if checked.elapsed() >= Duration::from_secs(2) {
            checked = Instant::now();
            let stamp = std::fs::metadata(dir.join(CONFIG_FILE)).and_then(|m| m.modified()).ok();
            if stamp != modified || CONFIG.lock().map(|c| c.is_none()).unwrap_or(false) {
                if let Some((cfg, m)) = load(&dir) {
                    modified = m;
                    log(&format!(
                        "vfx: config loaded enabled={} charge={:?} x{} release={:?} x{}",
                        cfg.enabled, cfg.charge.effects, cfg.charge.scale, cfg.release.effects, cfg.release.scale
                    ));
                    ENABLED.store(cfg.enabled, Ordering::SeqCst);
                    if let Ok(mut c) = CONFIG.lock() {
                        *c = Some(cfg);
                    }
                }
            }
        }
        let Some(cfg) = CONFIG.lock().ok().and_then(|c| c.clone()) else { continue };
        if !cfg.enabled {
            continue;
        }
        let manager = spawner::server_manager();
        if manager == 0 {
            if !waiting_logged {
                waiting_logged = true;
                log("vfx: waiting for the world (effects are created by the server or the host)");
            }
            continue;
        }
        let key = super::config::parse_key(&cfg.test_key);
        let down = key.map(|k| unsafe { GetAsyncKeyState(k) } as u16 & 0x8000 != 0).unwrap_or(false) && spawner::game_has_focus();
        if down && !key_was_down {
            let host = spawner::host_player();
            match position_of(manager, host) {
                Some((at, facing)) => {
                    log("vfx: test key, release burst on the host");
                    queue_burst(at, facing, &cfg.release);
                }
                None => log("vfx: test key pressed but the host position is not known yet"),
            }
        }
        key_was_down = down;
        if scanned.elapsed() >= Duration::from_secs(3) {
            scanned = Instant::now();
            players = spawner::player_entities(manager);
            states.retain(|id, _| players.contains(id));
            if players.len() != reported_players {
                reported_players = players.len();
                log(&format!("vfx: watching {} player(s) {:?}", players.len(), players));
            }
        }
        let now = now_ms();
        for &id in &players {
            let Some(flags) = read_flags(manager, id) else { continue };
            let st = states.entry(id).or_default();
            let was = st.since.is_some();
            match step(st, flags, now, &cfg) {
                Event::Charge => {
                    if let Some((at, facing)) = position_of(manager, id) {
                        queue_burst(at, facing, &cfg.charge);
                    }
                }
                Event::Release => {
                    if let Some((at, facing)) = position_of(manager, id) {
                        queue_burst(at, facing, &cfg.release);
                    }
                    if SPAWNED.load(Ordering::Relaxed) < 40 {
                        log(&format!("vfx: entity {id} released a charged attack (flags {flags:#x})"));
                    }
                }
                Event::None => {}
            }
            if !was && st.since.is_some() && SPAWNED.load(Ordering::Relaxed) < 40 {
                log(&format!("vfx: entity {id} started charging (flags {flags:#x})"));
            }
        }
    }
}

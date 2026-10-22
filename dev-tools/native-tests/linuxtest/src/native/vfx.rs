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
    ("VFX_SubEntity_Projectile_Enemy_WilloWisp_Fireball_Deflected", "038a7a31-9901-4745-9b05-2f9da575dcdd"),
    ("VFX_SubEntity_Projectile_Enemy_IceWisp_IceBolt_Deflected", "4047af6e-7775-4d84-b4dd-10daa219b6c8"),
    ("VFX_SubEntity_Projectile_Enemy_Boss_FogDragon_Germinate_Deflected", "b0454193-a591-492b-844d-7242704fa370"),
    ("VFX_SubEntity_Projectile_Enemy_Fogger_Jelly_Seeker_Deflected", "fdeef833-002f-407e-892f-e247ee0d9e2e"),
    ("VFX_SubEntity_Projectile_FlyingSkull_Skeleton_Deflected", "19efcf99-bbe8-4084-8d01-2c10d5da73b4"),
    ("VFX_SubEntity_Projectile_Enemy_Mage_Sickle_Deflected", "685bebdd-77e3-4e4b-b186-c36debc93a1e"),
    ("VFX_WeaponGem_GemOfAgility_OnHit", "9ff81751-98fe-44bb-b2af-ffdbf532b492"),
    ("VFX_WeaponGem_GemOfOverpowering_OnBlock", "26e442b5-3f5d-49f4-b9c3-f49b63ee6444"),
    ("VFX_SubEntity_Enemy_FogMonstrosity_summon_spawn", "11dfe1bd-74fb-45e4-a921-369927a3a1f0"),
    ("VFX_SubEntity_Enemy_Vukah_Shaman_Elite_Beast_Summon_Spawn", "93fb1052-2eac-4b6b-84e4-4ca4128ed805"),
    ("VFX_WeaponGem_GemOfFocusedLight_SourceTarget_tier3", "b99fbda9-2a79-4f87-97e9-eeb7a3a0e19a"),
    ("VFX_Consumable_LightWisp", "ba445d10-0b7b-402e-853b-e49b7ff50876"),
    ("VFX_Resource_Increase_Health_NoSound", "b8e9aca9-8713-4c4c-b51c-1591e74873b4"),
    ("VFX_Spell_ManaDrain_Mana_OnPlayer", "e39c8dd9-d54a-4bef-8a09-8c5d739ea4e5"),
    ("VFX_Player_Companion_Skeleton_GemOfNecromancy_Spawn", "9852b802-6210-4122-82f6-d2bf5e0c277a"),
    ("VFX_Resource_Increase_Mana", "4f994c03-779b-4809-944c-9ca09f7f3bd2"),
    ("VFX_Player_Projectile_Wand_EvadeAttack_Impact_Fire", "eaf35f40-185b-4f5e-9d8a-c7cc71b19797"),
    ("VFX_Player_Projectile_Wand_EvadeAttack_Impact_Ice", "aea9621e-914f-4838-bf0e-076046a82a2d"),
    ("VFX_Player_Projectile_Wand_EvadeAttack_Impact_Fog", "fe8e40b5-a235-48a9-bd9b-3294546e3a94"),
    ("VFX_Player_Projectile_Wand_EvadeAttack_Impact_Blunt", "1fead2a1-7c83-4d09-a6a3-b97a64090043"),
    ("VFX_Player_Projectile_Wand_Heavy_Piercing_Impact_Blunt", "02845b07-b354-4d14-a58d-0b81b9c2f411"),
];

const HASH_CURRENT_TRANSFORM: u32 = 0x027a_c564;
const TRANSFORM_SIZE: usize = 56;
const COMBAT_SIZE: usize = 24;
const EQUIPMENT_SIZE: usize = 1380;
const MAIN_HAND_VISUAL: usize = 800 + 23 * 24 + 20;
const MAIN_HAND_ITEM: usize = 8 + 8;
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
    #[serde(skip)]
    pub life_ms: Option<u64>,
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
    pub melee_charge: Burst,
    pub melee_release: Burst,
    pub min_charge_ms: u64,
    pub test_key: String,
    pub auto: bool,
}

impl Default for Burst {
    fn default() -> Self {
        Self { effects: Vec::new(), life_ms: None, scale: 1.0, height: 0.0, distance: 0.0, ring: 1, radius: 0.0 }
    }
}

impl Default for VfxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            charge: Burst {
                life_ms: None,
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
            release: Burst::default(),
            melee_charge: Burst::default(),
            melee_release: Burst::default(),
            min_charge_ms: 300,
            test_key: "F9".into(),
            auto: true,
        }
    }
}

pub fn parse_config(text: &str) -> Result<VfxConfig, String> {
    let mut cfg: VfxConfig = serde_json::from_str(text).map_err(|e| e.to_string())?;
    for b in [&mut cfg.charge, &mut cfg.release, &mut cfg.melee_charge, &mut cfg.melee_release] {
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
    pub element: Option<Element>,
    pub last_hand: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    None,
    Charge { melee: bool },
    Release { melee: bool },
}

/// One step of the per-player charge tracker (`now` in milliseconds). A charge that showed the
/// heavy-attack flag is a melee charge, otherwise a ranged (staff, bow) charge.
pub fn step(st: &mut PlayerCharge, flags: u64, now: u64, every_ms: u64, min_charge_ms: u64) -> Event {
    if is_charging(flags) {
        if flags & (1 << FLAG_HEAVY_ATTACK) != 0 {
            st.heavy = true;
        }
        let since = *st.since.get_or_insert(now);
        let due = st.last_burst.map(|t| now.saturating_sub(t) >= every_ms).unwrap_or(true);
        if due && now.saturating_sub(since) >= 120 {
            st.last_burst = Some(now);
            return Event::Charge { melee: st.heavy };
        }
        return Event::None;
    }
    let Some(since) = st.since.take() else { return Event::None };
    st.last_burst = None;
    let melee = std::mem::take(&mut st.heavy);
    if now.saturating_sub(since) >= min_charge_ms && flags & (1 << FLAG_DEAD) == 0 {
        Event::Release { melee }
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
    hand: Option<(u32, u8)>,
    life_ms: Option<u64>,
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

/// True when the name is one of the game's own effect templates (known to every client).
pub fn is_vanilla(name: &str) -> bool {
    CATALOG.iter().any(|(n, _)| *n == name)
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
            q.push(Spawn { queued: now_ms(), template: *t, at: p, scale: b.scale, hand: None, life_ms: b.life_ms });
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
        let id = match s.hand {
            Some((entity, slot)) => create_on_slot(manager, s.template, entity, slot, s.at, s.life_ms.unwrap_or(HAND_LIFE_MS)),
            None => create_effect_for(manager, s.template, s.at, s.scale, s.life_ms),
        };
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
    create_effect_for(manager, template, at, scale, None)
}

/// Like `create_effect`, optionally overriding how long the effect lives.
pub fn create_effect_for(manager: usize, template: [u8; 16], at: (i64, i64, i64), scale: f32, life_ms: Option<u64>) -> u32 {
    let mut transform = [0u8; TRANSFORM_SIZE];
    transform[0..8].copy_from_slice(&(at.0 as u64).to_le_bytes());
    transform[8..16].copy_from_slice(&(at.1 as u64).to_le_bytes());
    transform[16..24].copy_from_slice(&(at.2 as u64).to_le_bytes());
    transform[36..40].copy_from_slice(&1.0f32.to_le_bytes());
    for off in [40, 44, 48] {
        transform[off..off + 4].copy_from_slice(&scale.to_le_bytes());
    }
    let life = life_time_bytes(life_ms.unwrap_or(0));
    let mut entries = [0u8; 0x100];
    entries[0..4].copy_from_slice(&HASH_CURRENT_TRANSFORM.to_le_bytes());
    entries[8..16].copy_from_slice(&(transform.as_ptr() as u64).to_le_bytes());
    entries[16..24].copy_from_slice(&(TRANSFORM_SIZE as u64).to_le_bytes());
    let mut count = 1u64;
    if life_ms.is_some() {
        entries[24..28].copy_from_slice(&HASH_LIFE_TIME.to_le_bytes());
        entries[32..40].copy_from_slice(&(life.as_ptr() as u64).to_le_bytes());
        entries[40..48].copy_from_slice(&(LIFE_TIME_SIZE as u64).to_le_bytes());
        count = 2;
    }
    let mut desc = [0u8; 0x100];
    desc[4..20].copy_from_slice(&template);
    desc[0x18..0x20].copy_from_slice(&(entries.as_ptr() as u64).to_le_bytes());
    desc[0x20..0x28].copy_from_slice(&count.to_le_bytes());
    original_create(manager, 0, desc.as_ptr() as usize, 0)
}

/// The visual entity of the weapon in the player's main hand and the weapon's item id,
/// read from the player's `Equipment` component.
pub fn main_hand(manager: usize, id: u32) -> Option<(u32, u32)> {
    let record = spawner::find_record(manager, id)?;
    let p = spawner::placement(record)?;
    for index in 0..spawner::COMPONENT_SLOTS {
        let Some((addr, stride)) = spawner::component_address(&p, index) else { continue };
        if stride != EQUIPMENT_SIZE {
            continue;
        }
        let b = read_bytes(addr, EQUIPMENT_SIZE)?;
        if b.len() < EQUIPMENT_SIZE {
            return None;
        }
        let visual = u32_at(&b, MAIN_HAND_VISUAL);
        let first = u32_at(&b, MAIN_HAND_ITEM);
        if item_kind(first).map(|k| k.weapon).unwrap_or(false) {
            return Some((visual, first));
        }
        for slot in 0..11 {
            let item = u32_at(&b, 8 + slot * 72 + 8);
            if item != 0 && item_kind(item).map(|k| k.weapon).unwrap_or(false) {
                return Some((visual, item));
            }
        }
        return Some((visual, first));
    }
    None
}

static NOTES: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn note_once(text: &str) {
    if let Ok(mut n) = NOTES.lock() {
        if !n.iter().any(|t| t == text) {
            n.push(text.to_string());
            log(text);
        }
    }
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


// ---------------------------------------------------------------------------------------------
// Elements: the effects follow the damage type of the weapon or spell in use.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Element {
    Lightning,
    Fire,
    Ice,
    Shroud,
    Healing,
    Poison,
    Bone,
    Arcane,
    Physical,
    Summon,
}

/// Element of an item from its damage distribution
/// (cut, pierce, blunt, fire, ice, fog, lightning, healing, poison).
pub fn element_of(d: [f32; 9]) -> Element {
    let magic = [
        (d[3], Element::Fire),
        (d[4], Element::Ice),
        (d[5], Element::Shroud),
        (d[6], Element::Lightning),
        (d[7], Element::Healing),
        (d[8], Element::Poison),
    ];
    let best = magic.iter().copied().fold((0.0f32, Element::Physical), |a, b| if b.0 > a.0 { b } else { a });
    if best.0 > 0.0 {
        return best.1;
    }
    if d[0] <= 0.0 && d[1] <= 0.0 && d[2] > 0.0 {
        return Element::Bone;
    }
    if d.iter().all(|v| *v <= 0.0) {
        return Element::Arcane;
    }
    Element::Physical
}

/// Effects around the body while a staff is charged.
pub fn staff_effects(e: Element) -> &'static [&'static str] {
    match e {
        Element::Lightning | Element::Physical => &["VFX_Skill_Aura_SunFire_Trigger_OnPlayer", "VFX_Trap_Electro_Impact"],
        Element::Fire => &["VFX_Skill_Aura_SunFire_Trigger_OnPlayer", "VFX_Trap_Fireball_Impact"],
        Element::Ice => &["VFX_Spell_Icicle_Explosion", "VFX_Player_Projectile_Wand_EvadeAttack_Impact_Ice"],
        Element::Shroud => &["VFX_Player_Projectile_Wand_EvadeAttack_Impact_Fog"],
        Element::Healing => &["VFX_Resource_Increase_Health_NoSound"],
        Element::Poison => &["VFX_Player_Projectile_Wand_EvadeAttack_Impact_Blunt"],
        Element::Bone => &["VFX_Player_Projectile_Wand_Heavy_Piercing_Impact_Blunt"],
        Element::Arcane => &["VFX_Spell_ManaDrain_Mana_OnPlayer", "VFX_Resource_Increase_Mana"],
        Element::Summon => &["VFX_Player_Companion_Skeleton_GemOfNecromancy_Spawn"],
    }
}

/// Size of the staff aura of an element relative to the configured scale.
pub fn staff_scale(e: Element) -> f32 {
    match e {
        Element::Shroud => 0.6,
        Element::Arcane => 1.25,
        Element::Healing => 1.5,
        _ => 1.0,
    }
}

/// Template made by the M182 VFX Lua mod: the element's glow, attached on the server to the
/// player's weapon hand.
/// Game templates (glider trails, never seen outside gliding) that the M182 VFX Lua mod turns
/// into the glows: on the server it adds the server-only parts (LifeTime, DoNotSave), on PCs
/// with the mod it swaps the look. Every client knows them, so nobody is disconnected; players
/// without the mod see the original trail.
pub const GLOW_CARRIERS: [&str; 14] = [
    "ad949f69-aadd-4f5e-97dc-1db3afc5ffde",
    "21f9f3e9-433f-48d7-b605-c0f2ad1ed5cf",
    "d6d33ef8-8681-47b6-a429-04c00666cca1",
    "f7bc7106-ac91-45da-9d93-056bddf0c8d0",
    "04c22eef-5e56-4a6e-b340-ee07366862f5",
    "c7fd6895-e278-4b10-abb2-d4b2f882a134",
    "cc9d30cb-cf05-4baf-ab07-77586516dc3c",
    "45d8ce2a-0b08-44d4-b47e-6a849a149279",
    "0b5af7c4-5183-4e61-99a2-12535f90d6d0",
    "25662ec7-c5cd-4595-ab2c-9ff46b8dbcd6",
    "be24c385-8b5e-4ea7-9827-48eaec9c5209",
    "132d2794-96a7-4e45-b80a-fd8d14810ac7",
    "f56a56dd-a068-43e9-8ec4-f4fdbb68846b",
    "73500153-66c8-420d-a99e-a7341daa6e32",
];

/// Marker written in the Lua mod that prepares the carriers (older versions do not).
pub const GLOW_MOD_MARKER: &str = "glow carriers ready";

static GLOWS: AtomicBool = AtomicBool::new(false);

/// True when this machine runs the Lua mod that prepares the glow carriers.
pub fn glows_ready() -> bool {
    GLOWS.load(Ordering::Relaxed)
}

static DEDICATED: AtomicBool = AtomicBool::new(false);

/// True on a dedicated server: only the staff charge effects (game effects, seen by everyone)
/// are created there, so the server is not loaded with per-player effects.
pub fn dedicated() -> bool {
    DEDICATED.load(Ordering::Relaxed)
}

pub fn set_dedicated(on: bool) {
    DEDICATED.store(on, Ordering::Relaxed);
}

pub fn is_server_exe(path: &std::path::Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).map(|n| n.to_ascii_lowercase().contains("server")).unwrap_or(false)
}

pub fn set_glows_ready(on: bool) {
    GLOWS.store(on, Ordering::Relaxed);
}

pub fn glow_mod_installed(dir: &std::path::Path) -> bool {
    let folder = dir.join("mods").join("m182_vfx");
    folder.join("mod.json").is_file()
        && std::fs::read_to_string(folder.join("src").join("mod.lua")).map(|t| t.contains(GLOW_MOD_MARKER)).unwrap_or(false)
}

/// Glow `n` (1..=14).
pub fn glow_template(n: u8) -> Option<[u8; 16]> {
    let i = (n as usize).checked_sub(1)?;
    super::config::parse_guid(GLOW_CARRIERS.get(i)?)
}

pub fn hand_template(e: Element) -> Option<[u8; 16]> {
    let n: u8 = match e {
        Element::Lightning => 1,
        Element::Fire => 2,
        Element::Ice => 3,
        Element::Shroud => 4,
        Element::Healing => 5,
        Element::Poison => 6,
        Element::Bone | Element::Summon => 7,
        Element::Arcane => 8,
        Element::Physical => 9,
    };
    glow_template(n)
}

/// Game effect glued (by the server) to the weapon hand while a melee attack is charged
/// when the glow mod is not installed.
pub fn hand_effect(e: Element) -> Option<[u8; 16]> {
    let guid = match e {
        Element::Poison => "eee801de-8da6-4e11-b5f7-159205ffb139",
        Element::Physical => "155a3096-7dbf-429c-b909-4f6096f7e1ec",
        _ => return None,
    };
    super::config::parse_guid(guid)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemKind {
    pub element: Element,
    pub wand: bool,
    pub weapon: bool,
}

static ITEMS: Mutex<Option<HashMap<u32, ItemKind>>> = Mutex::new(None);

/// Reads `item_id`, `debug_name`, `slot` and the damage columns of `export/m182_items.csv`.
pub fn parse_items(text: &str) -> HashMap<u32, ItemKind> {
    let mut out = HashMap::new();
    let mut lines = text.lines();
    let Some(header) = lines.next() else { return out };
    let cols: Vec<&str> = header.split(',').collect();
    let at = |name: &str| cols.iter().position(|c| *c == name);
    let names = ["cut", "pierce", "blunt", "fire", "ice", "fog", "lightning", "healing", "poison"];
    let (Some(id_col), Some(idx)) = (at("item_id"), names.iter().map(|n| at(n)).collect::<Option<Vec<usize>>>()) else { return out };
    let name_col = at("debug_name");
    let slot_col = at("slot");
    for line in lines {
        let f: Vec<&str> = line.split(',').collect();
        let Some(id) = f.get(id_col).and_then(|v| v.trim().parse::<u32>().ok()) else { continue };
        let mut d = [0f32; 9];
        for (k, i) in idx.iter().enumerate() {
            d[k] = f.get(*i).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0);
        }
        let name = name_col.and_then(|c| f.get(c)).copied().unwrap_or("");
        let slot = slot_col.and_then(|c| f.get(c)).copied().unwrap_or("");
        let element = if name.contains("_Spell_Skeleton") {
            Element::Summon
        } else if name.contains("_Spell_Root") || name.contains("_Spell_Heal") {
            Element::Healing
        } else {
            element_of(d)
        };
        let weapon = matches!(slot, "MeleeWeapon" | "RangeWeapon" | "MidRangeWeapon") || name.starts_with("Weapon_");
        out.insert(id, ItemKind { element, wand: slot == "MidRangeWeapon" || name.contains("_Wand"), weapon });
    }
    out
}

pub fn item_kind(item: u32) -> Option<ItemKind> {
    let mut guard = ITEMS.lock().ok()?;
    if guard.is_none() {
        let path = std::env::current_dir().ok()?.join("export").join("m182_items.csv");
        let table = std::fs::read_to_string(path).map(|t| parse_items(&t)).unwrap_or_default();
        log(&format!("vfx: {} items with damage types loaded", table.len()));
        *guard = Some(table);
    }
    guard.as_ref()?.get(&item).copied()
}

pub fn item_element(item: u32) -> Option<Element> {
    item_kind(item).map(|k| k.element)
}

pub fn set_items(table: HashMap<u32, ItemKind>) {
    if let Ok(mut g) = ITEMS.lock() {
        *g = Some(table);
    }
}

const RANGED_SIZE: usize = 112;
const SLOTS_SIZE: usize = 1760;
const SLOT_HAND_R: usize = 8;

fn component_with<F: Fn(&[u8]) -> bool>(manager: usize, id: u32, stride: usize, ok: F) -> Option<Vec<u8>> {
    let record = spawner::find_record(manager, id)?;
    let p = spawner::placement(record)?;
    for index in 0..spawner::COMPONENT_SLOTS {
        let Some((addr, size)) = spawner::component_address(&p, index) else { continue };
        if size != stride {
            continue;
        }
        if let Some(b) = read_bytes(addr, stride) {
            if b.len() == stride && ok(&b) {
                return Some(b);
            }
        }
    }
    None
}

/// Spell (ammunition) and weapon item ids from the player's `NetworkRangedWeapon`.
pub fn ranged_items(manager: usize, id: u32) -> Option<(u32, u32)> {
    let b = component_with(manager, id, RANGED_SIZE, |b| b[0] <= 4 && b[1..8].iter().all(|v| *v == 0) && (u32_at(b, 68) != 0 || u32_at(b, 56) != 0))?;
    Some((u32_at(&b, 68), u32_at(&b, 56)))
}

/// World position of a body slot (hand, head...) and its three axes, from the server's
/// animation slots of the player.
pub fn slot_world(manager: usize, id: u32, slot: usize) -> Option<((i64, i64, i64), [[f32; 3]; 3])> {
    let player = spawner::transform_of(manager, id)?;
    let (px, py, pz) = (u64_at(&player, 0) as i64, u64_at(&player, 8) as i64, u64_at(&player, 16) as i64);
    let b = component_with(manager, id, SLOTS_SIZE, |b| {
        let (ox, oy, oz) = (u64_at(b, 8) as i64, u64_at(b, 16) as i64, u64_at(b, 24) as i64);
        let near = |a: i64, c: i64| ((a.wrapping_sub(c)) as f64 / METERS).abs() < 6.0;
        near(ox, px) && near(oy, py) && near(oz, pz)
    })?;
    if (u64_at(&b, 0) >> slot) & 1 == 0 {
        return None;
    }
    let f = |off: usize| f32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]]);
    let base = 32 + slot * 48;
    let axes = [
        [f(base), f(base + 4), f(base + 8)],
        [f(base + 12), f(base + 16), f(base + 20)],
        [f(base + 24), f(base + 28), f(base + 32)],
    ];
    let w = [f(base + 36), f(base + 40), f(base + 44)];
    if w.iter().any(|v| !v.is_finite() || v.abs() > 5.0) {
        return None;
    }
    let (ox, oy, oz) = (u64_at(&b, 8) as i64, u64_at(&b, 16) as i64, u64_at(&b, 24) as i64);
    let at = (
        ox.wrapping_add((w[0] as f64 * METERS) as i64),
        oy.wrapping_add((w[1] as f64 * METERS) as i64),
        oz.wrapping_add((w[2] as f64 * METERS) as i64),
    );
    Some((at, axes))
}

const HASH_SERVER_SLOT_ATTACHMENT: u32 = 0x430f_162d;
const SERVER_SLOT_ATTACHMENT_SIZE: usize = 112;
const HASH_LIFE_TIME: u32 = 0xf22c_253c;
const LIFE_TIME_SIZE: usize = 24;
pub const HAND_LIFE_MS: u64 = 450;
pub const HAND_EVERY_MS: u64 = 300;

pub fn server_slot_bytes(entity: u32, slot: u8, at: (i64, i64, i64)) -> [u8; SERVER_SLOT_ATTACHMENT_SIZE] {
    let mut b = [0u8; SERVER_SLOT_ATTACHMENT_SIZE];
    b[36..40].copy_from_slice(&1.0f32.to_le_bytes());
    b[41] = slot;
    b[44..48].copy_from_slice(&entity.to_le_bytes());
    b[48..56].copy_from_slice(&(at.0 as u64).to_le_bytes());
    b[56..64].copy_from_slice(&(at.1 as u64).to_le_bytes());
    b[64..72].copy_from_slice(&(at.2 as u64).to_le_bytes());
    b[84..88].copy_from_slice(&1.0f32.to_le_bytes());
    for off in [88, 92, 96] {
        b[off..off + 4].copy_from_slice(&1.0f32.to_le_bytes());
    }
    b
}

pub fn life_time_bytes(ms: u64) -> [u8; LIFE_TIME_SIZE] {
    let mut b = [0u8; LIFE_TIME_SIZE];
    b[0..8].copy_from_slice(&ms.saturating_mul(1_000_000).to_le_bytes());
    b
}

/// Creates `template` glued (on the server) to `slot` of `entity`.
pub fn create_on_slot(manager: usize, template: [u8; 16], entity: u32, slot: u8, at: (i64, i64, i64), life_ms: u64) -> u32 {
    let mut transform = [0u8; TRANSFORM_SIZE];
    transform[0..8].copy_from_slice(&(at.0 as u64).to_le_bytes());
    transform[8..16].copy_from_slice(&(at.1 as u64).to_le_bytes());
    transform[16..24].copy_from_slice(&(at.2 as u64).to_le_bytes());
    transform[36..40].copy_from_slice(&1.0f32.to_le_bytes());
    for off in [40, 44, 48] {
        transform[off..off + 4].copy_from_slice(&1.0f32.to_le_bytes());
    }
    let attach = server_slot_bytes(entity, slot, at);
    let life = life_time_bytes(life_ms);
    let mut entries = [0u8; 0x100];
    let parts: [(u32, usize, usize); 3] = [
        (HASH_CURRENT_TRANSFORM, transform.as_ptr() as usize, TRANSFORM_SIZE),
        (HASH_SERVER_SLOT_ATTACHMENT, attach.as_ptr() as usize, SERVER_SLOT_ATTACHMENT_SIZE),
        (HASH_LIFE_TIME, life.as_ptr() as usize, LIFE_TIME_SIZE),
    ];
    for (i, (hash, ptr, size)) in parts.iter().enumerate() {
        entries[i * 24..i * 24 + 4].copy_from_slice(&hash.to_le_bytes());
        entries[i * 24 + 8..i * 24 + 16].copy_from_slice(&(*ptr as u64).to_le_bytes());
        entries[i * 24 + 16..i * 24 + 24].copy_from_slice(&(*size as u64).to_le_bytes());
    }
    let mut desc = [0u8; 0x100];
    desc[4..20].copy_from_slice(&template);
    desc[0x18..0x20].copy_from_slice(&(entries.as_ptr() as u64).to_le_bytes());
    desc[0x20..0x28].copy_from_slice(&3u64.to_le_bytes());
    original_create(manager, 0, desc.as_ptr() as usize, 0)
}

pub fn queue_hand(template: [u8; 16], entity: u32, at: (i64, i64, i64)) {
    queue_slot(template, entity, SLOT_HAND_R as u8, at, HAND_LIFE_MS);
}

/// Queues `template` glued (on the server) to `slot` of `entity` for `life_ms`.
pub fn queue_slot(template: [u8; 16], entity: u32, slot: u8, at: (i64, i64, i64), life_ms: u64) {
    let Ok(mut q) = QUEUE.lock() else { return };
    if q.len() > 48 {
        return;
    }
    q.push(Spawn { queued: now_ms(), template, at, scale: 1.0, hand: Some((entity, slot)), life_ms: Some(life_ms) });
    QUEUED.store(1, Ordering::SeqCst);
}

pub const SLOT_CHEST: u8 = 2;


// ---------------------------------------------------------------------------------------------
// Walking aura: each player turns it on/off and picks the effect with crouch gestures, which the
// server sees for every player (PC or console) without any key or chat message.
// ---------------------------------------------------------------------------------------------


pub struct Aura {
    pub name: &'static str,
    pub effects: &'static [&'static str],
    pub scale: f32,
    pub glow: Option<u8>,
    pub life_ms: Option<u64>,
    pub every_ms: u64,
}

pub const LINGER_MS: u64 = 1200;

const fn aura(name: &'static str, effects: &'static [&'static str], scale: f32, life_ms: Option<u64>, every_ms: u64) -> Aura {
    Aura { name, effects, scale, glow: None, life_ms, every_ms }
}

const fn glow(name: &'static str, n: u8) -> Aura {
    Aura { name, effects: &[], scale: 1.0, glow: Some(n), life_ms: None, every_ms: HAND_EVERY_MS }
}

pub const GLOW_AURAS: [Aura; 16] = [
    glow("fire", 2),
    glow("lightning", 1),
    glow("frost", 3),
    glow("shroud", 4),
    glow("holy glow", 5),
    glow("poison", 6),
    glow("ectoplasm tendrils", 7),
    glow("arcane", 8),
    glow("blood rage", 9),
    glow("water", 10),
    glow("death fog", 11),
    glow("ectoplasm", 12),
    glow("stars", 13),
    glow("fire ring", 14),
    aura("mana", &["VFX_Spell_ManaDrain_Mana_OnPlayer"], 1.5, Some(LINGER_MS), 150),
    aura("necromancy", &["VFX_Player_Companion_Skeleton_GemOfNecromancy_Spawn"], 1.0, None, 400),
];

/// The walking auras in use: the glows when the glow mod is installed, else game effects only.
pub fn aura_list() -> &'static [Aura] {
    if glows_ready() { &GLOW_AURAS } else { &AURAS }
}

pub const AURAS: [Aura; 7] = [
    aura("fire", &["VFX_Skill_Aura_SunFire_Trigger_OnPlayer", "VFX_Player_Projectile_Wand_EvadeAttack_Impact_Fire"], 1.5, Some(LINGER_MS), 250),
    aura("lightning", &["VFX_Trap_Electro_Impact"], 1.5, Some(LINGER_MS), 250),
    aura("frost", &["VFX_Player_Projectile_Wand_EvadeAttack_Impact_Ice"], 1.5, Some(LINGER_MS), 250),
    aura("shroud", &["VFX_Player_Projectile_Wand_EvadeAttack_Impact_Fog"], 1.0, Some(LINGER_MS), 250),
    aura("holy light", &["VFX_Resource_Increase_Health_NoSound"], 1.5, Some(LINGER_MS), 250),
    aura("mana", &["VFX_Spell_ManaDrain_Mana_OnPlayer"], 1.5, Some(LINGER_MS), 150),
    aura("necromancy", &["VFX_Player_Companion_Skeleton_GemOfNecromancy_Spawn"], 1.0, None, 400),
];

#[derive(Debug, Default, Clone, PartialEq)]
pub struct AuraState {
    pub on: bool,
    pub index: usize,
    pub emote: Option<(u32, u32)>,
    pub last_spawn: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    None,
    Toggle,
    Next,
}

pub const EMOTE_VICTORY: u32 = 1_355_035_795;
pub const EMOTE_GREET: u32 = 1_773_809_286;
const PLAYER_INPUT_SIZE: usize = 1472;
const EMOTE_AT: usize = 696;

/// The last emote the player triggered (version counter, emote id), from the server's
/// `PlayerInput` component.
pub fn last_emote(manager: usize, id: u32) -> Option<(u32, u32)> {
    let b = component_with(manager, id, PLAYER_INPUT_SIZE, |_| true)?;
    Some((u32_at(&b, EMOTE_AT), u32_at(&b, EMOTE_AT + 4)))
}

/// The greeting emote turns the aura on or off; the victory emote switches to the next effect
/// (and turns it on).
pub fn gesture_step(st: &mut AuraState, flags: u64, emote: Option<(u32, u32)>, now: u64) -> Gesture {
    let _ = (flags, now);
    let fresh = match (st.emote, emote) {
        (Some(old), Some(cur)) if cur != old => Some(cur.1),
        _ => None,
    };
    if emote.is_some() {
        st.emote = emote;
    }
    match fresh {
        Some(EMOTE_VICTORY) => {
            st.index = if st.on { (st.index + 1) % aura_list().len() } else { st.index };
            st.on = true;
            Gesture::Next
        }
        Some(EMOTE_GREET) => {
            st.on = !st.on;
            Gesture::Toggle
        }
        _ => Gesture::None,
    }
}

fn position_of(manager: usize, id: u32) -> Option<((i64, i64, i64), (f64, f64))> {
    let t = spawner::transform_of(manager, id)?;
    if t.len() < TRANSFORM_SIZE {
        return None;
    }
    let at = (u64_at(&t, 0) as i64, u64_at(&t, 8) as i64, u64_at(&t, 16) as i64);
    Some((at, spawner::facing_direction(&t)))
}

/// `m182_vfx.json` next to the game, matched without regard to letter case.
pub fn config_path(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let exact = dir.join(CONFIG_FILE);
    if exact.is_file() {
        return Some(exact);
    }
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.is_file() && p.file_name().and_then(|n| n.to_str()).map(|n| n.eq_ignore_ascii_case(CONFIG_FILE)).unwrap_or(false)
    })
}

/// The module is installed when its config or its Lua mod folder is next to the game.
pub fn installed(dir: &std::path::Path) -> bool {
    config_path(dir).is_some() || dir.join("mods").join("m182_vfx").join("mod.json").is_file()
}

fn load(dir: &std::path::Path) -> Option<(VfxConfig, Option<SystemTime>)> {
    let Some(path) = config_path(dir) else {
        return Some((VfxConfig::default(), None));
    };
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
    if !installed(&dir) {
        tracing::info!(target: "m182::vfx", dir = ?dir, "M182 VFX not installed (no m182_vfx.json and no mods/m182_vfx), effects off");
        return;
    }
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = CLOCK.get_or_init(Instant::now);
    let server = std::env::current_exe().map(|p| is_server_exe(&p)).unwrap_or(false);
    set_dedicated(server);
    let glows = !server && glow_mod_installed(&dir);
    set_glows_ready(glows);
    tracing::info!(target: "m182::vfx", dir = ?dir, log = LOG_FILE, glows, server, "M182 VFX started");
    if server {
        log("vfx: started on a dedicated server: staff charge effects only");
    } else {
        log(&format!("vfx: started (custom glows {})", if glows { "on" } else { "off: mods/m182_vfx not installed, game effects only" }));
    }
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
    let mut auras: HashMap<u32, AuraState> = HashMap::new();
    let mut key_was_down = false;
    let mut reported_players = usize::MAX;
    let mut waiting_logged = false;
    loop {
        std::thread::sleep(Duration::from_millis(60));
        if checked.elapsed() >= Duration::from_secs(2) {
            checked = Instant::now();
            let stamp = config_path(&dir).and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
            if stamp != modified || CONFIG.lock().map(|c| c.is_none()).unwrap_or(false) {
                if let Some((cfg, m)) = load(&dir) {
                    modified = m;
                    log(&format!(
                        "vfx: config loaded enabled={} charge={:?} release={:?} melee_charge={:?} melee_release={:?} auto={}",
                        cfg.enabled, cfg.charge.effects, cfg.release.effects, cfg.melee_charge.effects, cfg.melee_release.effects, cfg.auto
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
            auras.retain(|id, _| players.contains(id));
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
            let every = if st.heavy {
                100
            } else if cfg.auto && st.element == Some(Element::Arcane) {
                150
            } else if cfg.auto && st.element == Some(Element::Fire) && glows_ready() && !dedicated() {
                HAND_EVERY_MS
            } else {
                cfg.charge_every_ms
            };
            match step(st, flags, now, every, cfg.min_charge_ms) {
                Event::Charge { melee: false } => {
                    let items = ranged_items(manager, id);
                    let wand = items.and_then(|(_, weapon)| item_kind(weapon)).map(|k| k.wand).unwrap_or(false);
                    if wand {
                        note_once("vfx: wands do not get the staff aura");
                    } else if let Some((at, facing)) = position_of(manager, id) {
                        let element = items
                            .and_then(|(ammo, weapon)| item_element(ammo).filter(|e| *e != Element::Physical).or_else(|| item_element(weapon)))
                            .unwrap_or(Element::Lightning);
                        if st.element != Some(element) {
                            st.element = Some(element);
                            log(&format!("vfx: entity {id} staff element {element:?}"));
                        }
                        let burning = if cfg.auto && element == Element::Fire && glows_ready() && !dedicated() { hand_template(Element::Fire) } else { None };
                        if let Some(template) = burning {
                            let chest = slot_world(manager, id, SLOT_CHEST as usize).map(|(c, _)| c).unwrap_or(at);
                            queue_slot(template, id, SLOT_CHEST, chest, HAND_LIFE_MS);
                        } else {
                            let mut burst = cfg.charge.clone();
                            if cfg.auto {
                                burst.effects = staff_effects(element).iter().map(|s| s.to_string()).collect();
                                burst.scale *= staff_scale(element);
                                if matches!(element, Element::Arcane | Element::Fire | Element::Healing) {
                                    burst.life_ms = Some(LINGER_MS);
                                }
                            }
                            queue_burst(at, facing, &burst);
                        }
                    }
                }
                Event::Charge { melee: true } => {
                    if let Some((at, facing)) = position_of(manager, id) {
                        queue_burst(at, facing, &cfg.melee_charge);
                    }
                    if cfg.auto && !dedicated() {
                        let element = main_hand(manager, id).and_then(|(_, item)| item_element(item)).unwrap_or(Element::Physical);
                        if st.element != Some(element) {
                            st.element = Some(element);
                            log(&format!("vfx: entity {id} weapon element {element:?}"));
                        }
                        if now.saturating_sub(st.last_hand) >= HAND_EVERY_MS {
                            st.last_hand = now;
                            let at = slot_world(manager, id, SLOT_HAND_R).map(|(h, _)| h).or_else(|| position_of(manager, id).map(|(p, _)| p));
                            let template = if glows_ready() { hand_template(element) } else { hand_effect(element) };
                            if let (Some(at), Some(template)) = (at, template) {
                                queue_hand(template, id, at);
                            }
                        }
                    }
                }
                Event::Release { melee } => {
                    if let Some((at, facing)) = position_of(manager, id) {
                        queue_burst(at, facing, if melee { &cfg.melee_release } else { &cfg.release });
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
            let charging = st.since.is_some();
            let aura = auras.entry(id).or_default();
            match gesture_step(aura, flags, last_emote(manager, id), now) {
                Gesture::Toggle => log(&format!("vfx: entity {id} aura {} ({})", if aura.on { "on" } else { "off" }, aura_list()[aura.index % aura_list().len()].name)),
                Gesture::Next => log(&format!("vfx: entity {id} aura {}/{} {}", aura.index + 1, aura_list().len(), aura_list()[aura.index % aura_list().len()].name)),
                Gesture::None => {}
            }
            let a = &aura_list()[aura.index % aura_list().len()];
            if aura.on && !dedicated() && !charging && flags & (1 << FLAG_DEAD) == 0 && now.saturating_sub(aura.last_spawn) >= a.every_ms {
                aura.last_spawn = now;
                if let Some((at, facing)) = position_of(manager, id) {
                    if let Some(template) = a.glow.and_then(glow_template) {
                        let chest = slot_world(manager, id, SLOT_CHEST as usize).map(|(c, _)| c).unwrap_or(at);
                        queue_slot(template, id, SLOT_CHEST, chest, HAND_LIFE_MS);
                    }
                    if !a.effects.is_empty() {
                        let burst = Burst { effects: a.effects.iter().map(|e| e.to_string()).collect(), life_ms: a.life_ms, scale: a.scale, height: 0.9, distance: 0.0, ring: 1, radius: 0.0 };
                        queue_burst(at, facing, &burst);
                    }
                }
            }
        }
    }
}

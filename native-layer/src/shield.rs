//! Object shield: zones whose `shield` flag is on give every decorative object created inside them the
//! game's own `DisableDestructible` component, at the moment the game creates the object. Nothing runs
//! afterwards: no timers, no scans, no hook on damage.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::groups::Zone;
use super::spawn_probe::{read_bytes, u32_at, u64_at};

const HASH_CURRENT_TRANSFORM: u32 = 0x027a_c564;
const HASH_DISABLE_DESTRUCTIBLE: u32 = 0xd169_6237;
const HASH_COMBAT_FLAGS: u32 = 0x8cf9_8906;
const COMBAT_FLAGS_IMMUNE: [u8; 12] = [1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
const ENTRY: usize = 24;
const MAX_ENTRIES: usize = 0x80;
const METERS: f64 = 4_294_967_296.0;
const STATE_FILE: &str = "m182_shield_state.json";
const OPTIONS_FILE: &str = "m182_shield.json";
const CATALOG_FILE: &str = "m182_objects.csv";
const RELEASED_MAX: usize = 200;
const REPORT_SHIELDED: u32 = 8;
const REPORT_RELEASED: u32 = 4;
const REPORT_SKIPPED: u32 = 6;
const SKIPPED_LOOKS: u32 = 300;
const REPORT_BASELINE: u32 = 4;
const REPORT_FAILED: u32 = 5;

/// Components that mark a destructible template as something other than decoration: creatures, traps,
/// spawners, resource nodes and containers that hand out loot when they break.
const NOT_DECORATION: [&str; 22] = [
    "Actor",
    "Health",
    "HealthBar",
    "Faction",
    "CombatFlags",
    "CombatTargetActor",
    "Impact",
    "ImpactHierarchy",
    "EnemySettings",
    "EnemySpawnerClient",
    "MiningNode",
    "ResourceNodePickupDrops",
    "RandomLoot",
    "RandomLootContainer",
    "RandomLootOnDestroy",
    "RandomLootSlotCount",
    "DestroyOnLoot",
    "DestroyOnState",
    "FogVoxelModel",
    "LifeTime",
    "DoNotSave",
    "DisableDestructible",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Area {
    pub name: String,
    pub x: f64,
    pub z: f64,
    pub radius: f64,
}

impl Area {
    fn holds(&self, x: f64, z: f64) -> bool {
        let (dx, dz) = (x - self.x, z - self.z);
        dx * dx + dz * dz <= self.radius * self.radius
    }

    fn same_place(&self, other: &Area) -> bool {
        self.x == other.x && self.z == other.z && self.radius == other.radius
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Areas {
    pub shielded: Vec<Area>,
    /// Places that were shielded once: objects that load there get their shield taken off.
    pub released: Vec<Area>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Options {
    /// Trial: also give the object `CombatFlags` with `immune` set.
    pub immune: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Leave,
    Shield(String),
    Release(String),
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
static IMMUNE: AtomicBool = AtomicBool::new(false);
static LOADED: AtomicBool = AtomicBool::new(false);
static AREAS: Mutex<Areas> = Mutex::new(Areas { shielded: Vec::new(), released: Vec::new() });
static CATALOG: Mutex<Option<HashSet<[u8; 16]>>> = Mutex::new(None);
static CATALOG_MISSING: AtomicBool = AtomicBool::new(false);
static SHIELDED: AtomicU32 = AtomicU32::new(0);
static RELEASED: AtomicU32 = AtomicU32::new(0);
static SKIPPED: AtomicU32 = AtomicU32::new(0);
static LOOKS: AtomicU32 = AtomicU32::new(0);
static BASELINE: AtomicU32 = AtomicU32::new(0);
static FAILED: AtomicU32 = AtomicU32::new(0);
static TAG: [u8; 8] = [0; 8];

fn log(text: &str) {
    super::spawner::log(text);
}

pub fn shielded_areas(zones: &[Zone]) -> Vec<Area> {
    zones.iter().filter(|z| z.shields()).map(|z| Area { name: z.name.clone(), x: z.x, z: z.z, radius: z.radius }).collect()
}

/// The areas after the zones changed: what stopped being shielded is remembered as released, and a
/// released place that is shielded again is forgotten.
pub fn next_areas(old: &Areas, shielded: Vec<Area>) -> Areas {
    let mut released: Vec<Area> = old.released.iter().filter(|r| !shielded.iter().any(|s| s.same_place(r))).cloned().collect();
    for gone in old.shielded.iter().filter(|o| !shielded.iter().any(|s| s.same_place(o))) {
        if !released.iter().any(|r| r.same_place(gone)) {
            released.push(gone.clone());
        }
    }
    if released.len() > RELEASED_MAX {
        let extra = released.len() - RELEASED_MAX;
        released.drain(0..extra);
    }
    Areas { shielded, released }
}

/// What to do with an object the game is about to create.
pub fn decide(areas: &Areas, decoration: bool, at: Option<(f64, f64)>, has_tag: bool) -> Decision {
    let (true, Some((x, z))) = (decoration, at) else { return Decision::Leave };
    if let Some(area) = areas.shielded.iter().find(|a| a.holds(x, z)) {
        return if has_tag { Decision::Leave } else { Decision::Shield(area.name.clone()) };
    }
    match areas.released.iter().find(|a| a.holds(x, z)) {
        Some(area) if has_tag => Decision::Release(area.name.clone()),
        _ => Decision::Leave,
    }
}

/// Templates of the object list that are destructible decoration.
pub fn parse_catalog(text: &str) -> HashSet<[u8; 16]> {
    let mut out = HashSet::new();
    for line in text.lines().skip(1) {
        let mut parts = line.splitn(3, ',');
        let (Some(_), Some(guid), Some(components)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let mut destructible = false;
        let mut other = false;
        for c in components.trim().split('|') {
            destructible |= c == "Destructible";
            other |= NOT_DECORATION.contains(&c);
        }
        if destructible && !other {
            if let Some(g) = super::config::parse_guid(guid) {
                out.insert(g);
            }
        }
    }
    out
}

/// The component list with one more entry.
pub fn with_entry(entries: &[u8], hash: u32, data: usize, size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(entries.len() + ENTRY);
    out.extend_from_slice(entries);
    let mut extra = [0u8; ENTRY];
    extra[..4].copy_from_slice(&hash.to_le_bytes());
    extra[8..16].copy_from_slice(&(data as u64).to_le_bytes());
    extra[16..24].copy_from_slice(&(size as u64).to_le_bytes());
    out.extend_from_slice(&extra);
    out
}

/// The component list without the entries of one component.
pub fn without_entry(entries: &[u8], hash: u32) -> Vec<u8> {
    entries.chunks_exact(ENTRY).filter(|e| u32_at(e, 0) != hash).flatten().copied().collect()
}

fn has_entry(entries: &[u8], hash: u32) -> bool {
    entries.chunks_exact(ENTRY).any(|e| u32_at(e, 0) == hash)
}

fn position(entries: &[u8]) -> Option<(f64, f64)> {
    let e = entries.chunks_exact(ENTRY).find(|e| u32_at(e, 0) == HASH_CURRENT_TRANSFORM && u64_at(e, 16) >= 24)?;
    let t = read_bytes(u64_at(e, 8) as usize, 24)?;
    if t.len() < 24 {
        return None;
    }
    Some((u64_at(&t, 0) as i64 as f64 / METERS, u64_at(&t, 16) as i64 as f64 / METERS))
}

fn load_catalog(dir: &std::path::Path) {
    let Ok(mut catalog) = CATALOG.lock() else { return };
    if catalog.is_some() {
        return;
    }
    match std::fs::read_to_string(dir.join("export").join(CATALOG_FILE)) {
        Ok(text) => {
            let set = parse_catalog(&text);
            log(&format!("object shield: {} kinds of decoration can be shielded", set.len()));
            CATALOG_MISSING.store(set.is_empty(), Ordering::SeqCst);
            if set.is_empty() {
                log("object shield: ERROR export/m182_objects.csv lists no destructible decoration; nothing will be shielded");
            } else {
                *catalog = Some(set);
            }
        }
        Err(e) => {
            if !CATALOG_MISSING.swap(true, Ordering::SeqCst) {
                log(&format!("object shield: ERROR export/m182_objects.csv cannot be read ({e}); nothing will be shielded until the mod m182_spawn_variants has written it"));
            }
        }
    }
}

fn is_decoration(template: &[u8]) -> bool {
    let Ok(catalog) = CATALOG.lock() else { return false };
    let mut key = [0u8; 16];
    key.copy_from_slice(template);
    catalog.as_ref().is_some_and(|c| c.contains(&key))
}

fn save(dir: &std::path::Path, areas: &Areas) {
    let Ok(text) = serde_json::to_string(areas) else { return };
    let tmp = dir.join("m182_shield_state.tmp");
    if std::fs::write(&tmp, text).is_err() || std::fs::rename(&tmp, dir.join(STATE_FILE)).is_err() {
        log("object shield: ERROR m182_shield_state.json could not be written; objects of a zone whose shield is switched off may keep it after a restart");
    }
}

/// Called when the zones were read again (they changed, or the game just started).
pub fn set_zones(dir: &std::path::Path, zones: &[Zone]) {
    let shielded = shielded_areas(zones);
    let Ok(mut areas) = AREAS.lock() else { return };
    if !LOADED.swap(true, Ordering::SeqCst) {
        if let Ok(text) = std::fs::read_to_string(dir.join(STATE_FILE)) {
            *areas = serde_json::from_str(&text).unwrap_or_default();
        }
    }
    let next = next_areas(&areas, shielded);
    let options: Options = std::fs::read_to_string(dir.join(OPTIONS_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let immune_changed = IMMUNE.swap(options.immune, Ordering::SeqCst) != options.immune;
    let active = !next.shielded.is_empty() || !next.released.is_empty();
    if next != *areas || immune_changed {
        let names: Vec<&str> = next.shielded.iter().map(|a| a.name.as_str()).collect();
        log(&format!(
            "object shield: {} zone(s) shield what is built inside {:?}, {} place(s) released{}",
            next.shielded.len(),
            names,
            next.released.len(),
            if options.immune { ", trial: immune to hits" } else { "" }
        ));
        if next != *areas {
            save(dir, &next);
        }
        *areas = next;
    }
    drop(areas);
    if active {
        load_catalog(dir);
    }
    ACTIVE.store(active, Ordering::SeqCst);
}

/// What `before` changed in the game's descriptor, to be put back by `after`.
pub struct Swap {
    desc: usize,
    original: [u8; 16],
    _entries: Vec<u8>,
    report: Option<String>,
}

fn fail(text: &str) {
    if FAILED.fetch_add(1, Ordering::Relaxed) < REPORT_FAILED {
        log(&format!("object shield: ERROR {text}"));
    }
}

fn describe(entries: &[u8]) -> String {
    entries.chunks_exact(ENTRY).map(|e| format!("{:08x}:{}", u32_at(e, 0), u64_at(e, 16))).collect::<Vec<_>>().join(" ")
}

/// Called before the game creates an entity from `desc`. When the object is decoration inside a
/// shielded zone the component list of the descriptor is swapped for one that also carries the shield.
pub fn before(manager: usize, kind: u32, desc: usize) -> Option<Swap> {
    if !ACTIVE.load(Ordering::Relaxed) || desc < 0x10000 || !super::spawner::is_server(manager) {
        return None;
    }
    let head = read_bytes(desc, 0x28)?;
    if head.len() < 0x28 {
        return None;
    }
    let decoration = is_decoration(&head[4..20]);
    if !decoration && (SKIPPED.load(Ordering::Relaxed) >= REPORT_SKIPPED || LOOKS.fetch_add(1, Ordering::Relaxed) >= SKIPPED_LOOKS) {
        return None;
    }
    let array = u64_at(&head, 0x18) as usize;
    let count = u64_at(&head, 0x20) as usize;
    if count == 0 || count > MAX_ENTRIES {
        return None;
    }
    let entries = read_bytes(array, count * ENTRY)?;
    if entries.len() < count * ENTRY {
        return None;
    }
    let at = position(&entries);
    let has_tag = has_entry(&entries, HASH_DISABLE_DESTRUCTIBLE);
    let areas = AREAS.lock().ok()?;
    let decision = decide(&areas, decoration, at, has_tag);
    let template = super::spawner::guid_text(&head[4..20]);
    let place = at.map(|(x, z)| format!("x={x:.1} z={z:.1}")).unwrap_or_else(|| "no position".into());
    if !decoration {
        if let Some((x, z)) = at {
            if let Some(area) = areas.shielded.iter().find(|a| a.holds(x, z)) {
                if SKIPPED.fetch_add(1, Ordering::Relaxed) < REPORT_SKIPPED {
                    log(&format!("object shield: template {template} (kind {kind:#x}) at {place} in '{}' is not decoration, left as it is; given [{}]", area.name, describe(&entries)));
                }
            }
        }
        return None;
    }
    drop(areas);
    let (new_entries, report) = match &decision {
        Decision::Leave => {
            if !has_tag && at.is_some() && BASELINE.fetch_add(1, Ordering::Relaxed) < REPORT_BASELINE {
                let report = format!("object shield: for comparison, template {template} (kind {kind:#x}) at {place} outside every shielded zone; given [{}]", describe(&entries));
                return Some(Swap { desc, original: [0; 16], _entries: Vec::new(), report: Some(report) });
            }
            return None;
        }
        Decision::Shield(zone) => {
            let extra = if IMMUNE.load(Ordering::Relaxed) && !has_entry(&entries, HASH_COMBAT_FLAGS) { 2 } else { 1 };
            if count + extra > MAX_ENTRIES {
                fail(&format!("template {template} already has {count} components, the shield does not fit"));
                return None;
            }
            let mut list = with_entry(&entries, HASH_DISABLE_DESTRUCTIBLE, TAG.as_ptr() as usize, 1);
            if extra == 2 {
                list = with_entry(&list, HASH_COMBAT_FLAGS, COMBAT_FLAGS_IMMUNE.as_ptr() as usize, COMBAT_FLAGS_IMMUNE.len());
            }
            let report = (SHIELDED.fetch_add(1, Ordering::Relaxed) < REPORT_SHIELDED)
                .then(|| format!("object shield: template {template} (kind {kind:#x}) at {place} in '{zone}' shielded{}; given [{}]", if extra == 2 { " and immune" } else { "" }, describe(&entries)));
            (list, report)
        }
        Decision::Release(zone) => {
            let list = without_entry(&entries, HASH_DISABLE_DESTRUCTIBLE);
            if list.is_empty() {
                return None;
            }
            let report = (RELEASED.fetch_add(1, Ordering::Relaxed) < REPORT_RELEASED)
                .then(|| format!("object shield: template {template} (kind {kind:#x}) at {place} in released place '{zone}': shield taken off; given [{}]", describe(&entries)));
            (list, report)
        }
    };
    let mut original = [0u8; 16];
    original.copy_from_slice(&head[0x18..0x28]);
    let mut patched = [0u8; 16];
    patched[..8].copy_from_slice(&(new_entries.as_ptr() as u64).to_le_bytes());
    patched[8..].copy_from_slice(&((new_entries.len() / ENTRY) as u64).to_le_bytes());
    if !super::spawner::write_bytes(desc + 0x18, &patched) {
        fail(&format!("the component list of template {template} could not be replaced; object left as it is"));
        return None;
    }
    Some(Swap { desc, original, _entries: new_entries, report })
}

/// Called right after the game created (or refused to create) the entity.
pub fn after(swap: Option<Swap>, manager: usize, entity: u32) {
    let Some(swap) = swap else { return };
    let swapped = !swap._entries.is_empty();
    if swapped && !super::spawner::write_bytes(swap.desc + 0x18, &swap.original) {
        fail("the game's component list could not be put back");
    }
    if swapped && entity == 0 {
        fail("the game did not create an object that was given the shield");
    }
    if let Some(report) = swap.report {
        let made = super::spawner::component_summary(manager, entity).unwrap_or_else(|| "not readable".into());
        log(&format!("{report}; entity {entity} has {made}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(name: &str, x: f64, z: f64, radius: f64) -> Area {
        Area { name: name.into(), x, z, radius }
    }

    #[test]
    fn only_zones_with_the_flag_shield() {
        let zones = super::super::groups::parse_zones(
            r#"{"zones":[{"name":"Town","x":10,"z":20,"radius":30,"shield":true},{"name":"Old","x":0,"z":0,"radius":5},{"name":"Off","x":0,"z":0,"radius":5,"shield":false},{"name":"Disabled","x":1,"z":1,"radius":5,"enabled":false,"shield":true}]}"#,
        );
        assert_eq!(shielded_areas(&zones), vec![area("Town", 10.0, 20.0, 30.0)]);
    }

    #[test]
    fn decoration_inside_a_shielded_zone_gets_the_shield_once() {
        let areas = Areas { shielded: vec![area("Town", 0.0, 0.0, 30.0)], released: Vec::new() };
        assert_eq!(decide(&areas, true, Some((10.0, 10.0)), false), Decision::Shield("Town".into()));
        assert_eq!(decide(&areas, true, Some((10.0, 10.0)), true), Decision::Leave);
        assert_eq!(decide(&areas, true, Some((100.0, 10.0)), false), Decision::Leave);
        assert_eq!(decide(&areas, false, Some((10.0, 10.0)), false), Decision::Leave);
        assert_eq!(decide(&areas, true, None, false), Decision::Leave);
    }

    #[test]
    fn switching_the_flag_off_releases_what_loads_there_later() {
        let on = next_areas(&Areas::default(), vec![area("Town", 0.0, 0.0, 30.0)]);
        assert!(on.released.is_empty());
        let off = next_areas(&on, Vec::new());
        assert_eq!(off.released, vec![area("Town", 0.0, 0.0, 30.0)]);
        assert_eq!(decide(&off, true, Some((1.0, 1.0)), true), Decision::Release("Town".into()));
        assert_eq!(decide(&off, true, Some((1.0, 1.0)), false), Decision::Leave);
        assert_eq!(decide(&off, true, Some((500.0, 1.0)), true), Decision::Leave);
        let again = next_areas(&off, vec![area("Town", 0.0, 0.0, 30.0)]);
        assert!(again.released.is_empty());
        assert_eq!(decide(&again, true, Some((1.0, 1.0)), false), Decision::Shield("Town".into()));
    }

    #[test]
    fn a_shielded_zone_wins_over_a_released_place_it_overlaps() {
        let areas = Areas { shielded: vec![area("New", 0.0, 0.0, 30.0)], released: vec![area("Old", 5.0, 0.0, 30.0)] };
        assert_eq!(decide(&areas, true, Some((4.0, 0.0)), true), Decision::Leave);
        assert_eq!(decide(&areas, true, Some((34.0, 0.0)), true), Decision::Release("Old".into()));
    }

    #[test]
    fn moving_or_resizing_a_zone_releases_the_old_place() {
        let on = next_areas(&Areas::default(), vec![area("Town", 0.0, 0.0, 30.0)]);
        let moved = next_areas(&on, vec![area("Town", 0.0, 0.0, 50.0)]);
        assert_eq!(moved.released, vec![area("Town", 0.0, 0.0, 30.0)]);
        assert_eq!(decide(&moved, true, Some((1.0, 1.0)), true), Decision::Leave);
    }

    #[test]
    fn released_places_are_capped() {
        let mut areas = Areas::default();
        for i in 0..(RELEASED_MAX + 20) {
            areas = next_areas(&areas, vec![area("Z", i as f64 * 100.0, 0.0, 10.0)]);
        }
        assert_eq!(areas.released.len(), RELEASED_MAX);
        assert_eq!(areas.shielded.len(), 1);
    }

    #[test]
    fn the_catalog_keeps_decoration_and_drops_creatures_nodes_and_loot() {
        let text = "name,guid,components\n\
            Torch,11111111-1111-1111-1111-111111111111,AttachToSurface|ColliderResourceComponent|CurrentTransform|Destructible\n\
            Station,22222222-2222-2222-2222-222222222222,Crafting|CurrentTransform\n\
            Ore,33333333-3333-3333-3333-333333333333,CurrentTransform|Destructible|MiningNode\n\
            Crate,44444444-4444-4444-4444-444444444444,CurrentTransform|Destructible|RandomLootContainer\n\
            Dummy,55555555-5555-5555-5555-555555555555,CurrentTransform|Destructible|Faction|Health\n\
            Broken,not-a-guid,Destructible\n";
        let set = parse_catalog(text);
        assert_eq!(set.len(), 1);
        assert!(set.contains(&super::super::config::parse_guid("11111111-1111-1111-1111-111111111111").unwrap()));
    }

    #[test]
    fn entries_are_added_and_removed_without_touching_the_others() {
        let mut entries = Vec::new();
        for (hash, data, size) in [(HASH_CURRENT_TRANSFORM, 0x1000usize, 56usize), (0x1234_5678, 0x2000, 4)] {
            entries = with_entry(&entries, hash, data, size);
        }
        let more = with_entry(&entries, HASH_DISABLE_DESTRUCTIBLE, 0x3000, 1);
        assert_eq!(more.len(), 3 * ENTRY);
        assert_eq!(&more[..2 * ENTRY], &entries[..]);
        assert_eq!(u32_at(&more, 2 * ENTRY), HASH_DISABLE_DESTRUCTIBLE);
        assert_eq!(u64_at(&more, 2 * ENTRY + 8), 0x3000);
        assert_eq!(u64_at(&more, 2 * ENTRY + 16), 1);
        assert!(has_entry(&more, HASH_DISABLE_DESTRUCTIBLE) && !has_entry(&entries, HASH_DISABLE_DESTRUCTIBLE));
        assert_eq!(without_entry(&more, HASH_DISABLE_DESTRUCTIBLE), entries);
        assert_eq!(without_entry(&entries, HASH_DISABLE_DESTRUCTIBLE), entries);
    }

    #[test]
    fn the_state_file_round_trips_and_tolerates_old_or_broken_content() {
        let areas = Areas { shielded: vec![area("Town", 1.0, 2.0, 30.0)], released: vec![area("Old", 5.0, 6.0, 10.0)] };
        let text = serde_json::to_string(&areas).unwrap();
        assert_eq!(serde_json::from_str::<Areas>(&text).unwrap(), areas);
        assert_eq!(serde_json::from_str::<Areas>("{}").unwrap(), Areas::default());
        assert_eq!(serde_json::from_str::<Options>(r#"{"immune":true}"#).unwrap(), Options { immune: true });
        assert_eq!(serde_json::from_str::<Options>("{}").unwrap(), Options::default());
    }

    fn transform(x: f64, z: f64) -> Box<[u8; 56]> {
        let mut t = Box::new([0u8; 56]);
        t[..8].copy_from_slice(&((x * METERS) as i64).to_le_bytes());
        t[16..24].copy_from_slice(&((z * METERS) as i64).to_le_bytes());
        t
    }

    fn descriptor(template: &[u8; 16], entries: &[u8]) -> Box<[u8; 0x30]> {
        let mut d = Box::new([0u8; 0x30]);
        d[4..20].copy_from_slice(template);
        d[0x18..0x20].copy_from_slice(&(entries.as_ptr() as u64).to_le_bytes());
        d[0x20..0x28].copy_from_slice(&((entries.len() / ENTRY) as u64).to_le_bytes());
        d
    }

    #[test]
    fn the_descriptor_carries_the_shield_only_while_the_game_creates_the_object() {
        let torch = super::super::config::parse_guid("11111111-1111-1111-1111-111111111111").unwrap();
        let ore = super::super::config::parse_guid("33333333-3333-3333-3333-333333333333").unwrap();
        *CATALOG.lock().unwrap() = Some(HashSet::from([torch]));
        *AREAS.lock().unwrap() = Areas { shielded: vec![area("Town", 100.0, 100.0, 30.0)], released: vec![area("Old", 500.0, 500.0, 30.0)] };
        LOADED.store(true, Ordering::SeqCst);
        IMMUNE.store(false, Ordering::SeqCst);
        ACTIVE.store(true, Ordering::SeqCst);
        let server = Box::new([0u8; 16]);
        let mut client = Box::new([0u8; 16]);
        client[8] = 1;
        let (server, client) = (server.as_ptr() as usize, client.as_ptr() as usize);

        let inside = transform(110.0, 95.0);
        let entries = with_entry(&[], HASH_CURRENT_TRANSFORM, inside.as_ptr() as usize, 56);
        let desc = descriptor(&torch, &entries);
        let at = desc.as_ptr() as usize;
        let before_bytes = *desc;

        assert!(before(client, 3, at).is_none());
        let swap = before(server, 3, at).expect("decoration inside the zone is shielded");
        let seen = read_bytes(at, 0x28).unwrap();
        assert_eq!(u64_at(&seen, 0x20), 2);
        let list = read_bytes(u64_at(&seen, 0x18) as usize, 2 * ENTRY).unwrap();
        assert_eq!(&list[..ENTRY], &entries[..]);
        assert_eq!(u32_at(&list, ENTRY), HASH_DISABLE_DESTRUCTIBLE);
        assert_eq!(u64_at(&list, ENTRY + 16), 1);
        assert_eq!(read_bytes(u64_at(&list, ENTRY + 8) as usize, 1).unwrap(), vec![0]);
        after(Some(swap), server, 0);
        assert_eq!(read_bytes(at, 0x30).unwrap(), before_bytes.to_vec());

        IMMUNE.store(true, Ordering::SeqCst);
        let swap = before(server, 3, at).unwrap();
        let seen = read_bytes(at, 0x28).unwrap();
        assert_eq!(u64_at(&seen, 0x20), 3);
        let list = read_bytes(u64_at(&seen, 0x18) as usize, 3 * ENTRY).unwrap();
        assert_eq!(u32_at(&list, 2 * ENTRY), HASH_COMBAT_FLAGS);
        assert_eq!(read_bytes(u64_at(&list, 2 * ENTRY + 8) as usize, 12).unwrap(), COMBAT_FLAGS_IMMUNE.to_vec());
        after(Some(swap), server, 0);
        assert_eq!(read_bytes(at, 0x30).unwrap(), before_bytes.to_vec());
        IMMUNE.store(false, Ordering::SeqCst);

        let ore_desc = descriptor(&ore, &entries);
        assert!(before(server, 3, ore_desc.as_ptr() as usize).is_none());
        assert_eq!(*ore_desc, *descriptor(&ore, &entries));

        let old = transform(505.0, 500.0);
        let tagged = with_entry(&with_entry(&[], HASH_CURRENT_TRANSFORM, old.as_ptr() as usize, 56), HASH_DISABLE_DESTRUCTIBLE, TAG.as_ptr() as usize, 1);
        let old_desc = descriptor(&torch, &tagged);
        let old_at = old_desc.as_ptr() as usize;
        let swap = before(server, 3, old_at).expect("a shielded object loading in a released place is released");
        let seen = read_bytes(old_at, 0x28).unwrap();
        assert_eq!(u64_at(&seen, 0x20), 1);
        assert!(!has_entry(&read_bytes(u64_at(&seen, 0x18) as usize, ENTRY).unwrap(), HASH_DISABLE_DESTRUCTIBLE));
        after(Some(swap), server, 0);
        assert_eq!(read_bytes(old_at, 0x30).unwrap(), old_desc.to_vec());

        ACTIVE.store(false, Ordering::SeqCst);
        assert!(before(server, 3, at).is_none());
        assert_eq!(read_bytes(at, 0x30).unwrap(), before_bytes.to_vec());
        *AREAS.lock().unwrap() = Areas::default();
        *CATALOG.lock().unwrap() = None;
    }
}

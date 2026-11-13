//! Protected objects: inside a zone whose `shield` flag is on, what players built with the hammer cannot be
//! broken by hits, by anybody. Every object created or loaded there carries `DisableDestructible` (the game's
//! switch for "Destroy entity if Health reaches 0") and `CombatFlags.immune`, added to the creation request
//! the game is about to run. The hammer's own removal does not go through either component and still works.
//! Objects the game already created before the zone protected them cannot get a component afterwards; they are
//! counted and named in the log, and get it when the game creates them again (server start). Numbers an
//! earlier version multiplied (`n * 40 + 39`, `n * 120 + 117`, a few fixed codes) are divided back. Work
//! happens only when the game creates an entity or the zones change: no timers, no hook on damage.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::groups::Zone;
use super::spawn_probe::{read_bytes, u32_at, u64_at};
use super::spawner;

const HASH_CURRENT_TRANSFORM: u32 = 0x027a_c564;
const HASH_DESTRUCTIBLE: u32 = 0x6cb1_eacd;
const HASH_USED_ITEM: u32 = 0x0b02_361d;
const HASH_HEALTH: u32 = 0xdc10_1091;
const HASH_COMBAT_FLAGS: u32 = 0x8cf9_8906;
const HASH_DISABLE_DESTRUCTIBLE: u32 = 0xd169_6237;
static COMBAT_FLAGS_IMMUNE: [u8; 12] = [1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
static DISABLE_DESTRUCTIBLE: [u8; 1] = [0];
const COMBAT_FLAGS_SIZE: usize = 12;
const IMMUNE_MAX: usize = 50_000;
const TRANSFORM_SIZE: usize = 56;
const DESTRUCTIBLE_SIZE: usize = 2;
const USED_ITEM_SIZE: usize = 4;
const TYPE_MAPS: [usize; 2] = [0x88, 0x30];
const FACTOR: u32 = 40;
const MARK: u32 = FACTOR - 1;
const WOOD_FACTOR: u32 = 120;
const WOOD_MARK: u32 = WOOD_FACTOR - 3;
#[cfg(test)]
const SMALL_MAX: u16 = ((u16::MAX as u32 - MARK) / FACTOR) as u16;
#[cfg(test)]
const WOOD_SMALL_MAX: u16 = ((u16::MAX as u32 - WOOD_MARK) / WOOD_FACTOR) as u16;
/// Originals too large to be multiplied inside 16 bits, and the fixed codes they are stored as.
const LARGE: [(u16, u16); 4] = [(1000, 65398), (2000, 65518), (4000, 65478), (10000, 65438)];
const SKIP_PREFIXES: [&str; 7] = ["NPC_", "Animal_", "Tool_", "Consumable_", "Blueprint_", "BuildTool_", "Building_"];
const WOOD_WORDS: [&str; 7] = ["wood", "wooden", "timber", "plank", "log", "bamboo", "palisade"];
const HEALTH_SIZE: usize = 40;
const HEALTH_CURRENT_AT: usize = 8;
const HEALTH_MAX_AT: usize = 36;
#[cfg(test)]
const HEALTH_MAX_LIMIT: u32 = 100_000_000;
const ENTRY: usize = 24;
const MAX_ENTRIES: usize = 0x80;
const METERS: f64 = 4_294_967_296.0;
const STATE_FILE: &str = "m182_shield_state.json";
const OPTIONS_FILE: &str = "m182_shield.json";
const ITEMS_FILE: &str = "m182_items.csv";
const CATALOG_FILE: &str = "m182_objects.csv";
const RELEASED_MAX: usize = 200;
const SETTLE: Duration = Duration::from_millis(300);
const WAITING_MAX: usize = 16384;
const WAITING_PER_EVENT: usize = 64;
const SWEEP_PER_EVENT: usize = 400;

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
    /// Places that were shielded once: built objects found there get their shield taken off.
    pub released: Vec<Area>,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(default)]
pub struct Options {
    /// Objects built in a protected zone are created with `CombatFlags.immune`.
    pub immune: bool,
    /// Objects built in a protected zone are created with `DisableDestructible`.
    pub disable: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { immune: true, disable: true }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Leave,
    Shield(String),
    Release(String),
}

/// Where the components this module needs sit in an entity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slots {
    pub transform: usize,
    pub destructible: usize,
    pub used_item: usize,
    pub health: Option<usize>,
    pub disable: Option<usize>,
    pub combat: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Gone,
    NotBuilt,
    OtherItem(u32),
    NoDestructible(u32),
    NoPosition(u32),
    Outside,
    Protected { zone: String, item: u32, disable: bool, immune: bool },
    Unprotected { zone: String, item: u32, shared: bool },
    Released { zone: String, item: u32, edit: Edit },
    Failed(String),
}

/// How a built item is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Normal,
    Wood,
    /// Creatures, tools and the like that the hammer menu also lists: never reinforced.
    Skip,
}

/// What was written into one object.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Edit {
    pub threshold: Option<(u16, u16)>,
    pub health_max: Option<(u32, u32)>,
    pub health_current: Option<(u32, u32)>,
    pub immune_off: bool,
}

impl Edit {
    fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some((a, b)) = self.threshold {
            parts.push(format!("break threshold {a} -> {b}"));
        }
        if let Some((a, b)) = self.health_max {
            parts.push(format!("health maximum {a} -> {b}"));
        }
        if let Some((a, b)) = self.health_current {
            parts.push(format!("health now {a} -> {b}"));
        }
        if self.immune_off {
            parts.push("immunity to hits removed".to_string());
        }
        parts.join(", ")
    }

    fn is_empty(&self) -> bool {
        self.threshold.is_none() && self.health_max.is_none() && !self.immune_off
    }
}

#[derive(Default)]
struct Tally {
    checked: u32,
    protected: u32,
    unprotected: u32,
    shared: u32,
    released: u32,
    nothing_to_break: u32,
}

struct Sweep {
    ids: Vec<u32>,
    next: usize,
    tally: Tally,
}

struct Work {
    waiting: VecDeque<(u32, Instant)>,
    sweep: Option<Sweep>,
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
static IMMUNE: AtomicBool = AtomicBool::new(true);
static DISABLE: AtomicBool = AtomicBool::new(true);
static EARLY: AtomicBool = AtomicBool::new(false);
static IMMUNE_SET: Mutex<Option<HashSet<u32>>> = Mutex::new(None);
static PROTECTED_NOW: AtomicU32 = AtomicU32::new(0);
static UNPROTECTED_NOW: AtomicU32 = AtomicU32::new(0);
static LOADED: AtomicBool = AtomicBool::new(false);
static SWEEP_WANTED: AtomicBool = AtomicBool::new(false);
static HAS_WORK: AtomicBool = AtomicBool::new(false);
static AREAS: Mutex<Areas> = Mutex::new(Areas { shielded: Vec::new(), released: Vec::new() });
static ITEMS: Mutex<Option<HashMap<u32, Class>>> = Mutex::new(None);
static CATALOG: Mutex<Option<HashSet<[u8; 16]>>> = Mutex::new(None);
static SLOTS: Mutex<Option<(usize, usize, Option<Slots>)>> = Mutex::new(None);
static WORK: Mutex<Work> = Mutex::new(Work { waiting: VecDeque::new(), sweep: None });
static ITEMS_MISSING: AtomicBool = AtomicBool::new(false);

struct Budget(AtomicU32, u32);

impl Budget {
    const fn new(limit: u32) -> Self {
        Self(AtomicU32::new(0), limit)
    }

    fn take(&self) -> bool {
        self.0.fetch_add(1, Ordering::Relaxed) < self.1
    }
}

static SAY_PLACED: Budget = Budget::new(8);
static SAY_PLACED_OUTSIDE: Budget = Budget::new(3);
static SAY_PROTECTED: Budget = Budget::new(12);
static SAY_UNPROTECTED: Budget = Budget::new(8);
static SAY_LOADED: Budget = Budget::new(8);
static SAY_REFUSED: Budget = Budget::new(2);
static SAY_RELEASED: Budget = Budget::new(6);
static SAY_NO_DESTRUCTIBLE: Budget = Budget::new(3);
static SAY_OTHER_ITEM: Budget = Budget::new(4);
static SAY_NO_POSITION: Budget = Budget::new(2);
static SAY_FAILED: Budget = Budget::new(6);
static SAY_TABLE: Budget = Budget::new(4);
static SLOT_TRIES: AtomicU32 = AtomicU32::new(1);
const SLOT_RETRY_EVERY: u32 = 64;

fn log(text: &str) {
    spawner::log(text);
}

fn fail(text: &str) {
    if SAY_FAILED.take() {
        log(&format!("object shield: ERROR {text}"));
    }
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

/// What to do with a built object standing at `x`, `z`.
pub fn decide(areas: &Areas, x: f64, z: f64) -> Decision {
    if let Some(area) = areas.shielded.iter().find(|a| a.holds(x, z)) {
        return Decision::Shield(area.name.clone());
    }
    match areas.released.iter().find(|a| a.holds(x, z)) {
        Some(area) => Decision::Release(area.name.clone()),
        None => Decision::Leave,
    }
}

/// The break threshold of a reinforced object, or `None` when the value cannot be reinforced.
#[cfg(test)]
pub fn reinforced_value(value: u16, wood: bool) -> Option<u16> {
    if value == 0 || is_reinforced(value) {
        return None;
    }
    let (factor, mark, small) = if wood { (WOOD_FACTOR, WOOD_MARK, WOOD_SMALL_MAX) } else { (FACTOR, MARK, SMALL_MAX) };
    if value <= small {
        return Some((value as u32 * factor + mark) as u16);
    }
    LARGE.iter().find(|(original, _)| *original == value).map(|(_, code)| *code)
}

pub fn is_reinforced(value: u16) -> bool {
    original_value(value).is_some()
}

/// The break threshold the object had before it was reinforced.
pub fn original_value(value: u16) -> Option<u16> {
    if let Some((original, _)) = LARGE.iter().find(|(_, code)| *code == value) {
        return Some(*original);
    }
    let v = value as u32;
    if v % FACTOR == MARK && v > MARK {
        return Some(((v - MARK) / FACTOR) as u16);
    }
    (v % WOOD_FACTOR == WOOD_MARK && v > WOOD_MARK).then(|| ((v - WOOD_MARK) / WOOD_FACTOR) as u16)
}

/// Health maximum and current after the object was reinforced. A full object stays full.
#[cfg(test)]
pub fn reinforced_health(current: u32, max: u32, wood: bool) -> Option<(u32, u32)> {
    if max == 0 || max > HEALTH_MAX_LIMIT || is_reinforced_max(max) {
        return None;
    }
    let (factor, mark) = if wood { (WOOD_FACTOR, WOOD_MARK) } else { (FACTOR, MARK) };
    let to = u32::try_from(max as u64 * factor as u64 + mark as u64).ok()?;
    Some((scaled(current, max, to), to))
}

fn is_reinforced_max(max: u32) -> bool {
    original_max(max).is_some()
}

fn original_max(max: u32) -> Option<u32> {
    if max <= u16::MAX as u32 {
        return original_value(max as u16).map(u32::from);
    }
    if max % FACTOR == MARK {
        return Some((max - MARK) / FACTOR);
    }
    (max % WOOD_FACTOR == WOOD_MARK).then(|| (max - WOOD_MARK) / WOOD_FACTOR)
}

/// Health maximum and current as they were before the object was reinforced.
pub fn released_health(current: u32, max: u32) -> Option<(u32, u32)> {
    let to = original_max(max)?;
    Some((scaled(current, max, to), to))
}

fn scaled(current: u32, from_max: u32, to_max: u32) -> u32 {
    if current >= from_max {
        return to_max;
    }
    if current == 0 {
        return 0;
    }
    ((current as u64 * to_max as u64 * 2 + from_max as u64) / (from_max as u64 * 2)).clamp(1, to_max as u64) as u32
}

fn words(name: &str) -> Vec<String> {
    let mut spaced = String::new();
    let mut last_lower = false;
    for c in name.chars() {
        if c.is_ascii_uppercase() && last_lower {
            spaced.push('_');
        }
        last_lower = c.is_ascii_lowercase();
        spaced.push(c);
    }
    spaced.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.trim_end_matches(|c: char| c.is_ascii_digit()).to_ascii_lowercase()).collect()
}

/// How a hammer item is treated, by its name.
pub fn classify(name: &str) -> Class {
    if SKIP_PREFIXES.iter().any(|p| name.starts_with(p)) {
        return Class::Skip;
    }
    let wooden = words(name).iter().any(|w| WOOD_WORDS.contains(&w.as_str()) || (w.len() > 4 && w.ends_with("wood")));
    if wooden { Class::Wood } else { Class::Normal }
}

/// Item numbers of everything that is placed with the build hammer, with how each is treated.
pub fn parse_build_items(text: &str) -> HashMap<u32, Class> {
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap_or("").split(',').collect();
    let id_at = header.iter().position(|h| h.trim() == "item_id").unwrap_or(0);
    let slot_at = header.iter().position(|h| h.trim() == "slot").unwrap_or(7);
    let name_at = header.iter().position(|h| h.trim() == "debug_name").unwrap_or(2);
    let mut out = HashMap::new();
    for line in lines {
        let cells: Vec<&str> = line.split(',').collect();
        if cells.get(slot_at).map(|s| s.trim()) != Some("BuildTool") {
            continue;
        }
        if let Some(id) = cells.get(id_at).and_then(|c| c.trim().parse::<u32>().ok()) {
            if id != 0 {
                out.insert(id, classify(cells.get(name_at).map(|n| n.trim()).unwrap_or("")));
            }
        }
    }
    out
}

/// Templates of the object list that can be broken at all.
pub fn parse_catalog(text: &str) -> HashSet<[u8; 16]> {
    let mut out = HashSet::new();
    for line in text.lines().skip(1) {
        let mut parts = line.splitn(3, ',');
        let (Some(_), Some(guid), Some(components)) = (parts.next(), parts.next(), parts.next()) else { continue };
        if components.trim().split('|').any(|c| c == "Destructible") {
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

fn entry(entries: &[u8], hash: u32, least: usize) -> Option<usize> {
    entries.chunks_exact(ENTRY).find(|e| u32_at(e, 0) == hash && u64_at(e, 16) as usize >= least).map(|e| u64_at(e, 8) as usize)
}

fn describe(entries: &[u8]) -> String {
    entries.chunks_exact(ENTRY).map(|e| format!("{:08x}:{}", u32_at(e, 0), u64_at(e, 16))).collect::<Vec<_>>().join(" ")
}

fn meters(b: &[u8]) -> Option<(f64, f64)> {
    if b.len() < 24 {
        return None;
    }
    let (x, z) = (u64_at(b, 0) as i64 as f64 / METERS, u64_at(b, 16) as i64 as f64 / METERS);
    (x.abs() < 40_000.0 && z.abs() < 40_000.0 && (x != 0.0 || z != 0.0)).then_some((x, z))
}

/// The slot number the game gave a component. The game keeps two tables of them: one for the components a
/// creation request carries (`+0x88`) and one for the components an entity template carries (`+0x30`);
/// `Destructible` is only in the second one.
pub fn component_index(manager: usize, hash: u32) -> Option<usize> {
    let world = u64_at(&read_bytes(manager, 8)?, 0) as usize;
    if world < 0x10000 {
        return None;
    }
    TYPE_MAPS.iter().find_map(|at| index_in(world + at, hash))
}

/// Whether a creation request can carry this component: only the request table (`+0x88`) says so.
fn request_has(manager: usize, hash: u32) -> bool {
    let Some(world) = read_bytes(manager, 8).map(|b| u64_at(&b, 0) as usize).filter(|w| *w >= 0x10000) else { return false };
    index_in(world + TYPE_MAPS[0], hash).is_some()
}

fn index_in(map: usize, hash: u32) -> Option<usize> {
    let head = read_bytes(map, 0x58)?;
    if head.len() < 0x58 {
        return None;
    }
    let capacity = u64_at(&head, 0x10) as usize;
    if u32_at(&head, 0x54) == 0 || capacity == 0 || capacity > (1 << 16) || capacity & (capacity - 1) != 0 {
        return None;
    }
    let (bitmap, keys, values) = (u64_at(&head, 8) as usize, u64_at(&head, 0x28) as usize, u64_at(&head, 0x40) as usize);
    let mut slot = hash as usize & (capacity - 1);
    for _ in 0..capacity {
        let word = read_bytes(bitmap + (slot >> 6) * 8, 8)?;
        if (u64_at(&word, 0) >> (slot & 63)) & 1 == 0 {
            return None;
        }
        if u32_at(&read_bytes(keys + slot * 4, 4)?, 0) == hash {
            let value = read_bytes(values + slot * 2, 2)?;
            let index = u16::from_le_bytes([value[0], *value.get(1)?]) as usize;
            return (index < spawner::COMPONENT_SLOTS).then_some(index);
        }
        slot = if slot + 1 < capacity { slot + 1 } else { 0 };
    }
    None
}

fn find_slots(manager: usize) -> Option<Slots> {
    let slots = Slots {
        transform: component_index(manager, HASH_CURRENT_TRANSFORM)?,
        destructible: component_index(manager, HASH_DESTRUCTIBLE)?,
        used_item: component_index(manager, HASH_USED_ITEM)?,
        health: component_index(manager, HASH_HEALTH),
        disable: component_index(manager, HASH_DISABLE_DESTRUCTIBLE),
        combat: component_index(manager, HASH_COMBAT_FLAGS),
    };
    let distinct = slots.transform != slots.destructible && slots.transform != slots.used_item && slots.destructible != slots.used_item;
    distinct.then_some(slots)
}

fn slots(manager: usize) -> Option<Slots> {
    let world = read_bytes(manager, 8).map(|b| u64_at(&b, 0) as usize).unwrap_or(0);
    let mut cached = SLOTS.lock().ok()?;
    if let Some((m, w, found)) = *cached {
        if m == manager && w == world && (found.is_some() || SLOT_TRIES.fetch_add(1, Ordering::Relaxed) % SLOT_RETRY_EVERY != 0) {
            return found;
        }
    }
    let found = find_slots(manager);
    match found {
        Some(s) => log(&format!(
            "object shield: component slots: position {}, destructible {}, used item {}, health {:?}, DisableDestructible {:?}, CombatFlags {:?}",
            s.transform, s.destructible, s.used_item, s.health, s.disable, s.combat
        )),
        None => {
            if SAY_TABLE.take() {
                log(&format!(
                    "object shield: the game's component table has no entry yet (position {:?}, destructible {:?}, used item {:?}); trying again at a later creation",
                    component_index(manager, HASH_CURRENT_TRANSFORM),
                    component_index(manager, HASH_DESTRUCTIBLE),
                    component_index(manager, HASH_USED_ITEM)
                ));
            }
        }
    }
    *cached = Some((manager, world, found));
    found
}

fn item_class(item: u32) -> Option<Class> {
    ITEMS.lock().ok().and_then(|i| i.as_ref().and_then(|map| map.get(&item).copied()))
}

/// False only when the object list says the template cannot be broken.
fn may_break(template: &[u8]) -> bool {
    let Ok(catalog) = CATALOG.lock() else { return true };
    let Some(set) = catalog.as_ref() else { return true };
    let mut key = [0u8; 16];
    key.copy_from_slice(template);
    set.contains(&key)
}

fn health_of(p: &spawner::Placement, slots: &Slots) -> Option<(usize, [u8; HEALTH_SIZE])> {
    let (addr, size) = spawner::component_address(p, slots.health?)?;
    if size != HEALTH_SIZE {
        return None;
    }
    let bytes = read_bytes(addr, HEALTH_SIZE).filter(|b| b.len() == HEALTH_SIZE)?;
    let mut out = [0u8; HEALTH_SIZE];
    out.copy_from_slice(&bytes);
    (u32_at(&out, 0) == spawner::HEALTH_SIGNATURE).then_some((addr, out))
}

/// Whether the entity carries `DisableDestructible` and whether its `CombatFlags` say immune.
fn protection(p: &spawner::Placement, slots: &Slots) -> (bool, bool) {
    let disable = slots.disable.is_some_and(|index| spawner::component_address(p, index).is_some());
    let immune = slots
        .combat
        .and_then(|index| spawner::component_address(p, index))
        .filter(|(_, size)| *size == COMBAT_FLAGS_SIZE)
        .and_then(|(addr, _)| read_bytes(addr, 1))
        .is_some_and(|b| b.first() == Some(&1));
    (disable, immune)
}

fn immune_set<R>(f: impl FnOnce(&mut HashSet<u32>) -> R) -> Option<R> {
    let mut guard = IMMUNE_SET.lock().ok()?;
    Some(f(guard.get_or_insert_with(HashSet::new)))
}

/// Puts the `CombatFlags` of an entity this module made immune back to not immune.
fn take_immunity_off(p: &spawner::Placement, slots: &Slots, id: u32) -> bool {
    if !immune_set(|set| set.remove(&id)).unwrap_or(false) {
        return false;
    }
    let Some((addr, size)) = slots.combat.and_then(|index| spawner::component_address(p, index)) else { return false };
    size == COMBAT_FLAGS_SIZE && spawner::write_bytes(addr, &[0])
}

/// An object that already stands and has a `CombatFlags` of its own can be made immune where it is.
fn make_immune_in_place(p: &spawner::Placement, slots: &Slots, id: u32) -> bool {
    let Some((addr, size)) = slots.combat.and_then(|index| spawner::component_address(p, index)) else { return false };
    if size != COMBAT_FLAGS_SIZE || !spawner::write_bytes(addr, &[1]) {
        return false;
    }
    immune_set(|set| {
        if set.len() < IMMUNE_MAX {
            set.insert(id);
        }
    });
    true
}

/// Looks at one built object inside a protected or released place. Numbers an earlier version multiplied are
/// divided back, immunity this module gave is taken off in a released place, and inside a protected zone the
/// object is reported as protected or not.
pub fn settle(manager: usize, id: u32, slots: &Slots, areas: &Areas) -> Outcome {
    let Some(p) = spawner::find_record(manager, id).and_then(spawner::placement) else { return Outcome::Gone };
    let Some((item_at, item_size)) = spawner::component_address(&p, slots.used_item) else { return Outcome::NotBuilt };
    if item_size != USED_ITEM_SIZE {
        return Outcome::Failed(format!("the used-item component is {item_size} bytes, expected {USED_ITEM_SIZE}"));
    }
    let Some(item) = read_bytes(item_at, 4).map(|b| u32_at(&b, 0)) else { return Outcome::Gone };
    let Some(class) = item_class(item) else { return Outcome::OtherItem(item) };
    let at = spawner::component_address(&p, slots.transform)
        .filter(|(_, size)| *size == TRANSFORM_SIZE)
        .and_then(|(addr, _)| read_bytes(addr, 24))
        .and_then(|b| meters(&b));
    let Some((x, z)) = at else { return Outcome::NoPosition(item) };
    let (zone, protecting) = match decide(areas, x, z) {
        Decision::Leave => return Outcome::Outside,
        Decision::Shield(zone) => (zone, true),
        Decision::Release(zone) => (zone, false),
    };
    let own = spawner::component_address(&p, slots.destructible);
    let shared = own.is_some_and(|(_, size)| size != DESTRUCTIBLE_SIZE);
    let threshold = own.filter(|(_, size)| *size == DESTRUCTIBLE_SIZE);
    let health = health_of(&p, slots);
    let mut edit = Edit::default();
    if let Some((addr, _)) = threshold {
        let Some(from) = read_bytes(addr, 2).filter(|b| b.len() == 2).map(|b| u16::from_le_bytes([b[0], b[1]])) else { return Outcome::Gone };
        if let Some(to) = original_value(from) {
            if !spawner::write_bytes(addr, &to.to_le_bytes()) {
                return Outcome::Failed(format!("item {item}: the write of the break threshold at {addr:#x} was refused"));
            }
            edit.threshold = Some((from, to));
        }
    }
    if let Some((addr, bytes)) = health {
        let (current, max) = (u32_at(&bytes, HEALTH_CURRENT_AT), u32_at(&bytes, HEALTH_MAX_AT));
        if let Some((now, top)) = released_health(current, max) {
            let mut span = [0u8; HEALTH_SIZE - HEALTH_CURRENT_AT];
            span.copy_from_slice(&bytes[HEALTH_CURRENT_AT..]);
            span[0..4].copy_from_slice(&now.to_le_bytes());
            span[HEALTH_MAX_AT - HEALTH_CURRENT_AT..].copy_from_slice(&top.to_le_bytes());
            if !spawner::write_bytes(addr + HEALTH_CURRENT_AT, &span) {
                return Outcome::Failed(format!("item {item}: the write of the health at {addr:#x} was refused"));
            }
            edit.health_max = Some((max, top));
            edit.health_current = Some((current, now));
        }
    }
    if !protecting {
        edit.immune_off = take_immunity_off(&p, slots, id);
    }
    if !edit.is_empty() {
        return Outcome::Released { zone, item, edit };
    }
    if !protecting || class == Class::Skip {
        return Outcome::Outside;
    }
    if own.is_none() && health.is_none() {
        return Outcome::NoDestructible(item);
    }
    match protection(&p, slots) {
        (false, false) if IMMUNE.load(Ordering::Relaxed) && make_immune_in_place(&p, slots, id) => Outcome::Protected { zone, item, disable: false, immune: true },
        (false, false) => Outcome::Unprotected { zone, item, shared },
        (disable, immune) => Outcome::Protected { zone, item, disable, immune },
    }
}

fn report(_manager: usize, id: u32, outcome: &Outcome, tally: Option<&mut Tally>) {
    if let Some(t) = tally {
        t.checked += 1;
        match outcome {
            Outcome::Protected { .. } => t.protected += 1,
            Outcome::Unprotected { shared, .. } => {
                t.unprotected += 1;
                t.shared += *shared as u32;
            }
            Outcome::Released { .. } => t.released += 1,
            Outcome::NoDestructible(_) => t.nothing_to_break += 1,
            _ => {}
        }
    }
    match outcome {
        Outcome::Protected { zone, item, disable, immune } => {
            PROTECTED_NOW.fetch_add(1, Ordering::Relaxed);
            if SAY_PROTECTED.take() {
                let how = match (disable, immune) {
                    (true, true) => "cannot be destroyed, immune to hits",
                    (true, false) => "cannot be destroyed",
                    _ => "immune to hits",
                };
                log(&format!("object shield: entity {id} (item {item}) protected in '{zone}': {how}"));
            }
        }
        Outcome::Unprotected { zone, item, shared } => {
            UNPROTECTED_NOW.fetch_add(1, Ordering::Relaxed);
            if SAY_UNPROTECTED.take() {
                let why = if *shared {
                    "its break threshold is shared by every object of its kind, so only creating it again can protect it"
                } else {
                    "it was created before the zone protected it, so only creating it again can protect it"
                };
                log(&format!("object shield: entity {id} (item {item}) in '{zone}' is not protected: {why}"));
            }
        }
        Outcome::Released { zone, item, edit } => {
            if SAY_RELEASED.take() {
                log(&format!("object shield: entity {id} (item {item}) in '{zone}' back to normal: {}", edit.describe()));
            }
        }
        Outcome::NoDestructible(item) => {
            if SAY_NO_DESTRUCTIBLE.take() {
                log(&format!("object shield: entity {id} (item {item}) has nothing that hits can break"));
            }
        }
        Outcome::OtherItem(item) => {
            if SAY_OTHER_ITEM.take() {
                log(&format!("object shield: entity {id} carries item {item}, which is not placed with the hammer; left as it is"));
            }
        }
        Outcome::NoPosition(item) => {
            if SAY_NO_POSITION.take() {
                log(&format!("object shield: entity {id} (item {item}) has no position of its own (a tool or item lying around); left as it is"));
            }
        }
        Outcome::Failed(why) => fail(&format!("entity {id} could not be switched: {why}")),
        Outcome::Gone | Outcome::NotBuilt | Outcome::Outside => {}
    }
}

fn start_sweep(manager: usize, work: &mut Work) {
    let mut ids: Vec<u32> = spawner::entity_ids(manager).map(|s| s.into_iter().collect()).unwrap_or_default();
    ids.sort_unstable();
    log(&format!("object shield: checking the {} entities that are loaded", ids.len()));
    work.sweep = Some(Sweep { ids, next: 0, tally: Tally::default() });
}

/// The bounded share of the pending work done per creation: objects that loaded a moment ago, and the
/// one pass over what is already standing after the zones changed.
fn work(manager: usize) {
    if !SWEEP_WANTED.load(Ordering::SeqCst) && !HAS_WORK.load(Ordering::Relaxed) {
        return;
    }
    let Some(slots) = slots(manager) else { return };
    let wanted = SWEEP_WANTED.swap(false, Ordering::SeqCst);
    let Ok(mut work) = WORK.lock() else { return };
    if wanted {
        start_sweep(manager, &mut work);
    }
    let Ok(areas) = AREAS.lock().map(|a| a.clone()) else { return };
    for _ in 0..WAITING_PER_EVENT {
        match work.waiting.front() {
            Some((_, since)) if since.elapsed() >= SETTLE => {}
            _ => break,
        }
        let Some((id, _)) = work.waiting.pop_front() else { break };
        let outcome = settle(manager, id, &slots, &areas);
        report(manager, id, &outcome, None);
    }
    let mut done = false;
    if let Some(sweep) = work.sweep.as_mut() {
        let end = (sweep.next + SWEEP_PER_EVENT).min(sweep.ids.len());
        for i in sweep.next..end {
            let id = sweep.ids[i];
            let outcome = settle(manager, id, &slots, &areas);
            report(manager, id, &outcome, Some(&mut sweep.tally));
        }
        sweep.next = end;
        done = end >= sweep.ids.len();
        if done {
            let t = &sweep.tally;
            log(&format!(
                "object shield: checked {} loaded entities: {} protected, {} not protected yet ({} of them share their break threshold), {} back to normal, {} built objects have nothing to break",
                t.checked, t.protected, t.unprotected, t.shared, t.released, t.nothing_to_break
            ));
        }
    }
    if done {
        work.sweep = None;
    }
    HAS_WORK.store(!work.waiting.is_empty() || work.sweep.is_some(), Ordering::SeqCst);
}

fn wait_for(id: u32) {
    let Ok(mut work) = WORK.lock() else { return };
    if work.waiting.len() >= WAITING_MAX {
        work.waiting.clear();
        SWEEP_WANTED.store(true, Ordering::SeqCst);
        return;
    }
    work.waiting.push_back((id, Instant::now()));
    HAS_WORK.store(true, Ordering::SeqCst);
}

fn read_table<C>(dir: &std::path::Path, file: &str, parse: fn(&str) -> C, empty: fn(&C) -> bool) -> Option<C> {
    std::fs::read_to_string(dir.join("export").join(file)).ok().map(|t| parse(&t)).filter(|c| !empty(c))
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
    let first = !LOADED.swap(true, Ordering::SeqCst);
    let stored: Option<Areas> = first.then(|| std::fs::read_to_string(dir.join(STATE_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok())).flatten();
    let options: Options = std::fs::read_to_string(dir.join(OPTIONS_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let immune_changed = IMMUNE.swap(options.immune, Ordering::SeqCst) != options.immune || DISABLE.swap(options.disable, Ordering::SeqCst) != options.disable;

    let (changed, active, next) = {
        let Ok(mut areas) = AREAS.lock() else { return };
        if let Some(stored) = stored {
            *areas = stored;
        }
        let next = next_areas(&areas, shielded);
        let changed = next != *areas;
        *areas = next.clone();
        (changed, !next.shielded.is_empty() || !next.released.is_empty(), next)
    };
    if changed || immune_changed {
        let names: Vec<&str> = next.shielded.iter().map(|a| a.name.as_str()).collect();
        log(&format!(
            "object shield: {} zone(s) protect what is built inside {:?}, {} place(s) released; new objects get DisableDestructible: {}, immunity to hits: {}",
            next.shielded.len(),
            names,
            next.released.len(),
            options.disable,
            options.immune
        ));
    }
    if changed {
        save(dir, &next);
    }
    if active {
        if ITEMS.lock().is_ok_and(|i| i.is_none()) {
            match read_table(dir, ITEMS_FILE, parse_build_items, |m| m.is_empty()) {
                Some(set) => {
                    let skipped = set.values().filter(|c| **c == Class::Skip).count();
                    log(&format!("object shield: {} items are placed with the hammer, {skipped} of them never protected (creatures, tools)", set.len()));
                    ITEMS_MISSING.store(false, Ordering::SeqCst);
                    if let Ok(mut items) = ITEMS.lock() {
                        *items = Some(set);
                    }
                }
                None => {
                    if !ITEMS_MISSING.swap(true, Ordering::SeqCst) {
                        log("object shield: ERROR export/m182_items.csv cannot be read or lists no hammer items; nothing is shielded until the mod m182_spawn_variants has written it");
                    }
                }
            }
        }
        if CATALOG.lock().is_ok_and(|c| c.is_none()) {
            if let Some(set) = read_table(dir, CATALOG_FILE, parse_catalog, |s| s.is_empty()) {
                if let Ok(mut catalog) = CATALOG.lock() {
                    *catalog = Some(set);
                }
            }
        }
    }
    let ready = active && ITEMS.lock().is_ok_and(|i| i.is_some());
    if ready && (changed || first) {
        SWEEP_WANTED.store(true, Ordering::SeqCst);
    }
    ACTIVE.store(ready, Ordering::SeqCst);
}

/// What `before` learned about the entity the game is about to create.
pub struct Seen {
    carried: bool,
    built: Option<u32>,
    breakable: bool,
    immune_added: bool,
    desc: usize,
    original: [u8; 16],
    entries: Vec<u8>,
}

/// Reads the zones once, when the game creates its first entity, so that what loads with the world is already
/// looked at before the first player joins.
fn early() {
    if LOADED.load(Ordering::SeqCst) || EARLY.swap(true, Ordering::SeqCst) {
        return;
    }
    let Ok(dir) = std::env::current_dir() else { return };
    let zones = std::fs::read_to_string(dir.join(super::groups::ZONES_FILE)).map(|t| super::groups::parse_zones(&t)).unwrap_or_default();
    set_zones(&dir, &zones);
    SWEEP_WANTED.store(false, Ordering::SeqCst);
}

/// The components a protected object is created with, when the creation request can carry them.
fn protection_entries(manager: usize, entries: &[u8], count: usize) -> Vec<(u32, usize, usize)> {
    let mut extra = Vec::new();
    let wanted = [
        (DISABLE.load(Ordering::Relaxed), HASH_DISABLE_DESTRUCTIBLE, DISABLE_DESTRUCTIBLE.as_ptr() as usize, DISABLE_DESTRUCTIBLE.len()),
        (IMMUNE.load(Ordering::Relaxed), HASH_COMBAT_FLAGS, COMBAT_FLAGS_IMMUNE.as_ptr() as usize, COMBAT_FLAGS_IMMUNE.len()),
    ];
    for (on, hash, data, size) in wanted {
        if !on || entry(entries, hash, 0).is_some() || count + extra.len() >= MAX_ENTRIES {
            continue;
        }
        if request_has(manager, hash) {
            extra.push((hash, data, size));
        } else if SAY_REFUSED.take() {
            log(&format!("object shield: ERROR the game does not list component {hash:#010x} among those a new object can be created with; new objects cannot get it"));
        }
    }
    extra
}

/// Called before the game creates an entity from `desc`.
pub fn before(manager: usize, kind: u32, desc: usize) -> Option<Seen> {
    if !LOADED.load(Ordering::Relaxed) && desc >= 0x10000 && spawner::is_server(manager) {
        early();
    }
    if !ACTIVE.load(Ordering::Relaxed) || desc < 0x10000 || !spawner::is_server(manager) {
        return None;
    }
    work(manager);
    let head = read_bytes(desc, 0x28)?;
    if head.len() < 0x28 {
        return None;
    }
    let array = u64_at(&head, 0x18) as usize;
    let count = u64_at(&head, 0x20) as usize;
    if count > MAX_ENTRIES {
        return None;
    }
    let entries = if count == 0 { Vec::new() } else { read_bytes(array, count * ENTRY).filter(|e| e.len() == count * ENTRY)? };
    let carried = entry(&entries, HASH_USED_ITEM, USED_ITEM_SIZE).and_then(|at| read_bytes(at, 4)).map(|b| u32_at(&b, 0));
    let mut seen = Seen { carried: carried.is_some(), built: carried.filter(|i| item_class(*i).is_some()), breakable: may_break(&head[4..20]), immune_added: false, desc, original: [0; 16], entries: Vec::new() };
    let Some(item) = seen.built else {
        if seen.breakable && !seen.carried && SAY_LOADED.take() {
            log(&format!("object shield: created without a hammer item: template {} given [{}]", spawner::guid_text(&head[4..20]), describe(&entries)));
        }
        return Some(seen);
    };

    let at = entry(&entries, HASH_CURRENT_TRANSFORM, 24).and_then(|at| read_bytes(at, 24)).and_then(|b| meters(&b));
    let zone = at.and_then(|(x, z)| AREAS.lock().ok().and_then(|a| a.shielded.iter().find(|s| s.holds(x, z)).map(|s| s.name.clone())));
    let place = at.map(|(x, z)| format!("x={x:.1} z={z:.1}")).unwrap_or_else(|| "no position given".into());
    let template = spawner::guid_text(&head[4..20]);
    match &zone {
        Some(zone) if SAY_PLACED.take() => log(&format!("object shield: created in '{zone}': template {template} item {item} (id {kind:#x}) at {place}; given [{}]", describe(&entries))),
        None if SAY_PLACED_OUTSIDE.take() => log(&format!("object shield: created outside every protected zone: template {template} item {item} at {place}; given [{}]", describe(&entries))),
        _ => {}
    }
    let protect = zone.is_some() && seen.breakable && item_class(item) != Some(Class::Skip);
    if protect {
        let extra = protection_entries(manager, &entries, count);
        if !extra.is_empty() {
            let mut list = entries.clone();
            for (hash, data, size) in &extra {
                list = with_entry(&list, *hash, *data, *size);
            }
            let mut patched = [0u8; 16];
            patched[..8].copy_from_slice(&(list.as_ptr() as u64).to_le_bytes());
            patched[8..].copy_from_slice(&((list.len() / ENTRY) as u64).to_le_bytes());
            if spawner::write_bytes(desc + 0x18, &patched) {
                seen.original.copy_from_slice(&head[0x18..0x28]);
                seen.entries = list;
                seen.immune_added = extra.iter().any(|(hash, _, _)| *hash == HASH_COMBAT_FLAGS);
            } else {
                fail(&format!("the component list of template {template} could not be extended"));
            }
        }
    }
    Some(seen)
}

/// Called right after the game created (or refused to create) the entity.
pub fn after(seen: Option<Seen>, manager: usize, entity: u32) {
    let Some(seen) = seen else { return };
    if !seen.entries.is_empty() && !spawner::write_bytes(seen.desc + 0x18, &seen.original) {
        fail("the game's component list could not be put back");
    }
    if entity == 0 {
        return;
    }
    if seen.immune_added {
        immune_set(|set| {
            if set.len() < IMMUNE_MAX {
                set.insert(entity);
            }
        });
    }
    if !seen.carried {
        if seen.breakable {
            wait_for(entity);
        }
        return;
    }
    if seen.built.is_none() {
        return;
    }
    let Some(slots) = slots(manager) else { return };
    let Ok(areas) = AREAS.lock().map(|a| a.clone()) else { return };
    let outcome = settle(manager, entity, &slots, &areas);
    report(manager, entity, &outcome, None);
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
    fn the_place_decides_between_shield_release_and_nothing() {
        let areas = Areas { shielded: vec![area("Town", 0.0, 0.0, 30.0)], released: vec![area("Old", 100.0, 0.0, 30.0)] };
        assert_eq!(decide(&areas, 10.0, 10.0), Decision::Shield("Town".into()));
        assert_eq!(decide(&areas, 110.0, 0.0), Decision::Release("Old".into()));
        assert_eq!(decide(&areas, 300.0, 0.0), Decision::Leave);
    }

    #[test]
    fn switching_the_flag_off_releases_the_place_and_on_again_forgets_that() {
        let on = next_areas(&Areas::default(), vec![area("Town", 0.0, 0.0, 30.0)]);
        assert!(on.released.is_empty());
        let off = next_areas(&on, Vec::new());
        assert_eq!(off.released, vec![area("Town", 0.0, 0.0, 30.0)]);
        assert_eq!(decide(&off, 1.0, 1.0), Decision::Release("Town".into()));
        let again = next_areas(&off, vec![area("Town", 0.0, 0.0, 30.0)]);
        assert!(again.released.is_empty());
        assert_eq!(decide(&again, 1.0, 1.0), Decision::Shield("Town".into()));
    }

    #[test]
    fn a_shielded_zone_wins_over_a_released_place_it_overlaps() {
        let areas = Areas { shielded: vec![area("New", 0.0, 0.0, 30.0)], released: vec![area("Old", 5.0, 0.0, 30.0)] };
        assert_eq!(decide(&areas, 4.0, 0.0), Decision::Shield("New".into()));
        assert_eq!(decide(&areas, 34.0, 0.0), Decision::Release("Old".into()));
    }

    #[test]
    fn moving_or_resizing_a_zone_releases_the_old_place() {
        let on = next_areas(&Areas::default(), vec![area("Town", 0.0, 0.0, 30.0)]);
        let moved = next_areas(&on, vec![area("Town", 0.0, 0.0, 50.0)]);
        assert_eq!(moved.released, vec![area("Town", 0.0, 0.0, 30.0)]);
        assert_eq!(decide(&moved, 1.0, 1.0), Decision::Shield("Town".into()));
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
    fn the_break_threshold_is_multiplied_once_and_comes_back_exactly() {
        for value in [1u16, 5, 10, 25, 40, 150, 200, 1000, SMALL_MAX] {
            let up = reinforced_value(value, false).unwrap();
            assert_eq!(up as u32, value as u32 * 40 + 39);
            assert!(is_reinforced(up));
            assert_eq!(reinforced_value(up, false), None);
            assert_eq!(reinforced_value(up, true), None);
            assert_eq!(original_value(up), Some(value));
            assert!(!is_reinforced(value), "{value} must not look reinforced");
        }
        for value in [1u16, 2, 5, 10, 25, 100, 250, 500, WOOD_SMALL_MAX] {
            let up = reinforced_value(value, true).unwrap();
            assert_eq!(up as u32, value as u32 * 120 + 117);
            assert_eq!(reinforced_value(up, true), None);
            assert_eq!(reinforced_value(up, false), None);
            assert_eq!(original_value(up), Some(value));
        }
        for (original, code) in LARGE {
            assert_eq!(reinforced_value(original, true), Some(code));
            assert_eq!(original_value(code), Some(original));
            assert!(!is_reinforced(original));
            assert_eq!(reinforced_value(code, true), None);
        }
        assert_eq!(reinforced_value(0, false), None);
        assert_eq!(reinforced_value(SMALL_MAX + 1, false), None);
        assert_eq!(reinforced_value(3000, true), None);
        assert_eq!(original_value(10), None);
        assert_eq!(original_value(39), None);
        assert_eq!(original_value(117), None);
    }

    #[test]
    fn the_two_forms_never_look_alike_and_no_game_value_looks_reinforced() {
        for v in 0..=u16::MAX {
            let normal = v as u32 % 40 == 39;
            let wood = v as u32 % 120 == 117;
            assert!(!(normal && wood), "{v}");
        }
        for value in [1u16, 2, 4, 5, 6, 7, 10, 12, 15, 16, 20, 25, 30, 40, 50, 60, 70, 75, 100, 150, 160, 200, 250, 300, 450, 500, 1000, 2000, 4000, 10000] {
            assert!(!is_reinforced(value), "{value}");
            assert!(reinforced_value(value, false).is_some(), "{value}");
        }
        for value in [1u16, 2, 5, 10, 12, 20, 25, 30, 40, 50, 75, 100, 150, 200, 250, 300, 500, 1000] {
            assert!(reinforced_value(value, true).is_some(), "{value}");
        }
    }

    #[test]
    fn health_is_multiplied_with_the_same_marker_and_restored() {
        assert_eq!(reinforced_health(100, 100, false), Some((4039, 4039)));
        assert_eq!(reinforced_health(50, 100, false), Some((2020, 4039)));
        assert_eq!(released_health(4039, 4039), Some((100, 100)));
        assert_eq!(released_health(2020, 4039), Some((50, 100)));
        assert_eq!(reinforced_health(100, 100, true), Some((12117, 12117)));
        assert_eq!(released_health(12117, 12117), Some((100, 100)));
        assert_eq!(reinforced_health(0, 100, false), Some((0, 4039)));
        assert_eq!(reinforced_health(1, 100, false), Some((40, 4039)));
        assert_eq!(reinforced_health(4039, 4039, false), None);
        assert_eq!(reinforced_health(12117, 12117, true), None);
        assert_eq!(reinforced_health(5, 0, false), None);
        assert_eq!(released_health(100, 100), None);
        assert_eq!(released_health(65518 - 7, 65518), Some((2000, 2000)));
        assert_eq!(released_health(160_039, 160_039), Some((4000, 4000)));
        assert_eq!(reinforced_health(100_000, 100_000, false), Some((4_000_039, 4_000_039)));
        assert_eq!(reinforced_health(100_000, 100_000, true), Some((12_000_117, 12_000_117)));
        assert_eq!(released_health(12_000_117, 12_000_117), Some((100_000, 100_000)));
        assert_eq!(reinforced_health(50672, 50672, false), Some((2_026_919, 2_026_919)));
        assert_eq!(reinforced_health(1, HEALTH_MAX_LIMIT + 1, false), None);
        assert_eq!(reinforced_health(HEALTH_MAX_LIMIT, HEALTH_MAX_LIMIT, true), None);
        let top = reinforced_health(30_000_000, 30_000_000, true).unwrap();
        assert_eq!(top.1 as u64, 30_000_000u64 * 120 + 117);
        assert_eq!(released_health(top.0, top.1), Some((30_000_000, 30_000_000)));
        let top = reinforced_health(HEALTH_MAX_LIMIT, HEALTH_MAX_LIMIT, false).unwrap();
        assert_eq!(top.1 as u64, HEALTH_MAX_LIMIT as u64 * 40 + 39);
    }

    #[test]
    fn damage_adds_up_so_a_reinforced_object_takes_many_more_hits() {
        let hits = |start: u32| (start + 6) / 7;
        assert_eq!(hits(25), 4);
        assert_eq!(hits(reinforced_value(25, false).unwrap() as u32), 149);
        assert_eq!(hits(reinforced_value(25, true).unwrap() as u32), 446);
        assert!(hits(reinforced_value(200, false).unwrap() as u32) >= 40 * hits(200) - 40);
    }

    #[test]
    fn items_are_sorted_into_wood_other_and_never_reinforced() {
        for name in [
            "Prop_Decoration_T1_Door_Carved_Wooden_Double",
            "Prop_Decoration_T6_Fence_Bamboo_01",
            "Prop_Decoration_T0_Fence_Palisade_08",
            "Prop_Decoration_T5_Fence_PineWood_Gate_01",
            "Prop_Decoration_T4_Shelf_Palmwood",
            "Prop_Decoration_T2_Barrel_Wooden03",
            "Base_Door_Secret_Wood01",
            "Prop_Decoration_T2_Fence_Plank_High_02",
            "Prop_Decoration_T3_Fence_HardWood_Corner_01",
            "Prop_Decoration_T0_Fence_Crude_Wooden1",
            "Prop_Storage_32_T2_Chest_Wood",
        ] {
            assert_eq!(classify(name), Class::Wood, "{name}");
        }
        for name in ["Prop_Logic_Lever", "Prop_Decoration_T7_AncientTemple_Shelf_Large01", "Prop_Decoration_Comfort3_T2_Table_Large", "Prop_Decoration_T2_StoneTotem_01B", "Prop_Production_T3_Oven", "Prop_Decoration_T5_Barrel_Steel", "Home_Base_FlameAltar"] {
            assert_eq!(classify(name), Class::Normal, "{name}");
        }
        for name in ["Animal_Farming_Capybara", "NPC_Townsfolk_Male_Villager_03", "Tool_T0_Pickaxe_Stone_Grassland", "Blueprint_Voxel_Stone", "Consumable_Toy_Ball"] {
            assert_eq!(classify(name), Class::Skip, "{name}");
        }
    }

    #[test]
    fn only_hammer_items_are_taken_from_the_item_list() {
        let text = "item_id,guid,debug_name,name,category,rarity,max_stack,slot,level_min\n\
            1001,a,Prop_Torch,Torch,BuildTools,Common,50,BuildTool,0\n\
            1002,b,Sword,Sword,Weapons,Common,1,MainHand,0\n\
            1003,c,Block,Block,Blueprints,Common,1,BuildTool,0\n\
            0,d,Broken,Broken,BuildTools,Common,1,BuildTool,0\n\
            x,e,Bad,Bad,BuildTools,Common,1,BuildTool,0\n";
        assert_eq!(parse_build_items(text), HashMap::from([(1001, Class::Normal), (1003, Class::Normal)]));
        let wooden = "item_id,guid,debug_name,slot\n5,a,Prop_Decoration_T0_Door_Wooden,BuildTool\n6,b,Animal_Pet_Cat,BuildTool\n";
        assert_eq!(parse_build_items(wooden), HashMap::from([(5, Class::Wood), (6, Class::Skip)]));
        let moved = "slot,item_id\nBuildTool,7\nHead,8\n";
        assert_eq!(parse_build_items(moved), HashMap::from([(7, Class::Normal)]));
        assert!(parse_build_items("").is_empty());
    }

    #[test]
    fn the_catalog_keeps_every_template_that_can_break() {
        let text = "name,guid,components\n\
            Torch,11111111-1111-1111-1111-111111111111,AttachToSurface|CurrentTransform|Destructible\n\
            Station,22222222-2222-2222-2222-222222222222,Crafting|CurrentTransform\n\
            Bed,33333333-3333-3333-3333-333333333333,CurrentTransform|Destructible|Health\n\
            Broken,not-a-guid,Destructible\n";
        let set = parse_catalog(text);
        assert_eq!(set.len(), 2);
        assert!(!set.contains(&super::super::config::parse_guid("22222222-2222-2222-2222-222222222222").unwrap()));
    }

    #[test]
    fn the_state_file_round_trips_and_tolerates_old_or_broken_content() {
        let areas = Areas { shielded: vec![area("Town", 1.0, 2.0, 30.0)], released: vec![area("Old", 5.0, 6.0, 10.0)] };
        let text = serde_json::to_string(&areas).unwrap();
        assert_eq!(serde_json::from_str::<Areas>(&text).unwrap(), areas);
        assert_eq!(serde_json::from_str::<Areas>("{}").unwrap(), Areas::default());
        assert_eq!(serde_json::from_str::<Options>(r#"{"immune":false}"#).unwrap(), Options { immune: false, disable: true });
        assert_eq!(serde_json::from_str::<Options>(r#"{"disable":false}"#).unwrap(), Options { immune: true, disable: false });
        assert_eq!(serde_json::from_str::<Options>("{}").unwrap(), Options { immune: true, disable: true });
    }

    const T: usize = 133;
    const D: usize = 301;
    const U: usize = 412;
    const H: usize = 77;
    const DIS: usize = 500;
    const CB: usize = 600;

    /// A stand-in for the game's memory: the component table, the entity table and one chunk per entity.
    struct World {
        manager: Box<[u8; 0x400]>,
        keep: Vec<Vec<u8>>,
        records: Vec<(u32, usize)>,
    }

    impl World {
        fn new(kind: u8) -> Self {
            let mut w = World { manager: Box::new([0; 0x400]), keep: Vec::new(), records: Vec::new() };
            w.manager[8] = kind;
            let world = w.block(vec![0u8; 0x100]);
            w.manager[..8].copy_from_slice(&(world as u64).to_le_bytes());
            for (map_at, table) in [(TYPE_MAPS[0], vec![(HASH_CURRENT_TRANSFORM, T), (HASH_USED_ITEM, U), (HASH_DISABLE_DESTRUCTIBLE, DIS), (HASH_COMBAT_FLAGS, CB)]), (TYPE_MAPS[1], vec![(HASH_DESTRUCTIBLE, D), (HASH_HEALTH, H)])] {
                let capacity = 16usize;
                let mut bitmap = vec![0u8; 8];
                let mut keys = vec![0u8; capacity * 4];
                let mut values = vec![0u8; capacity * 2];
                for (hash, index) in &table {
                    let mut slot = *hash as usize & (capacity - 1);
                    while bitmap[slot / 8] >> (slot % 8) & 1 == 1 {
                        slot = (slot + 1) % capacity;
                    }
                    bitmap[slot / 8] |= 1 << (slot % 8);
                    keys[slot * 4..slot * 4 + 4].copy_from_slice(&hash.to_le_bytes());
                    values[slot * 2..slot * 2 + 2].copy_from_slice(&(*index as u16).to_le_bytes());
                }
                let (bitmap, keys, values) = (w.block(bitmap), w.block(keys), w.block(values));
                let map = world + map_at;
                for (at, value) in [(8, bitmap as u64), (0x10, capacity as u64), (0x28, keys as u64), (0x40, values as u64)] {
                    spawner::write_bytes(map + at, &value.to_le_bytes());
                }
                spawner::write_bytes(map + 0x54, &(table.len() as u32).to_le_bytes());
            }
            w
        }

        fn at(&self) -> usize {
            self.manager.as_ptr() as usize
        }

        fn block(&mut self, bytes: Vec<u8>) -> usize {
            self.keep.push(bytes);
            self.keep.last().unwrap().as_ptr() as usize
        }

        /// Adds an entity with the given components and returns where its break threshold and its health sit.
        fn entity(&mut self, id: u32, at: Option<(f64, f64)>, item: Option<u32>, threshold: Option<u16>, health: Option<(u32, u32)>) -> (usize, usize) {
            self.entity_with(id, at, item, threshold, health, false, None)
        }

        #[allow(clippy::too_many_arguments)]
        fn entity_with(&mut self, id: u32, at: Option<(f64, f64)>, item: Option<u32>, threshold: Option<u16>, health: Option<(u32, u32)>, disable: bool, combat: Option<u8>) -> (usize, usize) {
            let mut def = vec![0u8; 0xf90];
            let mut chunk = vec![0u8; 0x200];
            let mut offset = 0x10usize;
            let mut threshold_at = 0;
            let mut put = |index: usize, bytes: &[u8], def: &mut Vec<u8>, chunk: &mut Vec<u8>| {
                def[index / 8] |= 1 << (index % 8);
                def[0x84 + index * 2..0x86 + index * 2].copy_from_slice(&(offset as u16).to_le_bytes());
                def[0xa84 + index * 2..0xa86 + index * 2].copy_from_slice(&(bytes.len() as u16).to_le_bytes());
                chunk[offset..offset + bytes.len()].copy_from_slice(bytes);
                offset += bytes.len() + 8;
                offset - bytes.len() - 8
            };
            if let Some((x, z)) = at {
                let mut t = [0u8; 56];
                t[..8].copy_from_slice(&((x * METERS) as i64).to_le_bytes());
                t[16..24].copy_from_slice(&((z * METERS) as i64).to_le_bytes());
                put(T, &t, &mut def, &mut chunk);
            }
            if let Some(item) = item {
                put(U, &item.to_le_bytes(), &mut def, &mut chunk);
            }
            if let Some(value) = threshold {
                threshold_at = put(D, &value.to_le_bytes(), &mut def, &mut chunk);
            }
            let mut health_at = 0;
            if let Some((current, max)) = health {
                let mut h = [0u8; 40];
                h[..4].copy_from_slice(&spawner::HEALTH_SIGNATURE.to_le_bytes());
                h[8..12].copy_from_slice(&current.to_le_bytes());
                h[36..40].copy_from_slice(&max.to_le_bytes());
                health_at = put(H, &h, &mut def, &mut chunk);
            }
            if disable {
                put(DIS, &[0u8], &mut def, &mut chunk);
            }
            if let Some(immune) = combat {
                let mut flags = COMBAT_FLAGS_IMMUNE;
                flags[0] = immune;
                put(CB, &flags, &mut def, &mut chunk);
            }
            let (def, chunk) = (self.block(def), self.block(chunk));
            let mut record = vec![0u8; 0x40];
            record[0x10..0x14].copy_from_slice(&id.to_le_bytes());
            record[0x18..0x20].copy_from_slice(&(def as u64).to_le_bytes());
            record[0x20..0x28].copy_from_slice(&(chunk as u64).to_le_bytes());
            let record = self.block(record);
            self.records.push((id, record));
            self.publish();
            (chunk + threshold_at, chunk + health_at)
        }

        fn publish(&mut self) {
            let capacity = 64usize;
            let mut bitmap = vec![0u8; 8];
            let mut keys = vec![0u8; capacity * 4];
            let mut values = vec![0u8; capacity * 8];
            for (id, record) in &self.records {
                let mut slot = spawner::hash_slot(*id, capacity as u64) as usize;
                while bitmap[slot / 8] >> (slot % 8) & 1 == 1 {
                    slot = (slot + 1) % capacity;
                }
                bitmap[slot / 8] |= 1 << (slot % 8);
                keys[slot * 4..slot * 4 + 4].copy_from_slice(&id.to_le_bytes());
                values[slot * 8..slot * 8 + 8].copy_from_slice(&(*record as u64).to_le_bytes());
            }
            let (bitmap, keys, values) = (self.block(bitmap), self.block(keys), self.block(values));
            let map = 0x148;
            self.manager[map + 8..map + 0x10].copy_from_slice(&(bitmap as u64).to_le_bytes());
            self.manager[map + 0x10..map + 0x18].copy_from_slice(&(capacity as u64).to_le_bytes());
            self.manager[map + 0x28..map + 0x30].copy_from_slice(&(keys as u64).to_le_bytes());
            self.manager[map + 0x40..map + 0x48].copy_from_slice(&(values as u64).to_le_bytes());
            self.manager[map + 0x54..map + 0x58].copy_from_slice(&(self.records.len() as u32).to_le_bytes());
        }
    }

    fn threshold(at: (usize, usize)) -> u16 {
        let b = read_bytes(at.0, 2).unwrap();
        u16::from_le_bytes([b[0], b[1]])
    }

    fn health(at: (usize, usize)) -> (u32, u32) {
        let b = read_bytes(at.1, 40).unwrap();
        (u32_at(&b, 8), u32_at(&b, 36))
    }

    fn released(zone: &str, threshold: Option<(u16, u16)>, health: Option<((u32, u32), (u32, u32))>) -> Outcome {
        Outcome::Released { zone: zone.into(), item: TORCH, edit: edit_of(threshold, health) }
    }

    fn edit_of(threshold: Option<(u16, u16)>, health: Option<((u32, u32), (u32, u32))>) -> Edit {
        Edit {
            threshold,
            health_max: health.map(|(_, max)| max),
            health_current: health.map(|(current, _)| current),
            immune_off: false,
        }
    }

    const TORCH: u32 = 1001;
    const SWORD: u32 = 2002;
    const DOOR: u32 = 1002;
    const LADDER: u32 = 1004;
    const PET: u32 = 1003;

    fn slots_for_tests() -> Slots {
        Slots { transform: T, destructible: D, used_item: U, health: Some(H), disable: Some(DIS), combat: Some(CB) }
    }

    fn descriptor(template: &[u8; 16], entries: &[u8]) -> Box<[u8; 0x30]> {
        let mut d = Box::new([0u8; 0x30]);
        d[4..20].copy_from_slice(template);
        d[0x18..0x20].copy_from_slice(&(entries.as_ptr() as u64).to_le_bytes());
        d[0x20..0x28].copy_from_slice(&((entries.len() / ENTRY) as u64).to_le_bytes());
        d
    }

    fn transform(x: f64, z: f64) -> Box<[u8; 56]> {
        let mut t = Box::new([0u8; 56]);
        t[..8].copy_from_slice(&((x * METERS) as i64).to_le_bytes());
        t[16..24].copy_from_slice(&((z * METERS) as i64).to_le_bytes());
        t
    }

    fn protected(item: u32, disable: bool, immune: bool) -> Outcome {
        Outcome::Protected { zone: "Town".into(), item, disable, immune }
    }

    fn read_flag(at: usize) -> u8 {
        read_bytes(at, 1).unwrap()[0]
    }

    #[test]
    fn the_whole_switch_against_a_stand_in_for_the_game() {
        *ITEMS.lock().unwrap() = Some(HashMap::from([(TORCH, Class::Normal), (DOOR, Class::Wood), (LADDER, Class::Wood), (PET, Class::Skip)]));
        *CATALOG.lock().unwrap() = None;
        *SLOTS.lock().unwrap() = None;
        *IMMUNE_SET.lock().unwrap() = None;
        let town = Areas { shielded: vec![area("Town", 100.0, 100.0, 30.0)], released: Vec::new() };
        let off = next_areas(&town, Vec::new());
        let slots = slots_for_tests();

        let mut w = World::new(0);
        assert_eq!(index_in(u64_at(&read_bytes(w.at(), 8).unwrap(), 0) as usize + TYPE_MAPS[0], HASH_DESTRUCTIBLE), None);
        assert_eq!(component_index(w.at(), HASH_DESTRUCTIBLE), Some(D));
        assert_eq!(component_index(w.at(), HASH_USED_ITEM), Some(U));
        assert_eq!(component_index(w.at(), 0x1234_5678), None);
        assert!(request_has(w.at(), HASH_DISABLE_DESTRUCTIBLE) && request_has(w.at(), HASH_COMBAT_FLAGS));
        assert!(!request_has(w.at(), HASH_DESTRUCTIBLE));
        assert_eq!(find_slots(w.at()), Some(slots));

        let torch = w.entity(10, Some((110.0, 95.0)), Some(TORCH), Some(10), None);
        let far = w.entity(11, Some((400.0, 95.0)), Some(TORCH), Some(10), None);
        let world_prop = w.entity(12, Some((101.0, 101.0)), None, Some(10), None);
        let impact = w.entity(13, Some((101.0, 101.0)), Some(SWORD), Some(10), None);
        w.entity(14, Some((101.0, 101.0)), Some(TORCH), None, None);
        w.entity(17, None, Some(TORCH), Some(10), None);
        w.entity_with(26, Some((100.0, 100.0)), Some(TORCH), Some(10), None, true, Some(1));
        w.entity_with(27, Some((100.0, 100.0)), Some(TORCH), Some(10), None, false, Some(1));
        w.entity_with(28, Some((100.0, 100.0)), Some(TORCH), Some(10), None, true, None);
        w.entity(29, Some((100.0, 100.0)), Some(TORCH), None, None);
        w.entity_with(32, Some((100.0, 100.0)), Some(TORCH), Some(10), None, false, Some(0));

        assert_eq!(settle(w.at(), 10, &slots, &town), Outcome::Unprotected { zone: "Town".into(), item: TORCH, shared: false });
        assert_eq!(threshold(torch), 10);
        assert_eq!(settle(w.at(), 11, &slots, &town), Outcome::Outside);
        assert_eq!(settle(w.at(), 12, &slots, &town), Outcome::NotBuilt);
        assert_eq!(settle(w.at(), 13, &slots, &town), Outcome::OtherItem(SWORD));
        assert_eq!(settle(w.at(), 14, &slots, &town), Outcome::NoDestructible(TORCH));
        assert_eq!(settle(w.at(), 17, &slots, &town), Outcome::NoPosition(TORCH));
        assert_eq!(settle(w.at(), 99, &slots, &town), Outcome::Gone);
        assert_eq!(settle(w.at(), 26, &slots, &town), protected(TORCH, true, true));
        assert_eq!(settle(w.at(), 27, &slots, &town), protected(TORCH, false, true));
        assert_eq!(settle(w.at(), 28, &slots, &town), protected(TORCH, true, false));
        let own_flags = {
            let p = spawner::find_record(w.at(), 32).and_then(spawner::placement).unwrap();
            spawner::component_address(&p, CB).unwrap().0
        };
        assert_eq!(read_flag(own_flags), 0);
        assert_eq!(settle(w.at(), 32, &slots, &town), protected(TORCH, false, true));
        assert_eq!(read_flag(own_flags), 1);
        assert_eq!(settle(w.at(), 32, &slots, &off), Outcome::Released { zone: "Town".into(), item: TORCH, edit: Edit { immune_off: true, ..Edit::default() } });
        assert_eq!(read_flag(own_flags), 0);
        assert_eq!((threshold(far), threshold(world_prop), threshold(impact)), (10, 10, 10));

        let mut z = World::new(0);
        let zero = {
            let mut def = vec![0u8; 0xf90];
            let mut chunk = vec![0u8; 0x200];
            let mut put = |index: usize, bytes: &[u8], offset: usize, stride: usize| {
                def[index / 8] |= 1 << (index % 8);
                def[0x84 + index * 2..0x86 + index * 2].copy_from_slice(&(offset as u16).to_le_bytes());
                def[0xa84 + index * 2..0xa86 + index * 2].copy_from_slice(&(stride as u16).to_le_bytes());
                chunk[offset..offset + bytes.len()].copy_from_slice(bytes);
            };
            let mut t = [0u8; 56];
            t[..8].copy_from_slice(&((100.0 * METERS) as i64).to_le_bytes());
            t[16..24].copy_from_slice(&((100.0 * METERS) as i64).to_le_bytes());
            put(T, &t, 0x10, 56);
            put(U, &TORCH.to_le_bytes(), 0x80, 4);
            put(D, &[0, 0], 0x90, 0);
            (z.block(def), z.block(chunk))
        };
        let mut record = vec![0u8; 0x40];
        record[0x10..0x14].copy_from_slice(&40u32.to_le_bytes());
        record[0x18..0x20].copy_from_slice(&(zero.0 as u64).to_le_bytes());
        record[0x20..0x28].copy_from_slice(&(zero.1 as u64).to_le_bytes());
        let record = z.block(record);
        z.records.push((40, record));
        z.publish();
        assert_eq!(settle(z.at(), 40, &slots, &town), Outcome::Unprotected { zone: "Town".into(), item: TORCH, shared: true });

        let bed = w.entity(15, Some((99.0, 101.0)), Some(TORCH), Some(1039), Some((1039, 1039)));
        let hurt = w.entity(19, Some((97.0, 97.0)), Some(TORCH), Some(8039), Some((2010, 8039)));
        let huge = w.entity(21, Some((96.0, 96.0)), Some(TORCH), Some(65438), None);
        let door = w.entity(22, Some((100.0, 99.0)), Some(DOOR), Some(12117), Some((12117, 12117)));
        let ladder = w.entity(23, Some((100.0, 98.0)), Some(LADDER), Some(65398), None);
        let pet = w.entity(24, Some((100.0, 97.0)), Some(PET), None, Some((22279, 22279)));
        let fresh_pet = w.entity(25, Some((100.0, 96.0)), Some(PET), None, Some((556, 556)));
        assert_eq!(settle(w.at(), 15, &slots, &town), Outcome::Released { zone: "Town".into(), item: TORCH, edit: edit_of(Some((1039, 25)), Some(((1039, 25), (1039, 25)))) });
        assert_eq!((threshold(bed), health(bed)), (25, (25, 25)));
        assert_eq!(settle(w.at(), 19, &slots, &town), Outcome::Released { zone: "Town".into(), item: TORCH, edit: edit_of(Some((8039, 200)), Some(((2010, 50), (8039, 200)))) });
        assert_eq!((threshold(hurt), health(hurt)), (200, (50, 200)));
        assert_eq!(settle(w.at(), 21, &slots, &town), Outcome::Released { zone: "Town".into(), item: TORCH, edit: edit_of(Some((65438, 10000)), None) });
        assert_eq!(threshold(huge), 10000);
        assert_eq!(settle(w.at(), 22, &slots, &town), Outcome::Released { zone: "Town".into(), item: DOOR, edit: edit_of(Some((12117, 100)), Some(((12117, 100), (12117, 100)))) });
        assert_eq!((threshold(door), health(door)), (100, (100, 100)));
        assert_eq!(settle(w.at(), 23, &slots, &town), Outcome::Released { zone: "Town".into(), item: LADDER, edit: edit_of(Some((65398, 1000)), None) });
        assert_eq!(threshold(ladder), 1000);
        assert_eq!(settle(w.at(), 24, &slots, &town), Outcome::Released { zone: "Town".into(), item: PET, edit: edit_of(None, Some(((22279, 556), (22279, 556)))) });
        assert_eq!(health(pet), (556, 556));
        assert_eq!(settle(w.at(), 25, &slots, &town), Outcome::Outside);
        assert_eq!(health(fresh_pet), (556, 556));
        assert_eq!(settle(w.at(), 10, &slots, &off), Outcome::Outside);

        *AREAS.lock().unwrap() = town.clone();
        LOADED.store(true, Ordering::SeqCst);
        IMMUNE.store(true, Ordering::SeqCst);
        DISABLE.store(true, Ordering::SeqCst);
        ACTIVE.store(true, Ordering::SeqCst);
        SWEEP_WANTED.store(true, Ordering::SeqCst);
        let guid = super::super::config::parse_guid("11111111-1111-1111-1111-111111111111").unwrap();
        let empty = descriptor(&guid, &[]);
        let seen = before(w.at(), 0, empty.as_ptr() as usize).expect("an object from the save is looked at");
        assert!(WORK.lock().unwrap().sweep.is_none());
        assert_eq!((threshold(bed), health(bed)), (25, (25, 25)));

        let item = TORCH.to_le_bytes();
        let inside = transform(110.0, 110.0);
        let entries = with_entry(&with_entry(&[], HASH_CURRENT_TRANSFORM, inside.as_ptr() as usize, 56), HASH_USED_ITEM, item.as_ptr() as usize, 4);
        let placed = descriptor(&guid, &entries);
        let placed_bytes = *placed;
        let seen2 = before(w.at(), 0x31, placed.as_ptr() as usize).unwrap();
        let head = read_bytes(placed.as_ptr() as usize, 0x28).unwrap();
        assert_eq!(u64_at(&head, 0x20), 4);
        let list = read_bytes(u64_at(&head, 0x18) as usize, 4 * ENTRY).unwrap();
        assert_eq!(&list[..2 * ENTRY], &entries[..]);
        assert_eq!((u32_at(&list, 2 * ENTRY), u64_at(&list, 2 * ENTRY + 16)), (HASH_DISABLE_DESTRUCTIBLE, 1));
        assert_eq!((u32_at(&list, 3 * ENTRY), u64_at(&list, 3 * ENTRY + 16)), (HASH_COMBAT_FLAGS, 12));
        assert_eq!(read_bytes(u64_at(&list, 3 * ENTRY + 8) as usize, 12).unwrap(), COMBAT_FLAGS_IMMUNE.to_vec());
        let fresh = w.entity_with(30, Some((110.0, 110.0)), Some(TORCH), Some(10), None, true, Some(1));
        let before_count = PROTECTED_NOW.load(Ordering::Relaxed);
        after(Some(seen2), w.at(), 30);
        assert_eq!(*placed, placed_bytes);
        assert_eq!(PROTECTED_NOW.load(Ordering::Relaxed), before_count + 1);
        assert_eq!(threshold(fresh), 10);
        assert!(IMMUNE_SET.lock().unwrap().as_ref().unwrap().contains(&30));

        IMMUNE.store(false, Ordering::SeqCst);
        let seen3 = before(w.at(), 0x32, placed.as_ptr() as usize).unwrap();
        let head = read_bytes(placed.as_ptr() as usize, 0x28).unwrap();
        assert_eq!(u64_at(&head, 0x20), 3);
        after(Some(seen3), w.at(), 0);
        assert_eq!(*placed, placed_bytes);
        IMMUNE.store(true, Ordering::SeqCst);
        DISABLE.store(false, Ordering::SeqCst);
        let seen4 = before(w.at(), 0x33, placed.as_ptr() as usize).unwrap();
        assert_eq!(u64_at(&read_bytes(placed.as_ptr() as usize, 0x28).unwrap(), 0x20), 3);
        after(Some(seen4), w.at(), 0);
        DISABLE.store(true, Ordering::SeqCst);

        let sword = SWORD.to_le_bytes();
        let hit = with_entry(&with_entry(&[], HASH_CURRENT_TRANSFORM, inside.as_ptr() as usize, 56), HASH_USED_ITEM, sword.as_ptr() as usize, 4);
        let hit_desc = descriptor(&guid, &hit);
        let seen5 = before(w.at(), 0x34, hit_desc.as_ptr() as usize).unwrap();
        assert_eq!(u64_at(&read_bytes(hit_desc.as_ptr() as usize, 0x28).unwrap(), 0x20), 2);
        after(Some(seen5), w.at(), 13);

        let pet_item = PET.to_le_bytes();
        let pet_entries = with_entry(&with_entry(&[], HASH_CURRENT_TRANSFORM, inside.as_ptr() as usize, 56), HASH_USED_ITEM, pet_item.as_ptr() as usize, 4);
        let pet_desc = descriptor(&guid, &pet_entries);
        let seen6 = before(w.at(), 0x35, pet_desc.as_ptr() as usize).unwrap();
        assert_eq!(u64_at(&read_bytes(pet_desc.as_ptr() as usize, 0x28).unwrap(), 0x20), 2);
        after(Some(seen6), w.at(), 0);

        let outside = transform(400.0, 400.0);
        let far_entries = with_entry(&with_entry(&[], HASH_CURRENT_TRANSFORM, outside.as_ptr() as usize, 56), HASH_USED_ITEM, item.as_ptr() as usize, 4);
        let far_desc = descriptor(&guid, &far_entries);
        let seen7 = before(w.at(), 0x36, far_desc.as_ptr() as usize).unwrap();
        assert_eq!(u64_at(&read_bytes(far_desc.as_ptr() as usize, 0x28).unwrap(), 0x20), 2);
        after(Some(seen7), w.at(), 0);

        let client = World::new(1);
        assert!(before(client.at(), 0, placed.as_ptr() as usize).is_none());

        let mut plain = World::new(0);
        let world = u64_at(&read_bytes(plain.at(), 8).unwrap(), 0) as usize;
        spawner::write_bytes(world + TYPE_MAPS[0] + 0x10, &0u64.to_le_bytes());
        assert!(!request_has(plain.at(), HASH_DISABLE_DESTRUCTIBLE));
        assert!(protection_entries(plain.at(), &[], 0).is_empty());
        plain.keep.clear();

        *AREAS.lock().unwrap() = off.clone();
        let released_flag = w.entity_with(31, Some((100.0, 100.0)), Some(TORCH), Some(10), None, true, Some(1));
        immune_set(|set| set.insert(31));
        let flag_at = |entity: usize| {
            let p = spawner::find_record(w.at(), entity as u32).and_then(spawner::placement).unwrap();
            spawner::component_address(&p, CB).unwrap().0
        };
        let at = flag_at(31);
        let _ = released_flag;
        assert_eq!(read_flag(at), 1);
        assert_eq!(settle(w.at(), 31, &slots, &off), Outcome::Released { zone: "Town".into(), item: TORCH, edit: Edit { immune_off: true, ..Edit::default() } });
        assert_eq!(read_flag(at), 0);
        assert_eq!(settle(w.at(), 31, &slots, &off), Outcome::Outside);
        let own = flag_at(27);
        assert_eq!(settle(w.at(), 27, &slots, &off), Outcome::Outside);
        assert_eq!(read_flag(own), 1);

        ACTIVE.store(false, Ordering::SeqCst);
        assert!(before(w.at(), 0, placed.as_ptr() as usize).is_none());
        *AREAS.lock().unwrap() = Areas::default();
        *ITEMS.lock().unwrap() = None;
        *SLOTS.lock().unwrap() = None;
        *IMMUNE_SET.lock().unwrap() = None;
        let _ = seen;
    }

    #[test]
    fn an_unknown_component_table_switches_nothing_and_is_tried_again_later() {
        *SLOTS.lock().unwrap() = None;
        let w = World::new(0);
        let world = u64_at(&read_bytes(w.at(), 8).unwrap(), 0) as usize;
        let capacity = read_bytes(world + TYPE_MAPS[1] + 0x10, 8).unwrap();
        spawner::write_bytes(world + TYPE_MAPS[1] + 0x10, &0u64.to_le_bytes());
        assert_eq!(component_index(w.at(), HASH_DESTRUCTIBLE), None);
        assert_eq!(find_slots(w.at()), None);
        assert_eq!(slots(w.at()), None);
        spawner::write_bytes(world + TYPE_MAPS[1] + 0x10, &capacity);
        let mut found = None;
        for _ in 0..SLOT_RETRY_EVERY {
            found = found.or(slots(w.at()));
        }
        assert_eq!(found, Some(slots_for_tests()));
        let nothing = [0u8; 16];
        assert_eq!(component_index(nothing.as_ptr() as usize, HASH_DESTRUCTIBLE), None);
        *SLOTS.lock().unwrap() = None;
    }
}

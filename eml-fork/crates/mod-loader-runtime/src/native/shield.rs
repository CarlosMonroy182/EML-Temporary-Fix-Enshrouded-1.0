//! Reinforced objects: inside a zone whose `shield` flag is on, what players built with the hammer takes
//! 40 times more to break. A hit takes `Destructible.maxHealth` points off an object that has no `Health`
//! yet and creates the `Health` it keeps afterwards, so damage adds up. The object's own numbers are
//! multiplied once (`Destructible.maxHealth`, and `Health` maximum and current where there is one) and
//! divided again when the zone stops reinforcing. A reinforced number always has the form `n * 40 + 39`
//! (or one of a few fixed codes for numbers too large for 16 bits), which no original number has, so it is
//! never applied twice and always undone exactly. Work happens only when the game creates an entity or the
//! zones change: no timers, no hook on damage.

use std::collections::{HashSet, VecDeque};
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
static COMBAT_FLAGS_IMMUNE: [u8; 12] = [1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
const TRANSFORM_SIZE: usize = 56;
const DESTRUCTIBLE_SIZE: usize = 2;
const USED_ITEM_SIZE: usize = 4;
const TYPE_MAPS: [usize; 2] = [0x88, 0x30];
const FACTOR: u32 = 40;
const MARK: u32 = FACTOR - 1;
const SMALL_MAX: u16 = ((u16::MAX as u32 - MARK) / FACTOR) as u16;
/// Originals too large to be multiplied inside 16 bits, and the fixed codes they are stored as.
const LARGE: [(u16, u16); 3] = [(2000, 65518), (4000, 65478), (10000, 65438)];
const HEALTH_SIZE: usize = 40;
const HEALTH_CURRENT_AT: usize = 8;
const HEALTH_MAX_AT: usize = 36;
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Options {
    /// Trial: objects built in a shielded zone are also created with `CombatFlags.immune`.
    pub immune: bool,
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Gone,
    NotBuilt,
    OtherItem(u32),
    NoDestructible(u32),
    NoPosition(u32),
    Outside,
    Already,
    Unusual(u32),
    Reinforced { zone: String, item: u32, edit: Edit },
    Released { zone: String, item: u32, edit: Edit },
    Failed,
}

/// What was written into one object.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Edit {
    pub threshold: Option<(u16, u16)>,
    pub health_max: Option<(u32, u32)>,
    pub health_current: Option<(u32, u32)>,
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
        parts.join(", ")
    }

    fn is_empty(&self) -> bool {
        self.threshold.is_none() && self.health_max.is_none()
    }
}

#[derive(Default)]
struct Tally {
    checked: u32,
    reinforced: u32,
    already: u32,
    released: u32,
    with_health: u32,
    nothing_to_break: u32,
    unusual: u32,
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
static IMMUNE: AtomicBool = AtomicBool::new(false);
static LOADED: AtomicBool = AtomicBool::new(false);
static SWEEP_WANTED: AtomicBool = AtomicBool::new(false);
static HAS_WORK: AtomicBool = AtomicBool::new(false);
static AREAS: Mutex<Areas> = Mutex::new(Areas { shielded: Vec::new(), released: Vec::new() });
static ITEMS: Mutex<Option<HashSet<u32>>> = Mutex::new(None);
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
static SAY_SHIELDED: Budget = Budget::new(12);
static SAY_ALREADY: Budget = Budget::new(3);
static SAY_RELEASED: Budget = Budget::new(6);
static SAY_HEALTH: Budget = Budget::new(6);
static SAY_NO_DESTRUCTIBLE: Budget = Budget::new(6);
static SAY_OTHER_ITEM: Budget = Budget::new(4);
static SAY_NO_POSITION: Budget = Budget::new(4);
static SAY_UNUSUAL: Budget = Budget::new(4);
static SAY_FAILED: Budget = Budget::new(6);
static SAY_LIMITED: Budget = Budget::new(3);
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
pub fn reinforced_value(value: u16) -> Option<u16> {
    if value == 0 || is_reinforced(value) {
        return None;
    }
    if value <= SMALL_MAX {
        return Some((value as u32 * FACTOR + MARK) as u16);
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
    (value as u32 % FACTOR == MARK && value as u32 > MARK).then(|| ((value as u32 - MARK) / FACTOR) as u16)
}

/// Health maximum and current after the object was reinforced. A full object stays full.
pub fn reinforced_health(current: u32, max: u32) -> Option<(u32, u32)> {
    if max == 0 || max > HEALTH_MAX_LIMIT || is_reinforced_max(max) {
        return None;
    }
    let to = max * FACTOR + MARK;
    Some((scaled(current, max, to), to))
}

fn is_reinforced_max(max: u32) -> bool {
    original_max(max).is_some()
}

fn original_max(max: u32) -> Option<u32> {
    if max <= u16::MAX as u32 {
        return original_value(max as u16).map(u32::from);
    }
    (max % FACTOR == MARK).then(|| (max - MARK) / FACTOR)
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

/// Item numbers of everything that is placed with the build hammer.
pub fn parse_build_items(text: &str) -> HashSet<u32> {
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap_or("").split(',').collect();
    let id_at = header.iter().position(|h| h.trim() == "item_id").unwrap_or(0);
    let slot_at = header.iter().position(|h| h.trim() == "slot").unwrap_or(7);
    let mut out = HashSet::new();
    for line in lines {
        let cells: Vec<&str> = line.split(',').collect();
        if cells.get(slot_at).map(|s| s.trim()) != Some("BuildTool") {
            continue;
        }
        if let Some(id) = cells.get(id_at).and_then(|c| c.trim().parse::<u32>().ok()) {
            if id != 0 {
                out.insert(id);
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
        Some(s) => log(&format!("object shield: component slots: position {}, destructible {}, used item {}, health {:?}", s.transform, s.destructible, s.used_item, s.health)),
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

fn is_build_item(item: u32) -> bool {
    ITEMS.lock().ok().is_some_and(|i| i.as_ref().is_some_and(|set| set.contains(&item)))
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

/// Looks at one entity and, when it is a built object inside a reinforced or released place, switches its
/// numbers: the break threshold and the health, in one write each.
pub fn settle(manager: usize, id: u32, slots: &Slots, areas: &Areas) -> Outcome {
    let Some(p) = spawner::find_record(manager, id).and_then(spawner::placement) else { return Outcome::Gone };
    let Some((item_at, item_size)) = spawner::component_address(&p, slots.used_item) else { return Outcome::NotBuilt };
    if item_size != USED_ITEM_SIZE {
        return Outcome::Failed;
    }
    let Some(item) = read_bytes(item_at, 4).map(|b| u32_at(&b, 0)) else { return Outcome::Gone };
    if !is_build_item(item) {
        return Outcome::OtherItem(item);
    }
    let at = spawner::component_address(&p, slots.transform)
        .filter(|(_, size)| *size == TRANSFORM_SIZE)
        .and_then(|(addr, _)| read_bytes(addr, 24))
        .and_then(|b| meters(&b));
    let Some((x, z)) = at else { return Outcome::NoPosition(item) };
    let decision = decide(areas, x, z);
    let (zone, release) = match decision {
        Decision::Leave => return Outcome::Outside,
        Decision::Shield(zone) => (zone, false),
        Decision::Release(zone) => (zone, true),
    };
    let threshold = spawner::component_address(&p, slots.destructible);
    if threshold.is_some_and(|(_, size)| size != DESTRUCTIBLE_SIZE) {
        return Outcome::Failed;
    }
    let health = health_of(&p, slots);
    if threshold.is_none() && health.is_none() {
        return Outcome::NoDestructible(item);
    }
    let mut edit = Edit::default();
    let mut already = false;
    let mut unusual = None;
    if let Some((addr, _)) = threshold {
        let Some(from) = read_bytes(addr, 2).filter(|b| b.len() == 2).map(|b| u16::from_le_bytes([b[0], b[1]])) else { return Outcome::Gone };
        let to = if release { original_value(from) } else { reinforced_value(from) };
        match to {
            Some(to) => {
                if !spawner::write_bytes(addr, &to.to_le_bytes()) {
                    return Outcome::Failed;
                }
                edit.threshold = Some((from, to));
            }
            None if release => {}
            None if is_reinforced(from) => already = true,
            None => unusual = Some(from as u32),
        }
    }
    if let Some((addr, bytes)) = health {
        let (current, max) = (u32_at(&bytes, HEALTH_CURRENT_AT), u32_at(&bytes, HEALTH_MAX_AT));
        let to = if release { released_health(current, max) } else { reinforced_health(current, max) };
        match to {
            Some((now, top)) => {
                let mut span = [0u8; HEALTH_SIZE - HEALTH_CURRENT_AT];
                span.copy_from_slice(&bytes[HEALTH_CURRENT_AT..]);
                span[0..4].copy_from_slice(&now.to_le_bytes());
                span[HEALTH_MAX_AT - HEALTH_CURRENT_AT..].copy_from_slice(&top.to_le_bytes());
                if !spawner::write_bytes(addr + HEALTH_CURRENT_AT, &span) {
                    return Outcome::Failed;
                }
                edit.health_max = Some((max, top));
                edit.health_current = Some((current, now));
            }
            None if release => {}
            None if is_reinforced_max(max) => already = true,
            None => unusual = Some(max),
        }
    }
    if !edit.is_empty() {
        return if release { Outcome::Released { zone, item, edit } } else { Outcome::Reinforced { zone, item, edit } };
    }
    match (already, unusual) {
        (true, _) => Outcome::Already,
        (false, Some(value)) => Outcome::Unusual(value),
        (false, None) => Outcome::Outside,
    }
}

fn report(manager: usize, id: u32, outcome: &Outcome, tally: Option<&mut Tally>) {
    if let Some(t) = tally {
        t.checked += 1;
        match outcome {
            Outcome::Reinforced { edit, .. } => {
                t.reinforced += 1;
                t.with_health += edit.health_max.is_some() as u32;
            }
            Outcome::Already => t.already += 1,
            Outcome::Released { .. } => t.released += 1,
            Outcome::NoDestructible(_) => t.nothing_to_break += 1,
            Outcome::Unusual(_) => t.unusual += 1,
            _ => {}
        }
    }
    match outcome {
        Outcome::Reinforced { zone, item, edit } => {
            if SAY_SHIELDED.take() {
                log(&format!("object shield: entity {id} (item {item}) in '{zone}' reinforced: {}", edit.describe()));
            }
            if edit.health_max.is_some() && SAY_HEALTH.take() {
                log(&format!("object shield: entity {id} (item {item}) also has health of its own; its maximum and current health were multiplied too"));
            }
            if edit.threshold.is_some_and(|(from, to)| from > SMALL_MAX && to != from) && SAY_LIMITED.take() {
                log(&format!("object shield: entity {id} has a break threshold of {}; 40 times that does not fit in 16 bits, so it got the largest value that still can be undone exactly", edit.threshold.map_or(0, |t| t.0)));
            }
        }
        Outcome::Already => {
            if SAY_ALREADY.take() {
                log(&format!("object shield: entity {id} was already reinforced"));
            }
        }
        Outcome::Released { zone, item, edit } => {
            if SAY_RELEASED.take() {
                log(&format!("object shield: entity {id} (item {item}) in released place '{zone}': back to normal, {}", edit.describe()));
            }
        }
        Outcome::NoDestructible(item) => {
            if SAY_NO_DESTRUCTIBLE.take() {
                let made = spawner::component_summary(manager, id).unwrap_or_else(|| "not readable".into());
                log(&format!("object shield: entity {id} (item {item}) is built inside a reinforced or released place and has nothing that hits can break; it has {made}"));
            }
        }
        Outcome::OtherItem(item) => {
            if SAY_OTHER_ITEM.take() {
                log(&format!("object shield: entity {id} carries item {item}, which is not placed with the hammer; left as it is"));
            }
        }
        Outcome::NoPosition(item) => {
            if SAY_NO_POSITION.take() {
                log(&format!("object shield: entity {id} (item {item}) is built but has no readable position; left as it is"));
            }
        }
        Outcome::Unusual(value) => {
            if SAY_UNUSUAL.take() {
                log(&format!("object shield: entity {id} has a break threshold or health maximum of {value} that cannot be multiplied exactly; left as it is"));
            }
        }
        Outcome::Failed => fail(&format!("entity {id} could not be switched (unexpected component size or write refused)")),
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
                "object shield: checked {} loaded entities: {} reinforced now, {} were reinforced already, {} released, {} of them have health of their own, {} built objects have nothing to break, {} left as they are",
                t.checked, t.reinforced, t.already, t.released, t.with_health, t.nothing_to_break, t.unusual
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

fn read_table<T>(dir: &std::path::Path, file: &str, parse: fn(&str) -> HashSet<T>) -> Option<HashSet<T>> {
    std::fs::read_to_string(dir.join("export").join(file)).ok().map(|t| parse(&t)).filter(|s| !s.is_empty())
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
    let immune_changed = IMMUNE.swap(options.immune, Ordering::SeqCst) != options.immune;

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
            "object shield: {} zone(s) shield what is built inside {:?}, {} place(s) released{}",
            next.shielded.len(),
            names,
            next.released.len(),
            if options.immune { ", trial: immune to hits" } else { "" }
        ));
    }
    if changed {
        save(dir, &next);
    }
    if active {
        if ITEMS.lock().is_ok_and(|i| i.is_none()) {
            match read_table(dir, ITEMS_FILE, parse_build_items) {
                Some(set) => {
                    log(&format!("object shield: {} items are placed with the hammer", set.len()));
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
            if let Some(set) = read_table(dir, CATALOG_FILE, parse_catalog) {
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
    desc: usize,
    original: [u8; 16],
    entries: Vec<u8>,
}

/// Called before the game creates an entity from `desc`.
pub fn before(manager: usize, kind: u32, desc: usize) -> Option<Seen> {
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
    let mut seen = Seen { carried: carried.is_some(), built: carried.filter(|i| is_build_item(*i)), breakable: may_break(&head[4..20]), desc, original: [0; 16], entries: Vec::new() };
    let Some(item) = seen.built else { return Some(seen) };

    let at = entry(&entries, HASH_CURRENT_TRANSFORM, 24).and_then(|at| read_bytes(at, 24)).and_then(|b| meters(&b));
    let zone = at.and_then(|(x, z)| AREAS.lock().ok().and_then(|a| a.shielded.iter().find(|s| s.holds(x, z)).map(|s| s.name.clone())));
    let place = at.map(|(x, z)| format!("x={x:.1} z={z:.1}")).unwrap_or_else(|| "no position given".into());
    let template = spawner::guid_text(&head[4..20]);
    match &zone {
        Some(zone) if SAY_PLACED.take() => log(&format!("object shield: placed in '{zone}': template {template} item {item} (id {kind:#x}) at {place}; given [{}]", describe(&entries))),
        None if SAY_PLACED_OUTSIDE.take() => log(&format!("object shield: placed outside every shielded zone: template {template} item {item} at {place}; given [{}]", describe(&entries))),
        _ => {}
    }
    if zone.is_some() && IMMUNE.load(Ordering::Relaxed) && entry(&entries, HASH_COMBAT_FLAGS, 0).is_none() && count < MAX_ENTRIES {
        let list = with_entry(&entries, HASH_COMBAT_FLAGS, COMBAT_FLAGS_IMMUNE.as_ptr() as usize, COMBAT_FLAGS_IMMUNE.len());
        let mut patched = [0u8; 16];
        patched[..8].copy_from_slice(&(list.as_ptr() as u64).to_le_bytes());
        patched[8..].copy_from_slice(&((list.len() / ENTRY) as u64).to_le_bytes());
        if spawner::write_bytes(desc + 0x18, &patched) {
            seen.original.copy_from_slice(&head[0x18..0x28]);
            seen.entries = list;
        } else {
            fail(&format!("the component list of template {template} could not be replaced for the immune trial"));
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
            let up = reinforced_value(value).unwrap();
            assert_eq!(up as u32, value as u32 * 40 + 39);
            assert!(is_reinforced(up));
            assert_eq!(reinforced_value(up), None);
            assert_eq!(original_value(up), Some(value));
            assert!(!is_reinforced(value), "{value} must not look reinforced");
        }
        for (original, code) in LARGE {
            assert_eq!(reinforced_value(original), Some(code));
            assert_eq!(original_value(code), Some(original));
            assert!(!is_reinforced(original));
        }
        assert_eq!(reinforced_value(0), None);
        assert_eq!(reinforced_value(SMALL_MAX + 1), None);
        assert_eq!(reinforced_value(3000), None);
        assert_eq!(original_value(10), None);
        assert_eq!(original_value(39), None);
    }

    #[test]
    fn the_game_templates_never_look_reinforced_and_always_fit() {
        for value in [1u16, 2, 4, 5, 6, 7, 10, 12, 15, 16, 20, 25, 30, 40, 50, 60, 70, 75, 100, 150, 160, 200, 250, 300, 450, 500, 1000, 2000, 4000, 10000] {
            assert!(!is_reinforced(value), "{value}");
            assert!(reinforced_value(value).is_some(), "{value}");
        }
    }

    #[test]
    fn health_is_multiplied_with_the_same_marker_and_restored() {
        assert_eq!(reinforced_health(100, 100), Some((4039, 4039)));
        assert_eq!(reinforced_health(50, 100), Some((2020, 4039)));
        assert_eq!(released_health(4039, 4039), Some((100, 100)));
        assert_eq!(released_health(2020, 4039), Some((50, 100)));
        assert_eq!(reinforced_health(0, 100), Some((0, 4039)));
        assert_eq!(reinforced_health(1, 100), Some((40, 4039)));
        assert_eq!(reinforced_health(4039, 4039), None);
        assert_eq!(reinforced_health(5, 0), None);
        assert_eq!(released_health(100, 100), None);
        assert_eq!(released_health(65518 - 7, 65518), Some((2000, 2000)));
        assert_eq!(released_health(160_039, 160_039), Some((4000, 4000)));
        assert_eq!(reinforced_health(100_000, 100_000), Some((4_000_039, 4_000_039)));
        assert_eq!(reinforced_health(1, HEALTH_MAX_LIMIT + 1), None);
    }

    #[test]
    fn damage_adds_up_so_a_reinforced_object_takes_forty_times_more_hits() {
        let hits = |start: u32| (start + 6) / 7;
        assert_eq!(hits(25), 4);
        assert_eq!(hits(reinforced_value(25).unwrap() as u32), 149);
        assert_eq!(hits(200), 29);
        assert!(hits(reinforced_value(200).unwrap() as u32) >= 40 * hits(200) - 40);
    }

    #[test]
    fn only_hammer_items_are_taken_from_the_item_list() {
        let text = "item_id,guid,debug_name,name,category,rarity,max_stack,slot,level_min\n\
            1001,a,Prop_Torch,Torch,BuildTools,Common,50,BuildTool,0\n\
            1002,b,Sword,Sword,Weapons,Common,1,MainHand,0\n\
            1003,c,Block,Block,Blueprints,Common,1,BuildTool,0\n\
            0,d,Broken,Broken,BuildTools,Common,1,BuildTool,0\n\
            x,e,Bad,Bad,BuildTools,Common,1,BuildTool,0\n";
        assert_eq!(parse_build_items(text), HashSet::from([1001, 1003]));
        let moved = "slot,item_id\nBuildTool,7\nHead,8\n";
        assert_eq!(parse_build_items(moved), HashSet::from([7]));
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
        assert_eq!(serde_json::from_str::<Options>(r#"{"immune":true}"#).unwrap(), Options { immune: true });
        assert_eq!(serde_json::from_str::<Options>("{}").unwrap(), Options::default());
    }

    const T: usize = 133;
    const D: usize = 301;
    const U: usize = 412;
    const H: usize = 77;

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
            for (map_at, table) in [(TYPE_MAPS[0], vec![(HASH_CURRENT_TRANSFORM, T), (HASH_USED_ITEM, U)]), (TYPE_MAPS[1], vec![(HASH_DESTRUCTIBLE, D), (HASH_HEALTH, H)])] {
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

    fn reinforced(zone: &str, threshold: Option<(u16, u16)>, health: Option<((u32, u32), (u32, u32))>) -> Outcome {
        Outcome::Reinforced { zone: zone.into(), item: TORCH, edit: edit_of(threshold, health) }
    }

    fn released(zone: &str, threshold: Option<(u16, u16)>, health: Option<((u32, u32), (u32, u32))>) -> Outcome {
        Outcome::Released { zone: zone.into(), item: TORCH, edit: edit_of(threshold, health) }
    }

    fn edit_of(threshold: Option<(u16, u16)>, health: Option<((u32, u32), (u32, u32))>) -> Edit {
        Edit {
            threshold,
            health_max: health.map(|(_, max)| max),
            health_current: health.map(|(current, _)| current),
        }
    }

    const TORCH: u32 = 1001;
    const SWORD: u32 = 2002;

    fn slots_for_tests() -> Slots {
        Slots { transform: T, destructible: D, used_item: U, health: Some(H) }
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

    #[test]
    fn the_whole_switch_against_a_stand_in_for_the_game() {
        *ITEMS.lock().unwrap() = Some(HashSet::from([TORCH]));
        *CATALOG.lock().unwrap() = None;
        *SLOTS.lock().unwrap() = None;
        let town = Areas { shielded: vec![area("Town", 100.0, 100.0, 30.0)], released: Vec::new() };
        let off = next_areas(&town, Vec::new());
        let slots = slots_for_tests();

        let mut w = World::new(0);
        assert_eq!(index_in(u64_at(&read_bytes(w.at(), 8).unwrap(), 0) as usize + TYPE_MAPS[0], HASH_DESTRUCTIBLE), None);
        assert_eq!(component_index(w.at(), HASH_DESTRUCTIBLE), Some(D));
        assert_eq!(component_index(w.at(), HASH_USED_ITEM), Some(U));
        assert_eq!(component_index(w.at(), 0x1234_5678), None);
        assert_eq!(find_slots(w.at()), Some(slots));

        let torch = w.entity(10, Some((110.0, 95.0)), Some(TORCH), Some(10), None);
        let far = w.entity(11, Some((400.0, 95.0)), Some(TORCH), Some(10), None);
        let world_prop = w.entity(12, Some((101.0, 101.0)), None, Some(10), None);
        let impact = w.entity(13, Some((101.0, 101.0)), Some(SWORD), Some(10), None);
        w.entity(14, Some((101.0, 101.0)), Some(TORCH), None, None);
        let bed = w.entity(15, Some((99.0, 101.0)), Some(TORCH), Some(25), Some((25, 25)));
        let odd = w.entity(16, Some((99.0, 99.0)), Some(TORCH), Some(3000), None);
        w.entity(17, None, Some(TORCH), Some(10), None);
        let oven = w.entity(18, Some((98.0, 98.0)), Some(TORCH), None, Some((60, 120)));
        let hurt = w.entity(19, Some((97.0, 97.0)), Some(TORCH), Some(200), Some((50, 200)));
        let huge = w.entity(21, Some((96.0, 96.0)), Some(TORCH), Some(10000), None);

        assert_eq!(settle(w.at(), 10, &slots, &town), reinforced("Town", Some((10, 439)), None));
        assert_eq!(threshold(torch), 439);
        assert_eq!(settle(w.at(), 10, &slots, &town), Outcome::Already);
        assert_eq!(threshold(torch), 439);
        assert_eq!(settle(w.at(), 11, &slots, &town), Outcome::Outside);
        assert_eq!(settle(w.at(), 12, &slots, &town), Outcome::NotBuilt);
        assert_eq!(settle(w.at(), 13, &slots, &town), Outcome::OtherItem(SWORD));
        assert_eq!(settle(w.at(), 14, &slots, &town), Outcome::NoDestructible(TORCH));
        assert_eq!(settle(w.at(), 15, &slots, &town), reinforced("Town", Some((25, 1039)), Some(((25, 1039), (25, 1039)))));
        assert_eq!(health(bed), (1039, 1039));
        assert_eq!(settle(w.at(), 15, &slots, &town), Outcome::Already);
        assert_eq!(settle(w.at(), 16, &slots, &town), Outcome::Unusual(3000));
        assert_eq!(settle(w.at(), 17, &slots, &town), Outcome::NoPosition(TORCH));
        assert_eq!(settle(w.at(), 99, &slots, &town), Outcome::Gone);
        assert_eq!(settle(w.at(), 18, &slots, &town), reinforced("Town", None, Some(((60, 2420), (120, 4839)))));
        assert_eq!(health(oven), (2420, 4839));
        assert_eq!(settle(w.at(), 19, &slots, &town), reinforced("Town", Some((200, 8039)), Some(((50, 2010), (200, 8039)))));
        assert_eq!(settle(w.at(), 21, &slots, &town), reinforced("Town", Some((10000, 65438)), None));
        assert_eq!((threshold(far), threshold(world_prop), threshold(impact), threshold(odd)), (10, 10, 10, 3000));

        assert_eq!(settle(w.at(), 10, &slots, &off), released("Town", Some((439, 10)), None));
        assert_eq!(threshold(torch), 10);
        assert_eq!(settle(w.at(), 10, &slots, &off), Outcome::Outside);
        assert_eq!(settle(w.at(), 15, &slots, &off), released("Town", Some((1039, 25)), Some(((1039, 25), (1039, 25)))));
        assert_eq!((threshold(bed), health(bed)), (25, (25, 25)));
        assert_eq!(settle(w.at(), 18, &slots, &off), released("Town", None, Some(((2420, 60), (4839, 120)))));
        assert_eq!(health(oven), (60, 120));
        assert_eq!(settle(w.at(), 19, &slots, &off), released("Town", Some((8039, 200)), Some(((2010, 50), (8039, 200)))));
        assert_eq!((threshold(hurt), health(hurt)), (200, (50, 200)));
        assert_eq!(settle(w.at(), 21, &slots, &off), released("Town", Some((65438, 10000)), None));
        assert_eq!(threshold(huge), 10000);

        *AREAS.lock().unwrap() = town.clone();
        LOADED.store(true, Ordering::SeqCst);
        IMMUNE.store(false, Ordering::SeqCst);
        ACTIVE.store(true, Ordering::SeqCst);
        SWEEP_WANTED.store(true, Ordering::SeqCst);
        let guid = super::super::config::parse_guid("11111111-1111-1111-1111-111111111111").unwrap();
        let empty = descriptor(&guid, &[]);
        let seen = before(w.at(), 0, empty.as_ptr() as usize).expect("an object from the save is looked at");
        assert_eq!((threshold(torch), threshold(bed), health(bed), health(oven)), (439, 1039, (1039, 1039), (2420, 4839)));
        assert_eq!((threshold(far), threshold(world_prop), threshold(impact)), (10, 10, 10));
        assert!(WORK.lock().unwrap().sweep.is_none());

        let loaded = w.entity(20, Some((105.0, 105.0)), Some(TORCH), Some(40), None);
        after(Some(seen), w.at(), 20);
        assert_eq!(threshold(loaded), 40);
        assert!(HAS_WORK.load(Ordering::SeqCst));
        std::thread::sleep(SETTLE + Duration::from_millis(50));
        let seen = before(w.at(), 0, empty.as_ptr() as usize);
        assert_eq!(threshold(loaded), 1639);
        after(seen, w.at(), 0);
        assert!(!HAS_WORK.load(Ordering::SeqCst));

        let item = TORCH.to_le_bytes();
        let inside = transform(110.0, 110.0);
        let entries = with_entry(&with_entry(&[], HASH_CURRENT_TRANSFORM, inside.as_ptr() as usize, 56), HASH_USED_ITEM, item.as_ptr() as usize, 4);
        let placed = descriptor(&guid, &entries);
        let placed_bytes = *placed;
        let seen = before(w.at(), 0x30, placed.as_ptr() as usize).unwrap();
        assert_eq!(*placed, placed_bytes);
        let fresh = w.entity(30, Some((110.0, 110.0)), Some(TORCH), Some(10), None);
        after(Some(seen), w.at(), 30);
        assert_eq!(threshold(fresh), 439);

        IMMUNE.store(true, Ordering::SeqCst);
        let seen = before(w.at(), 0x31, placed.as_ptr() as usize).unwrap();
        let head = read_bytes(placed.as_ptr() as usize, 0x28).unwrap();
        assert_eq!(u64_at(&head, 0x20), 3);
        let list = read_bytes(u64_at(&head, 0x18) as usize, 3 * ENTRY).unwrap();
        assert_eq!(&list[..2 * ENTRY], &entries[..]);
        assert_eq!((u32_at(&list, 2 * ENTRY), u64_at(&list, 2 * ENTRY + 16)), (HASH_COMBAT_FLAGS, 12));
        assert_eq!(read_bytes(u64_at(&list, 2 * ENTRY + 8) as usize, 12).unwrap(), COMBAT_FLAGS_IMMUNE.to_vec());
        after(Some(seen), w.at(), 0);
        assert_eq!(*placed, placed_bytes);
        IMMUNE.store(false, Ordering::SeqCst);

        let sword = SWORD.to_le_bytes();
        let hit = with_entry(&with_entry(&[], HASH_CURRENT_TRANSFORM, inside.as_ptr() as usize, 56), HASH_USED_ITEM, sword.as_ptr() as usize, 4);
        let hit_desc = descriptor(&guid, &hit);
        let seen = before(w.at(), 0x32, hit_desc.as_ptr() as usize).unwrap();
        after(Some(seen), w.at(), 13);
        assert_eq!(threshold(impact), 10);
        assert!(!HAS_WORK.load(Ordering::SeqCst));

        let client = World::new(1);
        assert!(before(client.at(), 0, placed.as_ptr() as usize).is_none());

        *AREAS.lock().unwrap() = off;
        SWEEP_WANTED.store(true, Ordering::SeqCst);
        let seen = before(w.at(), 0, empty.as_ptr() as usize);
        after(seen, w.at(), 0);
        assert_eq!((threshold(torch), threshold(bed), threshold(loaded), threshold(fresh)), (10, 25, 40, 10));
        assert_eq!((health(bed), health(oven), health(hurt)), ((25, 25), (60, 120), (50, 200)));

        ACTIVE.store(false, Ordering::SeqCst);
        assert!(before(w.at(), 0, placed.as_ptr() as usize).is_none());
        *AREAS.lock().unwrap() = Areas::default();
        *ITEMS.lock().unwrap() = None;
        *SLOTS.lock().unwrap() = None;
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

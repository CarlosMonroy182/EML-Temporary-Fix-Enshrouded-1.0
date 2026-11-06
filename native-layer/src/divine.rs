//! Divine aura: the glow of the towers' teleport platforms (`Prop_Ancient_Teleporter_Effect_On`)
//! follows an administrator who switched the aura on and is taken out of the world when it is
//! switched off. Nothing here runs for anybody else, nor while no aura is on.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::spawner;

const LIGHT_GUID: &str = "1c3281da-e7d4-4b41-9033-c85daec7f72a";
const RECORD_FILE: &str = "m182_divine_lights.json";
const MOVE_METERS: f64 = 2.5;
const STEP_METERS: f64 = 0.25;
const HELD_METERS: f64 = 1.0;
const RECORD_MOVE_METERS: f64 = 3.0;
const CHECK_EVERY: Duration = Duration::from_millis(100);
const ORDER_RETRY: Duration = Duration::from_secs(3);
const STRAY_METERS: f64 = 3.0;
const SWEEP_EVERY: Duration = Duration::from_secs(8);
const METERS: f64 = 4_294_967_296.0;

/// Owner key of the game this process hosts; chat clients use their client number.
pub const HOST: u32 = u32::MAX;

#[derive(Default)]
struct Owner {
    id: Option<u32>,
    spot: [f64; 3],
    asked: Option<Instant>,
    checked: Option<Instant>,
    old: Vec<u32>,
    wrote: Option<[f64; 3]>,
    fails: u8,
    rigid: bool,
}

static OWNERS: Mutex<Option<HashMap<u32, Owner>>> = Mutex::new(None);
static ORDERS: Mutex<Vec<(u32, [i64; 3])>> = Mutex::new(Vec::new());
static SEEN: Mutex<Vec<(usize, u32)>> = Mutex::new(Vec::new());
static PENDING: AtomicU32 = AtomicU32::new(0);
static WATCH: AtomicBool = AtomicBool::new(false);
static SWEPT: Mutex<Option<Instant>> = Mutex::new(None);

fn light() -> Option<[u8; 16]> {
    super::config::parse_guid(LIGHT_GUID)
}

pub fn pending() -> bool {
    PENDING.load(Ordering::Relaxed) != 0
}

fn flat(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn meters(raw: [i64; 3]) -> [f64; 3] {
    [raw[0] as f64 / METERS, raw[1] as f64 / METERS, raw[2] as f64 / METERS]
}

fn records() -> Vec<[f64; 3]> {
    std::fs::read_to_string(RECORD_FILE).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_records(list: &[[f64; 3]]) {
    if list.is_empty() {
        let _ = std::fs::remove_file(RECORD_FILE);
    } else {
        let _ = std::fs::write(RECORD_FILE, serde_json::to_string(list).unwrap_or_default());
    }
}

fn record(spot: [f64; 3]) {
    let mut list = records();
    list.push(spot);
    save_records(&list);
}

fn forget(spot: [f64; 3]) {
    let mut list = records();
    if let Some(at) = list.iter().position(|s| flat(*s, spot) < 0.5 && (s[1] - spot[1]).abs() < 1.0) {
        list.remove(at);
        save_records(&list);
    }
}

/// Starts watching for glows left in the world by an earlier run of the server.
pub fn start() {
    if std::path::Path::new(RECORD_FILE).is_file() {
        WATCH.store(true, Ordering::Relaxed);
    }
}

/// Called for every entity the game creates on the server: remembers the glows so strays can be found.
pub fn note_created(manager: usize, desc: usize, id: u32) {
    if !WATCH.load(Ordering::Relaxed) {
        return;
    }
    let Some(template) = light() else { return };
    let Some(head) = super::spawn_probe::read_bytes(desc + 4, 16) else { return };
    if head[..] != template[..] {
        return;
    }
    if let Ok(mut seen) = SEEN.lock() {
        if seen.len() >= 256 {
            seen.remove(0);
        }
        seen.push((manager, id));
    }
}

/// Keeps the glow with the administrator at `position` (raw world units) while the aura is on.
pub fn follow(manager: usize, key: u32, position: [i64; 3]) {
    let Ok(mut guard) = OWNERS.lock() else { return };
    let owners = guard.get_or_insert_with(HashMap::new);
    let owner = owners.entry(key).or_default();
    if owner.checked.is_some_and(|t| t.elapsed() < CHECK_EVERY) {
        return;
    }
    owner.checked = Some(Instant::now());
    WATCH.store(true, Ordering::Relaxed);
    for id in std::mem::take(&mut owner.old) {
        spawner::retire_entity(manager, id);
    }
    let here = meters(position);
    if let Some(id) = owner.id {
        let Some(current) = spawner::entity_position(manager, id) else {
            if spawner::find_record(manager, id).is_none() {
                owner.id = None;
                forget(owner.spot);
            }
            return;
        };
        let apart = flat(current, here).max((current[1] - here[1]).abs());
        if apart <= STEP_METERS {
            owner.fails = 0;
            return;
        }
        if !owner.rigid {
            if owner.wrote.is_some_and(|w| flat(current, w) > HELD_METERS) {
                owner.fails += 1;
            } else {
                owner.fails = 0;
            }
            if owner.fails >= 3 {
                owner.rigid = true;
                spawner::log("divine aura: the glow does not follow by moving it; it is placed again when you walk away");
            } else if spawner::move_entity(manager, id, position) {
                owner.wrote = Some(here);
                if flat(owner.spot, here) > RECORD_MOVE_METERS {
                    forget(owner.spot);
                    owner.spot = here;
                    record(here);
                }
                return;
            }
        }
        if apart <= MOVE_METERS {
            return;
        }
    }
    if owner.asked.is_some_and(|t| t.elapsed() < ORDER_RETRY) {
        return;
    }
    owner.asked = Some(Instant::now());
    if let Ok(mut orders) = ORDERS.lock() {
        orders.retain(|(k, _)| *k != key);
        orders.push((key, position));
        PENDING.store(1, Ordering::SeqCst);
    }
}

/// The aura of this owner went off (or the owner left): the glow leaves the world.
pub fn release(manager: usize, key: u32) {
    let Ok(mut guard) = OWNERS.lock() else { return };
    let Some(owner) = guard.as_mut().and_then(|m| m.remove(&key)) else { return };
    if let Ok(mut orders) = ORDERS.lock() {
        orders.retain(|(k, _)| *k != key);
    }
    for id in owner.old.into_iter().chain(owner.id) {
        spawner::retire_entity(manager, id);
    }
    if owner.id.is_some() {
        forget(owner.spot);
    }
}

/// Creates the ordered glows. Called from the game's own entity-create call on the server.
pub fn run_orders(manager: usize) {
    if PENDING.swap(0, Ordering::SeqCst) == 0 {
        return;
    }
    let orders: Vec<(u32, [i64; 3])> = match ORDERS.lock() {
        Ok(mut o) => std::mem::take(&mut *o),
        Err(_) => return,
    };
    let Some(template) = light() else { return };
    for (key, at) in orders.into_iter().take(8) {
        let id = spawner::create_prop(manager, template, at[0], at[1], at[2]);
        if id == 0 {
            spawner::log("divine aura: ERROR the game did not create the glow");
            continue;
        }
        let Ok(mut guard) = OWNERS.lock() else { continue };
        match guard.as_mut().and_then(|m| m.get_mut(&key)) {
            Some(owner) => {
                if let Some(previous) = owner.id.replace(id) {
                    owner.old.push(previous);
                    forget(owner.spot);
                }
                owner.spot = meters(at);
                owner.wrote = None;
                owner.fails = 0;
                record(owner.spot);
            }
            None => {
                record(meters(at));
                if spawner::retire_entity(manager, id) {
                    forget(meters(at));
                }
            }
        }
    }
}

/// Glows saved in the world by a run that ended with an aura on are taken out when they load.
pub fn sweep(manager: usize) {
    if !WATCH.load(Ordering::Relaxed) {
        return;
    }
    if let Ok(mut swept) = SWEPT.lock() {
        if swept.is_some_and(|t| t.elapsed() < SWEEP_EVERY) {
            return;
        }
        *swept = Some(Instant::now());
    }
    let spots = records();
    let owned: Vec<u32> = OWNERS
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|m| m.values().flat_map(|o| o.id.into_iter().chain(o.old.iter().copied())).collect()))
        .unwrap_or_default();
    let seen: Vec<u32> = SEEN.lock().map(|v| v.iter().filter(|(m, _)| *m == manager).map(|(_, id)| *id).collect()).unwrap_or_default();
    let mut gone = Vec::new();
    for id in seen {
        if owned.contains(&id) {
            continue;
        }
        let Some(at) = spawner::entity_position(manager, id) else { continue };
        if let Some(spot) = spots.iter().find(|s| flat(**s, at) <= STRAY_METERS) {
            if spawner::retire_entity(manager, id) {
                forget(*spot);
                gone.push(id);
            }
        }
    }
    if !gone.is_empty() {
        if let Ok(mut seen) = SEEN.lock() {
            seen.retain(|(_, id)| !gone.contains(id));
        }
    }
    if records().is_empty() {
        WATCH.store(OWNERS.lock().ok().is_some_and(|g| g.as_ref().is_some_and(|m| !m.is_empty())), Ordering::Relaxed);
    }
}

//! Divine aura: the glow of the towers' teleport platforms (`Prop_Ancient_Teleporter_Effect_On`)
//! appears under an administrator who switched the aura on once he has stood still for ten seconds,
//! and is taken out of the world as soon as he moves or switches the aura off. The glow is destroyed through its lifetime
//! (see `mods/m182_divine`), so every player sees it go. Nothing here runs for anybody else, nor while no aura is on.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::spawner;

const LIGHT_GUID: &str = "1c3281da-e7d4-4b41-9033-c85daec7f72a";
const RECORD_FILE: &str = "m182_divine_lights.json";
const CHECK_EVERY: Duration = Duration::from_millis(500);
const STILL_AFTER: Duration = Duration::from_secs(10);
const STILL_METERS: f64 = 1.0;
const ORDER_RETRY: Duration = Duration::from_secs(3);
const STRAY_METERS: f64 = 3.0;
const SWEEP_EVERY: Duration = Duration::from_secs(8);
const METERS: f64 = 4_294_967_296.0;
const LIFE_MS: u64 = 600_000;
const RENEW_AFTER: Duration = Duration::from_secs(540);
const CHECK_GONE_AFTER: Duration = Duration::from_secs(3);
const LIFE_NS: [u64; 2] = [LIFE_MS * 1_000_000, 1_000_000_000_000_000_000];

/// Owner key of the game this process hosts; chat clients use their client number.
pub const HOST: u32 = u32::MAX;

#[derive(Default)]
struct Owner {
    id: Option<u32>,
    spot: [f64; 3],
    asked: Option<Instant>,
    checked: Option<Instant>,
    old: Vec<u32>,
    anchor: Option<[f64; 3]>,
    still_since: Option<Instant>,
    born: Option<Instant>,
}

static OWNERS: Mutex<Option<HashMap<u32, Owner>>> = Mutex::new(None);
static ORDERS: Mutex<Vec<(u32, [i64; 3])>> = Mutex::new(Vec::new());
static SEEN: Mutex<Vec<(usize, u32)>> = Mutex::new(Vec::new());
static PENDING: AtomicU32 = AtomicU32::new(0);
static WATCH: AtomicBool = AtomicBool::new(false);
static LEAVING: Mutex<Vec<(usize, u32, Instant)>> = Mutex::new(Vec::new());
static MISSING_LOGGED: AtomicBool = AtomicBool::new(false);
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

/// Takes a glow out of the world through its lifetime, and keeps an eye on it until it is gone.
fn drop_glow(manager: usize, id: u32) -> bool {
    let done = spawner::expire_entity(manager, id, &LIFE_NS);
    if let Ok(mut leaving) = LEAVING.lock() {
        if leaving.len() >= 64 {
            leaving.remove(0);
        }
        leaving.push((manager, id, Instant::now()));
    }
    done
}

fn check_leaving() {
    let due: Vec<(usize, u32)> = match LEAVING.lock() {
        Ok(mut l) if !l.is_empty() => {
            let (due, rest): (Vec<_>, Vec<_>) = l.drain(..).partition(|(_, _, t)| t.elapsed() >= CHECK_GONE_AFTER);
            *l = rest;
            due.into_iter().map(|(m, id, _)| (m, id)).collect()
        }
        _ => return,
    };
    for (manager, id) in due {
        if spawner::find_record(manager, id).is_some() {
            spawner::log("divine aura: ERROR the glow did not go away (is the mods\\m182_divine folder installed on the server?)");
            spawner::expire_entity(manager, id, &LIFE_NS);
        }
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
        drop_glow(manager, id);
    }
    let here = meters(position);
    let moved = owner.anchor.is_none_or(|a| flat(a, here) > STILL_METERS || (a[1] - here[1]).abs() > STILL_METERS);
    if moved {
        owner.anchor = Some(here);
        owner.still_since = Some(Instant::now());
        if let Ok(mut orders) = ORDERS.lock() {
            orders.retain(|(k, _)| *k != key);
        }
        if let Some(id) = owner.id.take() {
            drop_glow(manager, id);
            forget(owner.spot);
        }
        return;
    }
    if let Some(id) = owner.id {
        if owner.born.is_some_and(|t| t.elapsed() >= RENEW_AFTER) {
            drop_glow(manager, id);
            forget(owner.spot);
            owner.id = None;
            owner.asked = None;
            return;
        }
        if spawner::find_record(manager, id).is_none() {
            owner.id = None;
            forget(owner.spot);
        } else if flat(owner.spot, here) > 2.0 {
            drop_glow(manager, id);
            forget(owner.spot);
            owner.id = None;
            owner.anchor = Some(here);
            owner.still_since = Some(Instant::now());
        }
        return;
    }
    if !owner.still_since.is_some_and(|t| t.elapsed() >= STILL_AFTER) {
        return;
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
        drop_glow(manager, id);
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
        let id = spawner::create_expiring_prop(manager, template, at[0], at[1], at[2], LIFE_MS);
        if id == 0 {
            spawner::log("divine aura: ERROR the game did not create the glow");
            continue;
        }
        if !spawner::has_life_time(manager, id, &LIFE_NS) && !MISSING_LOGGED.swap(true, Ordering::Relaxed) {
            spawner::log("divine aura: ERROR the glow has no lifetime, so it cannot be removed: install the mods\\m182_divine folder (server and the PC that hosts)");
        }
        let Ok(mut guard) = OWNERS.lock() else { continue };
        match guard.as_mut().and_then(|m| m.get_mut(&key)) {
            Some(owner) => {
                if let Some(previous) = owner.id.replace(id) {
                    owner.old.push(previous);
                    forget(owner.spot);
                }
                owner.spot = meters(at);
                owner.born = Some(Instant::now());
                record(owner.spot);
            }
            None => {
                record(meters(at));
                if drop_glow(manager, id) {
                    forget(meters(at));
                }
            }
        }
    }
}

/// Glows saved in the world by a run that ended with an aura on are taken out when they load.
pub fn sweep(manager: usize) {
    check_leaving();
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
            if drop_glow(manager, id) {
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

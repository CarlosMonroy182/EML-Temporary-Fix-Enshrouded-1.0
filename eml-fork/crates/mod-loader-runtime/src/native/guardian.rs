//! Base guardian (trial): a creature an admin places for the team that owns a zone, at most one per zone.
//! It carries the faction of the player group, so it is hostile to every creature, and the relation hooks
//! decide the rest by the address of its faction (and of the attack entities that copy it, its kin): it is
//! a friend of the owning team (and of the teams `friends` names) and an enemy of every other player. When
//! the guardian stands outside its zone the hooks stop naming targets, so the game's own AI sends it home.
//! Nothing here runs unless a guardian was placed: the hooks return before looking at anything, and there
//! is no timer. The bookkeeping follows game events only: the hooks' own questions, every 16th entity
//! creation, and a change of the zones.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::groups::{self, Zone};
use super::spawn_probe::read_bytes;
use super::{shield, spawner};

/// The faction the guardian is given: the player group's `PlayerAlly`.
pub const FACTION_VALUE: u8 = 11;
pub const HASH_DO_NOT_SAVE: u32 = 0x41e6_78ea;
pub const DO_NOT_SAVE_DATA: [u8; 1] = [0];
const HASH_FACTION: u32 = 0xa1f3_0f21;
const HASH_ENEMY_SETTINGS: u32 = 0x67fe_5508;
const FACTION_COMPONENT: usize = 205;
const ENEMY_SETTINGS_SIZE: usize = 68;
const HOME_ZONE_AT: usize = 60;
const LEASH_AT: usize = 64;
const LEASH_SHARE: f32 = 0.9;
const LEASH_LEAST: f32 = 5.0;
const MAX_GUARDIANS: usize = 16;
const MAX_KIN: usize = 48;
const REFRESH_EVERY: u32 = 16;
const POSITION_FRESH: Duration = Duration::from_secs(1);
const MAX_ASKED: usize = 24;

/// Where and for whom a guardian stands.
#[derive(Debug, Clone, PartialEq)]
pub struct Post {
    pub zone: String,
    pub team: u8,
    pub x: f64,
    pub z: f64,
    pub radius: f64,
    /// Placed again by the game's own events after a restart, not by an administrator.
    pub auto: bool,
}

/// What is needed to place a zone's guardian again: kept in `m182_guardians.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub zone: String,
    pub template: String,
    pub level: u32,
    pub at: [f64; 3],
}

#[derive(Debug, Clone)]
struct Guardian {
    id: u32,
    faction_at: usize,
    post: Post,
    /// Faction addresses of entities that copied the guardian's side (its attacks), found when the game asked.
    kin: Vec<usize>,
    outside: Option<(Instant, bool)>,
}

impl Guardian {
    fn new(id: u32, faction_at: usize, post: Post) -> Self {
        Self { id, faction_at, post, kin: Vec::new(), outside: None }
    }

    fn owns(&self, addr: usize) -> bool {
        self.faction_at == addr || self.kin.contains(&addr)
    }
}

static ANY: AtomicBool = AtomicBool::new(false);
static GUARDIANS: Mutex<Vec<Guardian>> = Mutex::new(Vec::new());
static ASKED: Mutex<Vec<(usize, bool)>> = Mutex::new(Vec::new());
static VALUE_CALLS: AtomicU32 = AtomicU32::new(0);
static CREATIONS: AtomicU32 = AtomicU32::new(0);
static WANTED: AtomicBool = AtomicBool::new(false);
static REMOVALS_PENDING: AtomicBool = AtomicBool::new(false);
static SAVED: Mutex<Vec<Saved>> = Mutex::new(Vec::new());
static DEAD: Mutex<Vec<String>> = Mutex::new(Vec::new());
static ASKED_FOR: Mutex<Vec<(String, Instant)>> = Mutex::new(Vec::new());
static REMOVALS: Mutex<Vec<String>> = Mutex::new(Vec::new());
const SAVED_FILE: &str = "m182_guardians.json";
const ASK_AGAIN: Duration = Duration::from_secs(45);

fn log(text: &str) {
    spawner::log(&format!("guardian: {text}"));
}

/// The post of an enabled zone with an owning team, by name.
pub fn post_for(zones: &[Zone], name: &str) -> Result<Post, String> {
    let wanted = name.trim();
    let Some(zone) = zones.iter().find(|z| z.name.trim().eq_ignore_ascii_case(wanted)) else {
        return Err(format!("there is no zone called '{wanted}'"));
    };
    if zone.enabled == Some(false) {
        return Err(format!("zone '{}' is switched off", zone.name));
    }
    let team = zone.owner();
    if team == 0 || team > 255 {
        return Err(format!("zone '{}' has no owning team", zone.name));
    }
    Ok(Post { zone: zone.name.clone(), team: team as u8, x: zone.x, z: zone.z, radius: zone.radius, auto: false })
}

/// Whether the guardian of team `owner` treats the players of team `other` as friends. This is the one place
/// that decides which teams count as friends of an owning team; a team is always a friend of itself.
fn friends(owner: u8, other: u8) -> bool {
    owner != 0 && owner == other
}

fn player_group(value: u8) -> bool {
    matches!(value, 7 | 8 | 11)
}

/// Whether a pair that involves a guardian is hostile, or `None` when the game's own rules stay in charge.
/// `team_of` gives the team of the faction at an address; `away` tells whether a guardian is outside its zone.
fn decide(guardians: &[Guardian], a: (usize, u8), b: (usize, u8), team_of: &dyn Fn(usize) -> u8, away: &dyn Fn(&Guardian) -> bool) -> Option<bool> {
    let of = |addr: usize| guardians.iter().find(|g| g.owns(addr));
    match (of(a.0), of(b.0)) {
        (None, None) => None,
        (Some(g), Some(h)) => Some(!friends(g.post.team, h.post.team)),
        (Some(g), None) => other(g, b, team_of, away),
        (None, Some(g)) => other(g, a, team_of, away),
    }
}

fn other(guardian: &Guardian, other: (usize, u8), team_of: &dyn Fn(usize) -> u8, away: &dyn Fn(&Guardian) -> bool) -> Option<bool> {
    if !player_group(other.1) {
        return creature_relation(other.1, away(guardian));
    }
    if friends(guardian.post.team, team_of(other.0)) || away(guardian) {
        return Some(false);
    }
    Some(true)
}

/// Creatures that are enemies of the players are enemies of the guardian whatever their species, so a
/// guardian never spares the raiders of its own kind. Peaceful factions stay with the game.
fn creature_relation(value: u8, away: bool) -> Option<bool> {
    if value == 0 || !(super::faction::enemy_vanilla(value, super::faction::PLAYER) || super::faction::enemy_vanilla(super::faction::PLAYER, value)) {
        return None;
    }
    Some(!away)
}

/// A faction of the guardian's own side that belongs to nobody: one of its attacks.
fn is_kin_candidate(guardians: &[Guardian], addr: usize, value: u8, team_of: &dyn Fn(usize) -> u8) -> bool {
    value == FACTION_VALUE && team_of(addr) == 0 && !guardians.iter().any(|g| g.owns(addr))
}

/// The hook point of the relation functions that look at faction addresses. `None` unless the pair involves a
/// guardian.
pub fn relation(a: usize, fa: u8, b: usize, fb: u8, asked: &str) -> Option<bool> {
    if !ANY.load(Ordering::Relaxed) {
        return None;
    }
    let mut guardians = GUARDIANS.try_lock().ok()?;
    let index = guardians.iter().position(|g| g.owns(a)).or_else(|| guardians.iter().position(|g| g.owns(b)))?;
    let (other, value) = if guardians[index].owns(a) { (b, fb) } else { (a, fa) };
    if is_kin_candidate(&guardians, other, value, &groups::team_of) && guardians[index].kin.len() < MAX_KIN {
        guardians[index].kin.push(other);
    }
    refresh_positions(&mut guardians, spawner::server_manager());
    let away = |g: &Guardian| g.outside.is_some_and(|(_, out)| out);
    let hostile = decide(&guardians, (a, fa), (b, fb), &groups::team_of, &away)?;
    let g = guardians[index].clone();
    drop(guardians);
    say_relation(&g, other, value, asked, hostile);
    Some(hostile)
}

/// Whether each guardian stands outside its zone, looked up at most once a second and only while the game asks.
fn refresh_positions(list: &mut [Guardian], manager: usize) {
    for g in list.iter_mut() {
        if g.outside.is_some_and(|(at, _)| at.elapsed() < POSITION_FRESH) {
            continue;
        }
        let out = spawner::entity_position(manager, g.id).is_some_and(|[x, _, z]| (x - g.post.x).powi(2) + (z - g.post.z).powi(2) > g.post.radius * g.post.radius);
        g.outside = Some((Instant::now(), out));
    }
}

fn say_relation(g: &Guardian, other: usize, value: u8, asked: &str, hostile: bool) {
    let Ok(mut known) = ASKED.try_lock() else { return };
    if known.len() >= MAX_ASKED || known.contains(&(other, hostile)) {
        return;
    }
    known.push((other, hostile));
    drop(known);
    let team = groups::team_of(other);
    log(&format!(
        "{asked} was asked about the guardian of '{}' and the entity whose faction is at {other:#x} (faction {value}, team {team}): {}; the guardian is {}",
        g.post.zone,
        if hostile { "enemy" } else { "friend" },
        distance_text(g)
    ));
}

/// The values variants of the relation functions carry no address, so all that can be told is that the game
/// asked while a guardian exists.
pub fn note_values(a: u8, b: u8, asked: &str) {
    if !ANY.load(Ordering::Relaxed) || !(a == FACTION_VALUE || b == FACTION_VALUE) {
        return;
    }
    if VALUE_CALLS.fetch_add(1, Ordering::Relaxed) < 8 {
        log(&format!("{asked} (values) asked about factions {a} and {b}; the values carry no address, so the guardian's own rule does not apply there"));
    }
}

fn distance_text(g: &Guardian) -> String {
    match spawner::entity_position(spawner::server_manager(), g.id) {
        Some([x, _, z]) => format!("{:.0} m from the center of '{}' (radius {:.0} m)", ((x - g.post.x).powi(2) + (z - g.post.z).powi(2)).sqrt(), g.post.zone, g.post.radius),
        None => "at a position that cannot be read".to_string(),
    }
}

fn faction_slot(manager: usize) -> usize {
    shield::component_index(manager, HASH_FACTION).unwrap_or(FACTION_COMPONENT)
}

fn faction_at(manager: usize, id: u32) -> Option<usize> {
    let p = spawner::find_record(manager, id).and_then(spawner::placement)?;
    let (addr, size) = spawner::component_address(&p, faction_slot(manager))?;
    (size == 1).then_some(addr)
}

fn put_f32(addr: usize, value: f32) -> bool {
    spawner::write_bytes(addr, &value.to_le_bytes())
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// The leash inside the creature's own data: how far it may go from where it was created, and how far from
/// that place it may pick a target.
fn leash(manager: usize, id: u32, post: &Post) {
    let Some(p) = spawner::find_record(manager, id).and_then(spawner::placement) else { return };
    let Some(index) = shield::component_index(manager, HASH_ENEMY_SETTINGS) else {
        log(&format!("'{}': the leash data of this creature were not found in the game's component table; it can leave the zone", post.zone));
        return;
    };
    let Some((addr, size)) = spawner::component_address(&p, index).filter(|(_, size)| *size == ENEMY_SETTINGS_SIZE) else {
        log(&format!("'{}': the creature has no leash data (component slot {index}); it can leave the zone", post.zone));
        return;
    };
    let Some(b) = read_bytes(addr, size).filter(|b| b.len() == ENEMY_SETTINGS_SIZE) else { return };
    let (home, length) = (f32_at(&b, HOME_ZONE_AT), f32_at(&b, LEASH_AT));
    let wanted = ((post.radius as f32) * LEASH_SHARE).max(LEASH_LEAST);
    let reach = if home.is_finite() && home > 0.0 { home.min(wanted) } else { wanted };
    let ok = put_f32(addr + HOME_ZONE_AT, reach) && put_f32(addr + LEASH_AT, wanted);
    log(&format!(
        "'{}': leash data at {addr:#x}: distance to home zone {home} -> {reach}, leash length {length} -> {wanted} ({})",
        post.zone,
        if ok { "written" } else { "write refused" }
    ));
}

/// Called right after the game created the guardian's entity.
pub fn adopt(manager: usize, id: u32, post: Post) -> bool {
    let Some(p) = spawner::find_record(manager, id).and_then(spawner::placement) else {
        log(&format!("entity {id} cannot be found right after it was created; it is not a guardian"));
        return false;
    };
    let slot = faction_slot(manager);
    let Some((addr, size)) = spawner::component_address(&p, slot).filter(|(_, size)| *size == 1) else {
        log(&format!("entity {id} has no faction at component slot {slot} (table says {:?}, built-in {FACTION_COMPONENT}); it is not a guardian", shield::component_index(manager, HASH_FACTION)));
        return false;
    };
    let before = read_bytes(addr, size).and_then(|b| b.first().copied()).unwrap_or(0);
    if !spawner::write_bytes(addr, &[FACTION_VALUE]) {
        log(&format!("the faction of entity {id} could not be written; it is not a guardian"));
        return false;
    }
    leash(manager, id, &post);
    let Ok(mut list) = GUARDIANS.lock() else { return false };
    if list.len() >= MAX_GUARDIANS && !list.iter().any(|g| same_zone(&g.post.zone, &post.zone)) {
        let oldest = list.remove(0);
        spawner::retire_entity(manager, oldest.id);
        log(&format!("entity {} of '{}' was taken out: no more than {MAX_GUARDIANS} guardians at once", oldest.id, oldest.post.zone));
    }
    log(&format!(
        "entity {id} guards '{}' for team {}: faction {before} -> {FACTION_VALUE} at {addr:#x} (component slot {slot}, table says {:?})",
        post.zone,
        post.team,
        shield::component_index(manager, HASH_FACTION)
    ));
    if let Some(old) = register(&mut list, Guardian::new(id, addr, post)) {
        let removed = spawner::retire_entity(manager, old.id);
        log(&format!("entity {} replaces entity {} as the guardian of '{}' ({})", id, old.id, old.post.zone, if removed { "the old one was taken out" } else { "the old one could not be taken out" }));
    }
    ANY.store(true, Ordering::SeqCst);
    if let Ok(mut dead) = DEAD.lock() {
        dead.retain(|z| !same_zone(z, &list.last().map(|g| g.post.zone.clone()).unwrap_or_default()));
    }
    true
}

fn same_zone(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Adds a guardian; the one the same zone had before, if any, is returned.
fn register(list: &mut Vec<Guardian>, guardian: Guardian) -> Option<Guardian> {
    let old = list.iter().position(|g| same_zone(&g.post.zone, &guardian.post.zone)).map(|i| list.remove(i));
    list.push(guardian);
    old
}

/// Guardians whose zone is gone, switched off, or owned by another team now, with the reason; the others
/// follow the zone's new place and size.
fn follow(list: &mut [Guardian], zones: &[Zone]) -> Vec<(u32, String)> {
    let mut orphans = Vec::new();
    for g in list.iter_mut() {
        match post_for(zones, &g.post.zone) {
            Err(_) => orphans.push((g.id, format!("zone '{}' is gone or has no owner", g.post.zone))),
            Ok(post) if post.team != g.post.team => orphans.push((g.id, format!("zone '{}' belongs to team {} now, not team {}", g.post.zone, post.team, g.post.team))),
            Ok(post) => g.post = post,
        }
    }
    orphans
}

/// Called when the zones were read again: a guardian whose zone changed owner or disappeared is taken out.
pub fn zones_changed(manager: usize, zones: &[Zone]) {
    load_saved();
    if !ANY.load(Ordering::Relaxed) {
        return;
    }
    let Ok(mut list) = GUARDIANS.lock() else { return };
    let orphans = follow(&mut list, zones);
    for (id, why) in &orphans {
        let removed = spawner::retire_entity(manager, *id);
        log(&format!("entity {id} stops guarding: {why} ({})", if removed { "taken out" } else { "could not be taken out" }));
        if let Some(g) = list.iter().find(|g| g.id == *id) {
            forget(&g.post.zone);
        }
    }
    list.retain(|g| !orphans.iter().any(|(id, _)| *id == g.id));
    if list.is_empty() {
        ANY.store(false, Ordering::SeqCst);
    }
}

fn saved_path() -> Option<std::path::PathBuf> {
    std::env::current_dir().ok().map(|d| d.join(SAVED_FILE))
}

fn write_saved(list: &[Saved]) {
    let Some(path) = saved_path() else { return };
    let tmp = path.with_extension("tmp");
    let ok = serde_json::to_string(list).ok().is_some_and(|t| std::fs::write(&tmp, t).is_ok() && std::fs::rename(&tmp, &path).is_ok());
    if !ok {
        log("ERROR m182_guardians.json could not be written; the guardians will not come back after a restart");
    }
}

/// Reads the guardians kept from an earlier run. Called when the zones are read.
fn load_saved() {
    let Ok(mut saved) = SAVED.lock() else { return };
    if saved.is_empty() {
        if let Some(list) = saved_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str::<Vec<Saved>>(&t).ok()) {
            *saved = list;
        }
    }
    WANTED.store(!saved.is_empty(), Ordering::SeqCst);
}

/// Keeps what is needed to place the zone's guardian again after a restart.
pub fn remember(zone: &str, template: &str, level: u32, at: [f64; 3]) {
    let Ok(mut saved) = SAVED.lock() else { return };
    saved.retain(|s| !same_zone(&s.zone, zone));
    saved.push(Saved { zone: zone.to_string(), template: template.to_string(), level, at });
    write_saved(&saved);
    WANTED.store(true, Ordering::SeqCst);
}

fn forget(zone: &str) -> bool {
    let Ok(mut saved) = SAVED.lock() else { return false };
    let before = saved.len();
    saved.retain(|s| !same_zone(&s.zone, zone));
    let changed = saved.len() != before;
    if changed {
        write_saved(&saved);
    }
    WANTED.store(!saved.is_empty(), Ordering::SeqCst);
    changed
}

/// The saved guardian that should be placed again: its zone is enabled and owned, nothing guards it now, it did
/// not die since the start, it was not asked for a moment ago, and somebody stands inside.
fn pick_due(saved: &[Saved], live: &[String], dead: &[String], asked: &[(String, Instant)], zones: &[Zone], players: &[(f64, f64)]) -> Option<(Post, Saved)> {
    saved.iter().find_map(|s| {
        if live.iter().any(|z| same_zone(z, &s.zone)) || dead.iter().any(|z| same_zone(z, &s.zone)) || asked.iter().any(|(z, at)| same_zone(z, &s.zone) && at.elapsed() < ASK_AGAIN) {
            return None;
        }
        let mut post = post_for(zones, &s.zone).ok()?;
        let inside = players.iter().any(|(x, z)| (x - post.x).powi(2) + (z - post.z).powi(2) <= post.radius * post.radius);
        post.auto = true;
        inside.then(|| (post, s.clone()))
    })
}

/// Called with the positions of the players while the group tick already goes over them: the first time somebody
/// stands in a zone whose guardian is missing (after a restart), it is asked for once.
pub fn due(zones: &[Zone], players: &[(f64, f64)]) -> Option<(Post, Saved)> {
    if !WANTED.load(Ordering::Relaxed) || players.is_empty() {
        return None;
    }
    let saved = SAVED.lock().ok()?.clone();
    let live: Vec<String> = GUARDIANS.lock().ok()?.iter().map(|g| g.post.zone.clone()).collect();
    let dead = DEAD.lock().ok()?.clone();
    let mut asked = ASKED_FOR.lock().ok()?;
    asked.retain(|(_, at)| at.elapsed() < ASK_AGAIN);
    let found = pick_due(&saved, &live, &dead, &asked, zones, players)?;
    asked.push((found.0.zone.clone(), Instant::now()));
    Some(found)
}

/// Asks for the guardian of a zone to be taken away; done at the next creation event.
pub fn queue_remove(zone: &str) {
    if let Ok(mut r) = REMOVALS.lock() {
        r.push(zone.to_string());
        REMOVALS_PENDING.store(true, Ordering::SeqCst);
    }
}

pub fn removals_pending() -> bool {
    REMOVALS_PENDING.load(Ordering::Relaxed)
}

/// Takes away the guardians that were asked to go: the live creature, and its place in the saved list.
pub fn run_removals(manager: usize) {
    if !REMOVALS_PENDING.swap(false, Ordering::SeqCst) {
        return;
    }
    let zones: Vec<String> = REMOVALS.lock().map(|mut r| std::mem::take(&mut *r)).unwrap_or_default();
    for zone in zones {
        let live = GUARDIANS.lock().ok().and_then(|mut list| list.iter().position(|g| same_zone(&g.post.zone, &zone)).map(|i| list.remove(i)));
        let kept = forget(&zone);
        if let Ok(mut dead) = DEAD.lock() {
            dead.retain(|z| !same_zone(z, &zone));
        }
        match live {
            Some(g) => {
                let removed = spawner::retire_entity(manager, g.id);
                log(&format!("the guardian of '{zone}' (entity {}) was removed: {}", g.id, if removed { "taken out of the world" } else { "it could not be taken out, remove it by hand" }));
            }
            None if kept => log(&format!("the saved guardian of '{zone}' was removed; none was standing")),
            None => log(&format!("'{zone}' has no guardian to remove")),
        }
    }
    if GUARDIANS.lock().is_ok_and(|l| l.is_empty()) {
        ANY.store(false, Ordering::SeqCst);
    }
}

/// Called for every entity the game creates; every 16th one looks after the guardians.
pub fn on_create(manager: usize) {
    if !ANY.load(Ordering::Relaxed) || CREATIONS.fetch_add(1, Ordering::Relaxed) % REFRESH_EVERY != 0 {
        return;
    }
    refresh(manager);
}

/// Called with the work the game's creation events and the group tick already do: finds each guardian's
/// faction again (an entity can move in memory), puts the faction back if something changed it, and forgets a
/// guardian that is gone.
pub fn refresh(manager: usize) {
    if !ANY.load(Ordering::Relaxed) {
        return;
    }
    let Ok(mut list) = GUARDIANS.try_lock() else { return };
    let mut gone = Vec::new();
    for g in list.iter_mut() {
        if spawner::find_record(manager, g.id).is_none() {
            gone.push(g.id);
            continue;
        }
        let Some(addr) = faction_at(manager, g.id) else { continue };
        if addr != g.faction_at {
            log(&format!("entity {} moved: its faction is at {addr:#x} now, was {:#x}", g.id, g.faction_at));
            g.faction_at = addr;
        }
        let value = read_bytes(addr, 1).and_then(|b| b.first().copied());
        if value != Some(FACTION_VALUE) {
            log(&format!("entity {} had faction {value:?} again; set back to {FACTION_VALUE} (it died, changed side, or the game reset it)", g.id));
            spawner::write_bytes(addr, &[FACTION_VALUE]);
        }
    }
    for id in &gone {
        log(&format!("entity {id} is gone (killed or removed)"));
        if let Some(g) = list.iter().find(|g| g.id == *id) {
            if let Ok(mut dead) = DEAD.lock() {
                dead.push(g.post.zone.clone());
            }
            log(&format!("'{}' is not placed again until the server restarts", g.post.zone));
        }
    }
    list.retain(|g| !gone.contains(&g.id));
    if list.is_empty() {
        ANY.store(false, Ordering::SeqCst);
        if let Ok(mut known) = ASKED.lock() {
            known.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(team: u8) -> Post {
        Post { zone: "Keep".into(), team, x: 0.0, z: 0.0, radius: 50.0, auto: false }
    }

    fn guardian(at: usize, team: u8) -> Guardian {
        Guardian::new(at as u32, at, post(team))
    }

    fn near(_: &Guardian) -> bool {
        false
    }

    fn far(_: &Guardian) -> bool {
        true
    }

    fn teams(addr: usize) -> u8 {
        match addr {
            0x100 | 0x101 => 1,
            0x200 => 2,
            _ => 0,
        }
    }

    #[test]
    fn the_owning_team_is_a_friend_and_every_other_player_an_enemy() {
        let g = [guardian(0x1000, 1)];
        assert_eq!(decide(&g, (0x1000, 11), (0x100, 7), &teams, &near), Some(false));
        assert_eq!(decide(&g, (0x101, 8), (0x1000, 11), &teams, &near), Some(false));
        assert_eq!(decide(&g, (0x1000, 11), (0x200, 7), &teams, &near), Some(true));
        assert_eq!(decide(&g, (0x300, 7), (0x1000, 11), &teams, &near), Some(true));
        assert_eq!(decide(&g, (0x1000, 11), (0x300, 8), &teams, &near), Some(true));
    }

    #[test]
    fn creatures_and_unrelated_pairs_stay_with_the_game() {
        let g = [guardian(0x1000, 1)];
        assert_eq!(decide(&g, (0x1000, 11), (0x400, 0), &teams, &near), None);
        assert_eq!(decide(&g, (0x400, 0), (0x1000, 11), &teams, &near), None);
        assert_eq!(decide(&g, (0x100, 7), (0x200, 7), &teams, &near), None);
        assert_eq!(decide(&[], (0x1000, 11), (0x100, 7), &teams, &near), None);
    }

    #[test]
    fn guardians_of_one_team_are_friends_and_of_two_teams_enemies() {
        let g = [guardian(0x1000, 1), guardian(0x1100, 1), guardian(0x1200, 2)];
        assert_eq!(decide(&g, (0x1000, 11), (0x1100, 11), &teams, &near), Some(false));
        assert_eq!(decide(&g, (0x1000, 11), (0x1200, 11), &teams, &near), Some(true));
    }

    #[test]
    fn nobody_is_a_friend_of_a_guardian_without_a_team() {
        let g = [guardian(0x1000, 0)];
        assert_eq!(decide(&g, (0x1000, 11), (0x300, 7), &teams, &near), Some(true));
        assert_eq!(decide(&g, (0x1000, 11), (0x100, 7), &teams, &near), Some(true));
    }

    #[test]
    fn a_guardian_post_needs_an_enabled_zone_with_an_owner() {
        let zones = super::super::groups::parse_zones(
            r#"{"zones":[{"name":"Keep","x":10,"z":20,"radius":30,"team":2},{"name":"Open","x":0,"z":0,"radius":5},{"name":"Off","x":0,"z":0,"radius":5,"team":1,"enabled":false}]}"#,
        );
        let post = post_for(&zones, " keep ").unwrap();
        assert_eq!((post.zone.as_str(), post.team, post.x, post.z, post.radius), ("Keep", 2, 10.0, 20.0, 30.0));
        assert!(post_for(&zones, "Open").unwrap_err().contains("no owning team"));
        assert!(post_for(&zones, "Off").unwrap_err().contains("no zone"));
        assert!(post_for(&zones, "Nowhere").unwrap_err().contains("no zone"));
    }

    fn zone_post(zone: &str, team: u8) -> Post {
        Post { zone: zone.into(), ..post(team) }
    }

    #[test]
    fn two_zones_with_two_teams_each_keep_their_own_guardian() {
        let mut list = Vec::new();
        assert!(register(&mut list, Guardian::new(1, 0x1000, zone_post("Keep", 1))).is_none());
        assert!(register(&mut list, Guardian::new(2, 0x1100, zone_post("Tower", 2))).is_none());
        assert_eq!(list.len(), 2);
        let old = register(&mut list, Guardian::new(3, 0x1200, zone_post(" keep ", 1))).unwrap();
        assert_eq!(old.id, 1);
        let ids: Vec<u32> = list.iter().map(|g| g.id).collect();
        assert_eq!(ids, vec![2, 3]);
        assert_eq!(decide(&list, (0x1100, 11), (0x100, 7), &teams, &near), Some(true));
        assert_eq!(decide(&list, (0x1100, 11), (0x200, 7), &teams, &near), Some(false));
        assert_eq!(decide(&list, (0x1200, 11), (0x100, 7), &teams, &near), Some(false));
        assert_eq!(decide(&list, (0x1200, 11), (0x200, 7), &teams, &near), Some(true));
        assert_eq!(decide(&list, (0x1000, 11), (0x100, 7), &teams, &near), None);
    }

    #[test]
    fn a_guardian_outside_its_zone_names_no_target() {
        let g = [guardian(0x1000, 1)];
        assert_eq!(decide(&g, (0x1000, 11), (0x200, 7), &teams, &far), Some(false));
        assert_eq!(decide(&g, (0x1000, 11), (0x200, 7), &teams, &near), Some(true));
    }

    #[test]
    fn attacks_that_copy_the_guardian_side_are_decided_like_the_guardian() {
        let mut g = [guardian(0x1000, 1)];
        assert!(is_kin_candidate(&g, 0x5000, 11, &teams));
        assert!(!is_kin_candidate(&g, 0x5000, 7, &teams));
        assert!(!is_kin_candidate(&g, 0x100, 11, &teams));
        assert!(!is_kin_candidate(&g, 0x1000, 11, &teams));
        g[0].kin.push(0x5000);
        assert_eq!(decide(&g, (0x5000, 11), (0x100, 7), &teams, &near), Some(false));
        assert_eq!(decide(&g, (0x5000, 11), (0x200, 7), &teams, &near), Some(true));
    }

    #[test]
    fn only_the_guardian_of_a_changed_zone_is_taken_out() {
        let zones = |json: &str| super::super::groups::parse_zones(json);
        let mut list = vec![
            Guardian::new(1, 0x1000, zone_post("Keep", 1)),
            Guardian::new(2, 0x1100, zone_post("Tower", 2)),
            Guardian::new(3, 0x1200, zone_post("Gate", 1)),
        ];
        let now = zones(r#"{"zones":[{"name":"Keep","x":5,"z":6,"radius":70,"team":3},{"name":"Tower","x":5,"z":6,"radius":70,"team":2}]}"#);
        let orphans = follow(&mut list, &now);
        let ids: Vec<u32> = orphans.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![1, 3]);
        assert!(orphans[0].1.contains("belongs to team 3"));
        assert!(orphans[1].1.contains("gone"));
        assert_eq!((list[1].post.x, list[1].post.z, list[1].post.radius), (5.0, 6.0, 70.0));
    }

    fn saved(zone: &str) -> Saved {
        Saved { zone: zone.into(), template: "Enemy_Boss_Fog_Dragon".into(), level: 40, at: [1.0, 2.0, 3.0] }
    }

    #[test]
    fn a_saved_guardian_comes_back_once_when_somebody_stands_in_its_zone() {
        let zones = super::super::groups::parse_zones(
            r#"{"zones":[{"name":"Keep","x":0,"z":0,"radius":50,"team":1},{"name":"Tower","x":500,"z":0,"radius":50,"team":2},{"name":"Open","x":900,"z":0,"radius":50}]}"#,
        );
        let list = [saved("Keep"), saved("Tower"), saved("Open"), saved("Gone")];
        let nobody: Vec<(f64, f64)> = Vec::new();
        assert!(pick_due(&list, &[], &[], &[], &zones, &nobody).is_none());
        let far = [(2000.0, 0.0)];
        assert!(pick_due(&list, &[], &[], &[], &zones, &far).is_none());
        let in_keep = [(10.0, 10.0)];
        let (post, found) = pick_due(&list, &[], &[], &[], &zones, &in_keep).unwrap();
        assert_eq!((post.zone.as_str(), post.team, post.auto, found.template.as_str()), ("Keep", 1, true, "Enemy_Boss_Fog_Dragon"));
        assert!(pick_due(&list, &["keep".to_string()], &[], &[], &zones, &in_keep).is_none());
        assert!(pick_due(&list, &[], &["Keep".to_string()], &[], &zones, &in_keep).is_none());
        let asked = [("Keep".to_string(), Instant::now())];
        assert!(pick_due(&list, &[], &[], &asked, &zones, &in_keep).is_none());
        let both = [(10.0, 10.0), (500.0, 5.0)];
        let (second, _) = pick_due(&list, &["Keep".to_string()], &[], &[], &zones, &both).unwrap();
        assert_eq!((second.zone.as_str(), second.team), ("Tower", 2));
        assert!(pick_due(&list, &[], &[], &[], &zones, &[(900.0, 0.0)]).is_none());
    }

    #[test]
    fn the_saved_list_round_trips() {
        let list = vec![saved("Keep")];
        let text = serde_json::to_string(&list).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Saved>>(&text).unwrap(), list);
    }

    #[test]
    fn creatures_that_hurt_players_are_enemies_of_a_guardian_inside_its_zone_only() {
        assert_eq!(creature_relation(3, false), Some(true));
        assert_eq!(creature_relation(6, false), Some(true));
        assert_eq!(creature_relation(3, true), Some(false));
        assert_eq!(creature_relation(0, false), None);
        let g = [guardian(0x1000, 1)];
        assert_eq!(decide(&g, (0x1000, 11), (0x400, 3), &teams, &near), Some(true));
        assert_eq!(decide(&g, (0x400, 6), (0x1000, 11), &teams, &near), Some(true));
        assert_eq!(decide(&g, (0x1000, 11), (0x400, 3), &teams, &far), Some(false));
    }

    #[test]
    fn nothing_changes_while_no_guardian_exists() {
        ANY.store(false, Ordering::SeqCst);
        assert_eq!(relation(0x1000, 11, 0x100, 7, "isEnemy"), None);
        note_values(11, 7, "isEnemy");
        assert_eq!(VALUE_CALLS.load(Ordering::Relaxed), 0);
        refresh(0);
        on_create(0);
        zones_changed(0, &[]);
        assert!(due(&[], &[(0.0, 0.0)]).is_none());
        run_removals(0);
    }
}

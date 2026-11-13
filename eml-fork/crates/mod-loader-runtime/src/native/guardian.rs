//! Base guardian (trial): a creature an admin places for the team that owns a zone. It carries the faction
//! of the player group, so it is hostile to every creature, and the relation hooks decide the rest by the
//! address of its faction: it is a friend of the owning team (and of the teams `friends` names) and an
//! enemy of every other player. Its leash is the zone. Nothing here runs unless a guardian was placed:
//! the hooks return before looking at anything, and there is no timer of its own.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

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
const MAX_GUARDIANS: usize = 8;
const MAX_ASKED: usize = 24;

/// Where and for whom a guardian stands.
#[derive(Debug, Clone, PartialEq)]
pub struct Post {
    pub zone: String,
    pub team: u8,
    pub x: f64,
    pub z: f64,
    pub radius: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct Guardian {
    id: u32,
    faction_at: usize,
    post: Post,
}

static ANY: AtomicBool = AtomicBool::new(false);
static GUARDIANS: Mutex<Vec<Guardian>> = Mutex::new(Vec::new());
static ASKED: Mutex<Vec<(usize, bool)>> = Mutex::new(Vec::new());
static VALUE_CALLS: AtomicU32 = AtomicU32::new(0);

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
    Ok(Post { zone: zone.name.clone(), team: team as u8, x: zone.x, z: zone.z, radius: zone.radius })
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
/// `team_of` gives the team of the faction at an address.
fn decide(guardians: &[Guardian], a: (usize, u8), b: (usize, u8), team_of: &dyn Fn(usize) -> u8) -> Option<bool> {
    let of = |addr: usize| guardians.iter().find(|g| g.faction_at == addr);
    match (of(a.0), of(b.0)) {
        (None, None) => None,
        (Some(g), Some(h)) => Some(!friends(g.post.team, h.post.team)),
        (Some(g), None) => other(g, b, team_of),
        (None, Some(g)) => other(g, a, team_of),
    }
}

fn other(guardian: &Guardian, other: (usize, u8), team_of: &dyn Fn(usize) -> u8) -> Option<bool> {
    if !player_group(other.1) {
        return None;
    }
    Some(!friends(guardian.post.team, team_of(other.0)))
}

/// The hook point of the relation functions that look at faction addresses. `None` unless the pair involves a
/// guardian.
pub fn relation(a: usize, fa: u8, b: usize, fb: u8, asked: &str) -> Option<bool> {
    if !ANY.load(Ordering::Relaxed) {
        return None;
    }
    let guardians = GUARDIANS.try_lock().ok()?;
    let hostile = decide(&guardians, (a, fa), (b, fb), &groups::team_of)?;
    let (g, other, value) = match guardians.iter().find(|g| g.faction_at == a) {
        Some(g) => (g.clone(), b, fb),
        None => (guardians.iter().find(|g| g.faction_at == b)?.clone(), a, fa),
    };
    drop(guardians);
    say_relation(&g, other, value, asked, hostile);
    Some(hostile)
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
    if list.len() >= MAX_GUARDIANS {
        list.remove(0);
    }
    log(&format!(
        "entity {id} guards '{}' for team {}: faction {before} -> {FACTION_VALUE} at {addr:#x} (component slot {slot}, table says {:?})",
        post.zone,
        post.team,
        shield::component_index(manager, HASH_FACTION)
    ));
    list.push(Guardian { id, faction_at: addr, post });
    ANY.store(true, Ordering::SeqCst);
    true
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
        Post { zone: "Keep".into(), team, x: 0.0, z: 0.0, radius: 50.0 }
    }

    fn guardian(at: usize, team: u8) -> Guardian {
        Guardian { id: at as u32, faction_at: at, post: post(team) }
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
        assert_eq!(decide(&g, (0x1000, 11), (0x100, 7), &teams), Some(false));
        assert_eq!(decide(&g, (0x101, 8), (0x1000, 11), &teams), Some(false));
        assert_eq!(decide(&g, (0x1000, 11), (0x200, 7), &teams), Some(true));
        assert_eq!(decide(&g, (0x300, 7), (0x1000, 11), &teams), Some(true));
        assert_eq!(decide(&g, (0x1000, 11), (0x300, 8), &teams), Some(true));
    }

    #[test]
    fn creatures_and_unrelated_pairs_stay_with_the_game() {
        let g = [guardian(0x1000, 1)];
        assert_eq!(decide(&g, (0x1000, 11), (0x400, 3), &teams), None);
        assert_eq!(decide(&g, (0x400, 3), (0x1000, 11), &teams), None);
        assert_eq!(decide(&g, (0x100, 7), (0x200, 7), &teams), None);
        assert_eq!(decide(&[], (0x1000, 11), (0x100, 7), &teams), None);
    }

    #[test]
    fn guardians_of_one_team_are_friends_and_of_two_teams_enemies() {
        let g = [guardian(0x1000, 1), guardian(0x1100, 1), guardian(0x1200, 2)];
        assert_eq!(decide(&g, (0x1000, 11), (0x1100, 11), &teams), Some(false));
        assert_eq!(decide(&g, (0x1000, 11), (0x1200, 11), &teams), Some(true));
    }

    #[test]
    fn nobody_is_a_friend_of_a_guardian_without_a_team() {
        let g = [guardian(0x1000, 0)];
        assert_eq!(decide(&g, (0x1000, 11), (0x300, 7), &teams), Some(true));
        assert_eq!(decide(&g, (0x1000, 11), (0x100, 7), &teams), Some(true));
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

    #[test]
    fn nothing_changes_while_no_guardian_exists() {
        ANY.store(false, Ordering::SeqCst);
        assert_eq!(relation(0x1000, 11, 0x100, 7, "isEnemy"), None);
        note_values(11, 7, "isEnemy");
        assert_eq!(VALUE_CALLS.load(Ordering::Relaxed), 0);
        refresh(0);
    }
}

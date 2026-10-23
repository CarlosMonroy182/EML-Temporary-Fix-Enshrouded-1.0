use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use serde::Deserialize;

pub const ZONES_FILE: &str = "m182_zones.json";
pub const TEAMS_FILE: &str = "m182_teams.json";
pub const PLAYERS_FILE: &str = "m182_players.json";

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Zone {
    pub name: String,
    pub x: f64,
    pub z: f64,
    pub radius: f64,
    pub enabled: Option<bool>,
    /// Enemies cannot hurt anyone inside (the default for zones without this field).
    pub safe: Option<bool>,
    /// Team that owns the place (1-based, as in the teams file); 0 or missing = nobody.
    pub team: Option<u32>,
    /// Anybody outside the owning team is a visitor: no chests, no building.
    pub lock: Option<bool>,
    /// Also restrict administrators (to try it out yourself).
    pub lock_admins: Option<bool>,
}

impl Zone {
    pub fn is_safe(&self) -> bool {
        self.safe.unwrap_or(true)
    }

    pub fn owner(&self) -> u32 {
        self.team.unwrap_or(0)
    }

    pub fn locks(&self) -> bool {
        self.lock.unwrap_or(false)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ZonesFile {
    zones: Vec<Zone>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Team {
    name: String,
    leaders: Vec<String>,
    members: Vec<u32>,
    names: Vec<String>,
    people: Vec<Person>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Person {
    name: String,
    key: String,
}

/// A team as the native layer needs it: the old player numbers plus the names of its people.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TeamDef {
    pub name: String,
    pub leaders: Vec<String>,
    pub members: Vec<u32>,
    pub names: Vec<String>,
    pub ids: Vec<String>,
    /// Panel edits per person (lowercase name or key -> counter): chat changes recorded before them lose.
    pub touched: Vec<(String, u64)>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TeamsFile {
    teams: Vec<Team>,
    touched: std::collections::BTreeMap<String, u64>,
}

impl TeamDef {
    /// The panel counter of a person (0 = the panel never edited them).
    pub fn touch_of(&self, id: &str, name: &str) -> u64 {
        let (id, name) = (id.trim().to_lowercase(), name.trim().to_lowercase());
        self.touched
            .iter()
            .filter(|(k, _)| (!name.is_empty() && (*k == name || *k == format!("name:{name}"))) || (!id.is_empty() && (*k == id || k.ends_with(&format!(":{id}")))))
            .map(|(_, v)| *v)
            .max()
            .unwrap_or(0)
    }
}

pub fn parse_zones(text: &str) -> Vec<Zone> {
    let Ok(file) = serde_json::from_str::<ZonesFile>(text) else { return Vec::new() };
    file.zones
        .into_iter()
        .filter(|z| z.enabled.unwrap_or(true) && z.x.is_finite() && z.z.is_finite() && z.radius.is_finite() && z.radius > 0.0 && z.radius <= 5000.0)
        .collect()
}

pub fn inside(zones: &[Zone], x: f64, z: f64) -> bool {
    zones.iter().any(|zone| {
        let dx = x - zone.x;
        let dz = z - zone.z;
        dx * dx + dz * dz <= zone.radius * zone.radius
    })
}

pub fn parse_team_defs(text: &str) -> Vec<TeamDef> {
    let Ok(file) = serde_json::from_str::<TeamsFile>(text) else { return Vec::new() };
    let touched: Vec<(String, u64)> = file.touched.iter().map(|(k, v)| (k.trim().to_lowercase(), *v)).collect();
    file.teams
        .into_iter()
        .map(|t| {
            let mut names = t.names;
            let ids: Vec<String> = t.people.iter().filter(|p| p.key.starts_with("steamid:")).map(|p| p.key.trim().to_lowercase()).collect();
            for p in &t.people {
                if let Some(n) = p.key.strip_prefix("slot:").and_then(|n| n.trim().parse::<u32>().ok()) {
                    names.push(format!("ID {n}"));
                }
            }
            names.extend(t.people.into_iter().map(|p| p.name));
            names.retain(|n| !n.trim().is_empty());
            let leaders = t.leaders.into_iter().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect();
            TeamDef { name: t.name, leaders, members: t.members, names, ids, touched: touched.clone() }
        })
        .collect()
}

/// One person of the invitation file (`m182_members.json`): who joined or left which team by chat.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Member {
    pub team: String,
    pub id: String,
    pub name: String,
    /// The panel counter of this person when the change was made.
    pub touch: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct MembersFile {
    pub joined: Vec<Member>,
    pub left: Vec<Member>,
}

pub const MEMBERS_FILE: &str = "m182_members.json";

/// Puts the people who joined by invitation into their teams and takes out those who left,
/// on top of what the admin's panel defined. Teams are found by name.
pub fn apply_members(teams: &mut [TeamDef], text: &str) {
    let Ok(file) = serde_json::from_str::<MembersFile>(text) else { return };
    let find = |teams: &mut [TeamDef], name: &str| teams.iter().position(|t| !t.name.trim().is_empty() && super::roster::same_name(&t.name, name));
    for m in &file.left {
        if let Some(i) = find(teams, &m.team) {
            let t = &mut teams[i];
            if t.touch_of(&m.id, &m.name) > m.touch {
                continue;
            }
            let id = m.id.trim().to_lowercase();
            if !id.is_empty() {
                t.ids.retain(|x| *x != id);
            }
            if !m.name.trim().is_empty() {
                t.names.retain(|n| !super::roster::same_name(n, &m.name));
            }
        }
    }
    for m in &file.joined {
        if let Some(i) = find(teams, &m.team) {
            let t = &mut teams[i];
            if t.touch_of(&m.id, &m.name) > m.touch {
                continue;
            }
            let id = m.id.trim().to_lowercase();
            if !id.is_empty() && !t.ids.contains(&id) {
                t.ids.push(id);
            }
            if !m.name.trim().is_empty() && !t.names.iter().any(|n| super::roster::same_name(n, &m.name)) {
                t.names.push(m.name.trim().to_string());
            }
        }
    }
}

/// Team (1-based) of a connected player; 0 when in none. Names win over numbers because
/// numbers change every time somebody reconnects.
pub fn team_for(teams: &[TeamDef], number: u32, name: Option<&str>, entity: u32) -> u8 {
    for (index, team) in teams.iter().enumerate() {
        let by_name = name.is_some_and(|n| team.names.iter().any(|t| super::roster::same_name(t, n)))
            || team.names.iter().any(|t| super::roster::placeholder_number(t) == Some(entity));
        if by_name {
            return (index + 1).min(255) as u8;
        }
    }
    let named = teams.iter().any(|t| !t.names.is_empty());
    if !named {
        for (index, team) in teams.iter().enumerate() {
            if team.members.contains(&number) {
                return (index + 1).min(255) as u8;
            }
        }
    }
    0
}

/// Like `team_for`, but a platform id read from the server's player table decides first.
pub fn team_for_player(teams: &[TeamDef], number: u32, name: Option<&str>, entity: u32, id: Option<&str>) -> u8 {
    if let Some(id) = id {
        let id = id.trim().to_lowercase();
        if let Some(index) = teams.iter().position(|t| t.ids.contains(&id)) {
            return (index + 1).min(255) as u8;
        }
    }
    team_for(teams, number, name, entity)
}

/// The locked, owned zone that rules at a point. Where several overlap, the smallest one wins
/// (a small base inside a big town), and on equal sizes the one whose centre is nearer.
pub fn ruling_zone(zones: &[Zone], x: f64, z: f64) -> Option<&Zone> {
    let dist = |zone: &Zone| ((x - zone.x).powi(2) + (z - zone.z).powi(2)).sqrt();
    zones
        .iter()
        .filter(|zone| zone.locks() && zone.owner() != 0 && inside(std::slice::from_ref(zone), x, z))
        .min_by(|a, b| a.radius.total_cmp(&b.radius).then(dist(a).total_cmp(&dist(b))))
}

pub fn parse_teams(text: &str) -> Vec<Vec<u32>> {
    let Ok(file) = serde_json::from_str::<TeamsFile>(text) else { return Vec::new() };
    file.teams.into_iter().map(|t| t.members).collect()
}

pub fn team_of_player(teams: &[Vec<u32>], number: u32) -> u8 {
    for (index, members) in teams.iter().enumerate() {
        if members.contains(&number) {
            return (index + 1).min(255) as u8;
        }
    }
    0
}

static TEAM_MAP: Mutex<Vec<(usize, u8)>> = Mutex::new(Vec::new());

pub fn set_team_map(map: Vec<(usize, u8)>) {
    if let Ok(mut m) = TEAM_MAP.lock() {
        *m = map;
    }
}

pub fn team_of(addr: usize) -> u8 {
    if addr == 0 {
        return 0;
    }
    let Ok(map) = TEAM_MAP.try_lock() else { return 0 };
    map.iter().find(|(a, _)| *a == addr).map(|(_, t)| *t).unwrap_or(0)
}

static PROTECTED: Mutex<Vec<usize>> = Mutex::new(Vec::new());

pub fn set_protected(addrs: Vec<usize>) {
    if let Ok(mut p) = PROTECTED.lock() {
        *p = addrs;
    }
}

pub fn is_protected(addr: usize) -> bool {
    if addr == 0 {
        return false;
    }
    let Ok(p) = PROTECTED.try_lock() else { return false };
    p.contains(&addr)
}

static PROTECTED_REPORTS: AtomicU32 = AtomicU32::new(0);

pub fn protected_report(a: usize, b: usize, fa: u8, fb: u8) -> Option<String> {
    if PROTECTED_REPORTS.fetch_add(1, Ordering::Relaxed) >= 40 {
        return None;
    }
    Some(format!("zones: hostile check blocked a={a:#x} faction={fa} protected={} b={b:#x} faction={fb} protected={}", is_protected(a), is_protected(b)))
}

static PAIR_REPORTS: AtomicU32 = AtomicU32::new(0);

pub fn pair_report(a: usize, b: usize) -> Option<String> {
    if PAIR_REPORTS.fetch_add(1, Ordering::Relaxed) >= 40 {
        return None;
    }
    Some(format!("teams: player pair a={a:#x} team={} b={b:#x} team={}", team_of(a), team_of(b)))
}

pub fn same_team(a: usize, b: usize) -> bool {
    let ta = team_of(a);
    ta != 0 && ta == team_of(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_parse_and_contain() {
        let zones = parse_zones(r#"{"zones":[{"name":"Base","x":100,"z":-50,"radius":30},{"name":"off","x":0,"z":0,"radius":10,"enabled":false},{"name":"bad","x":0,"z":0,"radius":-1}]}"#);
        assert_eq!(zones.len(), 1);
        assert!(inside(&zones, 100.0, -50.0));
        assert!(inside(&zones, 120.0, -30.0));
        assert!(!inside(&zones, 140.0, -50.0));
        assert!(parse_zones("nonsense").is_empty());
    }

    #[test]
    fn overlapping_locked_zones_have_one_ruler() {
        let zones = parse_zones(r#"{"zones":[{"name":"Base","x":2993.62,"z":1622.21,"radius":40,"team":2,"lock":true},{"name":"Mercado","x":2995.19,"z":1607.67,"radius":40,"team":1,"lock":true}]}"#);
        assert_eq!(ruling_zone(&zones, 2993.0, 1630.0).map(|z| z.owner()), Some(2));
        assert_eq!(ruling_zone(&zones, 2995.0, 1600.0).map(|z| z.owner()), Some(1));
        assert!(ruling_zone(&zones, 3500.0, 1600.0).is_none());
        let nested = parse_zones(r#"{"zones":[{"name":"Town","x":0,"z":0,"radius":100,"team":1,"lock":true},{"name":"Casa","x":10,"z":0,"radius":10,"team":2,"lock":true}]}"#);
        assert_eq!(ruling_zone(&nested, 12.0, 0.0).map(|z| z.owner()), Some(2));
        assert_eq!(ruling_zone(&nested, -50.0, 0.0).map(|z| z.owner()), Some(1));
    }

    #[test]
    fn teams_number_members() {
        let teams = parse_teams(r#"{"teams":[{"name":"A","members":[1,3]},{"name":"B","members":[2]}]}"#);
        assert_eq!(team_of_player(&teams, 1), 1);
        assert_eq!(team_of_player(&teams, 3), 1);
        assert_eq!(team_of_player(&teams, 2), 2);
        assert_eq!(team_of_player(&teams, 4), 0);
    }

    #[test]
    fn zones_know_their_owner_and_lock() {
        let zones = parse_zones(r#"{"zones":[{"name":"Base","x":0,"z":0,"radius":30,"team":2,"lock":true},{"name":"Vieja","x":5,"z":5,"radius":3}]}"#);
        assert_eq!(zones[0].owner(), 2);
        assert!(zones[0].locks() && zones[0].is_safe());
        assert_eq!(zones[1].owner(), 0);
        assert!(!zones[1].locks() && zones[1].is_safe());
        let open = parse_zones(r#"{"zones":[{"name":"Mia","x":0,"z":0,"radius":10,"safe":false,"team":1,"lock":true,"lock_admins":true}]}"#);
        assert!(!open[0].is_safe());
        assert_eq!(open[0].lock_admins, Some(true));
    }

    #[test]
    fn teams_match_people_by_name_before_numbers() {
        let teams = parse_team_defs(r#"{"teams":[{"name":"Rojo","members":[1],"people":[{"key":"a","name":"Ana"},{"key":"b","name":"Jugador 4"}]},{"name":"Azul","members":[2],"people":[{"key":"c","name":"luis"}]}]}"#);
        assert_eq!(team_for(&teams, 9, Some("ANA"), 5), 1);
        assert_eq!(team_for(&teams, 9, Some("Luis"), 6), 2);
        assert_eq!(team_for(&teams, 1, Some("Pedro"), 7), 0, "a stale number must not put a stranger in a team");
        assert_eq!(team_for(&teams, 1, None, 4), 1, "placeholder names stand for the entity number");
        assert_eq!(team_for(&teams, 1, Some("Pedro"), 4), 1, "a placeholder holds the place even when the real name is known");
        let numbers = parse_team_defs(r#"{"teams":[{"members":[1,3]},{"members":[2]}]}"#);
        assert_eq!(team_for(&numbers, 3, None, 9), 1);
        assert_eq!(team_for(&numbers, 2, Some("x"), 9), 2);
        assert_eq!(team_for(&numbers, 4, None, 9), 0);
    }

    #[test]
    fn a_player_kept_by_slot_key_is_found_by_entity_or_by_name() {
        let teams = parse_team_defs(r#"{"teams":[{"name":"M182","members":[1],"people":[{"key":"slot:1","name":"ID 1"}]},{"name":"B","people":[{"key":"slot:3","name":"moradoox"}]}]}"#);
        assert_eq!(team_for(&teams, 9, Some("moradoox"), 1), 1);
        assert_eq!(team_for(&teams, 9, Some("moradoox"), 5), 2);
        assert_eq!(team_for(&teams, 9, Some("moradoox"), 3), 2);
        assert_eq!(team_for(&teams, 9, Some("otro"), 4), 0);
    }

    #[test]
    fn a_platform_id_places_a_player_whatever_the_name_says() {
        let teams = parse_team_defs(r#"{"teams":[{"name":"Rojo","people":[{"key":"steamid:76561199474170255","name":"Nacht"}]},{"name":"Azul","people":[{"key":"name:luis","name":"luis"}]}]}"#);
        assert_eq!(team_for_player(&teams, 9, None, 5, Some("steamid:76561199474170255")), 1);
        assert_eq!(team_for_player(&teams, 9, Some("Otro"), 5, Some("STEAMID:76561199474170255")), 1);
        assert_eq!(team_for_player(&teams, 9, Some("Luis"), 5, Some("steamid:1")), 2);
        assert_eq!(team_for_player(&teams, 9, Some("Nadie"), 5, None), 0);
    }

    #[test]
    fn invited_people_join_and_left_people_go() {
        let mut teams = parse_team_defs(r#"{"teams":[{"name":"Rojo","leaders":["steamid:1"],"people":[{"key":"steamid:1","name":"Ana"},{"key":"steamid:2","name":"Luis"}]},{"name":"Azul","people":[]}]}"#);
        assert_eq!(teams[0].leaders, vec!["steamid:1".to_string()]);
        apply_members(&mut teams, r#"{"joined":[{"team":"azul","id":"STEAMID:9","name":"Pedro"}],"left":[{"team":"Rojo","id":"steamid:2","name":"luis"}]}"#);
        assert_eq!(team_for_player(&teams, 99, Some("Pedro"), 5, Some("steamid:9")), 2);
        assert_eq!(team_for_player(&teams, 99, Some("Luis"), 6, Some("steamid:2")), 0);
        assert_eq!(team_for_player(&teams, 99, Some("Ana"), 7, Some("steamid:1")), 1);
        apply_members(&mut teams, "not json");
    }

    #[test]
    fn protected_addresses() {
        set_protected(vec![0x10, 0x20]);
        assert!(is_protected(0x10));
        assert!(!is_protected(0x30));
        assert!(!is_protected(0));
        set_protected(Vec::new());
        assert!(!is_protected(0x10));
    }

    #[test]
    fn team_map_matches_addresses() {
        set_team_map(vec![(0x1000, 1), (0x2000, 1), (0x3000, 2)]);
        assert!(same_team(0x1000, 0x2000));
        assert!(!same_team(0x1000, 0x3000));
        assert!(!same_team(0x1000, 0x9000));
        assert!(!same_team(0, 0));
        set_team_map(Vec::new());
    }
}

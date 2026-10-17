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
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ZonesFile {
    zones: Vec<Zone>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Team {
    members: Vec<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TeamsFile {
    teams: Vec<Team>,
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
    fn teams_number_members() {
        let teams = parse_teams(r#"{"teams":[{"name":"A","members":[1,3]},{"name":"B","members":[2]}]}"#);
        assert_eq!(team_of_player(&teams, 1), 1);
        assert_eq!(team_of_player(&teams, 3), 1);
        assert_eq!(team_of_player(&teams, 2), 2);
        assert_eq!(team_of_player(&teams, 4), 0);
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

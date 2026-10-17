use std::path::Path;
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText, Vec2};
use serde_json::{json, Value};

use crate::theme::*;
use crate::{write_atomic, App};

pub const ZONES_FILE: &str = "m182_zones.json";
pub const TEAMS_FILE: &str = "m182_teams.json";
pub const PLAYERS_FILE: &str = "m182_players.json";

#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub name: String,
    pub x: f64,
    pub z: f64,
    pub radius: f64,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Team {
    pub name: String,
    pub members: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Online {
    pub number: u32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub me: bool,
}

pub fn player_label(p: &Online) -> String {
    if p.me {
        format!("Tú (jugador {})", p.number)
    } else {
        format!("Jugador {}", p.number)
    }
}

pub struct SocialState {
    pub zones: Vec<Zone>,
    pub teams: Vec<Team>,
    pub online: Vec<Online>,
    pub checked: Instant,
    pub zone_name: String,
    pub zone_radius: f64,
    pub team_name: String,
    pub notice: String,
}

impl Default for SocialState {
    fn default() -> Self {
        Self {
            zones: Vec::new(),
            teams: Vec::new(),
            online: Vec::new(),
            checked: Instant::now(),
            zone_name: String::new(),
            zone_radius: 40.0,
            team_name: String::new(),
            notice: String::new(),
        }
    }
}

pub fn parse_zones(text: &str) -> Vec<Zone> {
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let Some(Value::Array(list)) = root.get("zones") else { return Vec::new() };
    list.iter()
        .filter_map(|v| {
            Some(Zone {
                name: v.get("name")?.as_str()?.to_string(),
                x: v.get("x")?.as_f64()?,
                z: v.get("z")?.as_f64()?,
                radius: v.get("radius")?.as_f64()?,
                enabled: v.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true),
            })
        })
        .collect()
}

pub fn zones_json(zones: &[Zone]) -> Value {
    json!({
        "zones": zones.iter().map(|z| json!({"name": z.name, "x": z.x, "z": z.z, "radius": z.radius, "enabled": z.enabled})).collect::<Vec<_>>()
    })
}

pub fn parse_teams(text: &str) -> Vec<Team> {
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let Some(Value::Array(list)) = root.get("teams") else { return Vec::new() };
    list.iter()
        .filter_map(|v| {
            let members = v.get("members")?.as_array()?.iter().filter_map(|m| m.as_u64()).map(|m| m as u32).collect();
            Some(Team { name: v.get("name")?.as_str()?.to_string(), members })
        })
        .collect()
}

pub fn teams_json(teams: &[Team]) -> Value {
    json!({
        "teams": teams.iter().map(|t| json!({"name": t.name, "members": t.members})).collect::<Vec<_>>()
    })
}

pub fn parse_online(text: &str) -> Vec<Online> {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let mut out: Vec<Online> = list
        .iter()
        .filter_map(|v| {
            Some(Online {
                number: v.get("n")?.as_u64()? as u32,
                x: v.get("x")?.as_f64()?,
                y: v.get("y")?.as_f64()?,
                z: v.get("z")?.as_f64()?,
                me: v.get("me").and_then(|m| m.as_bool()).unwrap_or(false),
            })
        })
        .collect();
    out.sort_by_key(|p| p.number);
    out
}

pub fn zone_at<'a>(zones: &'a [Zone], x: f64, z: f64) -> Option<&'a Zone> {
    zones.iter().find(|zone| zone.enabled && (x - zone.x).powi(2) + (z - zone.z).powi(2) <= zone.radius * zone.radius)
}

pub fn set_member(teams: &mut [Team], team: usize, number: u32, member: bool) {
    for (i, t) in teams.iter_mut().enumerate() {
        t.members.retain(|m| *m != number);
        if member && i == team {
            t.members.push(number);
            t.members.sort_unstable();
        }
    }
}

impl SocialState {
    pub fn load(dir: &Path) -> Self {
        let mut s = Self::default();
        if let Ok(text) = std::fs::read_to_string(dir.join(ZONES_FILE)) {
            s.zones = parse_zones(&text);
        }
        if let Ok(text) = std::fs::read_to_string(dir.join(TEAMS_FILE)) {
            s.teams = parse_teams(&text);
        }
        s.refresh(dir);
        s
    }

    pub fn refresh(&mut self, dir: &Path) {
        self.checked = Instant::now();
        self.online = std::fs::read_to_string(dir.join(PLAYERS_FILE)).map(|t| parse_online(&t)).unwrap_or_default();
    }

    pub fn save_zones(&mut self, dir: &Path) {
        self.notice = match write_atomic(dir, ZONES_FILE, "m182_zones.json.tmp", &zones_json(&self.zones).to_string()) {
            Ok(()) => "Guardado. El juego lo aplica en un par de segundos.".to_string(),
            Err(e) => format!("Error: {e}"),
        };
    }

    pub fn save_teams(&mut self, dir: &Path) {
        self.notice = match write_atomic(dir, TEAMS_FILE, "m182_teams.json.tmp", &teams_json(&self.teams).to_string()) {
            Ok(()) => "Guardado. El juego lo aplica en un par de segundos.".to_string(),
            Err(e) => format!("Error: {e}"),
        };
    }
}

impl App {
    pub fn social_tick(&mut self) {
        if self.social.checked.elapsed() >= Duration::from_millis(1000) {
            let dir = self.dir.clone();
            self.social.refresh(&dir);
        }
    }

    fn add_zone(&mut self, x: f64, z: f64) {
        let name = if self.social.zone_name.trim().is_empty() {
            format!("Zona {}", self.social.zones.len() + 1)
        } else {
            self.social.zone_name.trim().to_string()
        };
        self.social.zones.push(Zone { name, x, z, radius: self.social.zone_radius, enabled: true });
        self.social.zone_name.clear();
        let dir = self.dir.clone();
        self.social.save_zones(&dir);
    }

    pub fn zones_tab(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Zonas seguras", 22.0);
        ui.label(RichText::new("Dentro de una zona nadie pierde vida: ni por monstruos ni por otros jugadores. Los cofres no se pueden proteger.").color(MUTED));
        divider(ui);
        match self.player {
            Some(p) => {
                ui.label(format!("Tu posición:  X {:.0}   Z {:.0}", p[0], p[2]));
                if let Some(zone) = zone_at(&self.social.zones, p[0], p[2]) {
                    ui.label(RichText::new(format!("Estás dentro de «{}»", zone.name)).color(GOOD));
                }
            }
            None => {
                ui.label(RichText::new("Sin posición: abre el juego y carga tu mundo.").color(MUTED));
            }
        }
        label_title(ui, "NUEVA ZONA");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.social.zone_name).hint_text("Nombre").desired_width(220.0));
            ui.label("Radio (m)");
            ui.add(egui::Slider::new(&mut self.social.zone_radius, 5.0..=500.0).fixed_decimals(0));
        });
        ui.horizontal_wrapped(|ui| {
            if button(ui, "Crear donde estoy", Vec2::new(190.0, 32.0), self.player.is_some(), true) {
                if let Some(p) = self.player {
                    self.add_zone(p[0], p[2]);
                }
            }
            if let Some(p) = self.map.selected {
                if button(ui, "Crear en el punto del mapa", Vec2::new(230.0, 32.0), true, false) {
                    self.add_zone(p[0], p[2]);
                }
            }
            if !self.places.is_empty() {
                let mut picked = None;
                egui::ComboBox::from_id_salt("zone_place").selected_text("Crear en un lugar guardado…").show_ui(ui, |ui| {
                    for (i, p) in self.places.iter().enumerate() {
                        if ui.selectable_label(false, &p.name).clicked() {
                            picked = Some(i);
                        }
                    }
                });
                if let Some(i) = picked {
                    let (x, z) = (self.places[i].x, self.places[i].z);
                    if self.social.zone_name.trim().is_empty() {
                        self.social.zone_name = self.places[i].name.clone();
                    }
                    self.add_zone(x, z);
                }
            }
        });
        divider(ui);
        let mut changed = false;
        let mut remove = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, z) in self.social.zones.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut z.enabled, "").changed();
                    ui.label(RichText::new(&z.name).font(title_font(15.0)).color(PARCHMENT));
                    ui.label(RichText::new(format!("X {:.0}  Z {:.0}", z.x, z.z)).color(MUTED));
                    ui.label("Radio");
                    changed |= ui.add(egui::DragValue::new(&mut z.radius).speed(1.0).range(5.0..=2000.0).fixed_decimals(0)).changed();
                    if button(ui, "Borrar", Vec2::new(80.0, 26.0), true, false) {
                        remove = Some(i);
                    }
                });
            }
            if self.social.zones.is_empty() {
                ui.label(RichText::new("Aún no hay zonas seguras.").color(MUTED));
            }
        });
        if let Some(i) = remove {
            self.social.zones.remove(i);
            changed = true;
        }
        if changed {
            let dir = self.dir.clone();
            self.social.save_zones(&dir);
        }
        if !self.social.notice.is_empty() {
            ui.label(RichText::new(&self.social.notice).color(GOOD));
        }
    }

    pub fn teams_tab(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Equipos", 22.0);
        ui.label(RichText::new("Los jugadores del mismo equipo no se pueden dañar entre sí cuando el PvP está activo. Los cofres no se pueden proteger.").color(MUTED));
        divider(ui);
        let mut pvp_changed = false;
        pvp_changed |= ui.checkbox(&mut self.pvp, "PvP activo (necesario para que los equipos tengan efecto)").changed();
        if pvp_changed {
            self.notice = match self.set_config(&[("pvp", json!(self.pvp)), ("pvp_friend_checks", json!(self.pvp && self.pvp_friends))]) {
                Ok(()) => "PvP guardado.".to_string(),
                Err(e) => format!("Error: {e}"),
            };
        }
        divider(ui);
        label_title(ui, "JUGADORES EN EL MUNDO");
        if self.social.online.is_empty() {
            ui.label(RichText::new("Sin datos: abre el juego y carga el mundo.").color(MUTED));
        }
        for p in &self.social.online {
            let team = self.social.teams.iter().find(|t| t.members.contains(&p.number)).map(|t| t.name.clone()).unwrap_or_else(|| "sin equipo".to_string());
            let text = format!("{}    X {:.0}  Z {:.0}    {}", player_label(p), p.x, p.z, team);
            if p.me {
                ui.label(RichText::new(text).color(GOLD));
            } else {
                ui.label(text);
            }
        }
        ui.label(RichText::new("Solo aparecen los jugadores conectados ahora mismo. Para saber quién es quién, compara las coordenadas con las que ves en el juego. Los números se reinician cada vez que abres el mundo.").color(MUTED).size(13.0));
        divider(ui);

        let online: Vec<Online> = self.social.online.clone();
        let mut numbers: Vec<u32> = online.iter().map(|p| p.number).chain(self.social.teams.iter().flat_map(|t| t.members.iter().copied())).collect();
        numbers.sort_unstable();
        numbers.dedup();
        let mut changed = false;
        let mut remove = None;
        let mut membership: Vec<(usize, u32, bool)> = Vec::new();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, team) in self.social.teams.iter_mut().enumerate() {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        changed |= ui.add(egui::TextEdit::singleline(&mut team.name).desired_width(200.0)).changed();
                        if button(ui, "Borrar equipo", Vec2::new(130.0, 26.0), true, false) {
                            remove = Some(i);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        for n in numbers.iter().copied() {
                            let mut on = team.members.contains(&n);
                            let label = match online.iter().find(|p| p.number == n) {
                                Some(p) => player_label(p),
                                None => format!("Jugador {n} (desconectado)"),
                            };
                            if ui.checkbox(&mut on, label).changed() {
                                membership.push((i, n, on));
                            }
                        }
                        if numbers.is_empty() {
                            ui.label(RichText::new("No hay jugadores conectados todavía.").color(MUTED));
                        }
                    });
                });
            }
        });
        for (team, number, on) in membership {
            set_member(&mut self.social.teams, team, number, on);
            changed = true;
        }
        if let Some(i) = remove {
            self.social.teams.remove(i);
            changed = true;
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.social.team_name).hint_text("Nombre del equipo").desired_width(220.0));
            if button(ui, "Crear equipo", Vec2::new(150.0, 30.0), true, true) {
                let name = if self.social.team_name.trim().is_empty() {
                    format!("Equipo {}", self.social.teams.len() + 1)
                } else {
                    self.social.team_name.trim().to_string()
                };
                self.social.teams.push(Team { name, members: Vec::new() });
                self.social.team_name.clear();
                changed = true;
            }
        });
        if changed {
            let dir = self.dir.clone();
            self.social.save_teams(&dir);
        }
        if !self.social.notice.is_empty() {
            ui.label(RichText::new(&self.social.notice).color(GOOD));
        }
        if !self.notice.is_empty() {
            ui.label(RichText::new(&self.notice).color(GOOD));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_roundtrip_and_lookup() {
        let zones = vec![Zone { name: "Base".into(), x: 100.0, z: -50.0, radius: 30.0, enabled: true }, Zone { name: "Off".into(), x: 0.0, z: 0.0, radius: 10.0, enabled: false }];
        let back = parse_zones(&zones_json(&zones).to_string());
        assert_eq!(back, zones);
        assert_eq!(zone_at(&back, 110.0, -40.0).map(|z| z.name.as_str()), Some("Base"));
        assert!(zone_at(&back, 0.0, 0.0).is_none());
        assert!(parse_zones("basura").is_empty());
    }

    #[test]
    fn teams_are_exclusive_and_roundtrip() {
        let mut teams = vec![Team { name: "A".into(), members: vec![1] }, Team { name: "B".into(), members: vec![] }];
        set_member(&mut teams, 1, 1, true);
        assert!(teams[0].members.is_empty());
        assert_eq!(teams[1].members, vec![1]);
        set_member(&mut teams, 1, 3, true);
        set_member(&mut teams, 1, 2, true);
        assert_eq!(teams[1].members, vec![1, 2, 3]);
        set_member(&mut teams, 1, 2, false);
        assert_eq!(teams[1].members, vec![1, 3]);
        assert_eq!(parse_teams(&teams_json(&teams).to_string()), teams);
    }

    #[test]
    fn online_players_file() {
        let list = parse_online(r#"[{"n":2,"x":1.5,"y":3.0,"z":-4.0},{"n":1,"x":10.0,"y":0.0,"z":5.0}]"#);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].number, 1);
        let me = parse_online(r#"[{"n":3,"x":0,"y":0,"z":0,"me":true}]"#);
        assert!(me[0].me);
        assert_eq!(player_label(&me[0]), "Tú (jugador 3)");
        assert!(parse_online("[").is_empty());
    }

    #[test]
    fn state_saves_files_the_dll_reads() {
        let dir = std::env::temp_dir().join(format!("m182_social_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = SocialState::default();
        s.zones.push(Zone { name: "Torre".into(), x: 5.0, z: 6.0, radius: 20.0, enabled: true });
        s.teams.push(Team { name: "Rojo".into(), members: vec![1, 2] });
        s.save_zones(&dir);
        s.save_teams(&dir);
        let loaded = SocialState::load(&dir);
        assert_eq!(loaded.zones, s.zones);
        assert_eq!(loaded.teams, s.teams);
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(TEAMS_FILE)).unwrap()).unwrap();
        assert_eq!(raw["teams"][0]["members"], json!([1, 2]));
        let _ = std::fs::remove_dir_all(dir);
    }
}

use std::path::Path;
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText, Vec2};
use serde_json::{json, Value};

use crate::session::{LogWatch, Member};
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
    /// Nobody loses health inside.
    pub safe: bool,
    /// Team that owns the place (1-based position in the teams list); 0 = nobody.
    pub team: usize,
    /// Everybody outside the owning team is a visitor: no chests, no building.
    pub lock: bool,
    /// Also applies to administrators (to try it out).
    pub lock_admins: bool,
}

impl Zone {
    pub fn new(name: String, x: f64, z: f64, radius: f64) -> Self {
        Self { name, x, z, radius, enabled: true, safe: true, team: 0, lock: false, lock_admins: false }
    }
}

/// Keeps the owners of the zones right after a team was deleted.
pub fn team_removed(zones: &mut [Zone], removed: usize) {
    for z in zones {
        if z.team == removed + 1 {
            z.team = 0;
            z.lock = false;
        } else if z.team > removed + 1 {
            z.team -= 1;
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub key: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Team {
    pub name: String,
    pub members: Vec<Person>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub member: Member,
    pub online: Option<Online>,
}

impl Seen {
    pub fn person(&self) -> Person {
        Person { key: self.member.key(), name: self.member.name.clone() }
    }
}

/// Matches the players the game reports in its log (name and platform id) with the characters the
/// native layer sees in the world: the host first, then the others in the order they appeared.
pub fn pair_players(log: &[Member], online: &[Online]) -> Vec<Seen> {
    if log.is_empty() {
        return online
            .iter()
            .map(|p| Seen {
                member: Member { slot: p.number, name: if p.me { format!("Tú (jugador {})", p.number) } else { format!("Jugador {}", p.number) }, platform: String::new(), id: String::new() },
                online: Some(p.clone()),
            })
            .collect();
    }
    let host_online = online.iter().find(|p| p.me).or_else(|| online.first());
    let mut others = online.iter().filter(|p| Some(p.number) != host_online.map(|h| h.number));
    log.iter()
        .map(|m| {
            let online = if m.slot == 0 { host_online.cloned() } else { others.next().cloned() };
            Seen { member: m.clone(), online }
        })
        .collect()
}

pub fn team_numbers(teams: &[Team], seen: &[Seen]) -> Vec<Vec<u32>> {
    teams
        .iter()
        .map(|t| {
            let mut numbers: Vec<u32> = seen
                .iter()
                .filter(|s| t.members.iter().any(|m| m.key == s.member.key()))
                .filter_map(|s| s.online.as_ref().map(|o| o.number))
                .collect();
            numbers.sort_unstable();
            numbers
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Online {
    pub number: u32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub me: bool,
}

impl Online {
    pub fn located(&self) -> bool {
        self.x.is_finite() && self.z.is_finite()
    }
}

pub fn player_label(p: &Online) -> String {
    if p.me {
        format!("Tú (jugador {})", p.number)
    } else {
        format!("Jugador {}", p.number)
    }
}

/// The list a server answered with: `{"seq":N,"players":[{"n":1,"name":"Ana"}]}` for request `seq`.
pub fn parse_roster(text: &str, seq: u32) -> Option<Vec<(u32, String)>> {
    let v: Value = serde_json::from_str(text).ok()?;
    if v.get("seq")?.as_u64()? != seq as u64 {
        return None;
    }
    let mut out: Vec<(u32, String)> = v
        .get("players")?
        .as_array()?
        .iter()
        .filter_map(|p| Some((p.get("n")?.as_u64()? as u32, p.get("name")?.as_str()?.trim().to_string())))
        .filter(|(_, name)| !name.is_empty())
        .collect();
    out.sort_by_key(|p| p.0);
    Some(out)
}

pub fn roster_seen(list: &[(u32, String)]) -> Vec<Seen> {
    list.iter()
        .map(|(n, name)| Seen {
            member: Member { slot: *n, name: name.clone(), platform: String::new(), id: String::new() },
            online: Some(Online { number: *n, x: f64::NAN, y: f64::NAN, z: f64::NAN, me: false }),
        })
        .collect()
}

pub const MANUAL_PLAYERS: u32 = 8;

pub fn manual_players() -> Vec<Seen> {
    (1..=MANUAL_PLAYERS)
        .map(|n| Seen {
            member: Member { slot: n, name: format!("Jugador {n}"), platform: "slot".to_string(), id: n.to_string() },
            online: Some(Online { number: n, x: 0.0, y: 0.0, z: 0.0, me: false }),
        })
        .collect()
}

pub struct SocialState {
    pub manual: bool,
    pub zones: Vec<Zone>,
    pub teams: Vec<Team>,
    pub online: Vec<Online>,
    pub seen: Vec<Seen>,
    pub watch: LogWatch,
    pub written: Vec<Vec<u32>>,
    pub checked: Instant,
    pub zone_name: String,
    pub zone_radius: f64,
    pub team_name: String,
    pub notice: String,
    pub roster: Vec<(u32, String)>,
    pub roster_seq: u32,
    pub roster_asked: Option<Instant>,
    pub extra: Vec<String>,
    pub extra_input: String,
}

impl Default for SocialState {
    fn default() -> Self {
        Self {
            manual: false,
            zones: Vec::new(),
            teams: Vec::new(),
            online: Vec::new(),
            seen: Vec::new(),
            watch: LogWatch::new(),
            written: Vec::new(),
            checked: Instant::now(),
            zone_name: String::new(),
            zone_radius: 40.0,
            team_name: String::new(),
            notice: String::new(),
            roster: Vec::new(),
            roster_seq: 0,
            roster_asked: None,
            extra: Vec::new(),
            extra_input: String::new(),
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
                safe: v.get("safe").and_then(|e| e.as_bool()).unwrap_or(true),
                team: v.get("team").and_then(|e| e.as_u64()).unwrap_or(0) as usize,
                lock: v.get("lock").and_then(|e| e.as_bool()).unwrap_or(false),
                lock_admins: v.get("lock_admins").and_then(|e| e.as_bool()).unwrap_or(false),
            })
        })
        .collect()
}

pub fn zones_json(zones: &[Zone]) -> Value {
    json!({
        "zones": zones.iter().map(|z| json!({"name": z.name, "x": z.x, "z": z.z, "radius": z.radius, "enabled": z.enabled, "safe": z.safe, "team": z.team, "lock": z.lock && z.team != 0, "lock_admins": z.lock_admins})).collect::<Vec<_>>()
    })
}

pub fn parse_teams(text: &str) -> Vec<Team> {
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let Some(Value::Array(list)) = root.get("teams") else { return Vec::new() };
    list.iter()
        .filter_map(|v| {
            let members = v
                .get("people")
                .and_then(|p| p.as_array())
                .map(|people| {
                    people
                        .iter()
                        .filter_map(|p| Some(Person { key: p.get("key")?.as_str()?.to_string(), name: p.get("name")?.as_str()?.to_string() }))
                        .collect()
                })
                .unwrap_or_default();
            Some(Team { name: v.get("name")?.as_str()?.to_string(), members })
        })
        .collect()
}

pub fn teams_json(teams: &[Team], numbers: &[Vec<u32>]) -> Value {
    json!({
        "teams": teams.iter().enumerate().map(|(i, t)| json!({
            "name": t.name,
            "members": numbers.get(i).cloned().unwrap_or_default(),
            "people": t.members.iter().map(|p| json!({"key": p.key, "name": p.name})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>()
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

pub fn set_member(teams: &mut [Team], team: usize, person: &Person, member: bool) {
    for (i, t) in teams.iter_mut().enumerate() {
        t.members.retain(|m| m.key != person.key);
        if member && i == team {
            t.members.push(person.clone());
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
        self.watch.refresh(dir);
        self.seen = pair_players(&self.watch.members, &self.online);
        if crate::remote::active() {
            self.read_roster(dir);
            if !self.roster.is_empty() {
                self.seen = roster_seen(&self.roster);
            }
        }
        self.manual = crate::remote::active() && self.seen.is_empty();
        if self.manual {
            self.seen = manual_players();
        }
    }

    fn read_roster(&mut self, dir: &Path) {
        if self.roster_asked.is_none() {
            return;
        }
        let Ok(text) = std::fs::read_to_string(dir.join(crate::remote::ONLINE_FILE)) else { return };
        if let Some(list) = parse_roster(&text, self.roster_seq) {
            self.roster = list;
            self.roster_asked = None;
            self.notice = format!("{} jugador(es) conectado(s) al servidor.", self.roster.len());
        }
    }

    /// Asks the server who is connected: the answer comes back through the chat and this game's
    /// native layer copies it into `m182_online.json`.
    pub fn request_roster(&mut self, dir: &Path) {
        let seq = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(1) % 1_000_000) as u32 + 1;
        self.roster_seq = seq;
        let _ = std::fs::remove_file(dir.join(crate::remote::ONLINE_FILE));
        let scan = json!({"seq": seq}).to_string();
        self.notice = match write_atomic(dir, crate::remote::SCAN_FILE, "m182_scan.request.tmp", &scan) {
            Ok(()) => {
                crate::remote::submit_players(seq);
                self.roster_asked = Some(Instant::now());
                "Preguntando al servidor quién está conectado…".to_string()
            }
            Err(e) => format!("Error: {e}"),
        };
    }

    pub fn roster_failed(&mut self) -> bool {
        match self.roster_asked {
            Some(at) if at.elapsed() > Duration::from_secs(75) => {
                self.roster_asked = None;
                true
            }
            _ => false,
        }
    }

    pub fn save_zones(&mut self, dir: &Path) {
        self.notice = match write_atomic(dir, ZONES_FILE, "m182_zones.json.tmp", &zones_json(&self.zones).to_string()) {
            Ok(()) => "Guardado. El juego lo aplica en un par de segundos.".to_string(),
            Err(e) => format!("Error: {e}"),
        };
    }

    pub fn save_teams(&mut self, dir: &Path) {
        let numbers = team_numbers(&self.teams, &self.seen);
        let result = write_atomic(dir, TEAMS_FILE, "m182_teams.json.tmp", &teams_json(&self.teams, &numbers).to_string());
        if result.is_ok() {
            self.written = numbers;
        }
        self.notice = match result {
            Ok(()) => "Guardado. El juego lo aplica en un par de segundos.".to_string(),
            Err(e) => format!("Error: {e}"),
        };
    }

    pub fn resync_teams(&mut self, dir: &Path) {
        if self.teams.is_empty() {
            return;
        }
        if team_numbers(&self.teams, &self.seen) != self.written {
            let notice = self.notice.clone();
            self.save_teams(dir);
            self.notice = notice;
        }
    }
}

impl App {
    pub fn social_tick(&mut self) {
        if self.social.checked.elapsed() >= Duration::from_millis(1000) {
            let dir = self.dir.clone();
            self.social.refresh(&dir);
            self.social.resync_teams(&dir);
        }
    }

    fn add_zone(&mut self, x: f64, z: f64) {
        let name = if self.social.zone_name.trim().is_empty() {
            format!("Zona {}", self.social.zones.len() + 1)
        } else {
            self.social.zone_name.trim().to_string()
        };
        self.social.zones.push(Zone::new(name, x, z, self.social.zone_radius));
        self.social.zone_name.clear();
        let dir = self.dir.clone();
        self.social.save_zones(&dir);
    }

    pub fn zones_tab(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Zonas y bases", 22.0);
        ui.label(RichText::new("Una zona segura protege de monstruos y jugadores. Una base protegida pertenece a un equipo: los demás son visitantes dentro y no pueden abrir cofres ni construir.").color(MUTED));
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
        let team_names: Vec<String> = self.social.teams.iter().map(|t| t.name.clone()).collect();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, z) in self.social.zones.iter_mut().enumerate() {
                ui.group(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        changed |= ui.checkbox(&mut z.enabled, "").changed();
                        ui.label(RichText::new(&z.name).font(title_font(17.0)).color(PARCHMENT));
                        ui.label(RichText::new(format!("X {:.0}  Z {:.0}", z.x, z.z)).color(MUTED));
                        ui.label("Radio");
                        changed |= ui.add(egui::DragValue::new(&mut z.radius).speed(1.0).range(5.0..=2000.0).fixed_decimals(0)).changed();
                        if button(ui, "Borrar", Vec2::new(80.0, 26.0), true, false) {
                            remove = Some(i);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        changed |= ui.checkbox(&mut z.safe, "Zona segura (nadie pierde vida)").changed();
                        ui.label("Equipo dueño");
                        let current = if z.team == 0 { "Nadie".to_string() } else { team_names.get(z.team - 1).cloned().unwrap_or_else(|| "—".to_string()) };
                        egui::ComboBox::from_id_salt(("zone_team", i)).selected_text(current).show_ui(ui, |ui| {
                            if ui.selectable_value(&mut z.team, 0, "Nadie").changed() {
                                z.lock = false;
                                changed = true;
                            }
                            for (t, name) in team_names.iter().enumerate() {
                                changed |= ui.selectable_value(&mut z.team, t + 1, name).changed();
                            }
                        });
                    });
                    if z.team != 0 {
                        ui.horizontal_wrapped(|ui| {
                            changed |= ui.checkbox(&mut z.lock, "Base protegida: quien no sea de ese equipo es visitante y no puede abrir cofres ni construir").changed();
                            if z.lock {
                                changed |= ui.checkbox(&mut z.lock_admins, "También a los administradores (para probarlo)").changed();
                            }
                        });
                    }
                });
            }
            if self.social.zones.is_empty() {
                ui.label(RichText::new("Aún no hay zonas.").color(MUTED));
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
        label_title(ui, "JUGADORES CONECTADOS");
        if self.social.seen.is_empty() {
            ui.label(RichText::new("Nadie conectado todavía: abre el juego y carga el mundo.").color(MUTED));
        }
        if crate::remote::active() {
            let dir = self.dir.clone();
            ui.horizontal_wrapped(|ui| {
                let waiting = self.social.roster_asked.is_some();
                if button(ui, "Buscar jugadores del servidor", Vec2::new(280.0, 32.0), crate::remote::connected() && !waiting, true) {
                    self.social.request_roster(&dir);
                }
                if waiting {
                    ui.label(RichText::new("esperando la respuesta del servidor…").color(GOLD_DIM));
                }
            });
            if self.social.roster_failed() {
                self.social.notice = "El servidor no respondió. Revisa la clave y que hayas subido los archivos nuevos al servidor.".to_string();
            }
            if self.social.manual {
                ui.label(RichText::new("Pulsa «Buscar jugadores del servidor». Mientras no responda se muestran 8 jugadores numerados; también puedes añadir nombres a mano abajo.").color(MUTED));
            }
        }
        let manual = self.social.manual;
        for s in self.social.seen.iter().filter(|_| !manual) {
            let team = self.social.teams.iter().find(|t| t.members.iter().any(|m| m.key == s.member.key())).map(|t| t.name.clone()).unwrap_or_else(|| "sin equipo".to_string());
            let place = match &s.online {
                Some(o) if o.located() => format!("X {:.0}  Z {:.0}", o.x, o.z),
                Some(_) => String::new(),
                None => "cargando…".to_string(),
            };
            let host = s.online.as_ref().map(|o| o.me).unwrap_or(s.member.slot == 0);
            let name = if host { format!("{}  (tú)", s.member.name) } else { s.member.name.clone() };
            ui.horizontal_wrapped(|ui| {
                let label = RichText::new(name).font(title_font(17.0));
                ui.label(if host { label.color(GOLD) } else { label.color(PARCHMENT) });
                let id = s.member.id_label();
                if !id.is_empty() {
                    ui.label(RichText::new(id).color(MUTED));
                }
                ui.label(RichText::new(place).color(MUTED));
                ui.label(team);
            });
        }
        ui.label(RichText::new("Los nombres y los ID de Steam o PS5 los toma el panel del registro del propio juego. Cada jugador queda en su equipo aunque se desconecte y vuelva a entrar.").color(MUTED).size(15.0));
        divider(ui);

        let seen: Vec<Seen> = self.social.seen.clone();
        let mut candidates: Vec<Person> = seen.iter().map(|s| s.person()).collect();
        for name in &self.social.extra {
            let person = Person { key: format!("name:{name}"), name: name.clone() };
            if !candidates.iter().any(|c| c.key == person.key) {
                candidates.push(person);
            }
        }
        for team in &self.social.teams {
            for m in &team.members {
                if !candidates.iter().any(|c| c.key == m.key) {
                    candidates.push(m.clone());
                }
            }
        }
        let mut changed = false;
        let mut remove = None;
        let mut membership: Vec<(usize, Person, bool)> = Vec::new();
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
                        for person in &candidates {
                            let mut on = team.members.iter().any(|m| m.key == person.key);
                            let connected = seen.iter().any(|s| s.member.key() == person.key);
                            let label = if connected { person.name.clone() } else { format!("{} (desconectado)", person.name) };
                            if ui.checkbox(&mut on, label).changed() {
                                membership.push((i, person.clone(), on));
                            }
                        }
                        if candidates.is_empty() {
                            ui.label(RichText::new("No hay jugadores conectados todavía.").color(MUTED));
                        }
                    });
                });
            }
        });
        for (team, person, on) in membership {
            set_member(&mut self.social.teams, team, &person, on);
            changed = true;
        }
        if let Some(i) = remove {
            self.social.teams.remove(i);
            team_removed(&mut self.social.zones, i);
            let dir = self.dir.clone();
            self.social.save_zones(&dir);
            changed = true;
        }
        if crate::remote::active() {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.social.extra_input).hint_text("Nombre exacto de un jugador").desired_width(260.0));
                if button(ui, "Añadir jugador", Vec2::new(160.0, 30.0), !self.social.extra_input.trim().is_empty(), false) {
                    let name = self.social.extra_input.trim().to_string();
                    if !self.social.extra.contains(&name) {
                        self.social.extra.push(name);
                    }
                    self.social.extra_input.clear();
                }
            });
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
        let mut off = Zone::new("Off".into(), 0.0, 0.0, 10.0);
        off.enabled = false;
        let mut base = Zone::new("Base".into(), 100.0, -50.0, 30.0);
        base.safe = false;
        base.team = 2;
        base.lock = true;
        base.lock_admins = true;
        let zones = vec![base, off];
        let back = parse_zones(&zones_json(&zones).to_string());
        assert_eq!(back, zones);
        assert_eq!(zone_at(&back, 110.0, -40.0).map(|z| z.name.as_str()), Some("Base"));
        assert!(zone_at(&back, 0.0, 0.0).is_none());
        assert!(parse_zones("basura").is_empty());
    }

    fn person(key: &str, name: &str) -> Person {
        Person { key: key.into(), name: name.into() }
    }

    #[test]
    fn teams_are_exclusive_and_roundtrip() {
        let a = person("steamid:1", "A");
        let b = person("steamid:2", "B");
        let c = person("name:C", "C");
        let mut teams = vec![Team { name: "A".into(), members: vec![a.clone()] }, Team { name: "B".into(), members: vec![] }];
        set_member(&mut teams, 1, &a, true);
        assert!(teams[0].members.is_empty());
        assert_eq!(teams[1].members, vec![a.clone()]);
        set_member(&mut teams, 1, &c, true);
        set_member(&mut teams, 1, &b, true);
        assert_eq!(teams[1].members.len(), 3);
        set_member(&mut teams, 1, &b, false);
        assert_eq!(teams[1].members, vec![a, c]);
        assert_eq!(parse_teams(&teams_json(&teams, &[vec![], vec![1, 2]]).to_string()), teams);
    }

    #[test]
    fn log_players_are_paired_with_world_characters() {
        let log = vec![
            Member { slot: 0, name: "moradoox".into(), platform: String::new(), id: String::new() },
            Member { slot: 1, name: "Nacht".into(), platform: "steamid".into(), id: "7656".into() },
        ];
        let online = vec![
            Online { number: 1, x: 1.0, y: 0.0, z: 2.0, me: true },
            Online { number: 2, x: 5.0, y: 0.0, z: 6.0, me: false },
        ];
        let seen = pair_players(&log, &online);
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].online.as_ref().unwrap().number, 1);
        assert_eq!(seen[1].online.as_ref().unwrap().number, 2);
        // a friend whose character has not loaded yet is listed without a position
        let loading = pair_players(&log, &online[..1]);
        assert_eq!(loading.len(), 2);
        assert!(loading[1].online.is_none());
        // a stray extra entity never becomes a listed player
        let extra = vec![online[0].clone(), online[1].clone(), Online { number: 3, x: 0.0, y: 0.0, z: 0.0, me: false }];
        assert_eq!(pair_players(&log[..1], &extra).len(), 1);
        let teams = vec![Team { name: "Rojo".into(), members: vec![seen[0].person(), seen[1].person()] }];
        assert_eq!(team_numbers(&teams, &seen), vec![vec![1, 2]]);
        assert_eq!(team_numbers(&teams, &loading), vec![vec![1]]);
        // without a readable log the characters of the world are listed by number
        let fallback = pair_players(&[], &online);
        assert_eq!(fallback.len(), 2);
        assert_eq!(fallback[0].member.name, "Tú (jugador 1)");
    }
    #[test]
    fn manual_players_give_team_numbers_without_a_player_list() {
        let seen = manual_players();
        assert_eq!(seen.len(), MANUAL_PLAYERS as usize);
        let teams = vec![Team { name: "Rojo".into(), members: vec![seen[0].person(), seen[2].person()] }];
        assert_eq!(team_numbers(&teams, &seen), vec![vec![1, 3]]);
    }

    #[test]
    fn the_server_roster_becomes_the_player_list_and_team_keys_are_names() {
        let text = r#"{"seq":42,"players":[{"n":2,"name":"Ana"},{"n":1,"name":" Luis "},{"n":3,"name":""}]}"#;
        assert!(parse_roster(text, 41).is_none(), "an answer to another question is ignored");
        let list = parse_roster(text, 42).unwrap();
        assert_eq!(list, vec![(1, "Luis".to_string()), (2, "Ana".to_string())]);
        let seen = roster_seen(&list);
        assert_eq!(seen[1].member.key(), "name:Ana");
        assert!(!seen[0].online.as_ref().unwrap().located());
        let teams = vec![Team { name: "Casa".into(), members: vec![seen[1].person()] }];
        assert_eq!(team_numbers(&teams, &seen), vec![vec![2]]);
        let json = teams_json(&teams, &team_numbers(&teams, &seen));
        assert_eq!(json["teams"][0]["people"][0]["name"], "Ana");
        assert!(parse_roster("[", 1).is_none());
    }

    #[test]
    fn deleting_a_team_releases_or_shifts_the_bases_it_owned() {
        let mut zones = vec![Zone::new("A".into(), 0.0, 0.0, 10.0), Zone::new("B".into(), 0.0, 0.0, 10.0), Zone::new("C".into(), 0.0, 0.0, 10.0)];
        zones[0].team = 1;
        zones[0].lock = true;
        zones[1].team = 2;
        zones[1].lock = true;
        zones[2].team = 3;
        team_removed(&mut zones, 0);
        assert_eq!((zones[0].team, zones[0].lock), (0, false));
        assert_eq!((zones[1].team, zones[2].team), (1, 2));
        assert!(zones[1].lock);
        let json = zones_json(&[Zone { team: 0, lock: true, ..Zone::new("X".into(), 1.0, 2.0, 5.0) }]);
        assert_eq!(json["zones"][0]["lock"], false, "a base without an owner never locks");
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
        s.zones.push(Zone::new("Torre".into(), 5.0, 6.0, 20.0));
        s.teams.push(Team { name: "Rojo".into(), members: vec![Person { key: "name:Nacht".into(), name: "Nacht".into() }] });
        s.save_zones(&dir);
        s.save_teams(&dir);
        let loaded = SocialState::load(&dir);
        assert_eq!(loaded.zones, s.zones);
        assert_eq!(loaded.teams, s.teams);
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(TEAMS_FILE)).unwrap()).unwrap();
        assert_eq!(raw["teams"][0]["members"], json!([]));
        assert_eq!(raw["teams"][0]["people"][0]["name"], json!("Nacht"));
        let _ = std::fs::remove_dir_all(dir);
    }
}

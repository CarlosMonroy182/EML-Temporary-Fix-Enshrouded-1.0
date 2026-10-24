//! Events and raids: up to 10 enemy templates (creature, amount, level and loot) that are sent to
//! the zones the administrator picks, automatically every few minutes or at the push of a button.

use std::collections::BTreeSet;

use eframe::egui::{self, RichText, Vec2};

use crate::spawners::{self, Drop, RaidTemplate, Spawner, MAX_DROPS, MAX_TEMPLATES};
use crate::theme::*;
use crate::App;

pub struct Draft {
    pub creature: Option<usize>,
    pub count: u32,
    pub level: u32,
    pub damage: f32,
    pub drops: Vec<Drop>,
    pub item: Option<usize>,
    pub amount: u32,
    pub rarity: u32,
    pub item_level: u32,
    pub chance: u32,
    pub creature_query: String,
    pub item_query: String,
}

impl Default for Draft {
    fn default() -> Self {
        Draft { creature: None, count: 3, level: 50, damage: 1.0, drops: Vec::new(), item: None, amount: 1, rarity: 0, item_level: 1, chance: 100, creature_query: String::new(), item_query: String::new() }
    }
}

#[derive(Default)]
pub struct EventState {
    pub zones: BTreeSet<usize>,
    pub notice: String,
    pub templates: Vec<RaidTemplate>,
    pub loaded: bool,
    pub name: String,
    pub minutes: u32,
    pub draft: Draft,
    pub counter: u64,
}

/// Height for a zone: a saved place inside it, otherwise where the administrator stands.
pub fn zone_height(places: &[(f64, f64, f64)], zone: (f64, f64, f64), me: Option<[f64; 3]>) -> Option<f64> {
    let (zx, zz, r) = zone;
    places.iter().find(|(x, _, z)| ((x - zx).powi(2) + (z - zz).powi(2)).sqrt() <= r).map(|(_, y, _)| *y).or_else(|| me.map(|p| p[1]))
}

/// A spawner that sends a random template to a random point of the zone every `every` seconds.
pub fn automatic_raid(id: String, place: &str, at: [f64; 3], radius: f32, every: u32, templates: &[RaidTemplate], pickup: &str) -> Spawner {
    Spawner {
        id,
        creature: "Raids automáticos".to_string(),
        template: templates.first().map(|t| t.template.clone()).unwrap_or_default(),
        place: place.to_string(),
        at,
        level: 1,
        count: 1,
        health: 0,
        damage: 1.0,
        radius: 3.0,
        respawn: 300,
        once: false,
        drops: Vec::new(),
        pickup: pickup.to_string(),
        pool: templates.iter().take(MAX_TEMPLATES).cloned().collect(),
        every,
        area: radius,
    }
}

/// A pseudo-random number in [0, 1) that differs on every call.
pub fn unit(counter: &mut u64) -> f64 {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    *counter = counter.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut x = nanos ^ *counter;
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// One immediate wave: a random template at a random point inside the zone.
pub fn instant_raid(id: String, place: &str, at: [f64; 3], radius: f32, template: &RaidTemplate, pickup: &str, a: f64, b: f64) -> Spawner {
    let angle = a * std::f64::consts::TAU;
    let dist = b.sqrt() * f64::from(radius);
    Spawner {
        id,
        creature: template.name.clone(),
        template: template.template.clone(),
        place: place.to_string(),
        at: [at[0] + dist * angle.cos(), at[1] + 3.0, at[2] + dist * angle.sin()],
        level: template.level,
        count: template.count,
        health: template.health,
        damage: template.damage,
        radius: if template.count > 1 { 4.0 } else { 3.0 },
        respawn: 300,
        once: true,
        drops: template.drops.clone(),
        pickup: if template.drops.is_empty() { String::new() } else { pickup.to_string() },
        pool: Vec::new(),
        every: 0,
        area: 0.0,
    }
}

impl App {
    fn creature_name(&self, i: usize) -> Option<String> {
        self.creatures.entries.get(i).map(|e| e.alias.clone().unwrap_or_else(|| e.label.clone()))
    }

    fn item_name(&self, i: usize) -> Option<String> {
        self.items.rows.get(i).map(|r| self.item_cat.entries.get(i).map(|e| e.label.clone()).unwrap_or_else(|| r.name.clone()))
    }

    fn zone_spots(&self) -> Result<Vec<(String, [f64; 3], f32)>, String> {
        let spots: Vec<(f64, f64, f64)> = self.places.iter().map(|p| (p.x, p.y, p.z)).collect();
        let mut out = Vec::new();
        for &i in &self.ev.zones {
            let Some(z) = self.social.zones.get(i) else { continue };
            let y = zone_height(&spots, (z.x, z.z, z.radius), self.player).ok_or_else(|| format!("Para «{}» guarda un lugar dentro de la zona (hace falta su altura).", z.name))?;
            out.push((z.name.clone(), [z.x, y, z.z], z.radius as f32));
        }
        if out.is_empty() {
            return Err("Elige al menos una zona.".to_string());
        }
        Ok(out)
    }

    fn save_templates_file(&mut self) {
        if let Err(e) = spawners::save_templates(&self.dir, &self.ev.templates) {
            self.ev.notice = format!("Error: {e}");
        }
    }

    fn add_draft_drop(&mut self) {
        let d = &self.ev.draft;
        if d.drops.len() >= MAX_DROPS {
            self.ev.notice = format!("Máximo {MAX_DROPS} objetos de botín.");
            return;
        }
        let Some(i) = d.item else {
            self.ev.notice = "Elige un objeto de la lista.".to_string();
            return;
        };
        let Some(item) = self.items.rows.get(i).cloned() else { return };
        if !item.deliverable() {
            self.ev.notice = "Ese objeto no se puede entregar.".to_string();
            return;
        }
        let name = self.item_name(i).unwrap_or_else(|| item.name.clone());
        let d = &mut self.ev.draft;
        d.drops.push(Drop { item: item.id, guid: item.guid.clone(), name, amount: d.amount.clamp(1, 65535), rarity: d.rarity.min(4), level: d.item_level.clamp(1, 255), chance: d.chance.clamp(1, 100) });
        self.ev.notice.clear();
    }

    fn save_template(&mut self) {
        if self.ev.templates.len() >= MAX_TEMPLATES {
            self.ev.notice = format!("Máximo {MAX_TEMPLATES} plantillas.");
            return;
        }
        let Some(ci) = self.ev.draft.creature else {
            self.ev.notice = "Elige un enemigo de la lista.".to_string();
            return;
        };
        let Some(entry) = self.creatures.entries.get(ci) else { return };
        let creature = self.creature_name(ci).unwrap_or_default();
        let guid = entry.guid.clone();
        let name = if self.ev.name.trim().is_empty() { format!("{} {}", creature, self.ev.templates.len() + 1) } else { self.ev.name.trim().to_string() };
        let d = &self.ev.draft;
        let t = RaidTemplate { name, template: guid, level: d.level.clamp(1, 1000), count: d.count.clamp(1, 20), health: 0, damage: d.damage, drops: d.drops.clone() };
        self.ev.templates.push(t);
        self.ev.name.clear();
        self.ev.draft.drops.clear();
        self.save_templates_file();
        self.ev.notice = "Plantilla guardada.".to_string();
    }

    fn activate_auto(&mut self) {
        if self.ev.templates.is_empty() {
            self.ev.notice = "Crea al menos una plantilla.".to_string();
            return;
        }
        let spots = match self.zone_spots() {
            Ok(s) => s,
            Err(e) => {
                self.ev.notice = e;
                return;
            }
        };
        let base_ms = spawners::now_ms();
        let wave: Vec<Spawner> = spots
            .iter()
            .enumerate()
            .map(|(n, (name, at, radius))| {
                let id = self.spawners.iter().find(|s| !s.pool.is_empty() && &s.place == name).map(|s| s.id.clone()).unwrap_or_else(|| format!("s{}", base_ms + n as u128));
                automatic_raid(id, name, *at, *radius, self.ev.minutes.clamp(1, 1440) * 60, &self.ev.templates, &self.items.generic_pickup)
            })
            .collect();
        self.push_wave(wave, format!("Raids automáticos activos en {} zona(s), cada {} min.", spots.len(), self.ev.minutes));
    }

    fn launch_now(&mut self) {
        if self.ev.templates.is_empty() {
            self.ev.notice = "Crea al menos una plantilla.".to_string();
            return;
        }
        let spots = match self.zone_spots() {
            Ok(s) => s,
            Err(e) => {
                self.ev.notice = e;
                return;
            }
        };
        let base_ms = spawners::now_ms();
        let mut wave = Vec::new();
        for (n, (name, at, radius)) in spots.iter().enumerate() {
            let pick = ((unit(&mut self.ev.counter) * self.ev.templates.len() as f64) as usize).min(self.ev.templates.len() - 1);
            let (a, b) = (unit(&mut self.ev.counter), unit(&mut self.ev.counter));
            wave.push(instant_raid(format!("s{}", base_ms + n as u128), name, *at, *radius, &self.ev.templates[pick], &self.items.generic_pickup, a, b));
        }
        self.push_wave(wave, format!("Raid enviado a {} zona(s). Aparece si hay un jugador a menos de {} m.", spots.len(), spawners::RANGE_METERS as u32));
    }

    fn push_wave(&mut self, wave: Vec<Spawner>, ok: String) {
        let op = spawners::op_add_many(&wave);
        for w in wave {
            match self.spawners.iter().position(|s| s.id == w.id) {
                Some(i) => self.spawners[i] = w,
                None => self.spawners.push(w),
            }
        }
        self.ev.notice = match spawners::save_change(&self.dir, &self.spawners, &op) {
            Ok(()) => ok,
            Err(e) => format!("Error: {e}"),
        };
    }

    /// Raids that finished (or are too old) leave the list.
    pub fn clean_raids(&mut self) {
        let done = std::fs::read_to_string(self.dir.join(spawners::STATUS_FILE)).map(|t| spawners::parse_done(&t)).unwrap_or_default();
        let now = spawners::now_ms();
        let before = self.spawners.len();
        self.spawners.retain(|s| !(s.once && done.contains(&s.id)) && !spawners::expired(s, now));
        if self.spawners.len() != before {
            let _ = spawners::save(&self.dir, &self.spawners);
        }
    }

    pub fn events_tab(&mut self, ui: &mut egui::Ui) {
        if self.ev.minutes == 0 {
            self.ev.minutes = 60;
        }
        if self.ev.draft.chance == 0 {
            self.ev.draft.chance = 100;
        }
        if !self.ev.loaded {
            self.ev.loaded = true;
            self.ev.templates = spawners::load_templates(&self.dir);
        }
        heading(ui, "Eventos y raids", 22.0);
        divider(ui);
        egui::ScrollArea::vertical().show(ui, |ui| {
            label_title(ui, &format!("PLANTILLAS ({}/{})", self.ev.templates.len(), MAX_TEMPLATES));
            let mut remove = None;
            for (i, t) in self.ev.templates.iter().enumerate() {
                ui.group(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&t.name).color(PARCHMENT));
                        ui.label(RichText::new(format!("{} × · nv.{} · daño ×{:.1}", t.count, t.level, t.damage)).color(MUTED));
                        if button(ui, "Quitar", Vec2::new(70.0, 24.0), true, false) {
                            remove = Some(i);
                        }
                    });
                    if t.drops.is_empty() {
                        ui.label(RichText::new("Botín: el normal del juego").color(MUTED).size(14.0));
                    }
                    for d in &t.drops {
                        ui.label(RichText::new(format!("Botín: {} × {} · {}%", d.amount, d.name, d.chance)).color(GOLD_DIM).size(14.0));
                    }
                });
            }
            if let Some(i) = remove {
                self.ev.templates.remove(i);
                self.save_templates_file();
            }

            label_title(ui, "NUEVA PLANTILLA");
            ui.label("Enemigo");
            ui.add(egui::TextEdit::singleline(&mut self.ev.draft.creature_query).hint_text("Buscar enemigo…").desired_width(260.0));
            let q = self.ev.draft.creature_query.trim().to_lowercase();
            if let Some(c) = self.ev.draft.creature.and_then(|i| self.creature_name(i)) {
                ui.label(RichText::new(format!("Elegido: {c}")).color(GOOD));
            }
            if !q.is_empty() {
                let found: Vec<(usize, String)> = self.creatures.entries.iter().enumerate().filter(|(_, e)| !e.internal && e.search.contains(&q)).take(8).map(|(i, e)| (i, e.alias.clone().unwrap_or_else(|| e.label.clone()))).collect();
                for (i, name) in found {
                    if ui.selectable_label(self.ev.draft.creature == Some(i), name).clicked() {
                        self.ev.draft.creature = Some(i);
                    }
                }
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Cantidad");
                ui.add(egui::DragValue::new(&mut self.ev.draft.count).range(1..=20));
                ui.label("Nivel");
                ui.add(egui::DragValue::new(&mut self.ev.draft.level).range(1..=1000));
                ui.label("Daño ×");
                ui.add(egui::DragValue::new(&mut self.ev.draft.damage).range(0.1..=1000.0).speed(0.1));
            });

            ui.label(format!("Botín de cada enemigo ({}/{})", self.ev.draft.drops.len(), MAX_DROPS));
            let mut drop_out = None;
            for (i, d) in self.ev.draft.drops.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} × {} · {}%", d.amount, d.name, d.chance)).color(PARCHMENT));
                    if button(ui, "Quitar", Vec2::new(70.0, 24.0), true, false) {
                        drop_out = Some(i);
                    }
                });
            }
            if let Some(i) = drop_out {
                self.ev.draft.drops.remove(i);
            }
            ui.add(egui::TextEdit::singleline(&mut self.ev.draft.item_query).hint_text("Buscar objeto…").desired_width(260.0));
            let q = self.ev.draft.item_query.trim().to_lowercase();
            if let Some(n) = self.ev.draft.item.and_then(|i| self.item_name(i)) {
                ui.label(RichText::new(format!("Elegido: {n}")).color(GOOD));
            }
            if !q.is_empty() {
                let found: Vec<(usize, String)> = self.item_cat.entries.iter().enumerate().filter(|(_, e)| e.search.contains(&q)).take(8).map(|(i, e)| (i, e.label.clone())).collect();
                let mut picked = None;
                for (i, name) in found {
                    if ui.selectable_label(self.ev.draft.item == Some(i), name).clicked() {
                        picked = Some(i);
                    }
                }
                if let Some(i) = picked {
                    self.ev.draft.item = Some(i);
                    if let Some(r) = self.items.rows.get(i) {
                        self.ev.draft.item_level = r.give_level.clamp(1, 255);
                        self.ev.draft.rarity = r.rarity_index();
                    }
                }
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Cantidad");
                ui.add(egui::DragValue::new(&mut self.ev.draft.amount).range(1..=65535));
                ui.label("Rareza");
                ui.add(egui::DragValue::new(&mut self.ev.draft.rarity).range(0..=4));
                ui.label("Nivel");
                ui.add(egui::DragValue::new(&mut self.ev.draft.item_level).range(1..=255));
                ui.label("Probabilidad (%)");
                ui.add(egui::DragValue::new(&mut self.ev.draft.chance).range(1..=100));
            });
            if button(ui, "Añadir al botín", Vec2::new(180.0, 28.0), self.ev.draft.item.is_some(), false) {
                self.add_draft_drop();
            }
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.ev.name).hint_text("Nombre de la plantilla").desired_width(220.0));
                if button(ui, "Guardar plantilla", Vec2::new(180.0, 32.0), self.ev.draft.creature.is_some(), true) {
                    self.save_template();
                }
            });

            label_title(ui, "ZONAS");
            if self.social.zones.is_empty() {
                ui.label(RichText::new("Crea zonas en la pestaña Zonas y bases.").color(MUTED));
            }
            for (i, z) in self.social.zones.iter().enumerate() {
                let mut on = self.ev.zones.contains(&i);
                if ui.checkbox(&mut on, format!("{} (radio {:.0} m)", z.name, z.radius)).changed() {
                    if on {
                        self.ev.zones.insert(i);
                    } else {
                        self.ev.zones.remove(&i);
                    }
                }
            }
            let ready = !self.ev.templates.is_empty() && !self.ev.zones.is_empty();
            ui.horizontal_wrapped(|ui| {
                ui.label("Automático cada (minutos)");
                ui.add(egui::Slider::new(&mut self.ev.minutes, 1..=1440));
            });
            if button(ui, "ACTIVAR RAIDS AUTOMÁTICOS", Vec2::new(ui.available_width(), 44.0), ready, true) {
                self.activate_auto();
            }
            if button(ui, "LANZAR UN RAID AHORA", Vec2::new(ui.available_width(), 44.0), ready, true) {
                self.launch_now();
            }
            ui.label(RichText::new("Cada raid elige una plantilla al azar y aparece en un punto al azar dentro del radio de la zona, si hay un jugador a menos de 150 m. Los automáticos se quitan desde la lista de apariciones de la pestaña Criaturas.").color(MUTED).size(14.0));
            if !self.ev.notice.is_empty() {
                ui.label(RichText::new(self.ev.notice.clone()).color(GOOD).size(15.0));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tpl(n: &str) -> RaidTemplate {
        RaidTemplate { name: n.into(), template: format!("{n}-guid"), level: 5, count: 2, health: 0, damage: 1.0, drops: Vec::new() }
    }

    #[test]
    fn zone_height_prefers_a_saved_place_inside() {
        let places = [(100.0, 50.0, 100.0), (900.0, 7.0, 900.0)];
        assert_eq!(zone_height(&places, (105.0, 102.0, 30.0), Some([0.0, 99.0, 0.0])), Some(50.0));
        assert_eq!(zone_height(&places, (500.0, 500.0, 30.0), Some([0.0, 99.0, 0.0])), Some(99.0));
        assert_eq!(zone_height(&places, (500.0, 500.0, 30.0), None), None);
    }

    #[test]
    fn an_automatic_raid_carries_all_templates_and_the_zone_radius() {
        let s = automatic_raid("s9".into(), "Base", [1.0, 2.0, 3.0], 40.0, 3600, &[tpl("a"), tpl("b")], "pick");
        assert_eq!((s.every, s.area, s.pool.len()), (3600, 40.0, 2));
        assert_eq!(s.template, "a-guid");
        assert!(!s.once);
        let wire = spawners::op_add(&s);
        assert!(wire.contains("\"e\":3600") && wire.contains("\"a\":40"));
    }

    #[test]
    fn an_instant_raid_lands_inside_the_zone() {
        let mut c = 0u64;
        for _ in 0..200 {
            let (a, b) = (unit(&mut c), unit(&mut c));
            let s = instant_raid("s1".into(), "Base", [100.0, 20.0, -50.0], 40.0, &tpl("a"), "pick", a, b);
            let d = ((s.at[0] - 100.0).powi(2) + (s.at[2] + 50.0).powi(2)).sqrt();
            assert!(d <= 40.0001, "{d}");
            assert!(s.once && s.pool.is_empty() && s.count == 2);
        }
    }

    #[test]
    fn random_numbers_cover_the_range() {
        let mut c = 0u64;
        let v: Vec<f64> = (0..500).map(|_| unit(&mut c)).collect();
        assert!(v.iter().all(|x| (0.0..1.0).contains(x)));
        assert!(v.iter().any(|x| *x < 0.2) && v.iter().any(|x| *x > 0.8));
    }
}

//! Events and raids: creatures sent once to the places of the players the administrator picks,
//! with a loot list that falls where each one dies.

use std::collections::BTreeSet;

use eframe::egui::{self, RichText, Vec2};

use crate::spawners::{self, Drop, Spawner, MAX_DROPS};
use crate::theme::*;
use crate::App;

#[derive(Default)]
pub struct EventState {
    pub players: BTreeSet<u32>,
    pub zones: BTreeSet<usize>,
    pub places: BTreeSet<usize>,
    pub drops: Vec<Drop>,
    pub chance: u32,
    pub notice: String,
    pub templates: Vec<spawners::RaidTemplate>,
    pub loaded: bool,
    pub name: String,
    pub minutes: u32,
}

/// Height for a zone: a saved place inside it, otherwise where the administrator stands.
pub fn zone_height(places: &[(f64, f64, f64)], zone: (f64, f64, f64), me: Option<[f64; 3]>) -> Option<f64> {
    let (zx, zz, r) = zone;
    places
        .iter()
        .find(|(x, _, z)| ((x - zx).powi(2) + (z - zz).powi(2)).sqrt() <= r)
        .map(|(_, y, _)| *y)
        .or_else(|| me.map(|p| p[1]))
}

/// A spawner that sends a random template to a random point of the zone every `every` seconds.
pub fn automatic_raid(id: String, place: &str, at: [f64; 3], radius: f32, every: u32, templates: &[spawners::RaidTemplate], pickup: &str) -> Spawner {
    let first = templates.first().cloned();
    Spawner {
        id,
        creature: "Raids automáticos".to_string(),
        template: first.map(|t| t.template).unwrap_or_default(),
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
        pool: templates.iter().take(spawners::MAX_TEMPLATES).cloned().collect(),
        every,
        area: radius,
    }
}

/// One raid wave per target, with unique ids that carry their creation time.
pub fn build_raid(base: &Spawner, targets: &[(String, [f64; 3])], now_ms: u128) -> Vec<Spawner> {
    targets
        .iter()
        .enumerate()
        .map(|(i, (label, at))| Spawner { id: format!("s{}", now_ms + i as u128), place: label.clone(), at: *at, once: true, ..base.clone() })
        .collect()
}

impl App {
    fn raid_targets(&self) -> Result<Vec<(String, [f64; 3])>, String> {
        let mut out: Vec<(String, [f64; 3])> = Vec::new();
        for s in &self.social.seen {
            let Some(o) = &s.online else { continue };
            if self.ev.players.contains(&o.number) {
                if !o.located() || !o.y.is_finite() {
                    return Err(format!("No sé dónde está {} ahora mismo. Pulsa «Actualizar jugadores».", s.member.name));
                }
                out.push((s.member.name.clone(), [o.x, o.y, o.z]));
            }
        }
        let spots: Vec<(f64, f64, f64)> = self.places.iter().map(|p| (p.x, p.y, p.z)).collect();
        for &i in &self.ev.zones {
            let Some(z) = self.social.zones.get(i) else { continue };
            let y = zone_height(&spots, (z.x, z.z, z.radius), self.player).ok_or_else(|| format!("Para «{}» guarda un lugar dentro de la zona (hace falta su altura).", z.name))?;
            out.push((z.name.clone(), [z.x, y, z.z]));
        }
        for &i in &self.ev.places {
            if let Some(p) = self.places.get(i) {
                out.push((p.name.clone(), [p.x, p.y, p.z]));
            }
        }
        if out.is_empty() {
            return Err("Elige al menos un jugador, zona o lugar.".to_string());
        }
        Ok(out)
    }

    fn launch_event(&mut self) {
        let Some(entry) = self.creatures.selected.and_then(|i| self.creatures.entries.get(i)) else {
            self.ev.notice = "Elige una criatura en la pestaña Criaturas.".to_string();
            return;
        };
        let (creature, template) = (entry.alias.clone().unwrap_or_else(|| entry.label.clone()), entry.guid.clone());
        let targets = match self.raid_targets() {
            Ok(t) => t,
            Err(e) => {
                self.ev.notice = e;
                return;
            }
        };
        let base = Spawner {
            id: String::new(),
            creature,
            template,
            place: String::new(),
            at: [0.0; 3],
            level: self.level,
            count: self.count,
            health: self.effective_health(),
            damage: self.damage,
            radius: if self.count > 1 { self.distance.clamp(2.0, 60.0) } else { 3.0 },
            respawn: 300,
            once: true,
            drops: self.ev.drops.clone(),
            pickup: if self.ev.drops.is_empty() { String::new() } else { self.items.generic_pickup.clone() },
            pool: Vec::new(),
            every: 0,
            area: 0.0,
        };
        let wave = build_raid(&base, &targets, spawners::now_ms());
        let op = spawners::op_add_many(&wave);
        let n = wave.len();
        self.spawners.extend(wave);
        self.ev.notice = match spawners::save_change(&self.dir, &self.spawners, &op) {
            Ok(()) => format!("Enviado a {n} sitio(s). Aparece cuando haya un jugador a menos de {} m.", spawners::RANGE_METERS as u32),
            Err(e) => format!("Error: {e}"),
        };
    }

    /// Keeps the creature, amount, level and loot currently chosen as one of the (at most 10) templates.
    fn save_template(&mut self) {
        if self.ev.templates.len() >= spawners::MAX_TEMPLATES {
            self.ev.notice = format!("Máximo {} plantillas.", spawners::MAX_TEMPLATES);
            return;
        }
        let Some(entry) = self.creatures.selected.and_then(|i| self.creatures.entries.get(i)) else {
            self.ev.notice = "Elige una criatura en la pestaña Criaturas.".to_string();
            return;
        };
        let creature = entry.alias.clone().unwrap_or_else(|| entry.label.clone());
        let name = if self.ev.name.trim().is_empty() { format!("{} {}", creature, self.ev.templates.len() + 1) } else { self.ev.name.trim().to_string() };
        self.ev.templates.push(spawners::RaidTemplate {
            name,
            template: entry.guid.clone(),
            level: self.level,
            count: self.count,
            health: self.effective_health(),
            damage: self.damage,
            drops: self.ev.drops.clone(),
        });
        self.ev.name.clear();
        self.ev.notice = match spawners::save_templates(&self.dir, &self.ev.templates) {
            Ok(()) => "Plantilla guardada.".to_string(),
            Err(e) => format!("Error: {e}"),
        };
    }

    /// Turns the chosen zones into automatic raid spots: every few minutes one template, picked at
    /// random, appears at a random point inside the zone.
    fn activate_auto(&mut self) {
        if self.ev.templates.is_empty() {
            self.ev.notice = "Crea al menos una plantilla.".to_string();
            return;
        }
        if self.ev.zones.is_empty() {
            self.ev.notice = "Elige al menos una zona.".to_string();
            return;
        }
        let spots: Vec<(f64, f64, f64)> = self.places.iter().map(|p| (p.x, p.y, p.z)).collect();
        let mut wave: Vec<Spawner> = Vec::new();
        let base_ms = spawners::now_ms();
        for (n, &i) in self.ev.zones.iter().enumerate() {
            let Some(z) = self.social.zones.get(i) else { continue };
            let Some(y) = zone_height(&spots, (z.x, z.z, z.radius), self.player) else {
                self.ev.notice = format!("Para «{}» guarda un lugar dentro de la zona (hace falta su altura).", z.name);
                return;
            };
            let id = self.spawners.iter().find(|s| !s.pool.is_empty() && s.place == z.name).map(|s| s.id.clone()).unwrap_or_else(|| format!("s{}", base_ms + n as u128));
            wave.push(automatic_raid(id, &z.name, [z.x, y, z.z], z.radius as f32, self.ev.minutes.clamp(1, 1440) * 60, &self.ev.templates, &self.items.generic_pickup));
        }
        if wave.is_empty() {
            return;
        }
        let op = spawners::op_add_many(&wave);
        let n = wave.len();
        for w in wave {
            match self.spawners.iter().position(|s| s.id == w.id) {
                Some(i) => self.spawners[i] = w,
                None => self.spawners.push(w),
            }
        }
        self.ev.notice = match spawners::save_change(&self.dir, &self.spawners, &op) {
            Ok(()) => format!("Raids automáticos activos en {n} zona(s), cada {} min.", self.ev.minutes),
            Err(e) => format!("Error: {e}"),
        };
    }

    fn add_drop(&mut self) {
        if self.ev.drops.len() >= MAX_DROPS {
            self.ev.notice = format!("Máximo {MAX_DROPS} objetos de botín.");
            return;
        }
        let Some(item) = self.item_cat.selected.and_then(|i| self.items.rows.get(i)).cloned() else {
            self.ev.notice = "Elige un objeto en la pestaña Ítems.".to_string();
            return;
        };
        if !item.deliverable() {
            self.ev.notice = "Ese objeto no se puede entregar.".to_string();
            return;
        }
        self.ev.drops.push(Drop {
            item: item.id,
            guid: item.guid.clone(),
            name: item.name.clone(),
            amount: self.items.amount.clamp(1, 65535),
            rarity: self.items.rarity.min(4),
            level: self.items.level.clamp(1, 255),
            chance: self.ev.chance.clamp(1, 100),
        });
        self.ev.notice.clear();
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
        if self.ev.chance == 0 {
            self.ev.chance = 100;
        }
        if self.ev.minutes == 0 {
            self.ev.minutes = 60;
        }
        if !self.ev.loaded {
            self.ev.loaded = true;
            self.ev.templates = spawners::load_templates(&self.dir);
        }
        heading(ui, "Eventos y raids", 22.0);
        divider(ui);
        let creature = self.creatures.selected.and_then(|i| self.creatures.entries.get(i)).map(|e| e.alias.clone().unwrap_or_else(|| e.label.clone()));
        let summary = match &creature {
            Some(c) => format!("{} × {} · nv.{} (se elige en la pestaña Criaturas)", self.count, c, self.level),
            None => "Elige una criatura en la pestaña Criaturas.".to_string(),
        };
        ui.label(RichText::new(summary).color(PARCHMENT));
        egui::ScrollArea::vertical().show(ui, |ui| {
            label_title(ui, "JUGADORES");
            ui.horizontal_wrapped(|ui| {
                if button(ui, "Actualizar jugadores", Vec2::new(200.0, 28.0), true, false) {
                    let dir = self.dir.clone();
                    self.social.request_roster(&dir);
                }
            });
            let mut any = false;
            for s in &self.social.seen {
                let Some(o) = &s.online else { continue };
                any = true;
                let mut on = self.ev.players.contains(&o.number);
                let place = if o.located() { format!("  X {:.0}  Z {:.0}", o.x, o.z) } else { "  sin posición".to_string() };
                if ui.checkbox(&mut on, format!("{}{}", s.member.name, place)).changed() {
                    if on {
                        self.ev.players.insert(o.number);
                    } else {
                        self.ev.players.remove(&o.number);
                    }
                }
            }
            if !any {
                ui.label(RichText::new("Nadie conectado a la vista.").color(MUTED));
            }
            label_title(ui, "ZONAS Y BASES");
            for (i, z) in self.social.zones.iter().enumerate() {
                let mut on = self.ev.zones.contains(&i);
                if ui.checkbox(&mut on, &z.name).changed() {
                    if on {
                        self.ev.zones.insert(i);
                    } else {
                        self.ev.zones.remove(&i);
                    }
                }
            }
            label_title(ui, "LUGARES GUARDADOS");
            for (i, p) in self.places.iter().enumerate() {
                let mut on = self.ev.places.contains(&i);
                if ui.checkbox(&mut on, &p.name).changed() {
                    if on {
                        self.ev.places.insert(i);
                    } else {
                        self.ev.places.remove(&i);
                    }
                }
            }
            label_title(ui, &format!("BOTÍN DE CADA ENEMIGO ({}/{})", self.ev.drops.len(), MAX_DROPS));
            let mut remove = None;
            for (i, d) in self.ev.drops.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} × {} · {}%", d.amount, d.name, d.chance)).color(PARCHMENT));
                    if button(ui, "Quitar", Vec2::new(70.0, 24.0), true, false) {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                self.ev.drops.remove(i);
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Probabilidad (%)");
                ui.add(egui::Slider::new(&mut self.ev.chance, 1..=100));
                if button(ui, "Añadir el ítem elegido", Vec2::new(210.0, 28.0), true, false) {
                    self.add_drop();
                }
            });
            ui.label(RichText::new("Cantidad, rareza y nivel salen de la pestaña Ítems. Sin botín, los enemigos sueltan lo normal del juego.").color(MUTED).size(14.0));
            label_title(ui, &format!("PLANTILLAS DE RAID ({}/{})", self.ev.templates.len(), spawners::MAX_TEMPLATES));
            let mut drop_template = None;
            for (i, t) in self.ev.templates.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} · {} × nv.{} · botín: {}", t.name, t.count, t.level, t.drops.len())).color(PARCHMENT));
                    if button(ui, "Quitar", Vec2::new(70.0, 24.0), true, false) {
                        drop_template = Some(i);
                    }
                });
            }
            if let Some(i) = drop_template {
                self.ev.templates.remove(i);
                let _ = spawners::save_templates(&self.dir, &self.ev.templates);
            }
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.ev.name).hint_text("Nombre de la plantilla").desired_width(220.0));
                if button(ui, "Guardar plantilla", Vec2::new(180.0, 28.0), creature.is_some(), false) {
                    self.save_template();
                }
            });
            ui.label(RichText::new("Una plantilla guarda la criatura, cantidad, nivel y el botín de arriba.").color(MUTED).size(14.0));
            label_title(ui, "RAIDS AUTOMÁTICOS EN LAS ZONAS ELEGIDAS");
            ui.horizontal_wrapped(|ui| {
                ui.label("Cada (minutos)");
                ui.add(egui::Slider::new(&mut self.ev.minutes, 1..=1440));
            });
            if button(ui, "ACTIVAR EN LAS ZONAS ELEGIDAS", Vec2::new(ui.available_width(), 44.0), !self.ev.templates.is_empty() && !self.ev.zones.is_empty(), true) {
                self.activate_auto();
            }
            ui.label(RichText::new("En cada ciclo se elige una plantilla al azar y aparece en un punto al azar dentro del radio de la zona, si hay un jugador a menos de 150 m. Para quitarlo, bórralo en la lista de apariciones de la pestaña Criaturas.").color(MUTED).size(14.0));
            ui.add_space(8.0);
            let ready = creature.is_some();
            if button(ui, "LANZAR", Vec2::new(ui.available_width(), 52.0), ready, true) {
                self.launch_event();
            }
            if !self.ev.notice.is_empty() {
                ui.label(RichText::new(self.ev.notice.clone()).color(GOOD).size(15.0));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Spawner {
        Spawner {
            id: String::new(),
            creature: "Troll".into(),
            template: "642098d4-fc09-48e2-811e-782d438bf551".into(),
            place: String::new(),
            at: [0.0; 3],
            level: 50,
            count: 3,
            health: 0,
            damage: 1.0,
            radius: 5.0,
            respawn: 300,
            once: true,
            drops: Vec::new(),
            pickup: String::new(),
            pool: Vec::new(),
            every: 0,
            area: 0.0,
        }
    }

    #[test]
    fn one_wave_per_target_with_unique_ids() {
        let wave = build_raid(&base(), &[("Ana".into(), [1.0, 2.0, 3.0]), ("Base norte".into(), [9.0, 8.0, 7.0])], 5000);
        assert_eq!(wave.len(), 2);
        assert_eq!(wave[0].id, "s5000");
        assert_eq!(wave[1].id, "s5001");
        assert_eq!(wave[1].at, [9.0, 8.0, 7.0]);
        assert!(wave.iter().all(|s| s.once && s.count == 3));
        assert_eq!(spawners::created_ms(&wave[1].id), Some(5001));
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
        let t = |n: &str| spawners::RaidTemplate { name: n.into(), template: format!("{n}-guid"), level: 5, count: 2, health: 0, damage: 1.0, drops: Vec::new() };
        let s = automatic_raid("s9".into(), "Base", [1.0, 2.0, 3.0], 40.0, 3600, &[t("a"), t("b")], "pick");
        assert_eq!((s.every, s.area, s.pool.len()), (3600, 40.0, 2));
        assert_eq!(s.template, "a-guid");
        assert!(!s.once);
        let wire = spawners::op_add(&s);
        assert!(wire.contains("\"e\":3600") && wire.contains("\"a\":40"));
    }

}

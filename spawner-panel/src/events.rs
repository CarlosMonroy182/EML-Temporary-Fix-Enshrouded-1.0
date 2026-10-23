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
}

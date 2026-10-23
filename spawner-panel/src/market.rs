use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui::{self, RichText, Vec2};
use serde_json::{json, Value};

use crate::items::{self, RARITIES};
use crate::theme::*;
use crate::{write_atomic, App};

pub const MARKET_FILE: &str = "m182_market.json";
const MAX_GRANTS: usize = 200;

#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub item: u32,
    pub guid: String,
    pub name: String,
    pub price: u64,
    pub level: u32,
    pub rarity: u32,
    pub amount: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grant {
    pub id: String,
    pub to: String,
    pub amount: i64,
}

#[derive(Default)]
pub struct MarketState {
    pub currency: String,
    pub zone: String,
    pub start: u64,
    pub offers: Vec<Offer>,
    pub grants: Vec<Grant>,
    pub search: String,
    pub grant_to: String,
    pub grant_amount: i64,
    pub notice: String,
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(|t| t.as_str()).unwrap_or("").to_string()
}

fn number(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(|t| t.as_u64()).unwrap_or(0)
}

impl MarketState {
    pub fn load(dir: &Path) -> Self {
        let mut state = Self { grant_amount: 100, ..Self::default() };
        let Some(v) = std::fs::read_to_string(dir.join(MARKET_FILE)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return state };
        state.currency = text(&v, "currency");
        state.zone = text(&v, "zone");
        state.start = number(&v, "start");
        for o in v.get("buy").and_then(|b| b.as_array()).into_iter().flatten() {
            state.offers.push(Offer {
                item: number(o, "item") as u32,
                guid: text(o, "guid"),
                name: text(o, "name"),
                price: number(o, "price"),
                level: (number(o, "level") as u32).max(1),
                rarity: (number(o, "rarity") as u32).min(4),
                amount: (number(o, "amount") as u32).max(1),
            });
        }
        for g in v.get("grants").and_then(|b| b.as_array()).into_iter().flatten() {
            state.grants.push(Grant { id: text(g, "id"), to: text(g, "to"), amount: g.get("amount").and_then(|a| a.as_i64()).unwrap_or(0) });
        }
        state
    }

    pub fn to_json(&self, pickup: &str) -> Value {
        json!({
            "currency": self.currency.trim(),
            "pickup": pickup,
            "zone": self.zone.trim(),
            "start": self.start,
            "buy": self.offers.iter().map(|o| json!({
                "item": o.item, "guid": o.guid, "name": o.name.trim(), "price": o.price,
                "level": o.level, "rarity": o.rarity, "amount": o.amount,
            })).collect::<Vec<_>>(),
            "grants": self.grants.iter().map(|g| json!({"id": g.id, "to": g.to, "amount": g.amount})).collect::<Vec<_>>(),
        })
    }

    pub fn add_offer(&mut self, row: &items::Item, name: &str) {
        if self.offers.iter().any(|o| o.item == row.id) {
            return;
        }
        self.offers.push(Offer {
            item: row.id,
            guid: row.guid.clone(),
            name: name.to_string(),
            price: 100,
            level: row.give_level.max(1),
            rarity: row.rarity_index(),
            amount: 1,
        });
    }

    pub fn add_grant(&mut self, to: &str, amount: i64) -> bool {
        let to = to.trim();
        if to.is_empty() || amount == 0 {
            return false;
        }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        self.grants.push(Grant { id: format!("g{stamp}-{}", self.grants.len()), to: to.to_string(), amount });
        if self.grants.len() > MAX_GRANTS {
            self.grants.remove(0);
        }
        true
    }
}

impl App {
    fn save_market(&mut self) {
        let json = self.market.to_json(&self.items.generic_pickup).to_string();
        self.market.notice = match write_atomic(&self.dir, MARKET_FILE, "m182_market.json.tmp", &json) {
            Ok(()) => "Guardado y enviado al servidor.".to_string(),
            Err(e) => format!("Error: {e}"),
        };
    }

    pub fn market_tab(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Mercado", 22.0);
        divider(ui);
        ui.label(RichText::new("Los jugadores usan el chat: !mercado saldo, lista, comprar, pagar. Lo que compran cae junto a ellos.").color(MUTED));
        let mut save = false;
        egui::ScrollArea::vertical().show(ui, |ui| {
            label_title(ui, "MONEDA Y LUGAR");
            ui.horizontal_wrapped(|ui| {
                ui.label("Nombre de la moneda");
                ui.add(egui::TextEdit::singleline(&mut self.market.currency).hint_text("Gold").desired_width(140.0));
                ui.label("Dinero inicial");
                ui.add(egui::DragValue::new(&mut self.market.start).speed(10.0).range(0..=1_000_000));
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Se compra solo en la zona");
                let shown = if self.market.zone.trim().is_empty() { "En cualquier lugar".to_string() } else { self.market.zone.clone() };
                egui::ComboBox::from_id_salt("market_zone").selected_text(shown).show_ui(ui, |ui| {
                    if ui.selectable_label(self.market.zone.trim().is_empty(), "En cualquier lugar").clicked() {
                        self.market.zone.clear();
                    }
                    for z in &self.social.zones {
                        if ui.selectable_label(self.market.zone == z.name, &z.name).clicked() {
                            self.market.zone = z.name.clone();
                        }
                    }
                });
            });
            divider(ui);
            label_title(ui, "LO QUE SE VENDE");
            ui.add(egui::TextEdit::singleline(&mut self.market.search).hint_text("Buscar un ítem para añadir…").desired_width(320.0));
            let query = self.market.search.trim().to_lowercase();
            if query.len() >= 2 {
                let mut picked: Option<(usize, String)> = None;
                for (i, e) in self.item_cat.entries.iter().enumerate().filter(|(_, e)| e.search.contains(&query)).take(8) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(e.alias.clone().unwrap_or_else(|| e.label.clone())).color(PARCHMENT));
                        if !e.meta.is_empty() {
                            ui.label(RichText::new(&e.meta).color(MUTED));
                        }
                        if button(ui, "Añadir", Vec2::new(80.0, 24.0), true, false) {
                            picked = Some((i, e.alias.clone().unwrap_or_else(|| e.label.clone())));
                        }
                    });
                }
                if let Some((i, name)) = picked {
                    let guid = self.item_cat.entries[i].guid.clone();
                    if let Some(row) = self.items.rows.iter().find(|r| r.guid == guid).cloned() {
                        self.market.add_offer(&row, &name);
                        self.market.search.clear();
                    }
                }
            }
            if self.market.offers.is_empty() {
                ui.label(RichText::new("Todavía no vendes nada.").color(MUTED));
            }
            let mut remove = None;
            for (i, o) in self.market.offers.iter_mut().enumerate() {
                ui.group(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut o.name).desired_width(220.0));
                        ui.label("Precio");
                        ui.add(egui::DragValue::new(&mut o.price).speed(1.0).range(1..=1_000_000_000));
                        ui.label("Cantidad");
                        ui.add(egui::DragValue::new(&mut o.amount).speed(1.0).range(1..=1000));
                        ui.label("Nivel");
                        ui.add(egui::DragValue::new(&mut o.level).speed(1.0).range(1..=255));
                        egui::ComboBox::from_id_salt(("market_rarity", i)).selected_text(RARITIES[o.rarity.min(4) as usize]).show_ui(ui, |ui| {
                            for (r, name) in RARITIES.iter().enumerate() {
                                ui.selectable_value(&mut o.rarity, r as u32, *name);
                            }
                        });
                        if button(ui, "Quitar", Vec2::new(80.0, 26.0), true, false) {
                            remove = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = remove {
                self.market.offers.remove(i);
            }
            ui.add_space(6.0);
            if button(ui, "Guardar y enviar al servidor", Vec2::new(280.0, 34.0), true, true) {
                save = true;
            }
            divider(ui);
            label_title(ui, "DAR O QUITAR MONEDAS");
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.market.grant_to).hint_text("ID de Steam o nombre").desired_width(220.0));
                if !self.social.seen.is_empty() {
                    egui::ComboBox::from_id_salt("market_players").selected_text("Jugadores").show_ui(ui, |ui| {
                        for s in &self.social.seen {
                            let value = if s.member.id.is_empty() { s.member.name.clone() } else { s.member.id.clone() };
                            if ui.selectable_label(false, &s.member.name).clicked() {
                                self.market.grant_to = value;
                            }
                        }
                    });
                }
                ui.add(egui::DragValue::new(&mut self.market.grant_amount).speed(10.0).range(-1_000_000..=1_000_000));
                if button(ui, "Aplicar", Vec2::new(100.0, 28.0), true, true) {
                    let (to, amount) = (self.market.grant_to.clone(), self.market.grant_amount);
                    if self.market.add_grant(&to, amount) {
                        save = true;
                    } else {
                        self.market.notice = "Escribe a quién y una cantidad distinta de cero.".to_string();
                    }
                }
            });
            if !self.market.notice.is_empty() {
                ui.label(RichText::new(self.market.notice.clone()).color(GOOD));
            }
        });
        if save {
            self.save_market();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("m182_panel_market_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn json_round_trip_keeps_offers_and_grants() {
        let d = dir("rt");
        let mut m = MarketState { currency: "Gold".into(), zone: "Market".into(), start: 25, ..Default::default() };
        m.offers.push(Offer { item: 7, guid: "g".into(), name: "Sword".into(), price: 50, level: 3, rarity: 2, amount: 1 });
        assert!(m.add_grant("76561198000000001", 120));
        assert!(!m.add_grant("", 5));
        assert!(!m.add_grant("Ana", 0));
        std::fs::write(d.join(MARKET_FILE), m.to_json("pickup-guid").to_string()).unwrap();
        let back = MarketState::load(&d);
        assert_eq!(back.currency, "Gold");
        assert_eq!(back.zone, "Market");
        assert_eq!(back.start, 25);
        assert_eq!(back.offers, m.offers);
        assert_eq!(back.grants, m.grants);
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(d.join(MARKET_FILE)).unwrap()).unwrap();
        assert_eq!(raw["pickup"], "pickup-guid");
        assert_eq!(raw["buy"][0]["price"], 50);
    }

    #[test]
    fn grant_ids_are_unique() {
        let mut m = MarketState::default();
        m.add_grant("Ana", 10);
        m.add_grant("Ana", 10);
        assert_ne!(m.grants[0].id, m.grants[1].id);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let m = MarketState::load(&dir("none"));
        assert!(m.offers.is_empty() && m.grant_amount == 100);
    }
}

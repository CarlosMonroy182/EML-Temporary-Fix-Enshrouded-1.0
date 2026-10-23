use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, RichText, Vec2};
use serde_json::{json, Value};

use crate::items::{self, RARITIES};
use crate::theme::*;
use crate::{write_atomic, App};

pub const MARKET_FILE: &str = "m182_market.json";
pub const LIVE_FILE: &str = "m182_market_live.json";
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LiveEvent {
    pub t: u64,
    pub kind: String,
    pub who: String,
    pub to: String,
    pub item: String,
    pub qty: u32,
    pub amount: i64,
}

/// What the server reports: balances and the latest operations.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Live {
    pub currency: String,
    pub wallets: Vec<(String, u64)>,
    pub log: Vec<LiveEvent>,
    pub seen: String,
}

pub fn parse_live(text: &str) -> Option<Live> {
    let v: Value = serde_json::from_str(text).ok()?;
    let wallets = v.get("wallets")?.as_array()?.iter().filter_map(|w| Some((text_of(w, "name"), w.get("coins")?.as_u64()?))).filter(|(n, _)| !n.is_empty()).collect();
    let log = v
        .get("log")
        .and_then(|l| l.as_array())
        .map(|l| {
            l.iter()
                .map(|e| LiveEvent {
                    t: number(e, "t"),
                    kind: text_of(e, "kind"),
                    who: text_of(e, "who"),
                    to: text_of(e, "to"),
                    item: text_of(e, "item"),
                    qty: number(e, "qty") as u32,
                    amount: e.get("amount").and_then(|a| a.as_i64()).unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Live { currency: text_of(&v, "currency"), wallets, log, seen: String::new() })
}

fn text_of(v: &Value, key: &str) -> String {
    text(v, key)
}

fn ago(t: u64) -> String {
    if t == 0 {
        return String::new();
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(t);
    let secs = now.saturating_sub(t);
    if secs < 90 {
        "ahora".to_string()
    } else if secs < 5400 {
        format!("{} min", secs / 60)
    } else {
        format!("{} h", secs / 3600)
    }
}

#[derive(Default)]
pub struct MarketState {
    pub live: Live,
    pub asked: Option<Instant>,
    pub last_asked: Option<Instant>,
    pub filter: String,
    pub group: String,
    pub rarity: usize,
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
            Ok(()) => {
                let now = Instant::now();
                self.market.last_asked = Some(now.checked_sub(Duration::from_secs(45)).unwrap_or(now));
                "Guardado y enviado al servidor.".to_string()
            }
            Err(e) => format!("Error: {e}"),
        };
    }

    /// Keeps balances and recent operations current: in a local game the server's file is read,
    /// against a server the answer to a chat question is asked while this tab is open.
    pub fn market_live_tick(&mut self) {
        let remote = crate::remote::active();
        let path = self.dir.join(LIVE_FILE);
        if remote && self.market.asked.is_some_and(|at| at.elapsed() > Duration::from_secs(75)) {
            self.market.asked = None;
        }
        if !remote || self.market.asked.is_some() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if text != self.market.live.seen {
                    if let Some(mut live) = parse_live(&text) {
                        live.seen = text;
                        self.market.live = live;
                        self.market.asked = None;
                    }
                } else if remote {
                    self.market.asked = None;
                }
            }
        }
        let idle = self.social.roster_asked.is_none() && self.social.teams_asked.is_none() && self.market.asked.is_none();
        let due = self.market.last_asked.map_or(true, |t| t.elapsed() >= Duration::from_secs(60));
        if remote && idle && due && self.tab == crate::Tab::Market && crate::remote::connected() && crate::remote::status().queued == 0 {
            let seq = (SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(1) % 1_000_000) as u32 + 1;
            let _ = std::fs::remove_file(&path);
            let scan = json!({"seq": seq, "kind": "market"}).to_string();
            if write_atomic(&self.dir, crate::remote::SCAN_FILE, "m182_scan.request.tmp", &scan).is_ok() {
                crate::remote::submit_wallets(seq);
                self.market.asked = Some(Instant::now());
            }
            self.market.last_asked = Some(Instant::now());
        }
    }

    fn event_text(&self, e: &LiveEvent) -> String {
        let coin = if self.market.live.currency.is_empty() { "Gold".to_string() } else { self.market.live.currency.clone() };
        let when = ago(e.t);
        let what = match e.kind.as_str() {
            "buy" => format!("{} {} {}x {} ({} {})", e.who, "compró", e.qty, e.item, e.amount, coin),
            "pay" => format!("{} {} {} {} → {}", e.who, "pagó", e.amount, coin, e.to),
            _ => format!("{} {} {} {} {}", "Administrador:", if e.amount >= 0 { "+" } else { "" }, e.amount, coin, format!("→ {}", e.to)),
        };
        if when.is_empty() { what } else { format!("{when} · {what}") }
    }

    pub fn market_tab(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Mercado", 22.0);
        divider(ui);
        ui.label(RichText::new("Los jugadores usan el chat: !mercado saldo, lista, comprar, pagar. Lo que compran cae junto a ellos.").color(MUTED));
        let mut save = false;
        let mut pick_player: Option<String> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
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

            label_title(ui, "CATÁLOGO DE ÍTEMS");
            let mut groups: Vec<String> = self.item_cat.entries.iter().filter(|e| !e.internal).map(|e| e.group.clone()).collect();
            groups.sort();
            groups.dedup();
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.market.filter).hint_text("Buscar por nombre…").desired_width(260.0));
                let shown = if self.market.group.is_empty() { "Todas las categorías".to_string() } else { self.market.group.clone() };
                egui::ComboBox::from_id_salt("market_group").selected_text(shown).show_ui(ui, |ui| {
                    if ui.selectable_label(self.market.group.is_empty(), "Todas las categorías").clicked() {
                        self.market.group.clear();
                    }
                    for g in &groups {
                        if ui.selectable_label(self.market.group == *g, g).clicked() {
                            self.market.group = g.clone();
                        }
                    }
                });
                let shown = if self.market.rarity == 0 { "Cualquier rareza" } else { RARITIES[(self.market.rarity - 1).min(4)] };
                egui::ComboBox::from_id_salt("market_rarity_filter").selected_text(shown).show_ui(ui, |ui| {
                    if ui.selectable_label(self.market.rarity == 0, "Cualquier rareza").clicked() {
                        self.market.rarity = 0;
                    }
                    for (i, name) in RARITIES.iter().enumerate() {
                        if ui.selectable_label(self.market.rarity == i + 1, *name).clicked() {
                            self.market.rarity = i + 1;
                        }
                    }
                });
            });
            let query = self.market.filter.trim().to_lowercase();
            let matches: Vec<usize> = self
                .item_cat
                .entries
                .iter()
                .enumerate()
                .filter(|(i, e)| {
                    !e.internal
                        && (query.is_empty() || e.search.contains(&query))
                        && (self.market.group.is_empty() || e.group == self.market.group)
                        && (self.market.rarity == 0 || self.items.rows.get(*i).is_some_and(|r| r.guid == e.guid && r.rarity_index() as usize + 1 == self.market.rarity))
                })
                .map(|(i, _)| i)
                .collect();
            ui.label(RichText::new(format!("{} {}", matches.len(), "resultados")).color(MUTED));
            let mut picked: Option<(usize, String)> = None;
            egui::ScrollArea::vertical().id_salt("market_catalog").max_height(300.0).auto_shrink([false, false]).show_rows(ui, 30.0, matches.len(), |ui, range| {
                for row in range {
                    let i = matches[row];
                    let e = &self.item_cat.entries[i];
                    let name = e.alias.clone().unwrap_or_else(|| e.label.clone());
                    let on_sale = self.market.offers.iter().any(|o| self.items.rows.get(i).is_some_and(|r| r.id == o.item));
                    ui.horizontal(|ui| {
                        ui.add_sized([300.0, 24.0], egui::Label::new(RichText::new(&name).size(16.0).color(PARCHMENT)).truncate());
                        ui.label(RichText::new(&e.meta).size(14.0).color(MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if on_sale {
                                ui.label(RichText::new("✓ En venta").color(GOOD));
                            } else if button(ui, "Añadir", Vec2::new(90.0, 24.0), true, false) {
                                picked = Some((i, name.clone()));
                            }
                        });
                    });
                }
            });
            if let Some((i, name)) = picked {
                if let Some(row) = self.items.rows.get(i).cloned() {
                    self.market.add_offer(&row, &name);
                    self.market.notice = format!("{} {}", name, "añadido a la venta. Ajusta el precio abajo y guarda.");
                }
            }
            divider(ui);

            label_title(ui, "A LA VENTA");
            if self.market.offers.is_empty() {
                ui.label(RichText::new("Todavía no vendes nada. Añade ítems desde el catálogo de arriba.").color(MUTED));
            }
            let mut remove = None;
            for (i, o) in self.market.offers.iter_mut().enumerate() {
                ui.group(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut o.name).desired_width(240.0));
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

            label_title(ui, "SALDOS DE LOS JUGADORES");
            if self.market.live.wallets.is_empty() {
                ui.label(RichText::new("Todavía no hay saldos. Aparecen cuando alguien usa el mercado o recibe monedas.").color(MUTED));
            }
            for (name, coins) in &self.market.live.wallets {
                ui.horizontal(|ui| {
                    ui.add_sized([240.0, 22.0], egui::Label::new(RichText::new(name).size(16.0).color(PARCHMENT)).truncate());
                    ui.label(RichText::new(coins.to_string()).size(16.0).color(GOLD));
                    if ui.small_button("Elegir").clicked() {
                        pick_player = Some(name.clone());
                    }
                });
            }
            ui.add_space(6.0);
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
                        self.market.notice = format!("{} {} → {}", "Enviado al servidor:", amount, to.trim());
                        save = true;
                    } else {
                        self.market.notice = "Escribe a quién y una cantidad distinta de cero.".to_string();
                    }
                }
            });
            divider(ui);

            label_title(ui, "ACTIVIDAD RECIENTE");
            if self.market.live.log.is_empty() {
                ui.label(RichText::new("Sin movimientos todavía.").color(MUTED));
            }
            for e in self.market.live.log.iter().take(20) {
                ui.label(RichText::new(self.event_text(e)).size(15.0).color(PARCHMENT));
            }
            if !self.market.notice.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(self.market.notice.clone()).color(GOOD));
            }
        });
        if let Some(name) = pick_player {
            self.market.grant_to = name;
        }
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

    #[test]
    fn live_reply_from_the_server_is_read() {
        let text = r#"{"currency":"Gold","wallets":[{"key":"name:ana","name":"Ana","coins":60},{"key":"","name":"Luis","coins":5}],"log":[{"t":0,"kind":"buy","who":"Ana","to":"","item":"Sword","qty":1,"amount":50},{"t":0,"kind":"grant","who":"","to":"Ana","item":"","qty":0,"amount":100}]}"#;
        let live = parse_live(text).unwrap();
        assert_eq!(live.currency, "Gold");
        assert_eq!(live.wallets, vec![("Ana".to_string(), 60), ("Luis".to_string(), 5)]);
        assert_eq!(live.log.len(), 2);
        assert_eq!(live.log[0].kind, "buy");
        assert_eq!(live.log[1].amount, 100);
        assert!(parse_live("basura").is_none());
    }
}

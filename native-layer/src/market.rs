//! The market: players hold a balance of coins (kept on the server), see the catalog and buy
//! items with chat commands, and can pay each other. Bought items are dropped next to the
//! buyer. The admin sets currency, catalog, prices and grants from the panel
//! (`m182_market.json`); balances live in `m182_wallets.json`, which only this module writes.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use serde::Deserialize;

use super::invites::Person;
use super::roster::{clean_typed_name, same_name};

pub const PREFIXES: [&str; 4] = ["!mercado", "/mercado", "!market", "/market"];
pub const CONFIG_FILE: &str = "m182_market.json";
pub const WALLETS_FILE: &str = "m182_wallets.json";
pub const GENERIC_PICKUP: &str = "c15a5c9d-4956-49ff-9d72-9f522229cf1c";
const PAGE: usize = 5;
const MAX_QTY: u32 = 20;
const MAX_COINS: u64 = 1_000_000_000_000;
const MAX_APPLIED: usize = 2000;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Offer {
    pub item: u32,
    pub guid: String,
    pub name: String,
    pub price: u64,
    pub level: u32,
    pub rarity: u32,
    pub amount: u32,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Grant {
    pub id: String,
    /// Steam id, or a player name.
    pub to: String,
    pub amount: i64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub currency: String,
    pub pickup: String,
    /// Name of the zone (from the zones file) where buying is allowed; empty = anywhere.
    pub zone: String,
    pub start: u64,
    pub buy: Vec<Offer>,
    pub grants: Vec<Grant>,
}

impl Config {
    fn coin(&self) -> &str {
        if self.currency.trim().is_empty() { "Gold" } else { self.currency.trim() }
    }

    pub fn pickup_template(&self) -> &str {
        if self.pickup.trim().is_empty() { GENERIC_PICKUP } else { self.pickup.trim() }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Wallet {
    pub name: String,
    pub coins: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct WalletsFile {
    wallets: std::collections::BTreeMap<String, Wallet>,
    applied: Vec<String>,
}

/// What the game has to do after a purchase.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    pub entity: u32,
    pub offer: Offer,
    pub copies: u32,
    pub template: String,
}

pub struct Reply {
    pub line: String,
    pub deliveries: Vec<Delivery>,
}

static PRESENT: Mutex<Option<HashMap<u32, Vec<String>>>> = Mutex::new(None);

/// Called by the player tracker: the zones a player is standing in.
pub fn note_position(entity: u32, zones: Vec<String>) {
    if let Ok(mut g) = PRESENT.lock() {
        g.get_or_insert_with(HashMap::new).insert(entity, zones);
    }
}

fn standing_in(entity: u32, zone: &str) -> bool {
    PRESENT.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&entity).cloned())).is_some_and(|zs| zs.iter().any(|z| same_name(z, zone)))
}

pub fn is_command(text: &str) -> bool {
    let t = text.trim_start();
    PREFIXES.iter().any(|p| {
        t.len() >= p.len() && t.is_char_boundary(p.len()) && t[..p.len()].eq_ignore_ascii_case(p) && t[p.len()..].chars().next().is_none_or(char::is_whitespace)
    })
}

fn next_token(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

fn wallet_keys(p: &Person) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(id) = p.id.as_deref().map(|i| i.trim().to_lowercase()).filter(|i| !i.is_empty()) {
        keys.push(id);
    }
    keys.push(format!("name:{}", p.name.trim().to_lowercase()));
    keys
}

fn read_config(dir: &Path) -> Config {
    std::fs::read_to_string(dir.join(CONFIG_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn read_wallets(dir: &Path) -> WalletsFile {
    std::fs::read_to_string(dir.join(WALLETS_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_wallets(dir: &Path, file: &WalletsFile) -> Result<(), String> {
    let body = serde_json::json!({ "wallets": file.wallets.iter().map(|(k, w)| (k.clone(), serde_json::json!({"name": w.name, "coins": w.coins}))).collect::<serde_json::Map<_, _>>(), "applied": file.applied });
    let tmp = dir.join("m182_wallets.json.tmp");
    std::fs::write(&tmp, body.to_string()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dir.join(WALLETS_FILE)).map_err(|e| e.to_string())
}

/// The key under which this player's wallet is stored (existing one first).
fn existing_key(file: &WalletsFile, p: &Person) -> Option<String> {
    wallet_keys(p).into_iter().find(|k| file.wallets.contains_key(k))
}

fn primary_key(p: &Person) -> String {
    wallet_keys(p).remove(0)
}

/// Applies grants not seen before (each has a unique id) and returns true when something changed.
fn apply_grants(cfg: &Config, file: &mut WalletsFile) -> bool {
    let mut changed = false;
    for g in &cfg.grants {
        if g.id.trim().is_empty() || file.applied.iter().any(|a| a == &g.id) || g.to.trim().is_empty() {
            continue;
        }
        let to = g.to.trim();
        let key = if to.chars().all(|c| c.is_ascii_digit()) && to.len() >= 8 { to.to_lowercase() } else { format!("name:{}", to.to_lowercase()) };
        let wallet = file.wallets.entry(key).or_insert_with(|| Wallet { name: to.to_string(), coins: cfg.start });
        wallet.coins = if g.amount >= 0 { wallet.coins.saturating_add(g.amount as u64).min(MAX_COINS) } else { wallet.coins.saturating_sub(g.amount.unsigned_abs()) };
        file.applied.push(g.id.clone());
        if file.applied.len() > MAX_APPLIED {
            file.applied.remove(0);
        }
        changed = true;
    }
    changed
}

struct Words {
    spanish: bool,
}

impl Words {
    fn pick<'a>(&self, es: &'a str, en: &'a str) -> &'a str {
        if self.spanish { es } else { en }
    }
}

fn find_offer(cfg: &Config, what: &str) -> Result<usize, bool> {
    let what = what.trim();
    if what.is_empty() {
        return Err(false);
    }
    if let Ok(n) = what.parse::<usize>() {
        return if n >= 1 && n <= cfg.buy.len() { Ok(n - 1) } else { Err(false) };
    }
    let lower = what.to_lowercase();
    if let Some(i) = cfg.buy.iter().position(|o| o.name.to_lowercase() == lower) {
        return Ok(i);
    }
    let hits: Vec<usize> = cfg.buy.iter().enumerate().filter(|(_, o)| o.name.to_lowercase().contains(&lower)).map(|(i, _)| i).collect();
    match hits.as_slice() {
        [one] => Ok(*one),
        [] => Err(false),
        _ => Err(true),
    }
}

fn find_online<'a>(online: &'a [Person], typed: &str) -> Result<&'a Person, bool> {
    let typed = clean_typed_name(typed);
    let typed = typed.as_str();
    if typed.is_empty() {
        return Err(false);
    }
    if let Some(p) = online.iter().find(|p| same_name(&p.name, typed)) {
        return Ok(p);
    }
    let lower = typed.to_lowercase();
    for hits in [
        online.iter().filter(|p| p.name.to_lowercase().starts_with(&lower)).collect::<Vec<&Person>>(),
        online.iter().filter(|p| p.name.to_lowercase().contains(&lower)).collect::<Vec<&Person>>(),
    ] {
        match hits.as_slice() {
            [one] => return Ok(one),
            [] => {}
            _ => return Err(true),
        }
    }
    Err(false)
}

pub fn handle(text: &str, who: &Person, online: &[Person], dir: &Path) -> Option<Reply> {
    handle_with(text, who, online, dir)
}

pub fn handle_with(text: &str, who: &Person, online: &[Person], dir: &Path) -> Option<Reply> {
    let (head, rest) = next_token(text.trim_start());
    let w = Words { spanish: head.to_ascii_lowercase().ends_with("mercado") };
    let (sub, rest) = next_token(rest);
    let sub = sub.to_ascii_lowercase();
    let cfg = read_config(dir);
    let mut file = read_wallets(dir);
    if apply_grants(&cfg, &mut file) {
        let _ = write_wallets(dir, &file);
    }
    let coin = cfg.coin().to_string();
    let tag = "[Market]";
    let say = |s: String| Some(Reply { line: s, deliveries: Vec::new() });
    let balance_of = |file: &WalletsFile| existing_key(file, who).and_then(|k| file.wallets.get(&k)).map(|w| w.coins).unwrap_or(cfg.start);

    match sub.as_str() {
        "" | "help" | "ayuda" => say(
            w.pick(
                "[Mercado] !mercado saldo | lista [página] | comprar número o nombre [cantidad] | pagar jugador monto",
                "[Market] !market balance | list [page] | buy number or name [quantity] | pay player amount",
            )
            .to_string(),
        ),
        "balance" | "saldo" | "bal" => say(format!("{} {}: {} {}", w.pick("[Mercado]", tag), who.name, balance_of(&file), coin)),
        "list" | "lista" | "catalogo" | "catálogo" | "shop" | "tienda" => {
            if cfg.buy.is_empty() {
                return say(w.pick("[Mercado] El catálogo está vacío.", "[Market] The catalog is empty.").to_string());
            }
            let pages = cfg.buy.len().div_ceil(PAGE);
            let page = next_token(rest).0.parse::<usize>().unwrap_or(1).clamp(1, pages);
            let from = (page - 1) * PAGE;
            let items: Vec<String> = cfg.buy.iter().enumerate().skip(from).take(PAGE).map(|(i, o)| {
                let each = if o.amount > 1 { format!(" x{}", o.amount) } else { String::new() };
                format!("{}) {}{} {}", i + 1, o.name, each, o.price)
            }).collect();
            say(format!("{} {} ({}/{}): {}", w.pick("[Mercado]", tag), coin, page, pages, items.join(" | ")))
        }
        "buy" | "comprar" => {
            let mut words: Vec<&str> = rest.split_whitespace().collect();
            let mut qty = 1u32;
            if words.len() >= 2 {
                if let Ok(n) = words[words.len() - 1].parse::<u32>() {
                    qty = n;
                    words.pop();
                }
            }
            let what = words.join(" ");
            if what.is_empty() {
                return say(w.pick("[Mercado] Uso: !mercado comprar número o nombre [cantidad]", "[Market] Use: !market buy number or name [quantity]").to_string());
            }
            if !(1..=MAX_QTY).contains(&qty) {
                return say(format!("{} {}.", w.pick("[Mercado] La cantidad va de 1 a", "[Market] Quantity must be 1 to"), MAX_QTY));
            }
            if !cfg.zone.trim().is_empty() && !standing_in(who.entity, &cfg.zone) {
                return say(format!("{} {}.", w.pick("[Mercado] Solo se compra dentro de la zona", "[Market] You can only buy inside the zone"), cfg.zone.trim()));
            }
            let index = match find_offer(&cfg, &what) {
                Ok(i) => i,
                Err(true) => return say(w.pick("[Mercado] Hay varios con ese nombre; usa el número.", "[Market] Several match; use the number.").to_string()),
                Err(false) => return say(w.pick("[Mercado] No encuentro ese artículo. Mira !mercado lista", "[Market] I cannot find that item. See !market list").to_string()),
            };
            let offer = cfg.buy[index].clone();
            if offer.item == 0 || offer.price == 0 {
                return say(w.pick("[Mercado] Ese artículo no está disponible.", "[Market] That item is not available.").to_string());
            }
            let total = offer.price.saturating_mul(qty as u64);
            let key = existing_key(&file, who).unwrap_or_else(|| primary_key(who));
            let have = file.wallets.get(&key).map(|w| w.coins).unwrap_or(cfg.start);
            if have < total {
                return say(format!("{} {} {}, {} {}.", w.pick("[Mercado] Necesitas", "[Market] You need"), total, coin, w.pick("tienes", "you have"), have));
            }
            file.wallets.insert(key, Wallet { name: who.name.clone(), coins: have - total });
            if let Err(e) = write_wallets(dir, &file) {
                return say(format!("[M182] {}: {e}", w.pick("no se pudo guardar", "could not save")));
            }
            let line = format!(
                "{} {} {} {}x {} ({} {}). {}: {}",
                w.pick("[Mercado]", tag),
                who.name,
                w.pick("compró", "bought"),
                qty,
                offer.name,
                total,
                coin,
                w.pick("Saldo", "Balance"),
                have - total
            );
            Some(Reply { line, deliveries: vec![Delivery { entity: who.entity, offer, copies: qty, template: cfg.pickup_template().to_string() }] })
        }
        "pay" | "pagar" | "dar" => {
            let (amount_text, name) = {
                let words: Vec<&str> = rest.split_whitespace().collect();
                match words.last().and_then(|l| l.parse::<u64>().ok()) {
                    Some(a) => (a, words[..words.len() - 1].join(" ")),
                    None => match words.first().and_then(|f| f.parse::<u64>().ok()) {
                        Some(a) => (a, words[1..].join(" ")),
                        None => return say(w.pick("[Mercado] Uso: !mercado pagar jugador monto", "[Market] Use: !market pay player amount").to_string()),
                    },
                }
            };
            if amount_text == 0 {
                return say(w.pick("[Mercado] El monto debe ser mayor que cero.", "[Market] The amount must be above zero.").to_string());
            }
            let target = match find_online(online, &name) {
                Ok(t) => t,
                Err(true) => return say(w.pick("[Mercado] Hay varios jugadores con ese nombre; escríbelo completo.", "[Market] Several players match; type the full name.").to_string()),
                Err(false) => return say(w.pick("[Mercado] No encuentro a ese jugador conectado.", "[Market] I cannot find that player online.").to_string()),
            };
            if target.entity == who.entity {
                return say(w.pick("[Mercado] No puedes pagarte a ti mismo.", "[Market] You cannot pay yourself.").to_string());
            }
            let from_key = existing_key(&file, who).unwrap_or_else(|| primary_key(who));
            let have = file.wallets.get(&from_key).map(|w| w.coins).unwrap_or(cfg.start);
            if have < amount_text {
                return say(format!("{} {}.", w.pick("[Mercado] No te alcanza; tienes", "[Market] Not enough; you have"), have));
            }
            file.wallets.insert(from_key, Wallet { name: who.name.clone(), coins: have - amount_text });
            let to_key = existing_key(&file, target).unwrap_or_else(|| primary_key(target));
            let to_have = file.wallets.get(&to_key).map(|w| w.coins).unwrap_or(cfg.start);
            file.wallets.insert(to_key, Wallet { name: target.name.clone(), coins: to_have.saturating_add(amount_text).min(MAX_COINS) });
            if let Err(e) = write_wallets(dir, &file) {
                return say(format!("[M182] {}: {e}", w.pick("no se pudo guardar", "could not save")));
            }
            say(format!("{} {} {} {} {} {}.", w.pick("[Mercado]", tag), who.name, w.pick("pagó", "paid"), amount_text, coin, format_args!("{} {}", w.pick("a", "to"), target.name)))
        }
        _ => say(w.pick("[Mercado] No entiendo. Escribe !mercado ayuda", "[Market] I do not understand. Type !market help").to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("m182_market_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn person(entity: u32, name: &str, id: Option<&str>) -> Person {
        Person { entity, name: name.to_string(), id: id.map(str::to_string) }
    }

    fn config(d: &Path, extra: &str) {
        let body = format!(
            r#"{{"currency":"Gold","start":0,"buy":[{{"item":11,"name":"Iron Sword","price":50,"amount":1}},{{"item":12,"name":"Healing Potion","price":10,"amount":5}},{{"item":13,"name":"Iron Shield","price":30}}],"grants":[{{"id":"g1","to":"76561198000000001","amount":120}},{{"id":"g2","to":"Bea","amount":15}}]{extra}}}"#
        );
        std::fs::write(d.join(CONFIG_FILE), body).unwrap();
    }

    fn run(d: &Path, who: &Person, online: &[Person], text: &str) -> Reply {
        handle_with(text, who, online, d).unwrap()
    }

    #[test]
    fn grants_apply_once_and_balance_shows() {
        let d = dir("grants");
        config(&d, "");
        let ana = person(1, "Ana", Some("76561198000000001"));
        let online = vec![ana.clone()];
        assert!(run(&d, &ana, &online, "!market balance").line.contains("120 Gold"));
        assert!(run(&d, &ana, &online, "!mercado saldo").line.contains("120 Gold"));
        assert!(run(&d, &ana, &online, "!market balance").line.contains("120 Gold"));
    }

    #[test]
    fn buying_charges_and_queues_the_items() {
        let d = dir("buy");
        config(&d, "");
        let ana = person(1, "Ana", Some("76561198000000001"));
        let online = vec![ana.clone()];
        let r = run(&d, &ana, &online, "!market buy iron sword");
        assert!(r.line.contains("bought 1x Iron Sword (50 Gold)") && r.line.contains("Balance: 70"), "{}", r.line);
        assert_eq!(r.deliveries.len(), 1);
        assert_eq!(r.deliveries[0].offer.item, 11);
        assert_eq!(r.deliveries[0].entity, 1);
        let r = run(&d, &ana, &online, "!mercado comprar 2 3");
        assert!(r.line.contains("compró 3x Healing Potion (30 Gold)") && r.line.contains("Saldo: 40"), "{}", r.line);
        assert_eq!(r.deliveries[0].copies, 3);
        let r = run(&d, &ana, &online, "!market buy 1 5");
        assert!(r.line.contains("You need 250") && r.deliveries.is_empty(), "{}", r.line);
        assert!(run(&d, &ana, &online, "!market balance").line.contains("40 Gold"));
    }

    #[test]
    fn ambiguous_and_unknown_items() {
        let d = dir("amb");
        config(&d, "");
        let ana = person(1, "Ana", Some("76561198000000001"));
        let online = vec![ana.clone()];
        assert!(run(&d, &ana, &online, "!market buy iron").line.contains("Several match"));
        assert!(run(&d, &ana, &online, "!market buy dragon").line.contains("cannot find"));
        assert!(run(&d, &ana, &online, "!market buy 9").line.contains("cannot find"));
        assert!(run(&d, &ana, &online, "!market buy 1 99").line.contains("1 to 20"));
    }

    #[test]
    fn paying_moves_coins_between_players() {
        let d = dir("pay");
        config(&d, "");
        let ana = person(1, "Ana", Some("76561198000000001"));
        let bea = person(2, "Bea", None);
        let online = vec![ana.clone(), bea.clone()];
        let r = run(&d, &ana, &online, "!market pay Bea 20");
        assert!(r.line.contains("paid 20 Gold to Bea"), "{}", r.line);
        assert!(run(&d, &bea, &online, "!market balance").line.contains("35 Gold"));
        assert!(run(&d, &ana, &online, "!market balance").line.contains("100 Gold"));
        assert!(run(&d, &ana, &online, "!market pay <Bea> 1").line.contains("paid 1 Gold to Bea"));
        assert!(run(&d, &ana, &online, "!market pay Ana 5").line.contains("yourself"));
        assert!(run(&d, &ana, &online, "!market pay Bea 500").line.contains("Not enough"));
        assert!(run(&d, &ana, &online, "!market pay Zed 5").line.contains("cannot find"));
    }

    #[test]
    fn list_pages_and_spanish() {
        let d = dir("list");
        config(&d, "");
        let ana = person(1, "Ana", None);
        let r = run(&d, &ana, &[ana.clone()], "!mercado lista");
        assert!(r.line.starts_with("[Mercado] Gold (1/1): 1) Iron Sword 50 | 2) Healing Potion x5 10 | 3) Iron Shield 30"), "{}", r.line);
    }

    #[test]
    fn zone_rule_blocks_buying_elsewhere() {
        let d = dir("zone");
        config(&d, r#","zone":"Market""#);
        let ana = person(77, "Ana", Some("76561198000000001"));
        let online = vec![ana.clone()];
        assert!(run(&d, &ana, &online, "!market buy 3").line.contains("inside the zone Market"));
        note_position(77, vec!["market".to_string()]);
        assert!(run(&d, &ana, &online, "!market buy 3").line.contains("bought"));
        note_position(77, Vec::new());
        assert!(run(&d, &ana, &online, "!market buy 3").line.contains("inside the zone"));
    }

    #[test]
    fn command_detection() {
        assert!(is_command("!market list") && is_command("/MERCADO") && !is_command("!marketing") && !is_command("hello !market"));
    }
}

//! Unique item copies: every weapon the admin edits is delivered as its own copy of the item
//! (own id, own stats) so the original and every other copy keep their values.
//!
//! The game only knows the items that exist when it starts, so each edited item gets a few spare
//! copies ("pool") created at start by the `zz_m182_item_stats` mod; editing and delivering then
//! only fills a free copy with stats, live.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{json, Value};

pub const FILE: &str = "m182_copies.json";
pub const READY_FILE: &str = "export/m182_custom_items.csv";
pub const SLOTS: u32 = 8;
pub const MAX_SLOTS: u32 = 64;

pub fn item_id(base: u32, slot: u32) -> u32 {
    ((base as u64 + slot as u64 * 2_654_435_761 + 182) % 4_294_967_296) as u32
}

pub fn item_guid(base_guid: &str, slot: u32) -> Option<String> {
    let parts: Vec<&str> = base_guid.trim().split('-').collect();
    if parts.len() != 5 || parts.iter().any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_hexdigit())) {
        return None;
    }
    Some(format!("{}-{}-5182-{:04x}-{}", parts[0], parts[1], slot, parts[4]))
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Used {
    pub name: String,
    pub dmg: Option<[f32; 15]>,
    pub arm: Option<[f32; 9]>,
    pub lvl: (u32, u32),
    /// False once the copy was released: the game may still hold these values until it restarts.
    pub active: bool,
}

impl Used {
    pub fn same_stats(&self, other: &Used) -> bool {
        self.dmg == other.dmg && self.arm == other.arm && self.lvl == other.lvl
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pool {
    pub slots: u32,
    pub pending: bool,
    pub used: BTreeMap<u32, Used>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Book {
    pub pools: BTreeMap<u32, Pool>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    NoPool,
    Pending,
    Full,
    Use { slot: u32, fresh: bool },
}

pub fn csv_numbers<const N: usize>(values: &[f32; N]) -> String {
    values.iter().map(|v| format!("{}", if v.is_finite() { *v } else { 0.0 })).collect::<Vec<_>>().join(",")
}

fn numbers<const N: usize>(value: &Value) -> Option<[f32; N]> {
    let list = value.as_array()?;
    if list.len() != N {
        return None;
    }
    let mut out = [0f32; N];
    for (slot, v) in out.iter_mut().zip(list) {
        *slot = v.as_f64()? as f32;
    }
    Some(out)
}

impl Book {
    pub fn load(dir: &Path) -> Book {
        let Some(root) = std::fs::read_to_string(dir.join(FILE)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return Book::default() };
        let mut book = Book::default();
        let Some(pools) = root.get("pools").and_then(|p| p.as_object()) else { return book };
        for (key, pool) in pools {
            let Ok(base) = key.parse::<u32>() else { continue };
            let mut used = BTreeMap::new();
            if let Some(map) = pool.get("used").and_then(|u| u.as_object()) {
                for (slot, entry) in map {
                    let Ok(slot) = slot.parse::<u32>() else { continue };
                    let lvl = entry.get("lvl").and_then(|l| l.as_array()).map(|l| (l.first().and_then(Value::as_u64).unwrap_or(0) as u32, l.get(1).and_then(Value::as_u64).unwrap_or(0) as u32)).unwrap_or((0, 0));
                    used.insert(
                        slot,
                        Used {
                            name: entry.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                            dmg: entry.get("dmg").and_then(numbers::<15>),
                            arm: entry.get("arm").and_then(numbers::<9>),
                            lvl,
                            active: entry.get("active").and_then(Value::as_bool).unwrap_or(true),
                        },
                    );
                }
            }
            book.pools.insert(
                base,
                Pool {
                    slots: pool.get("slots").and_then(Value::as_u64).unwrap_or(SLOTS as u64).clamp(1, MAX_SLOTS as u64) as u32,
                    pending: pool.get("pending").and_then(Value::as_bool).unwrap_or(false),
                    used,
                },
            );
        }
        book
    }

    pub fn to_json(&self) -> Value {
        let mut pools = serde_json::Map::new();
        for (base, pool) in &self.pools {
            let mut used = serde_json::Map::new();
            for (slot, u) in &pool.used {
                used.insert(slot.to_string(), json!({"name": u.name, "dmg": u.dmg.map(|d| d.to_vec()), "arm": u.arm.map(|a| a.to_vec()), "lvl": [u.lvl.0, u.lvl.1], "active": u.active}));
            }
            pools.insert(base.to_string(), json!({"slots": pool.slots, "pending": pool.pending, "used": used}));
        }
        json!({"pools": pools})
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&self.to_json()).map_err(|e| e.to_string())?;
        crate::write_atomic(dir, FILE, "m182_copies.json.tmp", &text)
    }

    /// `ready` lists the copies the running game reports having created; `None` when that cannot
    /// be known (the panel drives a server), in which case the pool's own flag decides.
    pub fn plan(&self, base: u32, want: &Used, ready: Option<&BTreeSet<(u32, u32)>>) -> Plan {
        let Some(pool) = self.pools.get(&base) else { return Plan::NoPool };
        let exists = |slot: u32| match ready {
            Some(set) => set.contains(&(base, slot)),
            None => !pool.pending,
        };
        if !(1..=pool.slots).any(exists) {
            return Plan::Pending;
        }
        if let Some((slot, _)) = pool.used.iter().find(|(slot, u)| exists(**slot) && u.active && u.same_stats(want)) {
            return Plan::Use { slot: *slot, fresh: false };
        }
        match (1..=pool.slots).find(|slot| exists(*slot) && !pool.used.get(slot).is_some_and(|u| u.active)) {
            Some(slot) => Plan::Use { slot, fresh: true },
            None => Plan::Full,
        }
    }

    pub fn register(&mut self, base: u32, slots: u32) -> bool {
        let slots = slots.clamp(1, MAX_SLOTS);
        match self.pools.get_mut(&base) {
            Some(pool) if pool.slots >= slots => false,
            Some(pool) => {
                pool.slots = slots;
                pool.pending = true;
                true
            }
            None => {
                self.pools.insert(base, Pool { slots, pending: true, used: BTreeMap::new() });
                true
            }
        }
    }

    pub fn mark_used(&mut self, base: u32, slot: u32, used: Used) {
        if let Some(pool) = self.pools.get_mut(&base) {
            pool.used.insert(slot, used);
        }
    }

    pub fn release(&mut self, base: u32) -> Vec<u32> {
        match self.pools.get_mut(&base) {
            Some(pool) => {
                let slots: Vec<u32> = pool.used.iter().filter(|(_, u)| u.active).map(|(slot, _)| *slot).collect();
                pool.used.values_mut().for_each(|u| u.active = false);
                slots
            }
            None => Vec::new(),
        }
    }

    /// Values a free copy may still hold in the running game (released earlier).
    pub fn leftover(&self, base: u32, slot: u32) -> Option<&Used> {
        self.pools.get(&base)?.used.get(&slot).filter(|u| !u.active)
    }

    pub fn active_count(&self, base: u32) -> usize {
        self.pools.get(&base).map(|p| p.used.values().filter(|u| u.active).count()).unwrap_or(0)
    }

    pub fn confirm_restart(&mut self) {
        self.pools.values_mut().for_each(|p| p.pending = false);
    }

    pub fn pending_count(&self) -> usize {
        self.pools.values().filter(|p| p.pending).count()
    }
}

/// Copies the game created at its last start, from the export the mod writes; `None` when the
/// file does not exist (nothing was created, or the panel is not next to the game).
pub fn ready_copies(dir: &Path) -> Option<BTreeSet<(u32, u32)>> {
    let text = std::fs::read_to_string(dir.join(READY_FILE)).ok()?;
    let mut set = BTreeSet::new();
    for line in text.lines().skip(1) {
        let mut parts = line.split(',');
        if let (Some(Ok(base)), Some(Ok(slot))) = (parts.next().map(str::parse::<u32>), parts.next().map(str::parse::<u32>)) {
            set.insert((base, slot));
        }
    }
    Some(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("m182_copies_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("export")).unwrap();
        dir
    }

    fn stats(cut: f32) -> Used {
        let mut dmg = [0f32; 15];
        dmg[2] = cut;
        Used { name: "Espada".into(), dmg: Some(dmg), arm: None, lvl: (0, 0), active: true }
    }

    #[test]
    fn ids_and_guids_follow_the_mods_formula() {
        assert_eq!(item_id(1234, 1), 2_654_437_177);
        assert_eq!(item_id(1234, 2), 1_013_905_642);
        assert_eq!(item_id(4_000_000_000, 1), 2_359_468_647);
        assert_eq!(item_guid("99999999-0000-0000-0000-000000000001", 2).unwrap(), "99999999-0000-5182-0002-000000000001");
        assert_eq!(item_guid("cf270d5e-eff3-4418-a6d3-6e2c06b1d6ae", 11).unwrap(), "cf270d5e-eff3-5182-000b-6e2c06b1d6ae");
        assert!(item_guid("not-a-guid", 1).is_none());
    }

    #[test]
    fn a_pool_must_exist_and_the_game_must_have_created_it() {
        let mut book = Book::default();
        assert_eq!(book.plan(10, &stats(5.0), None), Plan::NoPool);
        assert!(book.register(10, 3));
        assert!(!book.register(10, 3));
        assert_eq!(book.plan(10, &stats(5.0), None), Plan::Pending);
        let none: BTreeSet<(u32, u32)> = BTreeSet::new();
        assert_eq!(book.plan(10, &stats(5.0), Some(&none)), Plan::Pending);
        let ready: BTreeSet<(u32, u32)> = [(10, 1), (10, 2), (10, 3)].into_iter().collect();
        assert_eq!(book.plan(10, &stats(5.0), Some(&ready)), Plan::Use { slot: 1, fresh: true });
        book.confirm_restart();
        assert_eq!(book.plan(10, &stats(5.0), None), Plan::Use { slot: 1, fresh: true });
        assert_eq!(book.pending_count(), 0);
    }

    #[test]
    fn same_stats_reuse_a_copy_and_a_full_pool_is_reported() {
        let mut book = Book::default();
        book.register(10, 2);
        book.confirm_restart();
        book.mark_used(10, 1, stats(5.0));
        assert_eq!(book.plan(10, &stats(5.0), None), Plan::Use { slot: 1, fresh: false });
        assert_eq!(book.plan(10, &stats(9.0), None), Plan::Use { slot: 2, fresh: true });
        book.mark_used(10, 2, stats(9.0));
        assert_eq!(book.plan(10, &stats(12.0), None), Plan::Full);
        assert_eq!(book.active_count(10), 2);
        assert_eq!(book.release(10), vec![1, 2]);
        assert_eq!(book.active_count(10), 0);
        assert_eq!(book.plan(10, &stats(12.0), None), Plan::Use { slot: 1, fresh: true });
        assert_eq!(book.leftover(10, 1).unwrap().dmg, stats(5.0).dmg);
        assert_eq!(book.plan(10, &Used { active: false, ..stats(5.0) }, None), Plan::Use { slot: 1, fresh: true });
    }

    #[test]
    fn a_copy_only_counts_when_the_game_reports_it() {
        let mut book = Book::default();
        book.register(10, 4);
        let ready: BTreeSet<(u32, u32)> = [(10, 3), (10, 4)].into_iter().collect();
        assert_eq!(book.plan(10, &stats(5.0), Some(&ready)), Plan::Use { slot: 3, fresh: true });
    }

    #[test]
    fn the_book_survives_a_restart_of_the_panel() {
        let dir = temp("book");
        let mut book = Book::default();
        book.register(10, 4);
        book.mark_used(10, 2, stats(7.5));
        book.save(&dir).unwrap();
        let loaded = Book::load(&dir);
        assert_eq!(loaded, book);
        std::fs::write(dir.join(FILE), "not json").unwrap();
        assert_eq!(Book::load(&dir), Book::default());
    }

    #[test]
    fn ready_copies_come_from_the_games_export() {
        let dir = temp("ready");
        assert!(ready_copies(&dir).is_none());
        std::fs::write(dir.join(READY_FILE), "base_id,slot,item_id,guid,debug_name\n1234,1,5,g,Sword\n1234,2,6,g,Sword\nbad,line\n").unwrap();
        let set = ready_copies(&dir).unwrap();
        assert_eq!(set.len(), 2);
        assert!(set.contains(&(1234, 2)));
    }

    #[test]
    fn numbers_are_written_plainly() {
        assert_eq!(csv_numbers(&[1.0f32, 1.25, 0.0]), "1,1.25,0");
        assert_eq!(csv_numbers(&[f32::NAN]), "0");
    }
}

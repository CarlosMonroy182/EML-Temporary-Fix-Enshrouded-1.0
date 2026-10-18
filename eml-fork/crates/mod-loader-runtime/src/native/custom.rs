//! Persistence of the unique item copies (see the `zz_m182_item_stats` mod): which items get
//! spare copies ("pool") and which stats each copy carries. The Lua side reads the same file
//! when the game starts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const FILE: &str = "m182_custom.txt";
pub const MOD_DIR: &str = "zz_m182_item_stats";
pub const MAX_SLOTS: u32 = 64;
pub const DMG_VALUES: usize = 15;
pub const ARM_VALUES: usize = 9;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Copy {
    pub dmg: Option<String>,
    pub arm: Option<String>,
    pub lvl: (u32, u32),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Book {
    pub pools: BTreeMap<u32, u32>,
    pub copies: BTreeMap<(u32, u32), Copy>,
}

pub fn path(dir: &Path) -> PathBuf {
    dir.join("mods").join(MOD_DIR).join(FILE)
}

pub fn item_id(base: u32, slot: u32) -> u32 {
    ((base as u64 + slot as u64 * 2_654_435_761 + 182) % 4_294_967_296) as u32
}

pub fn parse(text: &str) -> Book {
    let mut book = Book::default();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["pool", base, slots] => {
                if let (Ok(base), Ok(slots)) = (base.parse::<u32>(), slots.parse::<u32>()) {
                    book.pools.insert(base, slots.min(MAX_SLOTS));
                }
            }
            ["copy", base, slot, dmg, arm, lo, hi] => {
                if let (Ok(base), Ok(slot), Ok(lo), Ok(hi)) = (base.parse::<u32>(), slot.parse::<u32>(), lo.parse::<u32>(), hi.parse::<u32>()) {
                    book.copies.insert(
                        (base, slot),
                        Copy {
                            dmg: (*dmg != "-").then(|| dmg.to_string()),
                            arm: (*arm != "-").then(|| arm.to_string()),
                            lvl: (lo, hi),
                        },
                    );
                }
            }
            _ => {}
        }
    }
    book
}

pub fn render(book: &Book) -> String {
    let mut out = String::new();
    for (base, slots) in &book.pools {
        out.push_str(&format!("pool {base} {slots}\n"));
    }
    for ((base, slot), copy) in &book.copies {
        out.push_str(&format!(
            "copy {base} {slot} {} {} {} {}\n",
            copy.dmg.as_deref().unwrap_or("-"),
            copy.arm.as_deref().unwrap_or("-"),
            copy.lvl.0,
            copy.lvl.1
        ));
    }
    out
}

pub fn load(dir: &Path) -> Book {
    std::fs::read_to_string(path(dir)).map(|t| parse(&t)).unwrap_or_default()
}

fn save(dir: &Path, book: &Book) -> Result<(), String> {
    let target = path(dir);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let temp = target.with_extension("txt.tmp");
    std::fs::write(&temp, render(book)).map_err(|e| format!("cannot write {}: {e}", temp.display()))?;
    std::fs::rename(&temp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("cannot replace {}: {e}", target.display())
    })
}

pub fn numbers_ok(text: &str, count: usize) -> bool {
    let parts: Vec<&str> = text.split(',').collect();
    parts.len() == count && parts.iter().all(|p| p.parse::<f64>().map(|v| v.is_finite() && v.abs() < 1.0e9).unwrap_or(false))
}

pub fn set_pool(dir: &Path, base: u32, slots: u32) -> Result<(), String> {
    if base == 0 || slots == 0 {
        return Err("pool needs an item and at least one slot".to_string());
    }
    let mut book = load(dir);
    let slots = slots.min(MAX_SLOTS);
    let entry = book.pools.entry(base).or_insert(0);
    if *entry >= slots {
        return Ok(());
    }
    *entry = slots;
    save(dir, &book)
}

pub fn set_pools(dir: &Path, bases: &[u32], slots: u32) -> Result<usize, String> {
    if slots == 0 {
        return Err("pool needs at least one slot".to_string());
    }
    let mut book = load(dir);
    let slots = slots.min(MAX_SLOTS);
    let mut changed = 0;
    for base in bases.iter().copied().filter(|b| *b != 0) {
        let entry = book.pools.entry(base).or_insert(0);
        if *entry < slots {
            *entry = slots;
            changed += 1;
        }
    }
    if changed > 0 {
        save(dir, &book)?;
    }
    Ok(changed)
}

pub fn set_copy(dir: &Path, base: u32, slot: u32, dmg: Option<&str>, arm: Option<&str>, lvl: (u32, u32)) -> Result<(), String> {
    if base == 0 || slot == 0 || slot > MAX_SLOTS {
        return Err("copy needs an item and a slot between 1 and 64".to_string());
    }
    if dmg.is_some_and(|d| !numbers_ok(d, DMG_VALUES)) {
        return Err("damage values are invalid".to_string());
    }
    if arm.is_some_and(|a| !numbers_ok(a, ARM_VALUES)) {
        return Err("armour values are invalid".to_string());
    }
    let mut book = load(dir);
    let pool = book.pools.entry(base).or_insert(0);
    if *pool < slot {
        *pool = slot;
    }
    book.copies.insert((base, slot), Copy { dmg: dmg.map(str::to_string), arm: arm.map(str::to_string), lvl });
    save(dir, &book)
}

pub fn drop_copy(dir: &Path, base: u32, slot: u32) -> Result<(), String> {
    let mut book = load(dir);
    if book.copies.remove(&(base, slot)).is_some() {
        save(dir, &book)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("m182_custom_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const DMG: &str = "1,1.25,12.5,0,0,0,0,0,0,0,0,0,0,0,0";
    const ARM: &str = "1,2,3,4,5,6,7,8,9";

    #[test]
    fn ids_match_the_lua_formula() {
        assert_eq!(item_id(1234, 1), 2_654_437_177);
        assert_eq!(item_id(1234, 2), 1_013_905_642);
        assert_eq!(item_id(4_000_000_000, 1), 2_359_468_647);
    }

    #[test]
    fn copies_and_pools_round_trip_and_survive_edits() {
        let dir = temp("round");
        set_pool(&dir, 1234, 8).unwrap();
        set_copy(&dir, 1234, 2, Some(DMG), None, (5, 40)).unwrap();
        set_copy(&dir, 1234, 3, Some(DMG), Some(ARM), (0, 0)).unwrap();
        set_copy(&dir, 77, 1, None, Some(ARM), (0, 0)).unwrap();
        let book = load(&dir);
        assert_eq!(book.pools.get(&1234), Some(&8));
        assert_eq!(book.pools.get(&77), Some(&1));
        assert_eq!(book.copies[&(1234, 2)], Copy { dmg: Some(DMG.into()), arm: None, lvl: (5, 40) });
        set_copy(&dir, 1234, 2, None, None, (0, 0)).unwrap();
        assert_eq!(load(&dir).copies[&(1234, 2)].dmg, None);
        drop_copy(&dir, 1234, 3).unwrap();
        assert!(!load(&dir).copies.contains_key(&(1234, 3)));
        assert_eq!(parse(&render(&load(&dir))), load(&dir));
        let text = std::fs::read_to_string(path(&dir)).unwrap();
        assert!(text.contains("pool 1234 8\n"));
    }

    #[test]
    fn many_pools_are_saved_in_one_go() {
        let dir = temp("many");
        assert_eq!(set_pools(&dir, &[10, 20, 0, 30], 4).unwrap(), 3);
        assert_eq!(set_pools(&dir, &[10, 20], 4).unwrap(), 0);
        assert_eq!(set_pools(&dir, &[10], 6).unwrap(), 1);
        let book = load(&dir);
        assert_eq!(book.pools.len(), 3);
        assert_eq!(book.pools[&10], 6);
        assert!(set_pools(&dir, &[1], 0).is_err());
    }

    #[test]
    fn invalid_input_is_refused_and_a_smaller_pool_never_shrinks_it() {
        let dir = temp("invalid");
        assert!(set_copy(&dir, 1234, 1, Some("1,2,3"), None, (0, 0)).is_err());
        assert!(set_copy(&dir, 1234, 1, Some(&DMG.replace("12.5", "nan")), None, (0, 0)).is_err());
        assert!(set_copy(&dir, 1234, 0, Some(DMG), None, (0, 0)).is_err());
        assert!(set_copy(&dir, 1234, 65, Some(DMG), None, (0, 0)).is_err());
        assert!(set_pool(&dir, 0, 3).is_err());
        set_pool(&dir, 1234, 8).unwrap();
        set_pool(&dir, 1234, 4).unwrap();
        assert_eq!(load(&dir).pools[&1234], 8);
        assert!(!path(&dir).with_extension("txt.tmp").exists());
    }
}

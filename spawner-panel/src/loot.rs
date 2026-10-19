use serde_json::{json, Value};

use crate::items::{is_internal_item, Item};

pub const MAX_DROPS: u32 = 8;

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn rarity_cap(level: u32) -> u32 {
    match level {
        0..=14 => 1,
        15..=34 => 2,
        35..=59 => 3,
        _ => 4,
    }
}

fn eligible(item: &Item) -> bool {
    item.deliverable() && !item.guid.is_empty() && item.stack > 0 && !is_internal_item(&item.debug, &item.category)
}

pub fn roll(rows: &[Item], creature_level: u32, drops: u32, generic_pickup: &str, seed: u64) -> Vec<Value> {
    let drops = drops.min(MAX_DROPS);
    if drops == 0 || generic_pickup.is_empty() {
        return Vec::new();
    }
    let cap = rarity_cap(creature_level);
    let pool: Vec<&Item> = rows.iter().filter(|i| eligible(i) && i.rarity_index() <= cap).collect();
    let stackable: Vec<&Item> = pool.iter().copied().filter(|i| i.stack > 1).collect();
    let gear: Vec<&Item> = pool.iter().copied().filter(|i| i.stack <= 1).collect();
    let mut state = seed | 1;
    for _ in 0..4 {
        next(&mut state);
    }
    let mut out: Vec<Value> = Vec::new();
    for _ in 0..drops {
        let want_gear = next(&mut state) % 10 < 3;
        let source = if (want_gear && !gear.is_empty()) || stackable.is_empty() { &gear } else { &stackable };
        if source.is_empty() {
            break;
        }
        let item = source[(next(&mut state) % source.len() as u64) as usize];
        if out.iter().any(|e| e["item"] == json!(item.id)) {
            continue;
        }
        let level = if item.level_max > 0 { creature_level.clamp(item.level_min.max(1), item.level_max) } else { creature_level.max(1) };
        let amount = if item.stack > 1 {
            let top = item.stack.min(3 + creature_level / 8).max(1);
            1 + (next(&mut state) % top as u64) as u32
        } else {
            1
        };
        out.push(json!({
            "item": item.id,
            "guid": item.guid,
            "level": level,
            "rarity": item.rarity_index().min(cap),
            "amount": amount,
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u32, debug: &str, category: &str, rarity: &str, stack: u32, lmin: u32, lmax: u32) -> Item {
        Item {
            id,
            guid: format!("aaaaaaaa-0000-0000-0000-{id:012}"),
            debug: debug.into(),
            name: debug.into(),
            category: category.into(),
            rarity: rarity.into(),
            stack,
            level_min: lmin,
            level_max: lmax,
            ..Item::default()
        }
    }

    fn rows() -> Vec<Item> {
        vec![
            item(1, "Wood", "Materials", "Common", 100, 1, 1),
            item(2, "Stone", "Materials", "Common", 100, 1, 1),
            item(3, "Sword_A", "Weapons", "Common", 1, 5, 20),
            item(4, "Sword_Epic", "Weapons", "Epic", 1, 30, 55),
            item(5, "Z_Debug_Thing", "Materials", "Common", 10, 1, 1),
            item(0, "NoId", "Materials", "Common", 10, 1, 1),
        ]
    }

    #[test]
    fn nothing_when_the_slider_is_zero_or_there_is_no_pickup() {
        assert!(roll(&rows(), 20, 0, "g", 5).is_empty());
        assert!(roll(&rows(), 20, 3, "", 5).is_empty());
    }

    #[test]
    fn drops_are_valid_distinct_and_respect_level_and_rarity() {
        for seed in 1..200u64 {
            let drops = roll(&rows(), 10, 4, "pickup-guid", seed);
            assert!(!drops.is_empty() && drops.len() <= 4);
            let mut ids: Vec<u64> = drops.iter().map(|d| d["item"].as_u64().unwrap()).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), drops.len(), "no repeats");
            for d in &drops {
                let id = d["item"].as_u64().unwrap();
                assert!(matches!(id, 1 | 2 | 3), "epic, internal and id-less items never drop at level 10: {id}");
                assert!(d.get("template").is_none());
                assert!(d["rarity"].as_u64().unwrap() <= 1);
                assert!(d["amount"].as_u64().unwrap() >= 1);
                if id == 3 {
                    assert_eq!(d["level"], 10);
                    assert_eq!(d["amount"], 1);
                }
                if id <= 2 {
                    assert!(d["amount"].as_u64().unwrap() <= 4);
                }
            }
        }
    }

    #[test]
    fn strong_creatures_can_drop_epic_gear_at_a_level_it_allows() {
        let found = (1..400u64).any(|seed| {
            roll(&rows(), 80, 8, "g", seed).iter().any(|d| d["item"] == 4 && d["level"] == 55 && d["rarity"] == 3)
        });
        assert!(found);
    }

    #[test]
    fn the_same_seed_gives_the_same_roll() {
        assert_eq!(roll(&rows(), 30, 5, "g", 99), roll(&rows(), 30, 5, "g", 99));
    }
}

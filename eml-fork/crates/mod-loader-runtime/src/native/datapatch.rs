use std::collections::HashMap;

use super::config::parse_guid;

pub const VANILLA_ALTARS: [u8; 21] = [1, 2, 4, 6, 8, 10, 11, 12, 13, 14, 15, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16];

#[derive(Clone, Debug)]
pub struct ItemRef {
    pub id: u32,
    pub guid: [u8; 16],
    pub vanilla: u16,
    pub target: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub addr: usize,
    pub item: usize,
    pub adj: bool,
}

pub fn parse_items(csv: &str, factor: f64, csv_scaled: bool) -> Vec<ItemRef> {
    let mut out = Vec::new();
    for line in csv.lines().skip(1) {
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 7 {
            continue;
        }
        let (Ok(id), Some(guid), Ok(stack)) = (parts[0].trim().parse::<u32>(), parse_guid(parts[1]), parts[6].trim().parse::<u32>()) else { continue };
        if stack <= 1 || stack > 65535 {
            continue;
        }
        let stack = stack as u16;
        let (vanilla, target) = if csv_scaled {
            (((stack as f64 / factor).round().clamp(1.0, 65535.0)) as u16, stack)
        } else {
            (stack, scaled_stack(stack, factor))
        };
        if vanilla != target {
            out.push(ItemRef { id, guid, vanilla, target });
        }
    }
    out
}

pub fn csv_values(csv: &str) -> Vec<u32> {
    csv.lines().skip(1).filter_map(|l| l.split(',').nth(6)?.trim().parse::<u32>().ok()).filter(|v| *v > 1).collect()
}

pub fn csv_is_scaled(values: &[u32], factor: f64, eml_log: Option<&str>) -> bool {
    if factor <= 1.0 {
        return false;
    }
    if let Some(log) = eml_log {
        if let Some(at) = log.rfind("stacked=") {
            let digits: String = log[at + 8..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u64>() {
                return n > 0;
            }
        }
    }
    match values.iter().min() {
        Some(min) => (*min as f64) >= 2.0 * factor,
        None => false,
    }
}

fn number_after(text: &str, key: &str) -> Option<f64> {
    let at = text.find(key)?;
    let rest = &text[at + key.len()..];
    let start = rest.find(|c: char| c.is_ascii_digit() || c == '-')?;
    let tail = &rest[start..];
    let end = tail.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e' || c == 'E' || c == '+')).unwrap_or(tail.len());
    tail[..end].parse::<f64>().ok()
}

pub fn factors(json: Option<&str>, lua: Option<&str>) -> (f64, f64) {
    let mut stack = 1.0;
    let mut altar = 1.0;
    if let Some(v) = json.and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok()) {
        if let Some(f) = v["balance"]["stackFactor"].as_f64() {
            stack = f;
        }
        if let Some(f) = v["balance"]["altarFactor"].as_f64() {
            altar = f;
        }
    } else if let Some(t) = lua {
        if let Some(f) = number_after(t, "\"stackFactor\"") {
            stack = f;
        }
        if let Some(f) = number_after(t, "\"altarFactor\"") {
            altar = f;
        }
    }
    let clean = |f: f64| if f.is_finite() && f >= 1.0 { f } else { 1.0 };
    (clean(stack), clean(altar))
}

pub fn scaled_stack(stack: u16, factor: f64) -> u16 {
    ((stack as f64 * factor).round().clamp(1.0, 65535.0)) as u16
}

pub fn scaled_altars(factor: f64) -> Vec<u8> {
    VANILLA_ALTARS.iter().map(|v| ((*v as f64 * factor).round().clamp(1.0, 255.0)) as u8).collect()
}

pub struct GuidIndex {
    map: HashMap<u64, Vec<usize>>,
    bloom: Vec<u64>,
}

const BLOOM_BITS: u32 = 22;

fn bloom_slot(key: u64) -> usize {
    (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - BLOOM_BITS)) as usize
}

impl GuidIndex {
    pub fn new(items: &[ItemRef]) -> Self {
        let mut map: HashMap<u64, Vec<usize>> = HashMap::new();
        let mut bloom = vec![0u64; 1usize << (BLOOM_BITS - 6)];
        for (i, item) in items.iter().enumerate() {
            let key = u64::from_le_bytes(item.guid[..8].try_into().unwrap());
            map.entry(key).or_default().push(i);
            let slot = bloom_slot(key);
            bloom[slot >> 6] |= 1u64 << (slot & 63);
        }
        Self { map, bloom }
    }

    pub fn scan(&self, buf: &[u8], base: usize, items: &[ItemRef], out: &mut Vec<Found>) {
        if buf.len() < 16 {
            return;
        }
        let mut i = 0usize;
        let last = buf.len() - 16;
        while i <= last {
            let key = u64::from_le_bytes(buf[i..i + 8].try_into().unwrap());
            let slot = bloom_slot(key);
            if self.bloom[slot >> 6] >> (slot & 63) & 1 == 1 {
                if let Some(list) = self.map.get(&key) {
                    for &item in list {
                        if buf[i + 8..i + 16] == items[item].guid[8..] {
                            let adj = i >= 4 && buf[i - 4..i] == items[item].id.to_le_bytes();
                            out.push(Found { addr: base + i, item, adj });
                        }
                    }
                }
            }
            i += 4;
        }
    }
}

pub const WINDOW_BEFORE: usize = 64;
pub const WINDOW_LEN: usize = 256;

pub fn vote_offset(found: &[Found], items: &[ItemRef], read: &dyn Fn(usize, usize) -> Option<Vec<u8>>) -> Option<(isize, usize, usize)> {
    let mut votes: HashMap<isize, usize> = HashMap::new();
    let mut sampled = 0usize;
    let step = (found.len() / 3000).max(1);
    for f in found.iter().step_by(step) {
        let Some(w) = read(f.addr.saturating_sub(WINDOW_BEFORE), WINDOW_LEN) else { continue };
        if w.len() < WINDOW_LEN {
            continue;
        }
        sampled += 1;
        let (a, b) = (items[f.item].vanilla.to_le_bytes(), items[f.item].target.to_le_bytes());
        let mut p = 0usize;
        while p + 2 <= w.len() {
            if w[p..p + 2] == a || w[p..p + 2] == b {
                *votes.entry(p as isize - WINDOW_BEFORE as isize).or_default() += 1;
            }
            p += 2;
        }
    }
    let mut ranked: Vec<(isize, usize)> = votes.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let (offset, count) = *ranked.first()?;
    if sampled == 0 || count * 100 < sampled * 60 {
        return None;
    }
    if let Some((_, second)) = ranked.get(1) {
        if *second * 100 >= count * 90 {
            return None;
        }
    }
    Some((offset, count, sampled))
}

pub fn id_before(buf: &[u8], item: &ItemRef) -> bool {
    buf.len() >= 4 && buf[..4] == item.id.to_le_bytes()
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct StackReport {
    pub total: usize,
    pub vanilla: usize,
    pub scaled: usize,
    pub other: usize,
}

pub fn classify(value: u16, item: &ItemRef) -> u8 {
    if value == item.vanilla {
        0
    } else if value == item.target {
        1
    } else {
        2
    }
}

pub fn altar_runs(buf: &[u8], base: usize, vanilla: &[u8], scaled: &[u8], vanilla_out: &mut Vec<usize>, scaled_out: &mut Vec<usize>) {
    let n = vanilla.len();
    if n == 0 || scaled.len() != n || buf.len() < n {
        return;
    }
    let last = buf.len() - n;
    let mut i = 0usize;
    while i <= last {
        let b = buf[i];
        if b == vanilla[0] && &buf[i..i + n] == vanilla {
            vanilla_out.push(base + i);
        } else if scaled != vanilla && b == scaled[0] && &buf[i..i + n] == scaled {
            scaled_out.push(base + i);
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<ItemRef> {
        let csv = "item_id,guid,debug_name,name,category,rarity,max_stack\n1,11111111-2222-3333-4444-555555555555,Wood,Wood,Materials,Common,250\n2,aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee,Sword,Sword,Weapons,Rare,1\n3,01234567-89ab-cdef-0123-456789abcdef,Stone,Stone,Materials,Common,1000\n";
        parse_items(csv, 10.0, false)
    }

    #[test]
    fn items_with_a_stack_are_listed_with_both_values() {
        let list = items();
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].vanilla, list[0].target), (250, 2500));
        assert_eq!(list[1].id, 3);
        let csv = "h\n1,11111111-2222-3333-4444-555555555555,a,a,c,r,2500\n";
        let scaled = parse_items(csv, 10.0, true);
        assert_eq!((scaled[0].vanilla, scaled[0].target), (250, 2500));
    }

    #[test]
    fn the_csv_state_comes_from_the_log_or_the_smallest_stack() {
        assert!(csv_is_scaled(&[250, 500], 10.0, Some("x items done applied=1 failed=0 widened=2 stacked=2266\n")));
        assert!(!csv_is_scaled(&[250, 500], 10.0, Some("items done stacked=0")));
        assert!(csv_is_scaled(&[20, 2500], 10.0, None));
        assert!(!csv_is_scaled(&[5, 250], 10.0, None));
        assert!(!csv_is_scaled(&[250], 1.0, None));
        assert_eq!(csv_values("h\n1,g,a,a,c,r,250\n2,g,a,a,c,r,1\n"), vec![250]);
    }

    #[test]
    fn factors_come_from_json_or_lua_and_never_drop_below_one() {
        assert_eq!(factors(Some(r#"{"balance":{"stackFactor":10.0,"altarFactor":2.0}}"#), None), (10.0, 2.0));
        assert_eq!(factors(None, Some("return{balance={[\"stackFactor\"]=5,[\"altarFactor\"]=3.5,}}")), (5.0, 3.5));
        assert_eq!(factors(Some(r#"{"balance":{"stackFactor":0.2}}"#), None), (1.0, 1.0));
        assert_eq!(factors(None, None), (1.0, 1.0));
    }

    #[test]
    fn scaled_values_are_clamped() {
        assert_eq!(scaled_stack(250, 10.0), 2500);
        assert_eq!(scaled_stack(30000, 10.0), 65535);
        assert_eq!(scaled_altars(2.0)[5], 20);
        assert_eq!(scaled_altars(100.0)[20], 255);
    }

    fn record(item: &ItemRef, value: u16) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        let g = 8usize;
        r[g - 4..g].copy_from_slice(&item.id.to_le_bytes());
        r[g..g + 16].copy_from_slice(&item.guid);
        r[g + 16..g + 18].copy_from_slice(&value.to_le_bytes());
        r
    }

    #[test]
    fn the_scan_finds_records_and_the_vote_finds_the_stack_field() {
        let list = items();
        let index = GuidIndex::new(&list);
        let mut mem = vec![0u8; 128];
        mem.extend(record(&list[0], list[0].vanilla));
        mem.extend(vec![7u8; 40]);
        mem.extend(record(&list[1], list[1].target));
        mem.extend(vec![0u8; 100]);
        let mut found = Vec::new();
        index.scan(&mem, 0x1000, &list, &mut found);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].addr, 0x1000 + 128 + 8);
        assert!(found[0].adj && found[1].adj);
        assert!(id_before(&mem[found[0].addr - 0x1000 - 4..], &list[0]));
        assert!(!id_before(&mem[found[0].addr - 0x1000 - 4..], &list[1]));
        let read = |addr: usize, len: usize| -> Option<Vec<u8>> {
            let start = addr.checked_sub(0x1000)?;
            let mut v = vec![0u8; len];
            for (i, b) in v.iter_mut().enumerate() {
                *b = *mem.get(start + i).unwrap_or(&0);
            }
            Some(v)
        };
        let (offset, count, sampled) = vote_offset(&found, &list, &read).unwrap();
        assert_eq!(offset, 16);
        assert_eq!((count, sampled), (2, 2));
    }

    #[test]
    fn classification_tells_vanilla_from_scaled() {
        let item = &items()[0];
        assert_eq!(classify(250, item), 0);
        assert_eq!(classify(2500, item), 1);
        assert_eq!(classify(77, item), 2);
    }

    #[test]
    fn altar_arrays_are_found_vanilla_or_scaled() {
        let scaled = scaled_altars(2.0);
        let mut mem = vec![9u8; 10];
        mem.extend_from_slice(&VANILLA_ALTARS);
        mem.extend(vec![9u8; 5]);
        mem.extend_from_slice(&scaled);
        let (mut v, mut s) = (Vec::new(), Vec::new());
        altar_runs(&mem, 0x2000, &VANILLA_ALTARS, &scaled, &mut v, &mut s);
        assert_eq!(v, vec![0x2000 + 10]);
        assert_eq!(s, vec![0x2000 + 10 + 21 + 5]);
    }
}

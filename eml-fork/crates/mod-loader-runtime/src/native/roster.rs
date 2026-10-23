//! Player roster helpers shared by the server hooks: reading a player's name out of a
//! `ServerPlayers` slot, the chat reply that lists the connected players and the permission
//! rule for visitors of a protected base.

pub const SLOT_NAME: usize = 0x20;
pub const SLOT_PERMISSIONS: usize = 0x10;
pub const SLOT_STATE: usize = 0x0a;
pub const SLOT_SIZE: usize = 0x148;
pub const MAX_NAME: usize = 40;

pub const PERM_KICK_BAN: u8 = 1 << 0;
pub const PERM_INVENTORIES: u8 = 1 << 1;
pub const PERM_EDIT_BASE: u8 = 1 << 2;
pub const PERM_EDIT_WORLD: u8 = 1 << 3;
pub const PERM_EXTEND_BASE: u8 = 1 << 4;
pub const VISITOR_DENIED: u8 = PERM_INVENTORIES | PERM_EDIT_BASE | PERM_EDIT_WORLD | PERM_EXTEND_BASE;

pub fn is_admin(permissions: u8) -> bool {
    permissions & PERM_KICK_BAN != 0
}

pub fn visitor_permissions(original: u8) -> u8 {
    original & !VISITOR_DENIED
}

fn printable(c: char) -> bool {
    !c.is_control() && c != '\u{fffd}'
}

fn clean(text: &str) -> Option<String> {
    let t = text.trim();
    let count = t.chars().count();
    if count == 0 || count > MAX_NAME || !t.chars().all(printable) {
        return None;
    }
    if !t.chars().any(|c| c.is_alphanumeric()) {
        return None;
    }
    Some(t.to_string())
}

pub fn utf8_text(bytes: &[u8]) -> Option<String> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    if end == 0 || end > MAX_NAME * 4 {
        return None;
    }
    clean(std::str::from_utf8(&bytes[..end]).ok()?)
}

pub fn utf16_text(bytes: &[u8]) -> Option<String> {
    let mut units = Vec::new();
    for pair in bytes.chunks_exact(2) {
        let u = u16::from_le_bytes([pair[0], pair[1]]);
        if u == 0 {
            break;
        }
        units.push(u);
        if units.len() > MAX_NAME {
            return None;
        }
    }
    if units.is_empty() {
        return None;
    }
    let ascii = units.iter().filter(|u| **u < 0x80 && **u >= 0x20).count();
    if ascii * 2 < units.len() {
        return None;
    }
    clean(&String::from_utf16(&units).ok()?)
}

fn pointer_like(v: u64) -> bool {
    (0x1_0000..0x7ff0_0000_0000).contains(&v)
}

fn qword(slot: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(slot.get(off..off + 8)?.try_into().ok()?))
}

/// Text stored as pointer, length and capacity (not terminated): the length decides where it ends.
fn counted_text(slot: &[u8], deref: &dyn Fn(usize) -> Option<Vec<u8>>) -> Option<(String, String)> {
    let mut off = 0;
    while off + 24 <= slot.len() {
        if let (Some(p), Some(len), Some(cap)) = (qword(slot, off), qword(slot, off + 8), qword(slot, off + 16)) {
            if pointer_like(p) && (2..=(MAX_NAME as u64 * 4)).contains(&len) && cap >= len && cap <= 0x1000 {
                if let Some(bytes) = deref(p as usize) {
                    let len = len as usize;
                    if bytes.len() >= len {
                        if let Some(n) = std::str::from_utf8(&bytes[..len]).ok().and_then(clean) {
                            return Some((n, format!("utf-8 of known length behind the pointer at +{off:#x}")));
                        }
                    }
                }
            }
        }
        off += 8;
    }
    None
}

const STEAM_FIRST: u64 = 0x0110_0001_0000_0000;
const STEAM_LAST: u64 = 0x0110_0001_ffff_ffff;

fn steam_text(bytes: &[u8]) -> Option<String> {
    let key = b"steamid:";
    let at = find_all(bytes, key).into_iter().next()?;
    let digits: String = bytes[at + key.len()..].iter().take_while(|b| b.is_ascii_digit()).map(|b| *b as char).collect();
    if digits.len() >= 15 && digits.len() <= 20 {
        Some(format!("steamid:{digits}"))
    } else {
        None
    }
}

/// The platform id of a player: a Steam id kept as number, or written as `steamid:<digits>`.
pub fn guess_id(slot: &[u8], deref: &dyn Fn(usize) -> Option<Vec<u8>>) -> Option<String> {
    let mut off = 0;
    while off + 8 <= slot.len() {
        if let Some(v) = qword(slot, off) {
            if (STEAM_FIRST..=STEAM_LAST).contains(&v) {
                return Some(format!("steamid:{v}"));
            }
        }
        off += 8;
    }
    if let Some(t) = steam_text(slot) {
        return Some(t);
    }
    let mut off = 0;
    while off + 8 <= slot.len() {
        if let Some(p) = qword(slot, off) {
            if pointer_like(p) && !(0x7ff0_0000_0000..).contains(&p) {
                if let Some(bytes) = deref(p as usize) {
                    if let Some(v) = qword(&bytes, 0) {
                        if (STEAM_FIRST..=STEAM_LAST).contains(&v) {
                            return Some(format!("steamid:{v}"));
                        }
                    }
                    if let Some(t) = steam_text(&bytes) {
                        return Some(t);
                    }
                }
            }
        }
        off += 8;
    }
    None
}

/// Finds the player's name in a slot: inline text first, then text behind a pointer.
/// Returns the name and a description of where it was found (for the log).
pub fn guess_name(slot: &[u8], deref: &dyn Fn(usize) -> Option<Vec<u8>>) -> Option<(String, String)> {
    if let Some(found) = counted_text(slot, deref) {
        return Some(found);
    }
    let at = |off: usize| slot.get(off..).unwrap_or(&[]);
    if let Some(n) = utf8_text(at(SLOT_NAME)) {
        if n.chars().count() >= 2 {
            return Some((n, "inline utf-8 at +0x20".to_string()));
        }
    }
    if let Some(n) = utf16_text(at(SLOT_NAME)) {
        if n.chars().count() >= 2 {
            return Some((n, "inline utf-16 at +0x20".to_string()));
        }
    }
    for off in [SLOT_NAME, SLOT_NAME + 8, SLOT_NAME + 16, 0x18, 0x40, 0x48] {
        let Some(raw) = slot.get(off..off + 8) else { continue };
        let p = u64::from_le_bytes(raw.try_into().unwrap_or([0; 8]));
        if !pointer_like(p) {
            continue;
        }
        let Some(bytes) = deref(p as usize) else { continue };
        if let Some(n) = utf8_text(&bytes) {
            if n.chars().count() >= 2 {
                return Some((n, format!("utf-8 behind the pointer at +{off:#x}")));
            }
        }
        if let Some(n) = utf16_text(&bytes) {
            if n.chars().count() >= 2 {
                return Some((n, format!("utf-16 behind the pointer at +{off:#x}")));
            }
        }
    }
    let mut off = 0x18;
    while off + 6 <= slot.len() {
        if let Some(n) = utf8_text(&slot[off..]) {
            if n.chars().count() >= 3 && n.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
                return Some((n, format!("inline utf-8 at +{off:#x}")));
            }
        }
        off += 4;
    }
    let mut off = 0x18;
    while off + 8 <= slot.len() {
        if let Some(n) = utf16_text(&slot[off..]) {
            if n.chars().count() >= 3 && n.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '_' || c == '-' || c == '.') {
                return Some((n, format!("inline utf-16 at +{off:#x}")));
            }
        }
        off += 2;
    }
    let mut off = 0;
    while off + 8 <= slot.len() {
        let p = u64::from_le_bytes(slot[off..off + 8].try_into().unwrap_or([0; 8]));
        if pointer_like(p) {
            if let Some(bytes) = deref(p as usize) {
                let found = utf8_text(&bytes).filter(|n| n.chars().count() >= 3 && n.chars().all(|c| c.is_ascii_graphic() || c == ' ')).map(|n| (n, "utf-8")).or_else(|| {
                    utf16_text(&bytes).filter(|n| n.chars().count() >= 3 && n.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '_' || c == '-' || c == '.')).map(|n| (n, "utf-16"))
                });
                if let Some((n, kind)) = found {
                    return Some((n, format!("{kind} behind the pointer at +{off:#x}")));
                }
            }
        }
        off += 8;
    }
    None
}

pub fn tag(seq: u32) -> String {
    format!("[M182#{seq}]")
}

/// `ID 3~steamid:765…` / `Ana~steamid:765…`: a player's label with the platform id when it is known.
pub fn label_with_id(name: &str, id: Option<&str>) -> String {
    let name: String = name.chars().filter(|c| *c != '~').collect();
    match id {
        Some(id) => format!("{name}~{}", id.chars().filter(|c| *c != '~').collect::<String>()),
        None => name,
    }
}

/// `[M182#7] 1=Ana|2=Luis ^2@2995.1,895.0,1607.7`: one chat line that tells the admin's panel who
/// is connected and, after the `^`, which entity asked and where it is standing.
pub fn reply_text(seq: u32, entries: &[(u32, String)]) -> String {
    reply_text_with_own(seq, entries, None)
}

pub fn reply_text_with_own(seq: u32, entries: &[(u32, String)], own: Option<(u32, [f64; 3])>) -> String {
    let mut out = tag(seq);
    out.push(' ');
    let mut first = true;
    for (number, name) in entries {
        let safe: String = name.chars().filter(|c| *c != '|' && *c != '=' && *c != '^' && !c.is_control()).collect();
        let part = format!("{}{number}={safe}", if first { "" } else { "|" });
        if out.len() + part.len() > 800 {
            break;
        }
        out.push_str(&part);
        first = false;
    }
    if first {
        out.push('-');
    }
    if let Some((entity, p)) = own {
        if p.iter().all(|v| v.is_finite()) {
            out.push_str(&format!(" ^{entity}@{:.1},{:.1},{:.1}", p[0], p[1], p[2]));
        }
    }
    out
}

fn reply_line(text: &str, seq: u32) -> Option<&str> {
    let marker = tag(seq);
    let start = text.find(&marker)? + marker.len();
    Some(text[start..].trim_start().lines().next().unwrap_or("").trim())
}

pub fn parse_reply(text: &str, seq: u32) -> Option<Vec<(u32, String)>> {
    let line = reply_line(text, seq)?;
    let line = line.split(" ^").next().unwrap_or("").trim();
    let mut out = Vec::new();
    if line == "-" {
        return Some(out);
    }
    for part in line.split('|') {
        let (n, name) = part.split_once('=')?;
        out.push((n.trim().parse().ok()?, name.trim().to_string()));
    }
    Some(out)
}

/// The asking entity and its position, when the server added them to the list.
pub fn parse_own(text: &str, seq: u32) -> Option<(u32, [f64; 3])> {
    let line = reply_line(text, seq)?;
    let (_, tail) = line.split_once(" ^")?;
    let (entity, pos) = tail.trim().split_once('@')?;
    let values: Vec<f64> = pos.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    if values.len() != 3 || values.iter().any(|v| !v.is_finite()) {
        return None;
    }
    Some((entity.trim().parse().ok()?, [values[0], values[1], values[2]]))
}

/// Start of every place where `needle` occurs in `hay`.
pub fn find_all(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    if needle.is_empty() || hay.len() < needle.len() {
        return out;
    }
    let first = needle[0];
    let last = hay.len() - needle.len();
    let mut i = 0;
    while i <= last {
        match hay[i..=last].iter().position(|b| *b == first) {
            Some(p) => {
                i += p;
                if &hay[i..i + needle.len()] == needle {
                    out.push(i);
                }
                i += 1;
            }
            None => break,
        }
    }
    out
}

/// File the panel writes once the server's player list shows which entity is this game's own character.
pub const SELF_FILE: &str = "m182_self.json";

/// `{"entity":5}`: the entity number of the character this game controls.
pub fn parse_self(text: &str) -> Option<u32> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let n = value.get("entity")?.as_u64()?;
    if n == 0 || n >= 0x8000_0000 {
        return None;
    }
    Some(n as u32)
}

/// A name as people type it in chat: the `<name>` of the help text, quotes, an `@` or a final dot are dropped.
pub fn clean_typed_name(typed: &str) -> String {
    let t = typed.trim().trim_matches(|c: char| matches!(c, '<' | '>' | '[' | ']' | '(' | ')' | '"' | '\'' | '@' | '`' | '*' | '.' | ',' | ';' | ':' | '!')).trim();
    t.to_string()
}

pub fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// `ID 3` (or the older `Jugador 3`) stands for entity number 3 when the real names are not readable.
pub fn placeholder_number(name: &str) -> Option<u32> {
    let trimmed = name.trim();
    let rest = trimmed.strip_prefix("ID ").or_else(|| trimmed.strip_prefix("Jugador "))?;
    rest.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nothing(_: usize) -> Option<Vec<u8>> {
        None
    }

    #[test]
    fn visitor_loses_chests_and_building_but_keeps_the_rest() {
        let all = 0b11_1111;
        let v = visitor_permissions(all);
        assert_eq!(v & PERM_INVENTORIES, 0);
        assert_eq!(v & PERM_EDIT_BASE, 0);
        assert_eq!(v & PERM_EDIT_WORLD, 0);
        assert_eq!(v & PERM_EXTEND_BASE, 0);
        assert_eq!(v & PERM_KICK_BAN, PERM_KICK_BAN);
        assert_eq!(v & (1 << 5), 1 << 5);
        assert!(is_admin(all));
        assert!(!is_admin(visitor_permissions(PERM_INVENTORIES)));
    }

    #[test]
    fn names_are_found_inline_or_behind_a_pointer() {
        let mut slot = vec![0u8; SLOT_SIZE];
        slot[SLOT_NAME..SLOT_NAME + 8].copy_from_slice(b"moradoox");
        assert_eq!(guess_name(&slot, &nothing).unwrap().0, "moradoox");

        let mut wide = vec![0u8; SLOT_SIZE];
        for (i, u) in "Ñandu".encode_utf16().enumerate() {
            wide[SLOT_NAME + i * 2..SLOT_NAME + i * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }
        assert_eq!(guess_name(&wide, &nothing).unwrap().0, "Ñandu");

        let mut indirect = vec![0u8; SLOT_SIZE];
        indirect[SLOT_NAME..SLOT_NAME + 8].copy_from_slice(&0x1234_5678_0000u64.to_le_bytes());
        let deref = |p: usize| if p == 0x1234_5678_0000 { Some(b"Nacht\0junk".to_vec()) } else { None };
        let (name, how) = guess_name(&indirect, &deref).unwrap();
        assert_eq!(name, "Nacht");
        assert!(how.contains("pointer"));

        let mut late = vec![0u8; SLOT_SIZE];
        late[0x100..0x100 + 6].copy_from_slice(b"Nacht9");
        assert_eq!(guess_name(&late, &nothing).unwrap().1, "inline utf-8 at +0x100");
        let mut wide_late = vec![0u8; SLOT_SIZE];
        for (i, u) in "Luis_M".encode_utf16().enumerate() {
            wide_late[0xe0 + i * 2..0xe0 + i * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }
        assert_eq!(guess_name(&wide_late, &nothing).unwrap().0, "Luis_M");
        let mut far = vec![0u8; SLOT_SIZE];
        far[0xf0..0xf8].copy_from_slice(&0x3000_1234_0000u64.to_le_bytes());
        let deref_far = |p: usize| if p == 0x3000_1234_0000 { Some(b"Valkyria\0".to_vec()) } else { None };
        assert_eq!(guess_name(&far, &deref_far).unwrap().0, "Valkyria");
        let mut module = vec![0u8; SLOT_SIZE];
        module[0x38..0x40].copy_from_slice(&0x7ff7_8245_2a90u64.to_le_bytes());
        let deref_any = |_p: usize| Some(b"RuntimeTypeName\0".to_vec());
        assert!(guess_name(&module, &deref_any).is_none(), "pointers into the game's own image are not names");
        assert!(guess_name(&vec![0u8; SLOT_SIZE], &nothing).is_none());
        let mut noise = vec![0u8; SLOT_SIZE];
        noise[SLOT_NAME..SLOT_NAME + 4].copy_from_slice(&[1, 2, 3, 4]);
        assert!(guess_name(&noise, &nothing).is_none());
    }

    #[test]
    fn a_name_kept_as_pointer_length_and_capacity_is_read_without_a_terminator() {
        let hex = "400000008100000000000300b4a3ffdc3f0000004e0304e40100000000000000102b0317010000000800000000000000080000000000000050f404400100000031ef5fb300000000768a3c87045580be0761f5146cfa194dbc599646ae0cd6a400000000000000000000000000000000ccd71a70b4a3ffdc010000000000000030ef5fb300000000c006e7c100000000400000000000000030ef5fb30000000030ef5fb300000000e006e7c1000000000001000000000000090000000000000030ef5fb3000000007005e7c100000000400000000000000030ef5fb30000000030ef5fb3000000009005e7c1000000000001000000000000000000000000000000020080230e00000000008062030000000200803805000000000080f304353f00000080f30435bf0000803f0000803f0000803f0000000000000000000ec0000000010000000000";
        let slot: Vec<u8> = (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap()).collect();
        assert_eq!(slot.len(), SLOT_SIZE);
        let deref = |p: usize| if p == 0x1_1703_2b10 { Some(b"moradooxAAAA\x01\x02junk".to_vec()) } else { Some(vec![0xff; 96]) };
        let (name, how) = guess_name(&slot, &deref).unwrap();
        assert_eq!(name, "moradoox");
        assert!(how.contains("known length"), "{how}");
        assert_eq!(placeholder_number("ID 4"), Some(4));
        assert_eq!(placeholder_number("Jugador 4"), Some(4));
        assert_eq!(label_with_id("Ana~", Some("steamid:1")), "Ana~steamid:1");
        assert_eq!(label_with_id("ID 2", None), "ID 2");
    }

    #[test]
    fn steam_ids_are_found_as_numbers_or_text_inline_or_behind_a_pointer() {
        let id = 76_561_199_474_170_255u64;
        let mut inline = vec![0u8; SLOT_SIZE];
        inline[0x58..0x60].copy_from_slice(&id.to_le_bytes());
        assert_eq!(guess_id(&inline, &nothing).as_deref(), Some("steamid:76561199474170255"));
        let mut behind = vec![0u8; SLOT_SIZE];
        behind[0x20..0x28].copy_from_slice(&0x1_1703_2b10u64.to_le_bytes());
        let number = |p: usize| if p == 0x1_1703_2b10 { Some(id.to_le_bytes().to_vec()) } else { None };
        assert_eq!(guess_id(&behind, &number).as_deref(), Some("steamid:76561199474170255"));
        let text = |p: usize| if p == 0x1_1703_2b10 { Some(b"xx steamid:76561199474170255\0".to_vec()) } else { None };
        assert_eq!(guess_id(&behind, &text).as_deref(), Some("steamid:76561199474170255"));
        assert!(guess_id(&vec![0u8; SLOT_SIZE], &nothing).is_none());
        let mut small = vec![0u8; SLOT_SIZE];
        small[0..8].copy_from_slice(&64u64.to_le_bytes());
        assert!(guess_id(&small, &nothing).is_none());
    }

    #[test]
    fn reply_round_trips_and_ignores_other_replies() {
        let text = reply_text(7, &[(1, "Ana".into()), (3, "Luis | el=bueno".into())]);
        assert!(text.starts_with("[M182#7] "));
        let parsed = parse_reply(&format!("Carlos: {text}"), 7).unwrap();
        assert_eq!(parsed, vec![(1, "Ana".to_string()), (3, "Luis  elbueno".to_string())]);
        assert!(parse_reply(&text, 8).is_none());
        assert_eq!(parse_reply(&reply_text(2, &[]), 2).unwrap(), vec![]);
    }

    #[test]
    fn finds_every_occurrence_in_a_buffer() {
        assert_eq!(find_all(b"xx[M182#1]yy[M182#1]", b"[M182#1]"), vec![2, 12]);
        assert!(find_all(b"abc", b"abcd").is_empty());
        assert!(find_all(b"abc", b"").is_empty());
        let wide: Vec<u8> = "[M182#9] 1=Ana".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let marker: Vec<u8> = "[M182#9]".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(find_all(&wide, &marker), vec![0]);
    }

    #[test]
    fn the_list_carries_the_asking_player_and_its_position() {
        let text = reply_text_with_own(9, &[(1, "Ana".into()), (5, "Carlos".into())], Some((5, [2995.14, 895.0, 1607.66])));
        assert_eq!(text, "[M182#9] 1=Ana|5=Carlos ^5@2995.1,895.0,1607.7");
        assert_eq!(parse_reply(&text, 9).unwrap(), vec![(1, "Ana".to_string()), (5, "Carlos".to_string())]);
        assert_eq!(parse_own(&format!("Server: {text}"), 9), Some((5, [2995.1, 895.0, 1607.7])));
        assert_eq!(parse_own(&reply_text(9, &[(1, "Ana".into())]), 9), None);
        assert_eq!(parse_own(&text, 8), None);
        let tricky = reply_text_with_own(3, &[(2, "a ^ b".into())], Some((2, [1.0, 2.0, 3.0])));
        assert_eq!(parse_reply(&tricky, 3).unwrap(), vec![(2, "a  b".to_string())]);
        let broken = reply_text_with_own(3, &[(2, "x".into())], Some((2, [f64::NAN, 0.0, 0.0])));
        assert_eq!(parse_own(&broken, 3), None);
    }

    #[test]
    fn the_own_character_file_is_read() {
        assert_eq!(parse_self(r#"{"entity":5}"#), Some(5));
        assert_eq!(parse_self(r#"{"entity":0}"#), None);
        assert_eq!(parse_self(r#"{"entity":-1}"#), None);
        assert_eq!(parse_self("nonsense"), None);
    }

    #[test]
    fn placeholders_and_names_compare_loosely() {
        assert_eq!(placeholder_number("Jugador 12"), Some(12));
        assert_eq!(placeholder_number("Ana"), None);
        assert!(same_name(" ANA ", "ana"));
    }
}

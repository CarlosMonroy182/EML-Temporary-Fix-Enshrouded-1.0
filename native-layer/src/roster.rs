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
    (0x1_0000..0x7fff_ffff_ffff).contains(&v)
}

/// Finds the player's name in a slot: inline text first, then text behind a pointer.
/// Returns the name and a description of where it was found (for the log).
pub fn guess_name(slot: &[u8], deref: &dyn Fn(usize) -> Option<Vec<u8>>) -> Option<(String, String)> {
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
    while off + 6 <= slot.len().min(0x120) {
        if let Some(n) = utf8_text(&slot[off..]) {
            if n.chars().count() >= 3 && n.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
                return Some((n, format!("inline utf-8 at +{off:#x}")));
            }
        }
        off += 4;
    }
    None
}

pub fn tag(seq: u32) -> String {
    format!("[M182#{seq}]")
}

/// `[M182#7] 1=Ana|2=Luis`: one chat line that tells the admin's panel who is connected.
pub fn reply_text(seq: u32, entries: &[(u32, String)]) -> String {
    let mut out = tag(seq);
    out.push(' ');
    let mut first = true;
    for (number, name) in entries {
        let safe: String = name.chars().filter(|c| *c != '|' && *c != '=' && !c.is_control()).collect();
        let part = format!("{}{number}={safe}", if first { "" } else { "|" });
        if out.len() + part.len() > 900 {
            break;
        }
        out.push_str(&part);
        first = false;
    }
    if first {
        out.push('-');
    }
    out
}

pub fn parse_reply(text: &str, seq: u32) -> Option<Vec<(u32, String)>> {
    let marker = tag(seq);
    let start = text.find(&marker)? + marker.len();
    let body = text[start..].trim_start();
    let line = body.lines().next().unwrap_or("").trim();
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

pub fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// `Jugador 3` stands for entity number 3 when the real names are not readable.
pub fn placeholder_number(name: &str) -> Option<u32> {
    let rest = name.trim().strip_prefix("Jugador ")?;
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

        assert!(guess_name(&vec![0u8; SLOT_SIZE], &nothing).is_none());
        let mut noise = vec![0u8; SLOT_SIZE];
        noise[SLOT_NAME..SLOT_NAME + 4].copy_from_slice(&[1, 2, 3, 4]);
        assert!(guess_name(&noise, &nothing).is_none());
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
    fn placeholders_and_names_compare_loosely() {
        assert_eq!(placeholder_number("Jugador 12"), Some(12));
        assert_eq!(placeholder_number("Ana"), None);
        assert!(same_name(" ANA ", "ana"));
    }
}

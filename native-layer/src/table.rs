//! The server's own player table: who is connected, under which name, and what each player
//! is allowed to do. Reached through the object the game's per-tick chat pump receives.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::patch::PLAYERS_OBJECT;
use super::roster::{self, SLOT_SIZE};
use super::spawn_probe::{hex, read_bytes, u32_at};
use super::spawner::{log, write_bytes};

const SLOTS_AT: usize = 0x1e0;
const SLOT_COUNT: usize = 17;
const MIN_STATE: u8 = 2;

#[derive(Clone, Debug)]
pub struct Entry {
    pub index: usize,
    pub client: u32,
    pub state: u8,
    pub permissions: u8,
    pub name: Option<String>,
}

impl Entry {
    pub fn entity(&self) -> u32 {
        self.index as u32 + 1
    }
}

fn object() -> Option<usize> {
    let o = PLAYERS_OBJECT.load(Ordering::SeqCst);
    if o < 0x10000 { None } else { Some(o) }
}

static REPORTED: Mutex<Vec<(usize, u32)>> = Mutex::new(Vec::new());

pub fn entries() -> Vec<Entry> {
    let Some(base) = object() else { return Vec::new() };
    let Some(raw) = read_bytes(base + SLOTS_AT, SLOT_COUNT * SLOT_SIZE) else { return Vec::new() };
    if raw.len() < SLOT_COUNT * SLOT_SIZE {
        return Vec::new();
    }
    let mut out = Vec::new();
    for index in 0..SLOT_COUNT {
        let slot = &raw[index * SLOT_SIZE..(index + 1) * SLOT_SIZE];
        let client = u32_at(slot, 0);
        let state = slot[roster::SLOT_STATE];
        if client == 0 || (client & 0x3f) as usize != index || state < MIN_STATE {
            continue;
        }
        let guess = roster::guess_name(slot, &|p| read_bytes(p, 96));
        if let Ok(mut seen) = REPORTED.lock() {
            if !seen.contains(&(index, client)) {
                seen.push((index, client));
                log(&format!(
                    "player table: slot {index} client {client} state {state} permissions {:#04x} name {} bytes {}",
                    slot[roster::SLOT_PERMISSIONS],
                    match &guess {
                        Some((n, how)) => format!("\"{n}\" ({how})"),
                        None => "unreadable".to_string(),
                    },
                    hex(&slot[..0x80])
                ));
            }
        }
        out.push(Entry { index, client, state, permissions: slot[roster::SLOT_PERMISSIONS], name: guess.map(|g| g.0) });
    }
    out
}

/// Entity number -> name, refreshed at most every two seconds.
pub fn names() -> HashMap<u32, String> {
    static CACHE: Mutex<Option<(Instant, HashMap<u32, String>)>> = Mutex::new(None);
    let Ok(mut cache) = CACHE.lock() else { return HashMap::new() };
    if let Some((at, map)) = cache.as_ref() {
        if at.elapsed() < Duration::from_secs(2) {
            return map.clone();
        }
    }
    let map: HashMap<u32, String> = entries().into_iter().filter_map(|e| e.name.clone().map(|n| (e.entity(), n))).collect();
    *cache = Some((Instant::now(), map.clone()));
    map
}

struct Held {
    client: u32,
    original: u8,
}

static HELD: Mutex<Vec<(usize, Held)>> = Mutex::new(Vec::new());

fn write_permissions(index: usize, value: u8) -> bool {
    let Some(base) = object() else { return false };
    write_bytes(base + SLOTS_AT + index * SLOT_SIZE + roster::SLOT_PERMISSIONS, &[value])
}

/// Visitors of a protected base cannot open chests or build there. `visitors` holds the table
/// slots that must be restricted right now; everybody else gets the permissions they came with.
pub fn apply_visitors(visitors: &[usize]) {
    let current = entries();
    let Ok(mut held) = HELD.lock() else { return };
    held.retain(|(index, h)| current.iter().any(|e| e.index == *index && e.client == h.client));
    for entry in &current {
        let locked = held.iter().position(|(i, _)| *i == entry.index);
        let wanted = visitors.contains(&entry.index);
        match (locked, wanted) {
            (None, true) => {
                let restricted = roster::visitor_permissions(entry.permissions);
                if restricted != entry.permissions && write_permissions(entry.index, restricted) {
                    log(&format!("base: {} is a visitor now (permissions {:#04x} -> {:#04x})", label(entry), entry.permissions, restricted));
                    held.push((entry.index, Held { client: entry.client, original: entry.permissions }));
                } else if restricted == entry.permissions {
                    held.push((entry.index, Held { client: entry.client, original: entry.permissions }));
                }
            }
            (Some(at), true) => {
                let restricted = roster::visitor_permissions(held[at].1.original);
                if entry.permissions != restricted {
                    write_permissions(entry.index, restricted);
                }
            }
            (Some(at), false) => {
                let original = held[at].1.original;
                if write_permissions(entry.index, original) {
                    log(&format!("base: {} is no longer a visitor (permissions back to {:#04x})", label(entry), original));
                    held.remove(at);
                }
            }
            (None, false) => {}
        }
    }
}

fn label(e: &Entry) -> String {
    match &e.name {
        Some(n) => format!("{n} (slot {})", e.index),
        None => format!("player in slot {}", e.index),
    }
}

pub fn restore_all() {
    let Ok(mut held) = HELD.lock() else { return };
    let current = entries();
    for (index, h) in held.drain(..) {
        if current.iter().any(|e| e.index == index && e.client == h.client) {
            write_permissions(index, h.original);
        }
    }
}

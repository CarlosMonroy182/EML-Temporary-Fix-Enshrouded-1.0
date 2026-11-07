//! M182 Guard (observe): follows the character copies the server keeps and compares every uploaded character with them.
//!
//! Nothing here changes game behaviour and nothing runs on a timer. Two hooks fire only when the game itself handles a
//! character: the upload a client sends when it joins, and the server's own update of the copy it keeps while the player
//! plays. Configuration lives in its own file, `m182_guard.json`, so it never touches the panel or VFX.

use std::collections::HashMap;
use std::ffi::c_void;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Deserialize;

use super::{patch, roster, signatures};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GuardConfig {
    /// "off" (default) or "observe".
    pub mode: String,
    pub max_dumps: u32,
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self { mode: "off".into(), max_dumps: 20 }
    }
}

static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static SET_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static DUMPS: AtomicUsize = AtomicUsize::new(0);
static SET_CALLS: AtomicUsize = AtomicUsize::new(0);
static FINAL_CALLS: AtomicUsize = AtomicUsize::new(0);
static FINAL_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
/// How many calls of the server-copy hook are described in detail in the log.
const DIAG_CALLS: usize = 12;
static MAX_DUMPS: AtomicUsize = AtomicUsize::new(20);

const MAX_BLOB: usize = 8 * 1024 * 1024;
const WRITE_EVERY_SECS: u64 = 20;
const FINAL_SAVE_STATE: u8 = 4;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn ReadProcessMemory(process: *mut c_void, base: *const c_void, buf: *mut c_void, size: usize, read: *mut usize) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn VirtualAlloc(addr: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
}

fn config(dir: &std::path::Path) -> GuardConfig {
    match std::fs::read_to_string(dir.join("m182_guard.json")) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => GuardConfig::default(),
    }
}

fn read_mem(addr: usize, len: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; len];
    let mut got = 0usize;
    let ok = unsafe { ReadProcessMemory(GetCurrentProcess(), addr as *const c_void, buf.as_mut_ptr().cast(), len, &mut got) };
    if ok != 0 && got == len { Some(buf) } else { None }
}

fn u32_le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn usize_le(b: &[u8], o: usize) -> usize {
    usize::from_le_bytes(b[o..o + 8].try_into().unwrap_or([0; 8]))
}

fn fnv1a(data: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in data {
        h ^= *b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn guard_dir() -> Option<std::path::PathBuf> {
    let dir = std::env::current_dir().ok()?.join("m182_guard");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

fn log(msg: &str) {
    let Some(dir) = guard_dir() else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("guard.log")) {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "[{secs}] {msg}");
    }
}

/// An uploaded character is `0x01` + a zstd frame.
fn unpack(blob: &[u8]) -> Option<Vec<u8>> {
    if blob.len() < 8 || blob[0] != 1 || blob[1..5] != [0x28, 0xb5, 0x2f, 0xfd] {
        return None;
    }
    zstd::stream::decode_all(&blob[1..]).ok()
}

/// The serialized character starts at its `S2BH` tag, which sits within the first bytes of either form.
fn character_bytes(raw: &[u8]) -> Option<&[u8]> {
    let end = raw.len().min(32);
    let at = raw[..end].windows(4).position(|w| w == b"S2BH")?;
    Some(&raw[at..])
}

/// The character's own id: the 16 bytes after the `S2BH` header (tag, version, two counters, flags).
fn character_id(character: &[u8]) -> Option<[u8; 16]> {
    character.get(16..32)?.try_into().ok()
}

fn stats(character: &[u8]) -> Option<(u32, u32, u32)> {
    let one = [0x00u8, 0x00, 0x80, 0x3f];
    let limit = character.len().min(1024);
    let mut i = 0usize;
    while i + 32 <= limit {
        if (0..3).all(|k| character[i + k * 4..i + k * 4 + 4] == one) {
            let mut j = i + 12;
            while j + 4 <= limit && character[j..j + 4] == one {
                j += 4;
            }
            if j + 12 > character.len() {
                return None;
            }
            return Some((u32_le(character, j), u32_le(character, j + 4), u32_le(character, j + 8)));
        }
        i += 1;
    }
    None
}

struct Held {
    bytes: Vec<u8>,
    written_at: Option<std::time::Instant>,
    saved: bool,
}

static LAST_SEEN: Mutex<Option<HashMap<u32, u32>>> = Mutex::new(None);
static COPIES: Mutex<Option<HashMap<[u8; 16], Held>>> = Mutex::new(None);

fn copy_path(id: &[u8; 16]) -> Option<std::path::PathBuf> {
    Some(guard_dir()?.join(format!("copy_{}.bin", hex(id))))
}

fn describe(character: &[u8]) -> String {
    match stats(character) {
        Some((level, xp, next)) => format!("level {level}, xp {xp}, next {next}"),
        None => "level and xp not found".to_string(),
    }
}

/// How two serialized characters differ: first differing byte and how many differ.
fn compare(a: &[u8], b: &[u8]) -> Option<(usize, usize)> {
    let common = a.len().min(b.len());
    let mut first = None;
    let mut count = a.len().abs_diff(b.len());
    for i in 0..common {
        if a[i] != b[i] {
            first.get_or_insert(i);
            count += 1;
        }
    }
    match (first, count) {
        (None, 0) => None,
        (Some(f), c) => Some((f, c)),
        (None, c) => Some((common, c)),
    }
}

/// The server keeps the first part of the character (header, level, XP, progress); the last bytes of its copy are a trailer
/// that the upload does not repeat.
const COPY_TRAILER: usize = 5;

/// The block of the character that holds time counters: it changes with every save, with or without play.
const CLOCK_BLOCK: (usize, usize) = (512, 548);

/// Progress rule: the uploaded character must start with the server's copy. `Err` carries the first differing byte.
fn progress_rule(copy: &[u8], upload: &[u8]) -> Result<usize, usize> {
    let n = copy.len().saturating_sub(COPY_TRAILER);
    if upload.len() < n {
        return Err(upload.len());
    }
    match (0..n).find(|i| copy[*i] != upload[*i] && !(CLOCK_BLOCK.0..CLOCK_BLOCK.1).contains(i)) {
        Some(at) => Err(at),
        None => Ok(n),
    }
}

static ACCOUNTS: Mutex<Option<HashMap<String, [u8; 16]>>> = Mutex::new(None);

fn accounts_path() -> Option<std::path::PathBuf> {
    Some(guard_dir()?.join("accounts.txt"))
}

/// One character per account: `steamid:<digits> <character id>` per line.
fn load_accounts() -> HashMap<String, [u8; 16]> {
    let mut map = HashMap::new();
    let Some(text) = accounts_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return map };
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(account), Some(id)) = (parts.next(), parts.next()) else { continue };
        if id.len() == 32 {
            let mut bytes = [0u8; 16];
            let ok = (0..16).all(|i| u8::from_str_radix(&id[i * 2..i * 2 + 2], 16).map(|b| bytes[i] = b).is_ok());
            if ok {
                map.insert(account.to_string(), bytes);
            }
        }
    }
    map
}

/// Registers the account's character the first time and says whether a later join uses the same one.
fn account_rule(account: &str, id: &[u8; 16]) -> String {
    let Ok(mut guard) = ACCOUNTS.lock() else { return "account list busy".into() };
    let map = guard.get_or_insert_with(load_accounts);
    match map.get(account).copied() {
        Some(known) if known == *id => format!("{account} uses its registered character"),
        Some(known) => format!("WOULD REJECT: {account} is registered with character {} (second character)", hex(&known)),
        None => {
            map.insert(account.to_string(), *id);
            if let Some(path) = accounts_path() {
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(f, "{account} {}", hex(id));
                }
            }
            format!("{account} registered with this character")
        }
    }
}

/// The platform id of the player the upload object belongs to.
fn account_of(obj: usize) -> Option<String> {
    let bytes = read_mem(obj, 0x400)?;
    roster::guess_id(&bytes, &|p| read_mem(p, 0x40))
}

fn span(bytes: &[u8], at: usize) -> String {
    let from = at.saturating_sub(8);
    let to = (at + 16).min(bytes.len());
    hex(&bytes[from.min(to)..to])
}

fn remember(id: [u8; 16], character: &[u8], state: u8) {
    let now = std::time::Instant::now();
    let Ok(mut guard) = COPIES.lock() else { return };
    let map = guard.get_or_insert_with(HashMap::new);
    let entry = map.entry(id).or_insert_with(|| Held { bytes: Vec::new(), written_at: None, saved: true });
    let changed = entry.bytes != character;
    if changed {
        entry.bytes = character.to_vec();
        entry.saved = false;
    }
    if entry.saved {
        return;
    }
    let due = state >= FINAL_SAVE_STATE || entry.written_at.map(|t| t.elapsed().as_secs() >= WRITE_EVERY_SECS).unwrap_or(true);
    if !due {
        return;
    }
    if let Some(path) = copy_path(&id) {
        if std::fs::write(path, &entry.bytes).is_ok() {
            entry.saved = true;
            entry.written_at = Some(now);
        }
    }
}

fn server_copy(id: &[u8; 16]) -> Option<Vec<u8>> {
    if let Ok(guard) = COPIES.lock() {
        if let Some(c) = guard.as_ref().and_then(|m| m.get(id)) {
            return Some(c.bytes.clone());
        }
    }
    std::fs::read(copy_path(id)?).ok()
}

/// The server updates the copy it keeps of a connected character: `(slot, &{data, size}, state, player)`.
extern "win64" fn on_set_save(slot: usize, span_ptr: usize, extra: usize, player_arg: usize) {
    let call = SET_CALLS.fetch_add(1, Ordering::SeqCst);
    let diag = call < DIAG_CALLS;
    if slot == 0 || span_ptr == 0 {
        if diag {
            log(&format!("save call {call}: empty arguments (slot={slot:#x}, span={span_ptr:#x})"));
        }
        return;
    }
    let Some(head) = read_mem(slot + 8, 8) else { return };
    let player = u32_le(&head, 0);
    let state = head[4];
    let Some(pair) = read_mem(span_ptr, 16) else {
        if diag {
            log(&format!("save call {call}: player {player}, state {state}, span {span_ptr:#x} not readable"));
        }
        return;
    };
    let (a, b) = (usize_le(&pair, 0), usize_le(&pair, 8));
    // The span can be {pointer, size}, {size, pointer} or {begin, end}; try each.
    let blob = [(a, b), (b, a)]
        .into_iter()
        .filter(|(p, n)| *p > 0x10000 && *n >= 0x30 && *n <= MAX_BLOB)
        .find_map(|(p, n)| read_mem(p, n))
        .or_else(|| {
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            let n = hi.wrapping_sub(lo);
            if lo > 0x10000 && (0x30..=MAX_BLOB).contains(&n) { read_mem(lo, n) } else { None }
        });
    if diag {
        let first = blob.as_ref().map(|v| hex(&v[..v.len().min(24)])).unwrap_or_else(|| "none".into());
        log(&format!(
            "save call {call}: slot player {player}, state {state}, extra {extra:#x}, player arg {player_arg:#x}, span {a:#x} / {b:#x}, blob {}, starts {first}",
            blob.as_ref().map(|v| v.len()).unwrap_or(0)
        ));
    }
    let Some(blob) = blob else { return };
    record_copy(player, state, blob, call);
}

/// Stores the character found in a save blob as the server's copy.
fn record_copy(player: u32, state: u8, blob: Vec<u8>, call: usize) {
    let diag = call < DIAG_CALLS;
    let hash = fnv1a(&blob);
    if let Ok(mut seen) = LAST_SEEN.lock() {
        if seen.get_or_insert_with(HashMap::new).insert(player, hash) == Some(hash) {
            return;
        }
    }
    let raw = unpack(&blob).unwrap_or(blob);
    let Some(character) = character_bytes(&raw) else {
        if diag {
            log(&format!("save call {call}: player {player}, no S2BH header in {} bytes", raw.len()));
        }
        return;
    };
    let Some(id) = character_id(character) else { return };
    let before = server_copy(&id);
    remember(id, character, state);
    let changed = before.as_deref().map(|b| b != character).unwrap_or(true);
    if changed && (before.is_none() || before.as_deref().and_then(stats) != stats(character)) {
        log(&format!("server copy: player {player}, state {state}, character {}, {}, {} bytes", hex(&id), describe(character), character.len()));
    }
}

/// The server sends the final save of a leaving player to the client: its first chunk.
extern "win64" fn on_final_chunk(player_obj: usize, offset: usize) {
    if offset != 0 || player_obj == 0 {
        return;
    }
    let Some(head) = read_mem(player_obj + 8, 8) else { return };
    let player = u32_le(&head, 0);
    let Some(vec) = read_mem(player_obj + 0x110, 0x10) else { return };
    let (ptr, size) = (usize_le(&vec, 0), usize_le(&vec, 8));
    let call = FINAL_CALLS.fetch_add(1, Ordering::SeqCst);
    if ptr < 0x10000 || size < 0x30 || size > MAX_BLOB {
        log(&format!("final save: player {player}, unreadable blob (ptr={ptr:#x}, size={size})"));
        return;
    }
    let Some(blob) = read_mem(ptr, size) else {
        log(&format!("final save: player {player}, blob not readable (size {size})"));
        return;
    };
    if call < DIAG_CALLS {
        log(&format!("final save: player {player}, {size} bytes, starts {}", hex(&blob[..blob.len().min(16)])));
    }
    record_copy(player, FINAL_SAVE_STATE, blob, call);
}

/// A client uploads its character when it joins.
extern "win64" fn probe(_out: usize, obj: usize) {
    if obj == 0 {
        return;
    }
    let Some(head) = read_mem(obj + 8, 4) else { return };
    let player = u32_le(&head, 0);
    let Some(vec) = read_mem(obj + 0x110, 0x10) else { return };
    let (ptr, size) = (usize_le(&vec, 0), usize_le(&vec, 8));
    if ptr == 0 || size < 0x30 || size > MAX_BLOB {
        log(&format!("upload: player {player}, unreadable blob (ptr={ptr:#x}, size={size})"));
        return;
    }
    let Some(blob) = read_mem(ptr, size) else {
        log(&format!("upload: player {player}, blob not readable (size {size})"));
        return;
    };
    let hash = fnv1a(&blob);
    let Some(raw) = unpack(&blob) else {
        log(&format!("upload: player {player}, {size} bytes, blob is not 0x01 + zstd"));
        return;
    };
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if DUMPS.fetch_add(1, Ordering::SeqCst) < MAX_DUMPS.load(Ordering::SeqCst) {
        if let Some(dir) = guard_dir() {
            let _ = std::fs::write(dir.join(format!("{secs}_p{player}_{hash:08x}.raw")), &raw);
        }
    }
    let Some(character) = character_bytes(&raw) else {
        log(&format!("upload: player {player}, {size} bytes, no S2BH header"));
        return;
    };
    let Some(id) = character_id(character) else {
        log(&format!("upload: player {player}, character too short ({} bytes)", character.len()));
        return;
    };
    let header = hex(&character[..character.len().min(32)]);
    let verdict = match server_copy(&id) {
        None => "no server copy yet (first time this server sees the character)".to_string(),
        Some(copy) => match compare(&copy, character) {
            None => "identical to the server copy".to_string(),
            Some((at, count)) => format!(
                "differs from the server copy: first at byte {at}, {count} bytes differ, upload {} bytes vs copy {} bytes; upload {} | copy {}; around it upload {} vs copy {}",
                character.len(),
                copy.len(),
                describe(character),
                describe(&copy),
                span(character, at),
                span(&copy, at)
            ),
        },
    };
    let progress = match server_copy(&id) {
        None => "progress: first time, accepted".to_string(),
        Some(copy) => match progress_rule(&copy, character) {
            Ok(n) => format!("progress: matches the server copy ({n} bytes)"),
            Err(at) => format!("WOULD REJECT: progress differs from the server copy at byte {at} of {} (upload {})", copy.len().saturating_sub(COPY_TRAILER), describe(character)),
        },
    };
    let account = match account_of(obj) {
        Some(a) => account_rule(&a, &id),
        None => "account: platform id not found".to_string(),
    };
    log(&format!("join: player {player}, character {}, {}, {} bytes unpacked: {verdict} (header {header}); {progress}; {account}", hex(&id), describe(character), character.len()));
}

macro_rules! entry_stub {
    ($name:ident, $handler:ident, $trampoline:ident) => {
        #[unsafe(naked)]
        unsafe extern "win64" fn $name() {
            core::arch::naked_asm!(
                "push rcx", "push rdx", "push r8", "push r9", "push r10", "push r11",
                "sub rsp, 0x88",
                "movups [rsp + 0x20], xmm0", "movups [rsp + 0x30], xmm1", "movups [rsp + 0x40], xmm2",
                "movups [rsp + 0x50], xmm3", "movups [rsp + 0x60], xmm4", "movups [rsp + 0x70], xmm5",
                "call {f}",
                "movups xmm0, [rsp + 0x20]", "movups xmm1, [rsp + 0x30]", "movups xmm2, [rsp + 0x40]",
                "movups xmm3, [rsp + 0x50]", "movups xmm4, [rsp + 0x60]", "movups xmm5, [rsp + 0x70]",
                "add rsp, 0x88",
                "pop r11", "pop r10", "pop r9", "pop r8", "pop rdx", "pop rcx",
                "jmp qword ptr [rip + {t}]",
                f = sym $handler,
                t = sym $trampoline,
            );
        }
    };
}
entry_stub!(stub_probe, probe, TRAMPOLINE);
entry_stub!(stub_set_save, on_set_save, SET_TRAMPOLINE);

/// Hook in the middle of the per-player update: passes `rbx` and `r13` to the handler and keeps every register.
#[unsafe(naked)]
unsafe extern "win64" fn stub_final_chunk() {
    core::arch::naked_asm!(
        "push rsi", "push rax", "push rcx", "push rdx", "push r8", "push r9", "push r10", "push r11",
        "mov rsi, rsp",
        "and rsp, -16",
        "sub rsp, 0xa0",
        "movups [rsp + 0x20], xmm0", "movups [rsp + 0x30], xmm1", "movups [rsp + 0x40], xmm2",
        "movups [rsp + 0x50], xmm3", "movups [rsp + 0x60], xmm4", "movups [rsp + 0x70], xmm5",
        "mov rcx, rbx",
        "mov rdx, r13",
        "call {f}",
        "movups xmm0, [rsp + 0x20]", "movups xmm1, [rsp + 0x30]", "movups xmm2, [rsp + 0x40]",
        "movups xmm3, [rsp + 0x50]", "movups xmm4, [rsp + 0x60]", "movups xmm5, [rsp + 0x70]",
        "mov rsp, rsi",
        "pop r11", "pop r10", "pop r9", "pop r8", "pop rdx", "pop rcx", "pop rax", "pop rsi",
        "jmp qword ptr [rip + {t}]",
        f = sym on_final_chunk,
        t = sym FINAL_TRAMPOLINE,
    );
}

/// Builds the code that runs the displaced instructions and continues the original function.
fn make_trampoline(target: *mut u8, displaced: usize) -> Option<usize> {
    let page = unsafe { VirtualAlloc(std::ptr::null_mut(), 0x1000, 0x3000, 0x40) } as *mut u8;
    if page.is_null() {
        return None;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(target, page, displaced);
        let tail = page.add(displaced);
        tail.write(0xFF);
        tail.add(1).write(0x25);
        tail.add(2).cast::<u32>().write_unaligned(0);
        tail.add(6).cast::<u64>().write_unaligned(target.add(displaced) as u64);
    }
    Some(page as usize)
}

fn hook(sections: &[&'static [u8]], signature: &[u8], displaced: usize, trampoline: &AtomicUsize, stub: usize, name: &str) -> bool {
    let target = match patch::find_unique(sections, signature) {
        Ok(addr) => addr,
        Err(count) => {
            log(&format!("guard: {name} signature matched {count} times (game version changed?)"));
            return false;
        }
    };
    let Some(page) = make_trampoline(target, displaced) else {
        log(&format!("guard: {name} trampoline allocation failed"));
        return false;
    };
    trampoline.store(page, Ordering::SeqCst);
    match unsafe { patch::write_jump(target, stub) } {
        Ok(()) => true,
        Err(code) => {
            trampoline.store(0, Ordering::SeqCst);
            log(&format!("guard: {name} VirtualProtect failed ({code})"));
            false
        }
    }
}

pub(super) fn install(sections: &[&'static [u8]]) {
    let Ok(dir) = std::env::current_dir() else { return };
    let cfg = config(&dir);
    if !cfg.mode.eq_ignore_ascii_case("observe") {
        return;
    }
    MAX_DUMPS.store(cfg.max_dumps as usize, Ordering::SeqCst);
    let upload = hook(sections, &signatures::SAVE_DESERIALIZE, signatures::SAVE_DESERIALIZE_DISPLACED, &TRAMPOLINE, stub_probe as usize, "upload");
    let copy = hook(sections, &signatures::SAVE_SET, signatures::SAVE_SET_DISPLACED, &SET_TRAMPOLINE, stub_set_save as usize, "server copy");
    let fin = hook(sections, &signatures::FINAL_CHUNK, signatures::FINAL_CHUNK_DISPLACED, &FINAL_TRAMPOLINE, stub_final_chunk as usize, "final save");
    log(&format!(
        "guard: observe mode (upload hook {}, server copy hook {}, final save hook {}); nothing is blocked",
        if upload { "on" } else { "off" },
        if copy { "on" } else { "off" },
        if fin { "on" } else { "off" }
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(level: u32, tail: u8) -> Vec<u8> {
        let mut v = vec![0u8, 1, 2, 3, 4];
        v.extend_from_slice(b"S2BH");
        v.extend_from_slice(&[2, 0, 0, 0, 9, 0, 5, 0, 0, 0, 0, 1]);
        v.extend_from_slice(&[0xaa; 16]);
        for _ in 0..5 {
            v.extend_from_slice(&1.0f32.to_le_bytes());
        }
        v.extend_from_slice(&level.to_le_bytes());
        v.extend_from_slice(&100u32.to_le_bytes());
        v.extend_from_slice(&200u32.to_le_bytes());
        v.push(tail);
        v
    }

    #[test]
    fn the_character_id_and_progress_are_read_after_the_header() {
        let raw = sample(22, 7);
        let c = character_bytes(&raw).unwrap();
        assert_eq!(&c[..4], b"S2BH");
        assert_eq!(character_id(c), Some([0xaa; 16]));
        assert_eq!(stats(c), Some((22, 100, 200)));
    }

    #[test]
    fn a_changed_character_is_reported_with_where_and_how_much() {
        let a = sample(22, 7);
        let b = sample(23, 7);
        let (ca, cb) = (character_bytes(&a).unwrap(), character_bytes(&b).unwrap());
        assert_eq!(compare(ca, ca), None);
        let (at, count) = compare(ca, cb).unwrap();
        assert_eq!((at, count), (16 + 16 + 20, 1));
        let longer = sample(22, 7).into_iter().chain([1]).collect::<Vec<u8>>();
        assert_eq!(compare(ca, character_bytes(&longer).unwrap()), Some((ca.len(), 1)));
    }

    #[test]
    fn the_progress_rule_accepts_the_server_prefix_and_rejects_other_progress() {
        let mut upload = sample(46, 7);
        upload.resize(700, 3);
        let mut copy = upload[..600].to_vec();
        copy[596] = 9;
        assert_eq!(progress_rule(&copy, &upload), Ok(595));
        let mut richer = upload.clone();
        richer[100] ^= 0xff;
        assert_eq!(progress_rule(&copy, &richer), Err(100));
        assert_eq!(progress_rule(&copy, &upload[..200]), Err(200));
        let mut clock = upload.clone();
        clock[520] ^= 0xff;
        assert_eq!(progress_rule(&copy, &clock), Ok(595));
    }

    #[test]
    fn the_server_copy_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("m182_guard_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let raw = sample(30, 1);
        let c = character_bytes(&raw).unwrap();
        let id = character_id(c).unwrap();
        remember(id, c, FINAL_SAVE_STATE);
        COPIES.lock().unwrap().take();
        assert_eq!(server_copy(&id).as_deref(), Some(c));
        let _ = std::fs::remove_dir_all(dir);
    }
}

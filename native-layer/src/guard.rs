//! M182 Guard: the server-side character lock.
//!
//! Nothing here runs on a timer. Two hooks fire only when the game itself handles a character: the upload a client sends
//! when it joins, and the save the server sends to a playing client (which is the copy the server holds). A character is
//! checked once, when it is uploaded:
//!
//! * one character per account (the account comes from the server's player table);
//! * its level, XP and plain item stacks must be the ones in one of the last copies the server held;
//! * optionally, a character that is new to the server may not start above a level.
//!
//! Administrators (kick/ban permission) and the entries of `m182_guard_allow.txt` skip every rule. A rejected upload is
//! damaged before the game reads it, so the game itself refuses the character with its own "savegame corrupt" error.
//! Configuration lives in `m182_guard.json`, so it never touches the panel or VFX.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;

use super::{patch, roster, signatures, table};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GuardConfig {
    /// "off" (default), "observe" (only says what it would refuse) or "enforce".
    pub mode: String,
    /// Players with the kick/ban permission are never refused.
    pub admins_exempt: bool,
    pub one_character_per_account: bool,
    /// A character the server has not seen yet may not be above this level (0 = no limit).
    pub new_character_max_level: u32,
    /// Also write the normal decisions to the log, not only the refusals.
    pub verbose: bool,
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self { mode: "off".into(), admins_exempt: true, one_character_per_account: true, new_character_max_level: 0, verbose: false }
    }
}

static CONFIG: OnceLock<GuardConfig> = OnceLock::new();
static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static FINAL_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
/// Set when two players online at once turn out to share the same account value: the one-character rule is then unusable.
static KEY_SHARED: AtomicBool = AtomicBool::new(false);

const MAX_BLOB: usize = 8 * 1024 * 1024;
const NOISE_WRITE_SECS: u64 = 20;
/// How many different states of a character the server remembers (a client may hold an older one if a save got lost).
const VERSIONS: usize = 3;
/// The last bytes of the server's copy are a trailer the upload does not repeat.
const COPY_TRAILER: usize = 5;
/// Offsets of the two account parts inside a player table slot.
const ACCOUNT_PARTS: [(usize, usize); 2] = [(72, 80), (96, 104)];

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

fn cfg() -> &'static GuardConfig {
    CONFIG.get_or_init(GuardConfig::default)
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

fn say(msg: &str) {
    if cfg().verbose {
        log(msg);
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

/// Level, XP and next-level value: they follow the run of 1.0 floats in the header.
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

/// The plain item stacks of the inventory pages: records of item id (4 bytes), count (3 bytes) and slot (1 byte), found
/// where two neighbouring records have consecutive slots. Items with data of their own (weapons, armor) are not plain
/// stacks and are not listed.
fn stacks(bytes: &[u8]) -> Vec<(u32, u32)> {
    let rec = |i: usize| -> Option<(u32, u32, u8)> {
        let b = bytes.get(i..i + 8)?;
        let item = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let count = b[4] as u32 | (b[5] as u32) << 8 | (b[6] as u32) << 16;
        (item != 0 && count > 0 && count < 100_000).then_some((item, count, b[7]))
    };
    let mut out = Vec::new();
    for i in 0..bytes.len().saturating_sub(7) {
        let Some((item, count, slot)) = rec(i) else { continue };
        let next = rec(i + 8).map(|r| r.2 == slot.wrapping_add(1)).unwrap_or(false);
        let prev = i.checked_sub(8).and_then(rec).map(|r| r.2 == slot.wrapping_sub(1)).unwrap_or(false);
        if next || prev {
            out.push((item, count));
        }
    }
    out.sort_unstable();
    out
}

/// What the server remembers of a character: the copy without its trailer.
fn body(copy: &[u8]) -> &[u8] {
    &copy[..copy.len().saturating_sub(COPY_TRAILER)]
}

fn describe(character: &[u8]) -> String {
    match stats(character) {
        Some((level, xp, next)) => format!("level {level}, xp {xp}, next {next}"),
        None => "level and xp not found".to_string(),
    }
}

/// What changed between two item lists, as text.
fn stack_diff(before: &[(u32, u32)], now: &[(u32, u32)]) -> String {
    let total = |list: &[(u32, u32)], item: u32| list.iter().filter(|e| e.0 == item).map(|e| e.1 as i64).sum::<i64>();
    let mut items: Vec<u32> = before.iter().chain(now.iter()).map(|e| e.0).collect();
    items.sort_unstable();
    items.dedup();
    let mut parts: Vec<String> = items
        .into_iter()
        .filter_map(|item| {
            let delta = total(now, item) - total(before, item);
            (delta != 0).then(|| format!("{}{} x{}", if delta > 0 { "+" } else { "-" }, item, delta.abs()))
        })
        .collect();
    let all = parts.len();
    parts.truncate(8);
    if all > 8 {
        parts.push(format!("and {} more", all - 8));
    }
    parts.join(", ")
}

/// The upload must be one of the remembered states of the character: same level and XP, same plain item stacks.
fn matches_copies(copies: &[Vec<u8>], upload: &[u8]) -> Result<(), String> {
    let Some(sent) = stats(upload) else { return Ok(()) };
    let mut why = String::new();
    for (k, copy) in copies.iter().enumerate() {
        let Some(have) = stats(copy) else { return Ok(()) };
        if have != sent {
            if k == 0 {
                why = format!("progress differs from the server copy (server: level {} xp {}, upload: level {} xp {})", have.0, have.1, sent.0, sent.1);
            }
            continue;
        }
        let kept = stacks(body(copy));
        let got = stacks(&upload[..body(copy).len().min(upload.len())]);
        if got == kept {
            return Ok(());
        }
        if k == 0 {
            why = format!("items differ from the server copy ({})", stack_diff(&kept, &got));
        }
    }
    Err(why)
}

struct Held {
    versions: Vec<Vec<u8>>,
    written_at: Option<std::time::Instant>,
    saved: bool,
}

static COPIES: Mutex<Option<HashMap<[u8; 16], Held>>> = Mutex::new(None);
static LAST_SEEN: Mutex<Option<HashMap<u32, u32>>> = Mutex::new(None);

fn copy_path(id: &[u8; 16]) -> Option<std::path::PathBuf> {
    Some(guard_dir()?.join(format!("held_{}.bin", hex(id))))
}

fn encode_versions(versions: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for v in versions {
        out.extend_from_slice(&(v.len() as u32).to_le_bytes());
        out.extend_from_slice(v);
    }
    out
}

fn decode_versions(data: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 4 <= data.len() && out.len() < VERSIONS {
        let len = u32_le(data, at) as usize;
        at += 4;
        let Some(v) = data.get(at..at + len) else { break };
        out.push(v.to_vec());
        at += len;
    }
    out
}

fn same_state(a: &[u8], b: &[u8]) -> bool {
    stats(a) == stats(b) && stacks(body(a)) == stacks(body(b))
}

/// Records the copy the server sends to a playing client. A new state is written at once; changes that are only
/// counters and timestamps replace the newest copy and are written at most every few seconds.
fn remember(id: [u8; 16], character: &[u8]) {
    let Ok(mut guard) = COPIES.lock() else { return };
    let map = guard.get_or_insert_with(HashMap::new);
    let entry = map.entry(id).or_insert_with(|| {
        let versions = copy_path(&id).and_then(|p| std::fs::read(p).ok()).map(|d| decode_versions(&d)).unwrap_or_default();
        Held { versions, written_at: None, saved: true }
    });
    let mut urgent = false;
    match entry.versions.first() {
        None => {
            entry.versions.insert(0, character.to_vec());
            urgent = true;
        }
        Some(newest) if newest == character => return,
        Some(newest) if same_state(newest, character) => {
            entry.versions[0] = character.to_vec();
            entry.saved = false;
        }
        Some(_) => {
            entry.versions.insert(0, character.to_vec());
            entry.versions.truncate(VERSIONS);
            entry.saved = false;
            urgent = true;
        }
    }
    let due = urgent || entry.written_at.map(|t| t.elapsed().as_secs() >= NOISE_WRITE_SECS).unwrap_or(true);
    if entry.saved || !due {
        return;
    }
    if let Some(path) = copy_path(&id) {
        if std::fs::write(path, encode_versions(&entry.versions)).is_ok() {
            entry.saved = true;
            entry.written_at = Some(std::time::Instant::now());
        }
    }
    if urgent {
        say(&format!("copy: character {}, {}, {} bytes", hex(&id), describe(character), character.len()));
    }
}

fn server_copies(id: &[u8; 16]) -> Vec<Vec<u8>> {
    if let Ok(guard) = COPIES.lock() {
        if let Some(c) = guard.as_ref().and_then(|m| m.get(id)) {
            return c.versions.clone();
        }
    }
    copy_path(id).and_then(|p| std::fs::read(p).ok()).map(|d| decode_versions(&d)).unwrap_or_default()
}

// ------------------------------------------------------------------------------------------------ accounts

static ACCOUNTS: Mutex<Option<HashMap<String, [u8; 16]>>> = Mutex::new(None);

fn accounts_path() -> Option<std::path::PathBuf> {
    Some(guard_dir()?.join("accounts.txt"))
}

fn parse_id(text: &str) -> Option<[u8; 16]> {
    if text.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

/// One character per account: `<account> <character id>` per line.
fn load_accounts() -> HashMap<String, [u8; 16]> {
    let Some(text) = accounts_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return HashMap::new() };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            Some((parts.next()?.to_string(), parse_id(parts.next()?)?))
        })
        .collect()
}

/// The character registered to an account, registering this one when the account has none yet.
fn registered(account: &str, id: &[u8; 16], register: bool) -> [u8; 16] {
    let Ok(mut guard) = ACCOUNTS.lock() else { return *id };
    let map = guard.get_or_insert_with(load_accounts);
    if let Some(known) = map.get(account) {
        return *known;
    }
    if register {
        map.insert(account.to_string(), *id);
        if let Some(path) = accounts_path() {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "{account} {}", hex(id));
            }
        }
    }
    *id
}

/// The account value in a player table slot: the same in every session and for every character of one account.
fn slot_key(slot: &[u8]) -> Option<String> {
    let mut key = Vec::new();
    for (a, b) in ACCOUNT_PARTS {
        key.extend_from_slice(slot.get(a..b)?);
    }
    key.iter().any(|b| *b != 0).then(|| format!("uid:{}", hex(&key)))
}

struct Who {
    account: Option<String>,
    admin: bool,
}

fn who_is(player: u32) -> Who {
    let slot = table::slot_dump(player);
    let admin = slot.as_ref().and_then(|s| s.get(roster::SLOT_PERMISSIONS)).map(|p| roster::is_admin(*p)).unwrap_or(false);
    let steam = table::entries().into_iter().find(|e| e.client == player).and_then(|e| e.id);
    let account = steam.or_else(|| slot.as_deref().and_then(slot_key));
    Who { account, admin }
}

/// Two players online at once with the same account value mean the value does not identify accounts.
fn check_key_is_personal(player: u32, key: &str) {
    if KEY_SHARED.load(Ordering::SeqCst) || !key.starts_with("uid:") {
        return;
    }
    for e in table::entries() {
        if e.client != player && table::slot_dump(e.client).as_deref().and_then(slot_key).as_deref() == Some(key) {
            KEY_SHARED.store(true, Ordering::SeqCst);
            log("ERROR: two different players share the same account value; the one-character-per-account rule is switched off");
            return;
        }
    }
}

// ------------------------------------------------------------------------------------------------ allow list

fn allow_path() -> Option<std::path::PathBuf> {
    Some(std::env::current_dir().ok()?.join("m182_guard_allow.txt"))
}

/// The allow list is read when a character joins: one account value or character id per line, `#` starts a comment.
fn allowed(account: Option<&str>, id: &[u8; 16]) -> bool {
    let Some(text) = allow_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return false };
    let id = hex(id);
    text.lines().filter_map(|l| l.split('#').next()?.split_whitespace().next()).any(|entry| {
        entry.eq_ignore_ascii_case(&id) || account.map(|a| a.eq_ignore_ascii_case(entry)).unwrap_or(false)
    })
}

// ------------------------------------------------------------------------------------------------ hooks

/// The server sends a save to a playing (or leaving) client: its first chunk. This is the copy the server holds.
extern "win64" fn on_final_chunk(player_obj: usize, offset: usize) {
    if offset != 0 || player_obj == 0 {
        return;
    }
    let Some(head) = read_mem(player_obj + 8, 4) else { return };
    let player = u32_le(&head, 0);
    let Some(vec) = read_mem(player_obj + 0x110, 0x10) else { return };
    let (ptr, size) = (usize_le(&vec, 0), usize_le(&vec, 8));
    if ptr < 0x10000 || size < 0x30 || size > MAX_BLOB {
        return;
    }
    let Some(blob) = read_mem(ptr, size) else { return };
    let hash = fnv1a(&blob);
    if let Ok(mut seen) = LAST_SEEN.lock() {
        if seen.get_or_insert_with(HashMap::new).insert(player, hash) == Some(hash) {
            return;
        }
    }
    let raw = unpack(&blob).unwrap_or(blob);
    let Some(character) = character_bytes(&raw) else { return };
    let Some(id) = character_id(character) else { return };
    remember(id, character);
}

/// Damages the frame of the uploaded character so that the game refuses to read it.
fn refuse_upload(ptr: usize) {
    for i in 1..5 {
        unsafe { std::ptr::write_volatile((ptr + i) as *mut u8, 0) };
    }
}

enum Decision {
    Allow(String),
    Refuse(String),
}

fn decide(player: u32, id: &[u8; 16], character: &[u8]) -> Decision {
    let who = who_is(player);
    let account = who.account.as_deref();
    if allowed(account, id) {
        return Decision::Allow("on the allow list".into());
    }
    if cfg().admins_exempt && who.admin {
        return Decision::Allow("administrator".into());
    }
    if cfg().one_character_per_account && !KEY_SHARED.load(Ordering::SeqCst) {
        match account {
            Some(a) => {
                check_key_is_personal(player, a);
                if !KEY_SHARED.load(Ordering::SeqCst) {
                    let owned = registered(a, id, false);
                    if owned != *id {
                        return Decision::Refuse(format!("this account already has the character {}", hex(&owned)));
                    }
                }
            }
            None => say(&format!("join: player {player}: account not found, one-character rule skipped")),
        }
    }
    let copies = server_copies(id);
    if copies.is_empty() {
        let limit = cfg().new_character_max_level;
        if limit > 0 {
            if let Some((level, _, _)) = stats(character) {
                if level > limit {
                    return Decision::Refuse(format!("a new character may not start above level {limit} (this one is level {level})"));
                }
            }
        }
    } else if let Err(why) = matches_copies(&copies, character) {
        return Decision::Refuse(why);
    }
    if let Some(a) = account {
        if cfg().one_character_per_account && !KEY_SHARED.load(Ordering::SeqCst) {
            registered(a, id, true);
        }
    }
    Decision::Allow(if copies.is_empty() { "new to this server".into() } else { "matches the server copy".into() })
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
        return;
    }
    let Some(blob) = read_mem(ptr, size) else { return };
    let Some(raw) = unpack(&blob) else { return };
    let Some(character) = character_bytes(&raw) else { return };
    let Some(id) = character_id(character) else { return };
    let enforce = cfg().mode.eq_ignore_ascii_case("enforce");
    match decide(player, &id, character) {
        Decision::Allow(reason) => say(&format!("join: player {player}, character {}, {}: allowed ({reason})", hex(&id), describe(character))),
        Decision::Refuse(why) => {
            let who = who_is(player);
            log(&format!(
                "{}: player {player}, account {}, character {}: {why}. To let this player in, add the account or the character to m182_guard_allow.txt",
                if enforce { "REFUSED" } else { "WOULD REFUSE" },
                who.account.as_deref().unwrap_or("unknown"),
                hex(&id)
            ));
            if enforce {
                refuse_upload(ptr);
            }
        }
    }
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
    if !(cfg.mode.eq_ignore_ascii_case("observe") || cfg.mode.eq_ignore_ascii_case("enforce")) {
        return;
    }
    let _ = CONFIG.set(cfg.clone());
    let upload = hook(sections, &signatures::SAVE_DESERIALIZE, signatures::SAVE_DESERIALIZE_DISPLACED, &TRAMPOLINE, stub_probe as usize, "upload");
    let copy = hook(sections, &signatures::FINAL_CHUNK, signatures::FINAL_CHUNK_DISPLACED, &FINAL_TRAMPOLINE, stub_final_chunk as usize, "server copy");
    if upload && copy {
        log(&format!("guard: {} mode, admins exempt: {}, one character per account: {}", cfg.mode, cfg.admins_exempt, cfg.one_character_per_account));
    } else {
        log(&format!(
            "ERROR: the character lock could not start (upload hook {}, server copy hook {}); nobody is refused",
            if upload { "on" } else { "off" },
            if copy { "on" } else { "off" }
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn character(level: u32, xp: u32, items: &[(u32, u32)]) -> Vec<u8> {
        let mut c = b"S2BH".to_vec();
        c.extend_from_slice(&[0u8; 12]);
        c.extend_from_slice(&[7u8; 16]);
        for _ in 0..4 {
            c.extend_from_slice(&1.0f32.to_le_bytes());
        }
        for v in [level, xp, 160] {
            c.extend_from_slice(&v.to_le_bytes());
        }
        c.extend_from_slice(&[0xAA; 24]);
        for (slot, (item, count)) in items.iter().enumerate() {
            c.extend_from_slice(&item.to_le_bytes());
            c.extend_from_slice(&count.to_le_bytes()[..3]);
            c.push(slot as u8);
        }
        c.extend_from_slice(&[0xBB; 40]);
        c
    }

    /// What the server keeps: the character followed by its 5-byte trailer.
    fn held(c: &[u8], trailer: u8) -> Vec<u8> {
        let mut v = c.to_vec();
        v.extend_from_slice(&[trailer; COPY_TRAILER]);
        v
    }

    /// What a client uploads: the same prefix, noise in the counters, and a long static tail.
    fn upload(c: &[u8]) -> Vec<u8> {
        let mut v = c.to_vec();
        v[5] = 0x55;
        v.extend_from_slice(&[0xCC; 5000]);
        v
    }

    #[test]
    fn the_id_and_progress_are_read_after_the_header() {
        let c = character(24, 1802, &[(100, 5)]);
        assert_eq!(character_id(&c), Some([7u8; 16]));
        assert_eq!(stats(&c), Some((24, 1802, 160)));
    }

    #[test]
    fn plain_stacks_are_found_only_in_chains() {
        let c = character(1, 0, &[(100, 5), (200, 7), (300, 9)]);
        assert_eq!(stacks(&c), vec![(100, 5), (200, 7), (300, 9)]);
        let lone = character(1, 0, &[(100, 5)]);
        assert!(stacks(&lone).is_empty());
    }

    #[test]
    fn the_same_character_is_accepted_even_with_noise() {
        let c = character(24, 1802, &[(100, 5), (200, 7), (300, 9)]);
        assert!(matches_copies(&[held(&c, 1)], &upload(&c)).is_ok());
    }

    #[test]
    fn brought_items_and_progress_are_refused() {
        let c = character(24, 1802, &[(100, 5), (200, 7), (300, 9)]);
        let more_stone = character(24, 1802, &[(100, 5), (200, 21), (300, 9)]);
        let why = matches_copies(&[held(&c, 1)], &upload(&more_stone)).unwrap_err();
        assert!(why.contains("+200 x14"), "{why}");
        let more_xp = character(24, 5764, &[(100, 5), (200, 7), (300, 9)]);
        let why = matches_copies(&[held(&c, 1)], &upload(&more_xp)).unwrap_err();
        assert!(why.contains("xp 5764"), "{why}");
    }

    #[test]
    fn an_older_remembered_state_is_still_accepted() {
        let old = character(24, 1700, &[(100, 5), (200, 7), (300, 9)]);
        let new = character(24, 1802, &[(100, 5), (200, 8), (300, 9)]);
        assert!(matches_copies(&[held(&new, 2), held(&old, 1)], &upload(&old)).is_ok());
    }

    #[test]
    fn a_blob_without_readable_progress_is_not_refused() {
        let c = character(24, 1802, &[(100, 5)]);
        assert!(matches_copies(&[held(&c, 1)], b"S2BH not a character").is_ok());
    }

    #[test]
    fn the_account_value_ignores_the_character_part() {
        let mut slot = vec![0u8; 0x148];
        slot[72..80].copy_from_slice(&[1; 8]);
        slot[96..104].copy_from_slice(&[2; 8]);
        slot[80..96].copy_from_slice(&[3; 16]);
        let key = slot_key(&slot).unwrap();
        slot[80..96].copy_from_slice(&[9; 16]);
        assert_eq!(slot_key(&slot).unwrap(), key);
        slot[72] = 5;
        assert_ne!(slot_key(&slot).unwrap(), key);
        assert!(slot_key(&vec![0u8; 0x148]).is_none());
    }

    #[test]
    fn remembered_states_survive_a_restart() {
        let a = held(&character(24, 1700, &[(100, 5)]), 1);
        let b = held(&character(24, 1802, &[(100, 6)]), 2);
        assert_eq!(decode_versions(&encode_versions(&[b.clone(), a.clone()])), vec![b, a]);
    }

    #[test]
    fn a_refused_upload_loses_its_zstd_magic() {
        let mut blob = vec![1u8, 0x28, 0xb5, 0x2f, 0xfd, 9, 9, 9, 9, 9, 9, 9];
        refuse_upload(blob.as_mut_ptr() as usize);
        assert!(unpack(&blob).is_none());
        assert_eq!(blob[0], 1);
    }
}

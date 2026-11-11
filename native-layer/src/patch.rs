use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use super::{config::LIVE, faction, groups, signatures};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Finished (patched, or a signature is missing and was reported).
    Done,
    /// Game code was not readable yet.
    Retry,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetCurrentProcess() -> *mut c_void;
    fn VirtualProtect(addr: *mut c_void, size: usize, new: u32, old: *mut u32) -> i32;
    fn VirtualAlloc(addr: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
    fn FlushInstructionCache(process: *mut c_void, addr: *const c_void, size: usize) -> i32;
}

const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const JUMP_LEN: usize = 14;
const MEM_COMMIT_RESERVE: u32 = 0x3000;

static INSTALLED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------- hooks

#[inline]
fn read_faction(ptr: *const u8) -> u8 {
    if ptr.is_null() { 0 } else { unsafe { ptr.read() } }
}

static TEAM_CHECKS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

static ZONE_PAIRS: std::sync::Mutex<Vec<(u8, u8, bool)>> = std::sync::Mutex::new(Vec::new());

fn team_check_report(a: u8, b: u8, hostile: bool, via: &str) {
    if faction::zone_pvp() && matches!(a, 7 | 8 | 11) && matches!(b, 7 | 8 | 11) {
        if let Ok(mut seen) = ZONE_PAIRS.lock() {
            if !seen.contains(&(a, b, hostile)) && seen.len() < 40 {
                seen.push((a, b, hostile));
                let line = format!("pvp zones: relation check {via} {a} vs {b} -> {}", if hostile { "hostile" } else { "not hostile" });
                super::spawner::log(&line);
            }
        }
    }
    let teams = faction::TEAM_FACTIONS;
    if (teams.contains(&a) || teams.contains(&b)) && faction::team_mask() != 0 && TEAM_CHECKS.fetch_add(1, Ordering::Relaxed) < 30 {
        super::spawner::log(&format!("teams: relation check {via} {a} vs {b} -> {}", if hostile { "hostile" } else { "not hostile" }));
    }
}

extern "win64" fn enemy_values(rcx: usize, rdx: usize) -> u8 {
    super::probe::hit(super::probe::ENEMY_VALUES);
    let (a, b) = (rcx as u8, rdx as u8);
    let hostile = faction::enemy(a, b, faction::pvp_effective(LIVE.pvp()));
    team_check_report(a, b, hostile, "values");
    hostile as u8
}

extern "win64" fn enemy_ptrs(a: *const u8, b: *const u8) -> u8 {
    super::probe::hit(super::probe::ENEMY_PTRS);
    let (fa, fb) = (read_faction(a), read_faction(b));
    if groups::is_protected(a as usize) || groups::is_protected(b as usize) {
        return 0;
    }
    let hostile = faction::enemy(fa, fb, faction::pvp_effective(LIVE.pvp()));
    team_check_report(fa, fb, hostile, "pointers");
    hostile as u8
}

extern "win64" fn friend_values(rcx: usize, rdx: usize) -> u8 {
    super::probe::hit(super::probe::FRIEND_VALUES);
    faction::friend(rcx as u8, rdx as u8, LIVE.pvp_friend_checks()) as u8
}

extern "win64" fn friend_opts(a: *const u8, b: *const u8, opts: *const u8) -> u8 {
    super::probe::hit(super::probe::FRIEND_OPTS);
    let flag = !opts.is_null() && unsafe { opts.add(1).read() } != 0;
    let (fa, fb) = (read_faction(a), read_faction(b));
    faction::friend_opts(fa, fb, flag, LIVE.pvp_friend_checks()) as u8
}

// The game may rely on registers that the Windows ABI treats as volatile (it was
// built with whole-program optimisation), so the stubs preserve all of them.
macro_rules! stub {
    ($name:ident, $target:path) => {
        #[unsafe(naked)]
        unsafe extern "win64" fn $name() {
            core::arch::naked_asm!(
                "push rcx",
                "push rdx",
                "push r8",
                "push r9",
                "push r10",
                "push r11",
                "sub rsp, 0x88",
                "movups [rsp + 0x20], xmm0",
                "movups [rsp + 0x30], xmm1",
                "movups [rsp + 0x40], xmm2",
                "movups [rsp + 0x50], xmm3",
                "movups [rsp + 0x60], xmm4",
                "movups [rsp + 0x70], xmm5",
                "call {f}",
                "movups xmm0, [rsp + 0x20]",
                "movups xmm1, [rsp + 0x30]",
                "movups xmm2, [rsp + 0x40]",
                "movups xmm3, [rsp + 0x50]",
                "movups xmm4, [rsp + 0x60]",
                "movups xmm5, [rsp + 0x70]",
                "add rsp, 0x88",
                "pop r11",
                "pop r10",
                "pop r9",
                "pop r8",
                "pop rdx",
                "pop rcx",
                "ret",
                f = sym $target,
            );
        }
    };
}

stub!(stub_enemy_values, enemy_values);
stub!(stub_enemy_ptrs, enemy_ptrs);
stub!(stub_friend_values, friend_values);
stub!(stub_friend_opts, friend_opts);


// ---------------------------------------------------------------- chat commands

pub(super) static CHAT_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
pub(super) static PUMP_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
/// The server's player table, captured the first time the game runs its per-tick chat pump.
pub(super) static PLAYERS_OBJECT: AtomicUsize = AtomicUsize::new(0);

static REPLY: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());

static PERMISSIONS_RESTRICTED: AtomicU64 = AtomicU64::new(0);
static PERMISSIONS_HOOKED: AtomicBool = AtomicBool::new(false);

pub fn permissions_hooked() -> bool {
    PERMISSIONS_HOOKED.load(Ordering::SeqCst)
}

/// Entities (1-based, up to 64) whose permissions the game must see without chests and building.
pub fn restrict_permissions(entities: u64) {
    PERMISSIONS_RESTRICTED.store(entities, Ordering::SeqCst);
}

pub fn restricted_permissions() -> u64 {
    PERMISSIONS_RESTRICTED.load(Ordering::SeqCst)
}

extern "win64" fn filter_permissions(entity: u32, permissions: u32, caller: usize) -> u32 {
    super::probe::hit(super::probe::PERMISSIONS);
    super::outbox::pump();
    let bit = entity.wrapping_sub(1);
    let shown = if bit < 64 && PERMISSIONS_RESTRICTED.load(Ordering::Relaxed) >> bit & 1 != 0 {
        super::roster::visitor_permissions(permissions as u8) as u32
    } else {
        permissions
    };
    shown
}

// Tail of the game's permission lookup: reads the stored byte, lets `filter_permissions` narrow it
// for visitors and finishes the way the original did.
#[unsafe(naked)]
unsafe extern "win64" fn stub_permissions() {
    core::arch::naked_asm!(
        "movzx eax, byte ptr [r9 + r8 + 0x9d]",
        "push rcx",
        "push rdx",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "sub rsp, 0x88",
        "movups [rsp + 0x20], xmm0",
        "movups [rsp + 0x30], xmm1",
        "movups [rsp + 0x40], xmm2",
        "movups [rsp + 0x50], xmm3",
        "movups [rsp + 0x60], xmm4",
        "movups [rsp + 0x70], xmm5",
        "mov ecx, edx",
        "mov edx, eax",
        "mov r8, [rsp + 0xb8]",
        "call {f}",
        "movups xmm0, [rsp + 0x20]",
        "movups xmm1, [rsp + 0x30]",
        "movups xmm2, [rsp + 0x40]",
        "movups xmm3, [rsp + 0x50]",
        "movups xmm4, [rsp + 0x60]",
        "movups xmm5, [rsp + 0x70]",
        "add rsp, 0x88",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdx",
        "pop rcx",
        "mov byte ptr [rcx], al",
        "mov rax, rcx",
        "ret",
        f = sym filter_permissions,
    );
}

/// Replaces the text of the message being queued, so the game delivers our line instead.
fn replace_text(text: usize, line: &str) -> bool {
    let Ok(mut buffer) = REPLY.lock() else { return false };
    buffer.clear();
    buffer.reserve(1100);
    let bytes = line.as_bytes();
    buffer.extend_from_slice(&bytes[..bytes.len().min(0x3f0)]);
    let (pointer, length) = (buffer.as_ptr() as u64, buffer.len() as u64);
    let mut head = [0u8; 16];
    head[0..8].copy_from_slice(&pointer.to_le_bytes());
    head[8..16].copy_from_slice(&length.to_le_bytes());
    super::spawner::write_bytes(text, &head)
}

/// `!equipo ...`: the answer replaces the typed line, so everyone in the chat sees it.
fn team_command(text: usize, message: &str, client: u32) -> u8 {
    let asking = (client & 0x3f) + 1;
    let online: Vec<super::invites::Person> = super::table::entries()
        .into_iter()
        .map(|e| super::invites::Person { entity: e.entity(), name: e.name.clone().unwrap_or_else(|| format!("ID {}", e.entity())), id: e.id.clone() })
        .collect();
    let Some(who) = online.iter().find(|p| p.entity == asking).cloned() else { return 1 };
    let Ok(dir) = std::env::current_dir() else { return 1 };
    let Some(line) = super::invites::handle(message, &who, &online, &dir) else { return 1 };
    super::spawner::log(&format!("teams: {} -> {line}", who.name));
    if line.contains("cannot find") || line.contains("No encuentro") {
        super::spawner::log(&format!("teams: online now {:?}; typed \"{}\"", online.iter().map(|p| (p.entity, p.name.clone())).collect::<Vec<_>>(), message.trim()));
    }
    if replace_text(text, &line) { 0 } else { 1 }
}

/// True once the chat hook is in place on a client, so the answers to the panel's requests are read straight
/// from the incoming chat line instead of searching the game's memory for them.
static REPLY_HOOK: AtomicBool = AtomicBool::new(false);

static HOOK_TAKEN: AtomicU32 = AtomicU32::new(0);

/// True once the chat hook has really read an answer on this game. Until then the answers are also searched in
/// the game's memory, because the hook only helps if the game delivers incoming chat through the hooked function.
pub(super) fn replies_read_by_hook() -> bool {
    REPLY_HOOK.load(Ordering::Relaxed) && HOOK_TAKEN.load(Ordering::Relaxed) > 0
}

fn message_text(text: usize) -> Option<String> {
    let head = super::spawn_probe::read_bytes(text, 16)?;
    let (pointer, length) = (super::spawn_probe::u64_at(&head, 0) as usize, super::spawn_probe::u64_at(&head, 8) as usize);
    if length == 0 || length > 0x400 {
        return None;
    }
    let bytes = super::spawn_probe::read_bytes(pointer, length)?;
    Some(String::from_utf8_lossy(&bytes).to_string())
}

/// An answer the server wrote for this game's panel (`[M182#n] ...` player list, `[M182T#n] ...` teams). It is
/// read here and never reaches the chat window. Returns true when the line was one of ours.
pub fn take_server_reply(text: usize) -> bool {
    let Some(head) = super::spawn_probe::read_bytes(text, 16) else { return false };
    let length = super::spawn_probe::u64_at(&head, 8) as usize;
    if length < 9 || length > 0x400 {
        return false;
    }
    let pointer = super::spawn_probe::u64_at(&head, 0) as usize;
    let Some(start) = super::spawn_probe::read_bytes(pointer, 7) else { return false };
    if &start[..5] != b"[M182" {
        return false;
    }
    let Some(message) = message_text(text) else { return false };
    let taken = accept_reply(&message);
    if taken && HOOK_TAKEN.fetch_add(1, Ordering::Relaxed) == 0 {
        super::spawner::log("chat hook: the first server answer was read straight from the incoming chat");
    }
    taken
}

/// Same, for a line this process produced itself (an asker living in the game that hosts the world).
pub fn accept_reply(message: &str) -> bool {
    let taken = accept_reply_line(message);
    if taken {
        super::outbox::note_answer();
    }
    taken
}

fn accept_reply_line(message: &str) -> bool {
    let Ok(dir) = std::env::current_dir() else { return false };
    let number = |from: usize| -> Option<u32> {
        let rest = &message[from..];
        rest[..rest.find(']')?].parse().ok()
    };
    if message.starts_with("[M182#") {
        let Some(seq) = number(6) else { return false };
        match super::roster::parse_reply(message, seq) {
            Some(list) => {
                let own = super::roster::parse_own(message, seq);
                super::spawner::store_roster(&dir, seq, &list, own);
                super::spawner::log(&format!("roster {seq}: {} player(s) read from the server's answer", list.len()));
            }
            None => super::spawner::log(&format!("roster {seq}: the server's answer could not be read")),
        }
        return true;
    }
    if message.starts_with(super::shared::REV_TAG) {
        let note = super::shared::receive_revs(&dir, message);
        if !note.is_empty() {
            super::spawner::log(&note);
        }
        return true;
    }
    if message.starts_with(super::shared::SNAP_TAG) {
        let note = super::shared::receive_snapshot(&dir, message);
        if !note.is_empty() {
            super::spawner::log(&note);
        }
        return true;
    }
    if message.starts_with("[M182T#") {
        let Some(seq) = number(7) else { return false };
        match super::groups::parse_live_line(message, seq) {
            Some((live, complete)) => {
                super::spawner::store_teams(&dir, &live, complete);
                super::spawner::log(&format!("teams {seq}: {} team(s) read from the server's answer", live.len()));
            }
            None => super::spawner::log(&format!("teams {seq}: the server's answer could not be read")),
        }
        return true;
    }
    false
}

type EnqueueFn = extern "win64" fn(*mut u8, usize, usize, usize, u32, u32, u32, u64, usize) -> usize;

/// Queues the (already replaced) message through the game's own function, then marks it as delivered for every
/// player except the one who asked, so the answer to a panel request is not shown to the whole server.
/// Returns false when it could not be done and the caller should let the game broadcast it as before.
fn send_to_asker_only(out: usize, queue: usize, kind: usize, text: usize, client: usize, frame: usize) -> bool {
    send_to_peers(out, queue, kind, text, &[client], frame, true)
}

/// Same, for several players at once (the panels that must hear about a change).
fn send_to_peers(out: usize, queue: usize, kind: usize, text: usize, clients: &[usize], frame: usize, narrow: bool) -> bool {
    let original = CHAT_TRAMPOLINE.load(Ordering::SeqCst);
    if original == 0 || queue == 0 {
        return false;
    }
    let Some(stack) = super::spawn_probe::read_bytes(frame + 0x28, 0x28) else { return false };
    let (a5, a6, a7) = (super::spawn_probe::u32_at(&stack, 0), super::spawn_probe::u32_at(&stack, 8), super::spawn_probe::u32_at(&stack, 0x10));
    let (a8, optional) = (super::spawn_probe::u64_at(&stack, 0x18), super::spawn_probe::u64_at(&stack, 0x20) as usize);
    let Some(line) = message_text(text) else { return false };
    let call: EnqueueFn = unsafe { std::mem::transmute(original) };
    let answer = call(out as *mut u8, queue, kind, text, a5, a6, a7, a8, optional);
    let _ = answer;
    let status = super::spawn_probe::read_bytes(out, 1).map(|b| b[0]).unwrap_or(0xff);
    if status != 0 {
        return true;
    }
    if !narrow {
        return true;
    }
    let peers: Vec<usize> = clients.iter().map(|c| c & 0x3f).collect();
    if !narrow_last_entry_to(queue, &peers, line.as_bytes()) {
        super::spawner::log("chat: could not limit an answer to the players it is for; it was sent to everybody");
    }
    true
}

/// The newest entry of the server's chat queue is delivered to every connected player that has not marked it as
/// done. Marks it done for everybody but `target` and takes back the byte count the game added for them.
pub fn narrow_last_entry(queue: usize, target: usize, line: &[u8]) -> bool {
    narrow_last_entry_to(queue, &[target], line)
}

pub fn narrow_last_entry_to(queue: usize, targets: &[usize], line: &[u8]) -> bool {
    use super::spawn_probe::{read_bytes, u32_at, u64_at};
    let Some(tail) = read_bytes(queue + 0x1c8, 8).map(|b| u64_at(&b, 0) as usize) else { return false };
    if tail == 0 {
        return false;
    }
    let Some(entry) = read_bytes(tail, 0x40) else { return false };
    let length = u32_at(&entry, 8) as usize;
    if length != line.len() || read_bytes(tail + 0x40, length).as_deref() != Some(line) {
        return false;
    }
    let size = {
        let n = length as u64 + 0x17;
        match n & 3 {
            0 => n,
            r => n - r + 4,
        }
    } as u32;
    let Some(peers) = read_bytes(queue + 0x0c, 32 * 12) else { return false };
    let mut mask = u32_at(&entry, 0x20);
    for peer in 0..32usize {
        if targets.contains(&peer) || mask >> peer & 1 != 0 {
            continue;
        }
        if peers[peer * 12] != 0 {
            let at = queue + 0x14 + peer * 12;
            let Some(count) = read_bytes(at, 4).map(|b| u32_at(&b, 0)) else { return false };
            if !super::spawner::write_bytes(at, &count.saturating_sub(size).to_le_bytes()) {
                return false;
            }
        }
        mask |= 1 << peer;
    }
    super::spawner::write_bytes(tail + 0x20, &mask.to_le_bytes())
}

/// Several lines for the player who asked, each queued separately and shown to nobody else.
fn send_lines_to_asker(out: usize, queue: usize, kind: usize, text: usize, client: usize, frame: usize, lines: &[String], open: bool) -> usize {
    let mut sent = 0;
    for line in lines {
        if !replace_text(text, line) || !send_to_peers(out, queue, kind, text, &[client], frame, !open) {
            break;
        }
        sent += 1;
    }
    sent
}

/// The panel that lives in this very game (the one hosting the world) has no network between it and the answer:
/// its lines are read here instead of being queued in the chat. A dedicated server (it has a chat key, panels
/// talk to it from other computers) never has such a panel: its first player is a remote one.
fn is_local_panel(client: usize) -> bool {
    !super::spawner::is_remote_client() && super::probe::inject_on() && LIVE.chat_key().is_empty() && client & 0x3f == 0
}

fn who_is(entry: &super::table::Entry) -> String {
    format!("{}|{}", entry.name.as_deref().unwrap_or(""), entry.id.as_deref().unwrap_or(""))
}

/// After a shared change: a short line with the new fingerprints goes to the other panels that are connected
/// (and to nobody else). Returns true when it was queued; the caller must put its own text back.
fn tell_other_panels(out: usize, queue: usize, kind: usize, text: usize, author: usize, frame: usize) -> bool {
    if !super::shared::take_change() {
        return false;
    }
    let present: Vec<(u32, String)> = super::table::entries().iter().map(|e| (e.client, who_is(e))).collect();
    let mut targets: Vec<usize> = super::shared::targets(std::time::Instant::now(), &present, author as u32).into_iter().map(|c| c as usize).collect();
    let Ok(dir) = std::env::current_dir() else { return false };
    if let Some(at) = targets.iter().position(|c| is_local_panel(*c)) {
        targets.remove(at);
        accept_reply(&super::shared::rev_line(&dir, 0));
    }
    if targets.is_empty() {
        return false;
    }
    if !replace_text(text, &super::shared::rev_line(&dir, 0)) {
        return false;
    }
    let sent = send_to_peers(out, queue, kind, text, &targets, frame, true);
    super::spawner::log(&format!("shared state: {} panel(s) told that something changed ({})", targets.len(), if sent { "sent" } else { "not sent" }));
    sent
}


extern "win64" fn hook_chat(out: usize, context: usize, kind: usize, text: usize, client: usize, frame: usize) -> u8 {
    if REPLY_HOOK.load(Ordering::Relaxed) && super::spawner::is_remote_client() && take_server_reply(text) {
        return 1;
    }
    let key = LIVE.chat_key();
    if super::probe::on() {
        let head = super::spawn_probe::read_bytes(text, 16).unwrap_or_default();
        let length = super::spawn_probe::u64_at(&head, 8) as usize;
        let pointer = super::spawn_probe::u64_at(&head, 0) as usize;
        let ours = !key.is_empty()
            && length > 0
            && length <= 0x400
            && super::spawn_probe::read_bytes(pointer, length).is_some_and(|b| super::chat::has_prefix(&String::from_utf8_lossy(&b)));
        super::probe::chat(context, kind, length, ours, client);
    }
    if key.is_empty() {
        return 0;
    }
    let Some(head) = super::spawn_probe::read_bytes(text, 16) else { return 0 };
    let (pointer, length) = (super::spawn_probe::u64_at(&head, 0) as usize, super::spawn_probe::u64_at(&head, 8) as usize);
    if length == 0 || length > 0x400 {
        return 0;
    }
    let Some(bytes) = super::spawn_probe::read_bytes(pointer, length) else { return 0 };
    let message = String::from_utf8_lossy(&bytes);
    if super::invites::is_command(&message) {
        let verdict = team_command(text, &message, client as u32);
        if verdict == 0 && super::shared::peek_change() {
            let line = message_text(text);
            if tell_other_panels(out, context, kind, text, client, frame) {
                if let Some(line) = line {
                    replace_text(text, &line);
                }
            }
        }
        return verdict;
    }
    if !super::chat::has_prefix(&message) {
        super::chat::note_other(&message, client as u32);
        return 0;
    }
    if LIVE.chat_admin_only() {
        let table = super::table::entries();
        match table.iter().find(|e| e.client == client as u32) {
            Some(entry) if super::roster::is_admin(entry.permissions) => {}
            Some(entry) => {
                super::spawner::log(&format!("chat: client {} is not an admin (permissions {:#04x}); command ignored", client as u32, entry.permissions));
                return 1;
            }
            None => {
                super::spawner::log(&format!("chat: client {} not found in the player table; command ignored", client as u32));
                return 1;
            }
        }
    }
    let Ok(dir) = std::env::current_dir() else { return 0 };
    if super::chat::authenticated(&message, &key) && !super::shared::seen_recently(client as u32, std::time::Instant::now()) {
        if let Some(entry) = super::table::entries().iter().find(|e| e.client == client as u32) {
            super::shared::remember_panel(client as u32, &who_is(entry), std::time::Instant::now());
        }
    }
    match super::chat::handle(&message, &key, client as u32, &dir) {
        super::chat::Verdict::Pass => 0,
        super::chat::Verdict::Swallowed(line) => {
            super::spawner::log(&line);
            tell_other_panels(out, context, kind, text, client, frame);
            1
        }
        super::chat::Verdict::Lines(lines) if is_local_panel(client) => {
            for line in &lines {
                accept_reply(line);
            }
            1
        }
        super::chat::Verdict::Lines(lines) => {
            let open = super::chat::take_open();
            let sent = send_lines_to_asker(out, context, kind, text, client, frame, &lines, open);
            if sent != lines.len() {
                super::spawner::log(&format!("chat: client {} asked for shared state but only {sent} of {} line(s) could be sent", client as u32, lines.len()));
            }
            1
        }
        super::chat::Verdict::Teams(seq) => {
            let line = super::spawner::teams_line(seq);
            if is_local_panel(client) {
                accept_reply(&line);
                return 1;
            }
            let open = super::chat::take_open();
            let sent = replace_text(text, &line);
            super::spawner::log(&format!("chat: client {} asked for the teams ({}{}): {line}", client as u32, if sent { "sent" } else { "could not be sent" }, if open { ", to everybody: the asker could not receive it privately" } else { "" }));
            if sent && !open && send_to_asker_only(out, context, kind, text, client, frame) { 1 } else if sent { 0 } else { 1 }
        }
        super::chat::Verdict::Roster(seq) => {
            let line = super::spawner::roster_line(seq, client as u32);
            if is_local_panel(client) {
                accept_reply(&line);
                return 1;
            }
            let open = super::chat::take_open();
            let sent = replace_text(text, &line);
            super::spawner::log(&format!("chat: client {} asked for the player list ({}{}): {line}", client as u32, if sent { "sent" } else { "could not be sent" }, if open { ", to everybody: the asker could not receive it privately" } else { "" }));
            if sent && !open && send_to_asker_only(out, context, kind, text, client, frame) { 1 } else if sent { 0 } else { 1 }
        }
    }
}

// Entry of the game's "enqueue chat message". When the message is one of ours it is
// dropped and the caller gets the same "ok" result a delivered message would give.
#[unsafe(naked)]
unsafe extern "win64" fn stub_chat() {
    core::arch::naked_asm!(
        "push rcx",
        "push rdx",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push rax",
        "sub rsp, 0x90",
        "movups [rsp + 0x30], xmm0",
        "movups [rsp + 0x40], xmm1",
        "movups [rsp + 0x50], xmm2",
        "movups [rsp + 0x60], xmm3",
        "movups [rsp + 0x70], xmm4",
        "movups [rsp + 0x80], xmm5",
        "mov eax, dword ptr [rsp + 0x100]",
        "mov dword ptr [rsp + 0x20], eax",
        "lea rax, [rsp + 0xc8]",
        "mov qword ptr [rsp + 0x28], rax",
        "call {f}",
        "movzx eax, al",
        "mov dword ptr [rsp + 0x28], eax",
        "movups xmm0, [rsp + 0x30]",
        "movups xmm1, [rsp + 0x40]",
        "movups xmm2, [rsp + 0x50]",
        "movups xmm3, [rsp + 0x60]",
        "movups xmm4, [rsp + 0x70]",
        "movups xmm5, [rsp + 0x80]",
        "mov eax, dword ptr [rsp + 0x28]",
        "test eax, eax",
        "lea rsp, [rsp + 0x90]",
        "pop rax",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdx",
        "pop rcx",
        "jnz 2f",
        "jmp qword ptr [rip + {t}]",
        "2:",
        "mov byte ptr [rcx], 0",
        "mov dword ptr [rcx + 4], 0",
        "mov rax, rcx",
        "ret",
        f = sym hook_chat,
        t = sym CHAT_TRAMPOLINE,
    );
}

// Entry of the server's per-tick chat pump: remembers the player table (rcx), lets the M182 VFX
// module create its queued effects on this server tick, and goes on.
#[unsafe(naked)]
unsafe extern "win64" fn stub_pump() {
    core::arch::naked_asm!(
        "mov qword ptr [rip + {object}], rcx",
        "push rcx",
        "push rdx",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push rax",
        "sub rsp, 0x90",
        "movups [rsp + 0x30], xmm0",
        "movups [rsp + 0x40], xmm1",
        "movups [rsp + 0x50], xmm2",
        "movups [rsp + 0x60], xmm3",
        "movups [rsp + 0x70], xmm4",
        "movups [rsp + 0x80], xmm5",
        "call {f}",
        "movups xmm0, [rsp + 0x30]",
        "movups xmm1, [rsp + 0x40]",
        "movups xmm2, [rsp + 0x50]",
        "movups xmm3, [rsp + 0x60]",
        "movups xmm4, [rsp + 0x70]",
        "movups xmm5, [rsp + 0x80]",
        "lea rsp, [rsp + 0x90]",
        "pop rax",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdx",
        "pop rcx",
        "jmp qword ptr [rip + {t}]",
        object = sym PLAYERS_OBJECT,
        f = sym hook_pump,
        t = sym PUMP_TRAMPOLINE,
    );
}

extern "win64" fn hook_pump() {
    super::probe::hit(super::probe::PUMP);
    super::spawner::tick();
    super::vfx::server_tick();
    super::veil_beam::tick();
}

fn install_pump(sections: &[&'static [u8]]) {
    let target = match find_unique(sections, &signatures::PLAYERS_PUMP) {
        Ok(addr) => addr,
        Err(count) => {
            super::spawner::log(&format!("player table: pump signature matched {count} times; names and base permissions off"));
            return;
        }
    };
    let page = unsafe { VirtualAlloc(std::ptr::null_mut(), 0x1000, MEM_COMMIT_RESERVE, PAGE_EXECUTE_READWRITE) } as *mut u8;
    if page.is_null() {
        super::spawner::log("player table: trampoline allocation failed");
        return;
    }
    let displaced = signatures::PLAYERS_PUMP_DISPLACED;
    unsafe {
        std::ptr::copy_nonoverlapping(target, page, displaced);
        let tail = page.add(displaced);
        tail.write(0xFF);
        tail.add(1).write(0x25);
        tail.add(2).cast::<u32>().write_unaligned(0);
        tail.add(6).cast::<u64>().write_unaligned(target.add(displaced) as u64);
    }
    PUMP_TRAMPOLINE.store(page as usize, Ordering::SeqCst);
    match unsafe { write_jump(target, stub_pump as usize) } {
        Ok(()) => super::spawner::log("player table hooked"),
        Err(code) => {
            PUMP_TRAMPOLINE.store(0, Ordering::SeqCst);
            super::spawner::log(&format!("player table: VirtualProtect failed ({code})"));
        }
    }
}

fn install_permissions(sections: &[&'static [u8]]) {
    let target = match find_unique(sections, &signatures::PLAYER_PERMISSIONS) {
        Ok(addr) => addr,
        Err(count) => {
            super::spawner::log(&format!("permissions: lookup signature matched {count} times; base visitors fall back to the player table"));
            return;
        }
    };
    match unsafe { write_jump(target.add(signatures::PLAYER_PERMISSIONS_AT), stub_permissions as usize) } {
        Ok(()) => {
            PERMISSIONS_HOOKED.store(true, Ordering::SeqCst);
            super::spawner::log("permissions hooked");
        }
        Err(code) => super::spawner::log(&format!("permissions: VirtualProtect failed ({code})")),
    }
}

pub(super) static PAY_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static PAY_LOGGED: AtomicU64 = AtomicU64::new(0);

fn cost_entries(costs: usize) -> Vec<(u32, i32)> {
    let Some(head) = super::spawn_probe::read_bytes(costs, 16) else { return Vec::new() };
    let data = super::spawn_probe::u64_at(&head, 0) as usize;
    let count = (super::spawn_probe::u64_at(&head, 8) as usize).min(64);
    if data == 0 || count == 0 {
        return Vec::new();
    }
    let Some(raw) = super::spawn_probe::read_bytes(data, count * 24) else { return Vec::new() };
    (0..raw.len() / 24)
        .map(|i| {
            let e = &raw[i * 24..i * 24 + 24];
            (u32::from_le_bytes(e[16..20].try_into().unwrap()), i32::from_le_bytes(e[20..24].try_into().unwrap()))
        })
        .collect()
}

#[unsafe(naked)]
unsafe extern "win64" fn stub_pay_costs() {
    core::arch::naked_asm!(
        "sub rsp, 0x38",
        "mov r11, [rsp + 0x38]",
        "mov [rsp + 0x20], r11",
        "call {f}",
        "add rsp, 0x38",
        "ret",
        f = sym hook_pay_costs,
    );
}

const GATHER_BEFORE: [u8; 13] = [0x4c, 0x89, 0x65, 0xb0, 0x48, 0x8d, 0x4d, 0x80, 0x44, 0x89, 0x65, 0xb8, 0xe8];
const GATHER_AFTER: [u8; 17] = [0x84, 0xc0, 0x0f, 0x84, 0xce, 0x00, 0x00, 0x00, 0x45, 0x84, 0xed, 0x0f, 0x84, 0xd1, 0x00, 0x00, 0x00];

static GATHER_SITES: std::sync::Mutex<Vec<(usize, bool)>> = std::sync::Mutex::new(Vec::new());

/// True when the payment was asked by the code that hands a mined resource over (the same instructions in the
/// game and in the dedicated server, at different addresses): those payments must reach the game untouched.
fn is_gather_site(caller: usize) -> bool {
    if let Ok(seen) = GATHER_SITES.try_lock() {
        if let Some((_, known)) = seen.iter().find(|(at, _)| *at == caller) {
            return *known;
        }
    }
    let matches = (|| {
        let before = super::spawn_probe::read_bytes(caller.checked_sub(GATHER_BEFORE.len() + 4)?, GATHER_BEFORE.len())?;
        let after = super::spawn_probe::read_bytes(caller, GATHER_AFTER.len())?;
        Some(before == GATHER_BEFORE && after == GATHER_AFTER)
    })()
    .unwrap_or(false);
    if let Ok(mut seen) = GATHER_SITES.try_lock() {
        if seen.len() < 64 {
            seen.push((caller, matches));
        }
    }
    matches
}

static PAY_CALLERS: std::sync::Mutex<Vec<(usize, u8)>> = std::sync::Mutex::new(Vec::new());

fn pay_caller_is_new(caller: usize, pay: u8) -> bool {
    let Ok(mut seen) = PAY_CALLERS.try_lock() else { return false };
    if seen.contains(&(caller, pay)) || seen.len() >= 64 {
        return false;
    }
    seen.push((caller, pay));
    true
}

extern "win64" fn hook_pay_costs(context: usize, costs: usize, pay: u8, _r9: usize, caller: usize) -> u8 {
    super::probe::hit(super::probe::PAY);
    if LIVE.free_building() {
        if let Some(head) = super::spawn_probe::read_bytes(context, 16) {
            if head[0] == 0 {
                let transaction = super::spawn_probe::u64_at(&head, 8) as usize;
                if let Some(raw) = super::spawn_probe::read_bytes(transaction + 0xe0, 4) {
                    let owner = super::spawn_probe::u32_at(&raw, 0);
                    let admin = super::table::entries().iter().any(|e| e.entity() == owner && super::roster::is_admin(e.permissions));
                    let entries = if admin { cost_entries(costs) } else { Vec::new() };
                    let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
                    if pay_caller_is_new(caller, pay) {
                        super::spawner::log(&format!(
                            "free building: new payment caller +{:#x} pay={pay} mode={} owner={owner} admin={admin} entries {:?}",
                            caller.wrapping_sub(base),
                            head[0],
                            &entries[..entries.len().min(6)]
                        ));
                    }
                    if PAY_LOGGED.fetch_add(1, Ordering::Relaxed) < 4 {
                        let known: Vec<u32> = super::table::entries().iter().map(|e| e.entity()).collect();
                        super::spawner::log(&format!(
                            "free building: payment by owner {owner} (pay={pay}) -> {} (players online {known:?}, entries {:?})",
                            if admin { "free, administrator" } else { "charged" },
                            &entries[..entries.len().min(6)]
                        ));
                    }
                    if admin && !is_gather_site(caller) {
                        return 1;
                    }
                }
            }
        }
    }
    let original = PAY_TRAMPOLINE.load(Ordering::SeqCst);
    if original == 0 {
        return 0;
    }
    let call: extern "win64" fn(usize, usize, u8) -> u8 = unsafe { std::mem::transmute(original) };
    call(context, costs, pay)
}

const LOCK_SLOTS: usize = 64;
#[allow(clippy::declare_interior_mutable_const)]
const LOCK_ZERO: AtomicUsize = AtomicUsize::new(0);
static RESOURCE_LOCK: [AtomicUsize; LOCK_SLOTS] = [LOCK_ZERO; LOCK_SLOTS];
static RESOURCE_TRAMPOLINE: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];
static RESOURCE_LOGGED: AtomicU64 = AtomicU64::new(0);

/// Addresses of the stamina and mana records that must not be spent. The game passes the record itself
/// as the first argument, so the comparison is a plain address match.
pub fn set_resource_lock(records: &[usize]) {
    for (n, slot) in RESOURCE_LOCK.iter().enumerate() {
        slot.store(records.get(n).copied().unwrap_or(0), Ordering::Relaxed);
    }
}

pub fn protected_records() -> Vec<usize> {
    RESOURCE_LOCK.iter().map(|s| s.load(Ordering::Relaxed)).filter(|a| *a != 0).collect()
}

static HEALTH_BLOCKED: AtomicU64 = AtomicU64::new(0);
const HEALTH_PROOF: u64 = 3;

/// True once the spend hook has itself stopped a few health losses of a protected player: from then on the
/// power loop does not have to top the health up every few milliseconds.
pub fn health_hook_proven() -> bool {
    HEALTH_BLOCKED.load(Ordering::Relaxed) >= HEALTH_PROOF
}

/// Which losses the hook stops for a locked record: stamina and mana (the power) and health (god mode).
pub(super) fn lock_applies(kind: u32, delta: i32) -> bool {
    delta < 0
        && (kind == super::spawner::STAMINA_SIGNATURE
            || kind == super::spawner::MANA_SIGNATURE
            || kind == super::spawner::HEALTH_SIGNATURE)
}

extern "win64" fn hook_resource_delta<const N: usize>(chunk: usize, entity: usize, delta: i32, kind: u32) -> usize {
    super::probe::hit(super::probe::RESOURCE);
    if lock_applies(kind, delta) {
        let locked = chunk != 0 && RESOURCE_LOCK.iter().any(|slot| slot.load(Ordering::Relaxed) == chunk);
        if locked && kind == super::spawner::HEALTH_SIGNATURE {
            HEALTH_BLOCKED.fetch_add(1, Ordering::Relaxed);
            return 0;
        }
        if !locked && kind != super::spawner::HEALTH_SIGNATURE && RESOURCE_LOGGED.fetch_add(1, Ordering::Relaxed) < 24 {
            let known: Vec<usize> = RESOURCE_LOCK.iter().map(|s| s.load(Ordering::Relaxed)).filter(|a| *a != 0).collect();
            super::spawner::log(&format!(
                "resource lock: copy {N} record {chunk:#x} type {kind:#x} delta {delta} was spent (locked records {known:x?})"
            ));
        }
        if locked {
            return 0;
        }
        if kind == super::spawner::HEALTH_SIGNATURE {
            super::spawner::damage_seen(chunk, entity, delta);
        }
    }
    let original = RESOURCE_TRAMPOLINE[N].load(Ordering::SeqCst);
    let call: extern "win64" fn(usize, usize, i32, u32) -> usize = unsafe { std::mem::transmute(original) };
    call(chunk, entity, delta, kind)
}

fn find_all(sections: &[&'static [u8]], signature: &[u8]) -> Vec<*mut u8> {
    let mut hits = Vec::new();
    for section in sections {
        if section.len() < signature.len() {
            continue;
        }
        let last_start = section.len() - signature.len();
        let mut i = 0usize;
        while i <= last_start {
            if section[i] == signature[0] && &section[i..i + signature.len()] == signature {
                hits.push(unsafe { section.as_ptr().add(i) } as *mut u8);
                i += signature.len();
            } else {
                i += 1;
            }
        }
    }
    hits
}

fn install_resource_lock(sections: &[&'static [u8]]) {
    let hits = find_all(sections, &signatures::RESOURCE_DELTA);
    if hits.len() != 2 {
        super::spawner::log(&format!("resource lock off: delta function matched {} times", hits.len()));
        return;
    }
    let hooks = [hook_resource_delta::<0> as usize, hook_resource_delta::<1> as usize];
    for (n, target) in hits.into_iter().enumerate() {
        let page = unsafe { VirtualAlloc(std::ptr::null_mut(), 0x1000, MEM_COMMIT_RESERVE, PAGE_EXECUTE_READWRITE) } as *mut u8;
        if page.is_null() {
            super::spawner::log("resource lock off: trampoline allocation failed");
            return;
        }
        let displaced = signatures::RESOURCE_DELTA_DISPLACED;
        unsafe {
            std::ptr::copy_nonoverlapping(target, page, displaced);
            let tail = page.add(displaced);
            tail.write(0xFF);
            tail.add(1).write(0x25);
            tail.add(2).cast::<u32>().write_unaligned(0);
            tail.add(6).cast::<u64>().write_unaligned(target.add(displaced) as u64);
        }
        RESOURCE_TRAMPOLINE[n].store(page as usize, Ordering::SeqCst);
        if unsafe { write_jump(target, hooks[n]) }.is_err() {
            RESOURCE_TRAMPOLINE[n].store(0, Ordering::SeqCst);
            super::spawner::log("resource lock off: VirtualProtect failed");
            return;
        }
    }
    super::spawner::log("resource lock ready: stamina and mana are not spent while the power is on");
}

fn install_pay_costs(sections: &[&'static [u8]]) {
    let target = match find_unique(sections, &signatures::PAY_COSTS) {
        Ok(addr) => addr,
        Err(count) => {
            super::spawner::log(&format!("free building off: payment function signature matched {count} times"));
            return;
        }
    };
    let page = unsafe { VirtualAlloc(std::ptr::null_mut(), 0x1000, MEM_COMMIT_RESERVE, PAGE_EXECUTE_READWRITE) } as *mut u8;
    if page.is_null() {
        super::spawner::log("free building off: trampoline allocation failed");
        return;
    }
    let displaced = signatures::PAY_COSTS_DISPLACED;
    unsafe {
        std::ptr::copy_nonoverlapping(target, page, displaced);
        let tail = page.add(displaced);
        tail.write(0xFF);
        tail.add(1).write(0x25);
        tail.add(2).cast::<u32>().write_unaligned(0);
        tail.add(6).cast::<u64>().write_unaligned(target.add(displaced) as u64);
    }
    PAY_TRAMPOLINE.store(page as usize, Ordering::SeqCst);
    match unsafe { write_jump(target, stub_pay_costs as usize) } {
        Ok(()) => super::spawner::log("free building ready: administrators build without spending materials"),
        Err(code) => {
            PAY_TRAMPOLINE.store(0, Ordering::SeqCst);
            super::spawner::log(&format!("free building off: VirtualProtect failed ({code})"));
        }
    }
}

pub(super) static ZONES_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static ZONES_LOGGED: AtomicU64 = AtomicU64::new(0);
static SETTER_LOGGED: AtomicU64 = AtomicU64::new(0);
static ZONES_AT: AtomicUsize = AtomicUsize::new(0);

fn zones_offset_from_setter(code: &[u8]) -> Option<usize> {
    let end = code.len().checked_sub(14)?;
    (0..=end).find_map(|i| {
        if code[i..i + 3] != [0x48, 0x8B, 0x87] || code[i + 7..i + 10] != [0x48, 0x8D, 0xB7] {
            return None;
        }
        let first = u32::from_le_bytes(code[i + 3..i + 7].try_into().ok()?) as usize;
        let second = u32::from_le_bytes(code[i + 10..i + 14].try_into().ok()?) as usize;
        (second >= 0x1000 && first == second + 0x10).then_some(second)
    })
}

fn add_everywhere_area(world: usize) {
    let at = ZONES_AT.load(Ordering::Relaxed);
    if at == 0 {
        return;
    }
    let array = world + at;
    let Some(head) = super::spawn_probe::read_bytes(array, 24) else { return };
    let (data, count, capacity) = (super::spawn_probe::u64_at(&head, 0) as usize, super::spawn_probe::u64_at(&head, 8), super::spawn_probe::u64_at(&head, 16));
    if data == 0 || count >= capacity || count > 4096 {
        if ZONES_LOGGED.fetch_add(1, Ordering::Relaxed) < 4 {
            super::spawner::log(&format!("build anywhere: no room for the extra area (count {count}, capacity {capacity})"));
        }
        return;
    }
    let mut entry = [0u8; 24];
    for (i, value) in [-1.0e6f32, -1.0e5, -1.0e6, 1.0e6, 1.0e5, 1.0e6].iter().enumerate() {
        entry[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    if count > 0 {
        if let Some(last) = super::spawn_probe::read_bytes(data + (count as usize - 1) * 24, 24) {
            if last == entry {
                return;
            }
        }
    }
    if super::spawner::write_bytes(data + count as usize * 24, &entry) && super::spawner::write_bytes(array + 8, &(count + 1).to_le_bytes()) {
        if ZONES_LOGGED.fetch_add(1, Ordering::Relaxed) < 4 {
            super::spawner::log(&format!("build anywhere: whole-map building area added ({} areas now)", count + 1));
        }
    }
}

static QUERY_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static QUERY_CALLS: AtomicU64 = AtomicU64::new(0);
static QUERY_LOGGED: AtomicU64 = AtomicU64::new(0);

extern "win64" fn note_query_world(world: usize) {
    super::probe::hit(super::probe::WORLD_QUERY);
    let calls = QUERY_CALLS.fetch_add(1, Ordering::Relaxed);
    if !LIVE.client_build_anywhere() || world == 0 {
        return;
    }
    if calls % 45 == 0 {
        if QUERY_LOGGED.fetch_add(1, Ordering::Relaxed) < 2 {
            let head = super::spawn_probe::read_bytes(world + ZONES_AT.load(Ordering::Relaxed), 24).unwrap_or_default();
            if head.len() == 24 {
                super::spawner::log(&format!(
                    "build anywhere: world {world:#x} has {} building areas (capacity {})",
                    super::spawn_probe::u64_at(&head, 8),
                    super::spawn_probe::u64_at(&head, 16)
                ));
            }
        }
        add_everywhere_area(world);
    }
}

#[unsafe(naked)]
unsafe extern "win64" fn stub_query() {
    core::arch::naked_asm!(
        "push rcx",
        "push rdx",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "sub rsp, 0x88",
        "movups [rsp + 0x20], xmm0",
        "movups [rsp + 0x30], xmm1",
        "movups [rsp + 0x40], xmm2",
        "movups [rsp + 0x50], xmm3",
        "movups [rsp + 0x60], xmm4",
        "movups [rsp + 0x70], xmm5",
        "mov rcx, rdx",
        "call {f}",
        "movups xmm0, [rsp + 0x20]",
        "movups xmm1, [rsp + 0x30]",
        "movups xmm2, [rsp + 0x40]",
        "movups xmm3, [rsp + 0x50]",
        "movups xmm4, [rsp + 0x60]",
        "movups xmm5, [rsp + 0x70]",
        "add rsp, 0x88",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdx",
        "pop rcx",
        "jmp qword ptr [rip + {t}]",
        f = sym note_query_world,
        t = sym QUERY_TRAMPOLINE,
    );
}

fn install_build_zones_query(sections: &[&'static [u8]]) {
    let target = match find_unique(sections, &signatures::BUILD_ZONES_QUERY) {
        Ok(addr) => addr,
        Err(count) => {
            super::spawner::log(&format!("build anywhere: building-area query matched {count} times"));
            return;
        }
    };
    let page = unsafe { VirtualAlloc(std::ptr::null_mut(), 0x1000, MEM_COMMIT_RESERVE, PAGE_EXECUTE_READWRITE) } as *mut u8;
    if page.is_null() {
        return;
    }
    let displaced = signatures::BUILD_ZONES_QUERY_DISPLACED;
    unsafe {
        std::ptr::copy_nonoverlapping(target, page, displaced);
        let tail = page.add(displaced);
        tail.write(0xFF);
        tail.add(1).write(0x25);
        tail.add(2).cast::<u32>().write_unaligned(0);
        tail.add(6).cast::<u64>().write_unaligned(target.add(displaced) as u64);
    }
    QUERY_TRAMPOLINE.store(page as usize, Ordering::SeqCst);
    match unsafe { write_jump(target, stub_query as usize) } {
        Ok(()) => super::spawner::log("build anywhere: area query watched"),
        Err(code) => {
            QUERY_TRAMPOLINE.store(0, Ordering::SeqCst);
            super::spawner::log(&format!("build anywhere: area query VirtualProtect failed ({code})"));
        }
    }
}

extern "win64" fn hook_build_zones_set(world: usize, zones: usize) {
    let original = ZONES_TRAMPOLINE.load(Ordering::SeqCst);
    if original == 0 {
        return;
    }
    let call: extern "win64" fn(usize, usize) = unsafe { std::mem::transmute(original) };
    call(world, zones);
    if SETTER_LOGGED.fetch_add(1, Ordering::Relaxed) < 2 {
        super::spawner::log(&format!("build anywhere: area setter called for world {world:#x}"));
    }
    if LIVE.client_build_anywhere() {
        add_everywhere_area(world);
    }
}

fn install_build_zones(sections: &[&'static [u8]]) {
    let client = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase())).is_some_and(|n| n == "enshrouded.exe");
    if !client {
        return;
    }
    let target = match find_unique(sections, &signatures::BUILD_ZONES_SET) {
        Ok(addr) => addr,
        Err(count) => {
            super::spawner::log(&format!("build anywhere off: building-area setter matched {count} times"));
            return;
        }
    };
    let code = unsafe { std::slice::from_raw_parts(target as *const u8, 0x120) };
    let Some(at) = zones_offset_from_setter(code) else {
        super::spawner::log("build anywhere off: this game version keeps its building areas somewhere unknown");
        return;
    };
    ZONES_AT.store(at, Ordering::SeqCst);
    let page = unsafe { VirtualAlloc(std::ptr::null_mut(), 0x1000, MEM_COMMIT_RESERVE, PAGE_EXECUTE_READWRITE) } as *mut u8;
    if page.is_null() {
        super::spawner::log("build anywhere off: trampoline allocation failed");
        return;
    }
    let displaced = signatures::BUILD_ZONES_SET_DISPLACED;
    unsafe {
        std::ptr::copy_nonoverlapping(target, page, displaced);
        let tail = page.add(displaced);
        tail.write(0xFF);
        tail.add(1).write(0x25);
        tail.add(2).cast::<u32>().write_unaligned(0);
        tail.add(6).cast::<u64>().write_unaligned(target.add(displaced) as u64);
    }
    install_build_zones_query(sections);
    ZONES_TRAMPOLINE.store(page as usize, Ordering::SeqCst);
    match unsafe { write_jump(target, hook_build_zones_set as usize) } {
        Ok(()) => super::spawner::log("build anywhere ready: the hammer works away from altars"),
        Err(code) => {
            ZONES_TRAMPOLINE.store(0, Ordering::SeqCst);
            super::spawner::log(&format!("build anywhere off: VirtualProtect failed ({code})"));
        }
    }
}

fn install_chat(sections: &[&'static [u8]]) {
    if LIVE.chat_key().is_empty() && !super::probe::on() && !super::probe::inject_on() {
        return;
    }
    let target = match find_unique(sections, &signatures::CHAT_ENQUEUE) {
        Ok(addr) => addr,
        Err(count) => {
            tracing::warn!(target: "m182::native", matches = count, "chat function signature not found exactly once; chat commands off");
            super::spawner::log(&format!("chat commands off: chat function signature matched {count} times"));
            return;
        }
    };

    let page = unsafe { VirtualAlloc(std::ptr::null_mut(), 0x1000, MEM_COMMIT_RESERVE, PAGE_EXECUTE_READWRITE) } as *mut u8;
    if page.is_null() {
        tracing::warn!(target: "m182::native", "chat trampoline allocation failed; chat commands off");
        return;
    }
    let displaced = signatures::CHAT_DISPLACED;
    unsafe {
        std::ptr::copy_nonoverlapping(target, page, displaced);
        let tail = page.add(displaced);
        tail.write(0xFF);
        tail.add(1).write(0x25);
        tail.add(2).cast::<u32>().write_unaligned(0);
        tail.add(6).cast::<u64>().write_unaligned(target.add(displaced) as u64);
    }
    CHAT_TRAMPOLINE.store(page as usize, Ordering::SeqCst);

    match unsafe { write_jump(target, stub_chat as usize) } {
        Ok(()) => {
            if super::probe::inject_on() && LIVE.chat_key().is_empty() {
                REPLY_HOOK.store(true, Ordering::SeqCst);
            }
            tracing::info!(target: "m182::native", address = format_args!("{:#x}", target as usize), "chat commands hooked");
            super::spawner::log("chat commands ready");
        }
        Err(code) => {
            CHAT_TRAMPOLINE.store(0, Ordering::SeqCst);
            tracing::error!(target: "m182::native", code, "VirtualProtect failed; chat commands off");
        }
    }
}


// ---------------------------------------------------------------- chat sending (chat_inject)

static FLUSH_T: AtomicUsize = AtomicUsize::new(0);

extern "win64" fn flush_hit(_id: usize, regs: *const usize) {
    // regs: rax, r11, r10, r9, r8, rdx, rcx, return address
    if !regs.is_null() {
        super::outbox::flush_seen(unsafe { *regs.add(6) });
    }
}

// Entry of the game's per-frame chat sender: lets us know the connection is alive and where its queue is.
#[unsafe(naked)]
unsafe extern "win64" fn stub_flush() {
    core::arch::naked_asm!(
        "push rcx",
        "push rdx",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push rax",
        "sub rsp, 0x90",
        "movups [rsp + 0x30], xmm0",
        "movups [rsp + 0x40], xmm1",
        "movups [rsp + 0x50], xmm2",
        "movups [rsp + 0x60], xmm3",
        "movups [rsp + 0x70], xmm4",
        "movups [rsp + 0x80], xmm5",
        "xor ecx, ecx",
        "lea rdx, [rsp + 0x90]",
        "call {f}",
        "movups xmm0, [rsp + 0x30]",
        "movups xmm1, [rsp + 0x40]",
        "movups xmm2, [rsp + 0x50]",
        "movups xmm3, [rsp + 0x60]",
        "movups xmm4, [rsp + 0x70]",
        "movups xmm5, [rsp + 0x80]",
        "lea rsp, [rsp + 0x90]",
        "pop rax",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdx",
        "pop rcx",
        "jmp qword ptr [rip + {t}]",
        f = sym flush_hit,
        t = sym FLUSH_T,
    );
}

/// Finds the game's "queue a chat line" function and watches its per-frame sender, so panel chat commands can be
/// queued from code instead of typed. Only done when `chat_inject` is on.
fn install_chat_out(sections: &[&'static [u8]]) {
    match find_unique(sections, &signatures::CHAT_SEND) {
        Ok(addr) => super::outbox::set_send_function(addr as usize),
        Err(count) => {
            super::spawner::log(&format!("chat out off: the game's chat function matched {count} times"));
            return;
        }
    }
    if super::probe::on() {
        // The probe already watches the sender and forwards it.
        return;
    }
    let target = match find_unique(sections, &signatures::WATCH_FLUSH) {
        Ok(addr) => addr,
        Err(count) => {
            super::spawner::log(&format!("chat out off: the game's chat sender matched {count} times"));
            super::outbox::set_send_function(0);
            return;
        }
    };
    let page = unsafe { VirtualAlloc(std::ptr::null_mut(), 0x1000, MEM_COMMIT_RESERVE, PAGE_EXECUTE_READWRITE) } as *mut u8;
    if page.is_null() {
        super::outbox::set_send_function(0);
        return;
    }
    let displaced = signatures::WATCH_FLUSH_DISPLACED;
    unsafe {
        std::ptr::copy_nonoverlapping(target, page, displaced);
        let tail = page.add(displaced);
        tail.write(0xFF);
        tail.add(1).write(0x25);
        tail.add(2).cast::<u32>().write_unaligned(0);
        tail.add(6).cast::<u64>().write_unaligned(target.add(displaced) as u64);
    }
    FLUSH_T.store(page as usize, Ordering::SeqCst);
    match unsafe { write_jump(target, stub_flush as usize) } {
        Ok(()) => super::spawner::log("chat out ready: panel commands are queued through the game's chat function"),
        Err(code) => {
            FLUSH_T.store(0, Ordering::SeqCst);
            super::outbox::set_send_function(0);
            super::spawner::log(&format!("chat out off: VirtualProtect failed ({code})"));
        }
    }
}

// ---------------------------------------------------------------- watch hooks (thread_probe only)

const WATCHED: usize = 4;
static WATCH_T0: AtomicUsize = AtomicUsize::new(0);
static WATCH_T1: AtomicUsize = AtomicUsize::new(0);
static WATCH_T2: AtomicUsize = AtomicUsize::new(0);
static WATCH_T3: AtomicUsize = AtomicUsize::new(0);

extern "win64" fn watch_hit(id: usize, regs: *const usize) {
    super::watch::hit(id, regs);
}

macro_rules! watch_stub {
    ($name:ident, $id:expr, $slot:ident) => {
        #[unsafe(naked)]
        unsafe extern "win64" fn $name() {
            core::arch::naked_asm!(
                "push rcx",
                "push rdx",
                "push r8",
                "push r9",
                "push r10",
                "push r11",
                "push rax",
                "sub rsp, 0x90",
                "movups [rsp + 0x30], xmm0",
                "movups [rsp + 0x40], xmm1",
                "movups [rsp + 0x50], xmm2",
                "movups [rsp + 0x60], xmm3",
                "movups [rsp + 0x70], xmm4",
                "movups [rsp + 0x80], xmm5",
                "mov ecx, {id}",
                "lea rdx, [rsp + 0x90]",
                "call {f}",
                "movups xmm0, [rsp + 0x30]",
                "movups xmm1, [rsp + 0x40]",
                "movups xmm2, [rsp + 0x50]",
                "movups xmm3, [rsp + 0x60]",
                "movups xmm4, [rsp + 0x70]",
                "movups xmm5, [rsp + 0x80]",
                "lea rsp, [rsp + 0x90]",
                "pop rax",
                "pop r11",
                "pop r10",
                "pop r9",
                "pop r8",
                "pop rdx",
                "pop rcx",
                "jmp qword ptr [rip + {t}]",
                id = const $id,
                f = sym watch_hit,
                t = sym $slot,
            );
        }
    };
}

watch_stub!(stub_watch_0, 0, WATCH_T0);
watch_stub!(stub_watch_1, 1, WATCH_T1);
watch_stub!(stub_watch_2, 2, WATCH_T2);
watch_stub!(stub_watch_3, 3, WATCH_T3);

fn install_watch(sections: &[&'static [u8]]) {
    let wanted: [(&str, &[u8], usize, &AtomicUsize, usize); WATCHED] = [
        ("encode", &signatures::WATCH_SERIALIZE, signatures::WATCH_SERIALIZE_DISPLACED, &WATCH_T0, stub_watch_0 as usize),
        ("decode", &signatures::WATCH_DESERIALIZE, signatures::WATCH_DESERIALIZE_DISPLACED, &WATCH_T1, stub_watch_1 as usize),
        ("chat flush", &signatures::WATCH_FLUSH, signatures::WATCH_FLUSH_DISPLACED, &WATCH_T2, stub_watch_2 as usize),
        ("chat receive", &signatures::WATCH_CLIENT_IN, signatures::WATCH_CLIENT_IN_DISPLACED, &WATCH_T3, stub_watch_3 as usize),
    ];
    for (name, signature, displaced, slot, stub) in wanted {
        let target = match find_unique(sections, signature) {
            Ok(addr) => addr,
            Err(count) => {
                super::spawner::log(&format!("probe: {name} function matched {count} times; not watched"));
                continue;
            }
        };
        let page = unsafe { VirtualAlloc(std::ptr::null_mut(), 0x1000, MEM_COMMIT_RESERVE, PAGE_EXECUTE_READWRITE) } as *mut u8;
        if page.is_null() {
            continue;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(target, page, displaced);
            let tail = page.add(displaced);
            tail.write(0xFF);
            tail.add(1).write(0x25);
            tail.add(2).cast::<u32>().write_unaligned(0);
            tail.add(6).cast::<u64>().write_unaligned(target.add(displaced) as u64);
        }
        slot.store(page as usize, Ordering::SeqCst);
        match unsafe { write_jump(target, stub) } {
            Ok(()) => super::spawner::log(&format!("probe: watching {name}")),
            Err(code) => {
                slot.store(0, Ordering::SeqCst);
                super::spawner::log(&format!("probe: {name}: VirtualProtect failed ({code})"));
            }
        }
    }
}

// ---------------------------------------------------------------- patching

struct Target {
    name: &'static str,
    signature: &'static [u8],
    hook: unsafe extern "win64" fn(),
}

pub fn install() -> Outcome {
    if INSTALLED.load(Ordering::SeqCst) {
        return Outcome::Done;
    }

    let Some(sections) = executable_sections() else {
        return Outcome::Retry;
    };

    let vfx = super::vfx::wanted();
    let pillar = super::pillar::wanted();
    if super::config::LIVE.spawn_probe() || super::config::LIVE.spawner() || vfx || pillar {
        super::spawn_probe::install(&sections);
    }

    super::guard::install(&sections);
    if super::config::LIVE.client_build_anywhere() {
        install_build_zones(&sections);
    }
    install_chat(&sections);
    if super::probe::on() {
        install_watch(&sections);
    }
    if super::probe::inject_on() && LIVE.chat_key().is_empty() {
        install_chat_out(&sections);
    }
    if super::config::LIVE.spawner() || vfx || pillar {
        install_pump(&sections);
    }
    if super::config::LIVE.spawner() {
        install_permissions(&sections);
        install_resource_lock(&sections);
        if super::config::LIVE.free_building() {
            install_pay_costs(&sections);
        }
    }

    let targets = [
        Target { name: "isEnemy(values)", signature: &signatures::IS_ENEMY_VALUES, hook: stub_enemy_values },
        Target { name: "isEnemy(pointers)", signature: &signatures::IS_ENEMY_PTRS, hook: stub_enemy_ptrs },
        Target { name: "isFriend(values)", signature: &signatures::IS_FRIEND_VALUES, hook: stub_friend_values },
        Target { name: "isFriend(options)", signature: &signatures::IS_FRIEND_OPTS, hook: stub_friend_opts },
    ];

    // Resolve everything first; only patch when the complete set is present so we never
    // end up with a half-hooked, inconsistent faction logic.
    let mut found = Vec::with_capacity(targets.len());
    for target in &targets {
        match find_unique(&sections, target.signature) {
            Ok(addr) => found.push((target, addr)),
            Err(count) => {
                tracing::warn!(target: "m182::native", function = target.name, matches = count, "signature not found exactly once; game version changed? PvP patch not applied");
                INSTALLED.store(true, Ordering::SeqCst);
                return Outcome::Done;
            }
        }
    }

    for (target, addr) in &found {
        match unsafe { write_jump(*addr, target.hook as usize) } {
            Ok(()) => {
                tracing::info!(target: "m182::native", function = target.name, address = format_args!("{:#x}", *addr as usize), "hooked");
            }
            Err(code) => {
                tracing::error!(target: "m182::native", function = target.name, code, "VirtualProtect failed");
                INSTALLED.store(true, Ordering::SeqCst);
                return Outcome::Done;
            }
        }
    }

    INSTALLED.store(true, Ordering::SeqCst);
    tracing::info!(target: "m182::native", pvp = LIVE.pvp(), "faction hooks installed");
    Outcome::Done
}

pub(super) unsafe fn write_jump(at: *mut u8, destination: usize) -> Result<(), u32> {
    let mut jump = [0u8; JUMP_LEN];
    // jmp qword ptr [rip + 0]
    jump[..6].copy_from_slice(&[0xFF, 0x25, 0, 0, 0, 0]);
    jump[6..].copy_from_slice(&(destination as u64).to_le_bytes());

    let mut old = 0u32;
    if unsafe { VirtualProtect(at.cast(), JUMP_LEN, PAGE_EXECUTE_READWRITE, &mut old) } == 0 {
        return Err(1);
    }
    unsafe { std::ptr::copy_nonoverlapping(jump.as_ptr(), at, JUMP_LEN) };
    let mut ignored = 0u32;
    unsafe {
        VirtualProtect(at.cast(), JUMP_LEN, old, &mut ignored);
        FlushInstructionCache(GetCurrentProcess(), at.cast(), JUMP_LEN);
    }
    Ok(())
}

pub(super) fn find_unique(sections: &[&'static [u8]], signature: &[u8]) -> Result<*mut u8, usize> {
    let mut hit: Option<*mut u8> = None;
    let mut count = 0usize;
    let first = signature[0];

    for section in sections {
        if section.len() < signature.len() {
            continue;
        }
        let last_start = section.len() - signature.len();
        let mut i = 0usize;
        while i <= last_start {
            if section[i] == first && &section[i..i + signature.len()] == signature {
                count += 1;
                hit = Some(unsafe { section.as_ptr().add(i) } as *mut u8);
                i += signature.len();
            } else {
                i += 1;
            }
        }
    }

    match (count, hit) {
        (1, Some(addr)) => Ok(addr),
        _ => Err(count),
    }
}

/// Executable sections of the main module, or `None` if the header looks wrong.
fn executable_sections() -> Option<Vec<&'static [u8]>> {
    unsafe {
        let base = GetModuleHandleW(std::ptr::null()) as *const u8;
        if base.is_null() {
            return None;
        }
        if base.cast::<u16>().read_unaligned() != 0x5A4D {
            return None;
        }
        let nt = base.add(base.add(0x3C).cast::<u32>().read_unaligned() as usize);
        if nt.cast::<u32>().read_unaligned() != 0x0000_4550 {
            return None;
        }
        let section_count = nt.add(6).cast::<u16>().read_unaligned() as usize;
        let optional_size = nt.add(20).cast::<u16>().read_unaligned() as usize;
        let table = nt.add(24 + optional_size);

        let mut sections = Vec::new();
        for i in 0..section_count.min(96) {
            let header = table.add(i * 40);
            let virtual_size = header.add(8).cast::<u32>().read_unaligned() as usize;
            let virtual_address = header.add(12).cast::<u32>().read_unaligned() as usize;
            let characteristics = header.add(36).cast::<u32>().read_unaligned();
            if characteristics & SCN_MEM_EXECUTE != 0 && virtual_size > 0 {
                sections.push(std::slice::from_raw_parts(base.add(virtual_address), virtual_size));
            }
        }
        if sections.is_empty() { None } else { Some(sections) }
    }
}

#[cfg(test)]
mod zones_offset_tests {
    use super::zones_offset_from_setter;

    #[test]
    fn offset_is_read_from_the_setter_code() {
        let mut code = vec![0x90u8; 20];
        code.extend_from_slice(&[0x48, 0x8B, 0x87, 0x78, 0x8A, 0x5F, 0x01, 0x48, 0x8D, 0xB7, 0x68, 0x8A, 0x5F, 0x01]);
        code.extend_from_slice(&[0x90; 8]);
        assert_eq!(zones_offset_from_setter(&code), Some(0x15F8A68));
        assert_eq!(zones_offset_from_setter(&[0x90; 64]), None);
    }
}

#[cfg(test)]
mod gather_site_tests {
    use super::*;

    #[test]
    fn the_code_that_hands_over_mined_resources_is_recognised_wherever_it_sits() {
        let mut code = Vec::new();
        code.extend_from_slice(&GATHER_BEFORE);
        code.extend_from_slice(&[0x11, 0x22, 0x33, 0x44]);
        code.extend_from_slice(&GATHER_AFTER);
        let caller = code.as_ptr() as usize + GATHER_BEFORE.len() + 4;
        assert!(is_gather_site(caller));
        let mut other = code.clone();
        other[0] = 0x4d;
        let caller = other.as_ptr() as usize + GATHER_BEFORE.len() + 4;
        assert!(!is_gather_site(caller));
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;
    use super::super::spawner::{HEALTH_SIGNATURE, MANA_SIGNATURE, STAMINA_SIGNATURE};

    #[test]
    fn only_losses_of_the_three_locked_resources_are_stopped() {
        assert!(lock_applies(HEALTH_SIGNATURE, -5));
        assert!(lock_applies(STAMINA_SIGNATURE, -1));
        assert!(lock_applies(MANA_SIGNATURE, -1));
        assert!(!lock_applies(HEALTH_SIGNATURE, 5));
        assert!(!lock_applies(HEALTH_SIGNATURE, 0));
        assert!(!lock_applies(0x1234_5678, -5));
    }

    #[test]
    fn the_health_hook_is_trusted_only_after_it_has_worked_a_few_times() {
        assert!(!health_hook_proven() || HEALTH_BLOCKED.load(Ordering::Relaxed) >= HEALTH_PROOF);
        HEALTH_BLOCKED.store(HEALTH_PROOF - 1, Ordering::Relaxed);
        assert!(!health_hook_proven());
        HEALTH_BLOCKED.store(HEALTH_PROOF, Ordering::Relaxed);
        assert!(health_hook_proven());
        HEALTH_BLOCKED.store(0, Ordering::Relaxed);
    }
}

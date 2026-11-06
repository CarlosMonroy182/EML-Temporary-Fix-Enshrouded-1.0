use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

#[cfg(windows)]
fn thread_id() -> u32 {
    unsafe { GetCurrentThreadId() }
}

#[cfg(not(windows))]
fn thread_id() -> u32 {
    1
}

#[cfg(windows)]
fn note(line: &str) {
    super::spawner::log(line);
}

#[cfg(not(windows))]
fn note(_line: &str) {}

pub const ENEMY_VALUES: usize = 0;
pub const ENEMY_PTRS: usize = 1;
pub const FRIEND_VALUES: usize = 2;
pub const FRIEND_OPTS: usize = 3;
pub const RESOURCE: usize = 4;
pub const CREATE: usize = 5;
pub const PUMP: usize = 6;
pub const PERMISSIONS: usize = 7;
pub const PAY: usize = 8;
pub const WORLD_QUERY: usize = 9;
pub const CHAT: usize = 10;
const SLOTS: usize = 11;
const NAMES: [&str; SLOTS] = [
    "enemy(values)",
    "enemy(pointers)",
    "friend(values)",
    "friend(options)",
    "resource delta",
    "entity create",
    "players pump",
    "permissions",
    "pay costs",
    "world query",
    "chat enqueue",
];
const THREADS: usize = 4;
const REPORT_EVERY_MS: u64 = 10_000;
const CHAT_LOGGED_MAX: u64 = 40;

#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);
#[allow(clippy::declare_interior_mutable_const)]
const ZERO32: AtomicU32 = AtomicU32::new(0);
#[allow(clippy::declare_interior_mutable_const)]
const ROW: [AtomicU32; THREADS] = [ZERO32; THREADS];

static ON: AtomicBool = AtomicBool::new(false);
static INJECT: AtomicBool = AtomicBool::new(false);
static CALLS: [AtomicU64; SLOTS] = [ZERO; SLOTS];
static SEEN: [[AtomicU32; THREADS]; SLOTS] = [ROW; SLOTS];
static REPORT_AT: AtomicU64 = AtomicU64::new(0);
static START: OnceLock<Instant> = OnceLock::new();
static CHAT_LOGGED: AtomicU64 = AtomicU64::new(0);
static CHAT_QUEUE: AtomicUsize = AtomicUsize::new(0);
static CHAT_THREAD: AtomicU32 = AtomicU32::new(0);

pub fn set(on: bool) {
    if on {
        START.get_or_init(Instant::now);
    }
    ON.store(on, Ordering::Relaxed);
}

pub fn set_inject(on: bool) {
    INJECT.store(on, Ordering::Relaxed);
}

#[inline]
pub fn inject_on() -> bool {
    INJECT.load(Ordering::Relaxed)
}

#[inline]
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Counts one call of a hooked game function and remembers which threads make it.
#[inline]
pub fn hit(slot: usize) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    count(slot);
}

#[cold]
fn count(slot: usize) {
    CALLS[slot].fetch_add(1, Ordering::Relaxed);
    let me = thread_id();
    for cell in &SEEN[slot] {
        let id = cell.load(Ordering::Relaxed);
        if id == me {
            break;
        }
        if id == 0 && cell.compare_exchange(0, me, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            break;
        }
    }
    maybe_report();
}

/// One line per call of the game's chat function while the probe is on (first calls only).
pub fn chat(queue: usize, kind: usize, length: usize, ours: bool, client: usize) {
    if !on() {
        return;
    }
    count(CHAT);
    let me = thread_id();
    CHAT_QUEUE.store(queue, Ordering::Relaxed);
    CHAT_THREAD.store(me, Ordering::Relaxed);
    if CHAT_LOGGED.fetch_add(1, Ordering::Relaxed) < CHAT_LOGGED_MAX {
        note(&format!(
            "probe: chat enqueue called by thread {me} with queue {queue:#x} kind {kind:#x} client {client:#x} length {length} ours {ours}"
        ));
    }
}

fn maybe_report() {
    let Some(start) = START.get() else { return };
    let now = start.elapsed().as_millis() as u64;
    let last = REPORT_AT.load(Ordering::Relaxed);
    if now.saturating_sub(last) < REPORT_EVERY_MS || REPORT_AT.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_err() {
        return;
    }
    let seconds = (now.saturating_sub(last).max(1)) as f64 / 1000.0;
    let mut parts = Vec::new();
    for (slot, name) in NAMES.iter().enumerate() {
        let calls = CALLS[slot].swap(0, Ordering::Relaxed);
        if calls == 0 {
            continue;
        }
        let threads: Vec<u32> = SEEN[slot].iter().map(|c| c.load(Ordering::Relaxed)).filter(|t| *t != 0).collect();
        parts.push(format!("{name}: {:.1}/s threads {threads:?}", calls as f64 / seconds));
    }
    let queue = CHAT_QUEUE.load(Ordering::Relaxed);
    let tail = if queue != 0 { format!("; chat queue {queue:#x} (thread {})", CHAT_THREAD.load(Ordering::Relaxed)) } else { String::new() };
    if !parts.is_empty() {
        note(&format!("probe: calls per second by thread over the last {seconds:.0} s -> {}{tail}", parts.join("; ")));
    }
}

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub const OUT_FILE: &str = "m182_chat_out.txt";
/// Tells the panel whether lines can be queued right now: holds `1` while connected to a server, `0` otherwise.
pub const READY_FILE: &str = "m182_chat_out.ready";
const MAX_LINE: usize = 0x400;
const MAX_WAITING: usize = 40;
const GAP: Duration = Duration::from_millis(300);
/// The sender function runs every frame while connected; if it has been silent longer than this the
/// connection is gone and the queue pointer must not be used.
const FRESH_MS: u64 = 1500;
const SENT_LOGGED_MAX: u64 = 30;

/// Chat queue the game's per-frame sender empties (address of the current connection's queue).
static QUEUE: AtomicUsize = AtomicUsize::new(0);
/// Milliseconds (since `START`) of the last time the per-frame sender ran.
static LAST_FLUSH: AtomicU64 = AtomicU64::new(0);
static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
/// Address of the game's "queue a chat line" function (0 until found).
static SEND_FN: AtomicUsize = AtomicUsize::new(0);
static LINES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static PENDING: AtomicBool = AtomicBool::new(false);
static LAST_SENT: Mutex<Option<Instant>> = Mutex::new(None);
static LAST_WARNED: Mutex<Option<Instant>> = Mutex::new(None);
static SENT: AtomicU64 = AtomicU64::new(0);
/// 0 = nothing written yet, 1 = ready file says connected, 2 = ready file says not connected.
static REPORTED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn now_ms() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// The chat queue the game last used (0 when the per-frame sender has not run yet).
pub fn known_queue() -> usize {
    QUEUE.load(Ordering::Relaxed)
}

/// Where the game's "queue a chat line" function lives (set once at start).
pub fn set_send_function(address: usize) {
    SEND_FN.store(address, Ordering::SeqCst);
}

/// Called from the game thread every time the game's per-frame chat sender starts: `this` is its object, whose
/// field at +0xf8 is the queue lines are appended to. Two stores, nothing else.
pub fn flush_seen(this: usize) {
    if this == 0 {
        return;
    }
    let queue = unsafe { std::ptr::read_unaligned((this + 0xf8) as *const usize) };
    QUEUE.store(queue, Ordering::Relaxed);
    LAST_FLUSH.store(now_ms(), Ordering::Relaxed);
}

fn connected() -> bool {
    let last = LAST_FLUSH.load(Ordering::Relaxed);
    last != 0 && QUEUE.load(Ordering::Relaxed) != 0 && now_ms().saturating_sub(last) <= FRESH_MS
}

/// The panel's lines are picked up on their own thread, so a slow disk never delays the power loop or the spawn orders.
pub fn start() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new().name("m182-outbox".into()).spawn(|| loop {
        std::thread::sleep(Duration::from_millis(250));
        if let Ok(dir) = std::env::current_dir() {
            poll(&dir);
        }
    });
}

/// Reads the lines the panel left in `m182_chat_out.txt` (only while chat sending is switched on).
pub fn poll(dir: &Path) {
    if !super::probe::inject_on() || !super::config::LIVE.chat_key().is_empty() {
        return;
    }
    report_ready(dir);
    let path = dir.join(OUT_FILE);
    if !path.exists() {
        return;
    }
    let taken = dir.join(format!("{OUT_FILE}.taken"));
    if std::fs::rename(&path, &taken).is_err() {
        return;
    }
    let text = std::fs::read_to_string(&taken).unwrap_or_default();
    let _ = std::fs::remove_file(&taken);
    let mut added = 0usize;
    if let Ok(mut lines) = LINES.lock() {
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && l.len() <= MAX_LINE) {
            if lines.len() >= MAX_WAITING {
                break;
            }
            lines.push_back(line.to_string());
            added += 1;
        }
    }
    if added > 0 {
        PENDING.store(true, Ordering::SeqCst);
        super::spawner::log(&format!("chat out: {added} line(s) waiting for the game thread"));
    }
}

/// Keeps `m182_chat_out.ready` in step with the connection (written only when it changes).
fn report_ready(dir: &Path) {
    let ready = SEND_FN.load(Ordering::Relaxed) != 0 && connected();
    let state = if ready { 1 } else { 2 };
    if REPORTED.swap(state, Ordering::Relaxed) != state {
        let _ = std::fs::write(dir.join(READY_FILE), if ready { "1" } else { "0" });
    }
}

fn warn_rarely(message: &str) {
    let Ok(mut last) = LAST_WARNED.lock() else { return };
    if last.map_or(true, |t| t.elapsed() >= Duration::from_secs(30)) {
        *last = Some(Instant::now());
        super::spawner::log(message);
    }
}

/// The number of a `rev <n>` / `pull <n> <topic>` request in a line the panel sent.
fn asked_seq(line: &str) -> Option<u32> {
    let mut words = line.split_whitespace();
    while let Some(word) = words.next() {
        if word == "rev" || word == "pull" {
            return words.next()?.parse().ok();
        }
    }
    None
}

type SendFn = extern "win64" fn(*mut u8, usize, usize, *const [u64; 2], u32, u32) -> usize;

/// Sends at most one waiting line. Runs inside a hook the game calls from its own thread, which is the
/// thread that owns the chat queue, so the queue is never touched from two threads.
pub fn pump() {
    if !PENDING.load(Ordering::Relaxed) {
        return;
    }
    if LAST_SENT.lock().ok().and_then(|t| *t).is_some_and(|t| t.elapsed() < GAP) {
        return;
    }
    let function = SEND_FN.load(Ordering::SeqCst);
    if function == 0 {
        warn_rarely("chat out: the game's chat function was not found in this build; nothing sent");
        if let Ok(mut lines) = LINES.lock() {
            lines.clear();
        }
        PENDING.store(false, Ordering::SeqCst);
        return;
    }
    if !connected() {
        warn_rarely("chat out: not connected to a server right now; lines are kept until the game sends chat again");
        return;
    }
    let queue = QUEUE.load(Ordering::Relaxed);
    let line = {
        let Ok(mut lines) = LINES.lock() else { return };
        let line = lines.pop_front();
        if lines.is_empty() {
            PENDING.store(false, Ordering::SeqCst);
        }
        line
    };
    let Some(line) = line else { return };
    if let Ok(mut t) = LAST_SENT.lock() {
        *t = Some(Instant::now());
    }
    let bytes = line.as_bytes();
    let text: [u64; 2] = [bytes.as_ptr() as u64, bytes.len() as u64];
    let mut out = [0u8; 16];
    let call: SendFn = unsafe { std::mem::transmute(function) };
    call(out.as_mut_ptr(), queue, 0, &text, 0, 0);
    if let Some(seq) = asked_seq(&line) {
        super::spawner::expect_shared_reply(seq);
    }
    if SENT.fetch_add(1, Ordering::Relaxed) < SENT_LOGGED_MAX {
        let status = out[0];
        super::spawner::log(&format!(
            "chat out: queued a line of {} bytes through the game (answer {status:#x}); {} more waiting",
            bytes.len(),
            LINES.lock().map(|l| l.len()).unwrap_or(0)
        ));
    }
}

#[cfg(test)]
mod asked_tests {
    use super::asked_seq;

    #[test]
    fn the_number_of_a_request_is_found() {
        assert_eq!(asked_seq("!m182 key pull 4021 zones"), Some(4021));
        assert_eq!(asked_seq("!m182 key rev 77"), Some(77));
        assert_eq!(asked_seq("!m182 key players 5"), None);
        assert_eq!(asked_seq("!m182 key rev"), None);
    }
}

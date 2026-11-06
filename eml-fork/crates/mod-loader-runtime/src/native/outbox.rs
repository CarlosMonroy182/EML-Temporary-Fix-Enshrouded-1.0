use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub const OUT_FILE: &str = "m182_chat_out.txt";
const MAX_LINE: usize = 0x400;
const GAP: Duration = Duration::from_millis(300);
const CAPTURE_LOGGED_MAX: u64 = 16;
const SENT_LOGGED_MAX: u64 = 30;

/// The extra arguments of a real chat call, copied from the game's own stack.
#[derive(Clone, Copy, Debug)]
struct Args {
    a5: u32,
    a6: u32,
    a7: u32,
    a8: u64,
    optional: bool,
}

static QUEUE: AtomicUsize = AtomicUsize::new(0);
static TEMPLATE: Mutex<Option<Args>> = Mutex::new(None);
static LINES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static PENDING: AtomicBool = AtomicBool::new(false);
static LAST_SENT: Mutex<Option<Instant>> = Mutex::new(None);
static LAST_WARNED: Mutex<Option<Instant>> = Mutex::new(None);
static CAPTURED: AtomicU64 = AtomicU64::new(0);
static SENT: AtomicU64 = AtomicU64::new(0);

/// The chat queue the game's own chat function last used (0 when it has not run yet).
pub fn known_queue() -> usize {
    QUEUE.load(Ordering::Relaxed)
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    super::spawn_probe::u32_at(b, at)
}

/// Called from the game thread by the hook on the chat function: remembers the chat queue and how the
/// game itself fills in the arguments, so lines can later be queued the same way.
pub fn capture(queue: usize, kind: usize, frame: usize, length: usize, client: usize) {
    if queue != 0 {
        QUEUE.store(queue, Ordering::Relaxed);
    }
    let Some(stack) = super::spawn_probe::read_bytes(frame + 0x28, 0x28) else { return };
    let extra = super::spawn_probe::u64_at(&stack, 0x20) as usize;
    let tail = super::spawn_probe::read_bytes(extra, 16).unwrap_or_default();
    let optional = tail.get(8).copied().unwrap_or(0) != 0;
    let args = Args { a5: u32_at(&stack, 0), a6: u32_at(&stack, 8), a7: u32_at(&stack, 0x10), a8: super::spawn_probe::u64_at(&stack, 0x18), optional };
    if super::probe::on() && CAPTURED.fetch_add(1, Ordering::Relaxed) < CAPTURE_LOGGED_MAX {
        super::spawner::log(&format!(
            "probe: chat call kind {kind:#x} length {length} client {client:#x} arguments {:#x} {:#x} {:#x} {:#x} optional {} ({})",
            args.a5,
            args.a6,
            args.a7,
            args.a8,
            args.optional,
            super::spawn_probe::hex(&tail)
        ));
    }
    if kind == 0 {
        if let Ok(mut t) = TEMPLATE.lock() {
            *t = Some(args);
        }
    }
}

/// Reads the lines the panel left in `m182_chat_out.txt` (only while chat sending is switched on).
pub fn poll(dir: &Path) {
    if !super::probe::inject_on() {
        return;
    }
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
            lines.push_back(line.to_string());
            added += 1;
        }
    }
    if added > 0 {
        PENDING.store(true, Ordering::SeqCst);
        super::spawner::log(&format!("chat out: {added} line(s) waiting for the game thread"));
    }
}

fn warn_rarely(message: &str) {
    let Ok(mut last) = LAST_WARNED.lock() else { return };
    if last.map_or(true, |t| t.elapsed() >= Duration::from_secs(30)) {
        *last = Some(Instant::now());
        super::spawner::log(message);
    }
}

/// Sends at most one waiting line. Runs inside a hook the game calls from its own thread, which is the
/// thread that owns the chat queue, so the queue is never touched from two threads.
pub fn pump() {
    if !PENDING.load(Ordering::Relaxed) {
        return;
    }
    if LAST_SENT.lock().ok().and_then(|t| *t).is_some_and(|t| t.elapsed() < GAP) {
        return;
    }
    let queue = QUEUE.load(Ordering::Relaxed);
    let original = super::patch::CHAT_TRAMPOLINE.load(Ordering::SeqCst);
    let template = TEMPLATE.lock().ok().and_then(|t| *t);
    let (Some(template), true, true) = (template, queue != 0, original != 0) else {
        warn_rarely("chat out: waiting for the game to show how it sends a chat line (write one normal message in the chat)");
        return;
    };
    if template.optional {
        warn_rarely("chat out: the game's own call carries an optional argument this build does not copy; nothing sent");
        if let Ok(mut lines) = LINES.lock() {
            lines.clear();
        }
        PENDING.store(false, Ordering::SeqCst);
        return;
    }
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
    let none: [u64; 2] = [0, 0];
    let mut out = [0u8; 16];
    let call: extern "win64" fn(*mut u8, usize, usize, *const [u64; 2], u32, u32, u32, u64, *const [u64; 2]) -> usize = unsafe { std::mem::transmute(original) };
    call(out.as_mut_ptr(), queue, 0, &text, template.a5, template.a6, template.a7, template.a8, &none);
    if SENT.fetch_add(1, Ordering::Relaxed) < SENT_LOGGED_MAX {
        let status = out[0];
        super::spawner::log(&format!(
            "chat out: queued a line of {} bytes through the game (answer {status:#x}); {} more waiting",
            bytes.len(),
            LINES.lock().map(|l| l.len()).unwrap_or(0)
        ));
    }
}

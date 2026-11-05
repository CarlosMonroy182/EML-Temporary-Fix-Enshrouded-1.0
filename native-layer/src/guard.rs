//! M182 Guard (stage 1, observe only): records what the server receives when a player joins.
//!
//! Nothing here changes game behaviour. The hook runs before the server deserializes the
//! character save a client uploads, copies the blob to `m182_guard/` and logs its identity.
//! Configuration lives in its own file, `m182_guard.json`, so it never touches the panel or VFX.

use std::ffi::c_void;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Deserialize;

use super::{patch, signatures};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GuardConfig {
    /// "off" (default) or "observe".
    pub mode: String,
    pub max_dumps: u32,
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self { mode: "off".into(), max_dumps: 60 }
    }
}

static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static DUMPS: AtomicUsize = AtomicUsize::new(0);
static MAX_DUMPS: AtomicUsize = AtomicUsize::new(60);

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

fn log(msg: &str) {
    let Ok(dir) = std::env::current_dir() else { return };
    let dir = dir.join("m182_guard");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("guard.log")) {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "[{secs}] {msg}");
    }
}

struct Joined {
    player: u32,
    level: u32,
    xp: u32,
    extra: u32,
    at: std::time::Instant,
}

struct Watch {
    player: u32,
    addr: usize,
    last: Vec<u8>,
}

static JOINED: Mutex<Vec<Joined>> = Mutex::new(Vec::new());
static WATCHES: Mutex<Vec<Watch>> = Mutex::new(Vec::new());

const MAX_WATCHES: usize = 96;
const SCAN_DELAY_SECS: u64 = 15;
const WATCH_BYTES: usize = 16;

fn unpack(blob: &[u8]) -> Option<Vec<u8>> {
    if blob.len() < 8 || blob[0] != 1 || blob[1..5] != [0x28, 0xb5, 0x2f, 0xfd] {
        return None;
    }
    zstd::stream::decode_all(&blob[1..]).ok()
}

fn stats(raw: &[u8]) -> Option<(u32, u32, u32)> {
    let one = [0x00u8, 0x00, 0x80, 0x3f];
    let limit = raw.len().min(1024);
    let mut i = 0usize;
    while i + 32 <= limit {
        if (0..3).all(|k| raw[i + k * 4..i + k * 4 + 4] == one) {
            let mut j = i + 12;
            while j + 4 <= limit && raw[j..j + 4] == one {
                j += 4;
            }
            if j + 12 > raw.len() {
                return None;
            }
            let u = |o: usize| u32::from_le_bytes([raw[o], raw[o + 1], raw[o + 2], raw[o + 3]]);
            return Some((u(j), u(j + 4), u(j + 8)));
        }
        i += 1;
    }
    None
}

fn context_dump(file: &mut std::fs::File, addr: usize) {
    let from = addr.saturating_sub(0x100) & !0xf;
    let Some(bytes) = read_mem(from, 0x300) else {
        let _ = writeln!(file, "  {addr:#x}: context not readable");
        return;
    };
    let _ = writeln!(file, "  hit {addr:#x} (context from {from:#x})");
    for (row, chunk) in bytes.chunks(16).enumerate() {
        let _ = writeln!(file, "    {:#x}: {}", from + row * 16, hex(chunk));
    }
}

fn scan(job: &Joined) {
    let started = std::time::Instant::now();
    let regions = super::spawner::readable_regions();
    let total: usize = regions.iter().map(|(a, b)| b - a).sum();
    let mut pair = job.level.to_le_bytes().to_vec();
    pair.extend_from_slice(&job.xp.to_le_bytes());
    let mut tail = job.xp.to_le_bytes().to_vec();
    tail.extend_from_slice(&job.extra.to_le_bytes());
    let by_pair = super::spawner::scan_regions(&regions, &pair, 64);
    let by_tail = super::spawner::scan_regions(&regions, &tail, 64);
    log(&format!(
        "scan: player id {}, level {} xp {}: {} hits for level+xp, {} hits for xp+next, {} regions {} MB, {} ms",
        job.player,
        job.level,
        job.xp,
        by_pair.len(),
        by_tail.len(),
        regions.len(),
        total >> 20,
        started.elapsed().as_millis()
    ));
    let Ok(dir) = std::env::current_dir() else { return };
    let dir = dir.join("m182_guard");
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(mut f) = std::fs::File::create(dir.join(format!("scan_{secs}_p{}.txt", job.player))) {
        let _ = writeln!(f, "player id {} level {} xp {} next {}", job.player, job.level, job.xp, job.extra);
        let _ = writeln!(f, "level+xp hits:");
        for &a in by_pair.iter().take(24) {
            context_dump(&mut f, a);
        }
        let _ = writeln!(f, "xp+next hits:");
        for &a in by_tail.iter().take(24) {
            context_dump(&mut f, a);
        }
    }
    let Ok(mut watches) = WATCHES.lock() else { return };
    for &a in by_pair.iter().chain(by_tail.iter()) {
        if watches.len() >= MAX_WATCHES {
            break;
        }
        if watches.iter().any(|w| w.addr == a) {
            continue;
        }
        if let Some(last) = read_mem(a, WATCH_BYTES) {
            watches.push(Watch { player: job.player, addr: a, last });
        }
    }
}

fn watch_tick() {
    let Ok(mut watches) = WATCHES.lock() else { return };
    let mut lines = Vec::new();
    watches.retain_mut(|w| match read_mem(w.addr, WATCH_BYTES) {
        Some(now) => {
            if now != w.last {
                let u = |b: &[u8], o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
                lines.push(format!(
                    "watch: player id {} at {:#x}: {} {} {} {} (was {} {} {} {})",
                    w.player,
                    w.addr,
                    u(&now, 0),
                    u(&now, 4),
                    u(&now, 8),
                    u(&now, 12),
                    u(&w.last, 0),
                    u(&w.last, 4),
                    u(&w.last, 8),
                    u(&w.last, 12)
                ));
                w.last = now;
            }
            true
        }
        None => {
            lines.push(format!("watch: player id {} at {:#x}: memory released", w.player, w.addr));
            false
        }
    });
    drop(watches);
    for l in lines {
        log(&l);
    }
}

fn start_worker() {
    std::thread::spawn(|| {
        let mut ticks = 0u64;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            ticks += 1;
            let due: Vec<Joined> = match JOINED.lock() {
                Ok(mut q) => {
                    let mut ready = Vec::new();
                    let mut i = 0;
                    while i < q.len() {
                        if q[i].at.elapsed().as_secs() >= SCAN_DELAY_SECS {
                            ready.push(q.remove(i));
                        } else {
                            i += 1;
                        }
                    }
                    ready
                }
                Err(_) => Vec::new(),
            };
            for job in &due {
                scan(job);
            }
            if ticks % 5 == 0 {
                watch_tick();
            }
        }
    });
}

fn list_saves() {
    let Ok(dir) = std::env::current_dir() else { return };
    let mut lines = Vec::new();
    let mut stack = vec![(dir.clone(), 0u32)];
    while let Some((path, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&path) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = p.strip_prefix(&dir).unwrap_or(&p).display().to_string();
            let lower = name.to_ascii_lowercase();
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                if depth < 3 && !lower.contains("mods") && !lower.contains("m182_guard") && !lower.contains("logs") {
                    stack.push((p, depth + 1));
                }
            } else if depth > 0 && !lower.contains(".dll") && !lower.contains(".lua") {
                let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                lines.push(format!("  {name} {} bytes mtime={mtime}", meta.len()));
            }
        }
    }
    lines.sort();
    lines.truncate(80);
    log(&format!("server files:\n{}", lines.join("\n")));
}

extern "win64" fn probe(_out: usize, obj: usize) {
    if obj == 0 {
        return;
    }
    let Some(head) = read_mem(obj + 8, 4) else { return };
    let player = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    let Some(vec) = read_mem(obj + 0x110, 0x10) else { return };
    let ptr = usize::from_le_bytes(vec[0..8].try_into().unwrap_or([0; 8]));
    let size = usize::from_le_bytes(vec[8..16].try_into().unwrap_or([0; 8]));
    if ptr == 0 || size < 0x30 || size > 8 * 1024 * 1024 {
        log(&format!("upload: player id {player}, unreadable blob (ptr={ptr:#x}, size={size})"));
        return;
    }
    let Some(blob) = read_mem(ptr, size) else {
        log(&format!("upload: player id {player}, blob not readable (size {size})"));
        return;
    };
    let magic = u32::from_le_bytes([blob[0], blob[1], blob[2], blob[3]]);
    let uuid = hex(&blob[0xC..0x1C]);
    let hash = fnv1a(&blob);
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut saved = String::from("not saved (limit reached)");
    if DUMPS.fetch_add(1, Ordering::SeqCst) < MAX_DUMPS.load(Ordering::SeqCst) {
        if let Ok(dir) = std::env::current_dir() {
            let dir = dir.join("m182_guard");
            let _ = std::fs::create_dir_all(&dir);
            let name = format!("{secs}_p{player}_{hash:08x}.bin");
            if std::fs::write(dir.join(&name), &blob).is_ok() {
                saved = name;
            }
        }
    }
    log(&format!("upload: player id {player}, {size} bytes, magic {magic:#x}, character {uuid}, fnv1a {hash:08x}, file {saved}"));
    match unpack(&blob) {
        Some(raw) => {
            if let Ok(dir) = std::env::current_dir() {
                let _ = std::fs::write(dir.join("m182_guard").join(format!("{secs}_p{player}_{hash:08x}.raw")), &raw);
            }
            match stats(&raw) {
                Some((level, xp, extra)) => {
                    log(&format!("join: player id {player}, level {level}, xp {xp}, next {extra}, {} bytes unpacked", raw.len()));
                    if let Ok(mut q) = JOINED.lock() {
                        q.push(Joined { player, level, xp, extra, at: std::time::Instant::now() });
                    }
                }
                None => log(&format!("join: player id {player}, level and xp not found ({} bytes unpacked)", raw.len())),
            }
        }
        None => log(&format!("join: player id {player}, blob is not 0x01 + zstd")),
    }
    list_saves();
}

macro_rules! entry_stub {
    () => {
        #[unsafe(naked)]
        unsafe extern "win64" fn stub_probe() {
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
                f = sym probe,
                t = sym TRAMPOLINE,
            );
        }
    };
}
entry_stub!();

pub(super) fn install(sections: &[&'static [u8]]) {
    let Ok(dir) = std::env::current_dir() else { return };
    let cfg = config(&dir);
    if !cfg.mode.eq_ignore_ascii_case("observe") {
        return;
    }
    MAX_DUMPS.store(cfg.max_dumps as usize, Ordering::SeqCst);
    let target = match patch::find_unique(sections, &signatures::SAVE_DESERIALIZE) {
        Ok(addr) => addr,
        Err(count) => {
            log(&format!("guard off: savegame function signature matched {count} times (game version changed?)"));
            return;
        }
    };
    let page = unsafe { VirtualAlloc(std::ptr::null_mut(), 0x1000, 0x3000, 0x40) } as *mut u8;
    if page.is_null() {
        log("guard off: trampoline allocation failed");
        return;
    }
    let displaced = signatures::SAVE_DESERIALIZE_DISPLACED;
    unsafe {
        std::ptr::copy_nonoverlapping(target, page, displaced);
        let tail = page.add(displaced);
        tail.write(0xFF);
        tail.add(1).write(0x25);
        tail.add(2).cast::<u32>().write_unaligned(0);
        tail.add(6).cast::<u64>().write_unaligned(target.add(displaced) as u64);
    }
    TRAMPOLINE.store(page as usize, Ordering::SeqCst);
    match unsafe { patch::write_jump(target, stub_probe as usize) } {
        Ok(()) => {
            start_worker();
            log("guard: observe mode ready (nothing is blocked; memory watch on)");
        }
        Err(code) => {
            TRAMPOLINE.store(0, Ordering::SeqCst);
            log(&format!("guard off: VirtualProtect failed ({code})"));
        }
    }
}

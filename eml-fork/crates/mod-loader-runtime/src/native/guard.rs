//! M182 Guard (stage 1, observe only): records what the server receives when a player joins.
//!
//! Nothing here changes game behaviour. The hook runs before the server deserializes the
//! character save a client uploads, copies the blob to `m182_guard/` and logs its identity.
//! Configuration lives in its own file, `m182_guard.json`, so it never touches the panel or VFX.

use std::ffi::c_void;
use std::io::Write;
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
        Ok(()) => log("guard: observe mode ready (nothing is blocked)"),
        Err(code) => {
            TRAMPOLINE.store(0, Ordering::SeqCst);
            log(&format!("guard off: VirtualProtect failed ({code})"));
        }
    }
}

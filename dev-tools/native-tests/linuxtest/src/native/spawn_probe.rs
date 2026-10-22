use std::ffi::c_void;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use super::config::LIVE;
use super::{patch, signatures, spawner};

unsafe fn GetModuleHandleW(_n: *const u16) -> *mut c_void { unsafe { super::patch::FAKE_BASE.cast() } }
unsafe fn GetCurrentProcess() -> *mut c_void { 1 as *mut c_void }
unsafe fn VirtualAlloc(_a: *mut c_void, size: usize, _k: u32, _p: u32) -> *mut c_void { unsafe { libc::mmap(std::ptr::null_mut(), size, 7, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0) } }
unsafe fn ReadProcessMemory(_p: *mut c_void, addr: *const c_void, buf: *mut c_void, size: usize, read: *mut usize) -> i32 {
    unsafe {
        let local = libc::iovec { iov_base: buf, iov_len: size };
        let remote = libc::iovec { iov_base: addr as *mut c_void, iov_len: size };
        let n = libc::process_vm_readv(libc::getpid(), &local, 1, &remote, 1, 0);
        if n < 0 || (n as usize) < size {
            *read = if n < 0 { 0 } else { n as usize };
            return 0;
        }
        *read = size;
    }
    1
}

const MEM_COMMIT_RESERVE: u32 = 0x3000;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const DISPLACED: usize = 19;
const MAX_LOG_BYTES: u64 = 24 * 1024 * 1024;
const FULL_DUMPS_PER_KIND: u32 = 6;
const MAX_COMPONENTS: usize = 96;
const DUMP_BYTES: usize = 192;
const HASH_CURRENT_TRANSFORM: u32 = 0x027a_c564;
const HASH_OVERRIDE_LEVEL: u32 = 0xa21e_7ee7;

static DONE: AtomicBool = AtomicBool::new(false);
static WRITTEN: AtomicU64 = AtomicU64::new(0);
static COUNTER: AtomicU64 = AtomicU64::new(0);
static START: OnceLock<Instant> = OnceLock::new();
static SINK: Mutex<Option<Sink>> = Mutex::new(None);

static mut M182_SPAWN_TRAMPOLINE: usize = 0;

struct Sink {
    file: File,
    csv: Option<BufWriter<File>>,
    rows: u64,
    kinds: Vec<(u64, u32)>,
}

pub fn install(sections: &[&'static [u8]]) {
    if DONE.swap(true, Ordering::SeqCst) {
        return;
    }

    let target = match patch::find_unique(sections, &signatures::SPAWN_CREATE) {
        Ok(addr) => addr,
        Err(count) => {
            tracing::warn!(target: "m182::probe", matches = count, "spawn create signature not found exactly once; probe off");
            return;
        }
    };

    let page = unsafe { VirtualAlloc(std::ptr::null_mut(), 0x1000, MEM_COMMIT_RESERVE, PAGE_EXECUTE_READWRITE) } as *mut u8;
    if page.is_null() {
        tracing::warn!(target: "m182::probe", "trampoline allocation failed; probe off");
        return;
    }

    unsafe {
        std::ptr::copy_nonoverlapping(target, page, DISPLACED);
        let tail = page.add(DISPLACED);
        tail.write(0xFF);
        tail.add(1).write(0x25);
        tail.add(2).cast::<u32>().write_unaligned(0);
        tail.add(6).cast::<u64>().write_unaligned(target.add(DISPLACED) as u64);
        M182_SPAWN_TRAMPOLINE = page as usize;
    }

    let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
    let log_path = if LIVE.spawn_probe() {
        std::env::current_dir().map(|d| d.join("m182_spawn_probe.log")).ok()
    } else {
        None
    };
    if let Some(path) = log_path {
        if let Ok(mut file) = File::create(&path) {
            let _ = writeln!(file, "# m182 spawn probe | module base {:#x} | create rva {:#x}", base, target as usize - base);
            let _ = file.flush();
            let csv = File::create(path.with_file_name("m182_spawn_positions.csv")).ok().map(|f| {
                let mut w = BufWriter::new(f);
                let _ = writeln!(w, "t_ms,caller,mgr,edx,hint,guid,level,x,y,z");
                let _ = w.flush();
                w
            });
            if let Ok(mut guard) = SINK.lock() {
                *guard = Some(Sink { file, csv, rows: 0, kinds: Vec::new() });
            }
        }
    }
    let _ = START.set(Instant::now());
    MODULE_BASE.store(base as u64, Ordering::Relaxed);
    spawner::start();

    match unsafe { patch::write_jump(target, stub_spawn as usize) } {
        Ok(()) => tracing::info!(target: "m182::probe", address = format_args!("{:#x}", target as usize), "spawn probe hooked"),
        Err(code) => tracing::error!(target: "m182::probe", code, "VirtualProtect failed"),
    }
}

static MODULE_BASE: AtomicU64 = AtomicU64::new(0);

#[unsafe(naked)]
unsafe extern "win64" fn stub_spawn() {
    core::arch::naked_asm!(
        "sub rsp, 0x38",
        "mov r11, [rsp + 0x38]",
        "mov [rsp + 0x20], r11",
        "call {f}",
        "add rsp, 0x38",
        "ret",
        f = sym hook_create,
    );
}

pub(super) fn original_create(manager: usize, edx: u32, desc: usize, r9: usize) -> u32 {
    let tramp = unsafe { M182_SPAWN_TRAMPOLINE };
    if tramp == 0 {
        return 0;
    }
    let f: extern "win64" fn(usize, u32, usize, usize) -> u32 = unsafe { std::mem::transmute(tramp) };
    f(manager, edx, desc, r9)
}

extern "win64" fn hook_create(manager: usize, edx: u32, desc: usize, r9: usize, caller: usize) -> u32 {
    if LIVE.spawn_probe() {
        log_call(manager, edx as usize, desc, caller);
    }
    let hooked = LIVE.spawner() || super::vfx::wanted();
    if hooked {
        spawner::before(manager, edx);
    }
    let ret = original_create(manager, edx, desc, r9);
    if hooked {
        spawner::after(manager, edx, desc, ret);
    }
    ret
}

pub(super) fn read_bytes(addr: usize, len: usize) -> Option<Vec<u8>> {
    if addr < 0x10000 || len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    let mut read = 0usize;
    let ok = unsafe { ReadProcessMemory(GetCurrentProcess(), addr as *const c_void, buf.as_mut_ptr().cast(), len, &mut read) };
    if ok == 0 || read == 0 {
        return None;
    }
    buf.truncate(read);
    Some(buf)
}

pub(super) fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

pub(super) fn u32_at(b: &[u8], off: usize) -> u32 {
    b.get(off..off + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0)
}

pub(super) fn u64_at(b: &[u8], off: usize) -> u64 {
    b.get(off..off + 8)
        .map(|s| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
        .unwrap_or(0)
}

fn log_call(manager: usize, layer: usize, desc_ptr: usize, caller: usize) {
    if WRITTEN.load(Ordering::Relaxed) > MAX_LOG_BYTES {
        return;
    }
    let Ok(mut guard) = SINK.try_lock() else { return };
    let Some(sink) = guard.as_mut() else { return };

    let n = COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
    let base = MODULE_BASE.load(Ordering::Relaxed) as usize;
    let ms = START.get().map(|s| s.elapsed().as_millis()).unwrap_or(0);

    let desc = read_bytes(desc_ptr, 0x30).unwrap_or_default();
    let array = u64_at(&desc, 0x18) as usize;
    let count = (u64_at(&desc, 0x20) as usize).min(MAX_COMPONENTS);
    let entries = read_bytes(array, count * 24).unwrap_or_default();
    let count = (entries.len() / 24).min(count);

    if let Some(csv) = sink.csv.as_mut() {
        let mut pos = None;
        let mut level = None;
        for i in 0..count {
            let hash = u32_at(&entries, i * 24);
            let data = u64_at(&entries, i * 24 + 8) as usize;
            let size = u64_at(&entries, i * 24 + 16) as usize;
            if hash == HASH_CURRENT_TRANSFORM && size >= 24 {
                if let Some(b) = read_bytes(data, 24) {
                    pos = Some((u64_at(&b, 0) as i64, u64_at(&b, 8) as i64, u64_at(&b, 16) as i64));
                }
            } else if hash == HASH_OVERRIDE_LEVEL && size >= 4 {
                if let Some(b) = read_bytes(data, 4) {
                    level = Some(u32_at(&b, 0));
                }
            }
        }
        if let Some((x, y, z)) = pos {
            let guid = desc.get(4..20).map(hex).unwrap_or_default();
            let line = format!(
                "{},{:x},{:x},{:08x},{:08x},{},{},{},{},{}\n",
                ms,
                caller.wrapping_sub(base),
                manager & 0xffff_ffff,
                layer as u32,
                u32_at(&desc, 0),
                guid,
                level.map(|l| l as i64).unwrap_or(-1),
                x,
                y,
                z
            );
            if WRITTEN.load(Ordering::Relaxed) < MAX_LOG_BYTES {
                WRITTEN.fetch_add(line.len() as u64, Ordering::Relaxed);
                let _ = csv.write_all(line.as_bytes());
                sink.rows += 1;
                let _ = csv.flush();
            }
        }
    }

    let mut key = 0xcbf2_9ce4_8422_2325u64;
    for i in 0..count {
        key ^= u32_at(&entries, i * 24) as u64;
        key = key.wrapping_mul(0x0000_0100_0000_01b3);
    }
    key ^= (caller.wrapping_sub(base)) as u64;

    let seen = match sink.kinds.iter_mut().find(|(k, _)| *k == key) {
        Some((_, c)) => {
            *c += 1;
            *c
        }
        None => {
            sink.kinds.push((key, 1));
            1
        }
    };

    if seen > FULL_DUMPS_PER_KIND && seen % 200 != 0 {
        return;
    }

    let mut text = format!(
        "#{n} t={ms}ms caller=+{:#x} mgr={:#x} edx={:#x} kind={:016x} seen={} count={} desc={}\n",
        caller.wrapping_sub(base),
        manager,
        layer as u32,
        key,
        seen,
        count,
        hex(&desc)
    );

    if seen <= FULL_DUMPS_PER_KIND {
        for i in 0..count {
            let hash = u32_at(&entries, i * 24);
            let data = u64_at(&entries, i * 24 + 8) as usize;
            let size = u64_at(&entries, i * 24 + 16) as usize;
            let dump = read_bytes(data, size.min(DUMP_BYTES)).unwrap_or_default();
            text.push_str(&format!("  c{i} hash={hash:08x} size={size} data={}\n", hex(&dump)));
        }
    }

    WRITTEN.fetch_add(text.len() as u64, Ordering::Relaxed);
    let _ = sink.file.write_all(text.as_bytes());
    let _ = sink.file.flush();
}

pub fn set_trampoline(v: usize) { unsafe { M182_SPAWN_TRAMPOLINE = v } }

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

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
    fn FlushInstructionCache(process: *mut c_void, addr: *const c_void, size: usize) -> i32;
}

const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const JUMP_LEN: usize = 14;

static INSTALLED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------- hooks

#[inline]
fn read_faction(ptr: *const u8) -> u8 {
    if ptr.is_null() { 0 } else { unsafe { ptr.read() } }
}

static TEAM_CHECKS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn team_check_report(a: u8, b: u8, hostile: bool, via: &str) {
    let teams = faction::TEAM_FACTIONS;
    if (teams.contains(&a) || teams.contains(&b)) && faction::team_mask() != 0 && TEAM_CHECKS.fetch_add(1, Ordering::Relaxed) < 30 {
        super::spawner::log(&format!("teams: relation check {via} {a} vs {b} -> {}", if hostile { "hostile" } else { "not hostile" }));
    }
}

extern "win64" fn enemy_values(rcx: usize, rdx: usize) -> u8 {
    let (a, b) = (rcx as u8, rdx as u8);
    let hostile = faction::enemy(a, b, LIVE.pvp());
    team_check_report(a, b, hostile, "values");
    hostile as u8
}

extern "win64" fn enemy_ptrs(a: *const u8, b: *const u8) -> u8 {
    let (fa, fb) = (read_faction(a), read_faction(b));
    if groups::is_protected(a as usize) || groups::is_protected(b as usize) {
        if let Some(line) = groups::protected_report(a as usize, b as usize, fa, fb) {
            super::spawner::log(&line);
        }
        return 0;
    }
    let hostile = faction::enemy(fa, fb, LIVE.pvp());
    team_check_report(fa, fb, hostile, "pointers");
    hostile as u8
}

extern "win64" fn friend_values(rcx: usize, rdx: usize) -> u8 {
    faction::friend(rcx as u8, rdx as u8, LIVE.pvp_friend_checks()) as u8
}

extern "win64" fn friend_opts(a: *const u8, b: *const u8, opts: *const u8) -> u8 {
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

    if super::config::LIVE.spawn_probe() || super::config::LIVE.spawner() {
        super::spawn_probe::install(&sections);
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

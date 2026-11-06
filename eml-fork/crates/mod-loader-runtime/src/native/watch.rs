use std::sync::Mutex;

const NAMES: [&str; 4] = ["encode", "decode", "chat flush", "chat receive"];
const MAX_KEYS: usize = 160;
const SHOWN: u64 = 12;

struct Key {
    id: usize,
    desc: usize,
    from: usize,
    count: u64,
}

static SEEN: Mutex<Vec<Key>> = Mutex::new(Vec::new());

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

/// Called by the probe stubs in front of a few game functions. `regs` holds rax, r11, r10, r9, r8, rdx, rcx and the
/// return address, in that order. Only logs; the game function then runs unchanged.
pub fn hit(id: usize, regs: *const usize) {
    if !super::probe::on() || id >= NAMES.len() || regs.is_null() {
        return;
    }
    let r = unsafe { std::slice::from_raw_parts(regs, 8) };
    let (r8, rdx, rcx, ret) = (r[4], r[5], r[6], r[7]);
    let base = super::spawn_probe::module_base();
    let from = ret.wrapping_sub(base);
    let desc = if id < 2 { r8.wrapping_sub(base) } else { 0 };
    let Ok(mut seen) = SEEN.lock() else { return };
    let index = match seen.iter().position(|k| k.id == id && k.desc == desc && k.from == from) {
        Some(i) => i,
        None => {
            if seen.len() >= MAX_KEYS {
                return;
            }
            seen.push(Key { id, desc, from, count: 0 });
            seen.len() - 1
        }
    };
    seen[index].count += 1;
    let count = seen[index].count;
    drop(seen);
    if count <= SHOWN || count == 100 || count == 1000 || count == 10000 {
        let extra = match id {
            2 => {
                let queue = super::spawn_probe::read_bytes(rcx + 0xf8, 8).map(|b| super::spawn_probe::u64_at(&b, 0)).unwrap_or(0);
                format!(" object {rcx:#x} queue {queue:#x} (chat function saw {:#x})", super::outbox::known_queue())
            }
            3 => format!(" object {rcx:#x} message {rdx:#x}"),
            _ => format!(" value {rdx:#x}"),
        };
        super::spawner::log(&format!(
            "probe: {} descriptor {desc:#x} called from {from:#x} thread {} (#{count}){extra}",
            NAMES[id],
            thread_id()
        ));
    }
}

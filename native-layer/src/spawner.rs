use std::ffi::c_void;
use std::fs::File;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::config::{LIVE, SPAWN};
use super::spawn_probe::{hex, original_create, read_bytes, u32_at, u64_at};

#[link(name = "user32")]
unsafe extern "system" {
    fn GetAsyncKeyState(vk: i32) -> i16;
    fn GetForegroundWindow() -> *mut c_void;
    fn GetWindowThreadProcessId(hwnd: *mut c_void, pid: *mut u32) -> u32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcessId() -> u32;
}

const GUID_PLAYER: [u8; 16] = [0xd6, 0x66, 0xdb, 0xc3, 0xa5, 0x89, 0x6c, 0x4a, 0xa8, 0xe3, 0xd9, 0xa1, 0xf6, 0x00, 0x98, 0x12];

const HASH_CURRENT_TRANSFORM: u32 = 0x027a_c564;
const HASH_OVERRIDE_LEVEL: u32 = 0xa21e_7ee7;

const TRANSFORM_SIZE: usize = 56;
const COMPONENT_SLOTS: usize = 640;
const METERS: f64 = 4_294_967_296.0;
const MAX_CALIBRATION_TRIES: u32 = 400;

static STARTED: AtomicBool = AtomicBool::new(false);
static SERVER_MGR: AtomicUsize = AtomicUsize::new(0);
static PLAYER_ID: AtomicU32 = AtomicU32::new(0);
static TRANSFORM_INDEX: AtomicI32 = AtomicI32::new(-1);
static CALIBRATION_TRIES: AtomicU32 = AtomicU32::new(0);
static CALIBRATION_DUMPS: AtomicU32 = AtomicU32::new(0);
static PENDING: AtomicU32 = AtomicU32::new(0);
static BUSY: AtomicBool = AtomicBool::new(false);
static LOG: Mutex<Option<File>> = Mutex::new(None);
static CLOCK: OnceLock<Instant> = OnceLock::new();

pub fn log(message: &str) {
    let ms = CLOCK.get_or_init(Instant::now).elapsed().as_millis();
    let Ok(mut guard) = LOG.lock() else { return };
    if guard.is_none() {
        if let Ok(dir) = std::env::current_dir() {
            if let Ok(mut f) = File::create(dir.join("m182_spawner.log")) {
                let _ = writeln!(f, "# m182 spawner log");
                *guard = Some(f);
            }
        }
    }
    if let Some(f) = guard.as_mut() {
        let _ = writeln!(f, "[{ms}ms] {message}");
        let _ = f.flush();
    }
}

pub fn start() {
    if !LIVE.spawner() || STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = CLOCK.get_or_init(Instant::now);
    log("spawner enabled");
    if let Err(e) = std::thread::Builder::new().name("m182-spawner-keys".into()).spawn(key_loop) {
        tracing::warn!(target: "m182::spawner", error = %e, "could not start hotkey thread");
    }
}

fn game_has_focus() -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        pid == GetCurrentProcessId()
    }
}

fn key_loop() {
    let mut was_down = [false; 3];
    let mut last_report = Instant::now();
    let mut last_pos: Option<(i64, i64, i64)> = None;
    loop {
        std::thread::sleep(Duration::from_millis(25));
        if !LIVE.spawner() {
            was_down = [false; 3];
            continue;
        }
        let keys = match SPAWN.lock() {
            Ok(s) => [s.key, s.dump_player_key, s.dump_spawned_key],
            Err(_) => continue,
        };
        let focus = game_has_focus();
        let mut down = [false; 3];
        for i in 0..3 {
            down[i] = unsafe { GetAsyncKeyState(keys[i]) } as u16 & 0x8000 != 0 && focus;
        }
        if down[0] && !was_down[0] {
            PENDING.store(1, Ordering::SeqCst);
            log("spawn requested");
        }
        if down[1] && !was_down[1] {
            dump_entity("player", PLAYER_ID.load(Ordering::SeqCst));
        }
        if down[2] && !was_down[2] {
            dump_entity("last spawned", LAST_SPAWNED.load(Ordering::SeqCst));
        }
        was_down = down;

        if last_report.elapsed() >= Duration::from_secs(4) {
            last_report = Instant::now();
            if let Some(t) = player_transform() {
                let pos = (le_i64(&t, 0), le_i64(&t, 8), le_i64(&t, 16));
                if last_pos != Some(pos) {
                    last_pos = Some(pos);
                    log(&format!(
                        "player position x={:.2} y={:.2} z={:.2}",
                        pos.0 as f64 / METERS,
                        pos.1 as f64 / METERS,
                        pos.2 as f64 / METERS
                    ));
                }
            }
        }
    }
}

static DUMP_COUNTER: AtomicU32 = AtomicU32::new(0);
static LAST_SPAWNED: AtomicU32 = AtomicU32::new(0);

pub fn dump_entity(label: &str, id: u32) {
    let manager = SERVER_MGR.load(Ordering::SeqCst);
    let n = DUMP_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
    if manager == 0 || id == 0 {
        log(&format!("dump #{n} {label}: no entity available"));
        return;
    }
    let Some(record) = find_record(manager, id) else {
        log(&format!("dump #{n} {label}: entity {id} not found"));
        return;
    };
    let Some(p) = placement(record) else {
        log(&format!("dump #{n} {label}: entity {id} layout unreadable"));
        return;
    };
    log(&format!("dump #{n} {label} id={id} record={record:#x} chunk={:#x} slot={}", p.chunk, p.slot));
    for index in 0..COMPONENT_SLOTS {
        let Some((addr, stride)) = component_address(&p, index) else { continue };
        if stride == 0 {
            continue;
        }
        let bytes = read_bytes(addr, stride.min(192)).unwrap_or_default();
        log(&format!("dump #{n} idx={index} size={stride} data={}", hex(&bytes)));
    }
    log(&format!("dump #{n} end"));
}

fn le_i64(b: &[u8], off: usize) -> i64 {
    u64_at(b, off) as i64
}

fn is_server(manager: usize) -> bool {
    matches!(read_bytes(manager + 8, 1), Some(b) if b[0] == 0)
}

fn bit_set(def: usize, index: usize) -> bool {
    match read_bytes(def + (index >> 6) * 8, 8) {
        Some(b) => (u64_at(&b, 0) >> (index & 63)) & 1 == 1,
        None => false,
    }
}

fn hash_slot(id: u32, capacity: u64) -> u64 {
    let mut a = (id >> 16) ^ id;
    a = a.wrapping_mul(0x045d_9f3b);
    let mut b = (a >> 16) ^ a;
    b = b.wrapping_mul(0x045d_9f3b);
    ((b as u64 >> 16) ^ b as u64) & capacity.wrapping_sub(1)
}

fn find_record(manager: usize, id: u32) -> Option<usize> {
    let map = manager + 0x148;
    let head = read_bytes(map, 0x58)?;
    let capacity = u64_at(&head, 0x10);
    let count = u32_at(&head, 0x54);
    if count == 0 || capacity == 0 || capacity > (1 << 26) || capacity & (capacity - 1) != 0 {
        return None;
    }
    let bitmap = u64_at(&head, 8) as usize;
    let keys = u64_at(&head, 0x28) as usize;
    let values = u64_at(&head, 0x40) as usize;

    let mut slot = hash_slot(id, capacity);
    for _ in 0..capacity.min(4096) {
        let word = read_bytes(bitmap + (slot as usize >> 6) * 8, 8)?;
        if (u64_at(&word, 0) >> (slot & 63)) & 1 == 0 {
            return None;
        }
        let key = read_bytes(keys + slot as usize * 4, 4)?;
        if u32_at(&key, 0) == id {
            let value = read_bytes(values + slot as usize * 8, 8)?;
            let record = u64_at(&value, 0) as usize;
            if record == 0 {
                return None;
            }
            let check = read_bytes(record + 0x10, 4)?;
            return if u32_at(&check, 0) == id { Some(record) } else { None };
        }
        slot = if slot + 1 < capacity { slot + 1 } else { 0 };
    }
    None
}

struct Placement {
    def: usize,
    chunk: usize,
    slot: usize,
}

fn placement(record: usize) -> Option<Placement> {
    let b = read_bytes(record, 0x38)?;
    let def = u64_at(&b, 0x18) as usize;
    let chunk = u64_at(&b, 0x20) as usize;
    let slot = u32_at(&b, 0x30) as usize;
    if def < 0x10000 || chunk < 0x10000 {
        return None;
    }
    Some(Placement { def, chunk, slot })
}

fn component_address(p: &Placement, index: usize) -> Option<(usize, usize)> {
    if index >= COMPONENT_SLOTS || !bit_set(p.def, index) {
        return None;
    }
    let offset = read_bytes(p.def + 0x84 + index * 2, 2)?;
    let stride = read_bytes(p.def + 0xa84 + index * 2, 2)?;
    let offset = u16::from_le_bytes([offset[0], offset[1]]) as usize;
    let stride = u16::from_le_bytes([stride[0], stride[1]]) as usize;
    Some((p.chunk + offset + stride * p.slot, stride))
}

fn calibrate(manager: usize, id: u32, expected: &[u8]) -> bool {
    let Some(record) = find_record(manager, id) else {
        dump_failure(manager, id, None, "record lookup failed");
        return false;
    };
    let Some(p) = placement(record) else {
        dump_failure(manager, id, Some(record), "record fields unreadable");
        return false;
    };
    for index in 0..COMPONENT_SLOTS {
        let Some((addr, stride)) = component_address(&p, index) else { continue };
        if stride != TRANSFORM_SIZE {
            continue;
        }
        if let Some(b) = read_bytes(addr, 24) {
            if b == expected {
                TRANSFORM_INDEX.store(index as i32, Ordering::SeqCst);
                log(&format!("transform component found: index={index} id={id} record={record:#x} chunk={:#x} slot={}", p.chunk, p.slot));
                return true;
            }
        }
    }
    dump_failure(manager, id, Some(record), "no 56-byte component matched");
    false
}

fn dump_failure(manager: usize, id: u32, record: Option<usize>, why: &str) {
    if CALIBRATION_DUMPS.fetch_add(1, Ordering::Relaxed) >= 3 {
        return;
    }
    let mut text = format!("calibration failed ({why}) id={id} mgr={manager:#x}");
    if let Some(r) = record {
        if let Some(b) = read_bytes(r, 0x60) {
            text.push_str(&format!(" record={r:#x} bytes={}", hex(&b)));
            let def = u64_at(&b, 0x18) as usize;
            if let Some(d) = read_bytes(def, 0x90) {
                text.push_str(&format!(" def={def:#x} defbytes={}", hex(&d)));
            }
        }
    }
    log(&text);
}

fn player_transform() -> Option<Vec<u8>> {
    let index = TRANSFORM_INDEX.load(Ordering::SeqCst);
    let id = PLAYER_ID.load(Ordering::SeqCst);
    let manager = SERVER_MGR.load(Ordering::SeqCst);
    if index < 0 || id == 0 || manager == 0 {
        return None;
    }
    let record = find_record(manager, id)?;
    let p = placement(record)?;
    let (addr, stride) = component_address(&p, index as usize)?;
    if stride != TRANSFORM_SIZE {
        return None;
    }
    read_bytes(addr, TRANSFORM_SIZE)
}

fn component_data(entries: &[u8], count: usize, hash: u32, size: usize) -> Option<Vec<u8>> {
    for i in 0..count {
        if u32_at(entries, i * 24) == hash {
            let data = u64_at(entries, i * 24 + 8) as usize;
            let len = u64_at(entries, i * 24 + 16) as usize;
            if len >= size {
                return read_bytes(data, size);
            }
        }
    }
    None
}

pub fn before(manager: usize, edx: u32) {
    if PENDING.load(Ordering::Relaxed) == 0 {
        return;
    }
    if edx == 0 || edx >= 0x8000_0000 || manager != SERVER_MGR.load(Ordering::Relaxed) || !is_server(manager) {
        return;
    }
    if PENDING.compare_exchange(1, 0, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return;
    }
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    spawn_now(manager);
    BUSY.store(false, Ordering::SeqCst);
}

fn calibration_done() -> bool {
    TRANSFORM_INDEX.load(Ordering::Relaxed) >= 0 || CALIBRATION_TRIES.load(Ordering::Relaxed) >= MAX_CALIBRATION_TRIES
}

pub fn after(manager: usize, edx: u32, desc: usize, ret: u32) {
    if ret == 0 {
        return;
    }
    let settled = PLAYER_ID.load(Ordering::Relaxed) != 0 && calibration_done();
    if settled && edx != 1 {
        return;
    }
    if !is_server(manager) {
        return;
    }

    let Some(head) = read_bytes(desc, 0x30) else { return };
    if head.len() < 0x30 {
        return;
    }
    if head[4..20] == GUID_PLAYER && edx == 1 {
        PLAYER_ID.store(ret, Ordering::SeqCst);
        SERVER_MGR.store(manager, Ordering::SeqCst);
        log(&format!("player entity id={ret} mgr={manager:#x}"));
    }
    if settled {
        return;
    }

    let _ = SERVER_MGR.compare_exchange(0, manager, Ordering::SeqCst, Ordering::SeqCst);
    if TRANSFORM_INDEX.load(Ordering::Relaxed) >= 0 || edx >= 0x8000_0000 || SERVER_MGR.load(Ordering::SeqCst) != manager {
        return;
    }
    if CALIBRATION_TRIES.fetch_add(1, Ordering::Relaxed) >= MAX_CALIBRATION_TRIES {
        return;
    }
    let array = u64_at(&head, 0x18) as usize;
    let count = (u64_at(&head, 0x20) as usize).min(96);
    let Some(entries) = read_bytes(array, count * 24) else { return };
    let count = entries.len() / 24;
    if let Some(expected) = component_data(&entries, count, HASH_CURRENT_TRANSFORM, TRANSFORM_SIZE) {
        if let Some(first) = expected.get(..24) {
            calibrate(manager, ret, first);
        }
    }
}

pub fn resolve_template(text: &str) -> Option<[u8; 16]> {
    if let Some(guid) = super::config::parse_guid(text) {
        return Some(guid);
    }
    let path = std::env::current_dir().ok()?.join("export").join("m182_enemies.csv");
    let content = std::fs::read_to_string(path).ok()?;
    let wanted = text.trim();
    for line in content.lines().skip(1) {
        let mut parts = line.split(',');
        let (Some(name), Some(guid)) = (parts.next(), parts.next()) else { continue };
        if name.eq_ignore_ascii_case(wanted) {
            return super::config::parse_guid(guid);
        }
    }
    None
}

#[repr(C, align(16))]
struct Block([u8; 0x100]);

fn write_u32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn write_u64(b: &mut [u8], off: usize, v: u64) {
    b[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

fn spawn_now(manager: usize) {
    let settings = match SPAWN.lock() {
        Ok(s) => s.clone(),
        Err(_) => return,
    };
    let Some(template) = resolve_template(&settings.template) else {
        log(&format!("spawn skipped: \"{}\" is not a known enemy name or id", settings.template));
        return;
    };
    let Some(t) = player_transform() else {
        log(&format!(
            "spawn skipped: player position not available (player id={}, transform index={}, tries={})",
            PLAYER_ID.load(Ordering::Relaxed),
            TRANSFORM_INDEX.load(Ordering::Relaxed),
            CALIBRATION_TRIES.load(Ordering::Relaxed)
        ));
        return;
    };
    let base = (le_i64(&t, 0), le_i64(&t, 8), le_i64(&t, 16));
    log(&format!(
        "spawning {} ({}) x{} level {} near x={:.2} y={:.2} z={:.2}",
        settings.template,
        hex(&template),
        settings.count,
        settings.level,
        base.0 as f64 / METERS,
        base.1 as f64 / METERS,
        base.2 as f64 / METERS
    ));

    for k in 0..settings.count {
        let angle = std::f64::consts::TAU * k as f64 / settings.count as f64;
        let dx = (settings.distance as f64 * angle.cos() * METERS) as i64;
        let dz = (settings.distance as f64 * angle.sin() * METERS) as i64;
        let dy = (0.5 * METERS) as i64;

        let mut transform = [0u8; TRANSFORM_SIZE];
        write_u64(&mut transform, 0, base.0.wrapping_add(dx) as u64);
        write_u64(&mut transform, 8, base.1.wrapping_add(dy) as u64);
        write_u64(&mut transform, 16, base.2.wrapping_add(dz) as u64);
        transform[36..40].copy_from_slice(&1.0f32.to_le_bytes());
        transform[40..44].copy_from_slice(&1.0f32.to_le_bytes());
        transform[44..48].copy_from_slice(&1.0f32.to_le_bytes());
        transform[48..52].copy_from_slice(&1.0f32.to_le_bytes());
        let level = settings.level.to_le_bytes();

        let mut entries = Block([0u8; 0x100]);
        write_u32(&mut entries.0, 0, HASH_CURRENT_TRANSFORM);
        write_u64(&mut entries.0, 8, transform.as_ptr() as u64);
        write_u64(&mut entries.0, 16, TRANSFORM_SIZE as u64);
        write_u32(&mut entries.0, 24, HASH_OVERRIDE_LEVEL);
        write_u64(&mut entries.0, 32, level.as_ptr() as u64);
        write_u64(&mut entries.0, 40, 4);

        let mut desc = Block([0u8; 0x100]);
        desc.0[4..20].copy_from_slice(&template);
        write_u64(&mut desc.0, 0x18, entries.0.as_ptr() as u64);
        write_u64(&mut desc.0, 0x20, 2);

        let id = original_create(manager, 0, desc.0.as_ptr() as usize, 0);
        LAST_SPAWNED.store(id, Ordering::SeqCst);
        log(&format!("create returned entity id={id}"));
    }
}

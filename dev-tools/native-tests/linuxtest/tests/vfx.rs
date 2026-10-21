use linuxtest::native::{config, spawn_probe, spawner, vfx};
use std::sync::Mutex;
use std::sync::atomic::Ordering;

static LOCK: Mutex<()> = Mutex::new(());
static SEEN: Mutex<Vec<(usize, Vec<u8>, Vec<u8>)>> = Mutex::new(Vec::new());

extern "win64" fn fake_create(mgr: usize, _edx: u32, desc: usize, _r9: usize) -> u32 {
    let d = unsafe { std::slice::from_raw_parts(desc as *const u8, 0x30) }.to_vec();
    let array = u64::from_le_bytes(d[0x18..0x20].try_into().unwrap()) as usize;
    let count = u64::from_le_bytes(d[0x20..0x28].try_into().unwrap()) as usize;
    let comps = unsafe { std::slice::from_raw_parts(array as *const u8, count * 24) }.to_vec();
    let mut data = Vec::new();
    for i in 0..count {
        let ptr = u64::from_le_bytes(comps[i * 24 + 8..i * 24 + 16].try_into().unwrap()) as usize;
        let size = u64::from_le_bytes(comps[i * 24 + 16..i * 24 + 24].try_into().unwrap()) as usize;
        data.extend_from_slice(&comps[i * 24..i * 24 + 4]);
        data.extend_from_slice(unsafe { std::slice::from_raw_parts(ptr as *const u8, size) });
    }
    SEEN.lock().unwrap().push((mgr, d, data));
    4321
}

fn combat(abilities: u32, flags: u64) -> [u8; 24] {
    let mut b = [0u8; 24];
    b[0..4].copy_from_slice(&abilities.to_le_bytes());
    b[8..16].copy_from_slice(&flags.to_le_bytes());
    b[16..24].copy_from_slice(&123456789u64.to_le_bytes());
    b
}

const READY: u64 = 1 << 50 | 1;

#[test]
fn config_defaults_and_limits() {
    let cfg = vfx::parse_config("{}").unwrap();
    assert!(cfg.enabled);
    assert!(!cfg.charge.effects.is_empty() && !cfg.release.effects.is_empty());
    let cfg = vfx::parse_config(r#"{"enabled":false,"charge":{"effects":["a","b","c","d","e","f","g"],"scale":99,"ring":40,"radius":-3},"charge_every_ms":1}"#).unwrap();
    assert!(!cfg.enabled);
    assert_eq!(cfg.charge.effects.len(), 6);
    assert_eq!(cfg.charge.scale, 20.0);
    assert_eq!(cfg.charge.ring, 8);
    assert_eq!(cfg.charge.radius, 0.0);
    assert_eq!(cfg.charge_every_ms, 100);
    assert!(vfx::parse_config("{nope").is_err());
}

#[test]
fn combat_state_is_recognised() {
    assert_eq!(vfx::combat_flags(&combat(0x3fff, READY)), Some(READY));
    assert_eq!(vfx::combat_flags(&combat(0x3fff, 1)), None, "not ready for gameplay");
    assert_eq!(vfx::combat_flags(&combat(0x3f80_0000, READY)), None, "a float, not an ability mask");
    let mut padded = combat(1, READY);
    padded[4] = 1;
    assert_eq!(vfx::combat_flags(&padded), None);
    assert!(vfx::is_charging(READY | 1 << vfx::FLAG_HEAVY_ATTACK));
    assert!(vfx::is_charging(READY | 1 << vfx::FLAG_RANGED_PREPARING));
    assert!(!vfx::is_charging(READY));
    assert!(!vfx::is_charging(READY | 1 << vfx::FLAG_HEAVY_ATTACK | 1 << vfx::FLAG_DEAD));
}

#[test]
fn charge_repeats_and_release_fires_once() {
    let cfg = vfx::VfxConfig { charge_every_ms: 300, min_charge_ms: 300, ..Default::default() };
    let mut st = vfx::PlayerCharge::default();
    let heavy = READY | 1 << vfx::FLAG_HEAVY_ATTACK;
    assert_eq!(vfx::step(&mut st, READY, 0, &cfg), vfx::Event::None);
    assert_eq!(vfx::step(&mut st, heavy, 1000, &cfg), vfx::Event::None);
    assert_eq!(vfx::step(&mut st, heavy, 1130, &cfg), vfx::Event::Charge);
    assert_eq!(vfx::step(&mut st, heavy, 1300, &cfg), vfx::Event::None);
    assert_eq!(vfx::step(&mut st, heavy, 1430, &cfg), vfx::Event::Charge);
    assert_eq!(vfx::step(&mut st, READY, 1500, &cfg), vfx::Event::Release);
    assert_eq!(vfx::step(&mut st, READY, 1600, &cfg), vfx::Event::None);
    assert_eq!(vfx::step(&mut st, heavy, 2000, &cfg), vfx::Event::None);
    assert_eq!(vfx::step(&mut st, READY, 2100, &cfg), vfx::Event::None, "too short to count as charged");
}

#[test]
fn burst_points_form_a_ring_in_front() {
    let m = 4294967296.0f64;
    let b = vfx::Burst { effects: vec!["x".into()], scale: 2.0, height: 1.0, distance: 2.0, ring: 4, radius: 1.0 };
    let pts = vfx::burst_points((10.0 * m) as i64, (5.0 * m) as i64, (20.0 * m) as i64, (0.0, 1.0), &b);
    assert_eq!(pts.len(), 4);
    for (x, y, z) in &pts {
        let (dx, dz) = (*x as f64 / m - 10.0, *z as f64 / m - 22.0);
        assert!(((dx * dx + dz * dz).sqrt() - 1.0).abs() < 1e-6);
        assert!((*y as f64 / m - 6.0).abs() < 1e-6);
    }
    let single = vfx::Burst { ring: 1, ..b };
    assert_eq!(vfx::burst_points(0, 0, 0, (1.0, 0.0), &single), vec![((2.0 * m) as i64, (1.0 * m) as i64, 0)]);
}

struct World {
    mem: Vec<u8>,
    chunk: Vec<u8>,
    def: Vec<u8>,
    record: Vec<u8>,
    bitmap: Vec<u64>,
    keys: Vec<u32>,
    values: Vec<u64>,
}

fn put(b: &mut [u8], off: usize, v: &[u8]) {
    b[off..off + v.len()].copy_from_slice(v);
}

fn world(id: u32, slot: usize, comps: &[(usize, u16, u16, Vec<u8>)]) -> Box<World> {
    let mut w = Box::new(World {
        mem: vec![0u8; 0x400],
        chunk: vec![0u8; 0x8000],
        def: vec![0u8; 0x1800],
        record: vec![0u8; 0x200],
        bitmap: vec![0u64; 1],
        keys: vec![0u32; 16],
        values: vec![0u64; 16],
    });
    for (index, stride, offset, bytes) in comps {
        let word = (index >> 6) * 8;
        let mask = u64::from_le_bytes(w.def[word..word + 8].try_into().unwrap()) | (1u64 << (index & 63));
        put(&mut w.def, word, &mask.to_le_bytes());
        put(&mut w.def, 0x84 + index * 2, &offset.to_le_bytes());
        put(&mut w.def, 0xa84 + index * 2, &stride.to_le_bytes());
        put(&mut w.chunk, *offset as usize + *stride as usize * slot, bytes);
    }
    put(&mut w.record, 0x10, &id.to_le_bytes());
    put(&mut w.record, 0x18, &(w.def.as_ptr() as u64).to_le_bytes());
    put(&mut w.record, 0x20, &(w.chunk.as_ptr() as u64).to_le_bytes());
    put(&mut w.record, 0x30, &(slot as u32).to_le_bytes());
    let s = spawner::hash_slot(id, 16) as usize;
    w.bitmap[0] |= 1 << s;
    w.keys[s] = id;
    w.values[s] = w.record.as_ptr() as u64;
    put(&mut w.mem, 8, &[0]);
    put(&mut w.mem, 0x148 + 8, &(w.bitmap.as_ptr() as u64).to_le_bytes());
    put(&mut w.mem, 0x148 + 0x10, &16u64.to_le_bytes());
    put(&mut w.mem, 0x148 + 0x28, &(w.keys.as_ptr() as u64).to_le_bytes());
    put(&mut w.mem, 0x148 + 0x40, &(w.values.as_ptr() as u64).to_le_bytes());
    put(&mut w.mem, 0x148 + 0x54, &1u32.to_le_bytes());
    w
}

#[test]
fn finds_the_combat_component_among_lookalikes() {
    let _g = LOCK.lock().unwrap();
    let mut float_mod = [0u8; 24];
    float_mod[0..4].copy_from_slice(&1.0f32.to_le_bytes());
    float_mod[8..12].copy_from_slice(&1.0f32.to_le_bytes());
    let comps = vec![
        (3usize, 24u16, 0x100u16, float_mod.to_vec()),
        (70, 24, 0x400, vec![0u8; 24]),
        (130, 24, 0x800, combat(0x1fff, READY | 1 << vfx::FLAG_HEAVY_ATTACK).to_vec()),
        (131, 56, 0xc00, vec![1u8; 56]),
    ];
    let w = world(9, 2, &comps);
    let mgr = w.mem.as_ptr() as usize;
    let flags = vfx::read_flags(mgr, 9).unwrap();
    assert!(vfx::is_charging(flags));
    assert_eq!(vfx::read_flags(mgr, 9), Some(flags));
    assert_eq!(vfx::read_flags(mgr, 10), None);
}

#[test]
fn queued_bursts_are_created_from_the_spawn_hook_with_scale() {
    let _g = LOCK.lock().unwrap();
    let t0 = {
        let mut t = [0u8; 56];
        t[0..8].copy_from_slice(&(5i64 << 32).to_le_bytes());
        t
    };
    let w = world(1, 0, &[(5, 56, 0x200, t0.to_vec())]);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    config::LIVE.apply(&config::NativeConfig { spawner: true, ..Default::default() });
    spawn_probe::set_trampoline(fake_create as usize);
    SEEN.lock().unwrap().clear();

    let b = vfx::Burst {
        effects: vec!["VFX_Trap_Electro_Impact".into(), "VFX_Enemy_Vukah_Shaman_Lightning_Sky_Explosion".into(), "missing_effect".into()],
        scale: 4.0,
        height: 0.0,
        distance: 0.0,
        ring: 3,
        radius: 1.0,
    };
    vfx::queue_burst((0, 0, 0), (0.0, 1.0), &b);
    assert!(vfx::pending());
    spawner::before(mgr, 0);
    assert_eq!(SEEN.lock().unwrap().len(), 0, "only runtime create calls run the queue");
    spawner::before(mgr, 7);
    assert!(!vfx::pending());
    let seen = SEEN.lock().unwrap();
    assert_eq!(seen.len(), 4, "both effects at the centre point, the first one around the ring");
    let electro = config::parse_guid("5493e543-a71e-46ec-b17b-d0a5e4d1d0a5").unwrap();
    let sky = config::parse_guid("bf944b00-58cc-45a1-a8da-c30a59d8c146").unwrap();
    assert_eq!(&seen[0].1[4..20], &electro);
    assert_eq!(&seen[1].1[4..20], &sky);
    for (m, _, data) in seen.iter() {
        assert_eq!(*m, mgr);
        assert_eq!(&data[0..4], &0x027a_c564u32.to_le_bytes());
        for off in [40, 44, 48] {
            assert_eq!(f32::from_le_bytes(data[4 + off..8 + off].try_into().unwrap()), 4.0);
        }
        assert_eq!(f32::from_le_bytes(data[4 + 36..8 + 36].try_into().unwrap()), 1.0);
    }
}

#[test]
fn server_tick_creates_queued_effects_without_waiting_for_the_game() {
    let _g = LOCK.lock().unwrap();
    let w = world(1, 0, &[(5, 56, 0x200, vec![0u8; 56])]);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawn_probe::set_trampoline(fake_create as usize);
    SEEN.lock().unwrap().clear();
    vfx::server_tick();
    assert_eq!(SEEN.lock().unwrap().len(), 0);
    let b = vfx::Burst { effects: vec!["VFX_Trap_Electro_Impact".into()], scale: 2.0, height: 0.0, distance: 0.0, ring: 1, radius: 0.0 };
    vfx::queue_burst((1 << 32, 2 << 32, 3 << 32), (0.0, 1.0), &b);
    vfx::server_tick();
    assert!(!vfx::pending());
    let seen = SEEN.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(i64::from_le_bytes(seen[0].2[4..12].try_into().unwrap()), 1 << 32);
    assert_eq!(i64::from_le_bytes(seen[0].2[20..28].try_into().unwrap()), 3 << 32);
}

static PUMPED: Mutex<Vec<(usize, usize, usize, usize)>> = Mutex::new(Vec::new());

extern "win64" fn fake_pump(rcx: usize, rdx: usize, r8: usize, r9: usize) -> usize {
    PUMPED.lock().unwrap().push((rcx, rdx, r8, r9));
    0xabc
}

#[test]
fn pump_hook_keeps_arguments_and_runs_the_queue() {
    use linuxtest::native::patch;
    let _g = LOCK.lock().unwrap();
    let w = world(1, 0, &[(5, 56, 0x200, vec![0u8; 56])]);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawn_probe::set_trampoline(fake_create as usize);
    SEEN.lock().unwrap().clear();
    PUMPED.lock().unwrap().clear();
    patch::PUMP_TRAMPOLINE.store(fake_pump as usize, Ordering::SeqCst);
    let b = vfx::Burst { effects: vec!["VFX_Trap_Electro_Impact".into()], scale: 2.0, height: 0.0, distance: 0.0, ring: 1, radius: 0.0 };
    vfx::queue_burst((0, 0, 0), (0.0, 1.0), &b);
    let stub: extern "win64" fn(usize, usize, usize, usize) -> usize = unsafe { std::mem::transmute(patch::stub_pump as usize) };
    let r = stub(0x1111, 0x2222, 0x3333, 0x4444);
    assert_eq!(r, 0xabc);
    assert_eq!(PUMPED.lock().unwrap().as_slice(), &[(0x1111, 0x2222, 0x3333, 0x4444)]);
    assert_eq!(patch::PLAYERS_OBJECT.load(Ordering::SeqCst), 0x1111);
    assert_eq!(SEEN.lock().unwrap().len(), 1);
    let r = stub(0x5555, 1, 2, 3);
    assert_eq!(r, 0xabc);
    assert_eq!(SEEN.lock().unwrap().len(), 1);
}

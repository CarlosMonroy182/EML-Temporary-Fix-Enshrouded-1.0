use linuxtest::native::{config, faction, groups, spawn_probe, spawner};
use std::sync::atomic::Ordering;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());
static SEEN: Mutex<Vec<(usize, u32, Vec<u8>, Vec<u8>)>> = Mutex::new(Vec::new());

extern "win64" fn fake_create(mgr: usize, edx: u32, desc: usize, _r9: usize) -> u32 {
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
    SEEN.lock().unwrap().push((mgr, edx, d, data));
    777
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

fn world(id: u32, slot: u32, transform: &[u8; 56]) -> Box<World> {
    let mut w = Box::new(World {
        mem: vec![0u8; 0x400],
        chunk: vec![0u8; 0x4000],
        def: vec![0u8; 0x1800],
        record: vec![0u8; 0x200],
        bitmap: vec![0u64; 1],
        keys: vec![0u32; 16],
        values: vec![0u64; 16],
    });
    for (index, stride, offset) in [(5usize, 56u16, 0x200u16), (7, 56, 0x900), (9, 24, 0x1400)] {
        let mask = u64::from_le_bytes(w.def[0..8].try_into().unwrap()) | (1u64 << index);
        put(&mut w.def, 0, &mask.to_le_bytes());
        put(&mut w.def, 0x84 + index * 2, &offset.to_le_bytes());
        put(&mut w.def, 0xa84 + index * 2, &stride.to_le_bytes());
    }
    let at5 = 0x200 + 56 * slot as usize;
    put(&mut w.chunk, at5, transform);
    let at7 = 0x900 + 56 * slot as usize;
    let mut other = *transform;
    other[0] ^= 0xff;
    put(&mut w.chunk, at7, &other);

    put(&mut w.record, 0x10, &id.to_le_bytes());
    put(&mut w.record, 0x18, &(w.def.as_ptr() as u64).to_le_bytes());
    put(&mut w.record, 0x20, &(w.chunk.as_ptr() as u64).to_le_bytes());
    put(&mut w.record, 0x30, &slot.to_le_bytes());

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

fn transform(x: i64, y: i64, z: i64) -> [u8; 56] {
    let mut t = [0u8; 56];
    t[0..8].copy_from_slice(&x.to_le_bytes());
    t[8..16].copy_from_slice(&y.to_le_bytes());
    t[16..24].copy_from_slice(&z.to_le_bytes());
    t[36..40].copy_from_slice(&1.0f32.to_le_bytes());
    t[40..52].copy_from_slice(&[0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63]);
    t
}

#[test]
fn guid_and_key_parsing() {
    let g = config::parse_guid("c3db66d6-89a5-4a6c-a8e3-d9a1f6009812").unwrap();
    assert_eq!(hex(&g), "d666dbc3a5896c4aa8e3d9a1f6009812");
    assert!(config::parse_guid("nope").is_none());
    assert_eq!(config::parse_key("F8"), Some(0x77));
    assert_eq!(config::parse_key("f1"), Some(0x70));
    assert_eq!(config::parse_key("K"), Some(0x4b));
    assert_eq!(config::parse_key("119"), Some(119));
    assert!(config::parse_key("zz").is_none());
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn calibrates_reads_and_spawns() {
    let _g = LOCK.lock().unwrap();
    SEEN.lock().unwrap().clear();
    let t0 = transform(5i64 << 32, 7i64 << 32, -(9i64 << 32));
    let w = world(1, 3, &t0);
    let mgr = w.mem.as_ptr() as usize;

    assert!(spawner::find_record(mgr, 1).is_some());
    assert!(spawner::find_record(mgr, 2).is_none());
    assert!(spawner::calibrate(mgr, 1, &t0[..24]));
    assert_eq!(spawner::TRANSFORM_INDEX.load(Ordering::SeqCst), 5);

    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    let got = spawner::player_transform().unwrap();
    assert_eq!(got, t0.to_vec());

    config::LIVE.apply(&config::NativeConfig { spawner: true, spawn_count: 2, spawn_level: 33, spawn_distance: 10.0, ..Default::default() });
    spawn_probe::set_trampoline(fake_create as usize);
    spawner::spawn_now(mgr);

    let seen = SEEN.lock().unwrap();
    assert_eq!(seen.len(), 2);
    for (m, edx, desc, data) in seen.iter() {
        assert_eq!(*m, mgr);
        assert_eq!(*edx, 0);
        assert_eq!(&desc[4..20], &config::parse_guid(config::DEFAULT_TEMPLATE).unwrap());
        assert_eq!(&data[0..4], &0x027ac564u32.to_le_bytes());
        let x = i64::from_le_bytes(data[4..12].try_into().unwrap());
        let y = i64::from_le_bytes(data[12..20].try_into().unwrap());
        let z = i64::from_le_bytes(data[20..28].try_into().unwrap());
        let dx = (x - (5i64 << 32)) as f64 / 4294967296.0;
        let dz = (z + (9i64 << 32)) as f64 / 4294967296.0;
        assert!(((dx * dx + dz * dz).sqrt() - 10.0).abs() < 0.01);
        assert_eq!(y - (7i64 << 32), (0.5 * 4294967296.0) as i64);
        assert_eq!(&data[4 + 56..4 + 56 + 4], &0xa21e7ee7u32.to_le_bytes());
        assert_eq!(&data[4 + 56 + 4..4 + 56 + 8], &33u32.to_le_bytes());
    }
}

#[test]
fn calibration_fails_cleanly() {
    let _g = LOCK.lock().unwrap();
    let t0 = transform(1 << 32, 2 << 32, 3 << 32);
    let w = world(1, 0, &t0);
    let mgr = w.mem.as_ptr() as usize;
    let mut wrong = t0;
    wrong[0] ^= 1;
    spawner::TRANSFORM_INDEX.store(-1, Ordering::SeqCst);
    assert!(!spawner::calibrate(mgr, 1, &wrong[..24]));
    assert!(!spawner::calibrate(mgr, 99, &t0[..24]));
    assert_eq!(spawner::TRANSFORM_INDEX.load(Ordering::SeqCst), -1);
}

#[test]
fn before_only_fires_on_runtime_calls() {
    let _g = LOCK.lock().unwrap();
    let t0 = transform(5i64 << 32, 7i64 << 32, 3i64 << 32);
    let w = world(1, 2, &t0);
    let mgr = w.mem.as_ptr() as usize;
    assert!(spawner::calibrate(mgr, 1, &t0[..24]));
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    config::LIVE.apply(&config::NativeConfig { spawner: true, ..Default::default() });
    spawn_probe::set_trampoline(fake_create as usize);
    SEEN.lock().unwrap().clear();

    spawner::PENDING.store(1, Ordering::SeqCst);
    spawner::before(mgr, 0);
    spawner::before(mgr, 0x8000_0001);
    spawner::before(mgr + 0x1000, 7);
    assert_eq!(SEEN.lock().unwrap().len(), 0);
    assert_eq!(spawner::PENDING.load(Ordering::SeqCst), 1);

    spawner::before(mgr, 7);
    assert_eq!(SEEN.lock().unwrap().len(), 1);
    assert_eq!(spawner::PENDING.load(Ordering::SeqCst), 0);
    spawner::before(mgr, 8);
    assert_eq!(SEEN.lock().unwrap().len(), 1);
}

#[test]
fn resolves_names_from_export_csv() {
    let _g = LOCK.lock().unwrap();
    std::fs::create_dir_all("export").unwrap();
    std::fs::write(
        "export/m182_enemies.csv",
        "name,guid,faction,boss,preset\nEnemy_A,642098d4-fc09-48e2-811e-782d438bf551,Drak,false,\nEnemy_A@x10,642098d4-fc09-4182-0002-782d438bf551,Drak,false,x10\n",
    )
    .unwrap();
    assert_eq!(hex(&spawner::resolve_template("enemy_a").unwrap()), "d498206409fce248811e782d438bf551");
    assert_eq!(hex(&spawner::resolve_template("Enemy_A@x10").unwrap()), "d498206409fc82410002782d438bf551");
    assert!(spawner::resolve_template("Enemy_B").is_none());
    assert!(spawner::resolve_template("c3db66d6-89a5-4a6c-a8e3-d9a1f6009812").is_some());
}

#[test]
fn dump_lists_components() {
    let _g = LOCK.lock().unwrap();
    let t0 = transform(1 << 32, 2 << 32, 3 << 32);
    let w = world(1, 1, &t0);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::dump_entity("player", mgr, 1);
    spawner::dump_entity("player", mgr, 99);
    let log = std::fs::read_to_string("m182_spawner.log").unwrap();
    assert!(log.contains("idx=5 size=56"));
    assert!(log.contains("idx=9 size=24"));
    assert!(log.contains("entity 99 not available"));
    assert!(log.contains("end"));
}

#[test]
fn apply_stats_sets_health_and_damage() {
    let _g = LOCK.lock().unwrap();
    let t0 = transform(1 << 32, 2 << 32, 3 << 32);
    let mut w = world(1, 3, &t0);
    for (index, stride, offset) in [(11usize, 40u16, 0x1800u16), (12, 24, 0x1c00), (13, 40, 0x2000)] {
        let mut mask = u64::from_le_bytes(w.def[0..8].try_into().unwrap());
        mask |= 1u64 << index;
        put(&mut w.def, 0, &mask.to_le_bytes());
        put(&mut w.def, 0x84 + index * 2, &offset.to_le_bytes());
        put(&mut w.def, 0xa84 + index * 2, &stride.to_le_bytes());
    }
    let health_at = 0x1800 + 40 * 3;
    let mut health = [0u8; 40];
    health[0..4].copy_from_slice(&0x8eb8_4995u32.to_le_bytes());
    for off in [8usize, 16, 36] {
        health[off..off + 4].copy_from_slice(&1031u32.to_le_bytes());
    }
    put(&mut w.chunk, health_at, &health);
    let damage_at = 0x1c00 + 24 * 3;
    let mut damage = [0u8; 24];
    damage[0..4].copy_from_slice(&0xb866_36b9u32.to_le_bytes());
    damage[8..12].copy_from_slice(&1.0f32.to_le_bytes());
    damage[12..16].copy_from_slice(&2.0f32.to_le_bytes());
    put(&mut w.chunk, damage_at, &damage);
    let decoy_at = 0x2000 + 40 * 3;
    put(&mut w.chunk, decoy_at, &[7u8; 40]);

    let mgr = w.mem.as_ptr() as usize;
    spawner::apply_stats(mgr, 1, 50000, 3.0);
    for off in [8usize, 16, 36] {
        assert_eq!(u32::from_le_bytes(w.chunk[health_at + off..health_at + off + 4].try_into().unwrap()), 50000);
    }
    assert_eq!(f32::from_le_bytes(w.chunk[damage_at + 8..damage_at + 12].try_into().unwrap()), 3.0);
    assert_eq!(f32::from_le_bytes(w.chunk[damage_at + 12..damage_at + 16].try_into().unwrap()), 6.0);
    assert_eq!(w.chunk[decoy_at..decoy_at + 40], [7u8; 40]);

    spawner::apply_stats(mgr, 1, 0, 1.0);
    assert_eq!(u32::from_le_bytes(w.chunk[health_at + 8..health_at + 12].try_into().unwrap()), 50000);
    spawner::apply_stats(mgr, 99, 10, 2.0);
}

#[test]
fn watch_reports_changes_but_not_noise() {
    let _g = LOCK.lock().unwrap();
    let t0 = transform(1 << 32, 2 << 32, 3 << 32);
    let mut w = world(1, 0, &t0);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    let noise_at = 0x1400;
    let still_at = 0x200 + 7;
    let base = w.chunk.as_mut_ptr() as usize;
    spawner::toggle_watch();
    for i in 0..170u8 {
        unsafe { *((base + noise_at) as *mut u8) = i };
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    unsafe { *((base + still_at) as *mut u8) = 0x77 };
    for i in 0..30u8 {
        unsafe { *((base + noise_at) as *mut u8) = i };
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    spawner::toggle_watch();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let log = std::fs::read_to_string("m182_spawner.log").unwrap();
    assert!(log.contains("watch ready"));
    assert!(log.contains("watch server idx=5"));
    assert!(!log.contains("watch server idx=9"));
    let _ = &mut w;
}

#[test]
fn panel_request_overrides_settings_once() {
    let _g = LOCK.lock().unwrap();
    let t0 = transform(5i64 << 32, 7i64 << 32, 3i64 << 32);
    let w = world(1, 2, &t0);
    let mgr = w.mem.as_ptr() as usize;
    assert!(spawner::calibrate(mgr, 1, &t0[..24]));
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    config::LIVE.apply(&config::NativeConfig { spawner: true, spawn_count: 1, ..Default::default() });
    spawn_probe::set_trampoline(fake_create as usize);
    SEEN.lock().unwrap().clear();

    std::fs::write(
        "m182_spawn.request",
        r#"{"template":"c3db66d6-89a5-4a6c-a8e3-d9a1f6009812","level":33,"count":3,"distance":9.0,"health":777,"damage":2.0}"#,
    )
    .unwrap();
    spawner::poll_request();
    assert!(!std::path::Path::new("m182_spawn.request").exists());
    assert_eq!(spawner::PENDING.load(Ordering::SeqCst), 1);
    spawner::before(mgr, 7);
    assert_eq!(SEEN.lock().unwrap().len(), 3);
    spawner::PENDING.store(1, Ordering::SeqCst);
    spawner::before(mgr, 7);
    assert_eq!(SEEN.lock().unwrap().len(), 4);

    std::fs::write("m182_spawn.request", "not json").unwrap();
    spawner::PENDING.store(0, Ordering::SeqCst);
    spawner::poll_request();
    assert_eq!(spawner::PENDING.load(Ordering::SeqCst), 0);
}

#[test]
fn census_reports_new_entities() {
    let _g = LOCK.lock().unwrap();
    let t0 = transform(1 << 32, 2 << 32, 3 << 32);
    let w = world(1, 0, &t0);
    let mgr = w.mem.as_ptr() as usize;
    let ids = spawner::entity_ids(mgr).unwrap();
    assert!(ids.contains(&1) && ids.len() == 1);
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::CLIENT_MGR.store(0, Ordering::SeqCst);
    spawner::census();
    spawner::census();
    let log = std::fs::read_to_string("m182_spawner.log").unwrap();
    assert!(log.contains("census saved"));
    assert!(log.contains("census server: 0 new, 0 removed"));
}

#[test]
fn panel_request_can_target_a_place() {
    let _g = LOCK.lock().unwrap();
    let t0 = transform(5i64 << 32, 7i64 << 32, 3i64 << 32);
    let w = world(1, 2, &t0);
    let mgr = w.mem.as_ptr() as usize;
    assert!(spawner::calibrate(mgr, 1, &t0[..24]));
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    config::LIVE.apply(&config::NativeConfig { spawner: true, ..Default::default() });
    spawn_probe::set_trampoline(fake_create as usize);
    SEEN.lock().unwrap().clear();
    std::fs::write(
        "m182_spawn.request",
        r#"{"template":"c3db66d6-89a5-4a6c-a8e3-d9a1f6009812","count":1,"at":[100.5,20.0,-30.25]}"#,
    )
    .unwrap();
    spawner::poll_request();
    spawner::before(mgr, 7);
    let seen = SEEN.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let data = &seen[0].3;
    let x = i64::from_le_bytes(data[4..12].try_into().unwrap());
    let z = i64::from_le_bytes(data[20..28].try_into().unwrap());
    assert_eq!(x, (100.5 * 4294967296.0) as i64);
    assert_eq!(z, (-30.25 * 4294967296.0) as i64);
    drop(seen);
    spawner::write_position((3 << 32, 4 << 32, 5 << 32));
    let text = std::fs::read_to_string("m182_player.json").unwrap();
    assert_eq!(text, "{\"x\":3.00,\"y\":4.00,\"z\":5.00}");
}

fn rotated_transform(x: i64, z: i64, yaw: f32) -> [u8; 56] {
    let mut t = transform(x, 0, z);
    let (s, c) = ((yaw / 2.0).sin(), (yaw / 2.0).cos());
    t[24..28].copy_from_slice(&0f32.to_le_bytes());
    t[28..32].copy_from_slice(&s.to_le_bytes());
    t[32..36].copy_from_slice(&0f32.to_le_bytes());
    t[36..40].copy_from_slice(&c.to_le_bytes());
    t
}

#[test]
fn facing_is_learned_from_movement_and_used_for_forward_spawns() {
    let _g = LOCK.lock().unwrap();
    let m = 4294967296i64;
    let yaw = std::f32::consts::FRAC_PI_2;
    for step in 0..4 {
        let a = (10 * m, 20 * m, 0i64);
        let b = (10 * m, 20 * m, 0i64);
        let _ = (a, b);
        let from = (10 * m + step * 5 * m, 0, 20 * m);
        let to = (from.0 + 5 * m, 0, 20 * m);
        let t = rotated_transform(to.0, to.2, yaw);
        spawner::facing_update(from, to, &t);
    }
    let t = rotated_transform(0, 0, yaw);
    let (fx, fz) = spawner::facing_direction(&t);
    assert!(fx > 0.99 && fz.abs() < 0.01, "got {fx} {fz}");

    let t0 = rotated_transform(5i64 << 32, 3i64 << 32, yaw);
    let w = world(1, 2, &t0);
    let mgr = w.mem.as_ptr() as usize;
    assert!(spawner::calibrate(mgr, 1, &t0[..24]));
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    config::LIVE.apply(&config::NativeConfig { spawner: true, ..Default::default() });
    spawn_probe::set_trampoline(fake_create as usize);
    SEEN.lock().unwrap().clear();
    std::fs::write(
        "m182_spawn.request",
        r#"{"template":"c3db66d6-89a5-4a6c-a8e3-d9a1f6009812","count":1,"distance":10.0,"forward":true,"object":true}"#,
    )
    .unwrap();
    spawner::poll_request();
    spawner::before(mgr, 7);
    let seen = SEEN.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let data = &seen[0].3;
    assert_eq!(data.len(), 4 + 56, "object spawns carry only the transform");
    let x = i64::from_le_bytes(data[4..12].try_into().unwrap());
    let z = i64::from_le_bytes(data[20..28].try_into().unwrap());
    assert!((x as f64 / 4294967296.0 - 15.0).abs() < 0.05);
    assert!((z as f64 / 4294967296.0 - 3.0).abs() < 0.05);
}

fn add_component(w: &mut World, index: usize, stride: u16, offset: u16) {
    let mut mask = u64::from_le_bytes(w.def[0..8].try_into().unwrap());
    mask |= 1u64 << index;
    put(&mut w.def, 0, &mask.to_le_bytes());
    put(&mut w.def, 0x84 + index * 2, &offset.to_le_bytes());
    put(&mut w.def, 0xa84 + index * 2, &stride.to_le_bytes());
}

fn resource_bytes(sig: u32, words: [u32; 8]) -> [u8; 40] {
    let mut b = [0u8; 40];
    b[0..4].copy_from_slice(&sig.to_le_bytes());
    b[4..8].copy_from_slice(&[8, 0, 1, 8]);
    for (i, v) in words.iter().enumerate() {
        b[8 + i * 4..12 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    b
}

fn word(w: &World, at: usize, off: usize) -> u32 {
    u32::from_le_bytes(w.chunk[at + off..at + off + 4].try_into().unwrap())
}

fn i64_at(w: &World, at: usize, off: usize) -> i64 {
    i64::from_le_bytes(w.chunk[at + off..at + off + 8].try_into().unwrap())
}

#[test]
fn powers_refill_resources_and_move_the_player() {
    let _g = LOCK.lock().unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    add_component(&mut w, 11, 40, 0x1800);
    add_component(&mut w, 12, 40, 0x1a00);
    add_component(&mut w, 13, 40, 0x1c00);
    add_component(&mut w, 14, 56, 0x1e00);
    add_component(&mut w, 15, 4, 0x2000);
    let health_at = 0x1800 + 40 * 3;
    let stamina_at = 0x1a00 + 40 * 3;
    let mana_at = 0x1c00 + 40 * 3;
    let copy_at = 0x1e00 + 56 * 3;
    let fog_at = 0x2000 + 4 * 3;
    put(&mut w.chunk, health_at, &resource_bytes(0x8eb8_4995, [500, 0, 1200, 873, 100, 800, 45, 2073]));
    put(&mut w.chunk, stamina_at, &resource_bytes(0x04b6_aa8b, [100, 0, 100, 2000, 500, 107, 720, 121]));
    put(&mut w.chunk, mana_at, &resource_bytes(0x6aa2_574e, [10, 10, 15, 800, 500, 0, 300, 61]));
    put(&mut w.chunk, copy_at, &t0);
    put(&mut w.chunk, fog_at, &404u32.to_le_bytes());

    let mgr = w.mem.as_ptr() as usize;
    assert!(spawner::refill_health(mgr, 1));
    assert_eq!(word(&w, health_at, 8), 2073);
    assert_eq!(word(&w, health_at, 16), 1200);

    let mut peak = 841u32;
    assert!(spawner::refill_stamina(mgr, 1, &mut peak));
    assert_eq!(word(&w, stamina_at, 8), 841);
    assert_eq!(word(&w, stamina_at, 16), 841);
    let mut mana_peak = 0u32;
    assert!(spawner::refill_mana(mgr, 1, &mut mana_peak));
    assert_eq!(word(&w, mana_at, 8), 300);
    assert_eq!(word(&w, mana_at, 12), 300);

    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    assert!(spawner::set_player_position(mgr, 1, [Some(111 << 32), None, Some(222 << 32)]));
    let main_at = 0x200 + 56 * 3;
    assert_eq!(i64_at(&w, main_at, 0), 111 << 32);
    assert_eq!(i64_at(&w, main_at, 8), 20 << 32);
    assert_eq!(i64_at(&w, main_at, 16), 222 << 32);
    assert_eq!(i64_at(&w, copy_at, 0), 111 << 32);
    assert_eq!(i64_at(&w, copy_at, 16), 222 << 32);
    let unrelated_at = 0x900 + 56 * 3;
    assert_ne!(i64_at(&w, unrelated_at, 0), 111 << 32);
}

#[test]
fn power_tick_reads_files_and_applies_toggles() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_powers_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    add_component(&mut w, 11, 40, 0x1800);
    add_component(&mut w, 12, 40, 0x1a00);
    add_component(&mut w, 15, 4, 0x2000);
    let health_at = 0x1800 + 40 * 3;
    let stamina_at = 0x1a00 + 40 * 3;
    let fog_at = 0x2000 + 4 * 3;
    put(&mut w.chunk, health_at, &resource_bytes(0x8eb8_4995, [5, 0, 1200, 873, 100, 800, 45, 2073]));
    put(&mut w.chunk, stamina_at, &resource_bytes(0x04b6_aa8b, [720, 0, 720, 2000, 500, 107, 720, 0]));
    put(&mut w.chunk, fog_at, &404u32.to_le_bytes());

    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);

    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_powers.json"), r#"{"god":true,"stamina":true,"fly":false}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 0);
    assert_eq!(word(&w, health_at, 8), 2073);
    assert_eq!(word(&w, stamina_at, 8), 720);
    w.chunk[stamina_at + 8..stamina_at + 12].copy_from_slice(&100u32.to_le_bytes());
    spawner::power_tick(&mut st, &dir, 1);
    assert_eq!(word(&w, stamina_at, 8), 720);
    spawner::power_tick(&mut st, &dir, 25);
    assert_eq!(word(&w, fog_at, 0), 404);

    std::fs::write(dir.join("m182_powers.json"), r#"{"god":false,"stamina":false}"#).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    std::fs::write(dir.join("m182_powers.json"), r#"{"god":false,"stamina":false,"fly_speed":5}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 50);
    assert_eq!(word(&w, fog_at, 0), 404);

    put(&mut w.chunk, health_at, &resource_bytes(0x8eb8_4995, [5, 0, 1200, 873, 100, 800, 45, 2073]));
    std::fs::write(dir.join("m182_command.request"), r#"{"cmd":"heal"}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 70);
    assert_eq!(word(&w, health_at, 8), 2073);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn delivery_request_creates_one_pickup_at_the_players_feet_with_the_item() {
    let _g = LOCK.lock().unwrap();
    let t0 = transform(5i64 << 32, 7i64 << 32, 3i64 << 32);
    let w = world(1, 2, &t0);
    let mgr = w.mem.as_ptr() as usize;
    assert!(spawner::calibrate(mgr, 1, &t0[..24]));
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    config::LIVE.apply(&config::NativeConfig { spawner: true, spawn_count: 1, ..Default::default() });
    spawn_probe::set_trampoline(fake_create as usize);
    SEEN.lock().unwrap().clear();
    std::fs::write(
        "m182_spawn.request",
        r#"{"template":"c15a5c9d-4956-49ff-9d72-9f522229cf1c","count":5,"distance":9.0,"forward":true,"item":3043209912,"item_guid":"cf270d5e-eff3-4418-a6d3-6e2c06b1d6ae","item_level":40,"item_rarity":4,"item_amount":250}"#,
    )
    .unwrap();
    spawner::poll_request();
    spawner::before(mgr, 7);
    let seen = SEEN.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let (_, _, desc, data) = &seen[0];
    assert_eq!(&desc[4..20], &config::parse_guid("c15a5c9d-4956-49ff-9d72-9f522229cf1c").unwrap());
    let x = i64::from_le_bytes(data[4..12].try_into().unwrap());
    let z = i64::from_le_bytes(data[20..28].try_into().unwrap());
    assert_eq!(x, 5i64 << 32);
    assert_eq!(z, 3i64 << 32);
    let at = 4 + 56;
    assert_eq!(&data[at..at + 4], &0x0877_5a90u32.to_le_bytes());
    let p = &data[at + 4..at + 4 + 28];
    assert_eq!(&p[0..4], &3_043_209_912u32.to_le_bytes());
    assert_eq!(&p[4..20], &config::parse_guid("cf270d5e-eff3-4418-a6d3-6e2c06b1d6ae").unwrap());
    assert_eq!(&p[20..24], &40u32.to_le_bytes());
    assert_eq!(p[24], 4);
    assert_eq!(&p[26..28], &250u16.to_le_bytes());
}

#[test]
fn fly_keeps_height_and_switches_gravity_off_and_back() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_fly_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    add_component(&mut w, 16, 20, 0x2400);
    let grav_at = 0x2400 + 20 * 3;
    let mut grav = [0u8; 20];
    grav[4..8].copy_from_slice(&(-1.0f32).to_le_bytes());
    grav[12..16].copy_from_slice(&9.81f32.to_le_bytes());
    grav[16] = 1;
    put(&mut w.chunk, grav_at, &grav);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_powers.json"), r#"{"fly":true,"fly_speed":30}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 0);
    assert_eq!(w.chunk[grav_at + 16], 0);
    let main_at = 0x200 + 56 * 3;
    assert_eq!(i64_at(&w, main_at, 8), 20 << 32);
    // the game pulls the player down; the next tick puts the height back
    put(&mut w.chunk, main_at + 8, &((20i64 << 32) - 5_000_000).to_le_bytes());
    spawner::power_tick(&mut st, &dir, 1);
    assert_eq!(i64_at(&w, main_at, 8), 20 << 32);
    // natural horizontal movement is amplified
    let x0 = i64_at(&w, main_at, 0);
    put(&mut w.chunk, main_at, &(x0 + (METER / 10)).to_le_bytes());
    spawner::power_tick(&mut st, &dir, 2);
    assert!(i64_at(&w, main_at, 0) > x0 + METER / 10 * 2);
    std::fs::write(dir.join("m182_powers.json"), r#"{"fly":false}"#).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    std::fs::write(dir.join("m182_powers.json"), r#"{"fly":false,"fly_speed":5}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 10);
    assert_eq!(w.chunk[grav_at + 16], 1);
}

#[test]
fn fly_forces_the_flying_locomotion_state_and_leaves_other_components_alone() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_fly_state_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    add_component(&mut w, 17, 456, 0x2800);
    add_component(&mut w, 18, 64, 0x3200);
    let loco_at = 0x2800 + 456 * 3;
    let net_at = 0x3200 + 64 * 3;
    let mut loco = [0u8; 456];
    loco[13..17].copy_from_slice(&1.0f32.to_le_bytes());
    loco[89] = 2;
    loco[90] = 2;
    put(&mut w.chunk, loco_at, &loco);
    let mut net = [0u8; 64];
    net[4..8].copy_from_slice(&1.0f32.to_le_bytes());
    net[12..16].copy_from_slice(&1.0f32.to_le_bytes());
    net[36] = 2;
    put(&mut w.chunk, net_at, &net);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_powers.json"), r#"{"fly":true,"fly_speed":10}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 0);
    assert_eq!(w.chunk[loco_at + 89], 3);
    assert_eq!(w.chunk[loco_at + 90], 3);
    assert_eq!(w.chunk[net_at + 36], 3);
    // the game decides "falling" again on its next frame
    w.chunk[loco_at + 89] = 2;
    w.chunk[net_at + 36] = 2;
    spawner::power_tick(&mut st, &dir, 1);
    assert_eq!(w.chunk[loco_at + 89], 3);
    assert_eq!(w.chunk[net_at + 36], 3);
    assert_eq!(w.chunk[loco_at + 225], 0);
    assert_eq!(w.chunk[loco_at + 88], 0);
}

#[test]
fn fly_ignores_components_that_do_not_look_like_locomotion() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_fly_noloco_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    add_component(&mut w, 17, 456, 0x2800);
    let loco_at = 0x2800 + 456 * 3;
    let mut junk = [0x77u8; 456];
    junk[89] = 200;
    put(&mut w.chunk, loco_at, &junk);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_powers.json"), r#"{"fly":true,"fly_speed":10}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 0);
    assert_eq!(w.chunk[loco_at + 89], 200);
    assert_eq!(w.chunk[loco_at + 90], 0x77);
}

const METER: i64 = 1 << 32;

#[test]
fn item_stats_command_patches_damage_integers_and_level_range() {
    let _g = LOCK.lock().unwrap();
    let guid = config::parse_guid("12345678-1234-4abc-8def-0123456789ab").unwrap();
    let item: u32 = 3_000_000_002;
    let mut big = vec![0u8; 0x8000];
    let start = big.as_ptr() as usize;
    let shift = ((start + 7) & !7) - start;
    let base = shift + 64;
    big[base..base + 4].copy_from_slice(&item.to_le_bytes());
    big[base + 4..base + 20].copy_from_slice(&guid);
    let mut dmg = |floats: [f32; 11], ints: [u32; 4]| -> Vec<u8> {
        let mut v: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
        v.extend(ints.iter().flat_map(|i| i.to_le_bytes()));
        v
    };
    let old = dmg([1.0, 1.5, 0.0, 0.0, 0.0, 296.0, 0.0, 0.0, 181.0, 0.0, 0.0], [63, 76, 98, 5]);
    let new = dmg([1.0, 9.0, 0.0, 0.0, 0.0, 296.0, 0.0, 0.0, 999.0, 0.0, 0.0], [63, 76, 98, 5]);
    big[base + 1660..base + 1720].copy_from_slice(&old);
    let lvl_old: Vec<u8> = [3u32, 20].iter().flat_map(|x| x.to_le_bytes()).collect();
    let lvl_new: Vec<u8> = [3u32, 255].iter().flat_map(|x| x.to_le_bytes()).collect();
    big[base + 1652..base + 1660].copy_from_slice(&lvl_old);
    let regions = vec![(start + shift, start + shift + 0x6000)];
    let blocks = vec![(1660usize, new.clone(), vec![old.clone()]), (1652usize, lvl_new.clone(), vec![lvl_old.clone()])];
    let (done, notes) = spawner::patch_item_struct(&guid, item, &blocks, &regions);
    assert_eq!(done.len(), 2, "{notes}");
    assert_eq!(&big[base + 1660..base + 1720], &new[..]);
    assert_eq!(&big[base + 1652..base + 1660], &lvl_new[..]);
    // a block that only matches the wrong encoding is refused, never forced
    let wrong: Vec<u8> = [1.0f32, 1.5, 0.0, 0.0, 0.0, 296.0, 0.0, 0.0, 181.0, 0.0, 0.0, 63.0, 76.0, 98.0, 5.0].iter().flat_map(|f| f.to_le_bytes()).collect();
    let (none, notes) = spawner::patch_item_struct(&guid, item, &[(1660usize, new.clone(), vec![wrong])], &regions);
    assert_eq!(none.len(), 1, "{notes}"); // already holds the wanted bytes
}

#[test]
fn zones_heal_players_inside_and_teams_map_faction_bytes() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_groups_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    add_component(&mut w, 11, 40, 0x1800);
    add_component(&mut w, 16, 1, 0x2100);
    let health_at = 0x1800 + 40 * 3;
    let faction_at = 0x2100 + 3;
    put(&mut w.chunk, health_at, &resource_bytes(0x8eb8_4995, [500, 0, 1200, 873, 100, 800, 45, 2073]));
    w.chunk[faction_at] = 7;
    let mgr = w.mem.as_ptr() as usize;
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    spawner::track_player(mgr, 1);

    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_zones.json"), r#"{"zones":[{"name":"Base","x":200,"z":200,"radius":20}]}"#).unwrap();
    std::fs::write(dir.join("m182_teams.json"), r#"{"teams":[{"name":"A","members":[1]}]}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 4);
    assert_eq!(word(&w, health_at, 8), 500);
    let players = std::fs::read_to_string(dir.join("m182_players.json")).unwrap();
    assert!(players.contains("\"n\":1") && players.contains("\"x\":10.0") && players.contains("\"z\":30.0"), "{players}");
    let addr = w.chunk.as_ptr() as usize + faction_at;
    assert_eq!(groups::team_of(addr), 1);

    std::fs::write(dir.join("m182_zones.json"), r#"{"zones":[{"name":"Aqui","x":12,"z":28,"radius":10}]}"#).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));
    spawner::power_tick(&mut st, &dir, 8);
    assert_eq!(word(&w, health_at, 8), 2073);

    groups::set_team_map(Vec::new());
    let _ = std::fs::remove_dir_all(dir);
}


fn second_entity(w: &mut World, id: u32, slot: u32, transform: &[u8; 56]) {
    w.record.resize(0x200, 0);
    let base = 0x100;
    put(&mut w.chunk, 0x200 + 56 * slot as usize, transform);
    put(&mut w.record, base + 0x10, &id.to_le_bytes());
    put(&mut w.record, base + 0x18, &(w.def.as_ptr() as u64).to_le_bytes());
    put(&mut w.record, base + 0x20, &(w.chunk.as_ptr() as u64).to_le_bytes());
    put(&mut w.record, base + 0x30, &slot.to_le_bytes());
    let s = spawner::hash_slot(id, 16) as usize;
    w.bitmap[0] |= 1 << s;
    w.keys[s] = id;
    w.values[s] = w.record.as_ptr() as u64 + base as u64;
}

#[test]
fn teammates_get_a_shared_faction_and_lose_it_when_the_team_breaks_up() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_lend_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    let t1 = transform(14 << 32, 20 << 32, 34 << 32);
    second_entity(&mut w, 2, 4, &t1);
    add_component(&mut w, 16, 1, 0x2100);
    w.chunk[0x2100 + 3] = 7;
    w.chunk[0x2100 + 4] = 7;
    let mgr = w.mem.as_ptr() as usize;
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    spawner::track_player(mgr, 1);
    spawner::track_player(mgr, 2);
    config::LIVE.apply(&config::NativeConfig { pvp: true, ..Default::default() });

    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_teams.json"), r#"{"teams":[{"name":"A","members":[1,2]}]}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 4);
    assert_eq!(w.chunk[0x2100 + 3], 8);
    assert_eq!(w.chunk[0x2100 + 4], 8);
    assert!(!faction::enemy(8, 8, true));
    assert!(faction::enemy(8, 7, true));

    std::fs::write(dir.join("m182_teams.json"), r#"{"teams":[{"name":"A","members":[1]}]}"#).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));
    spawner::power_tick(&mut st, &dir, 8);
    assert_eq!(w.chunk[0x2100 + 3], 7);
    assert_eq!(w.chunk[0x2100 + 4], 7);
    assert!(faction::enemy(7, 7, true));

    std::fs::write(dir.join("m182_teams.json"), r#"{"teams":[{"name":"A","members":[1,2]}]}"#).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));
    spawner::power_tick(&mut st, &dir, 12);
    assert_eq!(w.chunk[0x2100 + 3], 8);
    config::LIVE.apply(&config::NativeConfig { pvp: false, ..Default::default() });
    std::thread::sleep(std::time::Duration::from_millis(250));
    spawner::power_tick(&mut st, &dir, 16);
    assert_eq!(w.chunk[0x2100 + 3], 7);
    assert_eq!(faction::team_mask(), 0);

    groups::set_team_map(Vec::new());
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(dir);
}

fn susceptibility_bytes(values: [f32; 19]) -> [u8; 84] {
    let mut b = [0u8; 84];
    b[0..4].copy_from_slice(&2_415_994_894u32.to_le_bytes());
    for (i, v) in values.iter().enumerate() {
        b[8 + i * 4..12 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    b
}

fn fall_bytes(start: f32, lethal: f32) -> [u8; 24] {
    let mut b = [0u8; 24];
    b[0..4].copy_from_slice(&start.to_le_bytes());
    b[4..8].copy_from_slice(&lethal.to_le_bytes());
    b[8..24].copy_from_slice(&[0x53, 0xb0, 0xb6, 0x2d, 0x90, 0x4e, 0x82, 0x47, 0xab, 0x37, 0xf7, 0xff, 0xe8, 0x83, 0x50, 0x63]);
    b
}

fn f32_word(w: &World, at: usize, off: usize) -> f32 {
    f32::from_le_bytes(w.chunk[at + off..at + off + 4].try_into().unwrap())
}

const NORMAL: [f32; 19] = [0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0];

#[test]
fn god_mode_zeroes_damage_susceptibility_and_fall_damage_then_restores_them() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_shield_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    add_component(&mut w, 11, 40, 0x1800);
    add_component(&mut w, 17, 84, 0x2800);
    add_component(&mut w, 18, 24, 0x3000);
    let health_at = 0x1800 + 40 * 3;
    let susc_at = 0x2800 + 84 * 3;
    let fall_at = 0x3000 + 24 * 3;
    put(&mut w.chunk, health_at, &resource_bytes(0x8eb8_4995, [5, 0, 1200, 873, 100, 800, 45, 2073]));
    put(&mut w.chunk, susc_at, &susceptibility_bytes(NORMAL));
    put(&mut w.chunk, fall_at, &fall_bytes(8.0, 16.0));
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);

    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_powers.json"), r#"{"god":true}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 0);
    for i in 0..19 {
        assert_eq!(f32_word(&w, susc_at, 8 + i * 4), 0.0, "slot {i}");
    }
    assert_eq!(f32_word(&w, fall_at, 0), 1.0e9);
    assert_eq!(f32_word(&w, fall_at, 4), 1.0e9);
    assert_eq!(word(&w, health_at, 8), 2073);

    // the game recomputes a hit while shielded: the next tick zeroes it again
    put(&mut w.chunk, susc_at, &susceptibility_bytes(NORMAL));
    spawner::power_tick(&mut st, &dir, 1);
    assert_eq!(f32_word(&w, susc_at, 12), 0.0);

    std::fs::write(dir.join("m182_powers.json"), r#"{"god":false,"fly_speed":5}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 10);
    for i in 0..19 {
        assert_eq!(f32_word(&w, susc_at, 8 + i * 4), NORMAL[i], "restored slot {i}");
    }
    assert_eq!(f32_word(&w, fall_at, 0), 8.0);
    assert_eq!(f32_word(&w, fall_at, 4), 16.0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn shield_never_keeps_half_written_values_as_the_original() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_shield_half_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    add_component(&mut w, 11, 40, 0x1800);
    add_component(&mut w, 17, 84, 0x2800);
    let health_at = 0x1800 + 40 * 3;
    let susc_at = 0x2800 + 84 * 3;
    put(&mut w.chunk, health_at, &resource_bytes(0x8eb8_4995, [5, 0, 1200, 873, 100, 800, 45, 2073]));
    put(&mut w.chunk, susc_at, &susceptibility_bytes(NORMAL));
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_powers.json"), r#"{"god":true}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 0);
    // the game adds one effect on top of the zeroed values (not all zero, not the original either)
    let mut half = [0f32; 19];
    half[3] = 0.25;
    put(&mut w.chunk, susc_at, &susceptibility_bytes(half));
    spawner::power_tick(&mut st, &dir, 1);
    spawner::power_tick(&mut st, &dir, 2);
    assert_eq!(f32_word(&w, susc_at, 8 + 3 * 4), 0.0);
    // and again with the shield switched off right after such a write
    put(&mut w.chunk, susc_at, &susceptibility_bytes(half));
    std::fs::write(dir.join("m182_powers.json"), r#"{"god":false,"fly_speed":5}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 10);
    for i in 0..19 {
        assert_eq!(f32_word(&w, susc_at, 8 + i * 4), NORMAL[i], "restored slot {i}");
    }
    // a second activation cycle behaves the same
    std::fs::write(dir.join("m182_powers.json"), r#"{"god":true}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 20);
    assert_eq!(f32_word(&w, susc_at, 12), 0.0);
    std::fs::write(dir.join("m182_powers.json"), r#"{"god":false,"fly_speed":6}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 30);
    for i in 0..19 {
        assert_eq!(f32_word(&w, susc_at, 8 + i * 4), NORMAL[i], "second cycle slot {i}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn safe_zone_shields_only_the_players_inside() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_zone_shield_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    add_component(&mut w, 11, 40, 0x1800);
    add_component(&mut w, 17, 84, 0x2800);
    let health_at = 0x1800 + 40 * 3;
    let susc_at = 0x2800 + 84 * 3;
    put(&mut w.chunk, health_at, &resource_bytes(0x8eb8_4995, [500, 0, 1200, 873, 100, 800, 45, 2073]));
    put(&mut w.chunk, susc_at, &susceptibility_bytes(NORMAL));
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    spawner::track_player(mgr, 1);
    let mut st = spawner::new_power_state();

    std::fs::write(dir.join("m182_zones.json"), r#"{"zones":[{"name":"Far","x":900,"z":900,"radius":20}]}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 4);
    assert_eq!(f32_word(&w, susc_at, 12), 1.0);
    assert_eq!(word(&w, health_at, 8), 500);

    std::fs::write(dir.join("m182_zones.json"), r#"{"zones":[{"name":"Here","x":12,"z":28,"radius":10}]}"#).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));
    spawner::power_tick(&mut st, &dir, 8);
    spawner::power_tick(&mut st, &dir, 9);
    assert_eq!(f32_word(&w, susc_at, 12), 0.0);
    assert_eq!(word(&w, health_at, 8), 2073);

    std::fs::write(dir.join("m182_zones.json"), r#"{"zones":[{"name":"Far","x":900,"z":900,"radius":20}]}"#).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));
    spawner::power_tick(&mut st, &dir, 12);
    spawner::power_tick(&mut st, &dir, 13);
    assert_eq!(f32_word(&w, susc_at, 12), 1.0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_limit_the_game_lowers_is_followed_not_fought() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_rain_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    add_component(&mut w, 12, 40, 0x1a00);
    let stamina_at = 0x1a00 + 40 * 3;
    put(&mut w.chunk, stamina_at, &resource_bytes(0x04b6_aa8b, [720, 0, 720, 2000, 500, 107, 720, 0]));
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_powers.json"), r#"{"stamina":true}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 0);
    put(&mut w.chunk, stamina_at, &resource_bytes(0x04b6_aa8b, [300, 0, 300, 2000, 500, 107, 400, 0]));
    spawner::power_tick(&mut st, &dir, 1);
    assert_eq!(word(&w, stamina_at, 8), 400);
    assert_eq!(word(&w, stamina_at, 16), 400);
    assert_eq!(word(&w, stamina_at, 32), 400);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn item_struct_patch_uses_the_exact_layout_and_ignores_lookalikes() {
    let _g = LOCK.lock().unwrap();
    let guid = config::parse_guid("12345678-1234-4abc-8def-0123456789ab").unwrap();
    let item: u32 = 3_000_000_001;
    let old: Vec<u8> = [1.0f32, 1.25, 12.5, 3.0, 0.0, 7.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0].iter().flat_map(|f| f.to_le_bytes()).collect();
    let new: Vec<u8> = [1.0f32, 2.5, 99.0, 3.0, 0.0, 7.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0].iter().flat_map(|f| f.to_le_bytes()).collect();
    let mut big = vec![0u8; 0x60000];
    let start = big.as_ptr() as usize;
    let shift = ((start + 0xffff) & !0xffff) - start + 0x10000;
    let base = shift + 0x1000;
    big[base..base + 4].copy_from_slice(&item.to_le_bytes());
    big[base + 4..base + 20].copy_from_slice(&guid);
    big[base + 1660..base + 1720].copy_from_slice(&old);
    big[base + 1720] = 1;
    // an identical lookalike elsewhere in the same region must stay untouched
    let twin = base + 0x8000;
    big[twin + 1660..twin + 1720].copy_from_slice(&old);
    let regions = vec![(start + shift, start + shift + 0x20000)];
    let blocks = vec![(1660usize, new.clone(), vec![old.clone()])];
    let (done, notes) = spawner::patch_item_struct(&guid, item, &blocks, &regions);
    assert_eq!(done.len(), 1, "{notes}");
    assert_eq!(&big[base + 1660..base + 1720], &new[..]);
    assert_eq!(&big[twin + 1660..twin + 1720], &old[..]);
    // applying the same values again is recognised as already done
    let (again, _) = spawner::patch_item_struct(&guid, item, &blocks, &regions);
    assert_eq!(again.len(), 1);
}


#[test]
fn keeper_restores_reset_health_but_lets_creatures_take_damage_and_die() {
    let _g = LOCK.lock().unwrap();
    let t0 = transform(1 << 32, 2 << 32, 3 << 32);
    let mut w = world(1, 3, &t0);
    add_component(&mut w, 11, 40, 0x1800);
    let health_at = 0x1800 + 40 * 3;
    put(&mut w.chunk, health_at, &resource_bytes(0x8eb8_4995, [1031, 0, 1031, 0, 0, 0, 0, 1031]));
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::start_stats(mgr, 1, 50000, 1.0);
    spawner::keeper_pass();
    for off in [8usize, 16, 36] {
        assert_eq!(word(&w, health_at, off), 50000);
    }
    // a hit: only the current health goes down and it stays down
    w.chunk[health_at + 8..health_at + 12].copy_from_slice(&40000u32.to_le_bytes());
    spawner::keeper_pass();
    assert_eq!(word(&w, health_at, 8), 40000);
    // the game recomputes the health from the level much later: it is set again
    for off in [8usize, 16, 36] {
        w.chunk[health_at + off..health_at + off + 4].copy_from_slice(&7718u32.to_le_bytes());
    }
    spawner::keeper_pass();
    for off in [8usize, 16, 36] {
        assert_eq!(word(&w, health_at, off), 50000);
    }
    // killed: never resurrected
    w.chunk[health_at + 8..health_at + 12].copy_from_slice(&0u32.to_le_bytes());
    for off in [16usize, 36] {
        w.chunk[health_at + off..health_at + off + 4].copy_from_slice(&196411u32.to_le_bytes());
    }
    spawner::keeper_pass();
    assert_eq!(word(&w, health_at, 8), 0);
    // a new world forgets the old creatures
    spawner::SERVER_MGR.store(mgr + 8, Ordering::SeqCst);
    spawner::keeper_pass();
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    w.chunk[health_at + 8..health_at + 12].copy_from_slice(&5u32.to_le_bytes());
    spawner::keeper_pass();
    assert_eq!(word(&w, health_at, 8), 5);
}

#[test]
fn spawner_creates_waits_and_respawns_after_a_death() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_spawners_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(5i64 << 32, 7i64 << 32, 9i64 << 32);
    let mut w = world(777, 3, &t0);
    add_component(&mut w, 11, 40, 0x1800);
    let health_at = 0x1800 + 40 * 3;
    put(&mut w.chunk, health_at, &resource_bytes(0x8eb8_4995, [300, 0, 300, 0, 0, 0, 0, 300]));
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(777, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    config::LIVE.apply(&config::NativeConfig { spawner: true, ..Default::default() });
    spawn_probe::set_trampoline(fake_create as usize);
    spawner::before(mgr, 7);
    SEEN.lock().unwrap().clear();
    spawner::PENDING.store(0, Ordering::SeqCst);
    *spawner::REQUEST.lock().unwrap() = None;
    std::fs::write(
        dir.join("m182_spawners.json"),
        r#"{"spawners":[{"id":"s1","name":"Cueva","template":"c3db66d6-89a5-4a6c-a8e3-d9a1f6009812","level":50,"count":1,"health":1234,"at":[100.0,5.0,200.0],"radius":2.0,"respawn":1}]}"#,
    )
    .unwrap();
    let mut st = spawner::new_power_state();

    // first tick: the creature is requested; the game thread creates it on its next entity creation
    spawner::power_tick(&mut st, &dir, 16);
    assert_eq!(spawner::PENDING.load(Ordering::SeqCst), 1);
    spawner::before(mgr, 7);
    {
        let seen = SEEN.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let data = &seen[0].3;
        let x = i64::from_le_bytes(data[4..12].try_into().unwrap());
        assert_eq!(x, 100i64 << 32);
    }
    std::thread::sleep(std::time::Duration::from_millis(2100));
    spawner::power_tick(&mut st, &dir, 32);
    let status = std::fs::read_to_string(dir.join("m182_spawner_status.json")).unwrap();
    assert!(status.contains("\"id\":\"s1\"") && status.contains("\"alive\":1"), "{status}");
    // alive: nothing else is requested
    std::thread::sleep(std::time::Duration::from_millis(1100));
    spawner::power_tick(&mut st, &dir, 48);
    assert_eq!(spawner::PENDING.load(Ordering::SeqCst), 0);

    // it dies: after three bad readings it counts as gone, then the respawn time starts
    w.chunk[health_at + 8..health_at + 12].copy_from_slice(&0u32.to_le_bytes());
    for tick in [64u32, 80, 96] {
        std::thread::sleep(std::time::Duration::from_millis(1100));
        spawner::power_tick(&mut st, &dir, tick);
    }
    std::thread::sleep(std::time::Duration::from_millis(2100));
    spawner::power_tick(&mut st, &dir, 112);
    let status = std::fs::read_to_string(dir.join("m182_spawner_status.json")).unwrap();
    assert!(status.contains("\"alive\":0"), "{status}");
    assert_eq!(spawner::PENDING.load(Ordering::SeqCst), 1, "the respawn time (1 s) has passed");
    spawner::before(mgr, 7);
    assert_eq!(SEEN.lock().unwrap().len(), 2, "the first creature and the new creature");

    // removing the spawner from the file removes it from the status
    std::fs::write(dir.join("m182_spawners.json"), r#"{"spawners":[]}"#).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2100));
    spawner::power_tick(&mut st, &dir, 128);
    let status = std::fs::read_to_string(dir.join("m182_spawner_status.json")).unwrap();
    assert!(!status.contains("s1"), "{status}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn player_scan_finds_players_the_creation_hook_never_saw() {
    let _g = LOCK.lock().unwrap();
    let t0 = transform(1 << 32, 2 << 32, 3 << 32);
    let mut w = world(55, 3, &t0);
    add_component(&mut w, 12, 40, 0x1a00);
    add_component(&mut w, 16, 1, 0x2100);
    put(&mut w.chunk, 0x1a00 + 40 * 3, &resource_bytes(0x04b6_aa8b, [100, 0, 100, 2000, 500, 107, 720, 0]));
    w.chunk[0x2100 + 3] = 7;
    let mgr = w.mem.as_ptr() as usize;
    spawner::reset_player_cache();
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    assert_eq!(spawner::scan_players_once(mgr), 1);
    assert_eq!(spawner::tracked_ids(), vec![55]);
    // already tracked: not counted twice
    assert_eq!(spawner::scan_players_once(mgr), 0);

    // an entity that is not a player (no stamina) is ignored
    let mut other = world(56, 2, &t0);
    add_component(&mut other, 16, 1, 0x2100);
    other.chunk[0x2100 + 2] = 7;
    spawner::reset_player_cache();
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    let mgr2 = other.mem.as_ptr() as usize;
    assert_eq!(spawner::scan_players_once(mgr2), 0);
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
}

#[test]
fn creatures_that_only_look_like_players_are_not_listed() {
    let _g = LOCK.lock().unwrap();
    assert!(spawner::component_similarity(&[1, 2, 3, 4], &[1, 2, 3, 4]) > 0.99);
    assert!(spawner::component_similarity(&[1, 2, 3, 4], &[1, 2, 3, 4, 5]) >= 0.8);
    assert!(spawner::component_similarity(&[1, 2, 3, 4], &[1, 9]) < 0.8);
    assert_eq!(spawner::component_similarity(&[], &[1]), 0.0);

    let t0 = transform(1 << 32, 2 << 32, 3 << 32);
    let mut host = world(1, 3, &t0);
    for (i, index) in (20..40).enumerate() {
        add_component(&mut host, index, 8, 0x3000 + (i as u16) * 32);
    }
    add_component(&mut host, 12, 40, 0x1a00);
    add_component(&mut host, 16, 1, 0x2100);
    put(&mut host.chunk, 0x1a00 + 40 * 3, &resource_bytes(0x04b6_aa8b, [100, 0, 100, 2000, 500, 107, 720, 0]));
    host.chunk[0x2100 + 3] = 7;
    let hmgr = host.mem.as_ptr() as usize;

    let mut friend = world(77, 3, &t0);
    for (i, index) in (20..40).enumerate() {
        add_component(&mut friend, index, 8, 0x3000 + (i as u16) * 32);
    }
    add_component(&mut friend, 12, 40, 0x1a00);
    add_component(&mut friend, 16, 1, 0x2100);
    put(&mut friend.chunk, 0x1a00 + 40 * 3, &resource_bytes(0x04b6_aa8b, [100, 0, 100, 2000, 500, 107, 720, 0]));
    friend.chunk[0x2100 + 3] = 7;

    let mut creature = world(78, 3, &t0);
    add_component(&mut creature, 12, 40, 0x1a00);
    add_component(&mut creature, 16, 1, 0x2100);
    put(&mut creature.chunk, 0x1a00 + 40 * 3, &resource_bytes(0x04b6_aa8b, [100, 0, 100, 2000, 500, 107, 720, 0]));
    creature.chunk[0x2100 + 3] = 7;

    spawner::SERVER_MGR.store(hmgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::reset_player_cache();
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    let cmgr = creature.mem.as_ptr() as usize;
    assert_eq!(spawner::scan_players_once(cmgr), 0, "a creature with player faction and stamina is not a player");
    spawner::reset_player_cache();
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    let fmgr = friend.mem.as_ptr() as usize;
    assert_eq!(spawner::scan_players_once(fmgr), 1, "a character built like the host is");
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    spawner::SERVER_MGR.store(0, Ordering::SeqCst);
    spawner::PLAYER_ID.store(0, Ordering::SeqCst);
}

#[test]
fn on_screen_stamina_copy_is_never_touched() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_net_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut server = world(1, 3, &t0);
    add_component(&mut server, 12, 40, 0x1a00);
    put(&mut server.chunk, 0x1a00 + 40 * 3, &resource_bytes(0x04b6_aa8b, [1000, 0, 1000, 2000, 500, 107, 720, 280]));
    let mut client = world(1, 2, &t0);
    put(&mut client.mem, 8, &[1]);
    add_component(&mut client, 15, 4, 0x2200);
    add_component(&mut client, 20, 4, 0x2300);
    let net_at = 0x2200 + 4 * 2;
    let float_at = 0x2300 + 4 * 2;
    client.chunk[net_at..net_at + 2].copy_from_slice(&930u16.to_le_bytes());
    client.chunk[net_at + 2..net_at + 4].copy_from_slice(&2000u16.to_le_bytes());
    client.chunk[float_at..float_at + 4].copy_from_slice(&1.0f32.to_le_bytes());
    let smgr = server.mem.as_ptr() as usize;
    let cmgr = client.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(smgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    spawner::CLIENT_MGR.store(cmgr, Ordering::SeqCst);
    spawner::CLIENT_PLAYER_ID.store(1, Ordering::SeqCst);
    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_powers.json"), r#"{"stamina":true}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 0);
    spawner::power_tick(&mut st, &dir, 1);
    let cur = u16::from_le_bytes([client.chunk[net_at], client.chunk[net_at + 1]]);
    assert_eq!(cur, 930, "the copy on screen is left to the game");
    assert_eq!(&client.chunk[float_at..float_at + 4], &1.0f32.to_le_bytes(), "other 4-byte components stay untouched");
    assert_eq!(word(&server, 0x1a00 + 40 * 3, 8), 1000, "the value is held where it was, not pushed up");
    server.chunk[0x1a00 + 40 * 3 + 8..0x1a00 + 40 * 3 + 12].copy_from_slice(&600u32.to_le_bytes());
    spawner::power_tick(&mut st, &dir, 2);
    assert_eq!(word(&server, 0x1a00 + 40 * 3, 8), 1000, "a drain is undone");
    assert_eq!(u16::from_le_bytes([client.chunk[net_at], client.chunk[net_at + 1]]), 930);
    spawner::CLIENT_MGR.store(0, Ordering::SeqCst);
    spawner::CLIENT_PLAYER_ID.store(0, Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn stamina_follows_the_limit_the_game_sets_under_a_state_instead_of_fighting_it() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_follow_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    add_component(&mut w, 12, 40, 0x1a00);
    let at = 0x1a00 + 40 * 3;
    put(&mut w.chunk, at, &resource_bytes(0x04b6_aa8b, [400, 0, 400, 2000, 500, 107, 720, (-89i32) as u32]));
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_powers.json"), r#"{"stamina":true}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 0);
    assert_eq!(word(&w, at, 8), 631, "refilled to the limit the state leaves, never above it");
    assert_eq!(word(&w, at, 16), 631);
    put(&mut w.chunk, at + 36, &307i32.to_le_bytes());
    w.chunk[at + 8..at + 12].copy_from_slice(&100u32.to_le_bytes());
    spawner::power_tick(&mut st, &dir, 1);
    assert_eq!(word(&w, at, 8), 1027, "a food bonus raises the limit");
    let _ = std::fs::remove_dir_all(dir);
}

fn buff_container(debuffs: &[(usize, u32, u32)]) -> Vec<u8> {
    let mut b = vec![0u8; 472];
    for (slot, entity, kind) in debuffs {
        let at = 240 + slot * 12;
        b[at..at + 4].copy_from_slice(&entity.to_le_bytes());
        b[at + 4..at + 8].copy_from_slice(&kind.to_le_bytes());
    }
    b
}

#[test]
fn infinite_stamina_cleanses_debuffs_but_never_foods_or_unknown_layouts() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_buffs_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 0, &t0);
    add_component(&mut w, 12, 40, 0x1a00);
    put(&mut w.chunk, 0x1a00, &resource_bytes(0x04b6_aa8b, [720, 0, 720, 2000, 500, 107, 720, 0]));
    add_component(&mut w, 30, 472, 0x2000);
    let mut container = buff_container(&[(0, 77, 5), (3, 78, 9)]);
    container[0..4].copy_from_slice(&90u32.to_le_bytes());
    container[4..8].copy_from_slice(&11u32.to_le_bytes());
    container[48..52].copy_from_slice(&91u32.to_le_bytes());
    container[52..56].copy_from_slice(&12u32.to_le_bytes());
    put(&mut w.chunk, 0x2000, &container);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_powers.json"), r#"{"stamina":true}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 0);
    assert_eq!(w.chunk[0x2000 + 240 + 8], 1, "first debuff is cleansed");
    assert_eq!(w.chunk[0x2000 + 240 + 3 * 12 + 8], 1, "fourth debuff is cleansed");
    assert_eq!(w.chunk[0x2000 + 8], 0, "food buffs stay");
    assert_eq!(w.chunk[0x2000 + 48 + 8], 0, "generic buffs stay");

    let mut odd = buff_container(&[(1, 5, 0)]);
    odd[240 + 12 + 8] = 7;
    put(&mut w.chunk, 0x2000, &odd);
    spawner::power_tick(&mut st, &dir, 4);
    assert_eq!(w.chunk[0x2000 + 240 + 12 + 8], 7, "an unrecognised layout is left alone");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn stamina_is_held_at_the_limit_even_if_it_was_switched_on_while_drained() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_limit_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 0, &t0);
    add_component(&mut w, 12, 40, 0x1a00);
    put(&mut w.chunk, 0x1a00, &resource_bytes(0x04b6_aa8b, [300, 0, 300, 2000, 500, 107, 720, 0]));
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_powers.json"), r#"{"stamina":true}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 0);
    assert_eq!(word(&w, 0x1a00, 8), 720);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn players_inside_a_safe_zone_get_a_short_lived_aura_effect() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_aura_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all("export").unwrap();
    std::fs::write(
        "export/m182_objects.csv",
        "name,guid,components\nVFX_Other,11111111-2222-3333-4444-555555555555,A\nVFX_Resource_Increase_Health_NoSound,642098d4-fc09-48e2-811e-782d438bf551,CurrentTransform|LifeTime\nVFX_Resource_Increase_Health,01020304-0506-0708-090a-0b0c0d0e0f10,CurrentTransform|LifeTime\n",
    )
    .unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let w = world(1, 3, &t0);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    spawner::track_player(mgr, 1);
    spawn_probe::set_trampoline(fake_create as usize);
    SEEN.lock().unwrap().clear();
    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_zones.json"), r#"{"zones":[{"name":"Here","x":12,"z":28,"radius":10}]}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 4);
    spawner::before(mgr, 7);
    {
        let seen = SEEN.lock().unwrap();
        assert_eq!(seen.len(), 1, "one effect for the one player inside");
        assert_eq!(&seen[0].2[4..20], &[0x04, 0x03, 0x02, 0x01, 0x06, 0x05, 0x08, 0x07, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10], "walking in plays the sounded cue");
        let x = i64::from_le_bytes(seen[0].3[4..12].try_into().unwrap());
        assert_eq!(x, 10i64 << 32);
    }
    std::thread::sleep(std::time::Duration::from_millis(250));
    spawner::power_tick(&mut st, &dir, 8);
    spawner::before(mgr, 7);
    assert_eq!(SEEN.lock().unwrap().len(), 1, "the effect is repeated every few seconds, not on every tick");
    std::thread::sleep(std::time::Duration::from_millis(3100));
    spawner::power_tick(&mut st, &dir, 12);
    spawner::before(mgr, 7);
    {
        let seen = SEEN.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(&seen[1].2[4..20], &[0xd4, 0x98, 0x20, 0x64, 0x09, 0xfc, 0xe2, 0x48, 0x81, 0x1e, 0x78, 0x2d, 0x43, 0x8b, 0xf5, 0x51], "staying inside repeats the quiet pulse");
    }

    std::fs::write(dir.join("m182_zones.json"), r#"{"zones":[{"name":"Far","x":900,"z":900,"radius":20}]}"#).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));
    spawner::power_tick(&mut st, &dir, 16);
    spawner::before(mgr, 7);
    assert_eq!(SEEN.lock().unwrap().len(), 2);
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_file("export/m182_objects.csv");
}

#[test]
fn an_item_copy_is_found_by_its_own_id_and_only_with_the_expected_stats() {
    let _g = LOCK.lock().unwrap();
    let item: u32 = 2_654_437_177;
    let mut big = vec![0u8; 0x10000];
    let start = big.as_ptr() as usize;
    let shift = ((start + 7) & !7) - start;
    let copy = shift + 64;
    let other = shift + 0x4000;
    let vanilla: Vec<u8> = [1.0f32, 1.25, 12.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0].iter().flat_map(|f| f.to_le_bytes()).chain([0u32, 0, 0, 0].iter().flat_map(|i| i.to_le_bytes())).collect();
    let edited: Vec<u8> = [1.0f32, 3.0, 99.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0].iter().flat_map(|f| f.to_le_bytes()).chain([0u32, 0, 0, 0].iter().flat_map(|i| i.to_le_bytes())).collect();
    big[copy..copy + 4].copy_from_slice(&item.to_le_bytes());
    big[copy + 1660..copy + 1720].copy_from_slice(&vanilla);
    // the base item holds the same damage but another id: it must stay untouched
    big[other..other + 4].copy_from_slice(&1234u32.to_le_bytes());
    big[other + 1660..other + 1720].copy_from_slice(&vanilla);
    // a stray copy of the id with a different block is a lookalike and is refused
    let stray = shift + 0x8000;
    big[stray..stray + 4].copy_from_slice(&item.to_le_bytes());
    big[stray + 1660..stray + 1720].copy_from_slice(&[0x55u8; 60]);
    let regions = vec![(start + shift, start + shift + 0xC000)];
    let (done, notes) = spawner::patch_item_struct_by_id(item, &[(1660usize, edited.clone(), vec![vanilla.clone()])], &regions);
    assert_eq!(done.len(), 1, "{notes}");
    assert_eq!(&big[copy + 1660..copy + 1720], &edited[..]);
    assert_eq!(&big[other + 1660..other + 1720], &vanilla[..]);
    assert_eq!(&big[stray + 1660..stray + 1720], &[0x55u8; 60][..]);
}

#[test]
fn a_spawn_asked_from_chat_appears_next_to_the_player_who_asked() {
    let _g = LOCK.lock().unwrap();
    let t0 = transform(40i64 << 32, 7i64 << 32, 60i64 << 32);
    let w = world(2, 2, &t0);
    let mgr = w.mem.as_ptr() as usize;
    assert!(spawner::calibrate(mgr, 2, &t0[..24]));
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    spawner::track_player(mgr, 2);
    config::LIVE.apply(&config::NativeConfig { spawner: true, ..Default::default() });
    spawn_probe::set_trampoline(fake_create as usize);
    SEEN.lock().unwrap().clear();
    std::fs::write("m182_spawn.request", r#"{"template":"c3db66d6-89a5-4a6c-a8e3-d9a1f6009812","count":1,"distance":1.0}"#).unwrap();
    // chat client 65 is the second player of the world, entity 2
    m182_chat_requester(66);
    spawner::poll_request();
    spawner::before(mgr, 7);
    let seen = SEEN.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let x = i64::from_le_bytes(seen[0].3[4..12].try_into().unwrap());
    let z = i64::from_le_bytes(seen[0].3[20..28].try_into().unwrap());
    assert!(((x >> 32) - 40).abs() <= 2 && ((z >> 32) - 60).abs() <= 2, "{} {}", x >> 32, z >> 32);
    drop(seen);
    assert_eq!(m182_chat_requester(0), 0);
}

fn m182_chat_requester(value: u32) -> u32 {
    use linuxtest::native::chat::REQUESTER;
    REQUESTER.swap(value, Ordering::SeqCst)
}

fn fake_table(slots: &[(usize, u32, u8, &str)]) -> Vec<u8> {
    let mut t = vec![0u8; 0x1e0 + 17 * 0x148 + 64];
    for (index, client, perms, name) in slots {
        let at = 0x1e0 + index * 0x148;
        t[at..at + 4].copy_from_slice(&client.to_le_bytes());
        t[at + 0x0a] = 3;
        t[at + 0x10] = *perms;
        t[at + 0x20..at + 0x20 + name.len()].copy_from_slice(name.as_bytes());
    }
    t
}

fn resources_for(w: &mut World, slot: usize) -> (usize, usize, usize) {
    let health_at = 0x1800 + 40 * slot;
    let stamina_at = 0x1a00 + 40 * slot;
    let susc_at = 0x2800 + 84 * slot;
    put(&mut w.chunk, health_at, &resource_bytes(0x8eb8_4995, [500, 0, 1200, 873, 100, 800, 45, 2073]));
    put(&mut w.chunk, stamina_at, &resource_bytes(0x04b6_aa8b, [100, 0, 100, 2000, 500, 107, 720, 121]));
    put(&mut w.chunk, susc_at, &susceptibility_bytes(NORMAL));
    (health_at, stamina_at, susc_at)
}

#[test]
fn powers_sent_by_a_chat_client_apply_to_that_players_entity_only() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_per_client_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    let t1 = transform(14 << 32, 20 << 32, 34 << 32);
    second_entity(&mut w, 2, 4, &t1);
    add_component(&mut w, 11, 40, 0x1800);
    add_component(&mut w, 12, 40, 0x1a00);
    add_component(&mut w, 17, 84, 0x2800);
    let (h1, s1, u1) = resources_for(&mut w, 3);
    let (h2, s2, u2) = resources_for(&mut w, 4);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::CLIENT_MGR.store(0, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    let mut st = spawner::new_power_state();

    // chat client 65 is the second player of the world (entity 2)
    std::fs::write(dir.join("m182_powers_65.json"), r#"{"god":true,"stamina":true}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 0);
    for i in 0..19 {
        assert_eq!(f32_word(&w, u2, 8 + i * 4), 0.0, "the sender is immune");
        assert_eq!(f32_word(&w, u1, 8 + i * 4), NORMAL[i], "the other player is untouched");
    }
    assert_eq!(word(&w, h2, 8), 2073);
    assert_eq!(word(&w, h1, 8), 500);
    assert_eq!(word(&w, s2, 8), 720.max(word(&w, s2, 8)));
    assert!(word(&w, s2, 8) >= 100);
    assert_eq!(word(&w, s1, 8), 100);

    std::fs::write(dir.join("m182_powers_65.json"), r#"{"god":false,"stamina":false}"#).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    std::fs::write(dir.join("m182_powers_65.json"), r#"{"god":false,"stamina":false,"fly_speed":5}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 10);
    spawner::power_tick(&mut st, &dir, 11);
    for i in 0..19 {
        assert_eq!(f32_word(&w, u2, 8 + i * 4), NORMAL[i], "restored {i}");
    }
    spawner::SERVER_MGR.store(0, Ordering::SeqCst);
    spawner::PLAYER_ID.store(0, Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_game_connected_to_a_server_controls_the_character_on_screen() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_remote_world_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    add_component(&mut w, 11, 40, 0x1800);
    add_component(&mut w, 12, 40, 0x1a00);
    add_component(&mut w, 17, 84, 0x2800);
    let (h, s, u) = resources_for(&mut w, 3);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(0, Ordering::SeqCst);
    spawner::PLAYER_ID.store(0, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(-1, Ordering::SeqCst);
    spawner::CLIENT_TRANSFORM_INDEX.store(-1, Ordering::SeqCst);
    spawner::CLIENT_MGR.store(mgr, Ordering::SeqCst);
    spawner::CLIENT_PLAYER_ID.store(1, Ordering::SeqCst);
    assert!(spawner::remote_world());
    assert!(spawner::player_transform().is_none(), "the transform component is not known yet");
    spawner::guess_client_transform(mgr, 1);
    assert_eq!(spawner::CLIENT_TRANSFORM_INDEX.load(Ordering::SeqCst), 5);
    assert_eq!(spawner::player_transform().unwrap(), t0.to_vec());

    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_powers.json"), r#"{"god":true,"stamina":true}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 0);
    assert_eq!(f32_word(&w, u, 12), 0.0);
    assert_eq!(word(&w, h, 8), 2073);
    assert!(word(&w, s, 8) >= 100);

    m182_chat_requester(0);
    spawner::CLIENT_MGR.store(0, Ordering::SeqCst);
    spawner::CLIENT_PLAYER_ID.store(0, Ordering::SeqCst);
    spawner::CLIENT_TRANSFORM_INDEX.store(-1, Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_base_zone_turns_everybody_outside_its_team_into_a_visitor() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_base_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    let t1 = transform(14 << 32, 20 << 32, 34 << 32);
    second_entity(&mut w, 2, 4, &t1);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::CLIENT_MGR.store(0, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    spawner::track_player(mgr, 1);
    spawner::track_player(mgr, 2);
    let table = fake_table(&[(0, 128, 0b11_1110, "Bob"), (1, 65, 0b11_1110, "Ana")]);
    linuxtest::native::patch::PLAYERS_OBJECT.store(table.as_ptr() as usize, Ordering::SeqCst);
    let perms = |index: usize| table[0x1e0 + index * 0x148 + 0x10];

    let mut st = spawner::new_power_state();
    std::fs::write(
        dir.join("m182_teams.json"),
        r#"{"teams":[{"name":"Casa","members":[],"people":[{"key":"a","name":"ana"}]}]}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("m182_zones.json"),
        r#"{"zones":[{"name":"Base de Ana","x":12,"z":32,"radius":20,"safe":false,"team":1,"lock":true}]}"#,
    )
    .unwrap();
    spawner::power_tick(&mut st, &dir, 4);
    assert_eq!(perms(0), 0b10_0000, "Bob is a visitor: no chests, no building");
    assert_eq!(perms(1), 0b11_1110, "Ana belongs to the team that owns the place");
    assert!(st.zone_ids.is_empty(), "a base that is not a safe zone shields nobody");

    // Bob joins the team: his permissions come back
    std::fs::write(
        dir.join("m182_teams.json"),
        r#"{"teams":[{"name":"Casa","members":[],"people":[{"key":"a","name":"ana"},{"key":"b","name":"BOB"}]}]}"#,
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));
    spawner::power_tick(&mut st, &dir, 8);
    assert_eq!(perms(0), 0b11_1110);

    // an administrator is only restricted when the zone says so
    std::fs::write(dir.join("m182_teams.json"), r#"{"teams":[{"name":"Casa","members":[],"people":[{"key":"a","name":"ana"}]}]}"#).unwrap();
    let mut admin = fake_table(&[(0, 128, 0b11_1111, "Bob"), (1, 65, 0b11_1110, "Ana")]);
    linuxtest::native::patch::PLAYERS_OBJECT.store(admin.as_mut_ptr() as usize, Ordering::SeqCst);
    std::thread::sleep(std::time::Duration::from_millis(250));
    spawner::power_tick(&mut st, &dir, 12);
    assert_eq!(admin[0x1e0 + 0x10], 0b11_1111);
    std::fs::write(
        dir.join("m182_zones.json"),
        r#"{"zones":[{"name":"Prueba","x":12,"z":32,"radius":20,"safe":false,"team":1,"lock":true,"lock_admins":true}]}"#,
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));
    spawner::power_tick(&mut st, &dir, 16);
    assert_eq!(admin[0x1e0 + 0x10], 0b10_0001, "with the test switch even an administrator is a visitor");

    // leaving the zone gives everything back
    std::fs::write(dir.join("m182_zones.json"), r#"{"zones":[{"name":"Lejos","x":900,"z":900,"radius":20,"safe":false,"team":1,"lock":true,"lock_admins":true}]}"#).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));
    spawner::power_tick(&mut st, &dir, 20);
    assert_eq!(admin[0x1e0 + 0x10], 0b11_1111);

    linuxtest::native::patch::PLAYERS_OBJECT.store(0, Ordering::SeqCst);
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    spawner::SERVER_MGR.store(0, Ordering::SeqCst);
    spawner::PLAYER_ID.store(0, Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn every_connected_player_of_the_table_gets_tracked_and_a_late_joiner_becomes_a_visitor() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_late_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    let t1 = transform(14 << 32, 20 << 32, 34 << 32);
    second_entity(&mut w, 2, 4, &t1);
    add_component(&mut w, 12, 40, 0x1a00);
    add_component(&mut w, 16, 1, 0x2100);
    for slot in [3usize, 4] {
        put(&mut w.chunk, 0x1a00 + 40 * slot, &resource_bytes(0x04b6_aa8b, [100, 0, 100, 2000, 500, 107, 720, 0]));
        w.chunk[0x2100 + slot] = 7;
    }
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::CLIENT_MGR.store(0, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    spawner::reset_player_cache();
    spawner::track_player(mgr, 1);
    let table = fake_table(&[(0, 64, 0b11_1111, "Bob"), (1, 129, 0b11_1110, "Ana"), (2, 130, 0b11_1110, "Eve")]);
    linuxtest::native::patch::PLAYERS_OBJECT.store(table.as_ptr() as usize, Ordering::SeqCst);
    let perms = |index: usize| table[0x1e0 + index * 0x148 + 0x10];

    assert_eq!(spawner::track_table_players(mgr), 1, "Ana has an entity, Eve has none yet");
    assert_eq!(spawner::tracked_ids(), vec![1, 2]);
    assert_eq!(spawner::track_table_players(mgr), 0);

    let mut st = spawner::new_power_state();
    std::fs::write(dir.join("m182_zones.json"), r#"{"zones":[{"name":"Base","x":12,"z":32,"radius":20,"safe":false,"team":1,"lock":true}]}"#).unwrap();
    std::fs::write(dir.join("m182_teams.json"), r#"{"teams":[{"name":"Casa","members":[],"people":[{"key":"a","name":"bob"}]}]}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 4);
    assert_eq!(perms(1), 0b10_0000, "Ana was never seen by the creation hook, she still loses the chests");
    assert_eq!(perms(0), 0b11_1111, "Bob owns the base");

    linuxtest::native::patch::PLAYERS_OBJECT.store(0, Ordering::SeqCst);
    groups::set_team_map(Vec::new());
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    spawner::SERVER_MGR.store(0, Ordering::SeqCst);
    spawner::PLAYER_ID.store(0, Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_player_the_game_creates_with_another_kind_is_tracked_too() {
    let _g = LOCK.lock().unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let w = world(1, 3, &t0);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    let mut head = vec![0u8; 0x30];
    head[4..20].copy_from_slice(&[0xd6, 0x66, 0xdb, 0xc3, 0xa5, 0x89, 0x6c, 0x4a, 0xa8, 0xe3, 0xd9, 0xa1, 0xf6, 0x00, 0x98, 0x12]);
    spawner::after(mgr, 0, head.as_ptr() as usize, 12);
    assert!(spawner::tracked_ids().contains(&12));
    assert_eq!(spawner::PLAYER_ID.load(Ordering::SeqCst), 1, "the first player stays the host");
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    spawner::SERVER_MGR.store(0, Ordering::SeqCst);
    spawner::PLAYER_ID.store(0, Ordering::SeqCst);
}

#[test]
fn a_table_player_whose_entity_is_not_a_player_is_not_tracked() {
    let _g = LOCK.lock().unwrap();
    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(9, 3, &t0);
    add_component(&mut w, 16, 1, 0x2100);
    w.chunk[0x2100 + 3] = 7;
    let mgr = w.mem.as_ptr() as usize;
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
    spawner::reset_player_cache();
    let table = fake_table(&[(8, 136, 0b11_1110, "Ana")]);
    linuxtest::native::patch::PLAYERS_OBJECT.store(table.as_ptr() as usize, Ordering::SeqCst);
    assert_eq!(spawner::track_table_players(mgr), 0);
    assert!(!spawner::tracked_ids().contains(&9));
    linuxtest::native::patch::PLAYERS_OBJECT.store(0, Ordering::SeqCst);
    spawner::TRACKED_MGR.store(0, Ordering::SeqCst);
}

#[test]
fn the_player_list_is_read_back_from_wherever_the_chat_left_it() {
    use linuxtest::native::roster;
    let _g = LOCK.lock().unwrap();
    let line = roster::reply_text(41, &[(1, "Carlos".into()), (2, "Ñandú".into())]);
    let mut narrow = vec![0u8; 5000];
    narrow[1000..1000 + line.len()].copy_from_slice(line.as_bytes());
    let mut wide = vec![0u8; 5000];
    let units: Vec<u8> = format!("Carlos: {line}").encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    wide[2000..2000 + units.len()].copy_from_slice(&units);
    let stale = roster::reply_text(40, &[(9, "Viejo".into())]);
    let mut old = vec![0u8; 400];
    old[10..10 + stale.len()].copy_from_slice(stale.as_bytes());
    for (memory, label) in [(&narrow, "utf-8"), (&wide, "utf-16")] {
        let region = [(memory.as_ptr() as usize, memory.as_ptr() as usize + memory.len())];
        let found = spawner::find_roster(&region, 41).unwrap_or_else(|| panic!("{label}: not found"));
        assert_eq!(found, vec![(1, "Carlos".to_string()), (2, "Ñandú".to_string())], "{label}");
    }
    let region = [(old.as_ptr() as usize, old.as_ptr() as usize + old.len())];
    assert!(spawner::find_roster(&region, 41).is_none(), "an answer to an older question is not the answer");
}

fn world_creature(mgr: usize, template: &str, x: i64, z: i64, node: u32) {
    let guid = config::parse_guid(template).unwrap();
    let t = transform(x << 32, 7i64 << 32, z << 32);
    let mut loot = [0u8; 12];
    loot[4..8].copy_from_slice(&node.to_le_bytes());
    let mut head = vec![0u8; 0x30];
    head[4..20].copy_from_slice(&guid);
    let mut entries = vec![0u8; 48];
    entries[0..4].copy_from_slice(&0x027ac564u32.to_le_bytes());
    entries[8..16].copy_from_slice(&(t.as_ptr() as u64).to_le_bytes());
    entries[16..24].copy_from_slice(&56u64.to_le_bytes());
    entries[24..28].copy_from_slice(&0x061dda01u32.to_le_bytes());
    entries[32..40].copy_from_slice(&(loot.as_ptr() as u64).to_le_bytes());
    entries[40..48].copy_from_slice(&12u64.to_le_bytes());
    head[0x18..0x20].copy_from_slice(&(entries.as_ptr() as u64).to_le_bytes());
    head[0x20..0x28].copy_from_slice(&2u64.to_le_bytes());
    spawner::after(mgr, 5, head.as_ptr() as usize, 9);
}

fn loot_entry(data: &[u8], _count: usize) -> Option<u32> {
    let mut at = 0;
    let mut found = None;
    for hash in [0x027ac564u32, 0xa21e7ee7, 0x061dda01] {
        if data.len() < at + 4 {
            break;
        }
        let size = match hash { 0x027ac564 => 56, 0xa21e7ee7 => 4, _ => 12 };
        if u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) == hash && hash == 0x061dda01 {
            found = Some(u32::from_le_bytes(data[at + 8..at + 12].try_into().unwrap()));
        }
        at += 4 + size;
    }
    found
}

#[test]
fn a_panel_creature_adopts_the_loot_node_of_the_world_creatures_around() {
    let _g = LOCK.lock().unwrap();
    std::fs::create_dir_all("export").unwrap();
    std::fs::write(
        "export/m182_enemies.csv",
        "name,guid,faction,boss,preset\nEnemy_A,642098d4-fc09-48e2-811e-782d438bf551,Drak,false,\nEnemy_A@x10,642098d4-fc09-4182-0002-782d438bf551,Drak,false,x10\nEnemy_B,c3db66d6-89a5-4a6c-a8e3-d9a1f6009812,Drak,false,\n",
    )
    .unwrap();
    let _ = std::fs::remove_file("m182_loot_nodes.txt");
    let t0 = transform(5i64 << 32, 7i64 << 32, 9i64 << 32);
    let w = world(777, 3, &t0);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(777, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    config::LIVE.apply(&config::NativeConfig { spawner: true, ..Default::default() });
    spawn_probe::set_trampoline(fake_create as usize);

    world_creature(mgr, "642098d4-fc09-48e2-811e-782d438bf551", 60, 60, 0xabcd);
    world_creature(mgr, "642098d4-fc09-4182-0002-782d438bf551", 6, 9, 0x1111);
    world_creature(mgr, "642098d4-fc09-48e2-811e-782d438bf551", 70, 70, 0);

    SEEN.lock().unwrap().clear();
    std::fs::write("m182_spawn.request", r#"{"template":"642098d4-fc09-48e2-811e-782d438bf551","count":1}"#).unwrap();
    spawner::poll_request();
    spawner::before(mgr, 7);
    {
        let seen = SEEN.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let (_, _, desc, data) = &seen[0];
        assert_eq!(u64::from_le_bytes(desc[0x20..0x28].try_into().unwrap()), 3, "transform, level and the loot node");
        assert_eq!(loot_entry(data, 3), Some(0xabcd), "the node of a creature of the same kind wins over a closer one of another kind");
    }

    SEEN.lock().unwrap().clear();
    std::fs::write("m182_spawn.request", r#"{"template":"c3db66d6-89a5-4a6c-a8e3-d9a1f6009812","count":1}"#).unwrap();
    spawner::poll_request();
    spawner::before(mgr, 7);
    {
        let seen = SEEN.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(loot_entry(&seen[0].3, 3), Some(0x1111), "a kind never seen takes the closest node of any kind");
    }

    spawner::save_loot_nodes();
    let saved = std::fs::read_to_string("m182_loot_nodes.txt").unwrap();
    assert!(saved.contains("642098d4-fc09-48e2-811e-782d438bf551 60.0 60.0 0000abcd"), "{saved}");
    assert!(saved.contains("0000abcd") && saved.contains("00001111"));
    assert!(!saved.contains("70.0 70.0"), "creatures without a loot node are not kept");
}

#[test]
fn powers_of_a_player_who_left_do_not_pass_to_the_next_one() {
    let _g = LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("m182_gone_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("m182_powers_77.json"), r#"{"god":true}"#).unwrap();
    std::fs::write(dir.join("m182_powers.json"), r#"{"god":false}"#).unwrap();
    std::fs::write(dir.join("m182_zones.json"), r#"{"zones":[]}"#).unwrap();
    spawner::clear_client_power_files(&dir);
    assert!(!dir.join("m182_powers_77.json").exists(), "leftovers of an earlier run are cleared");
    assert!(dir.join("m182_powers.json").exists() && dir.join("m182_zones.json").exists(), "only per-player powers are cleared");

    let t0 = transform(10 << 32, 20 << 32, 30 << 32);
    let mut w = world(1, 3, &t0);
    add_component(&mut w, 11, 40, 0x1800);
    add_component(&mut w, 12, 40, 0x1a00);
    add_component(&mut w, 17, 84, 0x2800);
    let _ = resources_for(&mut w, 3);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawner::PLAYER_ID.store(1, Ordering::SeqCst);
    spawner::CLIENT_MGR.store(0, Ordering::SeqCst);
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    spawner::set_client_gone_ms(60);
    let mut st = spawner::new_power_state();

    std::fs::write(dir.join("m182_powers_66.json"), r#"{"god":true,"stamina":true}"#).unwrap();
    spawner::power_tick(&mut st, &dir, 0);
    assert!(dir.join("m182_powers_66.json").exists(), "still inside the grace period");
    std::thread::sleep(std::time::Duration::from_millis(120));
    spawner::power_tick(&mut st, &dir, 1);
    spawner::power_tick(&mut st, &dir, 10);
    assert!(!dir.join("m182_powers_66.json").exists(), "the entity is gone: its powers are dropped");
    spawner::power_tick(&mut st, &dir, 20);
    assert!(!dir.join("m182_powers_66.json").exists());
    spawner::set_client_gone_ms(12_000);
    spawner::SERVER_MGR.store(0, Ordering::SeqCst);
    spawner::PLAYER_ID.store(0, Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(dir);
}

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
    assert!(!cfg.charge.effects.is_empty());
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
    let mut st = vfx::PlayerCharge::default();
    let heavy = READY | 1 << vfx::FLAG_HEAVY_ATTACK;
    let aim = READY | 1 << vfx::FLAG_RANGED_PREPARING;
    assert_eq!(vfx::step(&mut st, READY, 0, 300, 300), vfx::Event::None);
    assert_eq!(vfx::step(&mut st, heavy, 1000, 300, 300), vfx::Event::None);
    assert_eq!(vfx::step(&mut st, heavy, 1130, 300, 300), vfx::Event::Charge { melee: true });
    assert_eq!(vfx::step(&mut st, heavy, 1300, 300, 300), vfx::Event::None);
    assert_eq!(vfx::step(&mut st, heavy, 1430, 300, 300), vfx::Event::Charge { melee: true });
    assert_eq!(vfx::step(&mut st, READY, 1500, 300, 300), vfx::Event::Release { melee: true });
    assert_eq!(vfx::step(&mut st, READY, 1600, 300, 300), vfx::Event::None);
    assert_eq!(vfx::step(&mut st, heavy, 2000, 300, 300), vfx::Event::None);
    assert_eq!(vfx::step(&mut st, READY, 2100, 300, 300), vfx::Event::None, "too short to count as charged");
    assert_eq!(vfx::step(&mut st, aim, 3000, 300, 300), vfx::Event::None);
    assert_eq!(vfx::step(&mut st, aim, 3200, 300, 300), vfx::Event::Charge { melee: false });
    assert_eq!(vfx::step(&mut st, READY, 3600, 300, 300), vfx::Event::Release { melee: false });
}

#[test]
fn defaults_keep_the_staff_aura_and_drop_flashes() {
    let cfg = vfx::parse_config("{}").unwrap();
    assert!(cfg.auto);
    assert!(!cfg.charge.effects.is_empty());
    assert!(cfg.release.effects.is_empty());
    assert!(cfg.melee_charge.effects.is_empty());
    assert!(cfg.melee_release.effects.is_empty());
}

#[test]
fn elements_follow_the_damage_columns() {
    use vfx::Element::*;
    assert_eq!(vfx::element_of([0.5, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0]), Fire);
    assert_eq!(vfx::element_of([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]), Lightning);
    assert_eq!(vfx::element_of([0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0]), Ice);
    assert_eq!(vfx::element_of([0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]), Shroud);
    assert_eq!(vfx::element_of([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.6, 0.0]), Healing);
    assert_eq!(vfx::element_of([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]), Poison);
    assert_eq!(vfx::element_of([0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]), Bone);
    assert_eq!(vfx::element_of([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0]), Arcane);
    assert_eq!(vfx::element_of([0.6, 0.4, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]), Physical);
    let csv = "item_id,guid,debug_name,slot,cut,pierce,blunt,fire,ice,fog,lightning,healing,poison\n518418528,x,Weapon_T4_1H_SwordEpic_01,MeleeWeapon,0.5,0,0,0.5,0,0,0,0,0\n1048548069,y,Ammo_T2_Spell_Lightning_Chain_lvl12,Ammunition,0,0,0,0,0,0,1,0,0\n1250423976,z,Ammo_T3_Spell_Skeleton_lvl17,Ammunition,0.5,0,0,0,0.5,0,0,0,0\n42,w,Weapon_T1_1H_Wand_02_Ice,MidRangeWeapon,0,0,0,0,1,0,0,0,0\nbad,line\n";
    let t = vfx::parse_items(csv);
    assert_eq!(t.len(), 4);
    assert_eq!(t[&518418528].element, Fire);
    assert!(!t[&518418528].wand);
    assert_eq!(t[&1048548069].element, Lightning);
    assert_eq!(t[&1250423976].element, Summon);
    assert!(t[&42].wand);
    let mut seen = std::collections::HashSet::new();
    for e in [Lightning, Fire, Ice, Shroud, Healing, Poison, Bone, Arcane, Physical] {
        assert!(!vfx::staff_effects(e).is_empty());
        seen.insert(vfx::staff_effects(e)[0]);
        for name in vfx::staff_effects(e) {
            assert!(vfx::is_vanilla(name), "{name} must be a game effect");
        }
    }
    assert!(!vfx::staff_effects(Summon).is_empty());
    assert!(vfx::hand_effect(Fire).is_none(), "no custom templates: clients without the mod would be kicked");
    let mut carriers = std::collections::HashSet::new();
    for e in [Lightning, Fire, Ice, Shroud, Healing, Poison, Bone, Arcane, Physical] {
        let g = vfx::hand_template(e).unwrap();
        assert!(carriers.insert(g));
        assert_ne!(&g[..4], &[0x18, 0x2e, 0x1e, 0x00], "glows must ride on game templates");
    }
    assert_eq!(vfx::hand_template(Fire), config::parse_guid("21f9f3e9-433f-48d7-b605-c0f2ad1ed5cf"));
    assert!(vfx::glow_template(0).is_none() && vfx::glow_template(15).is_none());
    let all: std::collections::HashSet<_> = (1..=14).map(|n| vfx::glow_template(n).unwrap()).collect();
    assert_eq!(all.len(), 14);
    assert_eq!(vfx::hand_effect(Poison), config::parse_guid("eee801de-8da6-4e11-b5f7-159205ffb139"));
    assert!(vfx::staff_scale(Shroud) < 1.0);
}

#[test]
fn burst_points_form_a_ring_in_front() {
    let m = 4294967296.0f64;
    let b = vfx::Burst { life_ms: None, effects: vec!["x".into()], scale: 2.0, height: 1.0, distance: 2.0, ring: 4, radius: 1.0 };
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
        life_ms: None,
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
    let b = vfx::Burst { life_ms: None, effects: vec!["VFX_Trap_Electro_Impact".into()], scale: 2.0, height: 0.0, distance: 0.0, ring: 1, radius: 0.0 };
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
    let b = vfx::Burst { life_ms: None, effects: vec!["VFX_Trap_Electro_Impact".into()], scale: 2.0, height: 0.0, distance: 0.0, ring: 1, radius: 0.0 };
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

#[test]
fn reads_spell_weapon_and_hand_of_the_player() {
    let _g = LOCK.lock().unwrap();
    let m = 4294967296.0f64;
    let mut player = [0u8; 56];
    player[0..8].copy_from_slice(&((100.0 * m) as i64).to_le_bytes());
    player[8..16].copy_from_slice(&((20.0 * m) as i64).to_le_bytes());
    player[16..24].copy_from_slice(&((-50.0 * m) as i64).to_le_bytes());
    let mut ranged = vec![0u8; 112];
    ranged[0] = 2;
    ranged[56..60].copy_from_slice(&777u32.to_le_bytes());
    ranged[68..72].copy_from_slice(&1048548069u32.to_le_bytes());
    let mut cursor = vec![7u8; 112];
    cursor[0] = 9;
    let mut slots = vec![0u8; 1760];
    slots[0..8].copy_from_slice(&(1u64 << 8).to_le_bytes());
    slots[8..16].copy_from_slice(&((100.0 * m) as i64).to_le_bytes());
    slots[16..24].copy_from_slice(&((20.0 * m) as i64).to_le_bytes());
    slots[24..32].copy_from_slice(&((-50.0 * m) as i64).to_le_bytes());
    let base = 32 + 8 * 48;
    let vals: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.3, 1.2, 0.4];
    for (k, v) in vals.iter().enumerate() {
        slots[base + k * 4..base + k * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    let w = world(1, 0, &[(5, 56, 0x100, player.to_vec()), (60, 112, 0x400, cursor), (61, 112, 0x800, ranged), (70, 1760, 0x1000, slots)]);
    let mgr = w.mem.as_ptr() as usize;
    spawner::TRANSFORM_INDEX.store(5, Ordering::SeqCst);
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    assert_eq!(vfx::ranged_items(mgr, 1), Some((1048548069, 777)));
    let (hand, axes) = vfx::slot_world(mgr, 1, 8).unwrap();
    assert!((hand.0 as f64 / m - 100.3).abs() < 1e-6);
    assert!((hand.1 as f64 / m - 21.2).abs() < 1e-6);
    assert!((hand.2 as f64 / m + 49.6).abs() < 1e-6);
    assert_eq!(axes[1], [0.0, 1.0, 0.0]);
    assert!(vfx::slot_world(mgr, 1, 9).is_none(), "slot not initialised");
}

#[test]
fn hand_glow_is_created_glued_to_the_players_right_hand() {
    let _g = LOCK.lock().unwrap();
    let w = world(1, 0, &[(5, 56, 0x200, vec![0u8; 56])]);
    let mgr = w.mem.as_ptr() as usize;
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawn_probe::set_trampoline(fake_create as usize);
    SEEN.lock().unwrap().clear();
    let t = vfx::hand_effect(vfx::Element::Poison).unwrap();
    vfx::queue_hand(t, 42, (7 << 32, 8 << 32, 9 << 32));
    vfx::server_tick();
    let seen = SEEN.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let (_, desc, data) = &seen[0];
    assert_eq!(&desc[4..20], &t);
    assert_eq!(u64::from_le_bytes(desc[0x20..0x28].try_into().unwrap()), 3);
    assert_eq!(&data[0..4], &0x027a_c564u32.to_le_bytes());
    let attach = &data[4 + 56..4 + 56 + 4 + 112];
    assert_eq!(&attach[0..4], &0x430f_162du32.to_le_bytes());
    let a = &attach[4..];
    assert_eq!(a[41], 8);
    assert_eq!(u32::from_le_bytes(a[44..48].try_into().unwrap()), 42);
    assert_eq!(i64::from_le_bytes(a[48..56].try_into().unwrap()), 7 << 32);
    let life = &data[4 + 56 + 4 + 112..];
    assert_eq!(&life[0..4], &0xf22c_253cu32.to_le_bytes());
    assert_eq!(u64::from_le_bytes(life[4..12].try_into().unwrap()), vfx::HAND_LIFE_MS * 1_000_000);
}

#[test]
fn greet_toggles_and_victory_switches_the_aura() {
    let sneak = READY | 1 << 21;
    let v = vfx::EMOTE_VICTORY;
    let g = vfx::EMOTE_GREET;
    let mut st = vfx::AuraState::default();
    assert_eq!(vfx::gesture_step(&mut st, READY, Some((5, 99)), 0), vfx::Gesture::None);
    assert_eq!(vfx::gesture_step(&mut st, sneak, Some((5, 99)), 10), vfx::Gesture::None, "crouching does nothing now");
    assert!(!st.on);
    assert_eq!(vfx::gesture_step(&mut st, READY, Some((6, g)), 20), vfx::Gesture::Toggle);
    assert!(st.on);
    assert_eq!(vfx::gesture_step(&mut st, READY, Some((6, g)), 30), vfx::Gesture::None, "same emote seen again");
    assert_eq!(vfx::gesture_step(&mut st, READY, Some((7, v)), 40), vfx::Gesture::Next);
    assert_eq!(st.index, 1);
    assert_eq!(vfx::gesture_step(&mut st, READY, Some((8, 12345)), 50), vfx::Gesture::None, "another emote");
    assert_eq!(vfx::gesture_step(&mut st, READY, Some((9, v)), 60), vfx::Gesture::Next);
    assert_eq!(st.index, 2);
    assert_eq!(vfx::gesture_step(&mut st, READY, Some((10, g)), 70), vfx::Gesture::Toggle);
    assert!(!st.on);
    assert_eq!(vfx::gesture_step(&mut st, READY, Some((11, v)), 80), vfx::Gesture::Next);
    assert!(st.on);
    assert_eq!(st.index, 2, "turning on with victory keeps the last effect");
    let mut fresh = vfx::AuraState::default();
    assert_eq!(vfx::gesture_step(&mut fresh, READY, Some((3, g)), 0), vfx::Gesture::None, "state already present when first seen");
    for a in vfx::AURAS.iter().chain(vfx::GLOW_AURAS.iter()) {
        assert!(!a.effects.is_empty() || a.glow.and_then(vfx::glow_template).is_some());
        for name in a.effects {
            assert!(vfx::is_vanilla(name), "{name} must be a game effect");
        }
    }
    vfx::set_glows_ready(false);
    assert_eq!(vfx::aura_list().len(), 7);
    vfx::set_glows_ready(true);
    assert_eq!(vfx::aura_list().len(), 16);
    vfx::set_glows_ready(false);
}

#[test]
fn lingering_effects_carry_a_longer_life_and_daggers_are_found_in_any_hand() {
    let _g = LOCK.lock().unwrap();
    let mut equipment = vec![0u8; 1380];
    equipment[16..20].copy_from_slice(&1u32.to_le_bytes());
    equipment[8 + 72 + 8..8 + 72 + 12].copy_from_slice(&2222u32.to_le_bytes());
    let w = world(1, 0, &[(5, 56, 0x200, vec![0u8; 56]), (40, 1380, 0x2000, equipment)]);
    let mgr = w.mem.as_ptr() as usize;
    let mut items = std::collections::HashMap::new();
    items.insert(2222u32, vfx::ItemKind { element: vfx::Element::Poison, wand: false, weapon: true });
    vfx::set_items(items);
    assert_eq!(vfx::main_hand(mgr, 1).map(|(_, i)| i), Some(2222));
    spawner::SERVER_MGR.store(mgr, Ordering::SeqCst);
    spawn_probe::set_trampoline(fake_create as usize);
    SEEN.lock().unwrap().clear();
    let b = vfx::Burst { life_ms: Some(1200), effects: vec!["VFX_Spell_ManaDrain_Mana_OnPlayer".into()], scale: 1.5, height: 0.0, distance: 0.0, ring: 1, radius: 0.0 };
    vfx::queue_burst((0, 0, 0), (0.0, 1.0), &b);
    vfx::server_tick();
    let seen = SEEN.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let (_, desc, data) = &seen[0];
    assert_eq!(u64::from_le_bytes(desc[0x20..0x28].try_into().unwrap()), 2);
    assert_eq!(&data[60..64], &0xf22c_253cu32.to_le_bytes());
    assert_eq!(u64::from_le_bytes(data[64..72].try_into().unwrap()), 1_200_000_000);
    vfx::set_items(std::collections::HashMap::new());
}

#[test]
fn installed_by_config_in_any_case_or_by_the_lua_mod() {
    let base = std::env::temp_dir().join(format!("m182vfx{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    assert!(!vfx::installed(&base));
    std::fs::write(base.join("M182_VFX.json"), "{}").unwrap();
    assert!(vfx::installed(&base));
    assert_eq!(vfx::config_path(&base).unwrap().file_name().unwrap(), "M182_VFX.json");
    std::fs::remove_file(base.join("M182_VFX.json")).unwrap();
    std::fs::create_dir_all(base.join("mods").join("m182_vfx")).unwrap();
    std::fs::write(base.join("mods").join("m182_vfx").join("mod.json"), "{}").unwrap();
    assert!(vfx::installed(&base));
    assert!(vfx::config_path(&base).is_none());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn custom_glows_need_the_current_lua_mod_next_to_the_game() {
    let dir = std::env::temp_dir().join(format!("m182_vfx_glow_{}", std::process::id()));
    let src = dir.join("mods").join("m182_vfx").join("src");
    std::fs::create_dir_all(&src).unwrap();
    assert!(!vfx::glow_mod_installed(&dir));
    std::fs::write(dir.join("mods").join("m182_vfx").join("mod.json"), "{}").unwrap();
    std::fs::write(src.join("mod.lua"), "print('glow effects ready')").unwrap();
    assert!(!vfx::glow_mod_installed(&dir), "old Lua mod (custom templates) must not enable glows");
    let lua = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../spawner-mods/m182_vfx/src/mod.lua")).unwrap();
    assert!(lua.contains(vfx::GLOW_MOD_MARKER));
    for g in vfx::GLOW_CARRIERS {
        assert!(lua.contains(g), "{g} missing in the Lua mod");
    }
    std::fs::write(src.join("mod.lua"), &lua).unwrap();
    assert!(vfx::glow_mod_installed(&dir));
    let _ = std::fs::remove_dir_all(&dir);
}

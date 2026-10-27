//! M182 Veil Beam: while a player channels the Eternal Veil Beam, the server places big game
//! effects along it so every player sees a thick beam without installing anything.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::pillar_cfg::{BEAM_ITEMS, FLAG_SHOOTING};
use super::{pillar, spawner, vfx};

const CHECK_EVERY_MS: u64 = 100;
const SCAN_EVERY_S: u64 = 3;
const LOG_LIMIT: u32 = 40;

struct State {
    checked: Instant,
    scanned: Option<Instant>,
    players: Vec<u32>,
    last: HashMap<u32, Instant>,
    logged: u32,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Called on every server tick (the player-table pump).
pub fn tick() {
    let cfg = pillar::config();
    if !cfg.active() || !cfg.beam {
        return;
    }
    let Ok(mut guard) = STATE.lock() else { return };
    let st = guard.get_or_insert_with(|| State { checked: Instant::now() - Duration::from_secs(1), scanned: None, players: Vec::new(), last: HashMap::new(), logged: 0 });
    if st.checked.elapsed() < Duration::from_millis(CHECK_EVERY_MS) {
        return;
    }
    st.checked = Instant::now();
    let manager = spawner::server_manager();
    if manager == 0 {
        return;
    }
    if st.scanned.map(|t| t.elapsed() >= Duration::from_secs(SCAN_EVERY_S)).unwrap_or(true) {
        st.scanned = Some(Instant::now());
        let mut players = spawner::player_entities(manager);
        for id in spawner::tracked_players(manager) {
            if !players.contains(&id) {
                players.push(id);
            }
        }
        players.sort_unstable();
        st.last.retain(|id, _| players.contains(id));
        st.players = players;
    }
    let templates: Vec<[u8; 16]> = cfg.beam_effects.iter().filter_map(|n| vfx::template_guid(n)).collect();
    if templates.is_empty() {
        return;
    }
    let every = Duration::from_millis(cfg.beam_every_ms.clamp(200, 2000));
    for &id in &st.players {
        if st.last.get(&id).map(|t| t.elapsed() < every).unwrap_or(false) {
            continue;
        }
        let Some(flags) = vfx::read_flags(manager, id) else { continue };
        if flags & (1 << FLAG_SHOOTING) == 0 || flags & (1 << vfx::FLAG_DEAD) != 0 {
            continue;
        }
        let Some((ammo, _)) = vfx::ranged_items(manager, id) else { continue };
        if !BEAM_ITEMS.contains(&ammo) {
            continue;
        }
        let Some(t) = spawner::transform_of(manager, id) else { continue };
        if t.len() < 24 {
            continue;
        }
        let origin = (le_i64(&t, 0), le_i64(&t, 8), le_i64(&t, 16));
        let facing = spawner::facing_direction(&t);
        st.last.insert(id, Instant::now());
        let life = cfg.beam_life_ms();
        let size = cfg.beam_size();
        for point in cfg.beam_points(origin, facing) {
            for template in &templates {
                vfx::queue_effect(*template, point, size, life);
            }
        }
        if st.logged < LOG_LIMIT {
            st.logged += 1;
            pillar::log(&format!("veil beam: entity {id} channels the beam (item {ammo}, facing {:.2},{:.2}), {} spot(s)\n", facing.0, facing.1, cfg.beam_points(origin, facing).len()));
        }
    }
}

fn le_i64(b: &[u8], at: usize) -> i64 {
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&b[at..at + 8]);
    i64::from_le_bytes(raw)
}

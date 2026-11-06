//! The panel's chat lines, played into the server's chat channel exactly as the game would pass them.
use linuxtest::native::chat::{self, Session, Verdict};
use std::time::Instant;

#[path = "../../../../spawner-panel/src/remote_proto.rs"]
#[allow(dead_code)]
mod proto;

const KEY: &str = "m182-7dde183b";

static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn play(lines: &[String], dir: &std::path::Path) {
    let mut session = Session::default();
    for line in lines {
        let verdict = chat::handle_with(&mut session, Instant::now(), line, KEY, 64, dir);
        match verdict {
            Verdict::Swallowed(note) => assert!(note.contains(" ok") || (line.contains(" endx ") && note.contains("nothing buffered")), "{line}: {note}"),
            Verdict::Pass | Verdict::Roster(_) | Verdict::Teams(_) => panic!("not recognised: {line}"),
        }
    }
}

#[test]
fn every_file_the_panel_forwards_arrives_intact() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    chat::flush_to_files(&std::env::temp_dir());
    let dir = std::env::temp_dir().join(format!("m182_protocol_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let bodies = [
        ("m182_spawn.request", serde_json::json!({"template": "c3db66d6-89a5-4a6c-a8e3-d9a1f6009812", "count": 3, "level": 20, "distance": 5.0}).to_string()),
        (
            "m182_command.request",
            serde_json::json!({"cmd": "item_copy", "base": 1234, "slot": 2, "dmg": "1,3,99,0,0,0,0,0,0,0,0,0,0,0,0", "lvl": [1, 50], "label": "Espada de Carlos ñandú"}).to_string(),
        ),
        (
            "m182_zones.json",
            serde_json::json!({"zones": (0..9).map(|i| serde_json::json!({"name": format!("Zona número {i}"), "x": i * 100, "z": -i * 33, "radius": 20})).collect::<Vec<_>>()}).to_string(),
        ),
        ("m182_teams.json", serde_json::json!({"teams": [{"name": "Equipo Ámbar", "members": [1, 2]}]}).to_string()),
    ];
    for limit in [80usize, 200, 900] {
        for (file, body) in &bodies {
            let _ = std::fs::remove_file(dir.join(file));
            play(&proto::compose(KEY, file, body, limit), &dir);
            assert_eq!(chat::flush_to_files(&dir), usize::from(file.ends_with(".request")), "{file}");
            let written = std::fs::read_to_string(dir.join(file)).unwrap();
            assert_eq!(written, *body, "{file} at limit {limit}");
        }
    }
    let mut session = Session::default();
    assert!(matches!(chat::handle_with(&mut session, Instant::now(), &proto::ping(KEY), KEY, 64, &dir), Verdict::Swallowed(n) if n.contains("ok")));
    assert!(matches!(chat::handle_with(&mut session, Instant::now(), &proto::pvp(KEY, true), KEY, 64, &dir), Verdict::Swallowed(n) if n.contains("ok")));
    assert!(std::fs::read_to_string(dir.join("m182_native.json")).unwrap().contains("\"pvp\": true"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn powers_follow_the_sender_and_the_player_list_is_requested_by_chat() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("m182_protocol_clients_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let body = r#"{"god":true,"stamina":true}"#;
    let mut session = Session::default();
    for line in proto::compose(KEY, "m182_powers.json", body, 200) {
        match chat::handle_with(&mut session, Instant::now(), &line, KEY, 65, &dir) {
            Verdict::Swallowed(note) => assert!(note.contains(" ok"), "{note}"),
            _ => panic!("not recognised: {line}"),
        }
    }
    assert_eq!(std::fs::read_to_string(dir.join("m182_powers_65.json")).unwrap(), body);
    assert!(!dir.join("m182_powers.json").exists(), "the server's own powers file belongs to the host");
    assert!(matches!(chat::handle_with(&mut session, Instant::now(), &proto::players(KEY, 12), KEY, 65, &dir), Verdict::Roster(12)));
    assert!(matches!(chat::handle_with(&mut session, Instant::now(), &proto::players("other-key", 12), KEY, 65, &dir), Verdict::Pass | Verdict::Swallowed(_)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn item_settings_reach_the_mod_folder_of_the_server() {
    let dir = std::env::temp_dir().join(format!("m182_protocol_data_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let lua = "return{overrides={[1234]={dmg={1,5,33.5,3,0,7,0,0,0,0,0,0,0,0,0},},},levels={[1234]={3,255},},balance={[\"freeSkills\"]=1,}}\n";
    let body = serde_json::json!({ "lua": lua }).to_string();
    let target = dir.join("mods").join("zz_m182_item_stats").join("src").join("m182_data.lua");
    for limit in [80usize, 900] {
        let _ = std::fs::remove_dir_all(dir.join("mods"));
        play(&proto::compose(KEY, "m182_data.json", &body, limit), &dir);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), lua, "limit {limit}");
        assert!(!dir.join("m182_data.json").exists());
    }
    let mut session = Session::default();
    for bad in [serde_json::json!({"lua": "os.execute('x')"}), serde_json::json!({"lua": 5}), serde_json::json!({"other": "return{}"})] {
        let before = std::fs::read_to_string(&target).unwrap();
        for line in proto::compose(KEY, "m182_data.json", &bad.to_string(), 900) {
            let _ = chat::handle_with(&mut session, Instant::now(), &line, KEY, 64, &dir);
        }
        assert_eq!(std::fs::read_to_string(&target).unwrap(), before, "a refused body leaves the file alone: {bad}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

fn send(session: &mut Session, client: u32, line: &str, dir: &std::path::Path) -> Verdict {
    chat::handle_with(session, Instant::now(), line, KEY, client, dir)
}

#[test]
fn two_admins_sending_the_same_file_at_once_do_not_mix_their_chunks() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("m182_protocol_two_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let a = serde_json::json!({"zones": (0..12).map(|i| serde_json::json!({"name": format!("Alfa {i}"), "x": i, "z": -i, "radius": 20})).collect::<Vec<_>>()}).to_string();
    let b = serde_json::json!({"zones": (0..12).map(|i| serde_json::json!({"name": format!("Beta {i}"), "x": i * 2, "z": i, "radius": 30})).collect::<Vec<_>>()}).to_string();
    let (la, lb) = (proto::compose(KEY, "m182_zones.json", &a, 80), proto::compose(KEY, "m182_zones.json", &b, 80));
    assert!(la.len() > 2 && lb.len() > 2);
    let mut session = Session::default();
    let mut ia = la.iter();
    let mut ib = lb.iter();
    loop {
        let (x, y) = (ia.next(), ib.next());
        if x.is_none() && y.is_none() {
            break;
        }
        if let Some(line) = x {
            assert!(matches!(send(&mut session, 3, line, &dir), Verdict::Swallowed(_)));
        }
        if let Some(line) = y {
            assert!(matches!(send(&mut session, 9, line, &dir), Verdict::Swallowed(_)));
        }
    }
    let written = std::fs::read_to_string(dir.join("m182_zones.json")).unwrap();
    assert!(written == a || written == b, "the file must be one admin's whole message, never a mix");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn orders_from_several_admins_wait_in_order_with_their_sender() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    chat::flush_to_files(&std::env::temp_dir());
    let dir = std::env::temp_dir().join(format!("m182_protocol_orders_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut session = Session::default();
    let spawn = |n: u32| serde_json::json!({"template": "c3db66d6-89a5-4a6c-a8e3-d9a1f6009812", "count": n}).to_string();
    for (client, n) in [(3u32, 1u32), (9, 2), (3, 3), (12, 4)] {
        let lines = proto::compose(KEY, "m182_spawn.request", &spawn(n), 200);
        for line in &lines {
            assert!(matches!(send(&mut session, client, line, &dir), Verdict::Swallowed(_)));
        }
    }
    assert_eq!(chat::queued(), 4);
    assert!(!dir.join("m182_spawn.request").exists());
    let mut seen = Vec::new();
    while let Some(item) = chat::take_request(|_| true) {
        let count = serde_json::from_str::<serde_json::Value>(&item.body).unwrap()["count"].as_u64().unwrap();
        seen.push((item.client, count));
    }
    assert_eq!(seen, vec![(3, 1), (9, 2), (3, 3), (12, 4)]);
    assert_eq!(chat::queued(), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_spawn_waits_its_turn_without_blocking_the_commands_behind_it_only_when_busy() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    chat::flush_to_files(&std::env::temp_dir());
    let dir = std::env::temp_dir().join(format!("m182_protocol_busy_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut session = Session::default();
    for (file, body) in [("m182_spawn.request", r#"{"template":"c3db66d6-89a5-4a6c-a8e3-d9a1f6009812"}"#), ("m182_command.request", r#"{"cmd":"heal"}"#)] {
        for line in proto::compose(KEY, file, body, 200) {
            send(&mut session, 5, &line, &dir);
        }
    }
    assert!(chat::take_request(|file| file != chat::SPAWN_REQUEST).is_none(), "the oldest order is the spawn: nothing jumps ahead of it");
    assert_eq!(chat::queued(), 2);
    assert_eq!(chat::take_request(|_| true).unwrap().file, chat::SPAWN_REQUEST);
    assert_eq!(chat::take_request(|_| true).unwrap().file, chat::COMMAND_REQUEST);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn one_admin_with_a_wrong_key_does_not_lock_the_others_out() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("m182_protocol_keys_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut session = Session::default();
    for _ in 0..20 {
        let verdict = chat::handle_with(&mut session, Instant::now(), "!m182 wrong-key-here ping", KEY, 4, &dir);
        assert!(matches!(verdict, Verdict::Swallowed(_)));
    }
    let blocked = chat::handle_with(&mut session, Instant::now(), &proto::ping(KEY), KEY, 4, &dir);
    assert!(matches!(blocked, Verdict::Swallowed(ref n) if n.contains("too many wrong keys")));
    let other = chat::handle_with(&mut session, Instant::now(), &proto::ping(KEY), KEY, 8, &dir);
    assert!(matches!(other, Verdict::Swallowed(ref n) if n.contains(" ok")), "{other:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn waiting_for_work_ends_as_soon_as_an_order_arrives() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    chat::flush_to_files(&std::env::temp_dir());
    let dir = std::env::temp_dir().join(format!("m182_protocol_wake_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let handle = {
        let dir = dir.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let mut session = Session::default();
            for line in proto::compose(KEY, "m182_command.request", r#"{"cmd":"heal"}"#, 200) {
                chat::handle_with(&mut session, Instant::now(), &line, KEY, 2, &dir);
            }
        })
    };
    let start = Instant::now();
    chat::wait_for_work(std::time::Duration::from_secs(5));
    let waited = start.elapsed();
    handle.join().unwrap();
    assert!(waited < std::time::Duration::from_secs(2), "{waited:?}");
    assert_eq!(chat::queued(), 1);
    chat::flush_to_files(&dir);
    let _ = std::fs::remove_dir_all(dir);
}

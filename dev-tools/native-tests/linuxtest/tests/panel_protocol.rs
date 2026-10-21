//! The panel's chat lines, played into the server's chat channel exactly as the game would pass them.
use linuxtest::native::chat::{self, Session, Verdict};
use std::time::Instant;

#[path = "../../../../spawner-panel/src/remote_proto.rs"]
#[allow(dead_code)]
mod proto;

const KEY: &str = "m182-7dde183b";

fn play(lines: &[String], dir: &std::path::Path) {
    let mut session = Session::default();
    for line in lines {
        let verdict = chat::handle_with(&mut session, Instant::now(), line, KEY, 64, dir);
        match verdict {
            Verdict::Swallowed(note) => assert!(note.contains(" ok") || (line.contains(" endx ") && note.contains("nothing buffered")), "{line}: {note}"),
            Verdict::Pass | Verdict::Roster(_) => panic!("not recognised: {line}"),
        }
    }
}

#[test]
fn every_file_the_panel_forwards_arrives_intact() {
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

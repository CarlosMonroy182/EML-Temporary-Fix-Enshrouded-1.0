use std::path::PathBuf;

use linuxtest::native::chat::{self, Verdict};
use linuxtest::native::shared;
use serde_json::{json, Value};

const KEY: &str = "secret-key";

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("m182_state_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hex(text: &str) -> String {
    text.bytes().map(|b| format!("{b:02x}")).collect()
}

fn send(dir: &PathBuf, client: u32, op: Value) -> Verdict {
    chat::handle(&format!("!m182 {KEY} setx m182_state_op.json {}", hex(&op.to_string())), KEY, client, dir)
}

fn lines(v: Verdict) -> Vec<String> {
    match v {
        Verdict::Lines(l) => l,
        other => panic!("expected lines, got {other:?}"),
    }
}

#[test]
fn a_clean_install_learns_everything_the_first_admin_built() {
    let server = scratch("server");
    let client = scratch("clean");
    let zone = json!({"name": "Base norte", "x": 10.0, "z": 20.0, "radius": 40.0, "enabled": true, "safe": true, "team": 1, "lock": true, "lock_admins": false, "pvp": false});
    let place = json!({"name": "Torre", "x": 1.0, "y": 2.0, "z": 3.0});
    let team = json!({"name": "Rojo", "people": [{"key": "name:Ana", "name": "Ana"}], "leaders": ["name:Ana"]});
    let template = json!({"name": "Horda", "active": true, "members": [{"creature": "Lobo", "template": "a", "level": 5, "count": 2, "health": 0, "damage": 1.0, "drops": []}]});
    for op in [
        json!({"t": "zones", "add": [zone]}),
        json!({"t": "places", "add": [place]}),
        json!({"t": "teams", "add": [team]}),
        json!({"t": "templates", "add": [template]}),
    ] {
        let Verdict::Swallowed(said) = send(&server, 3, op) else { panic!() };
        assert!(said.contains("ok:"), "{said}");
    }
    std::fs::write(server.join("m182_spawners.json"), json!({"spawners": [{"id": "s1", "template": "df787789-d005-42e8-a18f-f4fe2e5d2e42", "at": [1.0, 2.0, 3.0]}]}).to_string()).unwrap();

    let rev = lines(chat::handle(&format!("!m182 {KEY} rev 11"), KEY, 9, &server));
    assert_eq!(rev.len(), 1);
    shared::receive_revs(&client, &rev[0]);
    let revs: Value = serde_json::from_str(&std::fs::read_to_string(client.join("m182_remote_rev.json")).unwrap()).unwrap();
    for topic in ["zones", "places", "spawners", "teams", "templates"] {
        assert_eq!(revs["rev"][topic].as_str().unwrap().len(), 8, "{topic}");
    }

    for topic in ["zones", "places", "spawners", "teams", "templates"] {
        for line in lines(chat::handle(&format!("!m182 {KEY} pull 12 {topic}"), KEY, 9, &server)) {
            shared::receive_snapshot(&client, &line);
        }
        let got: Value = serde_json::from_str(&std::fs::read_to_string(client.join(format!("m182_remote_{topic}.json"))).unwrap()).unwrap();
        assert_eq!(got["hash"], revs["rev"][topic], "{topic} matches the revision the server announced");
    }
    let teams: Value = serde_json::from_str(&std::fs::read_to_string(client.join("m182_remote_teams.json")).unwrap()).unwrap();
    assert_eq!(teams["data"]["teams"][0]["name"], "Rojo");
    let zones: Value = serde_json::from_str(&std::fs::read_to_string(client.join("m182_remote_zones.json")).unwrap()).unwrap();
    assert_eq!(zones["data"]["zones"][0]["name"], "Base norte");
}

#[test]
fn a_wrong_key_or_topic_gets_nothing() {
    let dir = scratch("wrong");
    let Verdict::Swallowed(said) = chat::handle("!m182 not-the-key pull 1 zones", KEY, 4, &dir) else { panic!() };
    assert!(said.contains("wrong key"), "{said}");
    let Verdict::Swallowed(said) = chat::handle(&format!("!m182 {KEY} pull 1 secrets"), KEY, 4, &dir) else { panic!() };
    assert!(said.contains("unknown topic"), "{said}");
    assert!(!chat::authenticated("!m182 not-the-key pull 1 zones", KEY));
    assert!(chat::authenticated(&format!("/m182 {KEY} rev 1"), KEY));
    assert!(chat::authenticated("/M182 SECRET-KEY rev 1", "secret-key"), "Caps Lock flips the letters");
}

#[test]
fn state_changes_arrive_whole_even_when_split_into_several_lines() {
    let dir = scratch("split");
    let big: Vec<Value> = (0..40).map(|i| json!({"name": format!("Zona {i}"), "x": i as f64, "z": 1.0, "radius": 25.0})).collect();
    let body = json!({"t": "zones", "add": big}).to_string();
    let chunks: Vec<&str> = body.as_bytes().chunks(300).map(|c| std::str::from_utf8(c).unwrap()).collect();
    for (i, part) in chunks.iter().enumerate() {
        let command = if i == 0 { "put" } else { "add" };
        chat::handle(&format!("!m182 {KEY} {command} m182_state_op.json {part}"), KEY, 2, &dir);
    }
    let Verdict::Swallowed(said) = chat::handle(&format!("!m182 {KEY} end m182_state_op.json"), KEY, 2, &dir) else { panic!() };
    assert!(said.contains("zones: 40 now"), "{said}");
}

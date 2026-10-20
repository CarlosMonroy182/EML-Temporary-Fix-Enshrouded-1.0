//! Persistent creature spawners: a creature tied to a saved place that comes back by itself
//! (every few minutes after it dies, and after every restart of the game).

use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::write_atomic;

pub const SPAWNERS_FILE: &str = "m182_spawners.json";
pub const OP_FILE: &str = "m182_spawner_op.json";
pub const STATUS_FILE: &str = "m182_spawner_status.json";
pub const DEFAULT_RESPAWN_MINUTES: u32 = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct Spawner {
    pub id: String,
    pub creature: String,
    pub template: String,
    pub place: String,
    pub at: [f64; 3],
    pub level: u32,
    pub count: u32,
    pub health: u32,
    pub damage: f32,
    pub radius: f32,
    pub respawn: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Live {
    pub alive: u32,
    pub want: u32,
    pub state: String,
    pub next: Option<u64>,
    pub spawned: u32,
}

pub fn new_id() -> String {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("s{ms}")
}

pub fn to_json(list: &[Spawner]) -> Value {
    let rows: Vec<Value> = list
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "name": s.creature,
                "template": s.template,
                "place": s.place,
                "at": s.at,
                "level": s.level,
                "count": s.count,
                "health": s.health,
                "damage": s.damage,
                "radius": s.radius,
                "respawn": s.respawn,
            })
        })
        .collect();
    json!({ "spawners": rows })
}

fn round(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// What the server needs of one spawner, as short as it can be (it goes through the game chat).
fn wire_row(s: &Spawner) -> Value {
    json!({
        "i": s.id,
        "t": s.template.replace('-', ""),
        "p": [round(s.at[0]), round(s.at[1]), round(s.at[2])],
        "l": s.level,
        "c": s.count,
        "h": s.health,
        "d": (f64::from(s.damage) * 100.0).round() / 100.0,
        "r": (f64::from(s.radius) * 10.0).round() / 10.0,
        "w": s.respawn,
    })
}

/// The server learns about one new spawner without being sent the whole list again.
pub fn op_add(s: &Spawner) -> String {
    json!({ "add": wire_row(s) }).to_string()
}

pub fn op_delete(id: &str) -> String {
    json!({ "rm": id }).to_string()
}

/// Whole list for the server: the first message replaces what it has, the rest add to it.
pub fn op_replace_all(list: &[Spawner], per_message: usize) -> Vec<String> {
    let rows: Vec<Value> = list.iter().map(wire_row).collect();
    if rows.is_empty() {
        return vec![json!({ "set": [] }).to_string()];
    }
    rows.chunks(per_message.max(1)).enumerate().map(|(i, part)| json!({ if i == 0 { "set" } else { "add" }: part }).to_string()).collect()
}

/// Saves the list next to the game and, when the panel drives a server, tells it only what changed.
pub fn save_change(dir: &Path, list: &[Spawner], op: &str) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&to_json(list)).map_err(|e| e.to_string())?;
    crate::write_local(dir, SPAWNERS_FILE, "m182_spawners.json.tmp", &text)?;
    if crate::remote::route(SPAWNERS_FILE) != crate::remote::Route::Local {
        crate::remote::submit(OP_FILE, op)?;
    }
    Ok(())
}

pub fn parse(text: &str) -> Vec<Spawner> {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let Some(rows) = v.get("spawners").and_then(|r| r.as_array()) else { return Vec::new() };
    rows.iter()
        .filter_map(|r| {
            let at = r.get("at")?.as_array()?;
            if at.len() != 3 {
                return None;
            }
            let num = |i: usize| at[i].as_f64().filter(|x| x.is_finite());
            Some(Spawner {
                id: r.get("id")?.as_str()?.to_string(),
                creature: r.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                template: r.get("template")?.as_str()?.to_string(),
                place: r.get("place").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                at: [num(0)?, num(1)?, num(2)?],
                level: r.get("level").and_then(|x| x.as_u64()).unwrap_or(1).clamp(1, 1000) as u32,
                count: r.get("count").and_then(|x| x.as_u64()).unwrap_or(1).clamp(1, 20) as u32,
                health: r.get("health").and_then(|x| x.as_u64()).unwrap_or(0).min(2_000_000_000) as u32,
                damage: r.get("damage").and_then(|x| x.as_f64()).unwrap_or(1.0).clamp(0.0, 1000.0) as f32,
                radius: r.get("radius").and_then(|x| x.as_f64()).unwrap_or(3.0).clamp(0.5, 60.0) as f32,
                respawn: r.get("respawn").and_then(|x| x.as_u64()).unwrap_or(300).clamp(10, 86_400) as u32,
            })
        })
        .collect()
}

pub fn load(dir: &Path) -> Vec<Spawner> {
    std::fs::read_to_string(dir.join(SPAWNERS_FILE)).map(|t| parse(&t)).unwrap_or_default()
}

pub fn save(dir: &Path, list: &[Spawner]) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&to_json(list)).map_err(|e| e.to_string())?;
    write_atomic(dir, SPAWNERS_FILE, "m182_spawners.json.tmp", &text)
}

pub fn parse_status(text: &str) -> HashMap<String, Live> {
    let mut out = HashMap::new();
    let Ok(v) = serde_json::from_str::<Value>(text) else { return out };
    let Some(rows) = v.get("spawners").and_then(|r| r.as_array()) else { return out };
    for r in rows {
        let Some(id) = r.get("id").and_then(|x| x.as_str()) else { continue };
        out.insert(
            id.to_string(),
            Live {
                alive: r.get("alive").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                want: r.get("want").and_then(|x| x.as_u64()).unwrap_or(1) as u32,
                state: r.get("state").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                next: r.get("next").and_then(|x| x.as_u64()),
                spawned: r.get("spawned").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
            },
        );
    }
    out
}

pub fn load_status(dir: &Path) -> HashMap<String, Live> {
    std::fs::read_to_string(dir.join(STATUS_FILE)).map(|t| parse_status(&t)).unwrap_or_default()
}

pub fn clock(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

pub const RANGE_METERS: f64 = 150.0;

/// How far this spawner is from the player and whether that is close enough for it to be active.
pub fn distance_note(player: Option<[f64; 3]>, at: [f64; 3]) -> Option<String> {
    let p = player?;
    let d = ((p[0] - at[0]).powi(2) + (p[2] - at[2]).powi(2)).sqrt();
    let shown = d.round() as i64;
    Some(if d <= RANGE_METERS { format!("A {shown} m de ti: dentro del alcance") } else { format!("A {shown} m de ti: se activa a menos de 150 m") })
}

/// One short line for the list: how many are alive and when the next one comes.
pub fn describe(live: Option<&Live>) -> String {
    let Some(l) = live else { return "Esperando al juego…".to_string() };
    match l.state.as_str() {
        "vivo" => format!("Vivo ({}/{})", l.alive, l.want),
        "esperando para reaparecer" => match l.next {
            Some(n) => format!("Muerto: reaparece en {}", clock(n)),
            None => "Reaparecerá pronto".to_string(),
        },
        "creando" => "Apareciendo…".to_string(),
        "esperando el mundo" => "Esperando a que cargue el mundo".to_string(),
        "sin jugadores cerca" => "En reposo: se activa al acercarte".to_string(),
        _ => "Preparando…".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Spawner {
        Spawner {
            id: "s1".into(),
            creature: "Troll".into(),
            template: "642098d4-fc09-48e2-811e-782d438bf551".into(),
            place: "Cueva".into(),
            at: [10.5, 20.0, -3.0],
            level: 100,
            count: 2,
            health: 5000,
            damage: 1.5,
            radius: 4.0,
            respawn: 300,
        }
    }

    #[test]
    fn spawners_roundtrip_through_the_file_the_dll_reads() {
        let dir = std::env::temp_dir().join(format!("m182_spawners_panel_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        save(&dir, &[sample()]).unwrap();
        let text = std::fs::read_to_string(dir.join(SPAWNERS_FILE)).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["spawners"][0]["template"], "642098d4-fc09-48e2-811e-782d438bf551");
        assert_eq!(v["spawners"][0]["at"], json!([10.5, 20.0, -3.0]));
        assert_eq!(v["spawners"][0]["respawn"], 300);
        assert_eq!(load(&dir), vec![sample()]);
        save(&dir, &[]).unwrap();
        assert!(load(&dir).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn distance_tells_when_a_spawner_is_in_range() {
        assert_eq!(distance_note(None, [0.0, 0.0, 0.0]), None);
        assert_eq!(distance_note(Some([0.0, 5.0, 0.0]), [30.0, 9.0, 40.0]).unwrap(), "A 50 m de ti: dentro del alcance");
        assert_eq!(distance_note(Some([0.0, 5.0, 0.0]), [300.0, 9.0, 400.0]).unwrap(), "A 500 m de ti: se activa a menos de 150 m");
    }

    #[test]
    fn a_new_spawner_costs_only_a_couple_of_chat_lines() {
        let mut s = sample();
        s.id = "s1792524456177".into();
        s.template = "333ea93f-04f6-4bd2-b003-1582429dfb66".into();
        s.at = [3722.65, 842.38, 1367.64];
        s.count = 5;
        s.level = 20;
        s.health = 1800;
        s.damage = 1.1;
        s.radius = 4.0;
        s.respawn = 300;
        let op = op_add(&s);
        assert!(!op.contains("Troll") && !op.contains("Cueva"), "{op}");
        let lines = crate::remote_proto::compose("secret-key", OP_FILE, &op, crate::remote_proto::DEFAULT_LIMIT);
        assert!(lines.len() <= 5, "{} lines for {op}", lines.len());
        assert!(crate::remote_proto::compose("secret-key", OP_FILE, &op_delete("s1792524456177"), crate::remote_proto::DEFAULT_LIMIT).len() <= 2);
        let many: Vec<Spawner> = (0..50).map(|i| Spawner { id: format!("s{i}"), ..s.clone() }).collect();
        let all = op_replace_all(&many, 4);
        assert_eq!(all.len(), 13);
        assert!(all[0].starts_with(r#"{"set":"#) && all[1].starts_with(r#"{"add":"#));
        assert_eq!(op_replace_all(&[], 4), vec![r#"{"set":[]}"#.to_string()]);
    }

    #[test]
    fn status_text_is_readable() {
        let st = parse_status(r#"{"spawners":[{"id":"a","alive":1,"want":1,"state":"vivo","next":null,"spawned":3},{"id":"b","alive":0,"want":1,"state":"esperando para reaparecer","next":200,"spawned":1}]}"#);
        assert_eq!(describe(st.get("a")), "Vivo (1/1)");
        assert_eq!(describe(st.get("b")), "Muerto: reaparece en 3:20");
        assert_eq!(describe(st.get("c")), "Esperando al juego…");
        let far = Live { state: "sin jugadores cerca".into(), alive: 0, want: 1, next: None, spawned: 0 };
        assert_eq!(describe(Some(&far)), "En reposo: se activa al acercarte");
        assert!(parse_status("nonsense").is_empty());
    }

    #[test]
    fn ids_are_unique_enough() {
        assert!(new_id().starts_with('s'));
    }
}

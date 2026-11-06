//! The server is the one place where zones, saved places, creature spawners, teams and raid templates
//! really live. Every panel that connects reads them from there, and whatever an admin changes travels
//! as small per-item operations, so several admins can work at once without undoing each other.
//!
//! * On connecting the panel asks for every topic (`pull`), once.
//! * The server tells the connected panels when something changed, so nobody asks over and over;
//!   a short safety check every few minutes covers a notice that got lost.
//! * A local save is compared with what the server last said and only the difference is sent.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Value};

use crate::remote;

pub const OP_FILE: &str = "m182_state_op.json";
pub const REV_FILE: &str = "m182_remote_rev.json";
pub const TOPICS: [&str; 5] = ["zones", "places", "spawners", "teams", "templates"];
pub const STATUS: &str = "status";

const ASK_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_TRIES: u8 = 3;
const SAFETY_CHECK: Duration = Duration::from_secs(240);
const STATUS_EVERY: Duration = Duration::from_secs(20);
const NO_ANSWER: Duration = Duration::from_secs(15);
const LEGACY_PROBE: Duration = Duration::from_secs(300);

/// What the panel keeps in a file for each topic that is shared.
pub fn topic_of_file(file: &str) -> Option<&'static str> {
    match file {
        "m182_zones.json" => Some("zones"),
        "m182_places.json" => Some("places"),
        "m182_teams.json" => Some("teams"),
        "m182_raid_templates.json" => Some("templates"),
        _ => None,
    }
}

/// Files the panel no longer sends whole while it drives a server (their changes go as operations).
pub fn covers(file: &str) -> bool {
    topic_of_file(file).is_some()
}

fn lower(text: &str) -> String {
    text.trim().to_lowercase()
}

/// `(key, item)` of every entry of a topic, in the panel's own file shape and brought to one canonical form
/// so that two copies of the same thing compare equal.
pub fn items(topic: &str, text: &str) -> Vec<(String, Value)> {
    let value: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    match topic {
        "zones" => {
            let zones = crate::social::parse_zones(text);
            crate::social::zones_json(&zones)["zones"].as_array().cloned().unwrap_or_default().into_iter().filter_map(|z| Some((lower(z.get("name")?.as_str()?), z))).collect()
        }
        "places" => value
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|p| {
                        let name = p.get("name")?.as_str()?.to_string();
                        let item = json!({"name": name, "x": p.get("x")?.as_f64()?, "y": p.get("y")?.as_f64()?, "z": p.get("z")?.as_f64()?});
                        Some((lower(&name), item))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        "teams" => crate::social::parse_teams(text)
            .into_iter()
            .map(|t| {
                let mut people: Vec<Value> = t.members.iter().map(|p| json!({"key": p.key, "name": p.name})).collect();
                people.sort_by_key(|p| lower(p["key"].as_str().unwrap_or("")));
                let mut leaders: Vec<String> = t.leaders.iter().filter(|k| t.members.iter().any(|m| &m.key == *k)).cloned().collect();
                leaders.sort_by_key(|k| lower(k));
                (lower(&t.name), json!({"name": t.name, "people": people, "leaders": leaders}))
            })
            .collect(),
        "templates" => crate::spawners::templates_json(&crate::spawners::parse_templates(value.get("templates")))
            .into_iter()
            .filter_map(|mut t| {
                let name = t.get("name")?.as_str()?.trim().to_string();
                let name = if name.is_empty() { "(sin nombre)".to_string() } else { name };
                t["name"] = json!(name);
                Some((lower(&name), t))
            })
            .collect(),
        "spawners" => value
            .get("spawners")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(|r| Some((r.get("id")?.as_str()?.to_string(), r.clone()))).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Diff {
    pub add: Vec<Value>,
    pub up: Vec<Value>,
    pub rm: Vec<String>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.up.is_empty() && self.rm.is_empty()
    }

    pub fn op(&self, topic: &str) -> String {
        let mut op = json!({ "t": topic });
        if !self.add.is_empty() {
            op["add"] = Value::Array(self.add.clone());
        }
        if !self.up.is_empty() {
            op["up"] = Value::Array(self.up.clone());
        }
        if !self.rm.is_empty() {
            op["rm"] = json!(self.rm);
        }
        op.to_string()
    }
}

/// What has to be told to the server so that `known` becomes `wanted`.
pub fn diff(known: &[(String, Value)], wanted: &[(String, Value)]) -> Diff {
    let mut d = Diff::default();
    for (key, item) in wanted {
        match known.iter().find(|(k, _)| k == key) {
            None => d.add.push(item.clone()),
            Some((_, old)) if old != item => d.up.push(item.clone()),
            Some(_) => {}
        }
    }
    for (key, item) in known {
        if !wanted.iter().any(|(k, _)| k == key) {
            d.rm.push(item.get("name").and_then(Value::as_str).unwrap_or(key).to_string());
        }
    }
    d
}

struct Out {
    seq: u32,
    at: Instant,
    tries: u8,
    /// Something was sent after the question: the answer may not include it.
    stale: bool,
}

#[derive(Default)]
struct Core {
    dir: Option<std::path::PathBuf>,
    started: bool,
    snap: HashMap<String, Vec<(String, Value)>>,
    known: HashMap<String, String>,
    out: HashMap<String, Out>,
    attempted: HashMap<String, Vec<(String, Value)>>,
    rev_seen: Option<SystemTime>,
    file_seen: HashMap<String, SystemTime>,
    last_check: Option<Instant>,
    last_status: Option<Instant>,
    synced_at: Option<Instant>,
    begun: Option<Instant>,
    /// The server never answers the shared-state questions (an older server): the panel works as it always did.
    legacy: bool,
    note: String,
}

static CORE: Mutex<Option<Core>> = Mutex::new(None);

fn core() -> MutexGuard<'static, Option<Core>> {
    CORE.lock().unwrap_or_else(|e| e.into_inner())
}

fn with<R>(f: impl FnOnce(&mut Core) -> R) -> R {
    let mut guard = core();
    f(guard.get_or_insert_with(Core::default))
}

fn next_seq() -> u32 {
    (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(1) % 1_000_000) as u32 + 1
}

pub fn remote_file(topic: &str) -> String {
    format!("m182_remote_{topic}.json")
}

/// A topic arrived from the server.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    pub topic: String,
    pub data: Value,
    /// Names the panel tried to create that another admin had already taken.
    pub taken: Vec<String>,
}

fn ask(c: &mut Core, topic: &str, now: Instant) {
    match c.out.get_mut(topic) {
        Some(out) => {
            out.stale = true;
            remote::note(&format!("shared: {topic} changed while it was being asked, it will be asked again"));
        }
        None => {
            if let Some(dir) = &c.dir {
                let name = remote_file(topic);
                let _ = std::fs::remove_file(dir.join(&name));
                c.file_seen.remove(&name);
            }
            let seq = next_seq();
            c.out.insert(topic.to_string(), Out { seq, at: now, tries: 1, stale: false });
            remote::submit_pull(seq, topic);
        }
    }
}

/// A change was just sent for this topic: ask again afterwards so the panel shows what the server really kept.
fn ask_after_change(c: &mut Core, topic: &str, now: Instant) {
    match c.out.get_mut(topic) {
        Some(out) => out.stale = true,
        None => ask(c, topic, now),
    }
}

/// The panel saved one of the shared files. Only the difference with what the server said goes out.
pub fn local_change(file: &str, text: &str) -> Result<(), String> {
    let Some(topic) = topic_of_file(file) else { return Ok(()) };
    let wanted = items(topic, text);
    let op = with(|c| {
        let Some(known) = c.snap.get(topic) else { return Err("Sincronizando con el servidor: espera unos segundos y repite el cambio.".to_string()) };
        let d = diff(known, &wanted);
        if d.is_empty() {
            return Ok(None);
        }
        let wrote: Vec<(String, Value)> = d.add.iter().filter_map(|i| Some((lower(i.get("name")?.as_str()?), i.clone()))).collect();
        c.attempted.entry(topic.to_string()).or_default().extend(wrote);
        Ok(Some(d.op(topic)))
    })?;
    let Some(op) = op else { return Ok(()) };
    remote::submit(OP_FILE, &op)?;
    with(|c| ask_after_change(c, topic, Instant::now()));
    Ok(())
}

/// A spawner operation was sent: ask again afterwards.
pub fn spawners_changed() {
    with(|c| {
        if !c.legacy {
            ask_after_change(c, "spawners", Instant::now());
        }
    });
}

/// Forgets everything known about the server, so the next tick asks for it all again.
pub fn invalidate() {
    *core() = None;
}

/// Asks again for everything now (the "Sincronizar" button).
pub fn refresh_all() {
    with(|c| {
        if c.legacy {
            *c = Core::default();
            return;
        }
        let now = Instant::now();
        for t in TOPICS {
            c.out.remove(t);
            ask(c, t, now);
        }
        c.note = "Pidiendo todo al servidor…".to_string();
    });
}

/// Called while the spawner list is on screen: how the spawners are doing changes all the time, so it is
/// asked for on demand, not announced.
pub fn want_status() {
    with(|c| {
        if !c.started || c.legacy || c.out.contains_key(STATUS) {
            return;
        }
        let now = Instant::now();
        if c.last_status.is_some_and(|t| now.duration_since(t) < STATUS_EVERY) {
            return;
        }
        c.last_status = Some(now);
        ask(c, STATUS, now);
    });
}

/// One line for the header: how fresh the shared state is, and whether it is a problem.
pub fn line() -> (String, bool) {
    with(|c| {
        if !c.started {
            return (String::new(), false);
        }
        if !c.note.is_empty() {
            return (c.note.clone(), true);
        }
        match c.synced_at {
            None if c.begun.is_some_and(|t| t.elapsed() > NO_ANSWER) => ("Sin respuesta del servidor todavía. Pulsa «Sincronizar».".to_string(), true),
            None => ("Sincronizando con el servidor…".to_string(), false),
            Some(at) => (format!("Datos compartidos al día (hace {} s)", at.elapsed().as_secs()), false),
        }
    })
}

const UPLOADABLE: [&str; 5] = ["zones", "places", "teams", "templates", "spawners"];

fn local_items(dir: &Path, topic: &str) -> Vec<(String, Value)> {
    let file = if topic == "spawners" { "m182_spawners.json" } else { topic_file(topic) };
    let Ok(text) = std::fs::read_to_string(dir.join(file)) else { return Vec::new() };
    let mut list = items(topic, &text);
    if topic == "spawners" {
        list.retain(|(_, r)| !r.get("once").and_then(Value::as_bool).unwrap_or(false));
    }
    list
}

fn topic_file(topic: &str) -> &'static str {
    match topic {
        "zones" => "m182_zones.json",
        "places" => "m182_places.json",
        "teams" => "m182_teams.json",
        _ => "m182_raid_templates.json",
    }
}

/// Topics the server holds nothing of although this profile does (and how many items it has), so the
/// admin can hand them over with one button.
pub fn upload_hint(dir: &Path) -> Vec<(String, usize)> {
    with(|c| {
        UPLOADABLE
            .iter()
            .filter_map(|t| {
                if !c.snap.get(*t)?.is_empty() {
                    return None;
                }
                let n = local_items(dir, t).len();
                (n > 0).then(|| (t.to_string(), n))
            })
            .collect()
    })
}

/// Sends what this profile has and the server lacks: only creations, nothing that exists is changed or removed.
pub fn upload_missing(dir: &Path) -> usize {
    let mut total = 0;
    for topic in UPLOADABLE {
        let wanted = local_items(dir, topic);
        let Some(known) = with(|c| c.snap.get(topic).cloned()) else { continue };
        let missing: Vec<&(String, Value)> = wanted.iter().filter(|(k, _)| !known.iter().any(|(x, _)| x == k)).collect();
        if missing.is_empty() {
            continue;
        }
        total += missing.len();
        if topic == "spawners" {
            let rows: Vec<crate::spawners::Spawner> = crate::spawners::load(dir).into_iter().filter(|s| !s.once && missing.iter().any(|(k, _)| *k == s.id)).collect();
            for part in rows.chunks(4) {
                let _ = remote::submit(crate::spawners::OP_FILE, &crate::spawners::op_add_many(part));
            }
        } else {
            let op = Diff { add: missing.iter().map(|(_, v)| v.clone()).collect(), ..Default::default() }.op(topic);
            let _ = remote::submit(OP_FILE, &op);
        }
        with(|c| ask_after_change(c, topic, Instant::now()));
    }
    total
}

pub fn ready(topic: &str) -> bool {
    with(|c| c.snap.contains_key(topic))
}

fn read_if_new(dir: &Path, name: &str, seen: &mut Option<SystemTime>) -> Option<String> {
    let modified = std::fs::metadata(dir.join(name)).and_then(|m| m.modified()).ok()?;
    if *seen == Some(modified) {
        return None;
    }
    let text = std::fs::read_to_string(dir.join(name)).ok()?;
    *seen = Some(modified);
    Some(text)
}

/// Runs a few times a second: reads what the game wrote, asks for what changed and returns what arrived.
pub fn tick(dir: &Path, connected: bool) -> Vec<Applied> {
    tick_at(dir, connected, Instant::now())
}

/// True while the server does not know the shared-state commands (an older server).
pub fn legacy() -> bool {
    core().as_ref().is_some_and(|c| c.legacy)
}

fn tick_at(dir: &Path, connected: bool, now: Instant) -> Vec<Applied> {
    let mut applied = Vec::new();
    let mut guard = core();
    if !connected {
        if guard.as_ref().is_some_and(|c| c.started) {
            *guard = None;
        }
        return applied;
    }
    let c = guard.get_or_insert_with(Core::default);
    c.dir = Some(dir.to_path_buf());
    if c.legacy {
        if read_if_new(dir, REV_FILE, &mut c.rev_seen).is_some() {
            remote::note("shared: the server answers now, shared state is on again");
            *c = Core::default();
        } else if c.last_check.map_or(true, |t| now.duration_since(t) >= LEGACY_PROBE) {
            c.last_check = Some(now);
            remote::submit_rev(next_seq());
        }
        return applied;
    }
    if !c.started {
        c.started = true;
        c.begun = Some(now);
        let _ = std::fs::remove_file(dir.join(REV_FILE));
        for t in TOPICS.iter().chain(std::iter::once(&STATUS)) {
            let _ = std::fs::remove_file(dir.join(remote_file(t)));
        }
        for t in TOPICS {
            ask(c, t, now);
        }
        c.last_check = Some(now);
        return applied;
    }

    if let Some(text) = read_if_new(dir, REV_FILE, &mut c.rev_seen) {
        if let Some(rev) = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("rev").cloned()) {
            for t in TOPICS {
                let said = rev.get(t).and_then(Value::as_str);
                if said.is_some() && said != c.known.get(t).map(String::as_str) {
                    ask(c, t, now);
                }
            }
        }
        c.last_check = Some(now);
    }

    let waiting: Vec<String> = c.out.keys().cloned().collect();
    for topic in waiting {
        let name = remote_file(&topic);
        let mut seen = c.file_seen.get(&name).copied();
        let Some(text) = read_if_new(dir, &name, &mut seen) else { continue };
        if let Some(s) = seen {
            c.file_seen.insert(name, s);
        }
        let Some(envelope) = serde_json::from_str::<Value>(&text).ok() else { continue };
        if envelope.get("topic").and_then(Value::as_str) != Some(topic.as_str()) {
            continue;
        }
        let stale = c.out.get(&topic).is_some_and(|o| o.stale);
        c.out.remove(&topic);
        remote::note(&format!("shared: {topic} arrived ({} bytes{})", text.len(), if stale { ", discarded: asked again" } else { "" }));
        if stale {
            ask(c, &topic, now);
            continue;
        }
        let hash = envelope.get("hash").and_then(Value::as_str).unwrap_or("").to_string();
        let data = envelope.get("data").cloned().unwrap_or(Value::Null);
        if topic == STATUS {
            applied.push(Applied { topic, data, taken: Vec::new() });
            continue;
        }
        c.synced_at = Some(now);
        c.note.clear();
        let fresh = !c.snap.contains_key(&topic) || c.known.get(&topic) != Some(&hash);
        let server_items = items(&topic, &data.to_string());
        let taken: Vec<String> = c
            .attempted
            .remove(&topic)
            .unwrap_or_default()
            .into_iter()
            .filter(|(key, mine)| server_items.iter().any(|(k, theirs)| k == key && theirs != mine))
            .map(|(_, mine)| mine.get("name").and_then(Value::as_str).unwrap_or("").to_string())
            .collect();
        c.known.insert(topic.clone(), hash);
        c.snap.insert(topic.clone(), server_items);
        if fresh || !taken.is_empty() {
            applied.push(Applied { topic, data, taken });
        }
    }

    let late: Vec<String> = c.out.iter().filter(|(_, o)| now.duration_since(o.at) > ASK_TIMEOUT).map(|(t, _)| t.clone()).collect();
    for topic in late {
        let Some(out) = c.out.get_mut(&topic) else { continue };
        if out.tries >= MAX_TRIES {
            c.out.remove(&topic);
            remote::note(&format!("shared: no answer for {topic} after {MAX_TRIES} tries"));
            if c.synced_at.is_none() {
                c.legacy = true;
                c.out.clear();
                c.last_check = Some(now);
                c.note = "El servidor no tiene la sincronización (versión antigua): el panel funciona como antes. Actualiza su dbghelp.dll.".to_string();
                remote::note("shared: the server does not answer, working as before");
                break;
            }
            c.note = format!("El servidor no respondió con «{topic}». Pulsa «Sincronizar» para intentarlo otra vez.");
            continue;
        }
        out.tries += 1;
        out.at = now;
        out.seq = next_seq();
        remote::submit_pull(out.seq, &topic);
    }

    if c.out.is_empty() && c.last_check.map_or(true, |t| now.duration_since(t) >= SAFETY_CHECK) {
        c.last_check = Some(now);
        remote::submit_rev(next_seq());
    }
    applied
}

#[cfg(test)]
pub fn reset_for_tests() {
    *core() = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(name: &str, x: f64) -> String {
        json!({"zones": [{"name": name, "x": x, "z": 2.0, "radius": 30.0}]}).to_string()
    }

    #[test]
    fn an_unchanged_list_sends_nothing_and_each_difference_is_one_item() {
        let known = items("zones", &json!({"zones": [{"name": "A", "x": 1.0, "z": 1.0, "radius": 10.0}, {"name": "B", "x": 2.0, "z": 2.0, "radius": 10.0}]}).to_string());
        assert!(diff(&known, &known).is_empty());

        let wanted = items("zones", &json!({"zones": [{"name": "B", "x": 9.0, "z": 2.0, "radius": 10.0}, {"name": "C", "x": 3.0, "z": 3.0, "radius": 10.0}]}).to_string());
        let d = diff(&known, &wanted);
        assert_eq!(d.add.len(), 1);
        assert_eq!(d.add[0]["name"], "C");
        assert_eq!(d.up.len(), 1);
        assert_eq!(d.up[0]["x"], 9.0);
        assert_eq!(d.rm, vec!["A".to_string()]);
        let op: Value = serde_json::from_str(&d.op("zones")).unwrap();
        assert_eq!(op["t"], "zones");
        assert!(op["rm"][0] == "A");
    }

    #[test]
    fn names_match_without_looking_at_case() {
        let a = items("places", &json!([{"name": "Torre", "x": 1.0, "y": 2.0, "z": 3.0}]).to_string());
        let b = items("places", &json!([{"name": "torre", "x": 1.0, "y": 2.0, "z": 3.0}]).to_string());
        assert_eq!(a[0].0, b[0].0);
    }

    #[test]
    fn teams_compare_by_people_and_leaders_whatever_their_order() {
        let a = items("teams", &json!({"teams": [{"name": "Rojo", "people": [{"key": "name:Ana", "name": "Ana"}, {"key": "name:Luis", "name": "Luis"}], "leaders": ["name:Luis", "name:Ana"], "members": [3, 4]}], "touched": {"ana": 4}}).to_string());
        let b = items("teams", &json!({"teams": [{"name": "Rojo", "people": [{"key": "name:Luis", "name": "Luis"}, {"key": "name:Ana", "name": "Ana"}], "leaders": ["name:Ana", "name:Luis"], "members": []}]}).to_string());
        assert!(diff(&a, &b).is_empty());
        let c = items("teams", &json!({"teams": [{"name": "Rojo", "people": [{"key": "name:Ana", "name": "Ana"}], "leaders": ["name:Luis"]}]}).to_string());
        let d = diff(&a, &c);
        assert_eq!(d.up.len(), 1);
        assert_eq!(d.up[0]["leaders"], json!([]), "a leader who is not in the team is dropped");
    }

    #[test]
    fn templates_without_a_name_still_get_one() {
        let t = items("templates", &json!({"templates": [{"name": "", "active": true, "members": [{"creature": "Lobo", "template": "x", "level": 1, "count": 1}]}]}).to_string());
        assert_eq!(t[0].0, "(sin nombre)");
    }

    #[test]
    fn nothing_is_sent_before_the_server_has_answered_once() {
        reset_for_tests();
        let err = local_change("m182_zones.json", &zone("A", 1.0)).unwrap_err();
        assert!(err.contains("Sincronizando"), "{err}");
        assert!(local_change("m182_native.json", "{}").is_ok());
        reset_for_tests();
    }

    #[test]
    fn a_server_that_never_answers_leaves_the_panel_working_as_before() {
        reset_for_tests();
        remote::reset_for_tests();
        let dir = std::env::temp_dir().join(format!("m182_legacy_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let t0 = Instant::now();
        tick_at(&dir, true, t0);
        assert!(!legacy());
        tick_at(&dir, true, t0 + Duration::from_secs(26));
        tick_at(&dir, true, t0 + Duration::from_secs(52));
        assert!(!legacy());
        tick_at(&dir, true, t0 + Duration::from_secs(78));
        assert!(legacy());
        let (text, problem) = line();
        assert!(problem && text.contains("dbghelp"), "{text}");
        assert!(local_change("m182_zones.json", &zone("A", 1.0)).is_err() || legacy());
        std::fs::write(dir.join(REV_FILE), r#"{"seq":1,"rev":{}}"#).unwrap();
        tick_at(&dir, true, t0 + Duration::from_secs(90));
        assert!(!legacy());
        reset_for_tests();
    }

    use std::sync::Arc;

    struct Recorder(Arc<Mutex<Vec<String>>>);

    impl remote::Typist for Recorder {
        fn begin(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn line(&mut self, text: &str) -> Result<(), String> {
            self.0.lock().unwrap().push(text.to_string());
            Ok(())
        }
        fn end(&mut self) {}
    }

    fn unhex(text: &str) -> String {
        let bytes: Vec<u8> = (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect();
        String::from_utf8(bytes).unwrap()
    }

    fn unpack(line: &str) -> String {
        let payload = line.rsplit(' ').next().unwrap();
        if line.contains(" setx ") {
            return unhex(payload);
        }
        let mut raw = Vec::new();
        let (mut acc, mut bits) = (0u32, 0u32);
        for c in payload.bytes() {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'-' => 62,
                _ => 63,
            } as u32;
            acc = acc << 6 | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                raw.push((acc >> bits) as u8);
                acc &= (1 << bits) - 1;
            }
        }
        String::from_utf8(miniz_oxide::inflate::decompress_to_vec(&raw).unwrap()).unwrap()
    }

    fn wait_for(lines: &Arc<Mutex<Vec<String>>>, count: usize) {
        for _ in 0..200 {
            if lines.lock().unwrap().len() >= count {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("only {:?} was typed", lines.lock().unwrap());
    }

    fn answer(dir: &Path, topic: &str, hash: &str, data: Value) {
        std::fs::write(dir.join(remote_file(topic)), json!({"topic": topic, "hash": hash, "data": data}).to_string()).unwrap();
    }

    #[test]
    fn connecting_reads_everything_and_a_later_edit_sends_only_the_difference() {
        remote::reset_for_tests();
        reset_for_tests();
        let dir = std::env::temp_dir().join(format!("m182_panel_shared_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let typed = Arc::new(Mutex::new(Vec::new()));
        remote::configure(remote::Settings { enabled: true, key: "secret-key".into(), limit: 200 });
        remote::install_typist(Box::new(Recorder(typed.clone())));
        remote::set_hosting(Some(false));
        assert!(remote::connected());

        assert!(tick(&dir, true).is_empty());
        wait_for(&typed, 5);
        {
            let lines = typed.lock().unwrap();
            for topic in TOPICS {
                assert!(lines.iter().any(|l| l.contains(" pull ") && l.ends_with(topic)), "{topic} was asked: {lines:?}");
            }
        }

        let zones = json!({"zones": [{"name": "Base norte", "x": 1.0, "z": 2.0, "radius": 30.0, "enabled": true, "safe": true, "team": 0, "lock": false, "lock_admins": false, "pvp": false}]});
        answer(&dir, "zones", "aaaaaaaa", zones.clone());
        answer(&dir, "places", "bbbbbbbb", json!([{"name": "Torre", "x": 1.0, "y": 2.0, "z": 3.0}]));
        answer(&dir, "spawners", "cccccccc", json!({"spawners": []}));
        answer(&dir, "teams", "dddddddd", json!({"teams": [{"name": "Rojo", "people": [{"key": "name:Ana", "name": "Ana"}], "leaders": ["name:Ana"]}]}));
        answer(&dir, "templates", "eeeeeeee", json!({"templates": []}));
        let got = tick(&dir, true);
        assert_eq!(got.len(), 5, "{got:?}");
        assert!(ready("zones") && ready("teams"));
        assert!(tick(&dir, true).is_empty(), "nothing new, nothing to apply");

        typed.lock().unwrap().clear();
        let mut mine = zones["zones"].as_array().unwrap().clone();
        mine.push(json!({"name": "Base sur", "x": 9.0, "z": 9.0, "radius": 40.0, "enabled": true, "safe": true, "team": 0, "lock": false, "lock_admins": false, "pvp": false}));
        local_change("m182_zones.json", &json!({"zones": mine}).to_string()).unwrap();
        wait_for(&typed, 2);
        let sent = typed.lock().unwrap().clone();
        let setx = sent.iter().find(|l| l.contains(" m182_state_op.json ")).unwrap_or_else(|| panic!("{sent:?}"));
        let op: Value = serde_json::from_str(&unpack(setx)).unwrap();
        assert_eq!(op["t"], "zones");
        assert_eq!(op["add"].as_array().unwrap().len(), 1);
        assert_eq!(op["add"][0]["name"], "Base sur");
        assert!(op.get("up").is_none() && op.get("rm").is_none(), "the other zone is not mentioned");
        assert!(sent.iter().any(|l| l.contains(" pull ") && l.ends_with("zones")), "it asks again afterwards: {sent:?}");

        // the server answers with both zones, plus one made by another admin meanwhile
        mine.push(json!({"name": "Mercado", "x": 5.0, "z": 5.0, "radius": 20.0, "enabled": true, "safe": true, "team": 0, "lock": false, "lock_admins": false, "pvp": false}));
        answer(&dir, "zones", "ffffffff", json!({"zones": mine}));
        let got = tick(&dir, true);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].data["zones"].as_array().unwrap().len(), 3);
        assert!(got[0].taken.is_empty());

        // leaving the server forgets everything
        assert!(tick(&dir, false).is_empty());
        assert!(!ready("zones"));
        remote::reset_for_tests();
        reset_for_tests();
    }

    #[test]
    fn a_name_somebody_else_took_first_is_reported() {
        remote::reset_for_tests();
        reset_for_tests();
        let dir = std::env::temp_dir().join(format!("m182_panel_taken_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let typed = Arc::new(Mutex::new(Vec::new()));
        remote::configure(remote::Settings { enabled: true, key: "secret-key".into(), limit: 200 });
        remote::install_typist(Box::new(Recorder(typed.clone())));
        remote::set_hosting(Some(false));
        tick(&dir, true);
        for topic in TOPICS {
            let data = match topic {
                "places" => json!([]),
                "zones" => json!({"zones": []}),
                "teams" => json!({"teams": []}),
                "templates" => json!({"templates": []}),
                _ => json!({"spawners": []}),
            };
            answer(&dir, topic, "00000001", data);
        }
        assert_eq!(tick(&dir, true).len(), 5);
        let zone = |x: f64| json!({"name": "Base", "x": x, "z": 1.0, "radius": 30.0, "enabled": true, "safe": true, "team": 0, "lock": false, "lock_admins": false, "pvp": false});
        local_change("m182_zones.json", &json!({"zones": [zone(1.0)]}).to_string()).unwrap();
        answer(&dir, "zones", "00000002", json!({"zones": [zone(77.0)]}));
        let got = tick(&dir, true);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].taken, vec!["Base".to_string()]);
        remote::reset_for_tests();
        reset_for_tests();
    }

    #[test]
    fn an_answer_older_than_my_own_change_is_asked_for_again() {
        remote::reset_for_tests();
        reset_for_tests();
        let dir = std::env::temp_dir().join(format!("m182_panel_stale_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let typed = Arc::new(Mutex::new(Vec::new()));
        remote::configure(remote::Settings { enabled: true, key: "secret-key".into(), limit: 200 });
        remote::install_typist(Box::new(Recorder(typed.clone())));
        remote::set_hosting(Some(false));
        tick(&dir, true);
        for topic in TOPICS {
            let data = match topic {
                "places" => json!([]),
                "zones" => json!({"zones": []}),
                "teams" => json!({"teams": []}),
                "templates" => json!({"templates": []}),
                _ => json!({"spawners": []}),
            };
            answer(&dir, topic, "00000001", data);
        }
        tick(&dir, true);
        // a push made the panel ask for the zones; before the answer comes, the admin adds a place
        answer(&dir, "places", "00000001", json!([]));
        with(|c| ask(c, "places", Instant::now()));
        let place = json!([{"name": "Torre", "x": 1.0, "y": 2.0, "z": 3.0}]);
        local_change("m182_places.json", &place.to_string()).unwrap();
        answer(&dir, "places", "00000002", json!([]));
        assert!(tick(&dir, true).is_empty(), "the answer predates the change and is not shown");
        answer(&dir, "places", "00000003", place);
        let got = tick(&dir, true);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].data.as_array().unwrap().len(), 1);
        remote::reset_for_tests();
        reset_for_tests();
    }
}

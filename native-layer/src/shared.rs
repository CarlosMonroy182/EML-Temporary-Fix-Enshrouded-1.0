//! State that every admin of one server shares: zones, saved places, creature spawners, teams and
//! raid templates live on the server; each panel reads them from there and changes them through small
//! per-item operations, so two admins working at the same time never overwrite each other.
//!
//! * The server answers `rev` with one short line of content hashes and `pull <topic>` with the topic
//!   as a few chat lines. Both answers go to the asker only (see `patch`).
//! * The server tells the other panels that something changed with the same short hash line, so
//!   nobody has to ask over and over.
//! * A game with the panel reads those lines in its chat hook and writes `m182_remote_*.json`.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use super::groups;
use super::roster::same_name;

pub const OP_FILE: &str = "m182_state_op.json";
pub const REV_FILE: &str = "m182_remote_rev.json";
pub const REV_TAG: &str = "[M182R#";
pub const SNAP_TAG: &str = "[M182S#";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topic {
    Zones,
    Places,
    Spawners,
    Teams,
    Templates,
    /// What the spawners are doing right now; read on demand, never part of the revisions.
    Status,
}

impl Topic {
    pub const SHARED: [Topic; 5] = [Topic::Zones, Topic::Places, Topic::Spawners, Topic::Teams, Topic::Templates];

    pub fn name(self) -> &'static str {
        match self {
            Topic::Zones => "zones",
            Topic::Places => "places",
            Topic::Spawners => "spawners",
            Topic::Teams => "teams",
            Topic::Templates => "templates",
            Topic::Status => "status",
        }
    }

    pub fn from_name(name: &str) -> Option<Topic> {
        [Topic::Zones, Topic::Places, Topic::Spawners, Topic::Teams, Topic::Templates, Topic::Status].into_iter().find(|t| t.name().eq_ignore_ascii_case(name.trim()))
    }

    pub fn letter(self) -> char {
        match self {
            Topic::Zones => 'z',
            Topic::Places => 'p',
            Topic::Spawners => 's',
            Topic::Teams => 't',
            Topic::Templates => 'r',
            Topic::Status => 'a',
        }
    }

    pub fn file(self) -> &'static str {
        match self {
            Topic::Zones => "m182_zones.json",
            Topic::Places => "m182_places.json",
            Topic::Spawners => "m182_spawners.json",
            Topic::Teams => "m182_teams.json",
            Topic::Templates => "m182_raid_templates.json",
            Topic::Status => "m182_spawner_status.json",
        }
    }

    /// Name of the list inside the file; empty when the file is the list itself.
    fn container(self) -> &'static str {
        match self {
            Topic::Zones => "zones",
            Topic::Places => "",
            Topic::Spawners | Topic::Status => "spawners",
            Topic::Teams => "teams",
            Topic::Templates => "templates",
        }
    }

    fn limit(self) -> usize {
        match self {
            Topic::Zones | Topic::Places => 500,
            Topic::Spawners | Topic::Status => 1000,
            Topic::Teams => 100,
            Topic::Templates => 60,
        }
    }

    fn empty(self) -> Value {
        match self {
            Topic::Places => json!([]),
            other => {
                let mut map = Map::new();
                map.insert(other.container().to_string(), json!([]));
                Value::Object(map)
            }
        }
    }
}

const MEMBERS_FILE: &str = groups::MEMBERS_FILE;

fn fnv(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for b in part.iter().chain(std::iter::once(&0u8)) {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

fn short(h: u64) -> String {
    format!("{:08x}", (h ^ (h >> 32)) as u32)
}

fn read_bytes(dir: &Path, name: &str) -> Vec<u8> {
    std::fs::read(dir.join(name)).unwrap_or_default()
}

/// Fingerprint of what the server holds for a topic (eight hex digits). Equal fingerprints mean equal content.
pub fn revision(dir: &Path, topic: Topic) -> String {
    match topic {
        Topic::Teams => short(fnv(&[&read_bytes(dir, Topic::Teams.file()), &read_bytes(dir, MEMBERS_FILE)])),
        other => short(fnv(&[&read_bytes(dir, other.file())])),
    }
}

/// `[M182R#7] z=1a2b3c4d p=... s=... t=... r=...`: the short answer to `rev`, also the notice sent to the other panels.
pub fn rev_line(dir: &Path, seq: u32) -> String {
    let mut out = format!("{REV_TAG}{seq}]");
    for topic in Topic::SHARED {
        out.push_str(&format!(" {}={}", topic.letter(), revision(dir, topic)));
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Revs {
    pub seq: u32,
    pub hashes: Vec<(Topic, String)>,
}

pub fn parse_rev_line(text: &str) -> Option<Revs> {
    let start = text.find(REV_TAG)? + REV_TAG.len();
    let rest = &text[start..];
    let end = rest.find(']')?;
    let seq: u32 = rest[..end].trim().parse().ok()?;
    let mut hashes = Vec::new();
    for word in rest[end + 1..].split_whitespace() {
        let (letter, value) = word.split_once('=')?;
        let topic = Topic::SHARED.into_iter().find(|t| letter.len() == 1 && letter.starts_with(t.letter()))?;
        if value.len() != 8 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        hashes.push((topic, value.to_ascii_lowercase()));
    }
    (!hashes.is_empty()).then_some(Revs { seq, hashes })
}

fn json_or(dir: &Path, topic: Topic) -> Value {
    std::fs::read_to_string(dir.join(topic.file())).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or_else(|| topic.empty())
}

/// What a panel is shown for a topic. Teams are the panel's definition with the in-game joins and leaves applied.
pub fn data_of(dir: &Path, topic: Topic) -> Value {
    match topic {
        Topic::Teams => json!({ "teams": effective_teams(dir) }),
        other => json_or(dir, other),
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn b64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 4 / 3 + 4);
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(B64[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(B64[n as usize & 63] as char);
        }
    }
    out
}

pub const CHUNK: usize = 600;
pub const MAX_LINES: usize = 160;

/// The topic as chat lines: `[M182S#seq topic i/n hash] <part of the deflated, base64 envelope>`.
pub fn snapshot_lines(dir: &Path, seq: u32, topic: Topic) -> Result<Vec<String>, String> {
    let data = data_of(dir, topic);
    let hash = if topic == Topic::Status { short(fnv(&[data.to_string().as_bytes()])) } else { revision(dir, topic) };
    let envelope = json!({ "topic": topic.name(), "hash": hash, "data": data }).to_string();
    let packed = b64(&miniz_oxide::deflate::compress_to_vec(envelope.as_bytes(), 9));
    let parts: Vec<&[u8]> = packed.as_bytes().chunks(CHUNK).collect();
    if parts.len() > MAX_LINES {
        return Err(format!("{} is too large to send ({} lines)", topic.name(), parts.len()));
    }
    let total = parts.len();
    Ok(parts.iter().enumerate().map(|(i, p)| format!("{SNAP_TAG}{seq} {} {}/{total} {hash}] {}", topic.name(), i + 1, std::str::from_utf8(p).unwrap_or(""))).collect())
}

struct Chunk<'a> {
    seq: u32,
    topic: &'a str,
    index: usize,
    total: usize,
    hash: &'a str,
    payload: &'a str,
}

fn parse_chunk(line: &str) -> Option<Chunk<'_>> {
    let start = line.find(SNAP_TAG)? + SNAP_TAG.len();
    let rest = &line[start..];
    let close = rest.find(']')?;
    let mut head = rest[..close].split_whitespace();
    let seq = head.next()?.parse().ok()?;
    let topic = head.next()?;
    let (index, total) = head.next()?.split_once('/')?;
    let (index, total): (usize, usize) = (index.parse().ok()?, total.parse().ok()?);
    let hash = head.next()?;
    if index == 0 || index > total || total > MAX_LINES {
        return None;
    }
    Some(Chunk { seq, topic, index, total, hash, payload: rest[close + 1..].trim() })
}

struct Assembly {
    key: (u32, String),
    total: usize,
    hash: String,
    parts: Vec<Option<String>>,
    at: Instant,
}

/// Puts the chat lines of one answer back together on the client.
#[derive(Default)]
pub struct Assembler {
    open: Vec<Assembly>,
}

const ASSEMBLY_LIFE: Duration = Duration::from_secs(90);
const MAX_OPEN: usize = 12;
const MAX_UNPACKED: usize = 4 * 1024 * 1024;

impl Assembler {
    /// Takes one line; returns `(topic, envelope text)` when it was the last missing part.
    pub fn feed(&mut self, now: Instant, line: &str) -> Result<Option<(String, String)>, String> {
        let chunk = parse_chunk(line).ok_or_else(|| "a snapshot line could not be read".to_string())?;
        Topic::from_name(chunk.topic).ok_or_else(|| format!("unknown topic \"{}\"", chunk.topic))?;
        self.open.retain(|a| now.duration_since(a.at) < ASSEMBLY_LIFE);
        let key = (chunk.seq, chunk.topic.to_ascii_lowercase());
        let at = match self.open.iter().position(|a| a.key == key && a.total == chunk.total && a.hash == chunk.hash) {
            Some(i) => i,
            None => {
                self.open.retain(|a| a.key != key);
                if self.open.len() >= MAX_OPEN {
                    self.open.remove(0);
                }
                self.open.push(Assembly { key: key.clone(), total: chunk.total, hash: chunk.hash.to_string(), parts: vec![None; chunk.total], at: now });
                self.open.len() - 1
            }
        };
        let a = &mut self.open[at];
        a.parts[chunk.index - 1] = Some(chunk.payload.to_string());
        a.at = now;
        if a.parts.iter().any(Option::is_none) {
            return Ok(None);
        }
        let a = self.open.remove(at);
        let packed: String = a.parts.into_iter().flatten().collect();
        let raw = super::chat::unb64(&packed)?;
        let bytes = miniz_oxide::inflate::decompress_to_vec_with_limit(&raw, MAX_UNPACKED).map_err(|_| "snapshot is damaged".to_string())?;
        let text = String::from_utf8(bytes).map_err(|_| "snapshot is not text".to_string())?;
        let value: Value = serde_json::from_str(&text).map_err(|e| format!("snapshot is not valid: {e}"))?;
        if value.get("hash").and_then(Value::as_str) != Some(a.hash.as_str()) || value.get("data").is_none() {
            return Err("snapshot does not match its header".to_string());
        }
        Ok(Some((a.key.1, text)))
    }
}

static ASSEMBLER: Mutex<Option<Assembler>> = Mutex::new(None);

fn write_file(dir: &Path, name: &str, text: &str) -> Result<(), String> {
    let temp = dir.join(format!("{name}.tmp"));
    std::fs::write(&temp, text).map_err(|e| format!("cannot write {name}: {e}"))?;
    std::fs::rename(&temp, dir.join(name)).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("cannot replace {name}: {e}")
    })
}

pub fn remote_file(topic: &str) -> String {
    format!("m182_remote_{}.json", topic.to_ascii_lowercase())
}

/// Client side: a `[M182S#...]` line arrived. Returns a line for the log.
pub fn receive_snapshot(dir: &Path, line: &str) -> String {
    let Ok(mut guard) = ASSEMBLER.lock() else { return "snapshot: assembler unavailable".to_string() };
    let assembler = guard.get_or_insert_with(Assembler::default);
    match assembler.feed(Instant::now(), line) {
        Ok(None) => String::new(),
        Ok(Some((topic, text))) => match write_file(dir, &remote_file(&topic), &text) {
            Ok(()) => format!("shared state: {topic} received from the server ({} bytes)", text.len()),
            Err(e) => format!("shared state: {e}"),
        },
        Err(e) => format!("shared state: {e}"),
    }
}

/// Client side: a `[M182R#...]` line arrived (the answer to `rev`, or a notice that something changed).
pub fn receive_revs(dir: &Path, line: &str) -> String {
    let Some(revs) = parse_rev_line(line) else { return "shared state: a revision line could not be read".to_string() };
    let mut map = Map::new();
    for (topic, hash) in &revs.hashes {
        map.insert(topic.name().to_string(), json!(hash));
    }
    match write_file(dir, REV_FILE, &json!({ "seq": revs.seq, "rev": map }).to_string()) {
        Ok(()) => String::new(),
        Err(e) => format!("shared state: {e}"),
    }
}

static CHANGED: AtomicBool = AtomicBool::new(false);

/// Something shared was written: the other panels must be told.
pub fn note_change() {
    CHANGED.store(true, Ordering::SeqCst);
}

pub fn peek_change() -> bool {
    CHANGED.load(Ordering::SeqCst)
}

pub fn take_change() -> bool {
    CHANGED.swap(false, Ordering::SeqCst)
}

struct Panel {
    client: u32,
    who: String,
    seen: Instant,
}

static PANELS: Mutex<Vec<Panel>> = Mutex::new(Vec::new());

/// A chat client that proved it holds the key is a panel; the server remembers it for a while.
pub fn remember_panel(client: u32, who: &str, now: Instant) {
    let Ok(mut panels) = PANELS.lock() else { return };
    panels.retain(|p| now.duration_since(p.seen) < PANEL_LIFE);
    match panels.iter_mut().find(|p| p.client == client) {
        Some(p) => {
            p.who = who.to_string();
            p.seen = now;
        }
        None => panels.push(Panel { client, who: who.to_string(), seen: now }),
    }
}

pub const PANEL_LIFE: Duration = Duration::from_secs(12 * 60);

/// True when this client was registered a moment ago (so the table need not be read again for every line).
pub fn seen_recently(client: u32, now: Instant) -> bool {
    PANELS.lock().map(|p| p.iter().any(|x| x.client == client && now.duration_since(x.seen) < Duration::from_secs(30))).unwrap_or(false)
}

/// Panels to tell about a change: remembered recently, still the same player in that slot, not the one who made it.
pub fn targets(now: Instant, present: &[(u32, String)], except: u32) -> Vec<u32> {
    let Ok(panels) = PANELS.lock() else { return Vec::new() };
    panels
        .iter()
        .filter(|p| p.client != except && now.duration_since(p.seen) < PANEL_LIFE)
        .filter(|p| present.iter().any(|(c, who)| *c == p.client && *who == p.who))
        .map(|p| p.client)
        .collect()
}

#[cfg(test)]
pub fn forget_panels() {
    if let Ok(mut p) = PANELS.lock() {
        p.clear();
    }
    CHANGED.store(false, Ordering::SeqCst);
}

fn text_of(item: &Value, key: &str) -> String {
    item.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

fn item_key(topic: Topic, item: &Value) -> Option<String> {
    let key = if topic == Topic::Spawners { text_of(item, "id") } else { text_of(item, "name") };
    (!key.is_empty() && key.len() <= 64).then_some(key)
}

fn finite(item: &Value, field: &str) -> bool {
    item.get(field).and_then(Value::as_f64).is_some_and(f64::is_finite)
}

fn check(topic: Topic, item: &Value) -> Result<String, String> {
    if !item.is_object() {
        return Err("an item is not an object".to_string());
    }
    let key = item_key(topic, item).ok_or_else(|| "an item has no usable name".to_string())?;
    let ok = match topic {
        Topic::Zones => finite(item, "x") && finite(item, "z") && item.get("radius").and_then(Value::as_f64).is_some_and(|r| r.is_finite() && r > 0.0 && r <= 5000.0),
        Topic::Places => finite(item, "x") && finite(item, "y") && finite(item, "z"),
        Topic::Templates => item.get("members").is_some_and(Value::is_array),
        Topic::Teams => item.get("people").is_none_or(Value::is_array) && item.get("leaders").is_none_or(Value::is_array),
        _ => false,
    };
    if ok { Ok(key) } else { Err(format!("\"{key}\" is not valid for {}", topic.name())) }
}

fn same_key(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

#[derive(Default, Debug, PartialEq)]
pub struct Outcome {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub refused: Vec<String>,
}

impl Outcome {
    fn describe(&self, topic: Topic, now: usize) -> String {
        let mut text = format!("{}: {now} now ({} added, {} changed, {} removed)", topic.name(), self.added, self.updated, self.removed);
        if !self.refused.is_empty() {
            text.push_str(&format!("; kept as they were because they already exist: {}", self.refused.join(", ")));
        }
        text
    }
}

fn list_of(root: &Value, topic: Topic) -> Vec<Value> {
    match topic.container() {
        "" => root.as_array().cloned().unwrap_or_default(),
        name => root.get(name).and_then(Value::as_array).cloned().unwrap_or_default(),
    }
}

fn read_root(dir: &Path, topic: Topic) -> Result<Value, String> {
    match std::fs::read_to_string(dir.join(topic.file())) {
        Ok(text) => serde_json::from_str::<Value>(&text).map_err(|_| format!("{} is not readable; send the whole list instead", topic.file())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(topic.empty()),
        Err(e) => Err(format!("cannot read {}: {e}", topic.file())),
    }
}

fn with_list(root: Value, topic: Topic, list: Vec<Value>) -> Value {
    match topic.container() {
        "" => Value::Array(list),
        name => {
            let mut map = root.as_object().cloned().unwrap_or_default();
            map.insert(name.to_string(), Value::Array(list));
            Value::Object(map)
        }
    }
}

fn many(op: &Value, field: &str) -> Vec<Value> {
    match op.get(field) {
        Some(Value::Array(list)) => list.clone(),
        Some(other) => vec![other.clone()],
        None => Vec::new(),
    }
}

/// `{"t":"zones","add":[..],"up":[..],"rm":["name"]}`: `add` creates (an existing name is kept as it was),
/// `up` changes or creates, `rm` deletes. Every item is found by its name, never by its position.
pub fn apply_op(dir: &Path, op: &Value) -> Result<String, String> {
    let topic = op.get("t").and_then(Value::as_str).and_then(Topic::from_name).ok_or("a state change needs a topic")?;
    match topic {
        Topic::Spawners | Topic::Status => return Err("spawners change through the spawner command".to_string()),
        Topic::Teams => return apply_team_op(dir, op),
        _ => {}
    }
    if topic == Topic::Zones {
        if let Some(said) = apply_zone_op(dir, op)? {
            return Ok(said);
        }
    }
    let root = read_root(dir, topic)?;
    let mut list = list_of(&root, topic);
    let mut out = Outcome::default();
    for item in many(op, "add") {
        let key = check(topic, &item)?;
        match list.iter().position(|x| item_key(topic, x).is_some_and(|k| same_key(&k, &key))) {
            Some(i) if list[i] == item => {}
            Some(_) => out.refused.push(key),
            None => {
                list.push(item);
                out.added += 1;
            }
        }
    }
    for item in many(op, "up") {
        let key = check(topic, &item)?;
        match list.iter().position(|x| item_key(topic, x).is_some_and(|k| same_key(&k, &key))) {
            Some(i) if list[i] == item => {}
            Some(i) => {
                list[i] = item;
                out.updated += 1;
            }
            None => {
                list.push(item);
                out.added += 1;
            }
        }
    }
    for name in many(op, "rm") {
        let Some(name) = name.as_str() else { continue };
        let before = list.len();
        list.retain(|x| !item_key(topic, x).is_some_and(|k| same_key(&k, name)));
        out.removed += before - list.len();
    }
    if list.len() > topic.limit() {
        return Err(format!("too many items in {} ({})", topic.name(), list.len()));
    }
    let now = list.len();
    let text = with_list(root, topic, list).to_string();
    write_file(dir, topic.file(), &text)?;
    note_change();
    Ok(out.describe(topic, now))
}

/// A JSON object that keeps its keys in the order they were written.
#[derive(Debug, Clone, PartialEq)]
struct Ordered(Vec<(String, Value)>);

impl<'de> serde::Deserialize<'de> for Ordered {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Reader;
        impl<'de> serde::de::Visitor<'de> for Reader {
            type Value = Ordered;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Ordered, A::Error> {
                let mut out = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, Value>()? {
                    out.push((key, value));
                }
                Ok(Ordered(out))
            }
        }
        d.deserialize_map(Reader)
    }
}

impl serde::Serialize for Ordered {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl Ordered {
    fn get(&self, key: &str) -> Option<&Value> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

/// The zones file as it was written: every key of the root, and every zone with its keys in their own order.
struct ZonesDoc(Vec<(String, ZonesPart)>);

enum ZonesPart {
    Zones(Vec<Ordered>),
    Other(Value),
}

impl<'de> serde::Deserialize<'de> for ZonesDoc {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Reader;
        impl<'de> serde::de::Visitor<'de> for Reader {
            type Value = ZonesDoc;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<ZonesDoc, A::Error> {
                let mut out = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    let part = if key == "zones" { ZonesPart::Zones(map.next_value()?) } else { ZonesPart::Other(map.next_value()?) };
                    out.push((key, part));
                }
                Ok(ZonesDoc(out))
            }
        }
        d.deserialize_map(Reader)
    }
}

impl serde::Serialize for ZonesDoc {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (key, part) in &self.0 {
            match part {
                ZonesPart::Zones(list) => map.serialize_entry(key, list)?,
                ZonesPart::Other(value) => map.serialize_entry(key, value)?,
            }
        }
        map.end()
    }
}

/// The keys the panel writes for a zone, in the order it writes them. Any other key of a zone belongs to
/// somebody else (an older or newer panel, the administrator) and is left as it is.
const ZONE_KEYS: [&str; 13] = ["name", "x", "z", "radius", "enabled", "safe", "team", "lock", "lock_admins", "pvp", "shield", "guardian", "guardian_level"];

/// `old` with the panel's keys taken from `new` (a key `new` leaves out is removed) and everything else kept in place.
fn merged_zone(old: &Ordered, new: &Value) -> Ordered {
    let mut out = Vec::new();
    for (key, value) in &old.0 {
        if ZONE_KEYS.contains(&key.as_str()) {
            if let Some(fresh) = new.get(key) {
                out.push((key.clone(), fresh.clone()));
            }
        } else {
            out.push((key.clone(), value.clone()));
        }
    }
    for key in ZONE_KEYS {
        if out.iter().all(|(k, _)| k != key) {
            if let Some(fresh) = new.get(key) {
                out.push((key.to_string(), fresh.clone()));
            }
        }
    }
    Ordered(out)
}

fn fresh_zone(new: &Value) -> Ordered {
    merged_zone(&Ordered(Vec::new()), new).with_rest(new)
}

impl Ordered {
    fn with_rest(mut self, new: &Value) -> Ordered {
        if let Some(map) = new.as_object() {
            for (key, value) in map {
                if self.get(key).is_none() {
                    self.0.push((key.clone(), value.clone()));
                }
            }
        }
        self
    }
}

fn zone_name(zone: &Ordered) -> Option<String> {
    zone.get("name").and_then(Value::as_str).map(|n| n.trim().to_string()).filter(|n| !n.is_empty())
}

/// Zone changes keep the file as the administrator left it: untouched zones are not rewritten, the keys of a
/// zone nobody here knows stay where they are, and a change that changes nothing does not write at all.
/// `None` when the file is not in a shape this can follow; the general path then takes over.
fn apply_zone_op(dir: &Path, op: &Value) -> Result<Option<String>, String> {
    let topic = Topic::Zones;
    let text = match std::fs::read_to_string(dir.join(topic.file())) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {}: {e}", topic.file())),
    };
    let mut doc = match &text {
        Some(text) => match serde_json::from_str::<ZonesDoc>(text) {
            Ok(doc) => doc,
            Err(_) => return Ok(None),
        },
        None => ZonesDoc(vec![("zones".to_string(), ZonesPart::Zones(Vec::new()))]),
    };
    if !doc.0.iter().any(|(k, _)| k == "zones") {
        doc.0.push(("zones".to_string(), ZonesPart::Zones(Vec::new())));
    }
    let Some(ZonesPart::Zones(list)) = doc.0.iter_mut().find(|(k, _)| k == "zones").map(|(_, p)| p) else { return Ok(None) };
    let mut out = Outcome::default();
    for item in many(op, "add") {
        let key = check(topic, &item)?;
        match list.iter().position(|x| zone_name(x).is_some_and(|k| same_key(&k, &key))) {
            Some(i) if merged_zone(&list[i], &item) == list[i] => {}
            Some(_) => out.refused.push(key),
            None => {
                list.push(fresh_zone(&item));
                out.added += 1;
            }
        }
    }
    for item in many(op, "up") {
        let key = check(topic, &item)?;
        match list.iter().position(|x| zone_name(x).is_some_and(|k| same_key(&k, &key))) {
            Some(i) => {
                let merged = merged_zone(&list[i], &item);
                if merged != list[i] {
                    list[i] = merged;
                    out.updated += 1;
                }
            }
            None => {
                list.push(fresh_zone(&item));
                out.added += 1;
            }
        }
    }
    for name in many(op, "rm") {
        let Some(name) = name.as_str() else { continue };
        let before = list.len();
        list.retain(|x| !zone_name(x).is_some_and(|k| same_key(&k, name)));
        out.removed += before - list.len();
    }
    if list.len() > topic.limit() {
        return Err(format!("too many items in {} ({})", topic.name(), list.len()));
    }
    let now = list.len();
    if out.added + out.updated + out.removed > 0 || text.is_none() {
        let written = serde_json::to_string(&doc).map_err(|e| e.to_string())?;
        write_file(dir, topic.file(), &written)?;
        note_change();
    }
    Ok(Some(out.describe(topic, now)))
}

fn person_ok(p: &Value) -> bool {
    !text_of(p, "key").is_empty() && !text_of(p, "name").is_empty() && text_of(p, "key").len() <= 80 && text_of(p, "name").len() <= 64
}

fn people_of(team: &Value) -> Vec<Value> {
    team.get("people").and_then(Value::as_array).cloned().unwrap_or_default()
}

fn person_matches(p: &Value, key: &str, name: &str) -> bool {
    (!key.is_empty() && text_of(p, "key").eq_ignore_ascii_case(key)) || (!name.is_empty() && same_name(&text_of(p, "name"), name))
}

fn base_teams(dir: &Path) -> (Vec<Value>, BTreeMap<String, u64>) {
    let root = std::fs::read_to_string(dir.join(Topic::Teams.file())).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or(Value::Null);
    let teams = root.get("teams").and_then(Value::as_array).cloned().unwrap_or_default();
    let touched = root.get("touched").and_then(Value::as_object).map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_u64()?))).collect()).unwrap_or_default();
    (teams, touched)
}

/// The counter the panel recorded for a person (0 when never edited), as `TeamDef::touch_of` finds it.
fn touch_of(touched: &BTreeMap<String, u64>, id: &str, name: &str) -> u64 {
    let (id, name) = (id.trim().to_lowercase(), name.trim().to_lowercase());
    touched
        .iter()
        .filter(|(k, _)| {
            let k = k.trim().to_lowercase();
            (!name.is_empty() && (k == name || k == format!("name:{name}"))) || (!id.is_empty() && (k == id || k.ends_with(&format!(":{id}"))))
        })
        .map(|(_, v)| *v)
        .max()
        .unwrap_or(0)
}

/// The panel's teams with the people who joined or left by chat put on top (what the server really enforces).
pub fn effective_teams(dir: &Path) -> Vec<Value> {
    let (mut teams, touched) = base_teams(dir);
    let members: groups::MembersFile = std::fs::read_to_string(dir.join(MEMBERS_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let find = |teams: &[Value], name: &str| teams.iter().position(|t| !text_of(t, "name").is_empty() && same_name(&text_of(t, "name"), name));
    for m in &members.left {
        let Some(i) = find(&teams, &m.team) else { continue };
        if touch_of(&touched, &m.id, &m.name) > m.touch {
            continue;
        }
        let (id, name) = (m.id.trim().to_lowercase(), m.name.trim().to_string());
        let kept: Vec<Value> = people_of(&teams[i]).into_iter().filter(|p| !person_matches(p, &id, &name)).collect();
        teams[i]["people"] = Value::Array(kept);
    }
    for m in &members.joined {
        let Some(i) = find(&teams, &m.team) else { continue };
        if touch_of(&touched, &m.id, &m.name) > m.touch {
            continue;
        }
        let (id, name) = (m.id.trim().to_lowercase(), m.name.trim().to_string());
        let mut people = people_of(&teams[i]);
        if !people.iter().any(|p| person_matches(p, &id, &name)) && !name.is_empty() {
            let key = if id.is_empty() { format!("name:{name}") } else { id.clone() };
            people.push(json!({ "key": key, "name": name }));
        }
        teams[i]["people"] = Value::Array(people);
    }
    for t in &mut teams {
        if t.get("people").is_none() {
            t["people"] = json!([]);
        }
        if t.get("leaders").is_none() {
            t["leaders"] = json!([]);
        }
    }
    teams
}

fn person_set(team: &Value) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = people_of(team).iter().map(|p| (text_of(p, "key").to_lowercase(), text_of(p, "name"))).collect();
    v.sort();
    v
}

fn clean_team(item: &Value) -> Result<Value, String> {
    let name = item_key(Topic::Teams, item).ok_or("a team has no usable name")?;
    let people: Vec<Value> = item.get("people").and_then(Value::as_array).cloned().unwrap_or_default();
    if people.len() > 200 || !people.iter().all(person_ok) {
        return Err(format!("team \"{name}\" has an unusable person"));
    }
    let mut seen: Vec<String> = Vec::new();
    let mut kept = Vec::new();
    for p in people {
        let key = text_of(&p, "key").to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        kept.push(json!({ "key": text_of(&p, "key"), "name": text_of(&p, "name") }));
    }
    let leaders: Vec<Value> = item.get("leaders").and_then(Value::as_array).map(|l| l.iter().filter_map(|k| k.as_str()).map(|k| json!(k.trim())).collect()).unwrap_or_default();
    Ok(json!({ "name": name, "people": kept, "leaders": leaders }))
}

fn apply_team_op(dir: &Path, op: &Value) -> Result<String, String> {
    let (base, mut touched) = base_teams(dir);
    let mut teams = effective_teams(dir);
    let next = touched.values().copied().max().unwrap_or(0) + 1;
    let mut out = Outcome::default();
    let mut moved: Vec<(String, String)> = Vec::new();

    for item in many(op, "rm") {
        let Some(name) = item.as_str() else { continue };
        let before = teams.len();
        teams.retain(|t| !same_key(&text_of(t, "name"), name));
        out.removed += before - teams.len();
    }
    for (field, create_only) in [("add", true), ("up", false)] {
        for item in many(op, field) {
            let team = clean_team(&item)?;
            let name = text_of(&team, "name");
            let at = teams.iter().position(|t| same_key(&text_of(t, "name"), &name));
            let mut old_people = Vec::new();
            match at {
                Some(i) => {
                    let same = person_set(&teams[i]) == person_set(&team) && {
                        let (mut a, mut b): (Vec<String>, Vec<String>) = (
                            teams[i].get("leaders").and_then(Value::as_array).map(|l| l.iter().filter_map(|x| x.as_str()).map(str::to_lowercase).collect()).unwrap_or_default(),
                            team.get("leaders").and_then(Value::as_array).map(|l| l.iter().filter_map(|x| x.as_str()).map(str::to_lowercase).collect()).unwrap_or_default(),
                        );
                        a.sort();
                        b.sort();
                        a == b
                    };
                    if same {
                        continue;
                    }
                    if create_only {
                        out.refused.push(name);
                        continue;
                    }
                    old_people = person_set(&teams[i]);
                    let members = teams[i].get("members").cloned().unwrap_or_else(|| json!([]));
                    let mut merged = team.clone();
                    merged["members"] = members;
                    teams[i] = merged;
                    out.updated += 1;
                }
                None => {
                    let mut merged = team.clone();
                    merged["members"] = json!([]);
                    teams.push(merged);
                    out.added += 1;
                }
            }
            let new_people = person_set(&team);
            for (key, pname) in new_people.iter().filter(|p| !old_people.contains(p)).chain(old_people.iter().filter(|p| !new_people.contains(p))) {
                moved.push((key.clone(), pname.clone()));
            }
            for (key, pname) in &new_people {
                for other in teams.iter_mut().filter(|t| !same_key(&text_of(t, "name"), &name)) {
                    let kept: Vec<Value> = people_of(other).into_iter().filter(|p| !person_matches(p, key, pname)).collect();
                    if kept.len() != people_of(other).len() {
                        other["people"] = Value::Array(kept);
                        moved.push((key.clone(), pname.clone()));
                    }
                }
            }
        }
    }
    if teams.len() > Topic::Teams.limit() {
        return Err(format!("too many teams ({})", teams.len()));
    }
    for (key, name) in &moved {
        for k in [key.trim().to_lowercase(), name.trim().to_lowercase(), format!("name:{}", name.trim().to_lowercase())] {
            if !k.is_empty() && k != "name:" {
                touched.insert(k, next);
            }
        }
    }
    if touched.len() > 400 {
        let mut by_age: Vec<(String, u64)> = touched.iter().map(|(k, v)| (k.clone(), *v)).collect();
        by_age.sort_by_key(|(_, v)| *v);
        for (k, _) in by_age.into_iter().take(touched.len() - 400) {
            touched.remove(&k);
        }
    }
    let unchanged = out.added == 0 && out.updated == 0 && out.removed == 0;
    if unchanged && base.len() == teams.len() {
        return Ok(Outcome { refused: out.refused, ..Default::default() }.describe(Topic::Teams, teams.len()));
    }
    let now = teams.len();
    let text = json!({ "teams": teams, "touched": touched }).to_string();
    write_file(dir, Topic::Teams.file(), &text)?;
    note_change();
    Ok(out.describe(Topic::Teams, now))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("m182_shared_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn zone(name: &str, x: f64) -> Value {
        json!({"name": name, "x": x, "z": 2.0, "radius": 30.0, "enabled": true, "safe": true, "team": 0, "lock": false, "lock_admins": false, "pvp": false})
    }

    fn read(dir: &Path, name: &str) -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir.join(name)).unwrap()).unwrap()
    }

    #[test]
    fn two_admins_adding_different_zones_both_stay() {
        let dir = scratch("zones");
        apply_op(&dir, &json!({"t": "zones", "add": [zone("Base norte", 1.0)]})).unwrap();
        apply_op(&dir, &json!({"t": "zones", "add": [zone("Base sur", 5.0)]})).unwrap();
        let names: Vec<String> = list_of(&read(&dir, "m182_zones.json"), Topic::Zones).iter().map(|z| text_of(z, "name")).collect();
        assert_eq!(names, vec!["Base norte", "Base sur"]);
    }

    #[test]
    fn a_zone_change_keeps_the_order_and_the_keys_it_does_not_know() {
        let dir = scratch("keep");
        let original = r#"{"zones":[{"name":"Home","x":1.5,"z":2.5,"radius":40.0,"team":2,"note":"mine","enabled":true,"lock":true},{"name":"Other","x":9.0,"z":9.0,"radius":20.0,"extra":{"b":1,"a":2}}]}"#;
        std::fs::write(dir.join("m182_zones.json"), original).unwrap();
        let before = revision(&dir, Topic::Zones);
        apply_op(&dir, &json!({"t": "zones", "up": [{"name": "Home", "x": 1.5, "z": 2.5, "radius": 40.0, "team": 2, "enabled": true, "lock": true}]})).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("m182_zones.json")).unwrap(), original, "a change that changes nothing writes nothing");
        assert_eq!(revision(&dir, Topic::Zones), before);
        apply_op(&dir, &json!({"t": "zones", "up": [{"name": "Home", "x": 1.5, "z": 2.5, "radius": 55.0, "team": 2, "enabled": true, "lock": true, "shield": true}]})).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("m182_zones.json")).unwrap(),
            r#"{"zones":[{"name":"Home","x":1.5,"z":2.5,"radius":55.0,"team":2,"note":"mine","enabled":true,"lock":true,"shield":true},{"name":"Other","x":9.0,"z":9.0,"radius":20.0,"extra":{"a":2,"b":1}}]}"#
        );
        apply_op(&dir, &json!({"t": "zones", "up": [{"name": "Home", "x": 1.5, "z": 2.5, "radius": 55.0, "team": 2, "enabled": true, "lock": true}]})).unwrap();
        let list = list_of(&read(&dir, "m182_zones.json"), Topic::Zones);
        assert!(list[0].get("shield").is_none(), "a key the panel leaves out is taken out");
        assert_eq!(list[0]["note"], "mine");
        apply_op(&dir, &json!({"t": "zones", "add": [zone("New", 3.0)], "rm": ["Other"]})).unwrap();
        let names: Vec<String> = list_of(&read(&dir, "m182_zones.json"), Topic::Zones).iter().map(|z| text_of(z, "name")).collect();
        assert_eq!(names, vec!["Home", "New"]);
    }

    #[test]
    fn creating_a_zone_that_exists_keeps_the_first_one() {
        let dir = scratch("clash");
        apply_op(&dir, &json!({"t": "zones", "add": [zone("Base", 1.0)]})).unwrap();
        let said = apply_op(&dir, &json!({"t": "zones", "add": [zone("base", 9.0)]})).unwrap();
        assert!(said.contains("already exist"), "{said}");
        let list = list_of(&read(&dir, "m182_zones.json"), Topic::Zones);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["x"], 1.0);
    }

    #[test]
    fn changing_and_removing_find_items_by_name() {
        let dir = scratch("edit");
        apply_op(&dir, &json!({"t": "zones", "add": [zone("A", 1.0), zone("B", 2.0), zone("C", 3.0)]})).unwrap();
        apply_op(&dir, &json!({"t": "zones", "up": [zone("B", 20.0)], "rm": ["A"]})).unwrap();
        let list = list_of(&read(&dir, "m182_zones.json"), Topic::Zones);
        assert_eq!(list.iter().map(|z| text_of(z, "name")).collect::<Vec<_>>(), vec!["B", "C"]);
        assert_eq!(list[0]["x"], 20.0);
    }

    #[test]
    fn repeating_the_same_change_does_nothing_new() {
        let dir = scratch("repeat");
        let op = json!({"t": "zones", "add": [zone("A", 1.0)]});
        apply_op(&dir, &op).unwrap();
        let before = revision(&dir, Topic::Zones);
        let said = apply_op(&dir, &op).unwrap();
        assert!(!said.contains("already exist"), "{said}");
        assert_eq!(revision(&dir, Topic::Zones), before);
    }

    #[test]
    fn places_are_a_plain_list_and_invalid_items_are_refused() {
        let dir = scratch("places");
        apply_op(&dir, &json!({"t": "places", "add": [{"name": "Torre", "x": 1.0, "y": 2.0, "z": 3.0}]})).unwrap();
        assert_eq!(read(&dir, "m182_places.json").as_array().unwrap().len(), 1);
        assert!(apply_op(&dir, &json!({"t": "places", "add": [{"name": "Rota", "x": 1.0}]})).is_err());
        assert!(apply_op(&dir, &json!({"t": "zones", "add": [{"name": "Gigante", "x": 1.0, "z": 1.0, "radius": 90000.0}]})).is_err());
        assert!(apply_op(&dir, &json!({"t": "spawners", "add": []})).is_err());
        assert!(apply_op(&dir, &json!({"add": []})).is_err());
    }

    #[test]
    fn a_damaged_file_is_never_overwritten_by_a_small_change() {
        let dir = scratch("damaged");
        std::fs::write(dir.join("m182_zones.json"), "not json").unwrap();
        assert!(apply_op(&dir, &json!({"t": "zones", "add": [zone("A", 1.0)]})).is_err());
        assert_eq!(std::fs::read_to_string(dir.join("m182_zones.json")).unwrap(), "not json");
    }

    fn team(name: &str, people: &[&str], leaders: &[&str]) -> Value {
        json!({"name": name, "people": people.iter().map(|p| json!({"key": format!("name:{p}"), "name": p})).collect::<Vec<_>>(), "leaders": leaders.iter().map(|p| format!("name:{p}")).collect::<Vec<_>>()})
    }

    #[test]
    fn people_belong_to_one_team_only() {
        let dir = scratch("teams");
        apply_op(&dir, &json!({"t": "teams", "add": [team("Rojo", &["Ana", "Luis"], &["Ana"]), team("Azul", &["Eva"], &[])]})).unwrap();
        apply_op(&dir, &json!({"t": "teams", "up": [team("Azul", &["Eva", "Luis"], &[])]})).unwrap();
        let teams = effective_teams(&dir);
        let names = |i: usize| person_set(&teams[i]).into_iter().map(|p| p.1).collect::<Vec<_>>();
        assert_eq!(names(0), vec!["Ana"]);
        assert_eq!(names(1), vec!["Eva", "Luis"]);
    }

    #[test]
    fn two_admins_editing_different_teams_do_not_undo_each_other() {
        let dir = scratch("twoteams");
        apply_op(&dir, &json!({"t": "teams", "add": [team("Rojo", &["Ana"], &[]), team("Azul", &["Eva"], &[])]})).unwrap();
        apply_op(&dir, &json!({"t": "teams", "up": [team("Rojo", &["Ana", "Bea"], &[])]})).unwrap();
        apply_op(&dir, &json!({"t": "teams", "up": [team("Azul", &["Eva", "Cris"], &[])]})).unwrap();
        let teams = effective_teams(&dir);
        assert_eq!(person_set(&teams[0]).len(), 2);
        assert_eq!(person_set(&teams[1]).len(), 2);
    }

    #[test]
    fn people_who_joined_in_game_show_up_and_a_later_panel_edit_wins() {
        let dir = scratch("members");
        apply_op(&dir, &json!({"t": "teams", "add": [team("Rojo", &["Ana"], &["Ana"])]})).unwrap();
        let joined = json!({"joined": [{"team": "Rojo", "id": "steamid:777", "name": "Zoe", "touch": 0}], "left": []});
        std::fs::write(dir.join("m182_members.json"), joined.to_string()).unwrap();
        let names: Vec<String> = person_set(&effective_teams(&dir)[0]).into_iter().map(|p| p.1).collect();
        assert_eq!(names, vec!["Ana", "Zoe"]);

        let rev_before = revision(&dir, Topic::Teams);
        let mut edited = team("Rojo", &["Ana"], &["Ana"]);
        edited["people"] = json!([{"key": "name:Ana", "name": "Ana"}]);
        apply_op(&dir, &json!({"t": "teams", "up": [edited]})).unwrap();
        let names: Vec<String> = person_set(&effective_teams(&dir)[0]).into_iter().map(|p| p.1).collect();
        assert_eq!(names, vec!["Ana"], "the admin removed Zoe after she joined");
        assert_ne!(revision(&dir, Topic::Teams), rev_before);
    }

    #[test]
    fn a_leave_in_game_shows_up_in_the_revision_and_the_snapshot() {
        let dir = scratch("leave");
        apply_op(&dir, &json!({"t": "teams", "add": [team("Rojo", &["Ana", "Luis"], &["Ana"])]})).unwrap();
        let before = revision(&dir, Topic::Teams);
        let left = json!({"joined": [], "left": [{"team": "Rojo", "id": "", "name": "Luis", "touch": 99}]});
        std::fs::write(dir.join("m182_members.json"), left.to_string()).unwrap();
        assert_ne!(revision(&dir, Topic::Teams), before);
        let names: Vec<String> = person_set(&effective_teams(&dir)[0]).into_iter().map(|p| p.1).collect();
        assert_eq!(names, vec!["Ana"]);
    }

    #[test]
    fn removing_a_team_and_unknown_removals() {
        let dir = scratch("rmteam");
        apply_op(&dir, &json!({"t": "teams", "add": [team("Rojo", &["Ana"], &[]), team("Azul", &[], &[])]})).unwrap();
        apply_op(&dir, &json!({"t": "teams", "rm": ["azul", "Nadie"]})).unwrap();
        assert_eq!(effective_teams(&dir).len(), 1);
    }

    #[test]
    fn revision_lines_round_trip_and_follow_the_content() {
        let dir = scratch("rev");
        let first = rev_line(&dir, 7);
        let parsed = parse_rev_line(&first).unwrap();
        assert_eq!(parsed.seq, 7);
        assert_eq!(parsed.hashes.len(), 5);
        apply_op(&dir, &json!({"t": "zones", "add": [zone("A", 1.0)]})).unwrap();
        let second = parse_rev_line(&rev_line(&dir, 8)).unwrap();
        let get = |r: &Revs, t: Topic| r.hashes.iter().find(|(x, _)| *x == t).unwrap().1.clone();
        assert_ne!(get(&parsed, Topic::Zones), get(&second, Topic::Zones));
        assert_eq!(get(&parsed, Topic::Places), get(&second, Topic::Places));
        assert!(parse_rev_line("[M182R#x] z=00000000").is_none());
        assert!(parse_rev_line("[M182R#1] z=zzzz").is_none());
        assert!(rev_line(&dir, 0).len() < 100);
    }

    #[test]
    fn a_big_topic_travels_in_a_few_short_lines_and_comes_back_whole() {
        let dir = scratch("snap");
        let rows: Vec<Value> = (0..300).map(|i| json!({"id": format!("s{i}"), "template": "df787789-d005-42e8-a18f-f4fe2e5d2e42", "at": [i as f64 * 1.5, 10.0, -3.25], "level": 20, "count": 3, "drops": [{"item": i, "guid": format!("{i:032x}")}]})).collect();
        let body = json!({"spawners": rows});
        std::fs::write(dir.join("m182_spawners.json"), body.to_string()).unwrap();
        let lines = snapshot_lines(&dir, 41, Topic::Spawners).unwrap();
        assert!(lines.len() > 1 && lines.len() < MAX_LINES);
        assert!(lines.iter().all(|l| l.len() < 700), "every line fits in the chat");
        assert!(lines.iter().all(|l| l.starts_with("[M182S#41 spawners ")));

        let mut assembler = Assembler::default();
        let now = Instant::now();
        let mut done = None;
        for line in lines.iter().rev() {
            if let Some(found) = assembler.feed(now, line).unwrap() {
                done = Some(found);
            }
        }
        let (topic, text) = done.expect("complete");
        assert_eq!(topic, "spawners");
        let envelope: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(envelope["data"], body);
        assert_eq!(envelope["hash"], revision(&dir, Topic::Spawners));
    }

    #[test]
    fn lines_that_are_cut_or_mixed_up_never_produce_a_file() {
        let dir = scratch("cut");
        apply_op(&dir, &json!({"t": "zones", "add": [zone("A", 1.0)]})).unwrap();
        let mut lines = snapshot_lines(&dir, 3, Topic::Zones).unwrap();
        lines.push(snapshot_lines(&dir, 3, Topic::Places).unwrap().remove(0));
        let mut assembler = Assembler::default();
        let now = Instant::now();
        let mut finished = 0;
        for line in &lines[..lines.len() - 1] {
            finished += assembler.feed(now, line).unwrap().is_some() as usize;
        }
        assert_eq!(finished, 1, "the zones answer is complete by itself");
        let broken = lines[0].replace("zones 1/", "zones 9/");
        assert!(assembler.feed(now, &broken).is_err() || assembler.feed(now, &broken).unwrap().is_none());
        assert!(assembler.feed(now, "[M182S#1 nothing 1/1 abcd] xx").is_err());
        let mut late = Assembler::default();
        let first = snapshot_lines(&dir, 4, Topic::Zones).unwrap();
        if first.len() > 1 {
            late.feed(now, &first[0]).unwrap();
            assert!(late.feed(now + ASSEMBLY_LIFE + Duration::from_secs(1), &first[1]).unwrap().is_none());
        }
    }

    #[test]
    fn the_client_writes_what_the_server_said() {
        let server = scratch("srv");
        let client = scratch("cli");
        apply_op(&server, &json!({"t": "places", "add": [{"name": "Torre", "x": 1.0, "y": 2.0, "z": 3.0}]})).unwrap();
        for line in snapshot_lines(&server, 5, Topic::Places).unwrap() {
            receive_snapshot(&client, &line);
        }
        let got = read(&client, "m182_remote_places.json");
        assert_eq!(got["topic"], "places");
        assert_eq!(got["data"].as_array().unwrap().len(), 1);
        assert_eq!(got["hash"], revision(&server, Topic::Places));

        receive_revs(&client, &rev_line(&server, 9));
        let rev = read(&client, REV_FILE);
        assert_eq!(rev["seq"], 9);
        assert_eq!(rev["rev"]["places"], revision(&server, Topic::Places));
    }

    #[test]
    fn only_recent_panels_still_in_their_slot_are_told_and_never_the_author() {
        forget_panels();
        let t0 = Instant::now();
        remember_panel(3, "Ana|steamid:1", t0);
        remember_panel(4, "Luis|steamid:2", t0);
        remember_panel(5, "Eva|steamid:3", t0);
        let present = vec![(3, "Ana|steamid:1".to_string()), (4, "Otro|steamid:9".to_string()), (5, "Eva|steamid:3".to_string())];
        assert_eq!(targets(t0, &present, 5), vec![3], "slot 4 now holds somebody else; 5 wrote the change");
        assert!(targets(t0 + PANEL_LIFE + Duration::from_secs(1), &present, 0).is_empty());
        remember_panel(3, "Ana|steamid:1", t0 + PANEL_LIFE);
        assert_eq!(targets(t0 + PANEL_LIFE + Duration::from_secs(1), &present, 0), vec![3]);
        forget_panels();
    }

    #[test]
    fn change_notices_are_raised_once() {
        forget_panels();
        let dir = scratch("flag");
        assert!(!take_change());
        apply_op(&dir, &json!({"t": "places", "add": [{"name": "A", "x": 1.0, "y": 1.0, "z": 1.0}]})).unwrap();
        assert!(take_change());
        assert!(!take_change());
    }
}

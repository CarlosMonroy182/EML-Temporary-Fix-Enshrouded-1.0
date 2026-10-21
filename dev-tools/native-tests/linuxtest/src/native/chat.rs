//! Chat command channel: lets an admin drive the native layer from the in-game chat,
//! which is the only way to reach a rented dedicated server that offers no console.
//!
//! A message such as `/m182 <key> set m182_zones.json {...}` is swallowed (other players
//! never see it) and turned into the same files the desktop panel writes.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const PREFIXES: [&str; 2] = ["/m182", "!m182"];
pub const MIN_KEY_LEN: usize = 6;

/// Files a chat command may write; each one is polled by the native layer.
pub const FILES: [&str; 7] = [
    "m182_spawn.request",
    "m182_command.request",
    "m182_powers.json",
    "m182_zones.json",
    "m182_teams.json",
    "m182_spawners.json",
    "m182_data.json",
];

pub const DATA_FILE: &str = "m182_data.json";
const DATA_TARGET: [&str; 4] = ["mods", "zz_m182_item_stats", "src", "m182_data.lua"];
const DATA_LIMIT: usize = 80 * 1024;

const NATIVE_CONFIG: &str = "m182_native.json";
const MAX_BUFFER: usize = 96 * 1024;
const BUFFER_LIFE: Duration = Duration::from_secs(120);
const MAX_FAILURES: usize = 8;
const FAILURE_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Not ours: the game handles the message as usual.
    Pass,
    /// Ours: hide it from the chat. The text is for the log (never contains the key).
    Swallowed(String),
    /// Ours: the message is replaced by the list of connected players (sequence number echoed).
    Roster(u32),
}

#[derive(Default)]
pub struct Session {
    buffers: HashMap<String, (String, Instant)>,
    failures: Vec<Instant>,
}

/// Chat client (plus one) that wrote the request waiting to be executed; 0 when none.
pub static REQUESTER: AtomicU32 = AtomicU32::new(0);

fn note_requester(file: &str, client: u32) {
    if file == "m182_spawn.request" || file == "m182_command.request" {
        REQUESTER.store(client.wrapping_add(1), Ordering::SeqCst);
    }
}

pub const POWERS_FILE: &str = "m182_powers.json";
pub const CLIENT_POWERS_PREFIX: &str = "m182_powers_";

/// Powers belong to the player that sent them, so each chat client gets a file of its own.
pub fn client_powers_file(client: u32) -> String {
    format!("{CLIENT_POWERS_PREFIX}{client}.json")
}

pub fn client_of_powers_file(name: &str) -> Option<u32> {
    name.strip_prefix(CLIENT_POWERS_PREFIX)?.strip_suffix(".json")?.parse().ok()
}

fn destination(name: &str, client: u32) -> String {
    if name == POWERS_FILE { client_powers_file(client) } else { name.to_string() }
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

static OTHER_SEEN: AtomicU32 = AtomicU32::new(0);

/// Logs the first few ordinary messages (their first characters only) so a silent
/// channel can be told apart from messages arriving in an unexpected shape.
pub fn note_other(message: &str, client: u32) {
    if OTHER_SEEN.fetch_add(1, Ordering::Relaxed) < 6 {
        let head: String = message.chars().take(6).collect();
        super::spawner::log(&format!("chat: client {client} sent an ordinary message ({} chars) starting \"{head}\"", message.chars().count()));
    }
}

pub fn has_prefix(text: &str) -> bool {
    let t = text.trim_start();
    PREFIXES.iter().any(|p| {
        t.len() >= p.len()
            && t.is_char_boundary(p.len())
            && t[..p.len()].eq_ignore_ascii_case(p)
            && t[p.len()..].chars().next().is_none_or(char::is_whitespace)
    })
}

pub fn valid_key(key: &str) -> bool {
    key.len() >= MIN_KEY_LEN && !key.chars().any(char::is_whitespace)
}

pub fn handle(text: &str, key: &str, client: u32, dir: &Path) -> Verdict {
    let Ok(mut guard) = SESSION.lock() else { return Verdict::Pass };
    let session = guard.get_or_insert_with(Session::default);
    handle_with(session, Instant::now(), text, key, client, dir)
}

fn next_token(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

pub fn handle_with(session: &mut Session, now: Instant, text: &str, key: &str, client: u32, dir: &Path) -> Verdict {
    if !valid_key(key) || !has_prefix(text) {
        return Verdict::Pass;
    }
    let t = text.trim_start();
    let (_, rest) = next_token(t);
    let (given, rest) = next_token(rest);

    session.failures.retain(|at| now.duration_since(*at) < FAILURE_WINDOW);
    if session.failures.len() >= MAX_FAILURES {
        return Verdict::Swallowed(format!("chat: client {client} ignored (too many wrong keys)"));
    }
    if !same(given, key) {
        session.failures.push(now);
        return Verdict::Swallowed(format!("chat: client {client} used a wrong key"));
    }

    session.buffers.retain(|_, (_, at)| now.duration_since(*at) < BUFFER_LIFE);

    let (command, rest) = next_token(rest);
    let lowered = command.to_ascii_lowercase();
    let (command, rest) = match lowered.as_str() {
        "setx" | "putx" | "addx" | "endx" => {
            let (file, hex) = next_token(rest);
            match unhex(hex) {
                Ok(body) => (lowered[..3].to_string(), format!("{file} {body}")),
                Err(why) => return Verdict::Swallowed(format!("chat: client {client} rejected: {why}")),
            }
        }
        _ => (lowered.clone(), rest.to_string()),
    };
    let rest = rest.as_str();
    if command == "players" {
        let (seq, _) = next_token(rest);
        return Verdict::Roster(seq.parse().unwrap_or(0));
    }
    let outcome = match command.as_str() {
        "ping" => Ok("ping".to_string()),
        "set" => {
            let (file, body) = next_token(rest);
            let written = write_json(dir, file, body, client);
            if written.is_ok() {
                note_requester(file, client);
            }
            written
        }
        "put" => {
            let (file, chunk) = next_token(rest);
            match allowed(file) {
                Some(name) if chunk.len() <= MAX_BUFFER => {
                    session.buffers.insert(name.to_string(), (chunk.to_string(), now));
                    Ok(format!("put {name} ({} bytes buffered)", chunk.len()))
                }
                Some(_) => Err("chunk too large".to_string()),
                None => Err(format!("file \"{file}\" is not allowed")),
            }
        }
        "add" => {
            let (file, chunk) = next_token(rest);
            match allowed(file) {
                Some(name) => match session.buffers.get_mut(name) {
                    Some((buffer, at)) if buffer.len() + chunk.len() <= MAX_BUFFER => {
                        buffer.push_str(chunk);
                        *at = now;
                        Ok(format!("add {name} ({} bytes buffered)", buffer.len()))
                    }
                    Some(_) => Err("buffer too large".to_string()),
                    None => Err(format!("nothing buffered for {name}; send put first")),
                },
                None => Err(format!("file \"{file}\" is not allowed")),
            }
        }
        "end" => {
            let (file, _) = next_token(rest);
            match allowed(file) {
                Some(name) => match session.buffers.remove(name) {
                    Some((buffer, _)) => {
                        let written = write_json(dir, name, &buffer, client);
                        if written.is_ok() {
                            note_requester(name, client);
                        }
                        written
                    }
                    None => Err(format!("nothing buffered for {name}")),
                },
                None => Err(format!("file \"{file}\" is not allowed")),
            }
        }
        "pvp" => {
            let (value, _) = next_token(rest);
            match value.to_ascii_lowercase().as_str() {
                "on" | "true" | "1" => set_pvp(dir, true),
                "off" | "false" | "0" => set_pvp(dir, false),
                _ => Err("pvp needs on or off".to_string()),
            }
        }
        "" => Err("missing command".to_string()),
        other => Err(format!("unknown command \"{other}\"")),
    };

    Verdict::Swallowed(match outcome {
        Ok(done) => format!("chat: client {client} ok: {done}"),
        Err(why) => format!("chat: client {client} rejected: {why}"),
    })
}

fn unhex(text: &str) -> Result<String, String> {
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if digits.len() % 2 != 0 {
        return Err("hex payload has an odd length".to_string());
    }
    let mut bytes = Vec::with_capacity(digits.len() / 2);
    for pair in digits.chunks(2) {
        let hi = (pair[0] as char).to_digit(16);
        let lo = (pair[1] as char).to_digit(16);
        match (hi, lo) {
            (Some(h), Some(l)) => bytes.push((h * 16 + l) as u8),
            _ => return Err("hex payload has a non-hex character".to_string()),
        }
    }
    String::from_utf8(bytes).map_err(|_| "hex payload is not text".to_string())
}

fn allowed(file: &str) -> Option<&'static str> {
    FILES.iter().copied().find(|f| f.eq_ignore_ascii_case(file))
}

fn write_json(dir: &Path, file: &str, body: &str, client: u32) -> Result<String, String> {
    let name = allowed(file).ok_or_else(|| format!("file \"{file}\" is not allowed"))?;
    if body.trim().is_empty() {
        return Err("empty content".to_string());
    }
    let value = serde_json::from_str::<serde_json::Value>(body).map_err(|e| format!("invalid JSON for {name}: {e}"))?;
    if name == DATA_FILE {
        return write_data(dir, &value);
    }
    let target = destination(name, client);
    store(dir, &target, body)?;
    Ok(format!("wrote {target} ({} bytes)", body.len()))
}

fn write_data(dir: &Path, value: &serde_json::Value) -> Result<String, String> {
    let lua = value.get("lua").and_then(|v| v.as_str()).ok_or_else(|| "m182_data.json needs a \"lua\" text".to_string())?;
    if lua.len() > DATA_LIMIT {
        return Err(format!("m182_data.json is too large ({} bytes)", lua.len()));
    }
    if !lua.trim_start().starts_with("return") || lua.contains('\0') {
        return Err("m182_data.json does not hold a data table".to_string());
    }
    let mut target = dir.to_path_buf();
    for part in &DATA_TARGET[..3] {
        target.push(part);
    }
    std::fs::create_dir_all(&target).map_err(|e| format!("cannot create the mod folder: {e}"))?;
    let name = DATA_TARGET[3];
    store(&target, name, lua)?;
    Ok(format!("wrote {} ({} bytes); it applies when the server starts", DATA_TARGET.join("/"), lua.len()))
}

fn store(dir: &Path, name: &str, body: &str) -> Result<(), String> {
    let temp = dir.join(format!("{name}.chat.tmp"));
    std::fs::write(&temp, body).map_err(|e| format!("cannot write {name}: {e}"))?;
    std::fs::rename(&temp, dir.join(name)).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("cannot replace {name}: {e}")
    })
}

fn set_pvp(dir: &Path, on: bool) -> Result<String, String> {
    let path = dir.join(NATIVE_CONFIG);
    let mut value = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<serde_json::Value>(&text).map_err(|e| format!("{NATIVE_CONFIG} is invalid: {e}"))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(format!("cannot read {NATIVE_CONFIG}: {e}")),
    };
    let Some(map) = value.as_object_mut() else { return Err(format!("{NATIVE_CONFIG} is not an object")) };
    map.insert("pvp".to_string(), serde_json::Value::Bool(on));
    let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    store(dir, NATIVE_CONFIG, &text)?;
    Ok(format!("pvp {}", if on { "on" } else { "off" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("m182_chat_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run(session: &mut Session, dir: &Path, text: &str) -> Verdict {
        handle_with(session, Instant::now(), text, "secret-key", 7, dir)
    }

    #[test]
    fn hex_commands_carry_json_without_special_characters() {
        let dir = scratch("hex");
        let mut s = Session::default();
        let hex = |t: &str| t.bytes().map(|b| format!("{b:02x}")).collect::<String>();
        let body = "{\"a\":[1,2,3]}";
        let v = run(&mut s, &dir, &format!("!m182 secret-key setx m182_zones.json {}", hex(body)));
        assert!(matches!(v, Verdict::Swallowed(ref l) if l.contains("ok")));
        assert_eq!(std::fs::read_to_string(dir.join("m182_zones.json")).unwrap(), body);
        let (a, b) = body.split_at(5);
        run(&mut s, &dir, &format!("!m182 secret-key putx m182_teams.json {}", hex(a)));
        run(&mut s, &dir, &format!("!m182 secret-key addx m182_teams.json {}", hex(b)));
        run(&mut s, &dir, "!m182 secret-key endx m182_teams.json");
        assert_eq!(std::fs::read_to_string(dir.join("m182_teams.json")).unwrap(), body);
        let Verdict::Swallowed(line) = run(&mut s, &dir, "!m182 secret-key setx m182_powers.json zz") else { panic!() };
        assert!(line.contains("rejected"));
    }

    #[test]
    fn ordinary_chat_is_left_alone() {
        let dir = scratch("plain");
        let mut s = Session::default();
        assert_eq!(run(&mut s, &dir, "hola a todos"), Verdict::Pass);
        assert_eq!(run(&mut s, &dir, "/m1823 secret-key ping"), Verdict::Pass);
        assert_eq!(run(&mut s, &dir, "/other secret-key ping"), Verdict::Pass);
    }

    #[test]
    fn nothing_is_intercepted_without_a_usable_key() {
        let dir = scratch("nokey");
        let mut s = Session::default();
        assert_eq!(handle_with(&mut s, Instant::now(), "/m182 x ping", "", 1, &dir), Verdict::Pass);
        assert_eq!(handle_with(&mut s, Instant::now(), "/m182 abc ping", "abc", 1, &dir), Verdict::Pass);
    }

    #[test]
    fn wrong_key_is_hidden_and_never_echoed() {
        let dir = scratch("wrong");
        let mut s = Session::default();
        let Verdict::Swallowed(line) = run(&mut s, &dir, "/m182 guess-key set m182_zones.json {}") else { panic!() };
        assert!(line.contains("wrong key"));
        assert!(!line.contains("guess-key"));
        assert!(!dir.join("m182_zones.json").exists());
    }

    #[test]
    fn repeated_wrong_keys_lock_the_channel_for_a_while() {
        let dir = scratch("lock");
        let mut s = Session::default();
        let start = Instant::now();
        for _ in 0..MAX_FAILURES {
            handle_with(&mut s, start, "/m182 nope-nope ping", "secret-key", 3, &dir);
        }
        let Verdict::Swallowed(line) = handle_with(&mut s, start, "/m182 secret-key set m182_zones.json {}", "secret-key", 3, &dir) else { panic!() };
        assert!(line.contains("too many"));
        assert!(!dir.join("m182_zones.json").exists());
        let later = start + FAILURE_WINDOW + Duration::from_secs(1);
        let Verdict::Swallowed(line) = handle_with(&mut s, later, "/m182 secret-key set m182_zones.json {\"a\":1}", "secret-key", 3, &dir) else { panic!() };
        assert!(line.contains("wrote"), "{line}");
    }

    #[test]
    fn set_writes_valid_json_to_allowed_files_only() {
        let dir = scratch("set");
        let mut s = Session::default();
        let Verdict::Swallowed(line) = run(&mut s, &dir, "!M182 secret-key SET m182_zones.json {\"zones\": []}") else { panic!() };
        assert!(line.contains("wrote m182_zones.json"), "{line}");
        assert_eq!(std::fs::read_to_string(dir.join("m182_zones.json")).unwrap(), "{\"zones\": []}");

        let Verdict::Swallowed(line) = run(&mut s, &dir, "/m182 secret-key set evil.dll {}") else { panic!() };
        assert!(line.contains("not allowed"));
        let Verdict::Swallowed(line) = run(&mut s, &dir, "/m182 secret-key set ../m182_zones.json {}") else { panic!() };
        assert!(line.contains("not allowed"));
        let Verdict::Swallowed(line) = run(&mut s, &dir, "/m182 secret-key set m182_teams.json {broken") else { panic!() };
        assert!(line.contains("invalid JSON"));
        assert!(!dir.join("m182_teams.json").exists());
        assert!(!dir.join("evil.dll").exists());
    }

    #[test]
    fn long_content_is_assembled_from_parts() {
        let dir = scratch("parts");
        let mut s = Session::default();
        run(&mut s, &dir, "/m182 secret-key put m182_teams.json {\"teams\": [[1,2],");
        let Verdict::Swallowed(line) = run(&mut s, &dir, "/m182 secret-key end m182_teams.json") else { panic!() };
        assert!(line.contains("invalid JSON"), "{line}");
        assert!(!dir.join("m182_teams.json").exists());

        run(&mut s, &dir, "/m182 secret-key put m182_teams.json {\"teams\": [[1,2],");
        run(&mut s, &dir, "/m182 secret-key add m182_teams.json [3,4]]}");
        let Verdict::Swallowed(line) = run(&mut s, &dir, "/m182 secret-key end m182_teams.json") else { panic!() };
        assert!(line.contains("wrote"), "{line}");
        let text = std::fs::read_to_string(dir.join("m182_teams.json")).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&text).unwrap()["teams"][1][1], 4);
        let Verdict::Swallowed(line) = run(&mut s, &dir, "/m182 secret-key end m182_teams.json") else { panic!() };
        assert!(line.contains("nothing buffered"));
    }

    #[test]
    fn buffers_expire() {
        let dir = scratch("expire");
        let mut s = Session::default();
        let t0 = Instant::now();
        handle_with(&mut s, t0, "/m182 secret-key put m182_zones.json {", "secret-key", 1, &dir);
        let Verdict::Swallowed(line) = handle_with(&mut s, t0 + BUFFER_LIFE + Duration::from_secs(1), "/m182 secret-key add m182_zones.json }", "secret-key", 1, &dir) else { panic!() };
        assert!(line.contains("nothing buffered"), "{line}");
    }

    #[test]
    fn pvp_edits_only_that_field_of_the_native_config() {
        let dir = scratch("pvp");
        std::fs::write(dir.join(NATIVE_CONFIG), "{\"spawner\": true, \"pvp\": false, \"spawn_level\": 30}").unwrap();
        let mut s = Session::default();
        let Verdict::Swallowed(line) = run(&mut s, &dir, "/m182 secret-key pvp on") else { panic!() };
        assert!(line.contains("pvp on"), "{line}");
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(NATIVE_CONFIG)).unwrap()).unwrap();
        assert_eq!(v["pvp"], true);
        assert_eq!(v["spawner"], true);
        assert_eq!(v["spawn_level"], 30);
        let Verdict::Swallowed(line) = run(&mut s, &dir, "/m182 secret-key pvp maybe") else { panic!() };
        assert!(line.contains("on or off"));
    }

    #[test]
    fn ping_and_unknown_commands_are_answered_in_the_log() {
        let dir = scratch("misc");
        let mut s = Session::default();
        assert_eq!(run(&mut s, &dir, "/m182 secret-key ping"), Verdict::Swallowed("chat: client 7 ok: ping".into()));
        let Verdict::Swallowed(line) = run(&mut s, &dir, "/m182 secret-key dance") else { panic!() };
        assert!(line.contains("unknown command"));
        let Verdict::Swallowed(line) = run(&mut s, &dir, "/m182 secret-key") else { panic!() };
        assert!(line.contains("missing command"));
    }
}

//! Drives a dedicated server from the in-game panel: whatever the panel would write for the
//! native layer is typed into the game chat as `/m182 <key> ...` lines, which the server's chat
//! channel turns into the same files.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

pub use crate::remote_proto::{DEFAULT_LIMIT, MAX_LIMIT, MIN_LIMIT};
use crate::remote_proto as proto;

pub const SETTINGS_FILE: &str = "m182_panel.json";
pub const CONFIG_KEY: &str = "remote_key";

/// Files the server accepts through chat.
pub const FORWARDED: [&str; 6] = [
    "m182_spawn.request",
    "m182_command.request",
    "m182_powers.json",
    "m182_zones.json",
    "m182_teams.json",
    "m182_spawners.json",
];

/// Requests that only make sense on the server: they are not written next to the game.
const REMOTE_ONLY: [&str; 2] = ["m182_spawn.request", "m182_command.request"];

/// Types lines into the game; implemented by the Windows host.
pub trait Typist: Send {
    fn begin(&mut self) -> Result<(), String>;
    fn line(&mut self, text: &str) -> Result<(), String>;
    fn end(&mut self);
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub key: String,
    pub limit: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub queued: usize,
    pub sending: bool,
    pub sent_lines: u64,
    pub last: String,
    pub error: String,
}

struct Job {
    file: String,
    lines: Vec<String>,
}

struct State {
    settings: Settings,
    queue: VecDeque<Job>,
    typist: Option<Box<dyn Typist>>,
    installed: bool,
    worker: bool,
    status: Status,
}

static STATE: Mutex<State> = Mutex::new(State {
    settings: Settings { enabled: false, key: String::new(), limit: DEFAULT_LIMIT },
    queue: VecDeque::new(),
    typist: None,
    installed: false,
    worker: false,
    status: Status { queued: 0, sending: false, sent_lines: 0, last: String::new(), error: String::new() },
});

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn read_settings(dir: &Path) -> Settings {
    let config: Value = std::fs::read_to_string(dir.join("m182_native.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let key = config.get(CONFIG_KEY).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let panel: Value = std::fs::read_to_string(dir.join(SETTINGS_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let enabled = panel.get("remote").and_then(Value::as_bool).unwrap_or_else(|| !key.is_empty());
    let limit = panel.get("remote_limit").and_then(Value::as_u64).map(|l| l as usize).unwrap_or(DEFAULT_LIMIT).clamp(MIN_LIMIT, MAX_LIMIT);
    Settings { enabled, key, limit }
}

pub fn save_settings(dir: &Path, settings: &Settings) -> Result<(), String> {
    let mut config: serde_json::Map<String, Value> = std::fs::read_to_string(dir.join("m182_native.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    config.entry("enabled").or_insert(json!(true));
    config.insert(CONFIG_KEY.to_string(), json!(settings.key.trim()));
    let text = serde_json::to_string_pretty(&Value::Object(config)).map_err(|e| e.to_string())?;
    plain_write(dir, "m182_native.json", &text)?;
    let panel = json!({"remote": settings.enabled, "remote_limit": settings.limit});
    plain_write(dir, SETTINGS_FILE, &panel.to_string())
}

fn plain_write(dir: &Path, name: &str, text: &str) -> Result<(), String> {
    let temp = dir.join(format!("{name}.tmp"));
    std::fs::write(&temp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&temp, dir.join(name)).map_err(|e| e.to_string())
}

pub fn configure(settings: Settings) {
    state().settings = settings;
}

pub fn settings() -> Settings {
    state().settings.clone()
}

pub fn install_typist(typist: Box<dyn Typist>) {
    let mut s = state();
    s.typist = Some(typist);
    s.installed = true;
    if !s.worker {
        s.worker = true;
        drop(s);
        if std::thread::Builder::new().name("m182-remote".into()).spawn(worker).is_err() {
            let mut s = state();
            s.worker = false;
            s.status.error = "No se pudo iniciar el envío al servidor.".to_string();
        }
    }
}

/// True when the panel is set to drive a server and can type into the game.
pub fn active() -> bool {
    let s = state();
    s.settings.enabled && proto::valid_key(&s.settings.key) && s.installed
}

/// How a file the panel wants to write must be handled.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Route {
    Local,
    Both,
    ServerOnly,
}

pub fn route(name: &str) -> Route {
    if !active() || !FORWARDED.contains(&name) {
        return Route::Local;
    }
    if REMOTE_ONLY.contains(&name) {
        Route::ServerOnly
    } else {
        Route::Both
    }
}

fn push(job: Job, replace: bool) {
    let mut s = state();
    if replace {
        s.queue.retain(|j| j.file != job.file);
    }
    s.queue.push_back(job);
    s.status.queued = s.queue.len();
    s.status.error.clear();
}

/// Queues the file for the server. State files replace an older copy that was not sent yet.
pub fn submit(name: &str, text: &str) -> Result<(), String> {
    let (key, limit) = {
        let s = state();
        (s.settings.key.clone(), s.settings.limit)
    };
    let body = serde_json::from_str::<Value>(text).map_err(|e| format!("{name}: {e}"))?.to_string();
    let replace = !REMOTE_ONLY.contains(&name);
    push(Job { file: name.to_string(), lines: proto::compose(&key, name, &body, limit) }, replace);
    Ok(())
}

pub fn submit_pvp(on: bool) {
    let key = state().settings.key.clone();
    push(Job { file: "pvp".to_string(), lines: vec![proto::pvp(&key, on)] }, true);
}

pub fn submit_ping() {
    let key = state().settings.key.clone();
    push(Job { file: "ping".to_string(), lines: vec![proto::ping(&key)] }, true);
}

pub fn status() -> Status {
    let mut s = state();
    let queued = s.queue.len();
    s.status.queued = queued;
    s.status.clone()
}

pub fn clear_queue() {
    let mut s = state();
    s.queue.clear();
    s.status.queued = 0;
}

fn worker() {
    loop {
        std::thread::sleep(Duration::from_millis(150));
        let (job, typist) = {
            let mut s = state();
            if s.queue.is_empty() || s.typist.is_none() {
                continue;
            }
            let job = s.queue.pop_front();
            s.status.queued = s.queue.len();
            s.status.sending = true;
            (job, s.typist.take())
        };
        let (Some(job), Some(mut typist)) = (job, typist) else { continue };
        let started = Instant::now();
        let mut error = String::new();
        let mut sent = 0u64;
        match typist.begin() {
            Ok(()) => {
                for line in &job.lines {
                    match typist.line(line) {
                        Ok(()) => sent += 1,
                        Err(e) => {
                            error = e;
                            break;
                        }
                    }
                }
                typist.end();
            }
            Err(e) => error = e,
        }
        let mut s = state();
        s.typist = Some(typist);
        s.status.sending = false;
        s.status.sent_lines += sent;
        if error.is_empty() {
            s.status.last = format!("{} enviado ({} mensaje{}, {:.1} s)", job.file, job.lines.len(), if job.lines.len() == 1 { "" } else { "s" }, started.elapsed().as_secs_f32());
        } else {
            s.status.error = format!("{}: {error}", job.file);
            s.queue.clear();
        }
    }
}

#[cfg(test)]
pub fn reset_for_tests() {
    let mut s = state();
    s.queue.clear();
    s.typist = None;
    s.installed = false;
    s.settings = Settings { enabled: false, key: String::new(), limit: DEFAULT_LIMIT };
    s.status = Status::default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    static GUARD: Mutex<()> = Mutex::new(());

    struct Recorder(Arc<Mutex<Vec<String>>>, bool);

    impl Typist for Recorder {
        fn begin(&mut self) -> Result<(), String> {
            self.0.lock().unwrap().push("<begin>".into());
            Ok(())
        }
        fn line(&mut self, text: &str) -> Result<(), String> {
            if self.1 && text.contains("fail") {
                return Err("sin foco".into());
            }
            self.0.lock().unwrap().push(text.to_string());
            Ok(())
        }
        fn end(&mut self) {
            self.0.lock().unwrap().push("<end>".into());
        }
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("m182_remote_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wait_idle() {
        for _ in 0..200 {
            std::thread::sleep(Duration::from_millis(25));
            let st = status();
            if st.queued == 0 && !st.sending {
                return;
            }
        }
        panic!("the sender never went idle");
    }

    #[test]
    fn settings_come_from_the_config_and_the_panel_file() {
        let dir = temp("settings");
        assert_eq!(read_settings(&dir), Settings { enabled: false, key: String::new(), limit: DEFAULT_LIMIT });
        std::fs::write(dir.join("m182_native.json"), r#"{"pvp":true,"remote_key":" abcdef1 "}"#).unwrap();
        assert_eq!(read_settings(&dir), Settings { enabled: true, key: "abcdef1".into(), limit: DEFAULT_LIMIT });
        std::fs::write(dir.join(SETTINGS_FILE), r#"{"remote":false,"remote_limit":5000}"#).unwrap();
        assert_eq!(read_settings(&dir), Settings { enabled: false, key: "abcdef1".into(), limit: MAX_LIMIT });
        save_settings(&dir, &Settings { enabled: true, key: "newkey99".into(), limit: 150 }).unwrap();
        let back = read_settings(&dir);
        assert_eq!(back, Settings { enabled: true, key: "newkey99".into(), limit: 150 });
        let config: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("m182_native.json")).unwrap()).unwrap();
        assert_eq!(config["pvp"], true);
        assert_eq!(config["enabled"], true);
    }

    #[test]
    fn nothing_is_forwarded_unless_the_panel_drives_a_server_and_can_type() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_tests();
        assert_eq!(route("m182_spawn.request"), Route::Local);
        configure(Settings { enabled: true, key: "abcdef1".into(), limit: DEFAULT_LIMIT });
        assert_eq!(route("m182_spawn.request"), Route::Local);
        install_typist(Box::new(Recorder(Arc::new(Mutex::new(Vec::new())), false)));
        assert_eq!(route("m182_spawn.request"), Route::ServerOnly);
        assert_eq!(route("m182_command.request"), Route::ServerOnly);
        assert_eq!(route("m182_zones.json"), Route::Both);
        assert_eq!(route("m182_places.json"), Route::Local);
        assert_eq!(route("m182_native.json"), Route::Local);
        configure(Settings { enabled: false, key: "abcdef1".into(), limit: DEFAULT_LIMIT });
        assert_eq!(route("m182_zones.json"), Route::Local);
        configure(Settings { enabled: true, key: "abc".into(), limit: DEFAULT_LIMIT });
        assert_eq!(route("m182_zones.json"), Route::Local);
    }

    #[test]
    fn lines_are_typed_in_order_and_state_files_keep_only_the_latest() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_tests();
        let typed = Arc::new(Mutex::new(Vec::new()));
        configure(Settings { enabled: true, key: "abcdef1".into(), limit: DEFAULT_LIMIT });
        // queued before a typist exists: nothing is sent yet
        submit("m182_zones.json", r#"{"zones":[]}"#).unwrap();
        submit("m182_zones.json", r#"{"zones":[{"name":"B"}]}"#).unwrap();
        submit("m182_command.request", r#"{"cmd":"heal"}"#).unwrap();
        submit("m182_command.request", r#"{"cmd":"teleport"}"#).unwrap();
        assert_eq!(status().queued, 3);
        install_typist(Box::new(Recorder(typed.clone(), false)));
        wait_idle();
        let lines = typed.lock().unwrap().clone();
        let sent: Vec<&String> = lines.iter().filter(|l| l.starts_with('/')).collect();
        assert_eq!(sent.len(), 3, "{lines:?}");
        assert!(sent[0].contains(r#"{"zones":[{"name":"B"}]}"#));
        assert!(sent[1].contains("heal") && sent[2].contains("teleport"));
        assert_eq!(lines.iter().filter(|l| *l == "<begin>").count(), 3);
        assert_eq!(lines.iter().filter(|l| *l == "<end>").count(), 3);
        assert!(status().last.contains("m182_command.request"));
    }

    #[test]
    fn a_failure_stops_the_queue_and_is_reported() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_tests();
        let typed = Arc::new(Mutex::new(Vec::new()));
        configure(Settings { enabled: true, key: "abcdef1".into(), limit: DEFAULT_LIMIT });
        install_typist(Box::new(Recorder(typed.clone(), true)));
        submit("m182_command.request", r#"{"cmd":"fail"}"#).unwrap();
        submit("m182_command.request", r#"{"cmd":"heal"}"#).unwrap();
        wait_idle();
        std::thread::sleep(Duration::from_millis(200));
        let st = status();
        assert!(st.error.contains("sin foco"), "{st:?}");
        assert!(!typed.lock().unwrap().iter().any(|l| l.contains("heal")));
        assert!(submit("m182_zones.json", "not json").is_err());
    }
}

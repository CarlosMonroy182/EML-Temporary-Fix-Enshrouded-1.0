use std::path::Path;
use std::time::SystemTime;

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub slot: u32,
    pub name: String,
    pub platform: String,
    pub id: String,
}

impl Member {
    pub fn key(&self) -> String {
        if self.id.is_empty() {
            format!("name:{}", self.name)
        } else {
            format!("{}:{}", self.platform, self.id)
        }
    }

    pub fn id_label(&self) -> String {
        if self.id.is_empty() {
            return String::new();
        }
        let platform = match self.platform.as_str() {
            "steamid" | "steam" => "Steam",
            "psn" | "ps5" | "psnid" => "PS5",
            "xbl" | "xbox" | "xuid" => "Xbox",
            other => other,
        };
        format!("{platform} {}", self.id)
    }
}

const SESSION_START: &str = "Switching state from Create to Initialize";

fn between<'a>(line: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = line.find(start)? + start.len();
    let rest = &line[from..];
    let to = rest.rfind(end)?;
    Some(&rest[..to])
}

fn handle_index(text: &str) -> Option<u32> {
    let open = text.find('(')?;
    let close = text[open..].find(')')? + open;
    text[open + 1..close].trim().parse().ok()
}

/// Reads the game's own log and returns the players connected to the world right now.
pub fn parse_log(text: &str) -> Vec<Member> {
    let start = text.rfind(SESSION_START).and_then(|at| text[..at].rfind('\n').map(|n| n + 1)).unwrap_or(0);
    let mut members: Vec<Member> = Vec::new();
    let mut pending: Vec<(u32, String, String)> = Vec::new();
    let mut expecting: Option<u32> = None;
    for line in text[start..].lines() {
        if let Some(rest) = line.split("Added peer ").nth(1) {
            if let (Some(slot), Some(open)) = (handle_index(rest), rest.rfind(" (")) {
                let tag = rest[open + 2..].trim().trim_end_matches(')');
                if let Some((platform, id)) = tag.split_once(':') {
                    pending.retain(|p| p.0 != slot);
                    pending.push((slot, platform.trim().to_string(), id.trim().to_string()));
                }
            }
            continue;
        }
        if line.contains("[server] Machine '") && line.contains("logged in") {
            expecting = line.split("Machine '").nth(1).and_then(|r| r.split('\'').next()).and_then(|m| m.parse().ok());
            continue;
        }
        if line.contains("[server] Player '") && line.contains("' logged in") {
            let Some(name) = between(line, "Player '", "' logged in") else { continue };
            let slot = expecting.take().unwrap_or(0);
            let (platform, id) = pending
                .iter()
                .find(|p| p.0 == slot)
                .map(|p| (p.1.clone(), p.2.clone()))
                .unwrap_or_default();
            members.retain(|m| m.slot != slot);
            members.push(Member { slot, name: name.to_string(), platform, id });
            continue;
        }
        if let Some(rest) = line.split("Player removed. Player handle: ").nth(1) {
            if let Some(slot) = handle_index(rest) {
                members.retain(|m| m.slot != slot);
                pending.retain(|p| p.0 != slot);
            }
            continue;
        }
        if line.contains("Removing all peers") {
            members.retain(|m| m.slot == 0);
            pending.clear();
        }
    }
    members.sort_by_key(|m| m.slot);
    members
}

pub struct LogWatch {
    stamp: Option<(SystemTime, u64)>,
    pub members: Vec<Member>,
}

impl LogWatch {
    pub fn new() -> Self {
        Self { stamp: None, members: Vec::new() }
    }

    pub fn refresh(&mut self, dir: &Path) {
        let path = dir.join("enshrouded.log");
        let Ok(meta) = std::fs::metadata(&path) else {
            self.members.clear();
            self.stamp = None;
            return;
        };
        let stamp = (meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len());
        if self.stamp == Some(stamp) {
            return;
        }
        self.stamp = Some(stamp);
        if let Ok(bytes) = std::fs::read(&path) {
            self.members = parse_log(&String::from_utf8_lossy(&bytes));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\
[I 00:00:00,001] [GameFramework] Platform: win32_x64
[I 00:00:23,062] [game_server] Switching state from Create to Initialize after 219.00 us
[I 00:01:17,991] [server] Machine '0': Player '0(0)' logged in
[I 00:01:17,993] [server] Player 'moradoox' logged in with Permissions:
[I 00:01:48,300] [online] Session accepted with peer (steamid:76561199474170255)
[I 00:01:48,300] [online] Added peer 0(1) (steamid:76561199474170255)
[I 00:02:12,047] [server] Machine '1': Player '1(1)' logged in
[I 00:02:12,049] [server] Player 'Nacht' logged in with Permissions:
";

    #[test]
    fn reads_names_and_platform_ids() {
        let list = parse_log(LOG);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "moradoox");
        assert_eq!(list[0].slot, 0);
        assert_eq!(list[1].name, "Nacht");
        assert_eq!(list[1].id, "76561199474170255");
        assert_eq!(list[1].key(), "steamid:76561199474170255");
        assert_eq!(list[1].id_label(), "Steam 76561199474170255");
        assert_eq!(list[0].key(), "name:moradoox");
    }

    #[test]
    fn leaving_players_disappear_and_old_sessions_are_ignored() {
        let mut text = String::from("[I 00:00:01,000] [server] Machine '0': Player '0(0)' logged in\n[I 00:00:01,001] [server] Player 'Viejo' logged in with Permissions:\n");
        text.push_str(LOG);
        assert_eq!(parse_log(&text).len(), 2);
        text.push_str("[I 01:09:17,738] [online] Removed peer 0(1)\n[I 01:09:17,742] [session] Player removed. Player handle: 1(1)\n");
        let list = parse_log(&text);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "moradoox");
    }

    #[test]
    fn other_platforms_and_names_with_quotes() {
        let text = "[I 1] [game_server] Switching state from Create to Initialize after 1 us\n[I 2] [online] Added peer 0(2) (psn:abc123)\n[I 3] [server] Machine '2': Player '2(2)' logged in\n[I 4] [server] Player 'O'Neil' logged in with Permissions:\n";
        let list = parse_log(text);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].slot, 2);
        assert_eq!(list[0].name, "O'Neil");
        assert_eq!(list[0].id_label(), "PS5 abc123");
    }

    #[test]
    fn watch_follows_the_file() {
        let dir = std::env::temp_dir().join(format!("m182_session_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = LogWatch::new();
        w.refresh(&dir);
        assert!(w.members.is_empty());
        std::fs::write(dir.join("enshrouded.log"), LOG).unwrap();
        w.refresh(&dir);
        assert_eq!(w.members.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}

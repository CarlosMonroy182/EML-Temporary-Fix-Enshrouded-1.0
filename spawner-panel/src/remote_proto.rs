//! Chat lines understood by the server's chat channel (`/m182 <key> ...`).

pub const MIN_LIMIT: usize = 80;
pub const DEFAULT_LIMIT: usize = 200;
pub const MAX_LIMIT: usize = 900;

fn head(key: &str, command: &str, file: &str) -> String {
    format!("/m182 {key} {command} {file} ")
}

fn cut(text: &str, from: usize, size: usize) -> usize {
    let rest = &text[from..];
    if rest.len() <= size {
        return text.len();
    }
    let mut end = size;
    while !rest.is_char_boundary(end) {
        end -= 1;
    }
    let ok = |at: usize| {
        let before = rest[..at].chars().next_back();
        let after = rest[at..].chars().next();
        at > 0 && !before.is_some_and(char::is_whitespace) && !after.is_some_and(char::is_whitespace)
    };
    let mut at = end;
    while at > 0 && !ok(at) {
        at -= 1;
        while at > 0 && !rest.is_char_boundary(at) {
            at -= 1;
        }
    }
    if at == 0 {
        at = end;
    }
    from + at
}

/// Every chat line needed to write `body` (compact JSON) into `file` on the server.
pub fn compose(key: &str, file: &str, body: &str, limit: usize) -> Vec<String> {
    let limit = limit.clamp(MIN_LIMIT, MAX_LIMIT);
    let single = format!("{}{}", head(key, "set", file), body);
    if single.len() <= limit {
        return vec![single];
    }
    let room = limit.saturating_sub(head(key, "add", file).len()).max(16);
    let mut lines = Vec::new();
    let mut from = 0;
    while from < body.len() {
        let to = cut(body, from, room);
        let command = if from == 0 { "put" } else { "add" };
        lines.push(format!("{}{}", head(key, command, file), &body[from..to]));
        from = to;
    }
    lines.push(format!("/m182 {key} end {file}"));
    lines
}

pub fn pvp(key: &str, on: bool) -> String {
    format!("/m182 {key} pvp {}", if on { "on" } else { "off" })
}

pub fn ping(key: &str) -> String {
    format!("/m182 {key} ping")
}

pub fn valid_key(key: &str) -> bool {
    key.len() >= 6 && !key.chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rebuild(lines: &[String]) -> String {
        let mut out = String::new();
        for line in lines {
            let mut parts = line.splitn(5, ' ');
            let (_, _, command, _file) = (parts.next(), parts.next(), parts.next(), parts.next());
            if matches!(command, Some("put" | "add" | "set")) {
                out.push_str(parts.next().unwrap());
            }
        }
        out
    }

    #[test]
    fn short_bodies_go_in_one_line() {
        let lines = compose("secret-key", "m182_spawn.request", r#"{"template":"x"}"#, 200);
        assert_eq!(lines, vec![r#"/m182 secret-key set m182_spawn.request {"template":"x"}"#.to_string()]);
    }

    #[test]
    fn long_bodies_are_split_and_rebuilt_exactly() {
        let body = serde_json::json!({"zones": (0..12).map(|i| serde_json::json!({"name": format!("Zona número {i} ñandú"), "x": i * 100, "z": -i * 33, "radius": 20})).collect::<Vec<_>>()}).to_string();
        for limit in [80usize, 120, 200, 333, 900] {
            let lines = compose("secret-key", "m182_zones.json", &body, limit);
            assert!(lines.len() >= 2 || body.len() < limit);
            assert!(lines.iter().all(|l| l.len() <= limit.clamp(MIN_LIMIT, MAX_LIMIT)), "limit {limit}: {:?}", lines.iter().map(|l| l.len()).collect::<Vec<_>>());
            assert!(lines[0].contains(" put ") || lines.len() == 1);
            assert!(lines.last().unwrap().ends_with(" end m182_zones.json") || lines.len() == 1);
            assert_eq!(rebuild(&lines), body, "limit {limit}");
            for l in &lines {
                let payload = l.splitn(5, ' ').nth(4).unwrap_or("");
                assert!(payload.is_empty() || (!payload.starts_with(' ') && !payload.ends_with(' ')), "{l:?}");
            }
        }
    }

    #[test]
    fn small_commands() {
        assert_eq!(pvp("abcdefg", true), "/m182 abcdefg pvp on");
        assert_eq!(pvp("abcdefg", false), "/m182 abcdefg pvp off");
        assert_eq!(ping("abcdefg"), "/m182 abcdefg ping");
        assert!(valid_key("abcdef") && !valid_key("abc") && !valid_key("abc def"));
    }
}

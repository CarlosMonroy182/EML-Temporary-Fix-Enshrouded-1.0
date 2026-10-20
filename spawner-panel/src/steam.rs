use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub name: String,
    pub id: String,
}

fn quoted(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('"') else { break };
        out.push(&after[..close]);
        rest = &after[close + 1..];
    }
    out
}

/// The account Steam logged in last, from `config/loginusers.vdf`.
pub fn parse_login_users(text: &str) -> Option<Account> {
    let mut found: Vec<(Account, bool)> = Vec::new();
    let mut current: Option<(String, String, bool)> = None;
    let mut depth = 0i32;
    let mut pending_id: Option<String> = None;
    for line in text.lines() {
        let t = line.trim();
        if t == "{" {
            depth += 1;
            if depth == 2 {
                current = Some((pending_id.take().unwrap_or_default(), String::new(), false));
            }
            continue;
        }
        if t == "}" {
            if depth == 2 {
                if let Some((id, name, recent)) = current.take() {
                    if !id.is_empty() && !name.is_empty() {
                        found.push((Account { name, id }, recent));
                    }
                }
            }
            depth -= 1;
            continue;
        }
        let parts = quoted(t);
        match (depth, parts.as_slice()) {
            (1, [id]) if id.chars().all(|c| c.is_ascii_digit()) && !id.is_empty() => pending_id = Some((*id).to_string()),
            (2, [key, value]) => {
                if let Some((_, name, recent)) = current.as_mut() {
                    match key.to_ascii_lowercase().as_str() {
                        "personaname" => *name = (*value).to_string(),
                        "mostrecent" => *recent = *value == "1",
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let at = found.iter().position(|(_, recent)| *recent).unwrap_or(0);
    found.into_iter().nth(at).map(|(account, _)| account)
}

fn candidates(game_dir: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(base) = std::env::var_os(var) {
            roots.push(PathBuf::from(base).join("Steam"));
        }
    }
    for ancestor in game_dir.ancestors() {
        roots.push(ancestor.to_path_buf());
        roots.push(ancestor.join("Steam"));
    }
    roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    roots.into_iter().map(|r| r.join("config").join("loginusers.vdf")).collect()
}

pub fn local_account(game_dir: &Path) -> Option<Account> {
    candidates(game_dir).into_iter().find_map(|path| parse_login_users(&std::fs::read_to_string(path).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_used_last_is_the_one_listed() {
        let text = "\"users\"\n{\n\t\"76561190000000001\"\n\t{\n\t\t\"AccountName\"\t\t\"old\"\n\t\t\"PersonaName\"\t\t\"Viejo\"\n\t\t\"MostRecent\"\t\t\"0\"\n\t}\n\t\"76561199474170255\"\n\t{\n\t\t\"AccountName\"\t\t\"moradoox\"\n\t\t\"PersonaName\"\t\t\"moradoox\"\n\t\t\"RememberPassword\"\t\t\"1\"\n\t\t\"MostRecent\"\t\t\"1\"\n\t}\n}\n";
        assert_eq!(parse_login_users(text), Some(Account { name: "moradoox".into(), id: "76561199474170255".into() }));
        assert_eq!(parse_login_users("nothing"), None);
        let single = "\"users\"\n{\n\t\"76561190000000001\"\n\t{\n\t\t\"PersonaName\"\t\t\"Solo\"\n\t}\n}\n";
        assert_eq!(parse_login_users(single).map(|a| a.name), Some("Solo".to_string()));
    }
}

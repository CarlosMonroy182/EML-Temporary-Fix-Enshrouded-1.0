use std::fs;
use std::path::{Path, PathBuf};

pub const SETTINGS_KEY: &str = "profile";
const FOLDER: &str = "profiles";
const MARKER: &str = "active.txt";
const FILES: [&str; 7] = [
    "m182_zones.json",
    "m182_spawners.json",
    "m182_teams.json",
    "m182_places.json",
    "m182_raid_templates.json",
    "m182_sent.json",
    "m182_events.json",
];

pub fn clean_name(raw: &str) -> Option<String> {
    let name: String = raw.trim().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(32).collect();
    (!name.is_empty()).then_some(name)
}

pub fn wanted(settings: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(settings).ok()?;
    clean_name(value.get(SETTINGS_KEY)?.as_str()?)
}

fn profile_dir(dir: &Path, name: &str) -> PathBuf {
    dir.join(FOLDER).join(name)
}

fn store(dir: &Path, name: &str) -> std::io::Result<()> {
    let target = profile_dir(dir, name);
    fs::create_dir_all(&target)?;
    for file in FILES {
        let from = dir.join(file);
        if from.is_file() {
            fs::copy(&from, target.join(file))?;
        } else {
            let _ = fs::remove_file(target.join(file));
        }
    }
    Ok(())
}

fn load(dir: &Path, name: &str) -> std::io::Result<()> {
    let source = profile_dir(dir, name);
    for file in FILES {
        let from = source.join(file);
        let to = dir.join(file);
        if from.is_file() {
            fs::copy(&from, &to)?;
        } else if to.is_file() {
            fs::remove_file(&to)?;
        }
    }
    Ok(())
}

/// Returns the profile now active, or None when profiles are not in use.
pub fn apply(dir: &Path, settings: &str) -> Option<String> {
    let name = wanted(settings)?;
    let marker = dir.join(FOLDER).join(MARKER);
    let current = fs::read_to_string(&marker).ok().and_then(|t| clean_name(&t));
    if current.as_deref() == Some(name.as_str()) {
        return Some(name);
    }
    if let Some(previous) = current {
        if store(dir, &previous).is_err() {
            return None;
        }
        if load(dir, &name).is_err() {
            return None;
        }
    } else if store(dir, &name).is_err() {
        return None;
    }
    let _ = fs::create_dir_all(dir.join(FOLDER));
    fs::write(&marker, &name).ok()?;
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("m182_profile_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  Aincrad_1 "), Some("Aincrad_1".into()));
        assert_eq!(clean_name("../x/y"), Some("xy".into()));
        assert_eq!(clean_name("   "), None);
        assert_eq!(wanted(r#"{"profile":"a b"}"#), Some("ab".into()));
        assert_eq!(wanted(r#"{"remote_limit":200}"#), None);
    }

    #[test]
    fn no_profile_changes_nothing() {
        let dir = sandbox("none");
        fs::write(dir.join("m182_zones.json"), "A").unwrap();
        assert_eq!(apply(&dir, "{}"), None);
        assert_eq!(fs::read_to_string(dir.join("m182_zones.json")).unwrap(), "A");
        assert!(!dir.join(FOLDER).exists());
    }

    #[test]
    fn first_profile_adopts_current_files_and_switching_keeps_each_set() {
        let dir = sandbox("switch");
        fs::write(dir.join("m182_zones.json"), "aincrad-zones").unwrap();
        fs::write(dir.join("m182_spawners.json"), "aincrad-bosses").unwrap();
        assert_eq!(apply(&dir, r#"{"profile":"aincrad"}"#).as_deref(), Some("aincrad"));
        assert_eq!(fs::read_to_string(dir.join("m182_zones.json")).unwrap(), "aincrad-zones");

        assert_eq!(apply(&dir, r#"{"profile":"midnight"}"#).as_deref(), Some("midnight"));
        assert!(!dir.join("m182_zones.json").exists());
        assert!(!dir.join("m182_spawners.json").exists());
        fs::write(dir.join("m182_zones.json"), "midnight-zones").unwrap();

        assert_eq!(apply(&dir, r#"{"profile":"midnight"}"#).as_deref(), Some("midnight"));
        assert_eq!(fs::read_to_string(dir.join("m182_zones.json")).unwrap(), "midnight-zones");

        assert_eq!(apply(&dir, r#"{"profile":"aincrad"}"#).as_deref(), Some("aincrad"));
        assert_eq!(fs::read_to_string(dir.join("m182_zones.json")).unwrap(), "aincrad-zones");
        assert_eq!(fs::read_to_string(dir.join("m182_spawners.json")).unwrap(), "aincrad-bosses");

        assert_eq!(apply(&dir, r#"{"profile":"midnight"}"#).as_deref(), Some("midnight"));
        assert_eq!(fs::read_to_string(dir.join("m182_zones.json")).unwrap(), "midnight-zones");
        assert!(!dir.join("m182_spawners.json").exists());
    }
}

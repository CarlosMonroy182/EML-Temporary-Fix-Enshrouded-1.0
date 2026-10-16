use std::collections::HashMap;
use std::path::Path;

pub struct WikiEntry {
    pub name: String,
    pub category: String,
    pub file: String,
}

pub fn load(dir: &Path) -> HashMap<String, WikiEntry> {
    let mut out = HashMap::new();
    let Ok(text) = std::fs::read_to_string(dir.join("export").join("m182_wiki_index.csv")) else { return out };
    for line in text.lines().skip(1) {
        let parts: Vec<&str> = line.splitn(4, ',').collect();
        if parts.len() < 4 {
            continue;
        }
        out.insert(
            parts[0].trim().to_lowercase(),
            WikiEntry { name: parts[1].trim().to_string(), category: parts[2].trim().to_string(), file: parts[3].trim().to_string() },
        );
    }
    out
}

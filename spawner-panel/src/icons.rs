use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const USER_AGENT: &str = "M182-AdminPanel/1.0 (personal modding tool; low request volume)";
const WIKI_API: &str = "https://enshrouded.wiki.gg/api.php";
const INDEX_FILE: &str = "wiki_index.tsv";

pub fn norm(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

pub fn icons_dir(dir: &Path) -> PathBuf {
    dir.join("export").join("m182_icons")
}

pub fn wiki_file(dir: &Path, key: &str) -> PathBuf {
    icons_dir(dir).join("wiki").join(format!("{key}.png"))
}

fn curl_command() -> Command {
    let mut cmd = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.args(["-sS", "-L", "--fail", "--max-time", "45", "-A", USER_AGENT]);
    cmd.stdin(Stdio::null()).stderr(Stdio::null());
    cmd
}

pub fn fetch_bytes(url: &str) -> Option<Vec<u8>> {
    let out = curl_command().arg(url).stdout(Stdio::piped()).output().ok()?;
    if out.status.success() && !out.stdout.is_empty() {
        Some(out.stdout)
    } else {
        None
    }
}

pub fn fetch_bytes_retry(url: &str, tries: u32) -> Option<Vec<u8>> {
    for attempt in 0..tries {
        if let Some(b) = fetch_bytes(url) {
            return Some(b);
        }
        std::thread::sleep(Duration::from_millis(1500 * (attempt as u64 + 1)));
    }
    None
}

pub fn fetch_to_file(url: &str, path: &Path) -> bool {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Some(bytes) = fetch_bytes_retry(url, 3) else { return false };
    let tmp = path.with_extension("part");
    if std::fs::write(&tmp, &bytes).is_err() {
        return false;
    }
    std::fs::rename(&tmp, path).is_ok()
}

fn looks_like_image(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else { return false };
    bytes.len() > 100 && (bytes.starts_with(&[0x89, b'P', b'N', b'G']) || bytes.starts_with(&[0xFF, 0xD8]))
}

pub type WikiIndex = HashMap<String, String>;

pub fn load_index(dir: &Path) -> WikiIndex {
    let mut out = HashMap::new();
    let Ok(text) = std::fs::read_to_string(icons_dir(dir).join(INDEX_FILE)) else { return out };
    for line in text.lines() {
        if let Some((title, url)) = line.split_once('\t') {
            if !url.is_empty() {
                out.insert(norm(title), url.to_string());
            }
        }
    }
    out
}

pub fn parse_index_page(json: &serde_json::Value, titles: &mut HashMap<String, String>) {
    let Some(pages) = json.get("query").and_then(|q| q.get("pages")).and_then(|p| p.as_array()) else { return };
    for page in pages {
        let Some(title) = page.get("title").and_then(|t| t.as_str()) else { continue };
        let url = page.get("thumbnail").and_then(|t| t.get("source")).and_then(|s| s.as_str()).unwrap_or("");
        let entry = titles.entry(title.to_string()).or_default();
        if entry.is_empty() && !url.is_empty() {
            *entry = url.to_string();
        }
    }
}

fn build_index(dir: &Path, progress: &AtomicUsize) -> usize {
    let mut titles: HashMap<String, String> = HashMap::new();
    let mut cont: Vec<(String, String)> = Vec::new();
    for _ in 0..400 {
        let mut url = format!(
            "{WIKI_API}?action=query&generator=allpages&gaplimit=50&gapfilterredir=nonredirects&prop=pageimages&piprop=thumbnail&pithumbsize=128&pilimit=50&format=json&formatversion=2"
        );
        for (k, v) in &cont {
            url.push_str(&format!("&{k}={}", url_encode(v)));
        }
        let Some(bytes) = fetch_bytes_retry(&url, 3) else { break };
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else { break };
        parse_index_page(&json, &mut titles);
        progress.store(titles.len(), Ordering::Relaxed);
        match json.get("continue").and_then(|c| c.as_object()) {
            Some(map) => {
                cont = map.iter().map(|(k, v)| (k.clone(), v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string()))).collect();
            }
            None => break,
        }
        std::thread::sleep(Duration::from_millis(450));
    }
    let with_image = titles.values().filter(|u| !u.is_empty()).count();
    if with_image > 0 {
        let mut text = String::new();
        let mut rows: Vec<_> = titles.iter().collect();
        rows.sort();
        for (t, u) in rows {
            text.push_str(&format!("{}\t{}\n", t, u));
        }
        let base = icons_dir(dir);
        let _ = std::fs::create_dir_all(&base);
        let _ = std::fs::write(base.join(INDEX_FILE), text);
    }
    with_image
}

pub fn url_encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub struct Job {
    pub id: String,
    pub keys: Vec<String>,
}

#[derive(Default)]
pub struct Progress {
    pub index_pages: AtomicUsize,
    pub index_ready: AtomicUsize,
    pub queued: AtomicUsize,
    pub downloaded: AtomicUsize,
    pub missing: AtomicUsize,
}

pub struct Downloader {
    jobs: Sender<Job>,
    done: Receiver<String>,
    asked: HashSet<String>,
    pub progress: Arc<Progress>,
}

impl Downloader {
    pub fn start(dir: &Path) -> Self {
        let (jobs, job_rx) = channel::<Job>();
        let (done_tx, done) = channel::<String>();
        let progress = Arc::new(Progress::default());
        let shared = progress.clone();
        let dir = dir.to_path_buf();
        let _ = std::thread::Builder::new().name("m182-icons".into()).spawn(move || {
            let mut index = load_index(&dir);
            if index.len() < 1500 {
                let found = build_index(&dir, &shared.index_pages);
                if found > 0 {
                    index = load_index(&dir);
                }
            }
            shared.index_ready.store(index.len().max(1), Ordering::Relaxed);
            let index = Mutex::new(index);
            while let Ok(job) = job_rx.recv() {
                let urls: Vec<(String, String)> = {
                    let guard = index.lock().unwrap();
                    job.keys.iter().filter_map(|k| guard.get(k).map(|u| (k.clone(), u.clone()))).collect()
                };
                if urls.is_empty() {
                    shared.missing.fetch_add(1, Ordering::Relaxed);
                    shared.queued.fetch_sub(1, Ordering::Relaxed);
                    continue;
                }
                let mut ok = false;
                for (key, url) in urls {
                    let target = wiki_file(&dir, &key);
                    if looks_like_image(&target) {
                        ok = true;
                        break;
                    }
                    if fetch_to_file(&url, &target) && looks_like_image(&target) {
                        ok = true;
                        std::thread::sleep(Duration::from_millis(250));
                        break;
                    }
                    let _ = std::fs::remove_file(&target);
                }
                shared.queued.fetch_sub(1, Ordering::Relaxed);
                if ok {
                    shared.downloaded.fetch_add(1, Ordering::Relaxed);
                    let _ = done_tx.send(job.id);
                } else {
                    shared.missing.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        Self { jobs, done, asked: HashSet::new(), progress }
    }

    pub fn request(&mut self, id: &str, keys: Vec<String>) {
        if keys.is_empty() || !self.asked.insert(id.to_string()) {
            return;
        }
        self.progress.queued.fetch_add(1, Ordering::Relaxed);
        let _ = self.jobs.send(Job { id: id.to_string(), keys });
    }

    pub fn finished(&self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(id) = self.done.try_recv() {
            out.push(id);
        }
        out
    }

    pub fn status(&self) -> String {
        let p = &self.progress;
        if p.index_ready.load(Ordering::Relaxed) == 0 {
            return format!("Imágenes: leyendo el índice de la wiki ({} páginas)…", p.index_pages.load(Ordering::Relaxed));
        }
        let queued = p.queued.load(Ordering::Relaxed);
        if queued > 0 {
            format!("Imágenes: descargando ({queued} en cola, {} listas)", p.downloaded.load(Ordering::Relaxed))
        } else {
            format!("Imágenes de la wiki listas ({} descargadas esta vez)", p.downloaded.load(Ordering::Relaxed))
        }
    }
}

const ENEMY_RULES: &[(&[&str], &str)] = &[
    (&["skeleton", "necromancer"], "Hollow Mage"),
    (&["skeleton", "mage"], "Hollow Mage"),
    (&["skeleton", "hound"], "Hollow Hound"),
    (&["skeleton", "heavy"], "Hollow Reaper"),
    (&["skeleton", "elite"], "Hollow Soldier"),
    (&["skeleton"], "Hollow Skeleton"),
    (&["cyclops", "fog"], "Fell Cyclops"),
    (&["cyclops"], "Hollow Cyclops"),
    (&["drak00"], "Drak Skirmisher"),
    (&["drak01"], "Drak Warrior"),
    (&["drak02"], "Drak Skirmisher"),
    (&["drak03"], "Drak Chieftan"),
    (&["drak04"], "Drak Priest"),
    (&["drak05"], "Drak Chieftan"),
    (&["giantlizard"], "Drak Chieftan"),
    (&["scavenger", "hound"], "Scavenger Wolf"),
    (&["scavenger", "elite"], "Scavenger Pyreblade"),
    (&["scavenger", "ranger"], "Scavenger Scrapgun"),
    (&["scavenger", "crossbow"], "Scavenger Scrapgun"),
    (&["scavenger", "miner"], "Scavenger Miner"),
    (&["scavenger", "berserker"], "Scavenger Berserker"),
    (&["scavenger", "matron"], "Scavenger Matron"),
    (&["scavenger", "tyrant"], "Scavenger Tyrant"),
    (&["scavenger", "pyro"], "Scavenger Pyro"),
    (&["scavenger"], "Scavenger Scalper"),
    (&["vukah", "shaman"], "Vukah Shaman"),
    (&["vukah", "ranger"], "Vukah Ranger"),
    (&["vukah", "archer"], "Vukah Ranger"),
    (&["vukah", "berserker"], "Vukah Berserker"),
    (&["vukah", "elite"], "Vukah Brawler"),
    (&["vukah", "heavy"], "Vukah Brawler"),
    (&["vukah"], "Vukah Warrior"),
    (&["critterqueen"], "Fell Critter Queen"),
    (&["suicider"], "Exploding Fell Critter"),
    (&["bug", "critter"], "Fell Critter"),
    (&["banshee"], "Fell Banshee"),
    (&["fogcrab"], "Fell Mushroom Crab"),
    (&["sporehead"], "Fell Sporehead"),
    (&["jellyfloater"], "Shroud Gazer"),
    (&["predator"], "Shroud Stalker"),
    (&["monstrosity"], "Fell Monstrosity"),
    (&["blackknight"], "Fell Knight"),
    (&["fogger", "berserker"], "Fell Berserker"),
    (&["fogger", "heavy"], "Fell Thunderbrute"),
    (&["fogger", "ranger"], "Fell Ranger"),
    (&["fogger", "mage"], "Fell Sporehead"),
    (&["infestedboar"], "Fell Boar"),
    (&["fogger", "depleted"], "Fell Footsoldier"),
    (&["fogger", "soldier"], "Fell Footsoldier"),
    (&["fogger"], "Fell Footsoldier"),
    (&["fog", "dragon"], "Fell Dragon Youngling"),
    (&["plant", "spitter"], "Fell Vicious Vine"),
    (&["vulture"], "Fell Draconian Vulture"),
    (&["sabertooth"], "Sabertooth Tiger"),
    (&["crocodile"], "Snapjaw"),
    (&["spiderant"], "Spider"),
    (&["wildbeast", "mushroom"], "Mushroom Crab"),
    (&["wolf"], "Wolf"),
    (&["boar"], "Boar"),
    (&["scorpion"], "Scorpion"),
    (&["ancientconstruct", "heavy"], "Bronze Ancient S.P.D.R. Construct Trophy"),
    (&["ancientconstruct"], "Ancient S.P.D.R. Construct Trophy"),
    (&["vorgothorgan"], "Vorgoth's Eye"),
    (&["tentacle"], "Vorgoth's Eye"),
    (&["vorgoth"], "Vorgoth's Eye"),
    (&["wispwyvern"], "Fell Wispwyvern"),
    (&["sicklescythe"], "Fell Sicklescythe"),
    (&["striker"], "Fell Striker Head"),
    (&["thunderbrute"], "Fell Thunderbrute"),
    (&["titan"], "Bronze Hollow Titan Trophy"),
    (&["hydrak"], "Bronze Hydrak'Dal Trophy"),
];

const WILD: &[&str] = &[
    "Capybara", "Common Goat", "Dune Armadillo", "Fae Deer", "Flightless Redtail", "Frizzy Goat", "Hophare",
    "Round-Billed Sailer", "Yak", "Bat", "Crab", "Rat", "Snow Leopard", "Spider", "Water Reptile", "Whitewing Vulture",
    "Wisp", "Frost Wisp", "Pink Stinger", "Yellow Stinger", "Goat", "Deer", "Armadillo", "Bees",
];

pub fn enemy_keys(raw: &str) -> Vec<String> {
    let lower: String = raw.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
    let mut out: Vec<String> = Vec::new();
    for (needles, title) in ENEMY_RULES {
        if needles.iter().all(|n| lower.contains(n)) {
            out.push(norm(title));
            if !title.ends_with("Trophy") && !title.ends_with("Head") {
                out.push(norm(&format!("{title} Head")));
                out.push(norm(&format!("{title} Trophy")));
            }
            break;
        }
    }
    if out.is_empty() {
        for title in WILD {
            let key = norm(title);
            if lower.contains(&key) {
                out.push(key);
                break;
            }
        }
    }
    out
}

const NOISE: &[&str] = &[
    "material", "armor", "armour", "weapon", "food", "consumable", "tool", "prop", "decoration", "craft", "block", "blueprint", "vanity",
    "ring", "ammo", "ability", "build", "key", "raw", "processed", "dish", "loot", "drop", "hint", "dps", "tank", "healer", "ministory",
    "lore", "old", "norecipe", "crafting", "reward", "comfort", "z", "voxel", "terrain", "collection", "deprecated", "test", "debug",
    "lvl", "eternal", "tx", "npc", "item",
];

const GENERIC: &[&str] = &[
    "wand", "staff", "sword", "bow", "axe", "shield", "hammer", "dagger", "spear", "ranger", "hunter", "warrior", "mage", "rogue", "fire",
    "ice", "lightning", "stone", "wood", "metal", "bench", "table", "chair", "spider", "light", "dark", "big", "small", "large", "medium",
    "head", "body", "feet", "legs", "arms", "hand", "hands", "helmet", "chest", "boots", "gloves", "pants", "arrow", "spell", "fog",
    "square", "round", "simple", "basic", "wall", "floor", "door", "window", "roof", "stairs",
];

fn split_words(raw: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for part in raw.split(|c: char| c == '_' || c == '-' || c.is_whitespace()) {
        let chars: Vec<char> = part.chars().collect();
        let mut current = String::new();
        for (i, c) in chars.iter().enumerate() {
            let boundary = i > 0 && {
                let p = chars[i - 1];
                (c.is_uppercase() && (p.is_lowercase() || p.is_ascii_digit()))
                    || (c.is_uppercase() && p.is_uppercase() && chars.get(i + 1).map(|n| n.is_lowercase()).unwrap_or(false))
                    || (c.is_ascii_digit() != p.is_ascii_digit())
            };
            if boundary && !current.is_empty() {
                words.push(std::mem::take(&mut current).to_lowercase());
            }
            current.push(*c);
        }
        if !current.is_empty() {
            words.push(current.to_lowercase());
        }
    }
    words
}

fn singular(word: &str) -> String {
    if let Some(stem) = word.strip_suffix("ies") {
        return format!("{stem}y");
    }
    if word.len() > 3 && word.ends_with('s') && !word.ends_with("ss") {
        return word[..word.len() - 1].to_string();
    }
    word.to_string()
}

pub fn derived_keys(debug: &str, allow_single: bool) -> Vec<String> {
    let words: Vec<String> = split_words(debug)
        .into_iter()
        .filter(|w| !NOISE.contains(&w.as_str()))
        .filter(|w| w.chars().count() > 1)
        .filter(|w| !w.chars().all(|c| c.is_ascii_digit()))
        .filter(|w| !(w.starts_with("lvl")))
        .map(|w| singular(&w))
        .collect();
    let mut out: Vec<String> = Vec::new();
    let n = words.len();
    for len in (1..=n.min(5)).rev() {
        for start in 0..=(n - len) {
            let seg = &words[start..start + len];
            let mut variants = vec![seg.join("")];
            if len == 2 {
                variants.push(format!("{}{}", seg[1], seg[0]));
            }
            for key in variants {
                let ok = if len == 1 {
                    allow_single && key.len() >= 4 && !GENERIC.contains(&key.as_str())
                } else {
                    key.len() >= 6
                };
                if ok && !out.contains(&key) {
                    out.push(key);
                }
            }
        }
    }
    out
}

pub fn item_keys(name: &str, debug: &str, category: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if name != debug && !name.is_empty() {
        out.extend(name_keys(name));
    }
    let single = matches!(category, "Materials" | "Consumables" | "Collectibles" | "AnimalFood" | "Currency" | "WeaponGem" | "Tools" | "Instrument" | "FishingRod");
    for k in derived_keys(debug, single) {
        if !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

pub fn name_keys(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let k = norm(s);
        if !k.is_empty() && !out.contains(&k) {
            out.push(k);
        }
    };
    push(name);
    if let Some((head, _)) = name.split_once('(') {
        push(head);
    }
    if let Some((head, _)) = name.split_once(" - ") {
        push(head);
    }
    let replaced = name.replace("’", "'");
    push(&replaced);
    let mut words: Vec<&str> = name.split_whitespace().collect();
    if words.len() > 2 {
        if let Some(last) = words.last() {
            if matches!(*last, "I" | "II" | "III" | "IV" | "V" | "VI" | "VII" | "VIII" | "IX" | "X") {
                words.pop();
                push(&words.join(" "));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_titles() {
        assert_eq!(norm("Fell Critter Queen"), "fellcritterqueen");
        assert_eq!(norm("Artificer's Helmet"), "artificershelmet");
    }

    #[test]
    fn enemy_rules_pick_wiki_names() {
        assert_eq!(enemy_keys("Enemy_Skeleton_Mage01")[0], norm("Hollow Mage"));
        assert_eq!(enemy_keys("Enemy_Skeleton_Melee01")[0], norm("Hollow Skeleton"));
        assert_eq!(enemy_keys("Enemy_Vukah_Shaman_Lightning")[0], norm("Vukah Shaman"));
        assert_eq!(enemy_keys("Enemy_Wildbeast_Wolf_Heavy")[0], norm("Wolf"));
        assert_eq!(enemy_keys("Enemy_Wildlife_Capybara_01"), vec![norm("Capybara")]);
        assert_eq!(enemy_keys("Enemy_Fog_CritterQueen_T2_BossHealthBar")[0], norm("Fell Critter Queen"));
        assert_eq!(enemy_keys("Enemy_AncientConstruct_Modular01_Grunt"), vec![norm("Ancient S.P.D.R. Construct Trophy")]);
        assert!(enemy_keys("Totally_Unknown_Thing").is_empty());
    }

    #[test]
    fn derived_names_from_debug_names() {
        let keys = derived_keys("Material_T5_Night_PinkGentian", true);
        assert_eq!(keys[0], "nightpinkgentian");
        assert!(keys.contains(&"pinkgentian".to_string()));
        assert!(!derived_keys("Weapon_T6_1H_Wand_02_Lightning", false).iter().any(|k| k == "wand"));
        assert!(derived_keys("Food_T5_raw_vegetables_Pumpkin", true).contains(&"pumpkin".to_string()));
        assert!(derived_keys("Ammo_T2_Arrow_Copper_lvl12", false).contains(&"copperarrow".to_string()));
        assert_eq!(item_keys("Iron Sword", "Weapon_T3_Iron", "Weapons")[0], norm("Iron Sword"));
        assert_eq!(item_keys("Pumpkin", "Pumpkin", "Consumables")[0], "pumpkin");
    }

    #[test]
    fn name_variants() {
        assert_eq!(name_keys("Giant Bone Arrow II"), vec![norm("Giant Bone Arrow II"), norm("Giant Bone Arrow")]);
        assert_eq!(name_keys("Iron Sword (Old)")[1], norm("Iron Sword"));
    }

    #[test]
    fn index_page_merge() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"query":{"pages":[{"title":"Wolf","thumbnail":{"source":"https://x/w.png"}},{"title":"Iron"}]}}"#,
        )
        .unwrap();
        let mut titles = HashMap::new();
        parse_index_page(&json, &mut titles);
        assert_eq!(titles.get("Wolf").unwrap(), "https://x/w.png");
        assert_eq!(titles.get("Iron").unwrap(), "");
        let json2: serde_json::Value = serde_json::from_str(r#"{"query":{"pages":[{"title":"Iron","thumbnail":{"source":"https://x/i.png"}}]}}"#).unwrap();
        parse_index_page(&json2, &mut titles);
        assert_eq!(titles.get("Iron").unwrap(), "https://x/i.png");
    }

    #[test]
    fn downloads_from_a_local_server_when_asked() {
        let Ok(base) = std::env::var("M182_TEST_HTTP") else { return };
        let dir = std::env::temp_dir().join(format!("m182_dl_{}", std::process::id()));
        let target = dir.join("a").join("x.png");
        assert!(fetch_to_file(&format!("{base}/x.png"), &target));
        assert!(looks_like_image(&target));
        assert!(fetch_bytes(&format!("{base}/missing.png")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn url_encoding() {
        assert_eq!(url_encode("a b|c"), "a%20b%7Cc");
    }
}

#![cfg_attr(windows, windows_subsystem = "windows")]

mod emblems;
mod theme;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, RichText, Rounding, Sense, Stroke, TextureHandle, Vec2};
use serde_json::{json, Map, Value};

use theme::*;

const FALLBACK_GAME_DIR: &str = r"D:\SteamLibrary\steamapps\common\Enshrouded";
const CONFIG_FILE: &str = "m182_native.json";
const REQUEST_FILE: &str = "m182_spawn.request";
const REQUEST_TMP: &str = "m182_spawn.request.tmp";
const PLAYER_FILE: &str = "m182_player.json";
const PLACES_FILE: &str = "m182_places.json";
const DONATE_URL: &str = "https://www.paypal.me/MoradoGames";

struct Entry {
    guid: String,
    label: String,
    group: String,
    boss: bool,
    search: String,
    kind: emblems::Kind,
    internal: bool,
}

#[derive(Default)]
struct Catalog {
    entries: Vec<Entry>,
    error: Option<String>,
    selected: Option<usize>,
    search: String,
    group: String,
    only_bosses: bool,
    show_internal: bool,
}

impl Catalog {
    fn visible(&self) -> Vec<usize> {
        let query = self.search.trim().to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                (self.group.is_empty() || e.group == self.group)
                    && (!self.only_bosses || e.boss)
                    && (self.show_internal || !e.internal)
                    && (query.is_empty() || e.search.contains(&query))
            })
            .map(|(i, _)| i)
            .collect()
    }
}

#[derive(Clone)]
struct Place {
    name: String,
    x: f64,
    y: f64,
    z: f64,
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Creatures,
    Objects,
    Pvp,
    SafeZones,
    Teams,
    Places,
}

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Creatures,
    Objects,
}

#[derive(PartialEq, Clone, Copy)]
enum Target {
    Forward,
    Around,
    Place,
    Coords,
}

enum Status {
    Idle,
    Waiting(Instant),
    Delivered,
    NoAnswer,
    Error(String),
}

struct App {
    dir: PathBuf,
    tab: Tab,
    creatures: Catalog,
    objects: Catalog,
    level: u32,
    count: u32,
    distance: f32,
    custom_health: bool,
    health: u32,
    damage: f32,
    pvp: bool,
    pvp_friends: bool,
    target: Target,
    place_index: usize,
    coords: [f64; 3],
    places: Vec<Place>,
    new_place: String,
    player: Option<[f64; 3]>,
    player_checked: Instant,
    icons: HashMap<String, Option<TextureHandle>>,
    status: Status,
    notice: String,
}

fn faction_label(raw: &str) -> &str {
    match raw {
        "Ancients" => "Ancestrales",
        "Cyclops" => "Cíclope",
        "Drak" => "Drak",
        "Fogger" => "Niebla",
        "HitAll" => "Hostiles a todos",
        "HitOthers" => "Hostiles a otros",
        "Scavenger" => "Carroñeros",
        "Skeleton" => "Esqueletos",
        "Vukah" => "Vukah",
        "WildBeast" => "Bestias",
        "Wildlife" => "Fauna",
        other => other,
    }
}

fn group_color(raw: &str) -> Color32 {
    match raw {
        "Ancients" => Color32::from_rgb(132, 186, 204),
        "Cyclops" => Color32::from_rgb(214, 144, 98),
        "Drak" => Color32::from_rgb(124, 192, 118),
        "Fogger" => Color32::from_rgb(174, 140, 214),
        "HitAll" => Color32::from_rgb(222, 108, 100),
        "HitOthers" => Color32::from_rgb(214, 124, 160),
        "Scavenger" => Color32::from_rgb(210, 180, 104),
        "Skeleton" => Color32::from_rgb(206, 202, 188),
        "Vukah" => Color32::from_rgb(104, 188, 184),
        "WildBeast" => Color32::from_rgb(192, 154, 106),
        "Contenedores" => Color32::from_rgb(214, 170, 78),
        "Recogibles" => Color32::from_rgb(132, 200, 150),
        "Recursos" => Color32::from_rgb(150, 190, 110),
        "Construcción" => Color32::from_rgb(196, 160, 120),
        "Luces" => Color32::from_rgb(240, 190, 90),
        "Efectos" => Color32::from_rgb(170, 140, 214),
        "Personajes" => Color32::from_rgb(132, 186, 204),
        "Lugares" => Color32::from_rgb(214, 144, 98),
        "Otros" => Color32::from_rgb(170, 160, 140),
        _ => Color32::from_rgb(158, 190, 116),
    }
}

fn pretty(name: &str) -> String {
    let mut n = name;
    for prefix in ["Base_Enemy_", "Base_", "Enemy_"] {
        if let Some(rest) = n.strip_prefix(prefix) {
            n = rest;
            break;
        }
    }
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in n.chars() {
        if c == '_' {
            out.push(' ');
        } else {
            if let Some(p) = prev {
                if c.is_uppercase() && (p.is_lowercase() || p.is_ascii_digit()) {
                    out.push(' ');
                }
            }
            out.push(c);
        }
        prev = Some(c);
    }
    out
}

fn is_internal(raw: &str) -> bool {
    let n = raw.to_lowercase();
    n.starts_with("base_")
        || ["collider", "projectilespawner", "togglestate", "hookpt"].iter().any(|w| n.contains(w))
        || ["sleep", "spawntube", "spawn_tube", "parent", "group", "channel", "organ", "quickattack", "quick_attack", "_add", "dummy", "debug", "trigger", "placeholder", "test"]
            .iter()
            .any(|w| n.contains(w))
}

fn object_group(name: &str, components: &str) -> &'static str {
    let n = name.to_lowercase();
    let c = components.to_lowercase();
    let any = |text: &str, words: &[&str]| words.iter().any(|w| text.contains(w));
    if any(&n, &["chest", "container", "storage", "barrel", "loot"]) || any(&c, &["container", "inventory", "lootcontainer"]) {
        "Contenedores"
    } else if any(&n, &["pickup", "item_", "drop"]) || any(&c, &["pickup", "itemdrop", "droppeditem"]) {
        "Recogibles"
    } else if any(&n, &["tree", "ore", "rock", "stone", "plant", "herb", "mushroom", "bush", "flower", "resource", "harvest", "node"]) || any(&c, &["harvest", "resourcenode", "gather"]) {
        "Recursos"
    } else if any(&n, &["torch", "lamp", "light", "lantern", "campfire", "brazier", "fire"]) {
        "Luces"
    } else if any(&n, &["wall", "floor", "roof", "stair", "door", "window", "bed", "table", "chair", "furniture", "build", "fence", "bridge"]) {
        "Construcción"
    } else if any(&n, &["vfx", "particle", "effect", "fx_", "decal", "sound"]) || any(&c, &["particle", "vfx"]) {
        "Efectos"
    } else if any(&n, &["npc", "vendor", "trader", "villager", "merchant", "player"]) {
        "Personajes"
    } else if any(&n, &["altar", "shrine", "portal", "teleport", "waypoint", "flame", "ruin"]) {
        "Lugares"
    } else {
        "Otros"
    }
}

fn find_game_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("M182_GAME_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("enshrouded.exe").exists() {
                return dir.to_path_buf();
            }
        }
    }
    PathBuf::from(FALLBACK_GAME_DIR)
}

fn load_enemies(dir: &Path) -> Result<Vec<Entry>, String> {
    let path = dir.join("export").join("m182_enemies.csv");
    let text = std::fs::read_to_string(&path).map_err(|_| {
        format!(
            "No encuentro la lista de enemigos ({}). Abre el juego una vez con el mod m182_spawn_variants para generarla.",
            path.display()
        )
    })?;
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 5 || !parts[4].trim().is_empty() {
            continue;
        }
        let label = pretty(parts[0]);
        let group = parts[2].trim().to_string();
        out.push(Entry {
            guid: parts[1].trim().to_string(),
            search: format!("{} {}", label, faction_label(&group)).to_lowercase(),
            kind: emblems::kind_of(parts[0], &group),
            internal: is_internal(parts[0]),
            label,
            group,
            boss: parts[3].trim() == "true",
        });
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    if out.is_empty() {
        return Err("La lista de enemigos está vacía.".to_string());
    }
    Ok(out)
}

fn load_objects(dir: &Path) -> Result<Vec<Entry>, String> {
    let path = dir.join("export").join("m182_objects.csv");
    let text = std::fs::read_to_string(&path).map_err(|_| {
        "La lista de objetos aún no existe. Cierra el juego, ábrelo una vez con el mod m182_spawn_variants actualizado y vuelve a abrir este panel.".to_string()
    })?;
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let mut parts = line.splitn(3, ',');
        let (Some(name), Some(guid)) = (parts.next(), parts.next()) else { continue };
        let components = parts.next().unwrap_or("");
        let group = object_group(name, components).to_string();
        let label = pretty(name);
        out.push(Entry {
            guid: guid.trim().to_string(),
            search: format!("{} {}", label, group).to_lowercase(),
            kind: emblems::kind_of_object(name, &group),
            internal: is_internal(name) || !components.contains("Transform"),
            label,
            group,
            boss: false,
        });
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    if out.is_empty() {
        return Err("La lista de objetos está vacía.".to_string());
    }
    Ok(out)
}

fn read_config(dir: &Path) -> Map<String, Value> {
    std::fs::read_to_string(dir.join(CONFIG_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn write_atomic(dir: &Path, name: &str, tmp: &str, text: &str) -> Result<(), String> {
    let tmp_path = dir.join(tmp);
    std::fs::write(&tmp_path, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp_path, dir.join(name)).map_err(|e| e.to_string())
}

fn read_player(dir: &Path) -> Option<[f64; 3]> {
    let value: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(PLAYER_FILE)).ok()?).ok()?;
    Some([value.get("x")?.as_f64()?, value.get("y")?.as_f64()?, value.get("z")?.as_f64()?])
}

fn load_places(dir: &Path) -> Vec<Place> {
    let Ok(text) = std::fs::read_to_string(dir.join(PLACES_FILE)) else { return Vec::new() };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&text) else { return Vec::new() };
    items
        .iter()
        .filter_map(|v| {
            Some(Place {
                name: v.get("name")?.as_str()?.to_string(),
                x: v.get("x")?.as_f64()?,
                y: v.get("y")?.as_f64()?,
                z: v.get("z")?.as_f64()?,
            })
        })
        .collect()
}

fn load_icon(ctx: &egui::Context, dir: &Path, guid: &str) -> Option<TextureHandle> {
    let path = dir.join("export").join("m182_icons").join(format!("{guid}.png"));
    let bytes = std::fs::read(path).ok()?;
    let image = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Some(ctx.load_texture(format!("icon-{guid}"), color, egui::TextureOptions::LINEAR))
}

fn group_label(mode: Mode, group: &str) -> String {
    match mode {
        Mode::Creatures => faction_label(group).to_string(),
        Mode::Objects => group.to_string(),
    }
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        let dir = find_game_dir();
        let cfg = read_config(&dir);
        let creatures = match load_enemies(&dir) {
            Ok(entries) => Catalog { entries, ..Default::default() },
            Err(e) => Catalog { error: Some(e), ..Default::default() },
        };
        let objects = match load_objects(&dir) {
            Ok(entries) => Catalog { entries, ..Default::default() },
            Err(e) => Catalog { error: Some(e), ..Default::default() },
        };
        let get_u = |key: &str, default: u32| cfg.get(key).and_then(|v| v.as_u64()).map(|v| v as u32).unwrap_or(default);
        let get_f = |key: &str, default: f32| cfg.get(key).and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(default);
        let get_b = |key: &str| cfg.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
        let health = get_u("spawn_health", 0);
        let places = load_places(&dir);
        #[cfg(feature = "linux-preview")]
        let preview_tab = match std::env::var("M182_PREVIEW_TAB").as_deref() {
            Ok("places") => Tab::Places,
            Ok("pvp") => Tab::Pvp,
            Ok("teams") => Tab::Teams,
            Ok("objects") => Tab::Objects,
            _ => Tab::Creatures,
        };
        #[cfg(not(feature = "linux-preview"))]
        let preview_tab = Tab::Creatures;
        #[allow(unused_mut)]
        let mut app = Self {
            tab: preview_tab,
            creatures,
            objects,
            level: get_u("spawn_level", 20).clamp(1, 100),
            count: get_u("spawn_count", 1).clamp(1, 20),
            distance: get_f("spawn_distance", 5.0).clamp(1.0, 60.0),
            custom_health: health > 0,
            health: if health > 0 { health } else { 1000 },
            damage: get_f("spawn_damage", 1.0).clamp(0.0, 20.0),
            pvp: get_b("pvp"),
            pvp_friends: get_b("pvp_friend_checks"),
            target: Target::Forward,
            place_index: 0,
            coords: [0.0; 3],
            player: read_player(&dir),
            player_checked: Instant::now(),
            places,
            new_place: String::new(),
            icons: HashMap::new(),
            status: Status::Idle,
            notice: String::new(),
            dir,
        };
        #[cfg(feature = "linux-preview")]
        if std::env::var("M182_PREVIEW_SELECT").is_ok() {
            app.creatures.selected = Some(120.min(app.creatures.entries.len().saturating_sub(1)));
            app.objects.selected = Some(30.min(app.objects.entries.len().saturating_sub(1)));
        }
        app.cleanup_stale_request();
        app
    }

    fn cleanup_stale_request(&self) {
        let _ = std::fs::remove_file(self.dir.join(REQUEST_FILE));
    }

    fn effective_health(&self) -> u32 {
        if self.custom_health {
            self.health
        } else {
            0
        }
    }

    fn catalog(&self, mode: Mode) -> &Catalog {
        match mode {
            Mode::Creatures => &self.creatures,
            Mode::Objects => &self.objects,
        }
    }

    fn set_config(&self, entries: &[(&str, Value)]) -> Result<(), String> {
        let mut cfg = read_config(&self.dir);
        for (key, value) in entries {
            cfg.insert((*key).to_string(), value.clone());
        }
        cfg.entry("enabled").or_insert(json!(true));
        let text = serde_json::to_string_pretty(&Value::Object(cfg)).map_err(|e| e.to_string())?;
        write_atomic(&self.dir, CONFIG_FILE, "m182_native.json.tmp", &text)
    }

    fn save_places(&self) {
        let items: Vec<Value> = self.places.iter().map(|p| json!({"name": p.name, "x": p.x, "y": p.y, "z": p.z})).collect();
        if let Ok(text) = serde_json::to_string_pretty(&Value::Array(items)) {
            let _ = write_atomic(&self.dir, PLACES_FILE, "m182_places.json.tmp", &text);
        }
    }

    fn target_point(&self) -> Option<[f64; 3]> {
        match self.target {
            Target::Forward | Target::Around => None,
            Target::Place => self.places.get(self.place_index).map(|p| [p.x, p.y, p.z]),
            Target::Coords => Some(self.coords),
        }
    }

    fn spawn(&mut self, mode: Mode) {
        let Some(index) = self.catalog(mode).selected else { return };
        let guid = self.catalog(mode).entries[index].guid.clone();
        let at = self.target_point();
        if matches!(self.target, Target::Place) && at.is_none() {
            self.status = Status::Error("Elige un lugar guardado.".to_string());
            return;
        }
        let saved = match mode {
            Mode::Creatures => self.set_config(&[
                ("spawn_template", json!(guid)),
                ("spawn_level", json!(self.level)),
                ("spawn_count", json!(self.count)),
                ("spawn_distance", json!(self.distance)),
                ("spawn_health", json!(self.effective_health())),
                ("spawn_damage", json!(self.damage)),
                ("spawner", json!(true)),
            ]),
            Mode::Objects => Ok(()),
        };
        let result = saved.and_then(|_| {
            let mut request = json!({
                "template": guid,
                "count": self.count,
                "distance": self.distance,
            });
            if mode == Mode::Creatures {
                request["level"] = json!(self.level);
                request["health"] = json!(self.effective_health());
                request["damage"] = json!(self.damage);
            } else {
                request["object"] = json!(true);
            }
            if self.target == Target::Forward {
                request["forward"] = json!(true);
            }
            if let Some(p) = at {
                request["at"] = json!(p);
            }
            write_atomic(&self.dir, REQUEST_FILE, REQUEST_TMP, &request.to_string())
        });
        self.status = match result {
            Ok(()) => Status::Waiting(Instant::now()),
            Err(e) => Status::Error(e),
        };
    }

    fn tick(&mut self, ctx: &egui::Context) {
        if let Status::Waiting(since) = self.status {
            if !self.dir.join(REQUEST_FILE).exists() {
                self.status = Status::Delivered;
            } else if since.elapsed() > Duration::from_secs(6) {
                let _ = std::fs::remove_file(self.dir.join(REQUEST_FILE));
                self.status = Status::NoAnswer;
            }
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        if self.player_checked.elapsed() >= Duration::from_millis(700) {
            self.player_checked = Instant::now();
            self.player = read_player(&self.dir);
        }
        ctx.request_repaint_after(Duration::from_millis(700));
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("M182  ADMIN  PANEL").font(title_font(26.0)).color(GOLD));
                ui.label(RichText::new("Herramientas de administración para Enshrouded").color(MUTED));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !DONATE_URL.is_empty() && button(ui, "Apoya el proyecto", Vec2::new(190.0, 34.0), true, false) {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(DONATE_URL));
                }
            });
        });
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        for (tab, label, soon) in [
            (Tab::Creatures, "Criaturas", false),
            (Tab::Objects, "Objetos", false),
            (Tab::Places, "Lugares", false),
            (Tab::Pvp, "PvP", false),
            (Tab::SafeZones, "Zonas seguras", true),
            (Tab::Teams, "Equipos", true),
        ] {
            let selected = self.tab == tab;
            let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::click());
            let painter = ui.painter();
            let fill = if selected {
                CARD_SELECTED
            } else if response.hovered() {
                CARD_HOVER
            } else {
                CARD
            };
            painter.rect_filled(rect, Rounding::same(4.0), fill);
            painter.rect_stroke(rect, Rounding::same(4.0), Stroke::new(if selected { 1.8_f32 } else { 1.0_f32 }, if selected { GOLD } else { BRONZE }));
            if selected {
                diamond(painter, rect.left_center() + Vec2::new(12.0, 0.0), 4.0, GOLD);
            }
            let shift = if soon { -7.0 } else { 0.0 };
            painter.text(
                rect.left_center() + Vec2::new(28.0, shift),
                Align2::LEFT_CENTER,
                label,
                title_font(15.0),
                if soon { MUTED } else { PARCHMENT },
            );
            if soon {
                painter.text(rect.left_center() + Vec2::new(28.0, 11.0), Align2::LEFT_CENTER, "próximamente", egui::FontId::proportional(12.5), GOLD_DIM);
            }
            if response.clicked() {
                self.tab = tab;
            }
        }
    }

    fn catalog_view(&mut self, ui: &mut egui::Ui, mode: Mode) {
        egui::SidePanel::right(if mode == Mode::Creatures { "creature_controls" } else { "object_controls" })
            .exact_width(320.0)
            .frame(egui::Frame::none().fill(LEATHER).inner_margin(egui::Margin::same(16.0)).stroke(Stroke::new(1.0_f32, BRONZE)))
            .show_inside(ui, |ui| egui::ScrollArea::vertical().show(ui, |ui| self.controls(ui, mode)));

        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin { left: 20.0, right: 16.0, top: 16.0, bottom: 8.0 })).show_inside(ui, |ui| {
            let App { creatures, objects, icons, dir, .. } = self;
            let cat = if mode == Mode::Creatures { creatures } else { objects };
            if let Some(error) = &cat.error {
                ui.label(RichText::new(error).color(BAD));
                return;
            }
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut cat.search).hint_text("Buscar…").desired_width(240.0));
                let mut groups: Vec<String> = cat.entries.iter().map(|e| e.group.clone()).collect();
                groups.sort_unstable();
                groups.dedup();
                let all = if mode == Mode::Creatures { "Todas las facciones" } else { "Todas las categorías" };
                let current = if cat.group.is_empty() { all.to_string() } else { group_label(mode, &cat.group) };
                egui::ComboBox::from_id_salt(("group", mode == Mode::Creatures)).selected_text(current).show_ui(ui, |ui| {
                    ui.selectable_value(&mut cat.group, String::new(), all);
                    for g in groups {
                        let text = group_label(mode, &g);
                        ui.selectable_value(&mut cat.group, g, text);
                    }
                });
                if mode == Mode::Creatures {
                    ui.checkbox(&mut cat.only_bosses, "Solo jefes");
                }
                ui.checkbox(&mut cat.show_internal, "Mostrar entradas internas");
            });
            let visible = cat.visible();
            ui.label(RichText::new(format!("{} {}", visible.len(), if mode == Mode::Creatures { "criaturas" } else { "objetos" })).color(MUTED));
            Self::grid(ui, cat, icons, dir, &visible, mode);
        });
    }

    fn grid(ui: &mut egui::Ui, cat: &mut Catalog, icons: &mut HashMap<String, Option<TextureHandle>>, dir: &Path, visible: &[usize], mode: Mode) {
        let spacing = 10.0;
        let min_card = 200.0;
        let card_h = 70.0;
        let width = ui.available_width() - 14.0;
        let cols = (((width + spacing) / (min_card + spacing)).floor() as usize).max(1);
        let card_w = (width - spacing * (cols as f32 - 1.0)) / cols as f32 - 2.0;
        let rows = visible.len().div_ceil(cols);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, card_h, rows, |ui, range| {
            ui.spacing_mut().item_spacing = Vec2::new(spacing, spacing);
            for row in range {
                ui.horizontal(|ui| {
                    for &i in visible.iter().skip(row * cols).take(cols) {
                        let selected = cat.selected == Some(i);
                        if Self::card(ui, icons, dir, &cat.entries[i], selected, card_w, mode) {
                            cat.selected = Some(i);
                        }
                    }
                });
            }
        });
    }

    fn card(ui: &mut egui::Ui, icons: &mut HashMap<String, Option<TextureHandle>>, dir: &Path, e: &Entry, selected: bool, width: f32, mode: Mode) -> bool {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 70.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return response.clicked();
        }
        if !icons.contains_key(&e.guid) {
            let loaded = load_icon(ui.ctx(), dir, &e.guid);
            icons.insert(e.guid.clone(), loaded);
        }
        let texture = icons.get(&e.guid).and_then(|t| t.clone());
        let fill = if selected {
            CARD_SELECTED
        } else if response.hovered() {
            CARD_HOVER
        } else {
            CARD
        };
        let painter = ui.painter();
        painter.rect_filled(rect, Rounding::same(5.0), fill);
        painter.rect_stroke(rect, Rounding::same(5.0), Stroke::new(if selected { 2.0_f32 } else { 1.0_f32 }, if selected { GOLD } else { BRONZE }));
        let icon = egui::Rect::from_min_size(rect.min + Vec2::new(8.0, 8.0), Vec2::splat(54.0));
        painter.rect_filled(icon, Rounding::same(4.0), Color32::from_rgb(20, 15, 11));
        painter.rect_stroke(icon, Rounding::same(4.0), Stroke::new(1.0_f32, GOLD_DIM));
        match texture {
            Some(texture) => {
                painter.image(texture.id(), icon.shrink(2.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => emblems::draw(painter, icon.shrink(4.0), e.kind, group_color(&e.group), e.boss),
        }
        let text_left = icon.right() + 10.0;
        let mut job = egui::text::LayoutJob::simple(e.label.clone(), egui::FontId::proportional(15.0), PARCHMENT, rect.right() - text_left - 8.0);
        job.wrap.max_rows = 2;
        job.wrap.overflow_character = Some('…');
        let galley = painter.layout_job(job);
        painter.galley(egui::pos2(text_left, rect.top() + 7.0), galley, PARCHMENT);
        painter.text(
            egui::pos2(text_left, rect.bottom() - 8.0),
            Align2::LEFT_BOTTOM,
            group_label(mode, &e.group),
            title_font(10.5),
            group_color(&e.group),
        );
        response.clicked()
    }

    fn controls(&mut self, ui: &mut egui::Ui, mode: Mode) {
        heading(ui, if mode == Mode::Creatures { "Invocación" } else { "Crear objeto" }, 19.0);
        divider(ui);
        let selected = self.catalog(mode).selected;
        match selected {
            Some(i) => {
                let e = &self.catalog(mode).entries[i];
                let icon = ui.allocate_exact_size(Vec2::new(ui.available_width(), 96.0), Sense::hover()).0;
                let painter = ui.painter();
                painter.rect_filled(icon, Rounding::same(4.0), Color32::from_rgb(20, 15, 11));
                ornate_frame(painter, icon);
                match self.icons.get(&e.guid).and_then(|t| t.clone()) {
                    Some(texture) => {
                        let side = icon.height() - 12.0;
                        let target = egui::Rect::from_center_size(icon.center(), Vec2::splat(side));
                        painter.image(texture.id(), target, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        let target = egui::Rect::from_center_size(icon.center(), Vec2::splat(icon.height() - 22.0));
                        emblems::draw(painter, target, e.kind, group_color(&e.group), e.boss);
                    }
                }
                ui.label(RichText::new(&e.label).font(title_font(17.0)).color(PARCHMENT));
                ui.label(RichText::new(group_label(mode, &e.group)).color(group_color(&e.group)));
            }
            None => {
                ui.label(RichText::new(if mode == Mode::Creatures { "Elige una criatura de la lista" } else { "Elige un objeto de la lista" }).color(MUTED));
            }
        }
        divider(ui);

        if mode == Mode::Creatures {
            label_title(ui, "NIVEL");
            ui.add(egui::Slider::new(&mut self.level, 1..=100));
        }
        label_title(ui, "CANTIDAD");
        ui.add(egui::Slider::new(&mut self.count, 1..=20));
        if mode == Mode::Creatures {
            label_title(ui, "VIDA");
            ui.checkbox(&mut self.custom_health, "Vida personalizada");
            ui.add_enabled(self.custom_health, egui::Slider::new(&mut self.health, 10..=1_000_000).logarithmic(true));
            label_title(ui, "DAÑO  (1.00 = normal)");
            ui.add(egui::Slider::new(&mut self.damage, 0.0..=20.0).fixed_decimals(2));
        }
        divider(ui);

        label_title(ui, "DÓNDE APARECE");
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.target, Target::Forward, "Delante de mí");
            ui.selectable_value(&mut self.target, Target::Around, "Alrededor");
            ui.selectable_value(&mut self.target, Target::Place, "Lugar");
            ui.selectable_value(&mut self.target, Target::Coords, "Punto");
        });
        match self.target {
            Target::Forward => {
                ui.label("Distancia hacia donde mira tu personaje (m)");
                ui.add(egui::Slider::new(&mut self.distance, 1.0..=60.0).fixed_decimals(0));
            }
            Target::Around => {
                ui.label("Distancia de ti (metros)");
                ui.add(egui::Slider::new(&mut self.distance, 1.0..=60.0).fixed_decimals(0));
            }
            Target::Place => {
                if self.places.is_empty() {
                    ui.label(RichText::new("Aún no tienes lugares. Guárdalos en la pestaña Lugares.").color(MUTED));
                } else {
                    self.place_index = self.place_index.min(self.places.len() - 1);
                    let current = self.places[self.place_index].name.clone();
                    egui::ComboBox::from_id_salt("place").selected_text(current).show_ui(ui, |ui| {
                        for (i, p) in self.places.iter().enumerate() {
                            ui.selectable_value(&mut self.place_index, i, &p.name);
                        }
                    });
                    if self.count > 1 {
                        ui.label("Radio alrededor del lugar");
                        ui.add(egui::Slider::new(&mut self.distance, 1.0..=60.0).fixed_decimals(0));
                    }
                }
            }
            Target::Coords => {
                for (label, value) in ["X", "Y (altura)", "Z"].iter().zip(self.coords.iter_mut()) {
                    ui.horizontal(|ui| {
                        ui.label(*label);
                        ui.add(egui::DragValue::new(value).speed(1.0).fixed_decimals(1));
                    });
                }
                if let Some(p) = self.player {
                    if button(ui, "Usar mi posición", Vec2::new(ui.available_width(), 30.0), true, false) {
                        self.coords = p;
                    }
                }
                if self.count > 1 {
                    ui.label("Radio alrededor del punto");
                    ui.add(egui::Slider::new(&mut self.distance, 1.0..=60.0).fixed_decimals(0));
                }
            }
        }
        divider(ui);

        let enabled = selected.is_some();
        if button(ui, if mode == Mode::Creatures { "INVOCAR" } else { "CREAR" }, Vec2::new(ui.available_width(), 52.0), enabled, true) {
            self.spawn(mode);
        }
        let (text, color) = match &self.status {
            Status::Idle => (String::new(), MUTED),
            Status::Waiting(_) => ("Enviando al juego…".to_string(), MUTED),
            Status::Delivered => ("El juego recibió el pedido. Si no aparece, revisa que el mundo esté cargado.".to_string(), GOOD),
            Status::NoAnswer => ("El juego no respondió. Revisa que esté abierto con el DLL de M182 y con el mundo cargado.".to_string(), BAD),
            Status::Error(e) => (format!("Error: {e}"), BAD),
        };
        if !text.is_empty() {
            ui.label(RichText::new(text).color(color));
        }
        if mode == Mode::Creatures {
            ui.label(RichText::new("En el juego, F8 invoca con estos mismos valores.").color(MUTED).size(13.0));
        }
    }

    fn places_tab(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Lugares guardados", 22.0);
        ui.label(RichText::new("Guarda puntos del mundo para invocar criaturas u objetos ahí.").color(MUTED));
        divider(ui);
        match self.player {
            Some(p) => {
                ui.label(format!("Tu posición actual:  X {:.1}   Y {:.1}   Z {:.1}", p[0], p[1], p[2]));
            }
            None => {
                ui.label(RichText::new("Sin posición: abre el juego y carga tu mundo.").color(MUTED));
            }
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_place).hint_text("Nombre del lugar").desired_width(260.0));
            let ready = self.player.is_some() && !self.new_place.trim().is_empty();
            if button(ui, "Guardar mi posición", Vec2::new(220.0, 32.0), ready, false) {
                if let Some(p) = self.player {
                    self.places.push(Place { name: self.new_place.trim().to_string(), x: p[0], y: p[1], z: p[2] });
                    self.new_place.clear();
                    self.save_places();
                }
            }
        });
        divider(ui);
        let mut remove = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, p) in self.places.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&p.name).font(title_font(15.0)).color(PARCHMENT));
                    ui.label(RichText::new(format!("X {:.0}  Y {:.0}  Z {:.0}", p.x, p.y, p.z)).color(MUTED));
                    if button(ui, "Borrar", Vec2::new(80.0, 26.0), true, false) {
                        remove = Some(i);
                    }
                });
            }
            if self.places.is_empty() {
                ui.label(RichText::new("No hay lugares guardados todavía.").color(MUTED));
            }
        });
        if let Some(i) = remove {
            self.places.remove(i);
            self.save_places();
        }
    }

    fn pvp_tab(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Combate entre jugadores", 22.0);
        ui.label(RichText::new("Los cambios se aplican en el juego en unos 2 segundos, sin reiniciar.").color(MUTED));
        divider(ui);
        let mut changed = false;
        changed |= ui.checkbox(&mut self.pvp, "Activar PvP: los jugadores se pueden atacar entre sí").changed();
        changed |= ui.add_enabled(self.pvp, egui::Checkbox::new(&mut self.pvp_friends, "También aplicar a las comprobaciones de aliados")).changed();
        if changed {
            self.notice = match self.set_config(&[("pvp", json!(self.pvp)), ("pvp_friend_checks", json!(self.pvp && self.pvp_friends))]) {
                Ok(()) => "Guardado. El juego lo aplica solo.".to_string(),
                Err(e) => format!("Error: {e}"),
            };
        }
        if !self.notice.is_empty() {
            ui.label(RichText::new(&self.notice).color(GOOD));
        }
    }

    fn soon_tab(&mut self, ui: &mut egui::Ui, title: &str, text: &str) {
        heading(ui, title, 22.0);
        divider(ui);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width().min(560.0), 120.0), Sense::hover());
        ornate_frame(ui.painter(), rect);
        ui.painter().text(rect.center() - Vec2::new(0.0, 22.0), Align2::CENTER_CENTER, "PRÓXIMAMENTE", title_font(20.0), GOLD);
        let galley = ui.painter().layout(text.to_string(), egui::FontId::proportional(16.0), PARCHMENT, rect.width() - 50.0);
        let pos = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.center().y + 2.0);
        ui.painter().galley(pos, galley, PARCHMENT);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick(ctx);

        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::none().fill(LEATHER).inner_margin(egui::Margin::symmetric(20.0, 12.0)).stroke(Stroke::new(1.0_f32, BRONZE)))
            .show(ctx, |ui| {
                self.header(ui);
            });

        egui::SidePanel::left("tabs")
            .exact_width(210.0)
            .frame(egui::Frame::none().fill(LEATHER).inner_margin(egui::Margin::same(12.0)).stroke(Stroke::new(1.0_f32, BRONZE)))
            .show(ctx, |ui| self.tabs(ui));

        egui::CentralPanel::default().frame(egui::Frame::none().fill(BG)).show(ctx, |ui| match self.tab {
            Tab::Creatures => self.catalog_view(ui, Mode::Creatures),
            Tab::Objects => self.catalog_view(ui, Mode::Objects),
            tab => {
                egui::Frame::none().inner_margin(egui::Margin::same(20.0)).show(ui, |ui| match tab {
                    Tab::Places => self.places_tab(ui),
                    Tab::Pvp => self.pvp_tab(ui),
                    Tab::SafeZones => self.soon_tab(ui, "Zonas seguras", "Crea áreas donde nadie puede ser atacado ni saqueado."),
                    _ => self.soon_tab(ui, "Equipos", "Arma equipos de jugadores y define quién es aliado."),
                });
            }
        });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 780.0])
            .with_min_inner_size([980.0, 600.0])
            .with_title("M182 Admin Panel"),
        ..Default::default()
    };
    eframe::run_native("M182 Admin Panel", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(guid: &str, label: &str, group: &str, internal: bool) -> Entry {
        Entry {
            guid: guid.into(),
            label: label.into(),
            group: group.into(),
            boss: false,
            search: label.to_lowercase(),
            kind: emblems::Kind::Fang,
            internal,
        }
    }

    fn app_in(dir: &Path, entries: Vec<Entry>) -> App {
        App {
            dir: dir.to_path_buf(),
            tab: Tab::Creatures,
            creatures: Catalog { entries, selected: Some(0), ..Default::default() },
            objects: Catalog::default(),
            level: 30,
            count: 2,
            distance: 7.0,
            custom_health: true,
            health: 500,
            damage: 1.5,
            pvp: true,
            pvp_friends: false,
            target: Target::Around,
            place_index: 0,
            coords: [0.0; 3],
            places: Vec::new(),
            new_place: String::new(),
            player: None,
            player_checked: Instant::now(),
            icons: HashMap::new(),
            status: Status::Idle,
            notice: String::new(),
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("m182_panel_{tag}_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("export")).unwrap();
        dir
    }

    fn read_request(dir: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir.join(REQUEST_FILE)).unwrap()).unwrap()
    }

    #[test]
    fn names_are_readable() {
        assert_eq!(pretty("Base_Enemy_AncientConstruct_Heavy"), "Ancient Construct Heavy");
        assert_eq!(pretty("Enemy_LizardpeopleDrak01_Melee_Spear"), "Lizardpeople Drak01 Melee Spear");
    }

    #[test]
    fn internal_entries_are_hidden() {
        assert!(is_internal("Base_Enemy_AncientConstruct_Modular01_sleep"));
        assert!(is_internal("ProjectileSpawner_AncientConstruct_Modular_Head02_Turret"));
        assert!(!is_internal("Enemy_Skeleton_Melee01"));
    }

    #[test]
    fn emblem_kinds() {
        assert_eq!(emblems::kind_of("Enemy_Skeleton_Melee01", "Skeleton"), emblems::Kind::Skull);
        assert_eq!(emblems::kind_of("Enemy_Wildbeast_Wolf_Heavy", "WildBeast"), emblems::Kind::Paw);
        assert_eq!(emblems::kind_of_object("Chest_Wood", "Contenedores"), emblems::Kind::Chest);
        assert_eq!(emblems::kind_of_object("Iron_Ore_Node", "Recursos"), emblems::Kind::Gem);
    }

    #[test]
    fn object_catalog_is_grouped_and_filtered() {
        let dir = temp_dir("obj");
        std::fs::write(
            dir.join("export").join("m182_objects.csv"),
            "name,guid,components\nChest_Wood,22222222-2222-2222-2222-222222222222,CurrentTransform|Container\nRock_Big,33333333-3333-3333-3333-333333333333,CurrentTransform\nSomething,44444444-4444-4444-4444-444444444444,Health\nTorch_Wall,55555555-5555-5555-5555-555555555555,CurrentTransform|Light\n",
        )
        .unwrap();
        let list = load_objects(&dir).unwrap();
        assert_eq!(list.len(), 4);
        let mut cat = Catalog { entries: list, ..Default::default() };
        assert_eq!(cat.visible().len(), 3);
        cat.show_internal = true;
        assert_eq!(cat.visible().len(), 4);
        cat.show_internal = false;
        cat.group = "Contenedores".into();
        assert_eq!(cat.visible().len(), 1);
        assert_eq!(object_group("Torch_Wall", "CurrentTransform|Light"), "Luces");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn creature_request_roundtrip() {
        let dir = temp_dir("rt");
        std::fs::write(dir.join("export").join("m182_enemies.csv"), "name,guid,faction,boss,preset\nEnemy_A,642098d4-fc09-48e2-811e-782d438bf551,Drak,false,\nEnemy_A@x10,642098d4-fc09-4182-0002-782d438bf551,Drak,false,x10\n").unwrap();
        std::fs::write(dir.join(CONFIG_FILE), r#"{"pvp":true,"spawn_key":"F8","custom":1}"#).unwrap();
        let list = load_enemies(&dir).unwrap();
        assert_eq!(list.len(), 1);
        let mut app = app_in(&dir, list);
        app.spawn(Mode::Creatures);
        assert!(matches!(app.status, Status::Waiting(_)));
        let req = read_request(&dir);
        assert_eq!(req["health"], 500);
        assert_eq!(req["level"], 30);
        assert!(req.get("at").is_none());
        assert!(req.get("forward").is_none());
        assert!(req.get("object").is_none());
        let cfg = read_config(&dir);
        assert_eq!(cfg["custom"], 1);
        assert_eq!(cfg["spawn_health"], 500);
        assert_eq!(cfg["spawn_key"], "F8");
        std::fs::remove_file(dir.join(REQUEST_FILE)).unwrap();
        let ctx = egui::Context::default();
        app.tick(&ctx);
        assert!(matches!(app.status, Status::Delivered));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn targets_and_object_requests() {
        let dir = temp_dir("pl");
        std::fs::write(dir.join(PLAYER_FILE), r#"{"x":10.5,"y":20.0,"z":-3.25}"#).unwrap();
        assert_eq!(read_player(&dir), Some([10.5, 20.0, -3.25]));
        let mut app = app_in(&dir, vec![entry("642098d4-fc09-48e2-811e-782d438bf551", "A", "Drak", false)]);
        app.objects = Catalog { entries: vec![entry("22222222-2222-2222-2222-222222222222", "Chest", "Contenedores", false)], selected: Some(0), ..Default::default() };
        app.places.push(Place { name: "Torre".into(), x: 1.0, y: 2.0, z: 3.0 });
        app.save_places();
        assert_eq!(load_places(&dir).len(), 1);

        app.target = Target::Place;
        app.spawn(Mode::Creatures);
        assert_eq!(read_request(&dir)["at"], json!([1.0, 2.0, 3.0]));
        app.target = Target::Coords;
        app.coords = [7.0, 8.0, 9.0];
        app.spawn(Mode::Creatures);
        assert_eq!(read_request(&dir)["at"], json!([7.0, 8.0, 9.0]));

        app.target = Target::Forward;
        app.spawn(Mode::Objects);
        let req = read_request(&dir);
        assert_eq!(req["forward"], true);
        assert_eq!(req["object"], true);
        assert!(req.get("health").is_none());
        assert!(req.get("level").is_none());
        let cfg = read_config(&dir);
        assert_eq!(cfg.get("spawn_template").and_then(|v| v.as_str()), Some("642098d4-fc09-48e2-811e-782d438bf551"));

        app.places.clear();
        app.target = Target::Place;
        app.spawn(Mode::Creatures);
        assert!(matches!(app.status, Status::Error(_)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pvp_toggle_keeps_other_settings() {
        let dir = temp_dir("pvp");
        std::fs::write(dir.join(CONFIG_FILE), r#"{"pvp":false,"spawner":true,"spawn_level":9}"#).unwrap();
        let app = app_in(&dir, Vec::new());
        app.set_config(&[("pvp", json!(true)), ("pvp_friend_checks", json!(false))]).unwrap();
        let cfg = read_config(&dir);
        assert_eq!(cfg["pvp"], true);
        assert_eq!(cfg["spawn_level"], 9);
        assert_eq!(cfg["spawner"], true);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn real_catalog_loads() {
        let path = std::path::Path::new("/mnt/user-data/uploads/Enshrouded");
        if path.join("export/m182_enemies.csv").exists() {
            let list = load_enemies(path).unwrap();
            assert!(list.len() > 400);
        }
    }
}

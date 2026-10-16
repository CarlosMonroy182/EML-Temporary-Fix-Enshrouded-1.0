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
const DONATE_URL: &str = "";

struct Enemy {
    guid: String,
    label: String,
    faction: String,
    boss: bool,
    search: String,
    kind: emblems::Kind,
    internal: bool,
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
enum Target {
    Front,
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
    enemies: Vec<Enemy>,
    load_error: Option<String>,
    selected: Option<usize>,
    search: String,
    faction: String,
    only_bosses: bool,
    show_internal: bool,
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

fn faction_color(raw: &str) -> Color32 {
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

fn load_enemies(dir: &Path) -> Result<Vec<Enemy>, String> {
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
        out.push(Enemy {
            guid: parts[1].trim().to_string(),
            search: format!("{} {}", label, faction_label(parts[2])).to_lowercase(),
            internal: is_internal(parts[0]),
            kind: emblems::kind_of(parts[0], parts[2].trim()),
            label,
            faction: parts[2].trim().to_string(),
            boss: parts[3].trim() == "true",
        });
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    if out.is_empty() {
        return Err("La lista de enemigos está vacía.".to_string());
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

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        let dir = find_game_dir();
        let cfg = read_config(&dir);
        let (enemies, load_error) = match load_enemies(&dir) {
            Ok(list) => (list, None),
            Err(e) => (Vec::new(), Some(e)),
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
            _ => Tab::Creatures,
        };
        #[cfg(not(feature = "linux-preview"))]
        let preview_tab = Tab::Creatures;
        Self {
            tab: preview_tab,
            enemies,
            load_error,
            selected: if cfg!(feature = "linux-preview") && std::env::var("M182_PREVIEW_SELECT").is_ok() { Some(120) } else { None },
            search: String::new(),
            faction: String::new(),
            only_bosses: false,
            show_internal: false,
            level: get_u("spawn_level", 20).clamp(1, 100),
            count: get_u("spawn_count", 1).clamp(1, 20),
            distance: get_f("spawn_distance", 5.0).clamp(1.0, 60.0),
            custom_health: health > 0,
            health: if health > 0 { health } else { 1000 },
            damage: get_f("spawn_damage", 1.0).clamp(0.0, 20.0),
            pvp: get_b("pvp"),
            pvp_friends: get_b("pvp_friend_checks"),
            target: Target::Front,
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
        }
    }

    fn effective_health(&self) -> u32 {
        if self.custom_health {
            self.health
        } else {
            0
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
            Target::Front => None,
            Target::Place => self.places.get(self.place_index).map(|p| [p.x, p.y, p.z]),
            Target::Coords => Some(self.coords),
        }
    }

    fn spawn(&mut self) {
        let Some(index) = self.selected else { return };
        let guid = self.enemies[index].guid.clone();
        let at = self.target_point();
        if self.target != Target::Front && at.is_none() {
            self.status = Status::Error("Elige un lugar guardado.".to_string());
            return;
        }
        let saved = self.set_config(&[
            ("spawn_template", json!(guid)),
            ("spawn_level", json!(self.level)),
            ("spawn_count", json!(self.count)),
            ("spawn_distance", json!(self.distance)),
            ("spawn_health", json!(self.effective_health())),
            ("spawn_damage", json!(self.damage)),
            ("spawner", json!(true)),
        ]);
        let result = saved.and_then(|_| {
            let mut request = json!({
                "template": guid,
                "level": self.level,
                "count": self.count,
                "distance": self.distance,
                "health": self.effective_health(),
                "damage": self.damage,
            });
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

    fn visible(&self) -> Vec<usize> {
        let query = self.search.trim().to_lowercase();
        self.enemies
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                (self.faction.is_empty() || e.faction == self.faction)
                    && (!self.only_bosses || e.boss)
                    && (self.show_internal || !e.internal)
                    && (query.is_empty() || e.search.contains(&query))
            })
            .map(|(i, _)| i)
            .collect()
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
            (Tab::Places, "Lugares", false),
            (Tab::Pvp, "PvP", false),
            (Tab::Objects, "Objetos", true),
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

    fn creatures(&mut self, ui: &mut egui::Ui) {
        egui::SidePanel::right("creature_controls")
            .exact_width(320.0)
            .frame(egui::Frame::none().fill(LEATHER).inner_margin(egui::Margin::same(16.0)).stroke(Stroke::new(1.0_f32, BRONZE)))
            .show_inside(ui, |ui| egui::ScrollArea::vertical().show(ui, |ui| self.creature_controls(ui)));

        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin { left: 20.0, right: 16.0, top: 16.0, bottom: 8.0 })).show_inside(ui, |ui| {
            if let Some(error) = &self.load_error {
                ui.label(RichText::new(error).color(BAD));
                return;
            }
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Buscar criatura…").desired_width(240.0));
                let mut factions: Vec<&str> = self.enemies.iter().map(|e| e.faction.as_str()).collect();
                factions.sort_unstable();
                factions.dedup();
                let current = if self.faction.is_empty() { "Todas las facciones".to_string() } else { faction_label(&self.faction).to_string() };
                egui::ComboBox::from_id_salt("faction").selected_text(current).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.faction, String::new(), "Todas las facciones");
                    for f in factions {
                        ui.selectable_value(&mut self.faction, f.to_string(), faction_label(f));
                    }
                });
                ui.checkbox(&mut self.only_bosses, "Solo jefes");
                ui.checkbox(&mut self.show_internal, "Mostrar entradas internas");
            });
            let visible = self.visible();
            ui.label(RichText::new(format!("{} criaturas", visible.len())).color(MUTED));
            self.grid(ui, &visible);
        });
    }

    fn grid(&mut self, ui: &mut egui::Ui, visible: &[usize]) {
        let spacing = 10.0;
        let min_card = 200.0;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let width = ui.available_width();
            let cols = (((width + spacing) / (min_card + spacing)).floor() as usize).max(1);
            let card_w = (width - spacing * (cols as f32 - 1.0)) / cols as f32 - 2.0;
            ui.spacing_mut().item_spacing = Vec2::new(spacing, spacing);
            for row in visible.chunks(cols) {
                ui.horizontal(|ui| {
                    for &i in row {
                        self.card(ui, i, card_w);
                    }
                });
            }
        });
    }

    fn card(&mut self, ui: &mut egui::Ui, index: usize, width: f32) {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 70.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            if response.clicked() {
                self.selected = Some(index);
            }
            return;
        }
        let guid = self.enemies[index].guid.clone();
        if !self.icons.contains_key(&guid) {
            let loaded = load_icon(ui.ctx(), &self.dir, &guid);
            self.icons.insert(guid.clone(), loaded);
        }
        let texture = self.icons.get(&guid).and_then(|t| t.clone());
        let selected = self.selected == Some(index);
        let e = &self.enemies[index];
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
                painter.image(
                    texture.id(),
                    icon.shrink(2.0),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => emblems::draw(painter, icon.shrink(4.0), e.kind, faction_color(&e.faction), e.boss),
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
            faction_label(&e.faction),
            title_font(10.5),
            faction_color(&e.faction),
        );
        if response.clicked() {
            self.selected = Some(index);
            self.status = Status::Idle;
        }
    }

    fn creature_controls(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Invocación", 19.0);
        divider(ui);
        match self.selected {
            Some(i) => {
                let e = &self.enemies[i];
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
                        emblems::draw(painter, target, e.kind, faction_color(&e.faction), e.boss);
                    }
                }
                ui.label(RichText::new(&e.label).font(title_font(17.0)).color(PARCHMENT));
                ui.label(RichText::new(faction_label(&e.faction)).color(faction_color(&e.faction)));
            }
            None => {
                ui.label(RichText::new("Elige una criatura de la lista").color(MUTED));
            }
        }
        divider(ui);

        label_title(ui, "NIVEL");
        ui.add(egui::Slider::new(&mut self.level, 1..=100));
        label_title(ui, "CANTIDAD");
        ui.add(egui::Slider::new(&mut self.count, 1..=20));
        label_title(ui, "VIDA");
        ui.checkbox(&mut self.custom_health, "Vida personalizada");
        ui.add_enabled(self.custom_health, egui::Slider::new(&mut self.health, 10..=1_000_000).logarithmic(true));
        label_title(ui, "DAÑO  (1.00 = normal)");
        ui.add(egui::Slider::new(&mut self.damage, 0.0..=20.0).fixed_decimals(2));
        divider(ui);

        label_title(ui, "DÓNDE APARECE");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.target, Target::Front, "Cerca de mí");
            ui.selectable_value(&mut self.target, Target::Place, "Lugar");
            ui.selectable_value(&mut self.target, Target::Coords, "Punto");
        });
        match self.target {
            Target::Front => {
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

        let enabled = self.selected.is_some();
        if button(ui, "INVOCAR", Vec2::new(ui.available_width(), 52.0), enabled, true) {
            self.spawn();
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
        ui.label(RichText::new("En el juego, F8 invoca con estos mismos valores.").color(MUTED).size(13.0));
    }

    fn places_tab(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Lugares guardados", 22.0);
        ui.label(RichText::new("Guarda puntos del mundo para invocar criaturas ahí desde la pestaña Criaturas.").color(MUTED));
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
            Tab::Creatures => self.creatures(ui),
            tab => {
                egui::Frame::none().inner_margin(egui::Margin::same(20.0)).show(ui, |ui| match tab {
                    Tab::Places => self.places_tab(ui),
                    Tab::Pvp => self.pvp_tab(ui),
                    Tab::Objects => self.soon_tab(ui, "Objetos", "Invoca objetos y recursos directamente en tu mundo."),
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

    fn app_in(dir: &Path, enemies: Vec<Enemy>) -> App {
        App {
            dir: dir.to_path_buf(),
            tab: Tab::Creatures,
            enemies,
            load_error: None,
            selected: Some(0),
            search: String::new(),
            faction: String::new(),
            only_bosses: false,
            show_internal: false,
            level: 30,
            count: 2,
            distance: 7.0,
            custom_health: true,
            health: 500,
            damage: 1.5,
            pvp: true,
            pvp_friends: false,
            target: Target::Front,
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

    #[test]
    fn names_are_readable() {
        assert_eq!(pretty("Base_Enemy_AncientConstruct_Heavy"), "Ancient Construct Heavy");
        assert_eq!(pretty("Enemy_LizardpeopleDrak01_Melee_Spear"), "Lizardpeople Drak01 Melee Spear");
    }

    #[test]
    fn internal_entries_are_hidden() {
        assert!(is_internal("Base_Enemy_AncientConstruct_Modular01_sleep"));
        assert!(is_internal("Enemy_Fog_Vorgoth_Organ_Channel_Tentacle_QuickAttack01"));
        assert!(is_internal("Base_Enemy_Boss_Lizard"));
        assert!(is_internal("ProjectileSpawner_AncientConstruct_Modular_Head02_Turret"));
        assert!(!is_internal("Enemy_Skeleton_Melee01"));
        assert!(!is_internal("Enemy_Wildbeast_Wolf_Heavy"));
    }

    #[test]
    fn emblem_kinds() {
        assert_eq!(emblems::kind_of("Enemy_Skeleton_Melee01", "Skeleton"), emblems::Kind::Skull);
        assert_eq!(emblems::kind_of("Enemy_Wildbeast_Wolf_Heavy", "WildBeast"), emblems::Kind::Paw);
        assert_eq!(emblems::kind_of("Enemy_Wildbeast_SpiderAnt_Melee", "WildBeast"), emblems::Kind::Spider);
        assert_eq!(emblems::kind_of("Enemy_Fogger_Soldier01", "Fogger"), emblems::Kind::Mist);
    }

    #[test]
    fn request_and_config_roundtrip() {
        let dir = temp_dir("rt");
        std::fs::write(dir.join("export").join("m182_enemies.csv"), "name,guid,faction,boss,preset\nEnemy_A,642098d4-fc09-48e2-811e-782d438bf551,Drak,false,\nEnemy_A@x10,642098d4-fc09-4182-0002-782d438bf551,Drak,false,x10\n").unwrap();
        std::fs::write(dir.join(CONFIG_FILE), r#"{"pvp":true,"spawn_key":"F8","custom":1}"#).unwrap();
        let list = load_enemies(&dir).unwrap();
        assert_eq!(list.len(), 1);
        let mut app = app_in(&dir, list);
        app.spawn();
        assert!(matches!(app.status, Status::Waiting(_)));
        let req: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(REQUEST_FILE)).unwrap()).unwrap();
        assert_eq!(req["health"], 500);
        assert_eq!(req["level"], 30);
        assert!(req.get("at").is_none());
        let cfg = read_config(&dir);
        assert_eq!(cfg["custom"], 1);
        assert_eq!(cfg["spawn_health"], 500);
        assert_eq!(cfg["spawn_key"], "F8");
        assert_eq!(app.visible().len(), 1);
        std::fs::remove_file(dir.join(REQUEST_FILE)).unwrap();
        let ctx = egui::Context::default();
        app.tick(&ctx);
        assert!(matches!(app.status, Status::Delivered));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn places_and_targets() {
        let dir = temp_dir("pl");
        std::fs::write(dir.join(PLAYER_FILE), r#"{"x":10.5,"y":20.0,"z":-3.25}"#).unwrap();
        assert_eq!(read_player(&dir), Some([10.5, 20.0, -3.25]));
        let enemy = Enemy {
            guid: "642098d4-fc09-48e2-811e-782d438bf551".into(),
            label: "A".into(),
            faction: "Drak".into(),
            boss: false,
            search: "a".into(),
            kind: emblems::Kind::Fang,
            internal: false,
        };
        let mut app = app_in(&dir, vec![enemy]);
        app.places.push(Place { name: "Torre".into(), x: 1.0, y: 2.0, z: 3.0 });
        app.save_places();
        assert_eq!(load_places(&dir).len(), 1);
        app.target = Target::Place;
        app.spawn();
        let req: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(REQUEST_FILE)).unwrap()).unwrap();
        assert_eq!(req["at"], json!([1.0, 2.0, 3.0]));
        app.target = Target::Coords;
        app.coords = [7.0, 8.0, 9.0];
        app.spawn();
        let req: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(REQUEST_FILE)).unwrap()).unwrap();
        assert_eq!(req["at"], json!([7.0, 8.0, 9.0]));
        app.places.clear();
        app.target = Target::Place;
        app.spawn();
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

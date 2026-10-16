#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, FontId, RichText, Rounding, Sense, Stroke, Vec2};
use serde_json::{json, Map, Value};

const FALLBACK_GAME_DIR: &str = r"D:\SteamLibrary\steamapps\common\Enshrouded";
const CONFIG_FILE: &str = "m182_native.json";
const REQUEST_FILE: &str = "m182_spawn.request";
const REQUEST_TMP: &str = "m182_spawn.request.tmp";

const AMBER: Color32 = Color32::from_rgb(244, 176, 52);
const BG: Color32 = Color32::from_rgb(18, 19, 24);
const PANEL: Color32 = Color32::from_rgb(26, 28, 36);
const CARD: Color32 = Color32::from_rgb(36, 39, 50);
const CARD_HOVER: Color32 = Color32::from_rgb(48, 52, 67);
const TEXT: Color32 = Color32::from_rgb(232, 233, 238);
const MUTED: Color32 = Color32::from_rgb(140, 145, 160);

struct Enemy {
    guid: String,
    label: String,
    faction: String,
    boss: bool,
    search: String,
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
    enemies: Vec<Enemy>,
    load_error: Option<String>,
    selected: Option<usize>,
    search: String,
    faction: String,
    only_bosses: bool,
    level: u32,
    count: u32,
    distance: f32,
    custom_health: bool,
    health: u32,
    damage: f32,
    pvp: bool,
    status: Status,
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
        "Ancients" => Color32::from_rgb(120, 190, 220),
        "Cyclops" => Color32::from_rgb(220, 140, 90),
        "Drak" => Color32::from_rgb(110, 200, 120),
        "Fogger" => Color32::from_rgb(170, 130, 220),
        "HitAll" => Color32::from_rgb(230, 100, 100),
        "HitOthers" => Color32::from_rgb(220, 120, 160),
        "Scavenger" => Color32::from_rgb(210, 180, 100),
        "Skeleton" => Color32::from_rgb(200, 200, 190),
        "Vukah" => Color32::from_rgb(90, 190, 190),
        "WildBeast" => Color32::from_rgb(190, 150, 100),
        _ => Color32::from_rgb(150, 190, 110),
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

fn find_game_dir() -> PathBuf {
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

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BG;
        visuals.window_fill = PANEL;
        visuals.extreme_bg_color = Color32::from_rgb(12, 13, 17);
        visuals.selection.bg_fill = AMBER;
        visuals.selection.stroke = Stroke::new(1.0_f32, Color32::BLACK);
        visuals.widgets.inactive.bg_fill = CARD;
        visuals.widgets.inactive.weak_bg_fill = CARD;
        visuals.widgets.hovered.weak_bg_fill = CARD_HOVER;
        visuals.widgets.hovered.bg_fill = CARD_HOVER;
        visuals.widgets.active.bg_fill = AMBER;
        visuals.override_text_color = Some(TEXT);
        cc.egui_ctx.set_visuals(visuals);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.slider_width = 200.0;
        cc.egui_ctx.set_style(style);

        let dir = find_game_dir();
        let cfg = read_config(&dir);
        let (enemies, load_error) = match load_enemies(&dir) {
            Ok(list) => (list, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let get_u = |key: &str, default: u32| cfg.get(key).and_then(|v| v.as_u64()).map(|v| v as u32).unwrap_or(default);
        let get_f = |key: &str, default: f32| cfg.get(key).and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(default);
        let health = get_u("spawn_health", 0);
        Self {
            dir,
            enemies,
            load_error,
            selected: None,
            search: String::new(),
            faction: String::new(),
            only_bosses: false,
            level: get_u("spawn_level", 20).clamp(1, 100),
            count: get_u("spawn_count", 1).clamp(1, 20),
            distance: get_f("spawn_distance", 5.0).clamp(1.0, 60.0),
            custom_health: health > 0,
            health: if health > 0 { health } else { 1000 },
            damage: get_f("spawn_damage", 1.0).clamp(0.0, 20.0),
            pvp: cfg.get("pvp").and_then(|v| v.as_bool()).unwrap_or(false),
            status: Status::Idle,
        }
    }

    fn effective_health(&self) -> u32 {
        if self.custom_health {
            self.health
        } else {
            0
        }
    }

    fn save_config(&self, template: &str) -> Result<(), String> {
        let mut cfg = read_config(&self.dir);
        cfg.insert("spawn_template".into(), json!(template));
        cfg.insert("spawn_level".into(), json!(self.level));
        cfg.insert("spawn_count".into(), json!(self.count));
        cfg.insert("spawn_distance".into(), json!(self.distance));
        cfg.insert("spawn_health".into(), json!(self.effective_health()));
        cfg.insert("spawn_damage".into(), json!(self.damage));
        cfg.insert("pvp".into(), json!(self.pvp));
        cfg.entry("enabled").or_insert(json!(true));
        cfg.insert("spawner".into(), json!(true));
        let text = serde_json::to_string_pretty(&Value::Object(cfg)).map_err(|e| e.to_string())?;
        write_atomic(&self.dir, CONFIG_FILE, "m182_native.json.tmp", &text)
    }

    fn spawn(&mut self) {
        let Some(index) = self.selected else { return };
        let guid = self.enemies[index].guid.clone();
        let result = self.save_config(&guid).and_then(|_| {
            let request = json!({
                "template": guid,
                "level": self.level,
                "count": self.count,
                "distance": self.distance,
                "health": self.effective_health(),
                "damage": self.damage,
            });
            write_atomic(&self.dir, REQUEST_FILE, REQUEST_TMP, &request.to_string())
        });
        self.status = match result {
            Ok(()) => Status::Waiting(Instant::now()),
            Err(e) => Status::Error(e),
        };
    }

    fn update_status(&mut self) {
        if let Status::Waiting(since) = self.status {
            if !self.dir.join(REQUEST_FILE).exists() {
                self.status = Status::Delivered;
            } else if since.elapsed() > Duration::from_secs(6) {
                let _ = std::fs::remove_file(self.dir.join(REQUEST_FILE));
                self.status = Status::NoAnswer;
            }
        }
    }

    fn visible(&self) -> Vec<usize> {
        let query = self.search.trim().to_lowercase();
        self.enemies
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                (self.faction.is_empty() || e.faction == self.faction)
                    && (!self.only_bosses || e.boss)
                    && (query.is_empty() || e.search.contains(&query))
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn draw_grid(&mut self, ui: &mut egui::Ui) {
        let visible = self.visible();
        ui.label(RichText::new(format!("{} enemigos", visible.len())).color(MUTED));
        let spacing = 8.0;
        let min_card = 170.0;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let width = ui.available_width();
            let cols = (((width + spacing) / (min_card + spacing)).floor() as usize).max(1);
            let card_w = (width - spacing * (cols as f32 - 1.0)) / cols as f32 - 1.0;
            ui.spacing_mut().item_spacing = Vec2::new(spacing, spacing);
            for row in visible.chunks(cols) {
                ui.horizontal(|ui| {
                    for &i in row {
                        self.draw_card(ui, i, card_w);
                    }
                });
            }
        });
    }

    fn draw_card(&mut self, ui: &mut egui::Ui, index: usize, width: f32) {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 58.0), Sense::click());
        let selected = self.selected == Some(index);
        let e = &self.enemies[index];
        let fill = if selected {
            Color32::from_rgb(58, 48, 26)
        } else if response.hovered() {
            CARD_HOVER
        } else {
            CARD
        };
        let painter = ui.painter();
        painter.rect_filled(rect, Rounding::same(8.0), fill);
        let stroke = if selected { Stroke::new(2.0_f32, AMBER) } else { Stroke::new(1.0_f32, Color32::from_rgb(52, 56, 70)) };
        painter.rect_stroke(rect, Rounding::same(8.0), stroke);
        let bar = egui::Rect::from_min_size(rect.min + Vec2::new(0.0, 8.0), Vec2::new(4.0, rect.height() - 16.0));
        painter.rect_filled(bar, Rounding::same(2.0), faction_color(&e.faction));
        let galley = painter.layout(
            e.label.clone(),
            FontId::proportional(13.5),
            TEXT,
            width - 24.0,
        );
        painter.galley(rect.min + Vec2::new(14.0, 7.0), galley, TEXT);
        painter.text(
            rect.left_bottom() + Vec2::new(14.0, -7.0),
            Align2::LEFT_BOTTOM,
            faction_label(&e.faction),
            FontId::proportional(11.5),
            MUTED,
        );
        if e.boss {
            painter.text(
                rect.right_bottom() + Vec2::new(-8.0, -7.0),
                Align2::RIGHT_BOTTOM,
                "JEFE",
                FontId::proportional(11.0),
                AMBER,
            );
        }
        if response.clicked() {
            self.selected = Some(index);
            self.status = Status::Idle;
        }
    }

    fn draw_side(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label(RichText::new("M182 SPAWNER").size(20.0).strong().color(AMBER));
        ui.label(RichText::new("Invoca enemigos en tu mundo").color(MUTED));
        ui.separator();

        match self.selected {
            Some(i) => {
                let e = &self.enemies[i];
                ui.label(RichText::new("Enemigo elegido").color(MUTED).size(12.0));
                ui.label(RichText::new(&e.label).size(17.0).strong());
                ui.label(RichText::new(faction_label(&e.faction)).color(faction_color(&e.faction)));
            }
            None => {
                ui.label(RichText::new("Elige un enemigo de la cuadrícula").color(MUTED));
            }
        }
        ui.separator();

        ui.label("Nivel");
        ui.add(egui::Slider::new(&mut self.level, 1..=100));
        ui.label("Cantidad");
        ui.add(egui::Slider::new(&mut self.count, 1..=20));
        ui.label("Distancia de ti (metros)");
        ui.add(egui::Slider::new(&mut self.distance, 1.0..=60.0).fixed_decimals(0));
        ui.separator();

        ui.checkbox(&mut self.custom_health, "Vida personalizada");
        ui.add_enabled(
            self.custom_health,
            egui::Slider::new(&mut self.health, 10..=1_000_000).logarithmic(true),
        );
        if !self.custom_health {
            ui.label(RichText::new("Vida normal del enemigo").color(MUTED).size(12.0));
        }
        ui.label("Daño (×1.00 = normal)");
        ui.add(egui::Slider::new(&mut self.damage, 0.0..=20.0).fixed_decimals(2));
        ui.separator();
        ui.checkbox(&mut self.pvp, "PvP: los jugadores se pueden atacar");
        ui.add_space(6.0);

        let enabled = self.selected.is_some();
        let button = egui::Button::new(RichText::new("INVOCAR").size(18.0).strong().color(Color32::BLACK))
            .fill(if enabled { AMBER } else { Color32::from_rgb(90, 80, 55) })
            .min_size(Vec2::new(ui.available_width(), 46.0))
            .rounding(Rounding::same(8.0));
        if ui.add_enabled(enabled, button).clicked() {
            self.spawn();
        }

        let (text, color) = match &self.status {
            Status::Idle => (String::new(), MUTED),
            Status::Waiting(_) => ("Enviando al juego…".to_string(), MUTED),
            Status::Delivered => ("Pedido recibido por el juego. Si no aparece, mira que el mundo esté cargado.".to_string(), Color32::from_rgb(120, 200, 130)),
            Status::NoAnswer => ("El juego no respondió. Revisa que esté abierto con el DLL de M182 y que el mundo esté cargado.".to_string(), Color32::from_rgb(230, 110, 100)),
            Status::Error(e) => (format!("Error: {e}"), Color32::from_rgb(230, 110, 100)),
        };
        if !text.is_empty() {
            ui.label(RichText::new(text).color(color));
        }
        ui.add_space(6.0);
        ui.label(RichText::new("En el juego también puedes pulsar F8 para invocar con estos mismos valores.").color(MUTED).size(11.5));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_status();
        if matches!(self.status, Status::Waiting(_)) {
            ctx.request_repaint_after(Duration::from_millis(150));
        }

        egui::SidePanel::right("controls")
            .exact_width(300.0)
            .frame(egui::Frame::none().fill(PANEL).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| self.draw_side(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| {
                if let Some(error) = &self.load_error {
                    ui.label(RichText::new(error).color(Color32::from_rgb(230, 110, 100)));
                    return;
                }
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Buscar enemigo…").desired_width(260.0));
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
                });
                ui.add_space(4.0);
                self.draw_grid(ui);
            });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 720.0])
            .with_min_inner_size([760.0, 520.0])
            .with_title("M182 Spawner"),
        ..Default::default()
    };
    eframe::run_native("M182 Spawner", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_readable() {
        assert_eq!(pretty("Base_Enemy_AncientConstruct_Heavy"), "Ancient Construct Heavy");
        assert_eq!(pretty("Enemy_LizardpeopleDrak01_Melee_Spear"), "Lizardpeople Drak01 Melee Spear");
    }

    #[test]
    fn request_and_config_roundtrip() {
        let dir = std::env::temp_dir().join(format!("m182_panel_test_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("export")).unwrap();
        std::fs::write(dir.join("export").join("m182_enemies.csv"), "name,guid,faction,boss,preset\nEnemy_A,642098d4-fc09-48e2-811e-782d438bf551,Drak,false,\nEnemy_A@x10,642098d4-fc09-4182-0002-782d438bf551,Drak,false,x10\n").unwrap();
        std::fs::write(dir.join(CONFIG_FILE), r#"{"pvp":true,"spawn_key":"F8","custom":1}"#).unwrap();
        let list = load_enemies(&dir).unwrap();
        assert_eq!(list.len(), 1);
        let mut app = App {
            dir: dir.clone(),
            enemies: list,
            load_error: None,
            selected: Some(0),
            search: String::new(),
            faction: String::new(),
            only_bosses: false,
            level: 30,
            count: 2,
            distance: 7.0,
            custom_health: true,
            health: 500,
            damage: 1.5,
            pvp: true,
            status: Status::Idle,
        };
        app.spawn();
        assert!(matches!(app.status, Status::Waiting(_)));
        let req: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(REQUEST_FILE)).unwrap()).unwrap();
        assert_eq!(req["health"], 500);
        assert_eq!(req["level"], 30);
        let cfg = read_config(&dir);
        assert_eq!(cfg["custom"], 1);
        assert_eq!(cfg["spawn_health"], 500);
        assert_eq!(cfg["spawn_key"], "F8");
        assert_eq!(app.visible().len(), 1);
        std::fs::remove_file(dir.join(REQUEST_FILE)).unwrap();
        app.update_status();
        assert!(matches!(app.status, Status::Delivered));
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

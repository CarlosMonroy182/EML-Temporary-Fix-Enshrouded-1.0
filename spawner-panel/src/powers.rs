use std::path::Path;

use eframe::egui::{self, RichText, Vec2};
use serde_json::{json, Value};

use crate::theme::*;
use crate::{write_atomic, App};

pub const POWERS_FILE: &str = "m182_powers.json";
pub const COMMAND_FILE: &str = "m182_command.request";

#[derive(Clone)]
pub struct PowersState {
    pub god: bool,
    pub stamina: bool,
    pub fly: bool,
    pub fly_speed: f32,
    pub unknown_height: bool,
    pub teleport: [f64; 3],
    pub notice: String,
}

impl Default for PowersState {
    fn default() -> Self {
        Self { god: false, stamina: false, fly: false, fly_speed: 14.0, unknown_height: true, teleport: [0.0; 3], notice: String::new() }
    }
}

impl PowersState {
    pub fn load(dir: &Path) -> Self {
        let mut s = Self::default();
        if let Some(v) = std::fs::read_to_string(dir.join(POWERS_FILE)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            s.god = v.get("god").and_then(|x| x.as_bool()).unwrap_or(false);
            s.stamina = v.get("stamina").and_then(|x| x.as_bool()).unwrap_or(false);
            s.fly = v.get("fly").and_then(|x| x.as_bool()).unwrap_or(false);
            if let Some(f) = v.get("fly_speed").and_then(|x| x.as_f64()) {
                s.fly_speed = (f as f32).clamp(2.0, 60.0);
            }
        }
        s
    }

    pub fn to_json(&self) -> Value {
        json!({
            "god": self.god,
            "stamina": self.stamina,
            "fly": self.fly,
            "fly_speed": self.fly_speed,
        })
    }
}

pub fn save(dir: &Path, state: &PowersState) -> Result<(), String> {
    write_atomic(dir, POWERS_FILE, "m182_powers.json.tmp", &state.to_json().to_string())
}

pub fn command(dir: &Path, body: &Value) -> Result<(), String> {
    write_atomic(dir, COMMAND_FILE, "m182_command.request.tmp", &body.to_string())
}

#[allow(dead_code)]
pub fn teleport_body(point: [f64; 3], height_known: bool) -> Value {
    teleport_body_with_ground(point, height_known, None)
}

pub fn teleport_body_with_ground(point: [f64; 3], height_known: bool, ground: Option<f64>) -> Value {
    if height_known {
        json!({"cmd": "teleport", "at": [point[0], point[1], point[2]]})
    } else {
        let mut body = json!({"cmd": "teleport", "at": [point[0], Value::Null, point[2]]});
        if let Some(g) = ground.filter(|g| g.is_finite()) {
            body["ground"] = json!(g);
        }
        body
    }
}

impl App {
    fn save_powers(&mut self) {
        self.powers.notice = match save(&self.dir, &self.powers) {
            Ok(()) => "Enviado al juego. Se aplica en menos de un segundo.".to_string(),
            Err(e) => format!("Error: {e}"),
        };
    }

    pub fn powers_tab(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Admin del jugador", 22.0);
        ui.label(RichText::new("Solo afectan a tu personaje (el anfitrión). Se aplican mientras el juego está abierto.").color(MUTED));
        divider(ui);

        let mut changed = false;
        changed |= ui.checkbox(&mut self.powers.god, "Modo dios: la vida se mantiene llena").changed();
        changed |= ui.checkbox(&mut self.powers.stamina, "Stamina y maná infinitos").changed();
        changed |= ui.checkbox(&mut self.powers.fly, "Volar: Espacio sube, Ctrl izquierdo baja").changed();
        ui.add_enabled_ui(self.powers.fly, |ui| {
            label_title(ui, "VELOCIDAD DE VUELO (metros por segundo)");
            changed |= ui.add(egui::Slider::new(&mut self.powers.fly_speed, 2.0..=60.0).fixed_decimals(0)).changed();
        });
        if changed {
            self.save_powers();
        }
        ui.add_space(6.0);
        if button(ui, "Curarme ahora", Vec2::new(220.0, 36.0), true, false) {
            self.powers.notice = match command(&self.dir, &json!({"cmd": "heal"})) {
                Ok(()) => "Pedido enviado: vida, stamina y maná al máximo.".to_string(),
                Err(e) => format!("Error: {e}"),
            };
        }
        divider(ui);

        heading(ui, "Teletransporte", 18.0);
        ui.label(RichText::new("Elige un lugar guardado o escribe coordenadas. También puedes elegir un punto en la pestaña Mapa.").color(MUTED));
        if !self.places.is_empty() {
            let mut picked = None;
            egui::ComboBox::from_id_salt("tp_place").selected_text("Copiar coordenadas de un lugar guardado…").show_ui(ui, |ui| {
                for (i, p) in self.places.iter().enumerate() {
                    if ui.selectable_label(false, &p.name).clicked() {
                        picked = Some(i);
                    }
                }
            });
            if let Some(i) = picked {
                let p = &self.places[i];
                self.powers.teleport = [p.x, p.y, p.z];
                self.powers.unknown_height = false;
            }
        }
        for (label, value) in ["X", "Y (altura)", "Z"].iter().zip(self.powers.teleport.iter_mut()) {
            ui.horizontal(|ui| {
                ui.label(*label);
                ui.add(egui::DragValue::new(value).speed(1.0).fixed_decimals(1));
            });
        }
        ui.checkbox(&mut self.powers.unknown_height, "No conozco la altura: bajar solo hasta el suelo con caída protegida");
        ui.horizontal(|ui| {
            if let Some(p) = self.player {
                if button(ui, "Usar mi posición", Vec2::new(170.0, 32.0), true, false) {
                    self.powers.teleport = p;
                    self.powers.unknown_height = false;
                }
            }
            if let Some(p) = self.map.selected {
                if button(ui, "Usar punto del mapa", Vec2::new(190.0, 32.0), true, false) {
                    self.powers.teleport = p;
                    self.powers.unknown_height = !self.map.selected_height_known;
                }
            }
            if button(ui, "TELETRANSPORTAR", Vec2::new(210.0, 32.0), true, true) {
                let t = self.powers.teleport;
                let ground = self.ground_estimate(t[0], t[2]);
                let body = teleport_body_with_ground(t, !self.powers.unknown_height, ground);
                self.powers.notice = match command(&self.dir, &body) {
                    Ok(()) => "Pedido de teletransporte enviado.".to_string(),
                    Err(e) => format!("Error: {e}"),
                };
            }
        });
        if !self.powers.notice.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(&self.powers.notice).color(GOOD));
        }
    }
}

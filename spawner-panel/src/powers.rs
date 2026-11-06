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
    pub aura: bool,
    pub fly: bool,
    pub fly_speed: f32,
    pub notice: String,
}

impl Default for PowersState {
    fn default() -> Self {
        Self { god: false, stamina: false, aura: false, fly: false, fly_speed: 14.0, notice: String::new() }
    }
}

impl PowersState {
    pub fn load(dir: &Path) -> Self {
        let mut s = Self::default();
        if let Some(v) = std::fs::read_to_string(dir.join(POWERS_FILE)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            s.god = v.get("god").and_then(|x| x.as_bool()).unwrap_or(false);
            s.stamina = v.get("stamina").and_then(|x| x.as_bool()).unwrap_or(false);
            s.aura = v.get("aura").and_then(|x| x.as_bool()).unwrap_or(false);
        }
        s
    }

    pub fn to_json(&self) -> Value {
        json!({
            "god": self.god,
            "stamina": self.stamina,
            "aura": self.aura,
            "fly": false,
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

impl App {
    fn save_powers(&mut self) {
        self.powers.notice = match save(&self.dir, &self.powers) {
            Ok(()) => "Enviado al juego. Se aplica en menos de un segundo.".to_string(),
            Err(e) => format!("Error: {e}"),
        };
    }

    pub fn powers_tab(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Admin del jugador", 22.0);
        divider(ui);

        let mut changed = false;
        changed |= ui.checkbox(&mut self.powers.god, "Modo dios: la vida se mantiene llena").changed();
        changed |= ui.checkbox(&mut self.powers.stamina, "Stamina y maná infinitos").changed();
        changed |= ui.checkbox(&mut self.powers.aura, "Aura divina").changed();
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

        if !self.powers.notice.is_empty() {
            ui.add_space(6.0);
            let color = if self.powers.notice.starts_with("Error") { BAD } else { GOOD };
            ui.label(RichText::new(&self.powers.notice).color(color));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
}

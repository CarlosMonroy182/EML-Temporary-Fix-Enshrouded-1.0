use eframe::egui::{self, RichText, Vec2};

use crate::remote;
use crate::theme::*;
use crate::App;

impl App {
    /// One line under the header while the panel drives a server.
    pub fn server_strip(&mut self, ui: &mut egui::Ui) {
        if !remote::active() {
            return;
        }
        let status = remote::status();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("SERVIDOR").color(GOLD).size(14.5));
            if status.sending {
                ui.label(RichText::new("escribiendo… no toques el teclado").color(GOLD_DIM).size(14.5));
            } else if !status.error.is_empty() {
                ui.label(RichText::new(status.error.clone()).color(BAD).size(14.5));
            } else if status.queued > 0 {
                ui.label(RichText::new(format!("{} envíos en cola", status.queued)).color(MUTED).size(14.5));
            } else if !status.last.is_empty() {
                ui.label(RichText::new(status.last.clone()).color(GOOD).size(14.5));
            } else {
                ui.label(RichText::new("listo").color(MUTED).size(14.5));
            }
            if button(ui, "Reenviar todo", Vec2::new(130.0, 24.0), remote::connected() && !status.sending, false) {
                self.resend_state();
            }
        });
    }

    #[allow(dead_code)]
    pub fn server_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("SERVIDOR DEDICADO").font(title_font(22.0)).color(GOLD));
        ui.label(RichText::new("Con la clave del servidor, este panel manda sus órdenes escribiéndolas por ti en el chat del juego (nadie más las ve). Funciona con el juego abierto y dentro del mundo del servidor.").color(MUTED));
        ui.add_space(6.0);
        let mut changed = false;
        changed |= ui.checkbox(&mut self.remote_cfg.enabled, "Controlar un servidor dedicado (desmárcalo para partida local o anfitrión)").changed();
        ui.horizontal(|ui| {
            ui.label("Clave del servidor");
            changed |= ui.add(egui::TextEdit::singleline(&mut self.remote_cfg.key).password(true).desired_width(260.0)).changed();
        });
        ui.horizontal(|ui| {
            ui.label("Largo máximo de cada mensaje");
            let mut limit = self.remote_cfg.limit as u32;
            if ui.add(egui::DragValue::new(&mut limit).speed(5.0).range(remote::MIN_LIMIT as u32..=remote::MAX_LIMIT as u32)).changed() {
                self.remote_cfg.limit = limit as usize;
                changed = true;
            }
            ui.label(RichText::new("Si el chat del juego recorta mensajes largos, bájalo.").color(MUTED).size(14.5));
        });
        if changed {
            let settings = self.remote_cfg.clone();
            remote::configure(settings.clone());
            self.remote_notice = match remote::save_settings(&self.dir, &settings) {
                Ok(()) if settings.enabled && !crate::remote_proto::valid_key(&settings.key) => "La clave debe tener 6 o más caracteres y ningún espacio.".to_string(),
                Ok(()) => "Guardado.".to_string(),
                Err(e) => format!("Error: {e}"),
            };
        }
        ui.horizontal(|ui| {
            if button(ui, "PROBAR CONEXIÓN", Vec2::new(190.0, 32.0), remote::can_type(), true) {
                remote::submit_ping();
                self.remote_notice = "Prueba enviada. En el servidor, m182_spawner.log debe mostrar «chat: client N ok: ping».".to_string();
            }
            if button(ui, "Cancelar envíos", Vec2::new(150.0, 32.0), remote::status().queued > 0, false) {
                remote::clear_queue();
            }
        });
        if !self.remote_notice.is_empty() {
            ui.label(RichText::new(self.remote_notice.clone()).color(GOOD).size(14.5));
        }
        ui.label(RichText::new("Para que el envío funcione: el chat del juego debe abrirse con Enter y el mundo del servidor estar cargado. No uses el teclado mientras el panel escribe.").color(MUTED).size(14.5));
    }
}

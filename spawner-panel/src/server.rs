use eframe::egui::{self, RichText, Vec2};
use serde_json::json;

use crate::remote;
use crate::theme::*;
use crate::{powers, App};

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
                ui.label(RichText::new("escribiendo en el chat del juego… no toques el teclado").color(GOLD_DIM).size(14.5));
            } else if !status.error.is_empty() {
                ui.label(RichText::new(status.error.clone()).color(BAD).size(14.5));
            } else if status.queued > 0 {
                ui.label(RichText::new(format!("{} envíos en cola", status.queued)).color(MUTED).size(14.5));
            } else if !status.last.is_empty() {
                ui.label(RichText::new(status.last.clone()).color(GOOD).size(14.5));
            } else {
                ui.label(RichText::new("listo: lo que hagas aquí se envía al servidor").color(MUTED).size(14.5));
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

    #[allow(dead_code)]
    pub fn copies_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("ARMAS ÚNICAS").font(title_font(22.0)).color(GOLD));
        ui.label(RichText::new("Cada arma o armadura que edites sale como una copia propia, con sus stats, sin cambiar las demás. El juego solo conoce los ítems que existen al abrirse: por eso cada ítem editado reserva unas copias (se crean al abrir el juego o al reiniciar el servidor).").color(MUTED));
        let pending = self.items.book.pending_count();
        if remote::active() && pending > 0 {
            ui.label(RichText::new(format!("Hay {pending} ítems con copias reservadas esperando un reinicio del servidor.")).color(GOLD_DIM));
            if button(ui, "YA REINICIÉ EL SERVIDOR", Vec2::new(260.0, 32.0), true, true) {
                self.items.book.confirm_restart();
                let _ = self.items.book.save(&self.dir);
            }
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Reservar copias de varias armas de una vez (4 por arma). Alarga un poco el arranque del juego:").color(MUTED).size(14.5));
        ui.horizontal_wrapped(|ui| {
            for (label, slot) in [("Cuerpo a cuerpo", "MeleeWeapon"), ("Arcos y ballestas", "RangeWeapon"), ("Bastones y varitas", "MidRangeWeapon"), ("Escudos", "Shield")] {
                if button(ui, label, Vec2::new(170.0, 30.0), true, false) {
                    let bases: Vec<u32> = self.items.rows.iter().filter(|r| r.slot == slot && r.deliverable()).map(|r| r.id).collect();
                    let mut added = 0;
                    for base in &bases {
                        if self.items.book.register(*base, 4) {
                            added += 1;
                        }
                    }
                    let _ = self.items.book.save(&self.dir);
                    self.items.notice = match powers::command(&self.dir, &json!({"cmd": "item_pools", "bases": bases, "slots": 4})) {
                        Ok(()) if added == 0 => "Esas armas ya estaban reservadas.".to_string(),
                        Ok(()) => format!("Reservadas copias de {added} ítems. Reinicia {} una vez para crearlas.", if remote::active() { "el servidor y pulsa «Ya reinicié el servidor»" } else { "el juego" }),
                        Err(e) => format!("Error: {e}"),
                    };
                }
            }
        });
        if !self.items.notice.is_empty() {
            ui.label(RichText::new(self.items.notice.clone()).color(GOOD).size(14.5));
        }
        let used: usize = self.items.book.pools.values().map(|p| p.used.values().filter(|u| u.active).count()).sum();
        ui.label(RichText::new(format!("Ítems con copias reservadas: {} · copias en uso: {} · copias de {} ítems por crear", self.items.book.pools.len(), used, pending)).color(MUTED).size(14.5));
    }
}

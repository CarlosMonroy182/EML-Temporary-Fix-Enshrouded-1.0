use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::theme::*;
use crate::{powers, App, Place, Tab, Target};

pub const CELL: f64 = 12.0;

pub struct MapState {
    pub cells: HashMap<(i32, i32), (f32, u32)>,
    pub points: usize,
    pub y_min: f32,
    pub y_max: f32,
    pub mtime: Option<SystemTime>,
    pub checked: Instant,
    pub center: [f64; 2],
    pub scale: f64,
    pub fitted: bool,
    pub selected: Option<[f64; 3]>,
    pub selected_height_known: bool,
    pub name: String,
    pub paste: String,
    pub notice: String,
}

impl Default for MapState {
    fn default() -> Self {
        Self {
            cells: HashMap::new(),
            points: 0,
            y_min: 0.0,
            y_max: 1.0,
            mtime: None,
            checked: Instant::now(),
            center: [0.0, 0.0],
            scale: 0.1,
            fitted: false,
            selected: None,
            selected_height_known: false,
            name: String::new(),
            paste: String::new(),
            notice: String::new(),
        }
    }
}

pub fn cell_of(x: f64, z: f64) -> (i32, i32) {
    ((x / CELL).floor() as i32, (z / CELL).floor() as i32)
}

pub fn parse_trail(text: &str) -> HashMap<(i32, i32), (f32, u32)> {
    let mut cells: HashMap<(i32, i32), (f32, u32)> = HashMap::new();
    for line in text.lines().skip(1) {
        let mut it = line.split(',');
        let (Some(x), Some(y), Some(z)) = (it.next(), it.next(), it.next()) else { continue };
        let (Ok(x), Ok(y), Ok(z)) = (x.trim().parse::<f64>(), y.trim().parse::<f32>(), z.trim().parse::<f64>()) else { continue };
        let e = cells.entry(cell_of(x, z)).or_insert((0.0, 0));
        e.0 = (e.0 * e.1 as f32 + y) / (e.1 + 1) as f32;
        e.1 += 1;
    }
    cells
}

pub fn parse_coordinates(text: &str) -> Option<[f64; 3]> {
    let cleaned: String = text.chars().map(|c| if c.is_ascii_digit() || c == '-' || c == '.' { c } else { ' ' }).collect();
    let numbers: Vec<f64> = cleaned.split_whitespace().filter_map(|t| t.parse::<f64>().ok()).collect();
    match numbers.len() {
        3 => Some([numbers[0], numbers[1], numbers[2]]),
        2 => Some([numbers[0], f64::NAN, numbers[1]]),
        _ => None,
    }
}

impl MapState {
    pub fn reload(&mut self, dir: &Path) {
        let path = dir.join("m182_trail.csv");
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if mtime == self.mtime && self.points > 0 {
            return;
        }
        self.mtime = mtime;
        let Ok(text) = std::fs::read_to_string(&path) else { return };
        self.cells = parse_trail(&text);
        self.points = self.cells.values().map(|c| c.1 as usize).sum();
        if let Some(min) = self.cells.values().map(|c| c.0).reduce(f32::min) {
            self.y_min = min;
        }
        if let Some(max) = self.cells.values().map(|c| c.0).reduce(f32::max) {
            self.y_max = max.max(self.y_min + 1.0);
        }
    }

    pub fn height_near(&self, x: f64, z: f64) -> Option<f32> {
        let (cx, cz) = cell_of(x, z);
        let mut best: Option<(i32, f32)> = None;
        for dx in -3..=3 {
            for dz in -3..=3 {
                if let Some((y, _)) = self.cells.get(&(cx + dx, cz + dz)) {
                    let d = dx * dx + dz * dz;
                    if best.map(|b| d < b.0).unwrap_or(true) {
                        best = Some((d, *y));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    pub fn fit(&mut self, extra: &[[f64; 3]], size: Vec2) {
        let mut min = [f64::MAX; 2];
        let mut max = [f64::MIN; 2];
        for (cx, cz) in self.cells.keys() {
            for (axis, v) in [(0, *cx as f64 * CELL), (1, *cz as f64 * CELL)] {
                min[axis] = min[axis].min(v);
                max[axis] = max[axis].max(v);
            }
        }
        for p in extra {
            min[0] = min[0].min(p[0]);
            max[0] = max[0].max(p[0]);
            min[1] = min[1].min(p[2]);
            max[1] = max[1].max(p[2]);
        }
        if min[0] > max[0] {
            self.center = [4000.0, 2000.0];
            self.scale = 0.15;
            return;
        }
        self.center = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
        let w = (max[0] - min[0]).max(200.0) * 1.2;
        let h = (max[1] - min[1]).max(200.0) * 1.2;
        self.scale = ((size.x as f64) / w).min((size.y as f64) / h).clamp(0.02, 20.0);
    }
}

fn altitude_color(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let stops = [(0.0, [40.0, 70.0, 74.0]), (0.45, [96.0, 112.0, 70.0]), (0.8, [176.0, 150.0, 96.0]), (1.0, [232.0, 222.0, 196.0])];
    for w in stops.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.0 {
            let k = (t - a.0) / (b.0 - a.0);
            return Color32::from_rgb(
                (a.1[0] + (b.1[0] - a.1[0]) * k) as u8,
                (a.1[1] + (b.1[1] - a.1[1]) * k) as u8,
                (a.1[2] + (b.1[2] - a.1[2]) * k) as u8,
            );
        }
    }
    Color32::WHITE
}

fn grid_step(scale: f64) -> f64 {
    let target = 120.0 / scale;
    for step in [50.0, 100.0, 250.0, 500.0, 1000.0, 2000.0] {
        if step >= target {
            return step;
        }
    }
    4000.0
}

impl App {
    fn to_screen(&self, rect: Rect, x: f64, z: f64) -> Pos2 {
        let m = &self.map;
        Pos2::new(
            rect.center().x + ((x - m.center[0]) * m.scale) as f32,
            rect.center().y - ((z - m.center[1]) * m.scale) as f32,
        )
    }

    fn to_world(&self, rect: Rect, p: Pos2) -> (f64, f64) {
        let m = &self.map;
        (
            m.center[0] + ((p.x - rect.center().x) as f64) / m.scale,
            m.center[1] - ((p.y - rect.center().y) as f64) / m.scale,
        )
    }

    pub fn map_tab(&mut self, ui: &mut egui::Ui) {
        if self.map.checked.elapsed() >= Duration::from_secs(3) || !self.map.fitted {
            self.map.checked = Instant::now();
            self.map.reload(&self.dir);
        }
        egui::SidePanel::right("map_controls")
            .exact_width(320.0)
            .frame(egui::Frame::none().fill(LEATHER).inner_margin(egui::Margin::same(16.0)).stroke(Stroke::new(1.0_f32, BRONZE)))
            .show_inside(ui, |ui| egui::ScrollArea::vertical().show(ui, |ui| self.map_controls(ui)));
        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin::same(14.0))).show_inside(ui, |ui| {
            let size = ui.available_size();
            let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
            if !self.map.fitted {
                let mut extra: Vec<[f64; 3]> = self.places.iter().map(|p| [p.x, p.y, p.z]).collect();
                if let Some(p) = self.player {
                    extra.push(p);
                }
                self.map.fit(&extra, rect.size());
                self.map.fitted = true;
            }
            if response.dragged() {
                let d = response.drag_delta();
                self.map.center[0] -= d.x as f64 / self.map.scale;
                self.map.center[1] += d.y as f64 / self.map.scale;
            }
            if response.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    let pointer = ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center());
                    let before = self.to_world(rect, pointer);
                    self.map.scale = (self.map.scale * (1.0 + scroll as f64 * 0.0015)).clamp(0.02, 40.0);
                    let after = self.to_world(rect, pointer);
                    self.map.center[0] += before.0 - after.0;
                    self.map.center[1] += before.1 - after.1;
                }
            }
            if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let (x, z) = self.to_world(rect, pos);
                    let height = self.map.height_near(x, z);
                    let y = height.map(|h| h as f64).unwrap_or_else(|| self.player.map(|p| p[1]).unwrap_or(0.0));
                    self.map.selected = Some([x.round(), y.round(), z.round()]);
                    self.map.selected_height_known = height.is_some();
                    self.map.notice.clear();
                }
            }

            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 4.0, Color32::from_rgb(18, 14, 10));
            let step = grid_step(self.map.scale);
            let (x0, z1) = self.to_world(rect, rect.left_top());
            let (x1, z0) = self.to_world(rect, rect.right_bottom());
            let grid = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(160, 120, 60, 40));
            let mut gx = (x0 / step).floor() * step;
            while gx <= x1 {
                let a = self.to_screen(rect, gx, z0);
                painter.line_segment([Pos2::new(a.x, rect.top()), Pos2::new(a.x, rect.bottom())], grid);
                painter.text(Pos2::new(a.x + 3.0, rect.bottom() - 4.0), Align2::LEFT_BOTTOM, format!("{gx:.0}"), egui::FontId::proportional(11.0), GOLD_DIM);
                gx += step;
            }
            let mut gz = (z0 / step).floor() * step;
            while gz <= z1 {
                let a = self.to_screen(rect, x0, gz);
                painter.line_segment([Pos2::new(rect.left(), a.y), Pos2::new(rect.right(), a.y)], grid);
                painter.text(Pos2::new(rect.left() + 4.0, a.y - 2.0), Align2::LEFT_BOTTOM, format!("{gz:.0}"), egui::FontId::proportional(11.0), GOLD_DIM);
                gz += step;
            }

            let side = (CELL * self.map.scale).max(1.5) as f32;
            let span = (self.map.y_max - self.map.y_min).max(1.0);
            for ((cx, cz), (y, _)) in &self.map.cells {
                let wx = *cx as f64 * CELL;
                let wz = *cz as f64 * CELL;
                if wx < x0 - CELL || wx > x1 + CELL || wz < z0 - CELL || wz > z1 + CELL {
                    continue;
                }
                let p = self.to_screen(rect, wx, wz + CELL);
                painter.rect_filled(Rect::from_min_size(p, Vec2::splat(side)), 0.0, altitude_color((*y - self.map.y_min) / span));
            }
            for place in &self.places {
                let p = self.to_screen(rect, place.x, place.z);
                if rect.contains(p) {
                    crate::theme::diamond(&painter, p, 5.0, GOLD);
                    painter.text(p + Vec2::new(8.0, -2.0), Align2::LEFT_CENTER, &place.name, egui::FontId::proportional(13.0), PARCHMENT);
                }
            }
            if let Some(pl) = self.player {
                let p = self.to_screen(rect, pl[0], pl[2]);
                painter.circle_filled(p, 6.0, GOOD);
                painter.circle_stroke(p, 9.0, Stroke::new(1.5_f32, PARCHMENT));
            }
            if let Some(sel) = self.map.selected {
                let p = self.to_screen(rect, sel[0], sel[2]);
                let s = Stroke::new(2.0_f32, Color32::from_rgb(230, 90, 70));
                painter.line_segment([p - Vec2::new(10.0, 0.0), p + Vec2::new(10.0, 0.0)], s);
                painter.line_segment([p - Vec2::new(0.0, 10.0), p + Vec2::new(0.0, 10.0)], s);
                painter.circle_stroke(p, 6.0, s);
            }
            painter.rect_stroke(rect, 4.0, Stroke::new(1.5_f32, BRONZE));
            if self.map.cells.is_empty() {
                painter.text(rect.center(), Align2::CENTER_CENTER, "Aún no hay recorrido guardado.\nCamina por el mundo con el juego abierto y se irá dibujando.", egui::FontId::proportional(18.0), MUTED);
            }
            if let Some(pos) = response.hover_pos() {
                let (x, z) = self.to_world(rect, pos);
                painter.text(rect.right_top() + Vec2::new(-10.0, 8.0), Align2::RIGHT_TOP, format!("X {x:.0}   Z {z:.0}"), egui::FontId::proportional(14.0), PARCHMENT);
            }
        });
    }

    fn map_controls(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Mapa de coordenadas", 19.0);
        ui.label(RichText::new("Arrastra para mover, rueda para acercar. Haz clic para elegir un punto.").color(MUTED));
        divider(ui);
        match self.map.selected {
            Some(sel) => {
                label_title(ui, "PUNTO ELEGIDO");
                ui.label(format!("X {:.0}    Y {:.0}    Z {:.0}", sel[0], sel[1], sel[2]));
                if self.map.selected_height_known {
                    ui.label(RichText::new("Altura estimada de tu recorrido.").color(MUTED));
                } else {
                    ui.label(RichText::new("Sin recorrido cerca: la altura es la de tu posición actual y puede estar mal.").color(MUTED));
                }
                let text = format!("{:.0}, {:.0}, {:.0}", sel[0], sel[1], sel[2]);
                if button(ui, "Copiar coordenadas", Vec2::new(ui.available_width(), 32.0), true, false) {
                    ui.ctx().copy_text(text);
                    self.map.notice = "Coordenadas copiadas al portapapeles.".to_string();
                }
                ui.add_space(6.0);
                label_title(ui, "GUARDAR COMO LUGAR");
                ui.add(egui::TextEdit::singleline(&mut self.map.name).hint_text("Nombre del lugar").desired_width(ui.available_width()));
                if button(ui, "Guardar lugar", Vec2::new(ui.available_width(), 32.0), !self.map.name.trim().is_empty(), false) {
                    self.places.push(Place { name: self.map.name.trim().to_string(), x: sel[0], y: sel[1], z: sel[2] });
                    self.map.name.clear();
                    self.save_places();
                    self.map.notice = "Lugar guardado.".to_string();
                }
                ui.add_space(6.0);
                if button(ui, "Teletransportarme aquí", Vec2::new(ui.available_width(), 32.0), true, false) {
                    let body = powers::teleport_body(sel, self.map.selected_height_known);
                    self.map.notice = match powers::command(&self.dir, &body) {
                        Ok(()) => "Pedido de teletransporte enviado.".to_string(),
                        Err(e) => format!("Error: {e}"),
                    };
                }
                if button(ui, "Invocar aquí…", Vec2::new(ui.available_width(), 32.0), true, false) {
                    self.target = Target::Coords;
                    self.coords = sel;
                    self.tab = Tab::Creatures;
                }
            }
            None => {
                ui.label(RichText::new("Aún no elegiste ningún punto.").color(MUTED));
            }
        }
        divider(ui);
        label_title(ui, "PEGAR COORDENADAS");
        ui.add(egui::TextEdit::singleline(&mut self.map.paste).hint_text("Ej: 3722, 842, 1367").desired_width(ui.available_width()));
        if button(ui, "Usar estas coordenadas", Vec2::new(ui.available_width(), 30.0), !self.map.paste.trim().is_empty(), false) {
            match parse_coordinates(&self.map.paste) {
                Some(c) => {
                    let known = c[1].is_finite();
                    let y = if known { c[1] } else { self.player.map(|p| p[1]).unwrap_or(0.0) };
                    self.map.selected = Some([c[0], y, c[2]]);
                    self.map.selected_height_known = known;
                    self.map.center = [c[0], c[2]];
                    self.map.notice = "Punto colocado en el mapa.".to_string();
                }
                None => self.map.notice = "No entendí las coordenadas. Escribe X, Y, Z o solo X, Z.".to_string(),
            }
        }
        divider(ui);
        if button(ui, "Centrar en mi posición", Vec2::new(ui.available_width(), 30.0), self.player.is_some(), false) {
            if let Some(p) = self.player {
                self.map.center = [p[0], p[2]];
            }
        }
        if button(ui, "Ver todo el recorrido", Vec2::new(ui.available_width(), 30.0), true, false) {
            self.map.fitted = false;
        }
        ui.label(RichText::new(format!("{} puntos de recorrido guardados", self.map.points)).color(MUTED));
        if !self.map.notice.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(&self.map.notice).color(GOOD));
        }
    }
}

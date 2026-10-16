use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::theme::*;
use crate::tiles::{self, Markers, Slot, TileCache};
use crate::{powers, App, Place, Tab, Target};

pub const CELL: f64 = 12.0;

fn world_center() -> [f64; 2] {
    let (px, py) = tiles::world_to_px(4096.0, 4096.0);
    [px, py]
}

#[derive(PartialEq, Clone, Copy)]
pub enum MarkerLoad {
    Idle,
    Loading,
    Ready,
    Failed,
}

pub struct MapState {
    pub cells: HashMap<(i32, i32), (f32, u32)>,
    pub points: usize,
    pub y_min: f32,
    pub y_max: f32,
    pub mtime: Option<SystemTime>,
    pub checked: Instant,
    pub center: [f64; 2],
    pub zf: f64,
    pub fitted: bool,
    pub selected: Option<[f64; 3]>,
    pub selected_height_known: bool,
    pub selected_label: String,
    pub name: String,
    pub paste: String,
    pub notice: String,
    pub tiles: Option<TileCache>,
    pub markers: Markers,
    pub marker_state: MarkerLoad,
    pub marker_rx: Option<Receiver<bool>>,
    pub visible_cats: HashSet<u64>,
    pub cats_ready: bool,
    pub marker_search: String,
    pub show_trail: bool,
    pub show_places: bool,
    pub show_markers: bool,
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
            center: world_center(),
            zf: 10.8,
            fitted: false,
            selected: None,
            selected_height_known: false,
            selected_label: String::new(),
            name: String::new(),
            paste: String::new(),
            notice: String::new(),
            tiles: None,
            markers: Markers::default(),
            marker_state: MarkerLoad::Idle,
            marker_rx: None,
            visible_cats: HashSet::new(),
            cats_ready: false,
            marker_search: String::new(),
            show_trail: false,
            show_places: true,
            show_markers: true,
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
}


fn group_color(group: &str) -> Color32 {
    const PALETTE: [[u8; 3]; 10] = [
        [232, 96, 84],
        [96, 170, 232],
        [120, 208, 120],
        [236, 190, 72],
        [190, 120, 232],
        [84, 214, 200],
        [240, 140, 80],
        [220, 120, 170],
        [170, 200, 90],
        [150, 150, 240],
    ];
    let hash = group.bytes().fold(11u32, |h, b| h.wrapping_mul(33).wrapping_add(b as u32));
    let c = PALETTE[hash as usize % PALETTE.len()];
    Color32::from_rgb(c[0], c[1], c[2])
}

fn altitude_color(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgb((70.0 + 160.0 * t) as u8, (190.0 - 60.0 * t) as u8, (230.0 - 170.0 * t) as u8)
}

struct View {
    rect: Rect,
    center: [f64; 2],
    scale: f64,
}

impl View {
    fn to_screen(&self, px: f64, py: f64) -> Pos2 {
        Pos2::new(
            self.rect.center().x + ((px - self.center[0]) * self.scale) as f32,
            self.rect.center().y + ((py - self.center[1]) * self.scale) as f32,
        )
    }

    fn world_to_screen(&self, x: f64, z: f64) -> Pos2 {
        let (px, py) = tiles::world_to_px(x, z);
        self.to_screen(px, py)
    }

    fn to_world(&self, p: Pos2) -> (f64, f64) {
        let px = self.center[0] + ((p.x - self.rect.center().x) as f64) / self.scale;
        let py = self.center[1] + ((p.y - self.rect.center().y) as f64) / self.scale;
        tiles::px_to_world(px, py)
    }
}

impl App {
    fn start_markers(&mut self) {
        self.map.markers = tiles::load_markers(&self.dir);
        if self.map.markers.markers.is_empty() {
            self.map.marker_rx = Some(tiles::spawn_marker_download(&self.dir));
            self.map.marker_state = MarkerLoad::Loading;
        } else {
            self.map.marker_state = MarkerLoad::Ready;
            self.init_categories();
        }
    }

    fn init_categories(&mut self) {
        let important = ["Locations"];
        let ids: Vec<u64> = self.map.markers.categories.iter().filter(|c| important.contains(&c.group.as_str())).map(|c| c.id).collect();
        self.map.visible_cats = ids.into_iter().collect();
        self.map.cats_ready = true;
    }

    fn marker_at(&self, view: &View, pos: Pos2) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        for (i, m) in self.map.markers.markers.iter().enumerate() {
            if !self.map.visible_cats.contains(&m.category) {
                continue;
            }
            let p = view.world_to_screen(m.x, m.z);
            let d = p.distance(pos);
            if d <= 10.0 && best.map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, i));
            }
        }
        best.map(|b| b.1)
    }

    pub fn map_tab(&mut self, ui: &mut egui::Ui) {
        if self.map.tiles.is_none() {
            self.map.tiles = Some(TileCache::start(&self.dir));
        }
        if self.map.marker_state == MarkerLoad::Idle {
            self.start_markers();
        }
        if let Some(rx) = &self.map.marker_rx {
            if let Ok(ok) = rx.try_recv() {
                self.map.marker_rx = None;
                if ok {
                    self.map.markers = tiles::load_markers(&self.dir);
                    self.map.marker_state = MarkerLoad::Ready;
                    self.init_categories();
                } else {
                    self.map.marker_state = MarkerLoad::Failed;
                }
            }
        }
        if self.map.checked.elapsed() >= Duration::from_secs(3) || !self.map.fitted {
            self.map.checked = Instant::now();
            self.map.reload(&self.dir);
        }
        let ctx = ui.ctx().clone();
        if let Some(t) = self.map.tiles.as_mut() {
            if t.poll(&ctx) || t.pending() > 0 {
                ctx.request_repaint_after(Duration::from_millis(120));
            }
        }
        egui::SidePanel::right("map_controls")
            .exact_width(330.0)
            .frame(egui::Frame::none().fill(LEATHER).inner_margin(egui::Margin::same(16.0)).stroke(Stroke::new(1.0_f32, BRONZE)))
            .show_inside(ui, |ui| egui::ScrollArea::vertical().show(ui, |ui| self.map_controls(ui)));
        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin::same(14.0))).show_inside(ui, |ui| {
            let size = ui.available_size();
            let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
            if !self.map.fitted {
                if let Some(p) = self.player {
                    let (px, py) = tiles::world_to_px(p[0], p[2]);
                    self.map.center = [px, py];
                    self.map.zf = 11.5;
                }
                self.map.fitted = true;
            }
            let scale = 2f64.powf(self.map.zf - 16.0);
            if response.dragged() {
                let d = response.drag_delta();
                self.map.center[0] -= d.x as f64 / scale;
                self.map.center[1] -= d.y as f64 / scale;
            }
            let mut view = View { rect, center: self.map.center, scale };
            if response.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    let pointer = ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center());
                    let before = view.to_world(pointer);
                    self.map.zf = (self.map.zf + scroll as f64 * 0.003).clamp(8.0, 18.0);
                    view.scale = 2f64.powf(self.map.zf - 16.0);
                    let (bpx, bpy) = tiles::world_to_px(before.0, before.1);
                    let sx = (pointer.x - rect.center().x) as f64 / view.scale;
                    let sy = (pointer.y - rect.center().y) as f64 / view.scale;
                    self.map.center = [bpx - sx, bpy - sy];
                    view.center = self.map.center;
                }
            }
            let hover = response.hover_pos();
            let hovered_marker = if self.map.show_markers { hover.and_then(|p| self.marker_at(&view, p)) } else { None };
            if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    if let Some(i) = hovered_marker.or_else(|| if self.map.show_markers { self.marker_at(&view, pos) } else { None }) {
                        let m = self.map.markers.markers[i].clone();
                        let height = self.map.height_near(m.x, m.z);
                        let y = height.map(|h| h as f64).unwrap_or_else(|| self.player.map(|p| p[1]).unwrap_or(0.0));
                        self.map.selected = Some([m.x.round(), y.round(), m.z.round()]);
                        self.map.selected_height_known = height.is_some();
                        self.map.selected_label = m.title.clone();
                        self.map.name = m.title;
                    } else {
                        let (x, z) = view.to_world(pos);
                        let height = self.map.height_near(x, z);
                        let y = height.map(|h| h as f64).unwrap_or_else(|| self.player.map(|p| p[1]).unwrap_or(0.0));
                        self.map.selected = Some([x.round(), y.round(), z.round()]);
                        self.map.selected_height_known = height.is_some();
                        self.map.selected_label.clear();
                    }
                    self.map.notice.clear();
                }
            }

            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 4.0, Color32::from_rgb(14, 11, 8));
            self.draw_tiles(&painter, &view);

            if self.map.show_trail {
                let side = (tiles_cell_px(&view)).max(2.0);
                let span = (self.map.y_max - self.map.y_min).max(1.0);
                for ((cx, cz), (y, _)) in &self.map.cells {
                    let wx = *cx as f64 * CELL;
                    let wz = *cz as f64 * CELL;
                    let p = view.world_to_screen(wx, wz + CELL);
                    if !rect.expand(side).contains(p) {
                        continue;
                    }
                    painter.rect_filled(Rect::from_min_size(p, Vec2::splat(side)), 0.0, altitude_color((*y - self.map.y_min) / span).gamma_multiply(0.75));
                }
            }
            if self.map.show_markers {
                let labels = self.map.zf >= 13.0;
                let query = self.map.marker_search.trim().to_lowercase();
                let group_of: HashMap<u64, &str> = self.map.markers.categories.iter().map(|c| (c.id, c.group.as_str())).collect();
                for (i, m) in self.map.markers.markers.iter().enumerate() {
                    if !self.map.visible_cats.contains(&m.category) {
                        continue;
                    }
                    if !query.is_empty() && !m.title.to_lowercase().contains(&query) {
                        continue;
                    }
                    let p = view.world_to_screen(m.x, m.z);
                    if !rect.expand(10.0).contains(p) {
                        continue;
                    }
                    let color = group_color(group_of.get(&m.category).copied().unwrap_or(""));
                    let r = if hovered_marker == Some(i) { 7.0 } else { 5.0 };
                    painter.circle_filled(p, r + 1.5, Color32::from_rgb(10, 8, 6));
                    painter.circle_filled(p, r, color);
                    if labels || hovered_marker == Some(i) {
                        painter.text(p + Vec2::new(9.0, 0.0), Align2::LEFT_CENTER, &m.title, egui::FontId::proportional(12.5), Color32::WHITE);
                    }
                }
            }
            if self.map.show_places {
                for place in &self.places {
                    let p = view.world_to_screen(place.x, place.z);
                    if rect.expand(40.0).contains(p) {
                        crate::theme::diamond(&painter, p, 7.0, Color32::from_rgb(10, 8, 6));
                        crate::theme::diamond(&painter, p, 5.5, GOLD);
                        let galley = painter.layout_no_wrap(place.name.clone(), egui::FontId::proportional(13.5), PARCHMENT);
                        let pos = p + Vec2::new(10.0, -8.0);
                        painter.rect_filled(Rect::from_min_size(pos - Vec2::new(3.0, 1.0), galley.size() + Vec2::new(6.0, 2.0)), 3.0, Color32::from_black_alpha(170));
                        painter.galley(pos, galley, PARCHMENT);
                    }
                }
            }
            if let Some(pl) = self.player {
                let p = view.world_to_screen(pl[0], pl[2]);
                painter.circle_filled(p, 7.0, GOOD);
                painter.circle_stroke(p, 10.0, Stroke::new(2.0_f32, Color32::WHITE));
            }
            if let Some(sel) = self.map.selected {
                let p = view.world_to_screen(sel[0], sel[2]);
                let s = Stroke::new(2.5_f32, Color32::from_rgb(255, 80, 60));
                painter.line_segment([p - Vec2::new(14.0, 0.0), p + Vec2::new(14.0, 0.0)], s);
                painter.line_segment([p - Vec2::new(0.0, 14.0), p + Vec2::new(0.0, 14.0)], s);
                painter.circle_stroke(p, 8.0, s);
            }
            painter.rect_stroke(rect, 4.0, Stroke::new(1.5_f32, BRONZE));
            if let Some(pos) = hover {
                let (x, z) = view.to_world(pos);
                let text = format!("X {x:.0}    Z {z:.0}");
                let galley = painter.layout_no_wrap(text, egui::FontId::proportional(15.0), Color32::WHITE);
                let at = rect.right_top() + Vec2::new(-galley.size().x - 16.0, 10.0);
                painter.rect_filled(Rect::from_min_size(at - Vec2::new(6.0, 3.0), galley.size() + Vec2::new(12.0, 6.0)), 4.0, Color32::from_black_alpha(190));
                painter.galley(at, galley, Color32::WHITE);
            }
            let (pending, got) = self
                .map
                .tiles
                .as_ref()
                .map(|t| (t.pending(), t.downloaded.load(std::sync::atomic::Ordering::Relaxed)))
                .unwrap_or((0, 0));
            if pending > 0 {
                painter.text(rect.left_bottom() + Vec2::new(10.0, -10.0), Align2::LEFT_BOTTOM, format!("Cargando mapa… {pending} piezas pendientes ({got} descargadas)"), egui::FontId::proportional(13.0), Color32::from_rgb(236, 220, 170));
            }
        });
    }

    fn draw_tiles(&mut self, painter: &egui::Painter, view: &View) {
        let zt = (self.map.zf.round() as i64).clamp(tiles::MIN_ZOOM as i64, tiles::MAX_ZOOM as i64) as u8;
        let span = 256.0 * 2f64.powi(16 - zt as i32);
        let half_w = view.rect.width() as f64 / 2.0 / view.scale;
        let half_h = view.rect.height() as f64 / 2.0 / view.scale;
        let shift = 16 - zt as u32;
        let lo = 32512u32 >> shift;
        let hi = 32767u32 >> shift;
        let x0 = (((view.center[0] - half_w) / span).floor().max(0.0) as u32).max(lo);
        let x1 = (((view.center[0] + half_w) / span).floor().max(0.0) as u32).min(hi);
        let y0 = (((view.center[1] - half_h) / span).floor().max(0.0) as u32).max(lo);
        let y1 = (((view.center[1] + half_h) / span).floor().max(0.0) as u32).min(hi);
        let Some(cache) = self.map.tiles.as_mut() else { return };
        cache.request((tiles::MIN_ZOOM, 127, 127));
        if x1 < x0 || y1 < y0 || x1 - x0 > 40 || y1 - y0 > 40 {
            return;
        }
        let full = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                cache.request((zt, tx, ty));
                let min = view.to_screen(tx as f64 * span, ty as f64 * span);
                let max = view.to_screen((tx + 1) as f64 * span, (ty + 1) as f64 * span);
                let dest = Rect::from_min_max(min, max).expand(0.5);
                match cache.slots.get(&(zt, tx, ty)) {
                    Some(Slot::Ready(tex)) => {
                        painter.image(tex.id(), dest, full, Color32::WHITE);
                    }
                    _ => {
                        for k in 1..=(zt - tiles::MIN_ZOOM) {
                            let parent = (zt - k, tx >> k, ty >> k);
                            if let Some(Slot::Ready(tex)) = cache.slots.get(&parent) {
                                let div = (1u32 << k) as f32;
                                let ux = (tx - ((tx >> k) << k)) as f32 / div;
                                let uy = (ty - ((ty >> k) << k)) as f32 / div;
                                let uv = Rect::from_min_max(Pos2::new(ux, uy), Pos2::new(ux + 1.0 / div, uy + 1.0 / div));
                                painter.image(tex.id(), dest, uv, Color32::WHITE);
                                break;
                            }
                        }
                    }
                }
            }
        }
        cache.trim(zt);
    }

    fn map_controls(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Mapa del mundo", 19.0);
        ui.label(RichText::new("Arrastra para mover y usa la rueda para acercar. Haz clic en cualquier punto o marcador para elegirlo.").color(MUTED));
        divider(ui);
        let width = ui.available_width();
        label_title(ui, "PUNTO ELEGIDO");
        match self.map.selected {
            Some(sel) => {
                if !self.map.selected_label.is_empty() {
                    ui.label(RichText::new(self.map.selected_label.clone()).color(GOLD));
                }
                ui.label(RichText::new(format!("X {:.0}     Y {:.0}     Z {:.0}", sel[0], sel[1], sel[2])).size(17.0));
                if !self.map.selected_height_known {
                    ui.label(RichText::new("La altura es la de tu posición actual y puede no ser exacta. Al teletransportarte puedes volar para ajustar.").color(MUTED).size(12.5));
                }
            }
            None => {
                ui.label(RichText::new("Aún no elegiste ningún punto. Haz clic en el mapa.").color(MUTED));
            }
        }
        ui.add_space(4.0);
        let frame = egui::Frame::none().fill(CARD).stroke(Stroke::new(1.5_f32, GOLD_DIM)).rounding(5.0).inner_margin(egui::Margin::same(10.0));
        frame.show(ui, |ui| {
            label_title(ui, "GUARDAR LUGAR");
            ui.add(egui::TextEdit::singleline(&mut self.map.name).hint_text("Nombre del lugar").desired_width(ui.available_width()));
            let can_name = !self.map.name.trim().is_empty();
            if button(ui, "GUARDAR ESTE PUNTO", Vec2::new(ui.available_width(), 38.0), can_name && self.map.selected.is_some(), true) {
                if let Some(sel) = self.map.selected {
                    self.places.push(Place { name: self.map.name.trim().to_string(), x: sel[0], y: sel[1], z: sel[2] });
                    self.map.name.clear();
                    self.save_places();
                    self.map.notice = "Lugar guardado. Ya aparece en el mapa y en la pestaña Lugares.".to_string();
                }
            }
            if button(ui, "Guardar mi posición actual", Vec2::new(ui.available_width(), 32.0), can_name && self.player.is_some(), false) {
                if let Some(p) = self.player {
                    self.places.push(Place { name: self.map.name.trim().to_string(), x: p[0].round(), y: p[1].round(), z: p[2].round() });
                    self.map.name.clear();
                    self.save_places();
                    self.map.notice = "Tu posición se guardó como lugar.".to_string();
                }
            }
        });
        ui.add_space(6.0);
        if let Some(sel) = self.map.selected {
            if button(ui, "Copiar coordenadas", Vec2::new(width, 30.0), true, false) {
                ui.ctx().copy_text(format!("{:.0}, {:.0}, {:.0}", sel[0], sel[1], sel[2]));
                self.map.notice = "Coordenadas copiadas al portapapeles.".to_string();
            }
            if button(ui, "Teletransportarme aquí", Vec2::new(width, 32.0), true, true) {
                let body = powers::teleport_body(sel, self.map.selected_height_known);
                self.map.notice = match powers::command(&self.dir, &body) {
                    Ok(()) => "Pedido de teletransporte enviado.".to_string(),
                    Err(e) => format!("Error: {e}"),
                };
            }
            ui.horizontal(|ui| {
                if button(ui, "Invocar aquí", Vec2::new((width - 10.0) / 2.0, 30.0), true, false) {
                    self.target = Target::Coords;
                    self.coords = sel;
                    self.tab = Tab::Creatures;
                }
                if button(ui, "Entregar aquí", Vec2::new((width - 10.0) / 2.0, 30.0), true, false) {
                    self.target = Target::Coords;
                    self.coords = sel;
                    self.tab = Tab::Items;
                }
            });
        }
        if !self.map.notice.is_empty() {
            ui.label(RichText::new(self.map.notice.clone()).color(GOOD));
        }
        divider(ui);
        label_title(ui, "PEGAR COORDENADAS");
        ui.add(egui::TextEdit::singleline(&mut self.map.paste).hint_text("Ej: 3722, 842, 1367").desired_width(width));
        if button(ui, "Usar estas coordenadas", Vec2::new(width, 30.0), !self.map.paste.trim().is_empty(), false) {
            match parse_coordinates(&self.map.paste) {
                Some(c) => {
                    let known = c[1].is_finite();
                    let y = if known { c[1] } else { self.player.map(|p| p[1]).unwrap_or(0.0) };
                    self.map.selected = Some([c[0], y, c[2]]);
                    self.map.selected_height_known = known;
                    self.map.selected_label.clear();
                    let (px, py) = tiles::world_to_px(c[0], c[2]);
                    self.map.center = [px, py];
                    self.map.zf = self.map.zf.max(12.0);
                    self.map.notice = "Punto colocado en el mapa.".to_string();
                }
                None => self.map.notice = "No entendí las coordenadas. Escribe X, Y, Z o solo X, Z.".to_string(),
            }
        }
        divider(ui);
        label_title(ui, "VISTA");
        label_title(ui, "ZOOM");
        ui.add(egui::Slider::new(&mut self.map.zf, 8.0..=18.0).fixed_decimals(1));
        if button(ui, "Centrar en mi posición", Vec2::new(width, 30.0), self.player.is_some(), false) {
            if let Some(p) = self.player {
                let (px, py) = tiles::world_to_px(p[0], p[2]);
                self.map.center = [px, py];
                self.map.zf = self.map.zf.max(12.5);
            }
        }
        if button(ui, "Ver todo el mundo", Vec2::new(width, 30.0), true, false) {
            self.map.center = world_center();
            self.map.zf = 10.8;
        }
        ui.checkbox(&mut self.map.show_places, "Mostrar mis lugares guardados");
        ui.checkbox(&mut self.map.show_trail, format!("Mostrar mi recorrido ({} puntos)", self.map.points));
        ui.checkbox(&mut self.map.show_markers, "Mostrar marcadores del mapa");
        if self.map.show_markers {
            self.marker_panel(ui);
        }
    }

    fn marker_panel(&mut self, ui: &mut egui::Ui) {
        match self.map.marker_state {
            MarkerLoad::Loading | MarkerLoad::Idle => {
                ui.label(RichText::new("Descargando marcadores…").color(MUTED));
            }
            MarkerLoad::Failed => {
                ui.label(RichText::new("No pude descargar los marcadores. Revisa tu conexión y vuelve a abrir esta pestaña.").color(BAD));
                if button(ui, "Reintentar", Vec2::new(ui.available_width(), 28.0), true, false) {
                    self.map.marker_state = MarkerLoad::Loading;
                    self.map.marker_rx = Some(tiles::spawn_marker_download(&self.dir));
                }
            }
            MarkerLoad::Ready => {
                ui.add(egui::TextEdit::singleline(&mut self.map.marker_search).hint_text("Buscar marcador…").desired_width(ui.available_width()));
                ui.horizontal(|ui| {
                    if ui.small_button("Todos").clicked() {
                        self.map.visible_cats = self.map.markers.categories.iter().map(|c| c.id).collect();
                    }
                    if ui.small_button("Ninguno").clicked() {
                        self.map.visible_cats.clear();
                    }
                });
                let mut counts: HashMap<u64, usize> = HashMap::new();
                for m in &self.map.markers.markers {
                    *counts.entry(m.category).or_default() += 1;
                }
                let mut groups: Vec<String> = self.map.markers.categories.iter().map(|c| c.group.clone()).collect();
                groups.sort();
                groups.dedup();
                for group in groups {
                    egui::CollapsingHeader::new(RichText::new(if group.is_empty() { "Otros".to_string() } else { group.clone() }).color(group_color(&group))).id_salt(("mkgroup", group.clone())).show(ui, |ui| {
                        for c in self.map.markers.categories.iter().filter(|c| c.group == group) {
                            let mut on = self.map.visible_cats.contains(&c.id);
                            let count = counts.get(&c.id).copied().unwrap_or(0);
                            if ui.checkbox(&mut on, format!("{} ({})", c.title, count)).changed() {
                                if on {
                                    self.map.visible_cats.insert(c.id);
                                } else {
                                    self.map.visible_cats.remove(&c.id);
                                }
                            }
                        }
                    });
                }
            }
        }
    }
}

fn tiles_cell_px(view: &View) -> f32 {
    (CELL * tiles::PX_PER_METER * view.scale) as f32
}

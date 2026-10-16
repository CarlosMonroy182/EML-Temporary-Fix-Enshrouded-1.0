use eframe::egui::{
    self, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect, Rounding, Sense, Stroke, Vec2,
};

pub const GOLD: Color32 = Color32::from_rgb(214, 170, 78);
pub const GOLD_DIM: Color32 = Color32::from_rgb(150, 116, 52);
pub const BRONZE: Color32 = Color32::from_rgb(98, 72, 38);
pub const PARCHMENT: Color32 = Color32::from_rgb(234, 222, 192);
pub const MUTED: Color32 = Color32::from_rgb(160, 144, 112);
pub const BG: Color32 = Color32::from_rgb(16, 12, 9);
pub const LEATHER: Color32 = Color32::from_rgb(28, 21, 15);
pub const CARD: Color32 = Color32::from_rgb(40, 30, 21);
pub const CARD_HOVER: Color32 = Color32::from_rgb(58, 43, 28);
pub const CARD_SELECTED: Color32 = Color32::from_rgb(74, 52, 24);
pub const BLOOD: Color32 = Color32::from_rgb(150, 44, 40);
pub const GOOD: Color32 = Color32::from_rgb(132, 186, 112);
pub const BAD: Color32 = Color32::from_rgb(220, 104, 92);

pub fn title_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("Cinzel".into()))
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("Cinzel".to_owned(), FontData::from_static(include_bytes!("../assets/Cinzel.ttf")));
    fonts
        .font_data
        .insert("IMFell".to_owned(), FontData::from_static(include_bytes!("../assets/IMFell.ttf")));
    fonts.families.insert(FontFamily::Name("Cinzel".into()), vec!["Cinzel".to_owned(), "IMFell".to_owned()]);
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "IMFell".to_owned());
    ctx.set_fonts(fonts);

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = LEATHER;
    visuals.extreme_bg_color = Color32::from_rgb(10, 8, 6);
    visuals.override_text_color = Some(PARCHMENT);
    visuals.selection.bg_fill = GOLD_DIM;
    visuals.selection.stroke = Stroke::new(1.0_f32, PARCHMENT);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BRONZE);
    visuals.widgets.inactive.bg_fill = CARD;
    visuals.widgets.inactive.weak_bg_fill = CARD;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BRONZE);
    visuals.widgets.hovered.bg_fill = CARD_HOVER;
    visuals.widgets.hovered.weak_bg_fill = CARD_HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, GOLD_DIM);
    visuals.widgets.active.bg_fill = CARD_SELECTED;
    visuals.widgets.active.weak_bg_fill = CARD_SELECTED;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, GOLD);
    visuals.widgets.open.bg_fill = CARD;
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(10.0, 9.0);
    style.spacing.slider_width = 210.0;
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    for (_, font) in style.text_styles.iter_mut() {
        font.size *= 1.12;
    }
    ctx.set_style(style);
}

pub fn ornate_frame(painter: &egui::Painter, rect: Rect) {
    painter.rect_stroke(rect, Rounding::same(3.0), Stroke::new(2.0_f32, BRONZE));
    painter.rect_stroke(rect.shrink(5.0), Rounding::same(2.0), Stroke::new(1.0_f32, GOLD_DIM));
    for corner in [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()] {
        diamond(painter, corner, 5.0, GOLD);
    }
}

pub fn diamond(painter: &egui::Painter, center: Pos2, r: f32, color: Color32) {
    painter.add(egui::Shape::convex_polygon(
        vec![
            center + Vec2::new(0.0, -r),
            center + Vec2::new(r, 0.0),
            center + Vec2::new(0.0, r),
            center + Vec2::new(-r, 0.0),
        ],
        color,
        Stroke::NONE,
    ));
}

pub fn divider(ui: &mut egui::Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 12.0), Sense::hover());
    let y = rect.center().y;
    let painter = ui.painter();
    painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.center().x - 10.0, y)], Stroke::new(1.0_f32, BRONZE));
    painter.line_segment([Pos2::new(rect.center().x + 10.0, y), Pos2::new(rect.right(), y)], Stroke::new(1.0_f32, BRONZE));
    diamond(painter, rect.center(), 4.5, GOLD);
}

pub fn heading(ui: &mut egui::Ui, text: &str, size: f32) {
    ui.label(egui::RichText::new(text).font(title_font(size)).color(GOLD));
}

pub fn label_title(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(title_font(13.0)).color(GOLD_DIM));
}

pub fn button(ui: &mut egui::Ui, text: &str, size: Vec2, enabled: bool, primary: bool) -> bool {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let hovered = enabled && response.hovered();
    let fill = match (primary, hovered, enabled) {
        (_, _, false) => Color32::from_rgb(34, 28, 22),
        (true, true, _) => Color32::from_rgb(134, 38, 34),
        (true, false, _) => BLOOD,
        (false, true, _) => CARD_HOVER,
        (false, false, _) => CARD,
    };
    let border = if !enabled {
        Color32::from_rgb(60, 50, 38)
    } else if hovered {
        GOLD
    } else {
        GOLD_DIM
    };
    let painter = ui.painter();
    painter.rect_filled(rect, Rounding::same(4.0), fill);
    painter.rect_stroke(rect, Rounding::same(4.0), Stroke::new(1.5_f32, border));
    painter.rect_stroke(rect.shrink(3.0), Rounding::same(2.0), Stroke::new(1.0_f32, border.gamma_multiply(0.5)));
    let text_color = if enabled { PARCHMENT } else { Color32::from_rgb(110, 98, 80) };
    painter.text(rect.center(), Align2::CENTER_CENTER, text, title_font(if primary { 18.0 } else { 14.0 }), text_color);
    enabled && response.clicked()
}

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Skull,
    Paw,
    Spider,
    Cog,
    Eye,
    Mist,
    Helm,
    Fang,
    Wing,
    Shield,
}

pub fn kind_of(name: &str, faction: &str) -> Kind {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["skeleton", "necromancer", "bone"]) {
        Kind::Skull
    } else if has(&["spider", "ant", "scorpion", "crab", "insect", "beetle"]) {
        Kind::Spider
    } else if has(&["construct", "golem", "modular", "ancient"]) || faction == "Ancients" {
        Kind::Cog
    } else if has(&["cyclops", "eye", "watcher"]) || faction == "Cyclops" {
        Kind::Eye
    } else if has(&["banshee", "jelly", "fog", "wraith", "ghost", "floater"]) || faction == "Fogger" {
        Kind::Mist
    } else if has(&["wolf", "hound", "dog", "sabertooth", "bear", "boar", "cat", "deer", "stag", "fox"]) || faction == "WildBeast" {
        Kind::Paw
    } else if has(&["bird", "crow", "bat", "moth", "butterfly", "wing", "vulture", "owl"]) {
        Kind::Wing
    } else if has(&["drak", "lizard", "croc", "snake", "dragon", "vukah"]) || faction == "Drak" || faction == "Vukah" {
        Kind::Fang
    } else if has(&["scavenger", "soldier", "knight", "melee", "ranger", "mage", "hero", "shaman"]) || faction == "Scavenger" {
        Kind::Helm
    } else {
        Kind::Shield
    }
}

fn pt(rect: Rect, x: f32, y: f32) -> Pos2 {
    Pos2::new(rect.left() + rect.width() * x, rect.top() + rect.height() * y)
}

fn poly(painter: &Painter, rect: Rect, points: &[(f32, f32)], fill: Color32, stroke: Stroke) {
    painter.add(Shape::convex_polygon(points.iter().map(|(x, y)| pt(rect, *x, *y)).collect(), fill, stroke));
}

fn line(painter: &Painter, rect: Rect, points: &[(f32, f32)], stroke: Stroke) {
    painter.add(Shape::line(points.iter().map(|(x, y)| pt(rect, *x, *y)).collect(), stroke));
}

pub fn draw(painter: &Painter, rect: Rect, kind: Kind, tint: Color32, boss: bool) {
    let dark = Color32::from_rgb(20, 15, 11);
    let s = (rect.width() / 40.0).max(0.6);
    let thin = Stroke::new(1.4 * s, tint);
    let thick = Stroke::new(2.4 * s, tint);
    match kind {
        Kind::Skull => {
            painter.circle_filled(pt(rect, 0.5, 0.42), rect.width() * 0.32, tint);
            poly(painter, rect, &[(0.3, 0.62), (0.7, 0.62), (0.66, 0.88), (0.34, 0.88)], tint, Stroke::NONE);
            painter.circle_filled(pt(rect, 0.38, 0.45), rect.width() * 0.08, dark);
            painter.circle_filled(pt(rect, 0.62, 0.45), rect.width() * 0.08, dark);
            poly(painter, rect, &[(0.5, 0.55), (0.45, 0.65), (0.55, 0.65)], dark, Stroke::NONE);
            for x in [0.42, 0.5, 0.58] {
                line(painter, rect, &[(x, 0.76), (x, 0.88)], Stroke::new(1.2 * s, dark));
            }
        }
        Kind::Paw => {
            painter.circle_filled(pt(rect, 0.5, 0.64), rect.width() * 0.22, tint);
            painter.circle_filled(pt(rect, 0.5, 0.74), rect.width() * 0.16, tint);
            for (x, y) in [(0.24, 0.42), (0.4, 0.26), (0.6, 0.26), (0.76, 0.42)] {
                painter.circle_filled(pt(rect, x, y), rect.width() * 0.09, tint);
            }
        }
        Kind::Spider => {
            painter.circle_filled(pt(rect, 0.5, 0.58), rect.width() * 0.17, tint);
            painter.circle_filled(pt(rect, 0.5, 0.38), rect.width() * 0.1, tint);
            for (dx, ys) in [(-1.0_f32, [0.28, 0.42, 0.58, 0.74]), (1.0, [0.28, 0.42, 0.58, 0.74])] {
                for (i, y) in ys.iter().enumerate() {
                    let reach = 0.18 + 0.05 * (i % 2) as f32;
                    line(
                        painter,
                        rect,
                        &[(0.5 + dx * 0.1, 0.5), (0.5 + dx * (0.26 + reach), y - 0.12), (0.5 + dx * (0.34 + reach), *y + 0.06)],
                        thin,
                    );
                }
            }
        }
        Kind::Cog => {
            let c = pt(rect, 0.5, 0.5);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let dir = Vec2::new(a.cos(), a.sin());
                painter.line_segment([c + dir * rect.width() * 0.28, c + dir * rect.width() * 0.42], Stroke::new(5.0 * s, tint));
            }
            painter.circle_stroke(c, rect.width() * 0.27, Stroke::new(4.5 * s, tint));
            painter.circle_filled(c, rect.width() * 0.08, tint);
        }
        Kind::Eye => {
            let mut top = Vec::new();
            let mut bottom = Vec::new();
            for i in 0..=10 {
                let t = i as f32 / 10.0;
                let x = 0.08 + 0.84 * t;
                let h = (std::f32::consts::PI * t).sin() * 0.26;
                top.push((x, 0.5 - h));
                bottom.push((x, 0.5 + h));
            }
            bottom.reverse();
            top.extend(bottom);
            painter.add(Shape::closed_line(top.iter().map(|(x, y)| pt(rect, *x, *y)).collect(), thick));
            painter.circle_filled(pt(rect, 0.5, 0.5), rect.width() * 0.15, tint);
            painter.circle_filled(pt(rect, 0.5, 0.5), rect.width() * 0.06, dark);
        }
        Kind::Mist => {
            for (i, y) in [0.3, 0.5, 0.7].iter().enumerate() {
                let mut pts = Vec::new();
                for k in 0..=12 {
                    let t = k as f32 / 12.0;
                    pts.push((0.1 + 0.8 * t, y + 0.07 * (t * 9.0 + i as f32 * 1.6).sin()));
                }
                line(painter, rect, &pts, thick);
            }
        }
        Kind::Helm => {
            poly(painter, rect, &[(0.2, 0.6), (0.24, 0.32), (0.4, 0.16), (0.6, 0.16), (0.76, 0.32), (0.8, 0.6)], tint, Stroke::NONE);
            poly(painter, rect, &[(0.2, 0.6), (0.8, 0.6), (0.72, 0.88), (0.28, 0.88)], tint.gamma_multiply(0.8), Stroke::NONE);
            line(painter, rect, &[(0.26, 0.5), (0.74, 0.5)], Stroke::new(3.2 * s, dark));
            line(painter, rect, &[(0.5, 0.5), (0.5, 0.86)], Stroke::new(2.0 * s, dark));
        }
        Kind::Fang => {
            poly(painter, rect, &[(0.12, 0.5), (0.34, 0.22), (0.74, 0.22), (0.9, 0.42), (0.9, 0.58), (0.36, 0.7)], tint, Stroke::NONE);
            painter.circle_filled(pt(rect, 0.5, 0.36), rect.width() * 0.055, dark);
            for x in [0.46, 0.58, 0.7, 0.82] {
                poly(painter, rect, &[(x - 0.04, 0.58), (x + 0.04, 0.58), (x, 0.74)], PARCHMENT_TOOTH, Stroke::NONE);
            }
        }
        Kind::Wing => {
            poly(painter, rect, &[(0.5, 0.55), (0.1, 0.22), (0.14, 0.62), (0.5, 0.72)], tint, Stroke::NONE);
            poly(painter, rect, &[(0.5, 0.55), (0.9, 0.22), (0.86, 0.62), (0.5, 0.72)], tint, Stroke::NONE);
            painter.circle_filled(pt(rect, 0.5, 0.58), rect.width() * 0.07, dark);
        }
        Kind::Shield => {
            poly(painter, rect, &[(0.2, 0.18), (0.8, 0.18), (0.8, 0.54), (0.5, 0.88), (0.2, 0.54)], tint, Stroke::NONE);
            line(painter, rect, &[(0.5, 0.24), (0.5, 0.78)], Stroke::new(2.0 * s, dark));
            line(painter, rect, &[(0.26, 0.4), (0.74, 0.4)], Stroke::new(2.0 * s, dark));
        }
    }
    if boss {
        let c = pt(rect, 0.82, 0.14);
        let r = rect.width() * 0.2;
        painter.add(Shape::convex_polygon(
            vec![
                c + Vec2::new(-r, r * 0.7),
                c + Vec2::new(-r, -r * 0.5),
                c + Vec2::new(-r * 0.5, 0.0),
                c + Vec2::new(0.0, -r * 0.8),
                c + Vec2::new(r * 0.5, 0.0),
                c + Vec2::new(r, -r * 0.5),
                c + Vec2::new(r, r * 0.7),
            ],
            super::theme::GOLD,
            Stroke::NONE,
        ));
    }
}

const PARCHMENT_TOOTH: Color32 = Color32::from_rgb(236, 228, 206);

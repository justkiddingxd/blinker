//! Annotation model. Geometry is produced once as backend-neutral primitives,
//! so the on-screen preview (egui) and the export (tiny-skia) match.

use eframe::egui::{Color32, Pos2, Rect, Vec2, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Arrow,
    Line,
    Rect,
    Ellipse,
    Pen,
    Marker,
    Text,
    Pixelate,
    Counter,
    Crop,
}

impl Tool {
    pub const ALL: [Tool; 11] = [
        Tool::Select,
        Tool::Arrow,
        Tool::Line,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Pen,
        Tool::Marker,
        Tool::Text,
        Tool::Pixelate,
        Tool::Counter,
        Tool::Crop,
    ];

    pub fn key(self) -> eframe::egui::Key {
        use eframe::egui::Key;
        match self {
            Tool::Select => Key::V,
            Tool::Arrow => Key::A,
            Tool::Line => Key::L,
            Tool::Rect => Key::R,
            Tool::Ellipse => Key::O,
            Tool::Pen => Key::P,
            Tool::Marker => Key::M,
            Tool::Text => Key::T,
            Tool::Pixelate => Key::B,
            Tool::Counter => Key::N,
            Tool::Crop => Key::C,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => crate::i18n::t("выбор"),
            Tool::Arrow => crate::i18n::t("стрелка"),
            Tool::Line => crate::i18n::t("линия"),
            Tool::Rect => crate::i18n::t("прямоугольник"),
            Tool::Ellipse => crate::i18n::t("овал"),
            Tool::Pen => crate::i18n::t("карандаш"),
            Tool::Marker => crate::i18n::t("маркер"),
            Tool::Text => crate::i18n::t("текст"),
            Tool::Pixelate => crate::i18n::t("размытие (пиксели)"),
            Tool::Counter => crate::i18n::t("нумерация"),
            Tool::Crop => crate::i18n::t("обрезка"),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Tool::Select => crate::i18n::t("клик — выбрать, тяни — переместить, delete — удалить"),
            Tool::Arrow | Tool::Line => crate::i18n::t("тяни, чтобы нарисовать · shift — шаг 45°"),
            Tool::Rect | Tool::Ellipse => crate::i18n::t("тяни, чтобы нарисовать · shift — ровная фигура"),
            Tool::Pen => crate::i18n::t("рисуй от руки"),
            Tool::Marker => crate::i18n::t("выделяй текст · shift — прямая линия"),
            Tool::Text => crate::i18n::t("клик — добавить текст · enter — готово · shift+enter — новая строка"),
            Tool::Pixelate => crate::i18n::t("выдели область, которую нужно скрыть"),
            Tool::Counter => crate::i18n::t("клик — поставить номер шага"),
            Tool::Crop => crate::i18n::t("выдели область · enter — применить · esc — отмена"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Kind {
    Arrow { a: Pos2, b: Pos2 },
    /// Curved arrow: quadratic Bézier from `a` to `b` bent toward `c`.
    Curve { a: Pos2, b: Pos2, c: Pos2 },
    Line { a: Pos2, b: Pos2 },
    Rect { r: Rect },
    Ellipse { r: Rect },
    Pen { pts: Vec<Pos2> },
    Marker { pts: Vec<Pos2> },
    Text { pos: Pos2, text: String },
    Pixelate { r: Rect },
    /// Translucent fill over text, like a highlighter.
    Highlight { r: Rect },
    /// Solid black box that hides content.
    Redact { r: Rect },
    /// Text without a plate.
    Label { pos: Pos2, text: String },
    Counter { c: Pos2, n: u32 },
}

#[derive(Clone, Debug)]
pub struct Shape {
    pub id: u64,
    pub kind: Kind,
    pub color: Color32,
    pub width: f32,
}

/// Drawing primitive in image pixel space.
#[derive(Clone, Debug)]
pub enum Prim {
    /// Convex polygon.
    Fill { pts: Vec<Pos2>, color: Color32 },
    Stroke { pts: Vec<Pos2>, closed: bool, width: f32, color: Color32, round: bool },
    Circle { c: Pos2, r: f32, color: Color32 },
    /// Top-left anchored text (or centered on `pos` if `centered`).
    Text { pos: Pos2, text: String, size: f32, color: Color32, centered: bool, shadow: bool },
    Pixelate { r: Rect },
    /// Rounded filled rectangle (text plates).
    RoundFill { r: Rect, radius: f32, color: Color32 },
    /// Highlighter: multiply blend of the base image with `color`.
    Multiply { r: Rect, color: Color32 },
}

pub fn text_size(width: f32) -> f32 {
    12.0 + width * 2.5
}

/// Padding of the text plate around the glyphs.
pub fn text_pad(size: f32) -> Vec2 {
    vec2((size * 0.38).round(), (size * 0.16).round())
}

pub fn counter_radius(width: f32) -> f32 {
    11.0 + width * 1.7
}

impl Shape {
    pub fn prims(&self) -> Vec<Prim> {
        let c = self.color;
        let w = self.width;
        match &self.kind {
            Kind::Arrow { a, b } => arrow_prims(&[*a, *b], w, c),
            Kind::Curve { a, b, c: ctl } => arrow_prims(&bezier(*a, *ctl, *b), w, c),
            Kind::Line { a, b } => vec![Prim::Stroke { pts: vec![*a, *b], closed: false, width: w, color: c, round: true }],
            Kind::Rect { r } => vec![Prim::Stroke {
                pts: vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()],
                closed: true,
                width: w,
                color: c,
                round: false,
            }],
            Kind::Ellipse { r } => vec![Prim::Stroke { pts: ellipse_points(*r), closed: true, width: w, color: c, round: true }],
            Kind::Pen { pts } => vec![Prim::Stroke { pts: pts.clone(), closed: false, width: w, color: c, round: true }],
            Kind::Marker { pts } => vec![Prim::Stroke {
                pts: pts.clone(),
                closed: false,
                width: w * 3.6 + 6.0,
                color: Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 105),
                round: true,
            }],
            Kind::Text { pos, text } => {
                // Label on a solid plate in the chosen color, text in a contrasting one.
                let size = text_size(w);
                let pad = text_pad(size);
                let b = super::render::text_box(text, size);
                let plate = Rect::from_min_size(*pos, b + pad * 2.0);
                vec![
                    Prim::RoundFill { r: plate, radius: (size * 0.22).round(), color: c },
                    Prim::Text {
                        pos: *pos + pad,
                        text: text.clone(),
                        size,
                        color: contrast_on(c),
                        centered: false,
                        shadow: false,
                    },
                ]
            }
            Kind::Pixelate { r } => vec![Prim::Pixelate { r: *r }],
            Kind::Highlight { r } => vec![Prim::Multiply { r: *r, color: c }],
            Kind::Redact { r } => vec![Prim::Fill {
                pts: vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()],
                color: Color32::from_rgb(8, 8, 8),
            }],
            Kind::Label { pos, text } => vec![Prim::Text {
                pos: *pos,
                text: text.clone(),
                size: text_size(w),
                color: c,
                centered: false,
                shadow: true,
            }],
            Kind::Counter { c: center, n } => {
                let r = counter_radius(w);
                vec![
                    Prim::Circle { c: *center + vec2(0.0, r * 0.08), r: r + 1.0, color: Color32::from_black_alpha(70) },
                    Prim::Circle { c: *center, r, color: c },
                    Prim::Text {
                        pos: *center,
                        text: n.to_string(),
                        size: r * 1.15,
                        color: contrast_on(c),
                        centered: true,
                        shadow: false,
                    },
                ]
            }
        }
    }

    pub fn translate(&mut self, d: Vec2) {
        match &mut self.kind {
            Kind::Arrow { a, b } | Kind::Line { a, b } => {
                *a += d;
                *b += d;
            }
            Kind::Curve { a, b, c } => {
                *a += d;
                *b += d;
                *c += d;
            }
            Kind::Rect { r } | Kind::Ellipse { r } | Kind::Pixelate { r } | Kind::Highlight { r } | Kind::Redact { r } => {
                *r = r.translate(d)
            }
            Kind::Label { pos, .. } => *pos += d,
            Kind::Pen { pts } | Kind::Marker { pts } => pts.iter_mut().for_each(|p| *p += d),
            Kind::Text { pos, .. } => *pos += d,
            Kind::Counter { c, .. } => *c += d,
        }
    }

    /// Approximate bounds (used for selection outline); text bounds are supplied by caller.
    pub fn bounds(&self, text_size_px: impl Fn(&str, f32) -> Vec2) -> Rect {
        let pad = self.width / 2.0;
        match &self.kind {
            Kind::Arrow { a, b } | Kind::Line { a, b } => Rect::from_two_pos(*a, *b).expand(pad.max(self.width * 3.0)),
            Kind::Curve { a, b, c } => {
                let mut r = Rect::NOTHING;
                for p in bezier(*a, *c, *b) {
                    r.extend_with(p);
                }
                r.expand(self.width * 3.0)
            }
            Kind::Rect { r } | Kind::Ellipse { r } => r.expand(pad),
            Kind::Pixelate { r } | Kind::Highlight { r } | Kind::Redact { r } => *r,
            Kind::Label { pos, text } => Rect::from_min_size(*pos, text_size_px(text, text_size(self.width))),
            Kind::Pen { pts } | Kind::Marker { pts } => {
                let mut r = Rect::NOTHING;
                for p in pts {
                    r.extend_with(*p);
                }
                let extra = if matches!(self.kind, Kind::Marker { .. }) { self.width * 1.8 + 3.0 } else { pad };
                r.expand(extra)
            }
            Kind::Text { pos, text } => {
                let size = text_size(self.width);
                Rect::from_min_size(*pos, text_size_px(text, size) + text_pad(size) * 2.0)
            }
            Kind::Counter { c, .. } => Rect::from_center_size(*c, Vec2::splat(counter_radius(self.width) * 2.0)),
        }
    }

    pub fn hit(&self, p: Pos2, tol: f32, text_size_px: impl Fn(&str, f32) -> Vec2) -> bool {
        let w = self.width / 2.0 + tol;
        match &self.kind {
            Kind::Arrow { a, b } | Kind::Line { a, b } => dist_to_segment(p, *a, *b) <= w + self.width,
            Kind::Curve { a, b, c } => polyline_dist(p, &bezier(*a, *c, *b), false) <= w + self.width,
            Kind::Rect { r } => {
                let outer = r.expand(w);
                let inner = r.shrink(w);
                outer.contains(p) && !(inner.is_positive() && inner.contains(p))
            }
            Kind::Ellipse { r } => {
                let pts = ellipse_points(*r);
                polyline_dist(p, &pts, true) <= w
            }
            Kind::Pen { pts } => polyline_dist(p, pts, false) <= w,
            Kind::Marker { pts } => polyline_dist(p, pts, false) <= self.width * 1.8 + 3.0 + tol,
            Kind::Pixelate { r } | Kind::Highlight { r } | Kind::Redact { r } => r.contains(p),
            Kind::Label { .. } => self.bounds(text_size_px).expand(tol).contains(p),
            Kind::Text { .. } | Kind::Counter { .. } => self.bounds(text_size_px).expand(tol).contains(p),
        }
    }
}

/// Points of a quadratic Bézier `a → b` with control point `c`.
pub fn bezier(a: Pos2, c: Pos2, b: Pos2) -> Vec<Pos2> {
    let n = 48;
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            (a.to_vec2() * (u * u) + c.to_vec2() * (2.0 * u * t) + b.to_vec2() * (t * t)).to_pos2()
        })
        .collect()
}

/// Control point so the curve passes through `m` at its middle.
pub fn bezier_through(a: Pos2, m: Pos2, b: Pos2) -> Pos2 {
    (m.to_vec2() * 2.0 - (a.to_vec2() + b.to_vec2()) * 0.5).to_pos2()
}

/// Bold shaft along `path` ending in an open chevron ">" head, round caps.
fn arrow_prims(path: &[Pos2], w: f32, color: Color32) -> Vec<Prim> {
    let (Some(&a), Some(&b)) = (path.first(), path.last()) else { return vec![] };
    if (b - a).length() < 1.0 {
        return vec![];
    }
    let sw = w * 1.6 + 1.0;
    // Head direction: from a point a bit back along the path to the tip.
    let back = {
        let mut acc = 0.0;
        let mut q = a;
        for s in path.windows(2).rev() {
            acc += (s[1] - s[0]).length();
            q = s[0];
            if acc >= sw * 2.0 {
                break;
            }
        }
        q
    };
    let d = b - back;
    let dir = if d.length() > 0.01 { d / d.length() } else { (b - a) / (b - a).length() };
    let n = vec2(-dir.y, dir.x);
    let total: f32 = path.windows(2).map(|s| (s[1] - s[0]).length()).sum();
    let head = (sw * 2.6 + 8.0).min(total * 0.55);
    let spread = 0.62; // half-angle ~32°, tan ≈ 0.62
    let left = b - dir * head + n * head * spread;
    let right = b - dir * head - n * head * spread;
    vec![
        Prim::Stroke { pts: path.to_vec(), closed: false, width: sw, color, round: true },
        Prim::Stroke { pts: vec![left, b], closed: false, width: sw, color, round: true },
        Prim::Stroke { pts: vec![right, b], closed: false, width: sw, color, round: true },
    ]
}

pub fn ellipse_points(r: Rect) -> Vec<Pos2> {
    let n = 96;
    let c = r.center();
    let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            pos2(c.x + rx * t.cos(), c.y + ry * t.sin())
        })
        .collect()
}

pub fn contrast_on(c: Color32) -> Color32 {
    let l = 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
    if l > 170.0 { Color32::from_rgb(20, 20, 24) } else { Color32::WHITE }
}

pub fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t - p).length()
}

fn polyline_dist(p: Pos2, pts: &[Pos2], closed: bool) -> f32 {
    if pts.len() == 1 {
        return (pts[0] - p).length();
    }
    let mut best = f32::INFINITY;
    for s in pts.windows(2) {
        best = best.min(dist_to_segment(p, s[0], s[1]));
    }
    if closed && pts.len() > 2 {
        best = best.min(dist_to_segment(p, *pts.last().unwrap(), pts[0]));
    }
    best
}

/// Chaikin smoothing for freehand strokes (keeps endpoints).
pub fn smooth(pts: &[Pos2]) -> Vec<Pos2> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = Vec::with_capacity(pts.len() * 2);
    out.push(pts[0]);
    for s in pts.windows(2) {
        let (a, b) = (s[0], s[1]);
        out.push(a + (b - a) * 0.25);
        out.push(a + (b - a) * 0.75);
    }
    out.push(*pts.last().unwrap());
    out
}

pub fn snap_45(a: Pos2, b: Pos2) -> Pos2 {
    let d = b - a;
    let len = d.length();
    let ang = d.y.atan2(d.x);
    let step = std::f32::consts::FRAC_PI_4;
    let ang = (ang / step).round() * step;
    a + vec2(ang.cos(), ang.sin()) * len
}

pub fn square_from(a: Pos2, b: Pos2) -> Rect {
    let d = b - a;
    let m = d.x.abs().max(d.y.abs());
    Rect::from_two_pos(a, a + vec2(m * d.x.signum(), m * d.y.signum()))
}

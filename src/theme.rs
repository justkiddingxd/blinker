//! Visual identity "Graphite": warm dark greys, one dusty snow-white accent,
//! Fira Sans / Fira Mono embedded in the binary.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle, vec2};
use std::sync::Arc;
use tiny_skia as sk;

pub const BG: Color32 = Color32::from_rgb(0x16, 0x16, 0x15);
pub const PANEL: Color32 = Color32::from_rgb(0x1C, 0x1C, 0x1A);
pub const CANVAS: Color32 = Color32::from_rgb(0x12, 0x12, 0x11);
pub const LINE: Color32 = Color32::from_rgb(0x2B, 0x2B, 0x28);
pub const LINE_SOFT: Color32 = Color32::from_rgb(0x23, 0x23, 0x21);
/// A brighter hairline: the edge of a raised plate.
pub const LINE_HI: Color32 = Color32::from_rgb(0x3A, 0x3A, 0x36);
pub const HOVER: Color32 = Color32::from_rgb(0x23, 0x23, 0x21);
pub const ACTIVE: Color32 = Color32::from_rgb(0x2B, 0x2B, 0x28);
pub const TEXT: Color32 = Color32::from_rgb(0xE6, 0xE4, 0xDE);
pub const TEXT2: Color32 = Color32::from_rgb(0xB9, 0xB6, 0xAD);
pub const DIM: Color32 = Color32::from_rgb(0x9A, 0x97, 0x8E);
pub const FAINT: Color32 = Color32::from_rgb(0x6E, 0x6C, 0x66);
/// Descriptions under setting titles: between DIM and FAINT.
pub const FAINT_TEXT: Color32 = Color32::from_rgb(0x85, 0x83, 0x7C);
/// "Dirty snow": an off-white with a little warm grey in it.
pub const ACCENT: Color32 = Color32::from_rgb(0xD8, 0xD5, 0xCD);
pub const ACCENT_HI: Color32 = Color32::from_rgb(0xE9, 0xE6, 0xDF);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x16, 0x16, 0x15);
pub const DANGER: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);

pub static FONT_REGULAR: &[u8] = include_bytes!("../assets/fonts/FiraSans-Regular.ttf");
pub static FONT_MEDIUM: &[u8] = include_bytes!("../assets/fonts/FiraSans-Medium.ttf");
pub static FONT_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/FiraSans-SemiBold.ttf");
pub static FONT_MONO: &[u8] = include_bytes!("../assets/fonts/FiraMono-Regular.ttf");
pub static FONT_MONO_MEDIUM: &[u8] = include_bytes!("../assets/fonts/FiraMono-Medium.ttf");

pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}
pub fn mono_medium() -> FontFamily {
    FontFamily::Name("mono-medium".into())
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
pub fn font_medium(size: f32) -> FontId {
    FontId::new(size, medium())
}
pub fn font_semibold(size: f32) -> FontId {
    FontId::new(size, semibold())
}
pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

fn install_fonts(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    let fallback = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut add = |key: &str, bytes: &'static [u8], fam: FontFamily| {
        defs.font_data.insert(key.into(), Arc::new(FontData::from_static(bytes)));
        let mut list = vec![key.to_string()];
        list.extend(fallback.iter().cloned());
        defs.families.insert(fam, list);
    };
    add("fira", FONT_REGULAR, FontFamily::Proportional);
    add("fira-medium", FONT_MEDIUM, medium());
    add("fira-semibold", FONT_SEMIBOLD, semibold());
    add("fira-mono", FONT_MONO, FontFamily::Monospace);
    add("fira-mono-medium", FONT_MONO_MEDIUM, mono_medium());
    ctx.set_fonts(defs);
}

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, font(11.5)),
            (TextStyle::Body, font(13.0)),
            (TextStyle::Button, font(13.0)),
            (TextStyle::Heading, font_medium(14.0)),
            (TextStyle::Monospace, mono(11.5)),
        ]
        .into();
        s.spacing.item_spacing = vec2(6.0, 6.0);
        s.spacing.button_padding = vec2(10.0, 5.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.menu_margin = egui::Margin::same(4);
        // labels are part of the interface, not text to copy
        s.interaction.selectable_labels = false;

        let v = &mut s.visuals;
        v.dark_mode = true;
        v.panel_fill = BG;
        v.window_fill = PANEL;
        v.window_stroke = egui::Stroke::new(1.0, LINE);
        v.window_corner_radius = 3.into();
        v.menu_corner_radius = 3.into();
        v.extreme_bg_color = CANVAS;
        v.faint_bg_color = PANEL;
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = ACCENT.gamma_multiply(0.25);
        v.selection.stroke = egui::Stroke::new(1.0, ACCENT);
        v.window_shadow = egui::Shadow::NONE;
        v.popup_shadow = egui::Shadow::NONE;

        let w = &mut v.widgets;
        for wv in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            wv.corner_radius = 3.into();
            wv.expansion = 0.0;
        }
        w.noninteractive.bg_fill = PANEL;
        w.noninteractive.weak_bg_fill = PANEL;
        w.noninteractive.bg_stroke = egui::Stroke::new(1.0, LINE);
        w.noninteractive.fg_stroke = egui::Stroke::new(1.0, DIM);
        w.inactive.bg_fill = ACTIVE;
        w.inactive.weak_bg_fill = Color32::TRANSPARENT;
        w.inactive.bg_stroke = egui::Stroke::NONE;
        w.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT2);
        w.hovered.bg_fill = HOVER;
        w.hovered.weak_bg_fill = HOVER;
        w.hovered.bg_stroke = egui::Stroke::NONE;
        w.hovered.fg_stroke = egui::Stroke::new(1.0, TEXT);
        w.active.bg_fill = ACCENT;
        w.active.weak_bg_fill = ACTIVE;
        w.active.bg_stroke = egui::Stroke::NONE;
        w.active.fg_stroke = egui::Stroke::new(1.0, TEXT);
        w.open = w.hovered;

        if glass() {
            // Liquid Glass: popups, menus and tips are dark panes with a light
            // rim and round corners; rows in them are rounded too
            let pane = Color32::from_rgba_unmultiplied(30, 30, 29, 236);
            let rim = egui::Stroke::new(1.0, Color32::from_white_alpha(30));
            v.window_fill = pane;
            v.window_stroke = rim;
            v.window_corner_radius = 16.into();
            v.menu_corner_radius = 12.into();
            let shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(80) };
            v.window_shadow = shadow;
            v.popup_shadow = shadow;
            for wv in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
                wv.corner_radius = 7.into();
            }
            w.hovered.bg_fill = Color32::from_white_alpha(11);
            w.hovered.weak_bg_fill = Color32::from_white_alpha(11);
            w.open = w.hovered;
            v.selection.bg_fill = Color32::from_white_alpha(22);
            // text fields: a dark well in the glass
            v.extreme_bg_color = Color32::from_black_alpha(70);
            s.spacing.menu_margin = egui::Margin::same(6);
        }
    });
}

/// Liquid Glass panes (macOS 26) instead of flat panels.
pub fn glass() -> bool {
    #[cfg(target_os = "macos")]
    {
        static GLASS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *GLASS.get_or_init(crate::win::glass_available)
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

// ---------------------------------------------------------------------------
// Logo: a white "B" on a graphite tile, a four-point star (a blink of light)
// cut out of it, and a red dot of recording tucked into its corner. Laid out
// in tile units (0..1); the tray shows the dot only while the replay runs.

/// The B: left, top, right, bottom and its corner radius.
const B_BOX: (f32, f32, f32, f32) = (0.205, 0.225, 0.795, 0.785);
const B_R: f32 = 0.11;
/// The pinch on its right side: height, depth and half its length.
const B_WAIST: (f32, f32, f32) = (0.515, 0.04, 0.06);
/// The star cut out of it: center and radius.
const B_STAR: (f32, f32, f32) = (0.5, 0.5, 0.18);
/// The red dot's center.
const DOT_AT: (f32, f32) = (0.825, 0.21);

/// The dot's radius and the bite it takes out of the B.
#[derive(Clone, Copy)]
struct Dot {
    r: f32,
    bite: f32,
}
const DOT: Dot = Dot { r: 0.072, bite: 0.158 };

const GLYPH_TOP: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const GLYPH_BOTTOM: Color32 = Color32::from_rgb(0xE2, 0xE1, 0xDC);
const DOT_TOP: Color32 = Color32::from_rgb(0xFF, 0x4D, 0x44);
const DOT_BOTTOM: Color32 = Color32::from_rgb(0xE0, 0x24, 0x1E);

/// Circle arc from angle `a0` to `a1` (radians, y down) as cubics.
fn arc_to(pb: &mut sk::PathBuilder, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) {
    let n = ((a1 - a0).abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = (a1 - a0) / n as f32;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    for i in 0..n {
        let (s, e) = (a0 + step * i as f32, a0 + step * (i + 1) as f32);
        let (p0, p3) = ((s.cos(), s.sin()), (e.cos(), e.sin()));
        pb.cubic_to(
            cx + r * (p0.0 - k * p0.1),
            cy + r * (p0.1 + k * p0.0),
            cx + r * (p3.0 + k * p3.1),
            cy + r * (p3.1 - k * p3.0),
            cx + r * p3.0,
            cy + r * p3.1,
        );
    }
}

/// The B with the star cut out (even-odd), in tile units; `bite`: the bite
/// the dot takes out of its corner.
fn glyph_path(bite: Option<f32>) -> sk::Path {
    let (l, t, r, b) = B_BOX;
    let k = 0.552_284_8 * B_R;
    let mut pb = sk::PathBuilder::new();
    pb.move_to(l + B_R, t);
    match bite {
        Some(rn) => {
            // the top and right edges meet the bite's circle; small fillets
            let (cx, cy) = DOT_AT;
            let f = 0.022;
            let xt = cx - (rn * rn - (t - cy).powi(2)).sqrt();
            let yr = cy + (rn * rn - (r - cx).powi(2)).sqrt();
            let (a0, a1) = ((t - cy).atan2(xt - cx) - f / rn, (yr - cy).atan2(r - cx) + f / rn);
            pb.line_to(xt - f, t);
            pb.quad_to(xt, t, cx + rn * a0.cos(), cy + rn * a0.sin());
            arc_to(&mut pb, cx, cy, rn, a0, a1);
            pb.quad_to(r, yr, r, yr + f);
        }
        None => {
            pb.line_to(r - B_R, t);
            pb.cubic_to(r - B_R + k, t, r, t + B_R - k, r, t + B_R);
        }
    }
    let (wy, wd, wh) = B_WAIST;
    pb.line_to(r, wy - wh);
    pb.cubic_to(r, wy - wh * 0.45, r - wd, wy - wh * 0.35, r - wd, wy);
    pb.cubic_to(r - wd, wy + wh * 0.35, r, wy + wh * 0.45, r, wy + wh);
    pb.line_to(r, b - B_R);
    pb.cubic_to(r, b - B_R + k, r - B_R + k, b, r - B_R, b);
    pb.line_to(l + B_R, b);
    pb.cubic_to(l + B_R - k, b, l, b - B_R + k, l, b - B_R);
    pb.line_to(l, t + B_R);
    pb.cubic_to(l, t + B_R - k, l + B_R - k, t, l + B_R, t);
    pb.close();
    star(&mut pb, B_STAR.0, B_STAR.1, B_STAR.2, 0.17);
    pb.finish().expect("logo glyph")
}

/// Four points joined by curves pulled in towards the center; `plump`: how
/// far out the curves stay (0.17 slim … 0.3 full).
fn star(pb: &mut sk::PathBuilder, cx: f32, cy: f32, r: f32, plump: f32) {
    let w = r * plump;
    pb.move_to(cx, cy - r);
    pb.quad_to(cx + w, cy - w, cx + r, cy);
    pb.quad_to(cx + w, cy + w, cx, cy + r);
    pb.quad_to(cx - w, cy + w, cx - r, cy);
    pb.quad_to(cx - w, cy - w, cx, cy - r);
    pb.close();
}

fn circle_path(cx: f32, cy: f32, r: f32) -> sk::Path {
    sk::PathBuilder::from_circle(cx, cy, r).expect("circle")
}

/// A path as SVG path data, scaled by `s` (the Mac icon's layers).
#[cfg(test)]
fn svg_data(path: &sk::Path, s: f32) -> String {
    use std::fmt::Write;
    let mut d = String::new();
    let p = |v: f32| format!("{:.2}", v * s);
    for seg in path.segments() {
        let _ = match seg {
            sk::PathSegment::MoveTo(a) => write!(d, "M{} {}", p(a.x), p(a.y)),
            sk::PathSegment::LineTo(a) => write!(d, "L{} {}", p(a.x), p(a.y)),
            sk::PathSegment::QuadTo(a, b) => write!(d, "Q{} {} {} {}", p(a.x), p(a.y), p(b.x), p(b.y)),
            sk::PathSegment::CubicTo(a, b, c) => write!(d, "C{} {} {} {} {} {}", p(a.x), p(a.y), p(b.x), p(b.y), p(c.x), p(c.y)),
            sk::PathSegment::Close => write!(d, "Z"),
        };
    }
    d
}

fn solid(c: Color32, a: u8) -> sk::Paint<'static> {
    let mut p = sk::Paint::default();
    p.anti_alias = true;
    p.set_color_rgba8(c.r(), c.g(), c.b(), a);
    p
}

/// Top-to-bottom gradient between `y0` and `y1`.
fn vertical(c0: Color32, c1: Color32, y0: f32, y1: f32) -> sk::Paint<'static> {
    let mut p = solid(c0, 255);
    let stop = |o: f32, c: Color32| sk::GradientStop::new(o, sk::Color::from_rgba8(c.r(), c.g(), c.b(), c.a()));
    if let Some(sh) = sk::LinearGradient::new(
        sk::Point::from_xy(0.0, y0),
        sk::Point::from_xy(0.0, y1),
        vec![stop(0.0, c0), stop(1.0, c1)],
        sk::SpreadMode::Pad,
        sk::Transform::identity(),
    ) {
        p.shader = sh;
    }
    p
}

/// A soft shadow of `path` (drawn with `ts`), `dy` lower, blurred by `blur` px.
fn soft_shadow(pm: &mut sk::Pixmap, path: &sk::Path, rule: sk::FillRule, ts: sk::Transform, blur: f32, dy: f32, alpha: u8) {
    let Some(mut sh) = sk::Pixmap::new(pm.width(), pm.height()) else { return };
    sh.fill_path(path, &solid(Color32::BLACK, alpha), rule, ts.post_translate(0.0, dy), None);
    blur_alpha(&mut sh, blur);
    pm.draw_pixmap(0, 0, sh.as_ref(), &sk::PixmapPaint::default(), sk::Transform::identity(), None);
}

/// Three box blurs over the alpha (the pixmap holds black only), close to
/// a gaussian of about `radius`.
fn blur_alpha(pm: &mut sk::Pixmap, radius: f32) {
    let r = (radius / 2.0).round().max(1.0) as i32;
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let mut a: Vec<f32> = pm.data().chunks_exact(4).map(|p| p[3] as f32).collect();
    let mut tmp = vec![0f32; a.len()];
    let pass = |src: &[f32], dst: &mut [f32], len: i32, lines: i32, at: &dyn Fn(i32, i32) -> usize| {
        for line in 0..lines {
            let mut sum = 0.0;
            for i in -r..=r {
                sum += src[at(line, i.clamp(0, len - 1))];
            }
            for i in 0..len {
                dst[at(line, i)] = sum / (2 * r + 1) as f32;
                sum += src[at(line, (i + r + 1).min(len - 1))] - src[at(line, (i - r).max(0))];
            }
        }
    };
    for _ in 0..3 {
        pass(&a, &mut tmp, w, h, &|y, x| (y * w + x) as usize);
        pass(&tmp, &mut a, h, w, &|x, y| (y * w + x) as usize);
    }
    for (px, v) in pm.data_mut().chunks_exact_mut(4).zip(a) {
        px[..3].fill(0);
        px[3] = v.round().clamp(0.0, 255.0) as u8;
    }
}

/// How the tile looks: flat graphite with soft shadows (Windows), or glass
/// with a lit rim (the Mac, as its Liquid Glass icon looks).
#[derive(Clone, Copy, PartialEq)]
enum Finish {
    Classic,
    Glass,
}

/// The tile at `(x, y)`, `side` wide, onto `pm`, with the B and maybe the dot.
fn paint_logo(pm: &mut sk::Pixmap, x: f32, y: f32, side: f32, radius: f32, dot: Option<Dot>, finish: Finish) {
    let id = sk::Transform::identity();
    let ts = sk::Transform::from_row(side, 0.0, 0.0, side, x, y);
    let big = side >= 40.0;
    let Some(tile) = rounded_rect_path(x, y, side, side, radius) else { return };
    let (top, bottom) = match finish {
        Finish::Classic => (Color32::from_rgb(0x2C, 0x2C, 0x2B), Color32::from_rgb(0x11, 0x11, 0x11)),
        Finish::Glass => (Color32::from_rgb(0x35, 0x37, 0x3D), Color32::from_rgb(0x14, 0x15, 0x18)),
    };
    pm.fill_path(&tile, &vertical(top, bottom, y, y + side), sk::FillRule::Winding, id, None);
    // (a square tile is a full-bleed background: no edge to light)
    if big && radius > 0.0 {
        // light along the top edge; glass: a rim lit from above all round
        let (a0, a1, width) = match finish {
            Finish::Classic => (34, 0, (side / 256.0).max(1.0)),
            Finish::Glass => (150, 40, (side / 90.0).max(1.0)),
        };
        let rim = vertical(Color32::from_white_alpha(a0), Color32::from_white_alpha(a1), y, y + side * 0.6);
        if let Some(edge) = rounded_rect_path(x + width / 2.0, y + width / 2.0, side - width, side - width, radius - width / 2.0) {
            pm.stroke_path(&edge, &rim, &sk::Stroke { width, ..Default::default() }, id, None);
        }
        if finish == Finish::Glass {
            // a faint sheen over the upper half
            let sheen = vertical(Color32::from_white_alpha(16), Color32::from_white_alpha(0), y, y + side * 0.5);
            pm.fill_path(&tile, &sheen, sk::FillRule::Winding, id, None);
        }
    }
    let glyph = glyph_path(dot.map(|d| d.bite));
    let Some(glyph_px) = glyph.clone().transform(ts) else { return };
    if big {
        soft_shadow(pm, &glyph_px, sk::FillRule::EvenOdd, id, side * 0.035, side * 0.018, 120);
    }
    let (gt, gb) = (y + B_BOX.1 * side, y + B_BOX.3 * side);
    pm.fill_path(&glyph_px, &vertical(GLYPH_TOP, GLYPH_BOTTOM, gt, gb), sk::FillRule::EvenOdd, id, None);
    if let Some(d) = dot {
        let (cx, cy, r) = (x + DOT_AT.0 * side, y + DOT_AT.1 * side, d.r * side);
        let disc = circle_path(cx, cy, r);
        if big {
            soft_shadow(pm, &disc, sk::FillRule::Winding, id, side * 0.03, side * 0.012, 110);
        }
        pm.fill_path(&disc, &vertical(DOT_TOP, DOT_BOTTOM, cy - r, cy + r), sk::FillRule::Winding, id, None);
        if big {
            let glint = circle_path(cx - r * 0.32, cy - r * 0.38, r * 0.3);
            pm.fill_path(&glint, &solid(Color32::WHITE, 60), sk::FillRule::Winding, id, None);
        }
    }
}

/// The app icon (Windows, the site, the README): the tile with the dot. Big
/// sizes keep a margin for the tile's shadow.
pub fn logo(size: u32) -> image::RgbaImage {
    let s = size as f32;
    let mut pm = sk::Pixmap::new(size, size).expect("logo size");
    let m = if size >= 64 { s * 0.035 } else { 0.0 };
    let side = s - 2.0 * m;
    if m > 0.0
        && let Some(t) = rounded_rect_path(m, m, side, side, side * 0.225)
    {
        soft_shadow(&mut pm, &t, sk::FillRule::Winding, sk::Transform::identity(), m * 0.9, m * 0.35, 120);
    }
    paint_logo(&mut pm, m, m, side, side * 0.225, Some(DOT), Finish::Classic);
    pixmap_to_image(&pm)
}

/// The tray icon (the Windows tray, a Mac's menu bar): the white star on a
/// graphite tile, plain enough to read at 16 px, and a red dot in its corner
/// while the replay records. `inset`: room around the tile.
pub fn tray_icon(size: u32, live: bool, inset: f32) -> image::RgbaImage {
    let (s, id) = (size as f32, sk::Transform::identity());
    let mut pm = sk::Pixmap::new(size, size).expect("tray icon size");
    let (o, side) = (inset, s - 2.0 * inset);
    let radius = side * 0.24;
    if let Some(tile) = rounded_rect_path(o, o, side, side, radius) {
        let fill = vertical(Color32::from_rgb(0x2E, 0x2E, 0x2D), Color32::from_rgb(0x12, 0x12, 0x12), o, o + side);
        pm.fill_path(&tile, &fill, sk::FillRule::Winding, id, None);
    }
    if side >= 24.0 {
        let width = (side / 32.0).max(1.0);
        let rim = vertical(Color32::from_white_alpha(40), Color32::from_white_alpha(6), o, o + side);
        if let Some(edge) = rounded_rect_path(o + width / 2.0, o + width / 2.0, side - width, side - width, radius - width / 2.0) {
            pm.stroke_path(&edge, &rim, &sk::Stroke { width, ..Default::default() }, id, None);
        }
    }
    let mut pb = sk::PathBuilder::new();
    star(&mut pb, o + side * 0.5, o + side * 0.5, side * 0.29, 0.26);
    if let Some(p) = pb.finish() {
        pm.fill_path(&p, &vertical(GLYPH_TOP, GLYPH_BOTTOM, o + side * 0.2, o + side * 0.8), sk::FillRule::Winding, id, None);
    }
    if live {
        let (cx, cy, r) = (o + side * 0.79, o + side * 0.21, side * 0.11);
        pm.fill_path(&circle_path(cx, cy, r), &vertical(DOT_TOP, DOT_BOTTOM, cy - r, cy + r), sk::FillRule::Winding, id, None);
    }
    pixmap_to_image(&pm)
}

/// The macOS app icon: the glass tile on Apple's icon grid (a margin around
/// a rounder square, soft shadow), so it sits well next to other Dock icons.
pub fn app_icon(size: u32) -> image::RgbaImage {
    let s = size as f32;
    let mut pm = sk::Pixmap::new(size, size).expect("icon size");
    let (inset, side) = (s * 100.0 / 1024.0, s * 824.0 / 1024.0);
    let radius = side * 0.225;
    if let Some(t) = rounded_rect_path(inset, inset, side, side, radius) {
        soft_shadow(&mut pm, &t, sk::FillRule::Winding, sk::Transform::identity(), s * 0.025, s * 0.012, 110);
    }
    paint_logo(&mut pm, inset, inset, side, radius, Some(DOT), Finish::Glass);
    pixmap_to_image(&pm)
}

/// The B alone in the accent color, filling a transparent `size`² square
/// (title bars, notes).
pub fn mark_image(size: u32) -> image::RgbaImage {
    let s = size as f32;
    let (l, t, r, b) = B_BOX;
    let k = s * 0.94 / (r - l).max(b - t);
    let ts = sk::Transform::from_row(k, 0.0, 0.0, k, s / 2.0 - (l + r) / 2.0 * k, s / 2.0 - (t + b) / 2.0 * k);
    let mut pm = sk::Pixmap::new(size, size).expect("mark size");
    pm.fill_path(&glyph_path(None), &solid(ACCENT, 255), sk::FillRule::EvenOdd, ts, None);
    pixmap_to_image(&pm)
}

/// The four-point star alone in `color` (the "Clipped!" badge, the replay toast).
pub fn sparkle_image(size: u32, color: Color32) -> image::RgbaImage {
    let s = size as f32;
    let mut pm = sk::Pixmap::new(size, size).expect("mark size");
    let mut pb = sk::PathBuilder::new();
    star(&mut pb, s / 2.0, s / 2.0, s / 2.0, 0.17);
    if let Some(p) = pb.finish() {
        pm.fill_path(&p, &solid(color, 255), sk::FillRule::Winding, sk::Transform::identity(), None);
    }
    pixmap_to_image(&pm)
}

pub fn rounded_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<sk::Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let k = r * 0.552_284_8;
    let (l, t, rr, b) = (x, y, x + w, y + h);
    let mut pb = sk::PathBuilder::new();
    pb.move_to(l + r, t);
    pb.line_to(rr - r, t);
    pb.cubic_to(rr - r + k, t, rr, t + r - k, rr, t + r);
    pb.line_to(rr, b - r);
    pb.cubic_to(rr, b - r + k, rr - r + k, b, rr - r, b);
    pb.line_to(l + r, b);
    pb.cubic_to(l + r - k, b, l, b - r + k, l, b - r);
    pb.line_to(l, t + r);
    pb.cubic_to(l, t + r - k, l + r - k, t, l + r, t);
    pb.close();
    pb.finish()
}

/// Premultiplied tiny-skia pixmap -> straight-alpha RGBA image.
pub fn pixmap_to_image(pm: &sk::Pixmap) -> image::RgbaImage {
    let mut data = pm.data().to_vec();
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    image::RgbaImage::from_raw(pm.width(), pm.height(), data).expect("pixmap size")
}

pub fn icon_data(size: u32) -> Arc<egui::IconData> {
    // a Mac shows it in the Dock: the full icon on Apple's grid
    let (img, size) = if cfg!(target_os = "macos") { (app_icon(256), 256) } else { (logo(size), size) };
    Arc::new(egui::IconData { rgba: img.into_raw(), width: size, height: size })
}

pub fn color_image(img: &image::RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}


#[cfg(test)]
mod tests {
    #[test]
    fn logo_png() {
        let Ok(out) = std::env::var("BLINKER_TEST_LOGO") else { return };
        let size = std::env::var("BLINKER_TEST_LOGO_SIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(256);
        super::logo(size).save(out).unwrap();
    }

    /// The Windows icon: `BLINKER_TEST_ICO` = the .ico to write (PNG images
    /// of the logo tile, 16 to 256 px).
    #[test]
    fn windows_ico() {
        let Ok(out) = std::env::var("BLINKER_TEST_ICO") else { return };
        let sizes = [16u32, 20, 24, 32, 40, 48, 64, 96, 128, 256];
        let pngs: Vec<Vec<u8>> = sizes
            .iter()
            .map(|&s| {
                let mut v = Vec::new();
                super::logo(s).write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
                v
            })
            .collect();
        let mut ico = Vec::new();
        ico.extend_from_slice(&[0, 0, 1, 0]);
        ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * sizes.len() as u32;
        for (s, png) in sizes.iter().zip(&pngs) {
            let d = if *s >= 256 { 0 } else { *s as u8 };
            ico.extend_from_slice(&[d, d, 0, 0]);
            ico.extend_from_slice(&1u16.to_le_bytes());
            ico.extend_from_slice(&32u16.to_le_bytes());
            ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
            ico.extend_from_slice(&offset.to_le_bytes());
            offset += png.len() as u32;
        }
        for png in &pngs {
            ico.extend_from_slice(png);
        }
        std::fs::write(out, ico).unwrap();
    }

    /// The macOS icon set: `BLINKER_TEST_ICONSET` = a `.iconset` folder (then
    /// `iconutil -c icns` it).
    #[test]
    fn iconset() {
        let Ok(dir) = std::env::var("BLINKER_TEST_ICONSET") else { return };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for base in [16u32, 32, 128, 256, 512] {
            super::app_icon(base).save(dir.join(format!("icon_{base}x{base}.png"))).unwrap();
            super::app_icon(base * 2).save(dir.join(format!("icon_{base}x{base}@2x.png"))).unwrap();
        }
    }

    /// The Discord app's icon: `BLINKER_TEST_DISCORD` = the PNG (1024 px).
    /// Discord shows it at 16 px next to a name in a voice channel: the tray's
    /// big star, full bleed (Discord rounds it itself).
    #[test]
    fn discord_icon() {
        use super::{Color32, GLYPH_BOTTOM, GLYPH_TOP, sk, vertical};
        let Ok(out) = std::env::var("BLINKER_TEST_DISCORD") else { return };
        let s = 1024.0;
        let mut pm = sk::Pixmap::new(1024, 1024).unwrap();
        let bg = vertical(Color32::from_rgb(0x2E, 0x2E, 0x2D), Color32::from_rgb(0x12, 0x12, 0x12), 0.0, s);
        pm.fill_rect(sk::Rect::from_xywh(0.0, 0.0, s, s).unwrap(), &bg, sk::Transform::identity(), None);
        let mut pb = sk::PathBuilder::new();
        super::star(&mut pb, s / 2.0, s / 2.0, s * 0.36, 0.26);
        let fill = vertical(GLYPH_TOP, GLYPH_BOTTOM, s * 0.14, s * 0.86);
        pm.fill_path(&pb.finish().unwrap(), &fill, sk::FillRule::Winding, sk::Transform::identity(), None);
        super::pixmap_to_image(&pm).save(out).unwrap();
    }

    /// The big picture of the Discord status (`site/assets/discord-logo.png`):
    /// `BLINKER_TEST_DISCORD_LOGO` = the PNG. The B without the dot (the
    /// corner badge already says "recording"), full bleed and large.
    #[test]
    fn discord_logo() {
        let Ok(out) = std::env::var("BLINKER_TEST_DISCORD_LOGO") else { return };
        let s = 512.0;
        let mut pm = super::sk::Pixmap::new(512, 512).unwrap();
        super::paint_logo(&mut pm, -s * 0.1, -s * 0.1, s * 1.2, 0.0, None, super::Finish::Classic);
        super::pixmap_to_image(&pm).save(out).unwrap();
    }

    /// The same as `assets/macos/blinker.icns` without `iconutil`:
    /// `BLINKER_TEST_ICNS` = the file to write (PNG entries).
    #[test]
    fn icns() {
        let Ok(out) = std::env::var("BLINKER_TEST_ICNS") else { return };
        let mut body = Vec::new();
        for (kind, size) in [(b"icp4", 16u32), (b"icp5", 32), (b"ic11", 32), (b"ic12", 64), (b"ic07", 128), (b"ic13", 256), (b"ic08", 256), (b"ic14", 512), (b"ic09", 512), (b"ic10", 1024)] {
            let mut png = Vec::new();
            super::app_icon(size).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
            body.extend_from_slice(kind);
            body.extend_from_slice(&(png.len() as u32 + 8).to_be_bytes());
            body.extend_from_slice(&png);
        }
        let mut icns = b"icns".to_vec();
        icns.extend_from_slice(&(body.len() as u32 + 8).to_be_bytes());
        icns.extend_from_slice(&body);
        std::fs::write(out, icns).unwrap();
    }

    /// The layers of the Liquid Glass icon (`assets/macos/Blinker.icon`):
    /// `BLINKER_TEST_ICON_LAYERS` = its Assets folder.
    #[test]
    fn icon_layers() {
        let Ok(dir) = std::env::var("BLINKER_TEST_ICON_LAYERS") else { return };
        let dir = std::path::PathBuf::from(dir);
        let svg = |body: String| format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1024\" height=\"1024\" viewBox=\"0 0 1024 1024\">
  {body}
</svg>
");
        let glyph = super::svg_data(&super::glyph_path(Some(super::DOT.bite)), 1024.0);
        std::fs::write(dir.join("glyph.svg"), svg(format!("<path fill-rule=\"evenodd\" d=\"{glyph}\" fill=\"#FFFFFF\"/>"))).unwrap();
        let (x, y, r) = (super::DOT_AT.0 * 1024.0, super::DOT_AT.1 * 1024.0, super::DOT.r * 1024.0);
        std::fs::write(dir.join("dot.svg"), svg(format!("<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"{r:.1}\" fill=\"#F2352D\"/>"))).unwrap();
    }
}

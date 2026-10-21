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
    });
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

// ---------------------------------------------------------------------------
// Logo: a four-point sparkle (a blink of light) on graphite; the title bars
// show the same sparkle without the tile.

pub fn logo(size: u32) -> image::RgbaImage {
    let s = size as f32;
    let mut pm = sk::Pixmap::new(size, size).expect("logo size");
    let mut bg = sk::Paint::default();
    bg.anti_alias = true;
    bg.set_color_rgba8(0x1C, 0x1C, 0x1A, 255);
    if let Some(p) = rounded_rect_path(0.0, 0.0, s, s, s * 0.12) {
        pm.fill_path(&p, &bg, sk::FillRule::Winding, sk::Transform::identity(), None);
    }
    fill_sparkle(&mut pm, s * 0.33);
    pixmap_to_image(&pm)
}

/// The macOS app icon: the same tile on Apple's icon grid (a margin around a
/// rounder square, soft shadow), so it sits well next to other Dock icons.
pub fn app_icon(size: u32) -> image::RgbaImage {
    let s = size as f32;
    let mut pm = sk::Pixmap::new(size, size).expect("icon size");
    let (inset, side) = (s * 100.0 / 1024.0, s * 824.0 / 1024.0);
    let radius = side * 0.225;
    for (grow, alpha) in [(s * 0.012, 34u8), (s * 0.006, 40u8)] {
        let mut shadow = sk::Paint::default();
        shadow.anti_alias = true;
        shadow.set_color_rgba8(0, 0, 0, alpha);
        if let Some(p) = rounded_rect_path(inset - grow, inset - grow + s * 0.012, side + grow * 2.0, side + grow * 2.0, radius + grow) {
            pm.fill_path(&p, &shadow, sk::FillRule::Winding, sk::Transform::identity(), None);
        }
    }
    let mut bg = sk::Paint::default();
    bg.anti_alias = true;
    bg.set_color_rgba8(0x1C, 0x1C, 0x1A, 255);
    if let Some(p) = rounded_rect_path(inset, inset, side, side, radius) {
        pm.fill_path(&p, &bg, sk::FillRule::Winding, sk::Transform::identity(), None);
        let mut edge = sk::Paint::default();
        edge.anti_alias = true;
        edge.set_color_rgba8(0x2B, 0x2B, 0x28, 255);
        let st = sk::Stroke { width: (s / 512.0).max(1.0), ..Default::default() };
        pm.stroke_path(&p, &edge, &st, sk::Transform::identity(), None);
    }
    fill_sparkle(&mut pm, side * 0.33);
    pixmap_to_image(&pm)
}

/// Just the sparkle, filling a transparent `size`² square.
pub fn mark_image(size: u32) -> image::RgbaImage {
    mark_image_in(size, ACCENT)
}

/// The sparkle in another color (the red tray icon while the replay runs).
pub fn mark_image_in(size: u32, color: Color32) -> image::RgbaImage {
    let mut pm = sk::Pixmap::new(size, size).expect("mark size");
    fill_sparkle_in(&mut pm, size as f32 * 0.5, color);
    pixmap_to_image(&pm)
}

/// The logo tile with a sparkle of `color`.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn logo_in(size: u32, color: Color32) -> image::RgbaImage {
    let s = size as f32;
    let mut pm = sk::Pixmap::new(size, size).expect("logo size");
    let mut bg = sk::Paint::default();
    bg.anti_alias = true;
    bg.set_color_rgba8(0x1C, 0x1C, 0x1A, 255);
    if let Some(p) = rounded_rect_path(0.0, 0.0, s, s, s * 0.12) {
        pm.fill_path(&p, &bg, sk::FillRule::Winding, sk::Transform::identity(), None);
    }
    fill_sparkle_in(&mut pm, s * 0.33, color);
    pixmap_to_image(&pm)
}

fn fill_sparkle(pm: &mut sk::Pixmap, r: f32) {
    fill_sparkle_in(pm, r, ACCENT)
}

/// Sparkle of radius `r` in the middle of `pm`: four points joined by
/// curves pulled in towards the center.
fn fill_sparkle_in(pm: &mut sk::Pixmap, r: f32, color: Color32) {
    let (cx, cy) = (pm.width() as f32 / 2.0, pm.height() as f32 / 2.0);
    let w = r * 0.18;
    let mut pb = sk::PathBuilder::new();
    pb.move_to(cx, cy - r);
    pb.quad_to(cx + w, cy - w, cx + r, cy);
    pb.quad_to(cx + w, cy + w, cx, cy + r);
    pb.quad_to(cx - w, cy + w, cx - r, cy);
    pb.quad_to(cx - w, cy - w, cx, cy - r);
    pb.close();
    let mut accent = sk::Paint::default();
    accent.anti_alias = true;
    accent.set_color_rgba8(color.r(), color.g(), color.b(), 255);
    if let Some(p) = pb.finish() {
        pm.fill_path(&p, &accent, sk::FillRule::Winding, sk::Transform::identity(), None);
    }
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
        super::logo(256).save(out).unwrap();
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
}

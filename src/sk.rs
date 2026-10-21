//! Small CPU drawing helpers on tiny-skia pixmaps (text with ab_glyph, our
//! line icons): used by the tray menu, the recording HUD and click effects.

use crate::icons::{self, Icon};
use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use eframe::egui::Color32;
use tiny_skia as sk;

pub fn rgba(c: Color32) -> sk::Color {
    sk::Color::from_rgba8(c.r(), c.g(), c.b(), c.a())
}

pub fn paint_of(c: Color32) -> sk::Paint<'static> {
    let mut p = sk::Paint::default();
    p.anti_alias = true;
    p.set_color(rgba(c));
    p
}

pub fn blend(pm: &mut sk::Pixmap, x: i32, y: i32, c: Color32, cov: f32) {
    if x < 0 || y < 0 || x >= pm.width() as i32 || y >= pm.height() as i32 {
        return;
    }
    let a = cov.clamp(0.0, 1.0);
    let i = ((y as u32 * pm.width() + x as u32) * 4) as usize;
    let d = &mut pm.data_mut()[i..i + 4];
    let src = [c.r(), c.g(), c.b()];
    for k in 0..3 {
        d[k] = (src[k] as f32 * a + d[k] as f32 * (1.0 - a)).round() as u8;
    }
    d[3] = (255.0 * a + d[3] as f32 * (1.0 - a)).round() as u8;
}

pub fn text_width(font: &FontRef, px: f32, text: &str) -> f32 {
    let sf = font.as_scaled(PxScale::from(px));
    let mut w = 0.0;
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            w += sf.kern(p, id);
        }
        w += sf.h_advance(id);
        prev = Some(id);
    }
    w
}

pub fn draw_text(pm: &mut sk::Pixmap, font: &FontRef, px: f32, x: f32, cy: f32, text: &str, color: Color32) {
    let sf = font.as_scaled(PxScale::from(px));
    let baseline = (cy + (sf.ascent() + sf.descent()) / 2.0).round();
    let mut pen = x.round();
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            pen += sf.kern(p, id);
        }
        let g = id.with_scale_and_position(PxScale::from(px), ab_glyph::point(pen, baseline));
        pen += sf.h_advance(id);
        prev = Some(id);
        if let Some(og) = font.outline_glyph(g) {
            let b = og.px_bounds();
            let (bx, by) = (b.min.x as i32, b.min.y as i32);
            og.draw(|gx, gy, c| blend(pm, bx + gx as i32, by + gy as i32, color, c));
        }
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn draw_icon(pm: &mut sk::Pixmap, icon: Icon, cx: f32, cy: f32, size: f32, color: Color32) {
    let k = size / 16.0;
    let (ox, oy) = (cx - size / 2.0, cy - size / 2.0);
    let stroke = sk::Stroke { width: (1.4 * k).max(1.1), ..Default::default() };
    for (pts, closed) in icons::paths(icon) {
        let mut pb = sk::PathBuilder::new();
        for (i, p) in pts.iter().enumerate() {
            let (x, y) = (ox + p.x * k, oy + p.y * k);
            if i == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
        }
        if closed {
            pb.close();
        }
        if let Some(path) = pb.finish() {
            pm.stroke_path(&path, &paint_of(color), &stroke, sk::Transform::identity(), None);
        }
    }
}

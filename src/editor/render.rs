//! CPU rasterization: text (ab_glyph), pixelation and the final export (tiny-skia).

use super::doc::{Prim, Shape};
use crate::theme;
use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use eframe::egui::{Color32, Pos2, Rect, Vec2, vec2};
use image::RgbaImage;
use tiny_skia as sk;

pub struct RasterText {
    pub img: RgbaImage,
    /// Where the image's top-left goes, relative to the text anchor.
    pub offset: Vec2,
}

fn annotation_font() -> Option<FontRef<'static>> {
    FontRef::try_from_slice(theme::FONT_MEDIUM).ok()
}

pub fn line_height(size: f32) -> f32 {
    annotation_font()
        .map(|f| {
            let s = f.as_scaled(PxScale::from(size));
            s.ascent() - s.descent() + s.line_gap()
        })
        .unwrap_or(size * 1.3)
}

/// Layout box (not ink) of a text block, used for hit testing and editing.
pub fn text_box(text: &str, size: f32) -> Vec2 {
    let Some(font) = annotation_font() else { return vec2(size, size) };
    let s = font.as_scaled(PxScale::from(size));
    let lh = s.ascent() - s.descent() + s.line_gap();
    let mut w: f32 = 0.0;
    let mut lines = 0;
    for line in text.split('\n') {
        lines += 1;
        let mut x = 0.0;
        let mut prev = None;
        for ch in line.chars() {
            let id = font.glyph_id(ch);
            if let Some(p) = prev {
                x += s.kern(p, id);
            }
            x += s.h_advance(id);
            prev = Some(id);
        }
        w = w.max(x);
    }
    vec2(w.max(size * 0.3), lh * lines as f32)
}

pub fn rasterize_text(text: &str, size: f32, color: Color32, centered: bool, outline: bool) -> Option<RasterText> {
    if text.trim().is_empty() {
        return None;
    }
    let font = annotation_font()?;
    let s = font.as_scaled(PxScale::from(size));
    let lh = s.ascent() - s.descent() + s.line_gap();

    let mut glyphs = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        let mut x = 0.0;
        let mut prev = None;
        let y = s.ascent() + i as f32 * lh;
        for ch in line.chars() {
            let id = font.glyph_id(ch);
            if let Some(p) = prev {
                x += s.kern(p, id);
            }
            let g = id.with_scale_and_position(PxScale::from(size), ab_glyph::point(x, y));
            x += s.h_advance(id);
            prev = Some(id);
            if let Some(og) = font.outline_glyph(g) {
                glyphs.push(og);
            }
        }
    }
    let mut ink = Rect::NOTHING;
    for g in &glyphs {
        let b = g.px_bounds();
        ink = ink.union(Rect::from_min_max(Pos2::new(b.min.x, b.min.y), Pos2::new(b.max.x, b.max.y)));
    }
    if !ink.is_positive() {
        return None;
    }
    // `outline`: a dark rim around the glyphs so plain text reads on any background.
    let ro = if outline { (size * 0.05).round().max(1.0) as i32 } else { 0 };
    let base = if centered {
        ink
    } else {
        ink.union(Rect::from_min_size(Pos2::ZERO, text_box(text, size)))
    };
    let origin = base.min.floor() - Vec2::splat(1.0 + ro as f32);
    let w = (base.max.x - origin.x + ro as f32 + 2.0).ceil() as u32;
    let h = (base.max.y - origin.y + ro as f32 + 2.0).ceil() as u32;
    let mut cov = vec![0f32; (w * h) as usize];
    for g in &glyphs {
        let b = g.px_bounds();
        let (gx, gy) = ((b.min.x - origin.x).floor() as i32, (b.min.y - origin.y).floor() as i32);
        g.draw(|x, y, c| {
            let (px, py) = (gx + x as i32, gy + y as i32);
            if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                let i = (py as u32 * w + px as u32) as usize;
                cov[i] = (cov[i] + c).min(1.0);
            }
        });
    }
    let rim = if ro > 0 { dilate(&cov, w as usize, h as usize, ro as usize) } else { Vec::new() };
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    let a = a as f32 / 255.0;
    let mut img = RgbaImage::new(w, h);
    for i in 0..(w * h) as usize {
        let t = cov[i] * a;
        let o = if ro > 0 { rim[i] * 0.5 } else { 0.0 };
        // text over a black rim
        let out_a = t + o * (1.0 - t);
        if out_a <= 0.0 {
            continue;
        }
        let k = t / out_a;
        img.put_pixel(
            (i as u32) % w,
            (i as u32) / w,
            image::Rgba([
                (r as f32 * k).round() as u8,
                (g as f32 * k).round() as u8,
                (b as f32 * k).round() as u8,
                (out_a * 255.0).round() as u8,
            ]),
        );
    }
    let offset = if centered { origin.to_vec2() - ink.center().to_vec2() } else { origin.to_vec2() };
    Some(RasterText { img, offset })
}

/// Separable max filter (square kernel) for the text rim.
fn dilate(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut tmp = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
            tmp[y * w + x] = src[y * w + a..=y * w + b].iter().cloned().fold(0.0, f32::max);
        }
    }
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
        for x in 0..w {
            let mut m = 0f32;
            for yy in a..=b {
                m = m.max(tmp[yy * w + x]);
            }
            out[y * w + x] = m;
        }
    }
    out
}

fn clamp_region(src: &RgbaImage, r: Rect) -> Option<(u32, u32, u32, u32)> {
    let x0 = r.min.x.max(0.0).round() as u32;
    let y0 = r.min.y.max(0.0).round() as u32;
    let x1 = (r.max.x.round().max(0.0) as u32).min(src.width());
    let y1 = (r.max.y.round().max(0.0) as u32).min(src.height());
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
}

/// Per-process random seed: the frosting noise can't be predicted and undone.
fn seed() -> u32 {
    static SEED: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *SEED.get_or_init(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() ^ d.as_secs() as u32)
            .unwrap_or(0x1234_5678)
    })
}

/// Deterministic per-process noise in `-amp..=amp`.
fn noise(x: u32, y: u32, c: u32, amp: i32) -> i32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ c.wrapping_mul(0xC2B2_AE3D) ^ seed();
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h % (2 * amp as u32 + 1)) as i32 - amp
}

/// Irreversible blur. The area is averaged down to a coarse grid (the detail
/// is destroyed, not hidden), the grid gets random jitter and a Gaussian
/// blur, and is then smoothly re-expanded with a fine grain on top. Looks like
/// soft frosted glass; nothing of the original glyph shapes survives.
pub fn pixelate(src: &RgbaImage, r: Rect) -> Option<(RgbaImage, (u32, u32))> {
    let (x0, y0, w, h) = clamp_region(src, r)?;
    let cell = ((w.min(h) as f32 / 8.0).clamp(6.0, 16.0)) as u32;
    let gw = w.div_ceil(cell) as usize;
    let gh = h.div_ceil(cell) as usize;
    // 1. area average per cell + jitter
    let mut grid = vec![[0f32; 3]; gw * gh];
    for gy in 0..gh {
        for gx in 0..gw {
            let (bx, by) = (gx as u32 * cell, gy as u32 * cell);
            let (bw, bh) = (cell.min(w - bx), cell.min(h - by));
            let mut acc = [0u64; 3];
            for y in by..by + bh {
                for x in bx..bx + bw {
                    let p = src.get_pixel(x0 + x, y0 + y);
                    for c in 0..3 {
                        acc[c] += p[c] as u64;
                    }
                }
            }
            let n = (bw * bh) as f32;
            let j = noise(gx as u32, gy as u32, 7, 6) as f32;
            for c in 0..3 {
                grid[gy * gw + gx][c] = acc[c] as f32 / n + j;
            }
        }
    }
    // 2. separable Gaussian on the grid (sigma = 1.3 cells)
    let k: Vec<f32> = {
        let sigma = 1.3f32;
        let raw: Vec<f32> = (-3..=3).map(|i: i32| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
        let sum: f32 = raw.iter().sum();
        raw.iter().map(|v| v / sum).collect()
    };
    let blur = |g: &[[f32; 3]], horiz: bool| -> Vec<[f32; 3]> {
        let mut out = vec![[0f32; 3]; gw * gh];
        for y in 0..gh {
            for x in 0..gw {
                let mut acc = [0f32; 3];
                for (i, kv) in k.iter().enumerate() {
                    let o = i as i32 - 3;
                    let (sx, sy) = if horiz {
                        ((x as i32 + o).clamp(0, gw as i32 - 1) as usize, y)
                    } else {
                        (x, (y as i32 + o).clamp(0, gh as i32 - 1) as usize)
                    };
                    let v = g[sy * gw + sx];
                    for c in 0..3 {
                        acc[c] += v[c] * kv;
                    }
                }
                out[y * gw + x] = acc;
            }
        }
        out
    };
    let grid = blur(&blur(&grid, true), false);
    // 3. smooth re-expansion (bilinear with smoothstep weights) + fine grain
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let mut out = RgbaImage::new(w, h);
    for y in 0..h {
        let fy = ((y as f32 + 0.5) / cell as f32 - 0.5).clamp(0.0, (gh - 1) as f32);
        let (y_0, ty) = (fy.floor() as usize, smooth(fy.fract()));
        let y_1 = (y_0 + 1).min(gh - 1);
        for x in 0..w {
            let fx = ((x as f32 + 0.5) / cell as f32 - 0.5).clamp(0.0, (gw - 1) as f32);
            let (x_0, tx) = (fx.floor() as usize, smooth(fx.fract()));
            let x_1 = (x_0 + 1).min(gw - 1);
            let grain = noise(x0 + x, y0 + y, 3, 3) as f32;
            let mut px = [0u8; 4];
            px[3] = 255;
            for c in 0..3 {
                let a = grid[y_0 * gw + x_0][c] * (1.0 - tx) + grid[y_0 * gw + x_1][c] * tx;
                let b = grid[y_1 * gw + x_0][c] * (1.0 - tx) + grid[y_1 * gw + x_1][c] * tx;
                px[c] = (a * (1.0 - ty) + b * ty + grain).round().clamp(0.0, 255.0) as u8;
            }
            out.put_pixel(x, y, image::Rgba(px));
        }
    }
    Some((out, (x0, y0)))
}

/// Highlighter: multiplies the area with the color, so dark text stays crisp.
pub fn multiply(src: &RgbaImage, r: Rect, color: Color32) -> Option<(RgbaImage, (u32, u32))> {
    let (x0, y0, w, h) = clamp_region(src, r)?;
    let [cr, cg, cb, _] = color.to_srgba_unmultiplied();
    // Tint toward white: a highlighter, not paint.
    let tint = |c: u8| 255 - ((255 - c as u32) * 55 / 100);
    let k = [tint(cr), tint(cg), tint(cb)];
    let mut out = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let p = src.get_pixel(x0 + x, y0 + y);
            out.put_pixel(
                x,
                y,
                image::Rgba([
                    (p[0] as u32 * k[0] / 255) as u8,
                    (p[1] as u32 * k[1] / 255) as u8,
                    (p[2] as u32 * k[2] / 255) as u8,
                    255,
                ]),
            );
        }
    }
    Some((out, (x0, y0)))
}

fn to_pixmap(img: &RgbaImage) -> Option<sk::Pixmap> {
    let mut data = img.as_raw().clone();
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
    }
    sk::Pixmap::from_vec(data, sk::IntSize::from_wh(img.width(), img.height())?)
}

fn paint_of(c: Color32) -> sk::Paint<'static> {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let mut p = sk::Paint::default();
    p.set_color_rgba8(r, g, b, a);
    p.anti_alias = true;
    p
}

fn poly_path(pts: &[Pos2], close: bool) -> Option<sk::Path> {
    let mut pb = sk::PathBuilder::new();
    let first = pts.first()?;
    pb.move_to(first.x, first.y);
    for p in &pts[1..] {
        pb.line_to(p.x, p.y);
    }
    if pts.len() == 1 {
        pb.line_to(first.x + 0.01, first.y);
    }
    if close {
        pb.close();
    }
    pb.finish()
}

pub fn draw_prim(pm: &mut sk::Pixmap, base: &RgbaImage, prim: &Prim) {
    let id = sk::Transform::identity();
    match prim {
        Prim::Fill { pts, color } => {
            if let Some(path) = poly_path(pts, true) {
                pm.fill_path(&path, &paint_of(*color), sk::FillRule::Winding, id, None);
            }
        }
        Prim::Stroke { pts, closed, width, color, round } => {
            if let Some(path) = poly_path(pts, *closed) {
                let stroke = sk::Stroke {
                    width: *width,
                    line_cap: if *round { sk::LineCap::Round } else { sk::LineCap::Butt },
                    line_join: if *round { sk::LineJoin::Round } else { sk::LineJoin::Miter },
                    ..Default::default()
                };
                pm.stroke_path(&path, &paint_of(*color), &stroke, id, None);
            }
        }
        Prim::RoundFill { r, radius, color } => {
            if let Some(path) = crate::theme::rounded_rect_path(r.min.x, r.min.y, r.width(), r.height(), *radius) {
                pm.fill_path(&path, &paint_of(*color), sk::FillRule::Winding, id, None);
            }
        }
        Prim::Multiply { r, color } => {
            if let Some((img, (x, y))) = multiply(base, *r, *color)
                && let Some(tp) = to_pixmap(&img)
            {
                pm.draw_pixmap(x as i32, y as i32, tp.as_ref(), &sk::PixmapPaint::default(), id, None);
            }
        }
        Prim::Circle { c, r, color } => {
            if let Some(path) = sk::PathBuilder::from_circle(c.x, c.y, *r) {
                pm.fill_path(&path, &paint_of(*color), sk::FillRule::Winding, id, None);
            }
        }
        Prim::Text { pos, text, size, color, centered, shadow } => {
            if let Some(rt) = rasterize_text(text, *size, *color, *centered, *shadow)
                && let Some(tp) = to_pixmap(&rt.img)
            {
                let at = *pos + rt.offset;
                pm.draw_pixmap(at.x.round() as i32, at.y.round() as i32, tp.as_ref(), &sk::PixmapPaint::default(), id, None);
            }
        }
        Prim::Pixelate { r } => {
            if let Some((img, (x, y))) = pixelate(base, *r)
                && let Some(tp) = to_pixmap(&img)
            {
                pm.draw_pixmap(x as i32, y as i32, tp.as_ref(), &sk::PixmapPaint::default(), id, None);
            }
        }
    }
}

/// Flattens annotations into the image and applies the crop.
pub fn render(base: &RgbaImage, shapes: &[Shape], crop: Option<Rect>) -> RgbaImage {
    let out = if shapes.is_empty() {
        base.clone()
    } else if let Some(mut pm) = to_pixmap(base) {
        for s in shapes {
            for p in s.prims() {
                draw_prim(&mut pm, base, &p);
            }
        }
        theme::pixmap_to_image(&pm)
    } else {
        base.clone()
    };
    match crop {
        Some(c) => {
            let x = c.min.x.max(0.0) as u32;
            let y = c.min.y.max(0.0) as u32;
            let w = (c.width() as u32).min(out.width() - x).max(1);
            let h = (c.height() as u32).min(out.height() - y).max(1);
            image::imageops::crop_imm(&out, x, y, w, h).to_image()
        }
        None => out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::doc::{Kind, Shape};
    use eframe::egui::pos2;

    /// Renders every annotation kind onto a fake document; writes the PNG to
    /// `BLINKER_PREVIEW_OUT` when set (for eyeballing the look).
    #[test]
    fn render_preview_sheet() {
        let mut base = RgbaImage::from_pixel(900, 420, image::Rgba([246, 245, 241, 255]));
        for y in 150..230 {
            for x in 0..900 {
                let t = x as f32 / 900.0;
                base.put_pixel(x, y, image::Rgba([(40.0 + 180.0 * t) as u8, 90, (200.0 - 150.0 * t) as u8, 255]));
            }
        }
        let lines = [
            "Отчёт за квартал: выручка выросла на 14%",
            "Email: ivan.petrov@example.com  Тел: +7 900 123-45-67",
            "Ошибка в правом верхнем углу таблицы",
        ];
        for (i, l) in lines.iter().enumerate() {
            let rt = rasterize_text(l, 22.0, Color32::from_rgb(30, 30, 32), false, false).unwrap();
            image::imageops::overlay(&mut base, &rt.img, 40 + rt.offset.x as i64, 40 + i as i64 * 60 + rt.offset.y as i64);
        }
        let red = Color32::from_rgb(0xE5, 0x48, 0x4D);
        let yellow = Color32::from_rgb(0xE3, 0xB3, 0x41);
        let mk = |kind, color, width| Shape { id: 1, kind, color, width };
        let shapes = vec![
            mk(Kind::Highlight { r: Rect::from_min_max(pos2(36.0, 34.0), pos2(560.0, 72.0)) }, yellow, 4.0),
            mk(Kind::Pixelate { r: Rect::from_min_max(pos2(105.0, 96.0), pos2(370.0, 136.0)) }, red, 4.0),
            mk(Kind::Redact { r: Rect::from_min_max(pos2(470.0, 96.0), pos2(690.0, 136.0)) }, red, 4.0),
            mk(Kind::Arrow { a: pos2(700.0, 330.0), b: pos2(560.0, 175.0) }, red, 4.0),
            mk(Kind::Curve { a: pos2(420.0, 400.0), c: pos2(500.0, 250.0), b: pos2(640.0, 330.0) }, yellow, 4.0),
            mk(Kind::Pixelate { r: Rect::from_min_max(pos2(20.0, 160.0), pos2(380.0, 220.0)) }, red, 4.0),
            mk(Kind::Pen { pts: crate::editor::doc::smooth(&[pos2(760.0, 300.0), pos2(800.0, 260.0), pos2(850.0, 320.0), pos2(880.0, 280.0)]) }, red, 4.0),
            mk(Kind::Counter { c: pos2(740.0, 190.0), n: 1 }, red, 4.0),
            mk(Kind::Text { pos: pos2(40.0, 250.0), text: "Тут ошибка".into() }, red, 4.0),
            mk(Kind::Label { pos: pos2(40.0, 330.0), text: "Подпись без фона".into() }, yellow, 4.0),
        ];
        let out = render(&base, &shapes, None);
        if let Ok(path) = std::env::var("BLINKER_PREVIEW_OUT") {
            out.save(&path).unwrap();
            let zoom = image::imageops::crop_imm(&out, 400, 150, 500, 270).to_image();
            let zoom = image::imageops::resize(&zoom, 1000, 540, image::imageops::FilterType::Lanczos3);
            zoom.save(path.replace(".png", "-zoom.png")).unwrap();
        }
        assert_eq!(out.dimensions(), (900, 420));
    }
}

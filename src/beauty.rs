//! "Beautiful" shot for posting: the capture on a calm background with
//! padding, rounded corners and a soft shadow, optionally at a fixed aspect.

use crate::config::{Aspect, Beauty};
use image::RgbaImage;
use tiny_skia as sk;

pub struct Background {
    pub name: &'static str,
    /// One color = solid; two = soft diagonal blend. None = transparent.
    pub colors: Option<([u8; 3], [u8; 3])>,
}

pub const BACKGROUNDS: [Background; 8] = [
    Background { name: "графит", colors: Some(([0x2A, 0x2A, 0x27], [0x1A, 0x1A, 0x18])) },
    Background { name: "снег", colors: Some(([0xF2, 0xF0, 0xEB], [0xE2, 0xDF, 0xD7])) },
    Background { name: "песок", colors: Some(([0xDD, 0xCF, 0xB4], [0xC4, 0xB1, 0x8E])) },
    Background { name: "мох", colors: Some(([0x66, 0x75, 0x58], [0x3D, 0x48, 0x36])) },
    Background { name: "глина", colors: Some(([0xC0, 0x77, 0x58], [0x8C, 0x4B, 0x36])) },
    Background { name: "ночь", colors: Some(([0x2B, 0x3A, 0x48], [0x15, 0x1C, 0x24])) },
    Background { name: "туман", colors: Some(([0xBF, 0xC5, 0xCA], [0x93, 0x9B, 0xA3])) },
    Background { name: "без фона", colors: None },
];

pub fn background(b: &Beauty) -> &'static Background {
    &BACKGROUNDS[b.background.min(BACKGROUNDS.len() - 1)]
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

fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<sk::Path> {
    crate::theme::rounded_rect_path(x, y, w, h, r.min(w / 2.0).min(h / 2.0))
}

/// Three box-blur passes ≈ Gaussian, on an alpha map.
fn blur(a: &mut [f32], w: usize, h: usize, r: usize) {
    if r == 0 {
        return;
    }
    let mut tmp = vec![0f32; a.len()];
    for _ in 0..3 {
        for y in 0..h {
            let row = &a[y * w..(y + 1) * w];
            let mut sum: f32 = row[..r.min(w)].iter().sum::<f32>() + row[0] * (r + 1) as f32;
            for x in 0..w {
                let add = row[(x + r).min(w - 1)];
                let sub = row[x.saturating_sub(r + 1).min(w - 1)];
                sum += add - if x > r { sub } else { row[0] };
                tmp[y * w + x] = sum / (2 * r + 1) as f32;
            }
        }
        for x in 0..w {
            let col = |y: usize| tmp[y * w + x];
            let mut sum: f32 = (0..r.min(h)).map(col).sum::<f32>() + col(0) * (r + 1) as f32;
            for y in 0..h {
                let add = col((y + r).min(h - 1));
                let sub = if y > r { col(y - r - 1) } else { col(0) };
                sum += add - sub;
                a[y * w + x] = sum / (2 * r + 1) as f32;
            }
        }
    }
}

/// Renders `img` framed according to `b`.
pub fn render(img: &RgbaImage, b: &Beauty) -> RgbaImage {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let pad = (b.padding.clamp(0.0, 0.3) * w.max(h)).round().max(16.0);
    let (mut cw, mut ch) = (w + pad * 2.0, h + pad * 2.0);
    let ratio = match b.aspect {
        Aspect::Auto => None,
        Aspect::Wide => Some(16.0 / 9.0),
        Aspect::Square => Some(1.0),
        Aspect::Portrait => Some(4.0 / 5.0),
    };
    if let Some(r) = ratio {
        if cw / ch < r {
            cw = (ch * r).round();
        } else {
            ch = (cw / r).round();
        }
    }
    let (cw, ch) = (cw as u32, ch as u32);
    let Some(mut pm) = sk::Pixmap::new(cw, ch) else { return img.clone() };
    let id = sk::Transform::identity();

    // background
    if let Some((c1, c2)) = background(b).colors {
        let col = |c: [u8; 3]| sk::Color::from_rgba8(c[0], c[1], c[2], 255);
        let mut paint = sk::Paint::default();
        paint.shader = sk::LinearGradient::new(
            sk::Point::from_xy(0.0, 0.0),
            sk::Point::from_xy(cw as f32 * 0.6, ch as f32),
            vec![sk::GradientStop::new(0.0, col(c1)), sk::GradientStop::new(1.0, col(c2))],
            sk::SpreadMode::Pad,
            id,
        )
        .unwrap_or(sk::Shader::SolidColor(col(c1)));
        if let Some(r) = sk::Rect::from_xywh(0.0, 0.0, cw as f32, ch as f32) {
            pm.fill_rect(r, &paint, id, None);
        }
    }

    let x = ((cw as f32 - w) / 2.0).round();
    let y = ((ch as f32 - h) / 2.0).round();
    let radius = b.radius.clamp(0.0, 64.0) * (w.max(h) / 1200.0).clamp(0.6, 2.5);

    // soft shadow under the shot
    if b.shadow > 0.01 {
        let spread = (pad * 0.45).max(6.0);
        let offset = spread * 0.35;
        if let (Some(mut sp), Some(path)) = (sk::Pixmap::new(cw, ch), rounded(x, y + offset, w, h, radius)) {
            let mut paint = sk::Paint::default();
            paint.set_color(sk::Color::BLACK);
            sp.fill_path(&path, &paint, sk::FillRule::Winding, id, None);
            let mut alpha: Vec<f32> = sp.pixels().iter().map(|p| p.alpha() as f32 / 255.0).collect();
            blur(&mut alpha, cw as usize, ch as usize, (spread / 2.2) as usize);
            let k = 0.55 * b.shadow.clamp(0.0, 1.0);
            let data = pm.data_mut();
            for (i, a) in alpha.iter().enumerate() {
                let s = a * k;
                if s <= 0.001 {
                    continue;
                }
                let px = &mut data[i * 4..i * 4 + 4];
                for c in &mut px[..3] {
                    *c = (*c as f32 * (1.0 - s)) as u8;
                }
                px[3] = (px[3] as f32 + (255.0 - px[3] as f32) * s) as u8;
            }
        }
    }

    // the shot, clipped to its rounded rect
    if let (Some(src), Some(path)) = (to_pixmap(img), rounded(x, y, w, h, radius)) {
        let mut paint = sk::Paint::default();
        paint.anti_alias = true;
        paint.shader = sk::Pattern::new(src.as_ref(), sk::SpreadMode::Pad, sk::FilterQuality::Nearest, 1.0, sk::Transform::from_translate(x, y));
        pm.fill_path(&path, &paint, sk::FillRule::Winding, id, None);
        // hairline so a dark shot doesn't melt into a dark background
        let mut line = sk::Paint::default();
        line.anti_alias = true;
        line.set_color(sk::Color::from_rgba8(255, 255, 255, 22));
        pm.stroke_path(&path, &line, &sk::Stroke { width: 1.0, ..Default::default() }, id, None);
    }

    let mut out = RgbaImage::new(cw, ch);
    for (o, p) in out.pixels_mut().zip(pm.pixels()) {
        let c = p.demultiply();
        *o = image::Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    out
}

#[cfg(test)]
mod tests {
    /// Writes sample framings: `BLINKER_TEST_BEAUTY` = output folder.
    #[test]
    fn beauty_sheet() {
        let Ok(dir) = std::env::var("BLINKER_TEST_BEAUTY") else { return };
        let dir = std::path::PathBuf::from(dir);
        let mut shot = image::RgbaImage::from_pixel(900, 560, image::Rgba([250, 250, 248, 255]));
        for y in 0..48 {
            for x in 0..900 {
                shot.put_pixel(x, y, image::Rgba([36, 36, 40, 255]));
            }
        }
        for (i, line) in ["Отчёт за квартал", "Выручка выросла на 14%", "Клиенты: 1 240"].iter().enumerate() {
            if let Some(rt) = crate::editor::render::rasterize_text(line, 30.0, eframe::egui::Color32::from_rgb(30, 30, 32), false, false) {
                image::imageops::overlay(&mut shot, &rt.img, 60 + rt.offset.x as i64, 110 + i as i64 * 70 + rt.offset.y as i64);
            }
        }
        for (k, (bg, aspect)) in [(0usize, crate::config::Aspect::Auto), (2, crate::config::Aspect::Wide), (5, crate::config::Aspect::Square)].iter().enumerate() {
            let b = crate::config::Beauty { background: *bg, aspect: *aspect, ..Default::default() };
            super::render(&shot, &b).save(dir.join(format!("beauty-{k}.png"))).unwrap();
        }
    }
}

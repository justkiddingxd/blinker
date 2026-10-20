//! On-screen (egui) painting of annotation primitives, shared by the editor
//! and the capture overlay. Text and pixelation are rasterized on the CPU with
//! the same code as the export, then cached as textures.

use super::doc::Prim;
use super::render;
use crate::theme;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, TextureHandle, TextureOptions, Vec2, pos2, vec2};
use image::RgbaImage;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

fn hash_of(v: impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

/// Maps image pixels to screen points: `offset + p * zoom`.
#[derive(Clone, Copy)]
pub struct View {
    pub offset: Vec2,
    pub zoom: f32,
}

impl View {
    pub fn pt(&self, p: Pos2) -> Pos2 {
        (self.offset + p.to_vec2() * self.zoom).to_pos2()
    }
    pub fn rect(&self, r: Rect) -> Rect {
        Rect::from_min_max(self.pt(r.min), self.pt(r.max))
    }
}

#[derive(Default)]
pub struct PrimPainter {
    text: HashMap<u64, (TextureHandle, Vec2, Vec2)>,
    pix: HashMap<u64, (TextureHandle, Rect)>,
    used_text: HashSet<u64>,
    used_pix: HashSet<u64>,
}

impl PrimPainter {
    /// Drops textures that weren't used since the previous call.
    pub fn end_frame(&mut self) {
        let (ut, up) = (std::mem::take(&mut self.used_text), std::mem::take(&mut self.used_pix));
        self.text.retain(|k, _| ut.contains(k));
        self.pix.retain(|k, _| up.contains(k));
    }

    pub fn paint(&mut self, painter: &egui::Painter, ctx: &egui::Context, base: &RgbaImage, p: &Prim, v: View) {
        let z = v.zoom;
        match p {
            Prim::Fill { pts, color } => {
                let pts = pts.iter().map(|p| v.pt(*p)).collect();
                painter.add(egui::Shape::convex_polygon(pts, *color, Stroke::NONE));
            }
            Prim::Stroke { pts, closed, width, color, round } => {
                let sp: Vec<Pos2> = pts.iter().map(|p| v.pt(*p)).collect();
                let w = width * z;
                if sp.len() == 1 || (sp.len() == 2 && sp[0] == sp[1]) {
                    painter.circle_filled(sp[0], w / 2.0, *color);
                    return;
                }
                let opaque = color.a() == 255;
                if *closed {
                    painter.add(egui::Shape::closed_line(sp, Stroke::new(w, *color)));
                } else {
                    let (first, last) = (sp[0], *sp.last().unwrap());
                    painter.add(egui::Shape::line(sp, Stroke::new(w, *color)));
                    if *round && opaque {
                        painter.circle_filled(first, w / 2.0, *color);
                        painter.circle_filled(last, w / 2.0, *color);
                    }
                }
            }
            Prim::Circle { c, r, color } => {
                painter.circle_filled(v.pt(*c), r * z, *color);
            }
            Prim::RoundFill { r, radius, color } => {
                painter.rect_filled(v.rect(*r), radius * z, *color);
            }
            Prim::Multiply { r, color } => {
                let rr = Rect::from_min_max(r.min.round(), r.max.round());
                let key = hash_of(("mul", rr.min.x as i32, rr.min.y as i32, rr.max.x as i32, rr.max.y as i32, color.to_array()));
                self.used_pix.insert(key);
                if !self.pix.contains_key(&key)
                    && let Some((img, (x, y))) = render::multiply(base, rr, *color)
                {
                    let at = Rect::from_min_size(pos2(x as f32, y as f32), vec2(img.width() as f32, img.height() as f32));
                    let tex = ctx.load_texture("multiply", theme::color_image(&img), TextureOptions::NEAREST);
                    self.pix.insert(key, (tex, at));
                }
                if let Some((tex, at)) = self.pix.get(&key) {
                    painter.image(tex.id(), v.rect(*at), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                }
            }
            Prim::Text { pos, text, size, color, centered, shadow } => {
                let key = hash_of((text, size.to_bits(), color.to_array(), centered, shadow));
                self.used_text.insert(key);
                if !self.text.contains_key(&key)
                    && let Some(rt) = render::rasterize_text(text, *size, *color, *centered, *shadow)
                {
                    let size = vec2(rt.img.width() as f32, rt.img.height() as f32);
                    let tex = ctx.load_texture("text", theme::color_image(&rt.img), TextureOptions::LINEAR);
                    self.text.insert(key, (tex, rt.offset, size));
                }
                if let Some((tex, off, size)) = self.text.get(&key) {
                    let min = *pos + *off;
                    let r = v.rect(Rect::from_min_size(min.round(), *size));
                    painter.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                }
            }
            Prim::Pixelate { r } => {
                let rr = Rect::from_min_max(r.min.round(), r.max.round());
                let key = hash_of((rr.min.x as i32, rr.min.y as i32, rr.max.x as i32, rr.max.y as i32));
                self.used_pix.insert(key);
                if !self.pix.contains_key(&key)
                    && let Some((img, (x, y))) = render::pixelate(base, rr)
                {
                    let at = Rect::from_min_size(pos2(x as f32, y as f32), vec2(img.width() as f32, img.height() as f32));
                    let tex = ctx.load_texture("pixelate", theme::color_image(&img), TextureOptions::LINEAR);
                    self.pix.insert(key, (tex, at));
                }
                if let Some((tex, at)) = self.pix.get(&key) {
                    painter.image(tex.id(), v.rect(*at), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                }
            }
        }
    }
}

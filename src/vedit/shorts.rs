//! Clips for Shorts, Reels and TikTok: a vertical 9:16 frame (a slice of the
//! picture, or all of it over a blurred copy), and subtitles burnt in.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};

/// The shape of the saved video.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Aspect {
    /// As recorded.
    #[default]
    Original,
    /// 9:16, a slice of the picture (where it is: `crop_x`).
    Crop,
    /// 9:16, the whole picture across the middle over a blurred copy of it.
    Fit,
}

impl Aspect {
    pub fn vertical(self) -> bool {
        self != Aspect::Original
    }
}

/// The size of a vertical clip.
pub const VERTICAL: (u32, u32) = (1080, 1920);

/// A line of subtitles on screen from `from` to `to` (source time).
#[derive(Clone, Debug)]
pub struct Caption {
    pub from: i64,
    pub to: i64,
    pub text: String,
}

/// The caption on screen at `t`.
#[cfg(test)]
pub fn caption_at(captions: &[Caption], t: i64) -> Option<&Caption> {
    captions.iter().filter(|c| t >= c.from && t < c.to && !c.text.trim().is_empty()).max_by_key(|c| c.from)
}

/// The part of a `w`×`h` picture a 9:16 slice takes, as (x, width) in pixels;
/// `at` 0 = the left edge … 1 = the right.
pub fn slice(w: u32, h: u32, at: f32) -> (u32, u32) {
    let cw = ((h as f32 * 9.0 / 16.0).round() as u32).min(w).max(1);
    let x = ((w - cw) as f32 * at.clamp(0.0, 1.0)).round() as u32;
    (x, cw)
}

/// The picture zoomed in: how much, where the view's middle is (fractions of
/// the picture), and "push": a slow zoom in over the whole clip.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Transform {
    pub zoom: f32,
    pub x: f32,
    pub y: f32,
    pub push: bool,
}

impl Default for Transform {
    fn default() -> Self {
        Transform { zoom: 1.0, x: 0.5, y: 0.5, push: false }
    }
}

impl Transform {
    /// The zoom at `progress` (0 = the clip's start … 1 = its end).
    pub fn zoom_at(&self, progress: f32) -> f32 {
        if self.push {
            1.0 + (self.zoom.max(1.15) - 1.0) * progress.clamp(0.0, 1.0)
        } else {
            self.zoom.max(1.0)
        }
    }

    /// The part of a picture in view, as fractions (x, y, w, h).
    pub fn view(&self, progress: f32) -> (f32, f32, f32, f32) {
        let z = self.zoom_at(progress);
        let (vw, vh) = (1.0 / z, 1.0 / z);
        let x = (self.x - vw / 2.0).clamp(0.0, 1.0 - vw);
        let y = (self.y - vh / 2.0).clamp(0.0, 1.0 - vh);
        (x, y, vw, vh)
    }

    pub fn is_identity(&self) -> bool {
        !self.push && self.zoom <= 1.001
    }
}

/// `src` zoomed in by `t` at `progress`, at its own size.
pub fn zoomed<'a>(src: &'a RgbaImage, t: &Transform, progress: f32) -> std::borrow::Cow<'a, RgbaImage> {
    if t.is_identity() {
        return std::borrow::Cow::Borrowed(src);
    }
    let (w, h) = (src.width() as f32, src.height() as f32);
    let (x, y, vw, vh) = t.view(progress);
    let region = ((x * w) as u32, (y * h) as u32, ((vw * w) as u32).max(2), ((vh * h) as u32).max(2));
    std::borrow::Cow::Owned(scale(src, region, src.width(), src.height()))
}

/// The frame as it is saved: `src` turned vertical (or kept).
pub fn compose(src: &RgbaImage, aspect: Aspect, crop_x: f32) -> RgbaImage {
    let (w, h) = VERTICAL;
    let full = (0, 0, src.width(), src.height());
    match aspect {
        Aspect::Original => src.clone(),
        Aspect::Crop => {
            let (x, cw) = slice(src.width(), src.height(), crop_x);
            scale(src, (x, 0, cw, src.height()), w, h)
        }
        Aspect::Fit => {
            // behind: the middle slice, small, blurred and dimmed, back up
            let (x, cw) = slice(src.width(), src.height(), 0.5);
            let small = scale(src, (x, 0, cw, src.height()), w / 12, h / 12);
            let mut small = image::imageops::blur(&small, 2.5);
            for p in small.pixels_mut() {
                for c in &mut p.0[..3] {
                    *c = (*c as f32 * 0.55) as u8;
                }
                p.0[3] = 255;
            }
            let mut bg = scale(&small, (0, 0, small.width(), small.height()), w, h);
            // in front: the whole picture across the middle
            let fh = ((w as f32 * src.height() as f32 / src.width().max(1) as f32).round() as u32).clamp(2, h);
            let front = scale(src, full, w, fh);
            image::imageops::replace(&mut bg, &front, 0, ((h - fh) / 2) as i64);
            bg
        }
    }
}

/// Bilinear scale of the part (x, y, w, h) of `src` to `dw`×`dh`, the rows
/// shared out among the cores (a frame of a clip at a time: it has to be quick).
fn scale(src: &RgbaImage, (sx, sy, sw, sh): (u32, u32, u32, u32), dw: u32, dh: u32) -> RgbaImage {
    let mut out = RgbaImage::new(dw.max(1), dh.max(1));
    let (sw, sh) = (sw.max(1), sh.max(1));
    // each column: its two source columns and the second one's weight (of 256)
    let cols: Vec<(usize, usize, u32)> = (0..dw)
        .map(|x| {
            let f = ((x as f32 + 0.5) * sw as f32 / dw as f32 - 0.5).clamp(0.0, (sw - 1) as f32);
            let i = f.floor() as u32;
            (((sx + i) * 4) as usize, ((sx + (i + 1).min(sw - 1)) * 4) as usize, ((f - i as f32) * 256.0) as u32)
        })
        .collect();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let stride = dw as usize * 4;
    let rows_per = (dh as usize).div_ceil(threads).max(1);
    let (raw, src_stride) = (src.as_raw(), src.width() as usize * 4);
    let buf: &mut [u8] = &mut out;
    std::thread::scope(|s| {
        for (n, chunk) in buf.chunks_mut(rows_per * stride).enumerate() {
            let cols = &cols;
            s.spawn(move || {
                for (r, row) in chunk.chunks_exact_mut(stride).enumerate() {
                    let y = n * rows_per + r;
                    let f = ((y as f32 + 0.5) * sh as f32 / dh as f32 - 0.5).clamp(0.0, (sh - 1) as f32);
                    let i = f.floor() as u32;
                    let wy = ((f - i as f32) * 256.0) as u32;
                    let r0 = &raw[(sy + i) as usize * src_stride..];
                    let r1 = &raw[(sy + (i + 1).min(sh - 1)) as usize * src_stride..];
                    for (px, &(a, b, wx)) in row.chunks_exact_mut(4).zip(cols) {
                        for c in 0..4 {
                            let top = r0[a + c] as u32 * (256 - wx) + r0[b + c] as u32 * wx;
                            let bot = r1[a + c] as u32 * (256 - wx) + r1[b + c] as u32 * wx;
                            px[c] = ((top * (256 - wy) + bot * wy + 32768) >> 16) as u8;
                        }
                    }
                }
            });
        }
    });
    out
}

/// The typeface of subtitles.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CapFont {
    Regular,
    #[default]
    Bold,
    Mono,
}

impl CapFont {
    pub const ALL: [CapFont; 3] = [CapFont::Regular, CapFont::Bold, CapFont::Mono];

    pub fn label(self) -> &'static str {
        match self {
            CapFont::Regular => crate::i18n::l("обычный", "regular"),
            CapFont::Bold => crate::i18n::l("жирный", "bold"),
            CapFont::Mono => crate::i18n::l("моно", "mono"),
        }
    }

    pub fn bytes(self) -> &'static [u8] {
        match self {
            CapFont::Regular => crate::theme::FONT_MEDIUM,
            CapFont::Bold => crate::theme::FONT_SEMIBOLD,
            CapFont::Mono => crate::theme::FONT_MONO_MEDIUM,
        }
    }
}

/// What is behind the letters.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CapBack {
    /// A dark rounded plate.
    #[default]
    Plate,
    /// A dark outline round each letter (the TikTok look).
    Outline,
    /// A soft shadow only.
    Shadow,
}

impl CapBack {
    pub const ALL: [CapBack; 3] = [CapBack::Plate, CapBack::Outline, CapBack::Shadow];

    pub fn label(self) -> &'static str {
        match self {
            CapBack::Plate => crate::i18n::l("плашка", "plate"),
            CapBack::Outline => crate::i18n::l("обводка", "outline"),
            CapBack::Shadow => crate::i18n::l("тень", "shadow"),
        }
    }
}

/// How subtitles look (one style for the whole clip, remembered).
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CaptionStyle {
    /// Size against the usual one (1.0).
    pub size: f32,
    /// Where the middle of the text is, as a share of the height; None: the
    /// usual place (lower third, higher on a vertical clip).
    pub y: Option<f32>,
    pub font: CapFont,
    pub color: [u8; 3],
    pub back: CapBack,
    pub upper: bool,
    /// How each comes on.
    pub anim: Anim,
}

impl Default for CaptionStyle {
    fn default() -> Self {
        CaptionStyle { size: 1.0, y: None, font: CapFont::Bold, color: [255, 255, 255], back: CapBack::Plate, upper: false, anim: Anim::None }
    }
}

/// How a text comes on (and, for some, goes off).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Anim {
    #[default]
    None,
    /// Fades in and out.
    Fade,
    /// Pops in from small with a little overshoot.
    Pop,
    /// Slides up into place.
    Slide,
    /// Typed out letter by letter.
    Type,
}

impl Anim {
    pub const ALL: [Anim; 5] = [Anim::None, Anim::Fade, Anim::Pop, Anim::Slide, Anim::Type];

    pub fn label(self) -> &'static str {
        match self {
            Anim::None => crate::i18n::l("нет", "none"),
            Anim::Fade => crate::i18n::l("проявление", "fade"),
            Anim::Pop => crate::i18n::l("всплытие", "pop"),
            Anim::Slide => crate::i18n::l("снизу", "slide up"),
            Anim::Type => crate::i18n::l("печать", "typing"),
        }
    }
}

/// A text at moment `t` of its `from..to` life: how opaque, how big, how far
/// below its place (share of the text size), how much of it typed (0..1).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
    pub alpha: f32,
    pub scale: f32,
    pub dy: f32,
    pub typed: f32,
}

impl Look {
    pub const STILL: Look = Look { alpha: 1.0, scale: 1.0, dy: 0.0, typed: 1.0 };
}

/// `anim` at `t` (all 100 ns units).
pub fn look(anim: Anim, from: i64, to: i64, t: i64, chars: usize) -> Look {
    const HNS: f32 = 10_000_000.0;
    let age = (t - from).max(0) as f32 / HNS;
    let left = (to - t).max(0) as f32 / HNS;
    let ease = |x: f32| 1.0 - (1.0 - x.clamp(0.0, 1.0)).powi(3);
    let back = |x: f32| {
        let x = x.clamp(0.0, 1.0) - 1.0;
        1.0 + 2.70158 * x.powi(3) + 1.70158 * x.powi(2)
    };
    let fin = ease(age / 0.35);
    let fout = ease(left / 0.2);
    match anim {
        Anim::None => Look::STILL,
        Anim::Fade => Look { alpha: fin * fout, ..Look::STILL },
        Anim::Pop => Look { alpha: (age / 0.12).clamp(0.0, 1.0) * fout, scale: 0.6 + 0.4 * back(age / 0.35), ..Look::STILL },
        Anim::Slide => Look { alpha: fin * fout, dy: (1.0 - fin) * 1.2, ..Look::STILL },
        Anim::Type => {
            let need = (chars as f32 * 0.045).clamp(0.3, 1.6);
            Look { typed: (age / need).clamp(0.0, 1.0), ..Look::STILL }
        }
    }
}

/// The first `typed` share of `text`'s letters.
pub fn typed_part(text: &str, typed: f32) -> String {
    if typed >= 1.0 {
        return text.to_string();
    }
    let n = text.chars().count();
    text.chars().take(((n as f32 * typed).ceil() as usize).min(n)).collect()
}

/// A text laid over the video on its own track (a title, a callout): where
/// its middle is (fractions of the frame), how it looks and comes on.
#[derive(Clone, Debug)]
pub struct Title {
    pub from: i64,
    pub to: i64,
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub style: CaptionStyle,
}

impl Title {
    pub fn new(from: i64, to: i64) -> Title {
        Title {
            from,
            to,
            text: String::new(),
            x: 0.5,
            y: 0.3,
            style: CaptionStyle { size: 1.6, back: CapBack::Outline, anim: Anim::Pop, upper: true, ..CaptionStyle::default() },
        }
    }
}

/// The text colors to pick from.
pub const CAPTION_COLORS: [[u8; 3]; 5] = [[255, 255, 255], [255, 225, 77], [120, 230, 140], [110, 200, 255], [255, 120, 150]];

impl CaptionStyle {
    /// The middle of the text, as a share of the height.
    pub fn center_y(&self, vertical: bool) -> f32 {
        self.y.unwrap_or(if vertical { 0.70 } else { 0.86 }).clamp(0.05, 0.95)
    }

    /// Text height for a frame of `w`×`h` (bigger on a phone-shaped one).
    pub fn text_size(&self, w: u32, h: u32) -> f32 {
        w.min(h) as f32 * if h > w { 0.064 } else { 0.05 } * self.size.clamp(0.5, 2.5)
    }

    pub fn text(&self, s: &str) -> String {
        let s = s.trim();
        if self.upper { s.to_uppercase() } else { s.to_string() }
    }
}

/// A subtitle drawn once (straight alpha), laid over every frame it shows on.
pub struct Layer {
    pub x: i64,
    pub y: i64,
    pub img: RgbaImage,
}

/// The lines `text` breaks into at `max_w` pixels wide.
fn wrap(font: &ab_glyph::PxScaleFont<&FontRef<'_>>, text: &str, max_w: f32) -> Vec<String> {
    let width = |s: &str| line_width(font, s);
    let mut lines = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let next = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if width(&next) > max_w && !line.is_empty() {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = next;
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    lines
}

fn line_width(font: &ab_glyph::PxScaleFont<&FontRef<'_>>, s: &str) -> f32 {
    let mut x = 0.0;
    let mut prev = None;
    for ch in s.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            x += font.kern(p, id);
        }
        x += font.h_advance(id);
        prev = Some(id);
    }
    x
}

/// `text` as it goes on a frame of `w`×`h`: the layer and where it sits.
#[cfg(test)]
pub fn caption_layer(w: u32, h: u32, text: &str, vertical: bool, style: &CaptionStyle) -> Option<Layer> {
    text_layer(w, h, text, (0.5, style.center_y(vertical)), style)
}

/// `text` in `style` for a frame of `w`×`h`, its middle at `center`
/// (fractions of the frame).
pub fn text_layer(w: u32, h: u32, text: &str, center: (f32, f32), style: &CaptionStyle) -> Option<Layer> {
    let font = FontRef::try_from_slice(style.font.bytes()).ok()?;
    let size = style.text_size(w, h);
    let sf = font.as_scaled(PxScale::from(size));
    let lines = wrap(&sf, &style.text(text), w as f32 * 0.84);
    if lines.is_empty() {
        return None;
    }
    let line_h = size * 1.22;
    let widths: Vec<f32> = lines.iter().map(|l| line_width(&sf, l)).collect();
    let block_w = widths.iter().cloned().fold(0.0, f32::max);
    let block_h = line_h * lines.len() as f32;
    let (pad_x, pad_y) = match style.back {
        CapBack::Plate => (size * 0.55, size * 0.3),
        _ => (size * 0.25, size * 0.2),
    };
    let lw = (block_w + 2.0 * pad_x).ceil() as u32 + 4;
    let lh = (block_h + 2.0 * pad_y).ceil() as u32 + 4;
    let cy = h as f32 * center.1;
    let x = (w as f32 * center.0 - lw as f32 / 2.0).round() as i64;
    let y = (cy - lh as f32 / 2.0).round() as i64;
    let mut img = RgbaImage::new(lw, lh);
    if style.back == CapBack::Plate {
        fill_round(&mut img, (2.0, 2.0, lw as f32 - 4.0, lh as f32 - 4.0), size * 0.35, [12, 12, 11], 0.72);
    }
    // the letters' coverage, then the dark round them, then the color
    let mut cover = vec![0f32; (lw * lh) as usize];
    for (i, line) in lines.iter().enumerate() {
        let mut gx = lw as f32 / 2.0 - widths[i] / 2.0;
        let base = 2.0 + pad_y + line_h * i as f32 + (line_h + sf.ascent() + sf.descent()) / 2.0;
        let mut prev = None;
        for ch in line.chars() {
            let id = sf.glyph_id(ch);
            if let Some(p) = prev {
                gx += sf.kern(p, id);
            }
            prev = Some(id);
            let g = id.with_scale_and_position(PxScale::from(size), ab_glyph::point(gx, base));
            gx += sf.h_advance(id);
            if let Some(o) = font.outline_glyph(g) {
                let b = o.px_bounds();
                o.draw(|ox, oy, c| {
                    let (px, py) = (b.min.x as i64 + ox as i64, b.min.y as i64 + oy as i64);
                    if px >= 0 && py >= 0 && (px as u32) < lw && (py as u32) < lh {
                        let v = &mut cover[(py as u32 * lw + px as u32) as usize];
                        *v = (*v + c).min(1.0);
                    }
                });
            }
        }
    }
    let dark = match style.back {
        CapBack::Outline => Some(((size * 0.09).max(1.5), 0.0, 1.0)),
        CapBack::Shadow => Some(((size * 0.06).max(1.0), size * 0.05, 0.6)),
        CapBack::Plate => None,
    };
    if let Some((r, dy, a)) = dark {
        let edge = spread(&cover, lw, lh, r, dy);
        for (i, e) in edge.iter().enumerate() {
            if *e > 0.0 {
                over(img.get_pixel_mut(i as u32 % lw, i as u32 / lw), [8, 8, 8], e * a);
            }
        }
    }
    for (i, c) in cover.iter().enumerate() {
        if *c > 0.0 {
            over(img.get_pixel_mut(i as u32 % lw, i as u32 / lw), style.color, *c);
        }
    }
    Some(Layer { x, y, img })
}

/// The coverage grown by `r` pixels all round (and moved `dy` down): an
/// outline or a shadow under the letters.
fn spread(cover: &[f32], w: u32, h: u32, r: f32, dy: f32) -> Vec<f32> {
    let (w, h) = (w as i64, h as i64);
    let ri = r.ceil() as i64;
    let offsets: Vec<(i64, i64, f32)> = (-ri..=ri)
        .flat_map(|oy| (-ri..=ri).map(move |ox| (ox, oy)))
        .filter_map(|(ox, oy)| {
            let d = ((ox * ox + oy * oy) as f32).sqrt();
            (d <= r + 0.5).then_some((ox, oy, (r + 0.5 - d).clamp(0.0, 1.0)))
        })
        .collect();
    let shift = dy.round() as i64;
    let mut out = vec![0f32; cover.len()];
    for y in 0..h {
        for x in 0..w {
            let c = cover[(y * w + x) as usize];
            if c <= 0.0 {
                continue;
            }
            for &(ox, oy, k) in &offsets {
                let (nx, ny) = (x + ox, y + oy + shift);
                if nx >= 0 && ny >= 0 && nx < w && ny < h {
                    let v = &mut out[(ny * w + nx) as usize];
                    *v = v.max(c * k);
                }
            }
        }
    }
    out
}

/// Paints `color` at `a` over a straight-alpha pixel.
fn over(p: &mut Rgba<u8>, color: [u8; 3], a: f32) {
    let a = a.clamp(0.0, 1.0);
    let da = p.0[3] as f32 / 255.0;
    let oa = a + da * (1.0 - a);
    if oa <= 0.0 {
        return;
    }
    for (k, c) in color.iter().enumerate() {
        p.0[k] = ((*c as f32 * a + p.0[k] as f32 * da * (1.0 - a)) / oa).round() as u8;
    }
    p.0[3] = (oa * 255.0).round() as u8;
}

/// Lays a text over a frame as `look` has it: faded, scaled about its
/// middle, moved down by `dy` text sizes (`size`: the text's height in px).
pub fn put_layer_as(frame: &mut RgbaImage, layer: &Layer, look: Look, size: f32) {
    if look.alpha <= 0.0 {
        return;
    }
    let scaled;
    let (layer, ox, oy) = if (look.scale - 1.0).abs() > 0.01 {
        let (w, h) = (((layer.img.width() as f32 * look.scale) as u32).max(1), ((layer.img.height() as f32 * look.scale) as u32).max(1));
        let cx = layer.x as f32 + layer.img.width() as f32 / 2.0;
        let cy = layer.y as f32 + layer.img.height() as f32 / 2.0;
        scaled = Layer { x: (cx - w as f32 / 2.0) as i64, y: (cy - h as f32 / 2.0) as i64, img: image::imageops::resize(&layer.img, w, h, image::imageops::FilterType::Triangle) };
        (&scaled, 0, (look.dy * size) as i64)
    } else {
        (layer, 0, (look.dy * size) as i64)
    };
    let (fw, fh) = (frame.width() as i64, frame.height() as i64);
    for (lx, ly, p) in layer.img.enumerate_pixels() {
        let a = p.0[3] as f32 / 255.0 * look.alpha;
        if a <= 0.0 {
            continue;
        }
        let (x, y) = (layer.x + ox + lx as i64, layer.y + oy + ly as i64);
        if x >= 0 && y >= 0 && x < fw && y < fh {
            blend(frame.get_pixel_mut(x as u32, y as u32), [p.0[0], p.0[1], p.0[2]], a);
        }
    }
}

/// Lays a subtitle over a frame.
#[cfg(test)]
pub fn put_layer(frame: &mut RgbaImage, layer: &Layer) {
    let (fw, fh) = (frame.width() as i64, frame.height() as i64);
    for (lx, ly, p) in layer.img.enumerate_pixels() {
        let a = p.0[3];
        if a == 0 {
            continue;
        }
        let (x, y) = (layer.x + lx as i64, layer.y + ly as i64);
        if x >= 0 && y >= 0 && x < fw && y < fh {
            blend(frame.get_pixel_mut(x as u32, y as u32), [p.0[0], p.0[1], p.0[2]], a as f32 / 255.0);
        }
    }
}

/// Burns `text` into `img` in `style`.
#[cfg(test)]
pub fn draw_caption(img: &mut RgbaImage, text: &str, vertical: bool, style: &CaptionStyle) {
    if let Some(l) = caption_layer(img.width(), img.height(), text, vertical, style) {
        put_layer(img, &l);
    }
}

fn blend(p: &mut Rgba<u8>, c: [u8; 3], a: f32) {
    let a = a.clamp(0.0, 1.0);
    for (v, c) in p.0.iter_mut().zip(c) {
        *v = (*v as f32 * (1.0 - a) + c as f32 * a).round() as u8;
    }
}

/// A rounded rectangle (x, y, w, h) of `color` at `alpha` over `img`, smooth edges.
fn fill_round(img: &mut RgbaImage, (x, y, w, h): (f32, f32, f32, f32), r: f32, color: [u8; 3], alpha: f32) {
    let (x0, y0) = (x.floor().max(0.0) as u32, y.floor().max(0.0) as u32);
    let (x1, y1) = (((x + w).ceil() as u32).min(img.width()), ((y + h).ceil() as u32).min(img.height()));
    for py in y0..y1 {
        for px in x0..x1 {
            let (cx, cy) = (px as f32 + 0.5, py as f32 + 0.5);
            // distance outside the rounded box
            let dx = (x + r - cx).max(cx - (x + w - r)).max(0.0);
            let dy = (y + r - cy).max(cy - (y + h - r)).max(0.0);
            let cover = (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
            if cover > 0.0 {
                over(img.get_pixel_mut(px, py), color, alpha * cover);
            }
        }
    }
}

/// A picture laid over the video (imported): its middle (fractions of the
/// frame), its width (share of the frame's), its time and entrance.
#[derive(Clone, Debug)]
pub struct Sticker {
    pub path: std::path::PathBuf,
    pub img: std::sync::Arc<RgbaImage>,
    pub from: i64,
    pub to: i64,
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub anim: Anim,
}

/// A sticker as it goes on a frame of `w`×`h`.
pub fn sticker_layer(w: u32, h: u32, s: &Sticker) -> Option<Layer> {
    let lw = ((w as f32 * s.scale.clamp(0.02, 1.5)) as u32).max(2);
    let lh = ((lw as f32 * s.img.height() as f32 / s.img.width().max(1) as f32) as u32).max(2);
    let img = image::imageops::resize(&*s.img, lw, lh, image::imageops::FilterType::Triangle);
    Some(Layer { x: (w as f32 * s.x - lw as f32 / 2.0) as i64, y: (h as f32 * s.y - lh as f32 / 2.0) as i64, img })
}

/// Music under the clip (imported): where in the clip it starts (source
/// time), from where in its file, the file's length, how loud, and whether
/// it fades out at the end.
#[derive(Clone, Debug)]
pub struct Music {
    pub path: std::path::PathBuf,
    pub name: String,
    pub start: i64,
    pub from: i64,
    pub len: i64,
    pub gain: f32,
    pub fade: bool,
}

/// Interleaved stereo `pcm` played `speed` times faster with its pitch kept
/// (WSOLA: windowed pieces overlapped, each nudged to line up with the last).
pub fn stretch(pcm: &[i16], speed: f32) -> Vec<i16> {
    if (speed - 1.0).abs() < 0.01 || pcm.len() < 8 {
        return pcm.to_vec();
    }
    const N: usize = 1536; // 32 ms windows
    const HS: usize = N / 2; // their step in the output
    const TOL: usize = 360; // how far a piece may be nudged
    let frames = pcm.len() / 2;
    let ha = HS as f32 * speed; // their step in the input
    let out_frames = (frames as f32 / speed) as usize;
    let mono = |i: usize| (pcm[2 * i] as f32 + pcm[2 * i + 1] as f32) * 0.5;
    let win: Vec<f32> = (0..N).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / N as f32).cos()).collect();
    let mut out = vec![0f32; (out_frames + N) * 2];
    let mut norm = vec![0f32; out_frames + N];
    let mut natural: Option<usize> = None;
    let mut k = 0usize;
    loop {
        let out_pos = k * HS;
        let nominal = (k as f32 * ha) as usize;
        if out_pos >= out_frames || nominal + N + TOL >= frames {
            break;
        }
        // the piece that best continues the last one's natural next part
        let pos = match natural {
            Some(nat) if nat + HS < frames => {
                let lo = nominal.saturating_sub(TOL);
                let hi = (nominal + TOL).min(frames - N - 1);
                let mut best = (f32::MIN, nominal);
                let mut d = lo;
                while d <= hi {
                    let mut c = 0.0;
                    let mut j = 0;
                    while j < HS {
                        c += mono(d + j) * mono(nat + j);
                        j += 6;
                    }
                    if c > best.0 {
                        best = (c, d);
                    }
                    d += 3;
                }
                best.1
            }
            _ => nominal,
        };
        for j in 0..N {
            let w = win[j];
            out[(out_pos + j) * 2] += pcm[(pos + j) * 2] as f32 * w;
            out[(out_pos + j) * 2 + 1] += pcm[(pos + j) * 2 + 1] as f32 * w;
            norm[out_pos + j] += w;
        }
        natural = Some(pos + HS);
        k += 1;
    }
    let mut res = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let n = norm[i].max(1e-3);
        let k = if norm[i] < 0.05 { 0.0 } else { 1.0 / n };
        res.push((out[2 * i] * k).clamp(-32768.0, 32767.0) as i16);
        res.push((out[2 * i + 1] * k).clamp(-32768.0, 32767.0) as i16);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretch_keeps_length_ratio() {
        // a 440 Hz tone, 1 s: twice as fast is half as long, the same tone
        let tone: Vec<i16> = (0..48_000).flat_map(|i| {
            let v = ((i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 10_000.0) as i16;
            [v, v]
        }).collect();
        let fast = stretch(&tone, 2.0);
        assert!((fast.len() as i64 - 48_000).abs() < 2_000, "{}", fast.len());
        let slow = stretch(&tone, 0.5);
        assert!((slow.len() as i64 - 192_000).abs() < 8_000, "{}", slow.len());
        // zero crossings per second stay about 880
        let cross = |p: &[i16]| p.chunks_exact(2).collect::<Vec<_>>().windows(2).filter(|w| (w[0][0] < 0) != (w[1][0] < 0)).count() as f32 / (p.len() / 2) as f32 * 48_000.0;
        let c = cross(&fast[4000..fast.len() - 4000]);
        assert!((c - 880.0).abs() < 60.0, "{c}");
    }

    #[test]
    fn slices() {
        assert_eq!(slice(1920, 1080, 0.5), (656, 608));
        assert_eq!(slice(1920, 1080, 0.0), (0, 608));
        assert_eq!(slice(1920, 1080, 1.0), (1312, 608));
        // already narrow: all of it
        assert_eq!(slice(500, 1080, 0.5), (0, 500));
    }

    #[test]
    fn caption_timing() {
        let caps = vec![Caption { from: 0, to: 10, text: "a".into() }, Caption { from: 5, to: 20, text: "b".into() }];
        assert_eq!(caption_at(&caps, 3).map(|c| c.text.as_str()), Some("a"));
        assert_eq!(caption_at(&caps, 7).map(|c| c.text.as_str()), Some("b"));
        assert!(caption_at(&caps, 20).is_none());
    }

    /// `BLINKER_TEST_SHORTS` = a PNG of a made-up frame in each layout.
    #[test]
    fn sheet() {
        let Ok(out) = std::env::var("BLINKER_TEST_SHORTS") else { return };
        let src = RgbaImage::from_fn(1920, 1080, |x, y| Rgba([(x / 8) as u8, (y / 5) as u8, 140, 255]));
        let st = CaptionStyle::default();
        let mut a = compose(&src, Aspect::Crop, 0.3);
        draw_caption(&mut a, "вот это я понимаю клип, смотри до конца", true, &CaptionStyle { back: CapBack::Outline, upper: true, size: 1.3, ..st });
        let mut b = compose(&src, Aspect::Fit, 0.5);
        draw_caption(&mut b, "весь кадр по центру", true, &CaptionStyle { color: CAPTION_COLORS[1], y: Some(0.25), ..st });
        let mut c = src.clone();
        draw_caption(&mut c, "и обычное видео с подписью внизу", false, &CaptionStyle { back: CapBack::Shadow, font: CapFont::Mono, ..st });
        let mut sheet = RgbaImage::from_pixel(1080 * 2 + 1920 + 80, 1920, Rgba([20, 20, 20, 255]));
        image::imageops::overlay(&mut sheet, &a, 0, 0);
        image::imageops::overlay(&mut sheet, &b, 1120, 0);
        image::imageops::overlay(&mut sheet, &c, 2240, 420);
        image::imageops::resize(&sheet, sheet.width() / 3, sheet.height() / 3, image::imageops::FilterType::Triangle).save(out).unwrap();
    }
}

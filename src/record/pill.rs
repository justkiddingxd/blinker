//! The recording pill and viewfinder pictures (tiny-skia), shared by the
//! Windows and macOS HUDs and the README animation.

use crate::sk::{draw_text, paint_of};
use crate::theme;
use crate::win::{IRect, Monitor};
use ab_glyph::FontRef;
use eframe::egui::Color32;
use tiny_skia as sk;

const REC: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Btn {
    Pause,
    Stop,
}

/// Pill position under `area` (centered), above it when the bottom has no
/// room, inside along the bottom edge when neither fits.
pub(crate) fn pill_position(area: IRect, monitor: &Monitor, size: (i32, i32), scale: f32) -> (i32, i32) {
    let px = |v: f32| (v * scale).round() as i32;
    let gap = px(8.0);
    let margin = px(5.0);
    let (mon, work) = (monitor.rect, monitor.work);
    let mut pos = (area.x + area.w / 2 - size.0 / 2, area.bottom() + margin + gap);
    if pos.1 + size.1 > work.bottom() {
        pos.1 = area.y - margin - gap - size.1;
        if pos.1 < mon.y {
            pos.1 = area.bottom().min(work.bottom()) - px(14.0) - size.1;
        }
    }
    pos.0 = pos.0.clamp(mon.x, (mon.right() - size.0).max(mon.x));
    pos
}

/// Frame just outside the recorded area (`size` at `o` in `pm`): light and
/// dark dashes in turn plus corner brackets with a dark rim, so it reads on
/// white pages and dark desktops alike. Reaches 7 px × `s` beyond the area.
pub(crate) fn draw_frame(pm: &mut sk::Pixmap, o: (f32, f32), size: (f32, f32), s: f32) {
    let (w, h) = size;
    let inset = 2.5 * s;
    let (l, t, r, b) = (o.0 - inset, o.1 - inset, o.0 + w + inset, o.1 + h + inset);
    let light = Color32::from_rgba_unmultiplied(244, 242, 236, 245);
    let dark = Color32::from_rgba_unmultiplied(20, 20, 19, 235);
    let halo = Color32::from_black_alpha(55);
    let id = sk::Transform::identity();
    let mut pb = sk::PathBuilder::new();
    pb.move_to(l, t);
    pb.line_to(r, t);
    pb.line_to(r, b);
    pb.line_to(l, b);
    pb.close();
    if let Some(path) = pb.finish() {
        // a soft halo for mid tones, a solid dark line, light dashes on top
        for (col, width) in [(halo, 3.6 * s), (dark, 1.6 * s)] {
            let st = sk::Stroke { width, ..Default::default() };
            pm.stroke_path(&path, &paint_of(col), &st, id, None);
        }
        let dash = sk::StrokeDash::new(vec![6.0 * s, 6.0 * s], 0.0);
        let st = sk::Stroke { width: 1.6 * s, dash, ..Default::default() };
        pm.stroke_path(&path, &paint_of(light), &st, id, None);
    }
    let len = (20.0 * s).min(w / 3.0).min(h / 3.0);
    let ob = 1.2 * s;
    for (cx, cy, dx, dy) in [(l - ob, t - ob, 1.0, 1.0), (r + ob, t - ob, -1.0, 1.0), (l - ob, b + ob, 1.0, -1.0), (r + ob, b + ob, -1.0, -1.0)] {
        let mut pb = sk::PathBuilder::new();
        pb.move_to(cx, cy + dy * len);
        pb.line_to(cx, cy);
        pb.line_to(cx + dx * len, cy);
        if let Some(path) = pb.finish() {
            for (col, width) in [(dark, 5.4 * s), (light, 3.0 * s)] {
                let st = sk::Stroke { width, line_cap: sk::LineCap::Square, ..Default::default() };
                pm.stroke_path(&path, &paint_of(col), &st, id, None);
            }
        }
    }
}

fn fmt_time(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

fn fill(pm: &mut sk::Pixmap, path: Option<sk::Path>, c: Color32) {
    if let Some(p) = path {
        pm.fill_path(&p, &paint_of(c), sk::FillRule::Winding, sk::Transform::identity(), None);
    }
}

/// What the pill shows.
pub(crate) struct PillLook {
    pub secs: u64,
    pub paused: bool,
    /// Seconds since the pill appeared (drives the slow breathing of the dot).
    pub clock: f32,
    pub hover: Option<Btn>,
}

/// Pill size and the x ranges of its pause and stop buttons at scale `s`.
pub(crate) fn pill_layout(s: f32) -> ((i32, i32), (i32, i32), (i32, i32)) {
    let px = |v: f32| (v * s).round() as i32;
    let size = (px(156.0), px(32.0));
    (size, (size.0 - px(66.0), size.0 - px(35.0)), (size.0 - px(35.0), size.0 - px(3.0)))
}

/// One breath of the recording dot: slow and never fully dark.
const BREATH: f32 = 2.6;
const PAUSED: Color32 = Color32::from_rgb(0xE0, 0xA8, 0x3A);

/// "● 0:12 | ⏸ ■" on a dark rounded plate.
pub(crate) fn draw_pill(look: &PillLook, s: f32) -> Option<sk::Pixmap> {
    let ((w, h), pause_x, stop_x) = pill_layout(s);
    let (w, h) = (w as f32, h as f32);
    let mut pm = sk::Pixmap::new(w as u32, h as u32)?;
    let id = sk::Transform::identity();
    let cy = h / 2.0;

    // plate: a faint top-to-bottom shade, a hairline and a lit top edge
    if let Some(bg) = theme::rounded_rect_path(0.5, 0.5, w - 1.0, h - 1.0, h / 2.0 - 0.5) {
        let mut paint = sk::Paint { anti_alias: true, ..Default::default() };
        paint.shader = sk::LinearGradient::new(
            sk::Point::from_xy(0.0, 0.0),
            sk::Point::from_xy(0.0, h),
            vec![
                sk::GradientStop::new(0.0, sk::Color::from_rgba8(36, 36, 34, 246)),
                sk::GradientStop::new(1.0, sk::Color::from_rgba8(22, 22, 21, 246)),
            ],
            sk::SpreadMode::Pad,
            id,
        )
        .unwrap_or(sk::Shader::SolidColor(sk::Color::from_rgba8(26, 26, 24, 246)));
        pm.fill_path(&bg, &paint, sk::FillRule::Winding, id, None);
        let line = sk::Stroke { width: 1.0, ..Default::default() };
        pm.stroke_path(&bg, &paint_of(Color32::from_rgb(0x3A, 0x3A, 0x36)), &line, id, None);
    }
    if let Some(top) = theme::rounded_rect_path(1.5, 1.5, w - 3.0, h - 3.0, h / 2.0 - 1.5) {
        let line = sk::Stroke { width: 1.0, ..Default::default() };
        let clip = sk::Rect::from_xywh(0.0, 0.0, w, h * 0.35).and_then(|r| {
            let mut m = sk::Mask::new(w as u32, h as u32)?;
            m.fill_path(&sk::PathBuilder::from_rect(r), sk::FillRule::Winding, false, id);
            Some(m)
        });
        pm.stroke_path(&top, &paint_of(Color32::from_white_alpha(14)), &line, id, clip.as_ref());
    }

    // the dot: breathing red while recording, steady amber while paused
    let (dx, r) = (17.0 * s, 4.5 * s);
    if look.paused {
        fill(&mut pm, sk::PathBuilder::from_circle(dx, cy, r), PAUSED);
    } else {
        let k = 0.5 - 0.5 * (look.clock / BREATH * std::f32::consts::TAU).cos();
        for (grow, alpha) in [(4.5, 0.10), (2.6, 0.16)] {
            let a = (alpha * k * 255.0) as u8;
            fill(&mut pm, sk::PathBuilder::from_circle(dx, cy, r + grow * s * (0.6 + 0.4 * k)), Color32::from_rgba_unmultiplied(REC.r(), REC.g(), REC.b(), a));
        }
        let a = (255.0 * (0.78 + 0.22 * k)) as u8;
        fill(&mut pm, sk::PathBuilder::from_circle(dx, cy, r), Color32::from_rgba_unmultiplied(REC.r(), REC.g(), REC.b(), a));
    }
    if let Ok(f) = FontRef::try_from_slice(theme::FONT_MONO_MEDIUM) {
        let col = if look.paused { theme::DIM } else { theme::TEXT };
        draw_text(&mut pm, &f, 13.0 * s, 30.0 * s, cy, &fmt_time(look.secs), col);
    }

    let x0 = pause_x.0 as f32;
    if let Some(sep) = sk::Rect::from_xywh(x0 - 1.0 * s, 8.0 * s, 1.0, h - 16.0 * s) {
        pm.fill_rect(sep, &paint_of(theme::LINE), id, None);
    }
    for (btn, (a, b)) in [(Btn::Pause, pause_x), (Btn::Stop, stop_x)] {
        let c = ((a + b) as f32 / 2.0, cy);
        let hovered = look.hover == Some(btn);
        if hovered {
            let bg = if btn == Btn::Stop { Color32::from_rgba_unmultiplied(REC.r(), REC.g(), REC.b(), 60) } else { Color32::from_white_alpha(30) };
            fill(&mut pm, sk::PathBuilder::from_circle(c.0, c.1, 11.5 * s), bg);
        }
        let fg = match (btn, hovered) {
            (Btn::Stop, true) => REC,
            (_, true) => theme::ACCENT_HI,
            _ => theme::TEXT,
        };
        match btn {
            Btn::Stop => {
                let sq = 9.0 * s;
                fill(&mut pm, theme::rounded_rect_path(c.0 - sq / 2.0, c.1 - sq / 2.0, sq, sq, 2.0 * s), fg);
            }
            Btn::Pause if look.paused => {
                // resume: a play triangle with rounded feel
                let mut pb = sk::PathBuilder::new();
                pb.move_to(c.0 - 3.0 * s, c.1 - 5.5 * s);
                pb.line_to(c.0 + 5.5 * s, c.1);
                pb.line_to(c.0 - 3.0 * s, c.1 + 5.5 * s);
                pb.close();
                fill(&mut pm, pb.finish(), fg);
            }
            Btn::Pause => {
                for dx in [-2.75, 2.75] {
                    let x = c.0 + dx * s - 1.4 * s;
                    fill(&mut pm, theme::rounded_rect_path(x, c.1 - 5.0 * s, 2.8 * s, 10.0 * s, 1.0 * s), fg);
                }
            }
        }
    }
    Some(pm)
}

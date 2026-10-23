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

/// The places chosen in the settings for the recording going on: for an
/// area, for a whole display.
static PLACE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static PLACE_SCREEN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn index(place: crate::config::PillPlace) -> u8 {
    crate::config::PillPlace::ALL.iter().position(|p| *p == place).unwrap_or(0) as u8
}

fn from_index(i: u8) -> crate::config::PillPlace {
    crate::config::PillPlace::ALL.get(i as usize).copied().unwrap_or_default()
}

/// Takes the settings' places for the pill (at the start of a recording).
pub fn set_place(area: crate::config::PillPlace, screen: crate::config::PillPlace) {
    use std::sync::atomic::Ordering::Relaxed;
    PLACE.store(index(area), Relaxed);
    PLACE_SCREEN.store(index(screen), Relaxed);
}

/// Pill position: where the settings put it on the screen, or (auto) under
/// `area` (centered), above it when the bottom has no room, inside along the
/// bottom edge when neither fits; the whole screen puts it by the tray.
pub(crate) fn pill_position(area: IRect, monitor: &Monitor, size: (i32, i32), scale: f32) -> (i32, i32) {
    let px = |v: f32| (v * scale).round() as i32;
    let gap = px(8.0);
    let margin = px(5.0);
    let (mon, work) = (monitor.rect, monitor.work);
    let whole = area.x <= mon.x && area.y <= mon.y && area.right() >= mon.right() && area.bottom() >= mon.bottom();
    let chosen = from_index(if whole { &PLACE_SCREEN } else { &PLACE }.load(std::sync::atomic::Ordering::Relaxed));
    if let Some((fx, fy)) = chosen.anchor() {
        let g = px(12.0);
        let x = work.x + g + ((work.w - size.0 - 2 * g) as f32 * fx) as i32;
        let y = work.y + g + ((work.h - size.1 - 2 * g) as f32 * fy) as i32;
        return (x, y);
    }
    // the whole display: in the corner by the tray, just off the taskbar
    if whole {
        let g = px(12.0);
        let right = work.right() - size.0 - g;
        let bottom = work.bottom() - size.1 - g;
        return if cfg!(target_os = "macos") {
            // a Mac's menu bar has no room for it: the bottom corner, off the Dock
            (right, bottom)
        } else if work.y > mon.y {
            // taskbar on top: the tray is in its right end
            (right, work.y + g)
        } else if work.x > mon.x {
            // on the left: the tray is at its bottom
            (work.x + g, bottom)
        } else {
            (right, bottom)
        };
    }
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
    /// A pane of Liquid Glass lies under the picture (macOS 26): the card
    /// only tints it.
    pub glass: bool,
}

/// Pill size and the x ranges of its pause and stop buttons at scale `s`.
pub(crate) fn pill_layout(s: f32) -> ((i32, i32), (i32, i32), (i32, i32)) {
    let px = |v: f32| (v * s).round() as i32;
    let size = (px(170.0), px(40.0));
    (size, (size.0 - px(70.0), size.0 - px(38.0)), (size.0 - px(37.0), size.0 - px(5.0)))
}

/// One breath of the recording dot: slow and never fully dark.
const BREATH: f32 = 2.6;
const PAUSED: Color32 = Color32::from_rgb(0xE0, 0xA8, 0x3A);

/// A card like the rest of blinker: a red chip with the dot and the time,
/// then pause and a red stop.
pub(crate) fn draw_pill(look: &PillLook, s: f32) -> Option<sk::Pixmap> {
    let ((w, h), pause_x, stop_x) = pill_layout(s);
    let (w, h) = (w as f32, h as f32);
    let mut pm = sk::Pixmap::new(w as u32, h as u32)?;
    let id = sk::Transform::identity();
    let cy = h / 2.0;
    let line = sk::Stroke { width: 1.0, ..Default::default() };

    // the card
    let radius = 12.0 * s;
    if let Some(bg) = theme::rounded_rect_path(0.5, 0.5, w - 1.0, h - 1.0, radius) {
        if look.glass {
            fill(&mut pm, Some(bg), Color32::from_rgba_unmultiplied(0x1C, 0x1C, 0x1A, 90));
        } else {
            fill(&mut pm, Some(bg.clone()), Color32::from_rgba_unmultiplied(0x1C, 0x1C, 0x1A, 250));
            pm.stroke_path(&bg, &paint_of(theme::LINE_HI), &line, id, None);
        }
    }

    // the chip: tinted red while recording, amber while paused
    let tint = if look.paused { PAUSED } else { REC };
    let chip = (5.0 * s, 5.0 * s, pause_x.0 as f32 - 10.0 * s, h - 10.0 * s);
    fill(&mut pm, theme::rounded_rect_path(chip.0, chip.1, chip.2, chip.3, 8.0 * s), Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), 34));
    let (dx, r) = (chip.0 + 13.0 * s, 4.0 * s);
    if look.paused {
        // two short bars instead of the dot
        for x in [dx - 2.6 * s, dx + 1.0 * s] {
            fill(&mut pm, theme::rounded_rect_path(x - 0.8 * s, cy - 4.5 * s, 2.4 * s, 9.0 * s, 1.0 * s), PAUSED);
        }
    } else {
        let k = 0.5 - 0.5 * (look.clock / BREATH * std::f32::consts::TAU).cos();
        let a = (60.0 * k) as u8;
        fill(&mut pm, sk::PathBuilder::from_circle(dx, cy, r + 3.0 * s), Color32::from_rgba_unmultiplied(REC.r(), REC.g(), REC.b(), a));
        fill(&mut pm, sk::PathBuilder::from_circle(dx, cy, r), REC);
    }
    if let Ok(f) = FontRef::try_from_slice(theme::FONT_MONO_MEDIUM) {
        let col = if look.paused { PAUSED } else { theme::TEXT };
        draw_text(&mut pm, &f, 13.5 * s, dx + 11.0 * s, cy, &fmt_time(look.secs), col);
    }

    // the buttons: rounded squares, stop filled red
    for (btn, (a, b)) in [(Btn::Pause, pause_x), (Btn::Stop, stop_x)] {
        let (bx, bw) = (a as f32, (b - a) as f32);
        let by = (h - bw) / 2.0;
        let c = (bx + bw / 2.0, cy);
        let hovered = look.hover == Some(btn);
        let plate = match (btn, hovered) {
            (Btn::Stop, true) => Color32::from_rgb(0xF0, 0x5A, 0x5F),
            (Btn::Stop, false) => REC,
            (_, true) => Color32::from_white_alpha(30),
            _ => Color32::from_white_alpha(12),
        };
        fill(&mut pm, theme::rounded_rect_path(bx, by, bw, bw, 8.0 * s), plate);
        let fg = if btn == Btn::Stop { Color32::WHITE } else if hovered { theme::ACCENT_HI } else { theme::TEXT };
        match btn {
            Btn::Stop => {
                let sq = 9.0 * s;
                fill(&mut pm, theme::rounded_rect_path(c.0 - sq / 2.0, c.1 - sq / 2.0, sq, sq, 2.0 * s), fg);
            }
            Btn::Pause if look.paused => {
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

#[cfg(test)]
mod place_tests {
    use crate::win::{IRect, Monitor};

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn whole_screen_sits_by_the_tray() {
        let mon = IRect::new(0, 0, 1920, 1080);
        let size = (150, 30);
        // taskbar at the bottom: bottom right, above it
        let m = Monitor { rect: mon, work: IRect::new(0, 0, 1920, 1032), scale: 1.0 };
        assert_eq!(super::pill_position(mon, &m, size, 1.0), (1920 - 150 - 12, 1032 - 30 - 12));
        // on top: top right, below it
        let m = Monitor { rect: mon, work: IRect::new(0, 48, 1920, 1032), scale: 1.0 };
        assert_eq!(super::pill_position(mon, &m, size, 1.0), (1920 - 150 - 12, 48 + 12));
        // on the left: bottom left, next to it
        let m = Monitor { rect: mon, work: IRect::new(62, 0, 1858, 1080), scale: 1.0 };
        assert_eq!(super::pill_position(mon, &m, size, 1.0), (62 + 12, 1080 - 30 - 12));
        // an area keeps its pill under it
        let area = IRect::new(100, 100, 400, 300);
        let m = Monitor { rect: mon, work: IRect::new(0, 0, 1920, 1032), scale: 1.0 };
        assert_eq!(super::pill_position(area, &m, size, 1.0).1, 100 + 300 + 13);
    }
}

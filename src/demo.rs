//! `blinker.exe --demo <dir> [en|ru]`: plays a scripted capture on a made-up
//! desktop in an off-screen window and saves every frame as a PNG (for the
//! README animation). The real mouse and keyboard are never touched.

use crate::capture::Snapshot;
use crate::config::{Config, ToolsPos};
use crate::win::{IRect, Monitor};
use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use eframe::egui::{self, Pos2, Rect, pos2};
use std::path::PathBuf;
use tiny_skia as sk;

pub const W: u32 = 1000;
pub const H: u32 = 640;
/// Virtual frames per second (the overlay's clock steps at most 1/60 s).
const FPS: f64 = 60.0;

pub fn run(dir: PathBuf, lang: &str) {
    let _ = std::fs::create_dir_all(&dir);
    let cfg = Config { tools_pos: ToolsPos::Below, magnifier: true, sound_enabled: false, ..Config::default() };
    let shared = crate::runtime::Shared::new(cfg);
    crate::i18n::apply(lang);
    let image = scene();
    // a beat of the plain desktop before the overlay appears
    for i in 0..30 {
        save(&dir, i, &image, Some((pos2(180.0, 120.0), Cursor::Arrow)));
    }
    let vs = crate::win::virtual_screen();
    let full = IRect::new(0, 0, W as i32, H as i32);
    let snap = Snapshot {
        image: image.clone(),
        // right of every monitor: nobody sees or clicks it
        origin: (vs.right() + 400, vs.y),
        windows: Vec::new(),
        monitors: vec![Monitor { rect: full, work: full, scale: 1.0 }],
    };
    let player = Player::new(dir.clone(), 30);
    let count = player.out.clone();
    crate::overlay::run_demo(&shared, snap, player);
    let last = count.get();
    // the shot is taken: back to the desktop, with a note that it's copied
    let mut done = image.clone();
    copied_note(&mut done, crate::i18n::l("скопировано в буфер", "copied to clipboard"));
    for i in 0..90 {
        let img = if i < 6 { &image } else { &done };
        save(&dir, last + i, img, Some((pos2(940.0, 330.0), Cursor::Arrow)));
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Cursor {
    Arrow,
    Cross,
}

#[derive(Clone, Copy)]
enum Target {
    At(f32, f32),
    /// Button `i` of the tools panel.
    Tool(usize),
    /// Button `i` of the actions bar.
    Action(usize),
}

#[derive(Clone, Copy)]
enum Step {
    Wait(f64),
    Move(Target, f64),
    Press,
    Release,
}

fn script() -> Vec<Step> {
    use Step::*;
    use Target::*;
    let click = |v: &mut Vec<Step>| v.extend([Press, Wait(0.08), Release]);
    let mut v = vec![Wait(0.5), Move(At(262.0, 170.0), 0.7), Wait(0.25), Press, Move(At(916.0, 528.0), 1.0), Release, Wait(0.8)];
    // arrow at the chart's peak
    v.extend([Move(Tool(1), 0.6), Wait(0.1)]);
    click(&mut v);
    v.extend([Wait(0.3), Move(At(392.0, 452.0), 0.5), Press, Move(At(548.0, 336.0), 0.5), Release, Wait(0.4)]);
    // numbered steps on two cards
    v.extend([Move(Tool(4), 0.6), Wait(0.1)]);
    click(&mut v);
    v.extend([Wait(0.2), Move(At(452.0, 222.0), 0.5)]);
    click(&mut v);
    v.extend([Wait(0.25), Move(At(664.0, 222.0), 0.4)]);
    click(&mut v);
    v.push(Wait(0.4));
    // blur the contact line
    v.extend([Move(Tool(5), 0.6), Wait(0.1)]);
    click(&mut v);
    v.extend([Wait(0.2), Move(At(292.0, 489.0), 0.5), Press, Move(At(700.0, 513.0), 0.6), Release, Wait(0.6)]);
    // copy
    v.extend([Move(Action(0), 0.7), Wait(0.2)]);
    click(&mut v);
    v.push(Wait(1.5));
    v
}

/// Where the overlay's bars are, to aim at their buttons.
pub struct View {
    pub tools: Option<Rect>,
    pub actions: Option<Rect>,
    pub cursor: egui::CursorIcon,
}

pub struct Player {
    dir: PathBuf,
    /// Number of the next output frame (read after the window closes).
    out: std::rc::Rc<std::cell::Cell<usize>>,
    /// Frame number of the first overlay frame in the output.
    first: usize,
    frame: usize,
    saved: usize,
    /// Screenshots handed back by the renderer, oldest first.
    shots: std::sync::Arc<std::sync::Mutex<Vec<std::sync::Arc<egui::ColorImage>>>>,
    steps: Vec<Step>,
    step: usize,
    step_t: f64,
    from: Pos2,
    to: Option<Pos2>,
    pos: Pos2,
    pressed: bool,
    /// Pointer and cursor of each frame, matched with its screenshot.
    trail: Vec<(Pos2, Option<Cursor>)>,
    done_at: Option<f64>,
}

impl Player {
    fn new(dir: PathBuf, first: usize) -> Self {
        let start = pos2(180.0, 120.0);
        Self {
            dir,
            out: std::rc::Rc::new(std::cell::Cell::new(first)),
            first,
            frame: 0,
            saved: 0,
            shots: Default::default(),
            steps: script(),
            step: 0,
            step_t: 0.0,
            from: start,
            to: None,
            pos: start,
            pressed: false,
            trail: Vec::new(),
            done_at: None,
        }
    }

    fn resolve(&self, t: Target, view: &View) -> Pos2 {
        match t {
            Target::At(x, y) => pos2(x, y),
            // 36 px buttons 2 px apart after a 4 px margin; a 9 px separator follows the first
            Target::Tool(i) => view.tools.map_or(self.pos, |r| {
                pos2(r.left() + 4.0 + 18.0 + i as f32 * 38.0 + if i > 0 { 11.0 } else { 0.0 }, r.center().y)
            }),
            Target::Action(i) => view.actions.map_or(self.pos, |r| pos2(r.center().x, r.top() + 3.0 + 18.0 + i as f32 * 38.0)),
        }
    }

    /// Asks for a screenshot of the frame being drawn.
    pub fn shot_request(&self) -> egui::ScreenshotCallback {
        let shots = self.shots.clone();
        egui::ScreenshotCallback::new(move |img| shots.lock().unwrap().push(img))
    }

    fn save_shots(&mut self) {
        let shots = std::mem::take(&mut *self.shots.lock().unwrap());
        for img in shots {
            let (pos, cursor) = self.trail.get(self.saved).copied().unwrap_or((self.pos, None));
            let rgba = image::RgbaImage::from_raw(
                img.size[0] as u32,
                img.size[1] as u32,
                img.pixels.iter().flat_map(|c| c.to_array()).collect(),
            );
            if let Some(rgba) = rgba {
                save(&self.dir, self.first + self.saved, &rgba, cursor.map(|c| (pos, c)));
            }
            self.saved += 1;
            self.out.set(self.first + self.saved);
        }
    }

    /// Replaces the real input of this frame with the script's.
    pub fn feed(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput, view: &View) {
        self.save_shots();
        raw.events.clear();
        raw.viewports.entry(raw.viewport_id).or_default().focused = Some(true);
        let dt = 1.0 / FPS;
        let now = self.frame as f64 * dt;
        raw.time = Some(now);
        raw.predicted_dt = dt as f32;
        self.frame += 1;

        let was = self.pressed;
        let mut left = dt;
        while left > 0.0 && self.step < self.steps.len() {
            match self.steps[self.step] {
                Step::Press => self.pressed = true,
                Step::Release => self.pressed = false,
                Step::Wait(d) | Step::Move(_, d) => {
                    let to = match self.steps[self.step] {
                        Step::Move(t, _) => {
                            let to = self.to.unwrap_or_else(|| self.resolve(t, view));
                            self.to = Some(to);
                            Some(to)
                        }
                        _ => None,
                    };
                    let take = left.min(d - self.step_t);
                    self.step_t += take;
                    left -= take;
                    if let Some(to) = to {
                        let k = ease((self.step_t / d) as f32);
                        self.pos = self.from + (to - self.from) * k;
                    }
                    if self.step_t < d - 1e-9 {
                        break;
                    }
                }
            }
            self.step += 1;
            self.step_t = 0.0;
            self.from = self.pos;
            self.to = None;
        }
        raw.events.push(egui::Event::PointerMoved(self.pos));
        if self.pressed != was {
            raw.events.push(egui::Event::PointerButton {
                pos: self.pos,
                button: egui::PointerButton::Primary,
                pressed: self.pressed,
                modifiers: Default::default(),
            });
        }
        let cursor = match view.cursor {
            egui::CursorIcon::None => None,
            egui::CursorIcon::Crosshair => Some(Cursor::Cross),
            _ => Some(Cursor::Arrow),
        };
        self.trail.push((self.pos, cursor));
        if self.step >= self.steps.len() {
            let at = *self.done_at.get_or_insert(now);
            if now - at > 3.0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.save_shots();
    }
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

fn save(dir: &std::path::Path, n: usize, img: &image::RgbaImage, cursor: Option<(Pos2, Cursor)>) {
    let mut img = img.clone();
    if let Some((p, c)) = cursor {
        draw_cursor(&mut img, p, c);
    }
    let _ = img.save(dir.join(format!("{n:04}.png")));
}

fn draw_cursor(img: &mut image::RgbaImage, p: Pos2, c: Cursor) {
    let Some(mut pm) = sk::Pixmap::new(img.width(), img.height()) else { return };
    let mut white = sk::Paint::default();
    white.anti_alias = true;
    white.set_color_rgba8(255, 255, 255, 255);
    let mut black = white.clone();
    black.set_color_rgba8(0, 0, 0, 255);
    let id = sk::Transform::identity();
    let mut pb = sk::PathBuilder::new();
    match c {
        Cursor::Arrow => {
            let pts = [(0.0, 0.0), (0.0, 17.0), (4.0, 13.0), (7.0, 20.0), (9.5, 19.0), (6.5, 12.0), (12.0, 12.0)];
            pb.move_to(p.x + pts[0].0, p.y + pts[0].1);
            for (x, y) in &pts[1..] {
                pb.line_to(p.x + x, p.y + y);
            }
            pb.close();
            let path = pb.finish().unwrap();
            pm.fill_path(&path, &white, sk::FillRule::Winding, id, None);
            pm.stroke_path(&path, &black, &sk::Stroke { width: 1.2, ..Default::default() }, id, None);
        }
        Cursor::Cross => {
            for (a, b) in [((-9.0, 0.0), (9.0, 0.0)), ((0.0, -9.0), (0.0, 9.0))] {
                pb.move_to(p.x + a.0, p.y + a.1);
                pb.line_to(p.x + b.0, p.y + b.1);
            }
            let path = pb.finish().unwrap();
            pm.stroke_path(&path, &black, &sk::Stroke { width: 3.0, ..Default::default() }, id, None);
            pm.stroke_path(&path, &white, &sk::Stroke { width: 1.0, ..Default::default() }, id, None);
        }
    }
    let over = crate::theme::pixmap_to_image(&pm);
    image::imageops::overlay(img, &over, 0, 0);
}

// ---------------------------------------------------------------------------
// The made-up desktop: an analytics app on a graphite wallpaper.

const FONT: &[u8] = include_bytes!("../assets/fonts/FiraSans-Regular.ttf");
const FONT_MED: &[u8] = include_bytes!("../assets/fonts/FiraSans-Medium.ttf");

fn rgba(hex: u32, a: u8) -> sk::Color {
    sk::Color::from_rgba8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, a)
}

fn paint(c: sk::Color) -> sk::Paint<'static> {
    let mut p = sk::Paint::default();
    p.anti_alias = true;
    p.set_color(c);
    p
}

fn rrect(pm: &mut sk::Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, c: sk::Color) {
    if let Some(p) = crate::theme::rounded_rect_path(x, y, w, h, r) {
        pm.fill_path(&p, &paint(c), sk::FillRule::Winding, sk::Transform::identity(), None);
    }
}

fn scene() -> image::RgbaImage {
    let (w, h) = (W as f32, H as f32);
    let mut pm = sk::Pixmap::new(W, H).unwrap();
    let id = sk::Transform::identity();
    // wallpaper: graphite with two soft glows
    let bg = sk::LinearGradient::new(
        sk::Point::from_xy(0.0, 0.0),
        sk::Point::from_xy(w, h),
        vec![sk::GradientStop::new(0.0, rgba(0x2B2A27, 255)), sk::GradientStop::new(1.0, rgba(0x121211, 255))],
        sk::SpreadMode::Pad,
        id,
    )
    .unwrap();
    let mut p = sk::Paint::default();
    p.shader = bg;
    pm.fill_rect(sk::Rect::from_xywh(0.0, 0.0, w, h).unwrap(), &p, id, None);
    for (x, y, r, c) in [(120.0, 600.0, 420.0, rgba(0x8A7A5C, 70)), (900.0, 40.0, 380.0, rgba(0x5C6E8A, 60))] {
        let g = sk::RadialGradient::new(
            sk::Point::from_xy(x, y),
            0.0,
            sk::Point::from_xy(x, y),
            r,
            vec![sk::GradientStop::new(0.0, c), sk::GradientStop::new(1.0, rgba(0, 0))],
            sk::SpreadMode::Pad,
            id,
        )
        .unwrap();
        let mut p = sk::Paint::default();
        p.shader = g;
        pm.fill_rect(sk::Rect::from_xywh(0.0, 0.0, w, h).unwrap(), &p, id, None);
    }
    // window with a soft shadow
    let (wx, wy, ww, wh) = (70.0, 56.0, 860.0, 528.0);
    for i in 0..12 {
        let e = 2.0 + i as f32 * 2.0;
        rrect(&mut pm, wx - e, wy - e + 10.0, ww + e * 2.0, wh + e * 2.0, 10.0 + e, rgba(0, 9));
    }
    rrect(&mut pm, wx - 1.0, wy - 1.0, ww + 2.0, wh + 2.0, 11.0, rgba(0x3A3A36, 255));
    rrect(&mut pm, wx, wy, ww, wh, 10.0, rgba(0x1D1D1B, 255));
    // title bar and sidebar
    rrect(&mut pm, wx, wy, ww, 36.0, 10.0, rgba(0x232321, 255));
    pm.fill_rect(sk::Rect::from_xywh(wx, wy + 26.0, ww, 10.0).unwrap(), &paint(rgba(0x232321, 255)), id, None);
    pm.fill_rect(sk::Rect::from_xywh(wx, wy + 36.0, ww, 1.0).unwrap(), &paint(rgba(0x34342F, 255)), id, None);
    for (i, c) in [0xE0685A, 0xE0B25A, 0x6FBF73].iter().enumerate() {
        let path = sk::PathBuilder::from_circle(wx + 18.0 + i as f32 * 18.0, wy + 18.0, 5.5).unwrap();
        pm.fill_path(&path, &paint(rgba(*c, 255)), sk::FillRule::Winding, id, None);
    }
    pm.fill_rect(sk::Rect::from_xywh(wx + 180.0, wy + 37.0, 1.0, wh - 37.0).unwrap(), &paint(rgba(0x2E2E2B, 255)), id, None);
    rrect(&mut pm, wx + 10.0, wy + 92.0, 160.0, 30.0, 6.0, rgba(0x2C2C29, 255));
    // stat cards
    let (mx, my) = (278.0, 182.0);
    for i in 0..3 {
        rrect(&mut pm, mx + i as f32 * 212.0, my, 200.0, 80.0, 8.0, rgba(0x252523, 255));
    }
    // chart
    let (cx, cy, cw, ch) = (278.0, 282.0, 624.0, 188.0);
    rrect(&mut pm, cx, cy, cw, ch, 8.0, rgba(0x252523, 255));
    for i in 1..4 {
        let y = cy + 24.0 + i as f32 * 38.0;
        pm.fill_rect(sk::Rect::from_xywh(cx + 20.0, y, cw - 40.0, 1.0).unwrap(), &paint(rgba(0x30302C, 255)), id, None);
    }
    let series = |vals: &[f32]| -> Vec<(f32, f32)> {
        vals.iter()
            .enumerate()
            .map(|(i, v)| (cx + 24.0 + i as f32 * (cw - 48.0) / (vals.len() - 1) as f32, cy + ch - 26.0 - v * (ch - 60.0)))
            .collect()
    };
    let main = series(&[0.18, 0.24, 0.2, 0.34, 0.42, 0.38, 0.5, 0.62, 0.58, 0.74, 0.86, 0.8]);
    let second = series(&[0.1, 0.14, 0.18, 0.16, 0.22, 0.28, 0.26, 0.3, 0.36, 0.34, 0.4, 0.46]);
    let line = |pts: &[(f32, f32)]| {
        let mut pb = sk::PathBuilder::new();
        pb.move_to(pts[0].0, pts[0].1);
        for w in pts.windows(2) {
            let mid = (w[0].0 + w[1].0) / 2.0;
            pb.cubic_to(mid, w[0].1, mid, w[1].1, w[1].0, w[1].1);
        }
        pb
    };
    let mut area = line(&main);
    area.line_to(main.last().unwrap().0, cy + ch - 20.0);
    area.line_to(main[0].0, cy + ch - 20.0);
    area.close();
    let fill = sk::LinearGradient::new(
        sk::Point::from_xy(0.0, cy + 30.0),
        sk::Point::from_xy(0.0, cy + ch),
        vec![sk::GradientStop::new(0.0, rgba(0xD8D5CD, 60)), sk::GradientStop::new(1.0, rgba(0xD8D5CD, 0))],
        sk::SpreadMode::Pad,
        id,
    )
    .unwrap();
    let mut p = sk::Paint::default();
    p.anti_alias = true;
    p.shader = fill;
    pm.fill_path(&area.finish().unwrap(), &p, sk::FillRule::Winding, id, None);
    let st = |w: f32| sk::Stroke { width: w, line_cap: sk::LineCap::Round, ..Default::default() };
    pm.stroke_path(&line(&second).finish().unwrap(), &paint(rgba(0x7FA7C9, 255)), &st(2.0), id, None);
    pm.stroke_path(&line(&main).finish().unwrap(), &paint(rgba(0xD8D5CD, 255)), &st(2.5), id, None);
    let (px, py) = main[10];
    pm.fill_path(&sk::PathBuilder::from_circle(px, py, 5.0).unwrap(), &paint(rgba(0xD8D5CD, 255)), sk::FillRule::Winding, id, None);
    pm.fill_path(&sk::PathBuilder::from_circle(px, py, 2.5).unwrap(), &paint(rgba(0x252523, 255)), sk::FillRule::Winding, id, None);
    // contact line pill
    rrect(&mut pm, 278.0, 486.0, 624.0, 30.0, 6.0, rgba(0x232321, 255));

    let mut img = crate::theme::pixmap_to_image(&pm);
    let reg = FontRef::try_from_slice(FONT).unwrap();
    let med = FontRef::try_from_slice(FONT_MED).unwrap();
    let faint = [0x8C, 0x8A, 0x84];
    let text_c = [0xD8, 0xD5, 0xCD];
    text(&mut img, &reg, 12.5, wx + 72.0, wy + 23.0, faint, "analytics — q3 report");
    for (i, item) in ["overview", "revenue", "customers", "funnels", "exports", "settings"].iter().enumerate() {
        let c = if i == 1 { text_c } else { faint };
        text(&mut img, &reg, 13.5, wx + 24.0, wy + 70.0 + i as f32 * 36.0, c, item);
    }
    text(&mut img, &med, 24.0, 278.0, 140.0, text_c, "revenue");
    text(&mut img, &reg, 13.0, 278.0, 164.0, faint, "last 12 weeks · updated 2 min ago");
    for (i, (label, value)) in [("revenue", "$48.2k"), ("growth", "+12.4%"), ("customers", "1,284")].iter().enumerate() {
        let x = mx + i as f32 * 212.0 + 16.0;
        text(&mut img, &reg, 12.0, x, my + 24.0, faint, label);
        text(&mut img, &med, 26.0, x, my + 60.0, text_c, value);
    }
    text(&mut img, &reg, 12.0, cx + 20.0, cy + 22.0, faint, "weekly revenue");
    text(&mut img, &reg, 13.0, 292.0, 506.0, faint, "contact:");
    text(&mut img, &reg, 13.0, 346.0, 506.0, text_c, "anna.smith@example.com  ·  +1 (555) 013-4477");
    img
}

/// A small pill in the bottom right corner: the logo and `msg`.
fn copied_note(img: &mut image::RgbaImage, msg: &str) {
    let reg = FontRef::try_from_slice(FONT).unwrap();
    let sf = reg.as_scaled(PxScale::from(13.5));
    let tw: f32 = msg.chars().map(|c| sf.h_advance(sf.glyph_id(c))).sum();
    let (w, h) = (tw + 56.0, 40.0);
    let (x, y) = (W as f32 - w - 24.0, H as f32 - h - 24.0);
    let mut pm = sk::Pixmap::new(W, H).unwrap();
    for i in 0..6 {
        let e = 1.0 + i as f32 * 2.0;
        rrect(&mut pm, x - e, y - e + 4.0, w + e * 2.0, h + e * 2.0, 8.0 + e, rgba(0, 14));
    }
    rrect(&mut pm, x - 1.0, y - 1.0, w + 2.0, h + 2.0, 9.0, rgba(0x3A3A36, 255));
    rrect(&mut pm, x, y, w, h, 8.0, rgba(0x1F1F1D, 255));
    image::imageops::overlay(img, &crate::theme::pixmap_to_image(&pm), 0, 0);
    let mark = crate::theme::mark_image(16);
    image::imageops::overlay(img, &mark, (x + 14.0) as i64, (y + 12.0) as i64);
    text(img, &reg, 13.5, x + 40.0, y + 25.0, [0xD8, 0xD5, 0xCD], msg);
}

fn text(img: &mut image::RgbaImage, font: &FontRef, size: f32, x: f32, baseline: f32, c: [u8; 3], s: &str) {
    let sf = font.as_scaled(PxScale::from(size));
    let mut pen = x;
    let mut prev = None;
    for ch in s.chars() {
        let id = sf.glyph_id(ch);
        if let Some(p) = prev {
            pen += sf.kern(p, id);
        }
        let g = id.with_scale_and_position(sf.scale(), ab_glyph::point(pen, baseline));
        pen += sf.h_advance(id);
        prev = Some(id);
        let Some(o) = font.outline_glyph(g) else { continue };
        let b = o.px_bounds();
        o.draw(|gx, gy, cov| {
            let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
            if px < 0 || py < 0 || px >= img.width() as i32 || py >= img.height() as i32 {
                return;
            }
            let d = img.get_pixel_mut(px as u32, py as u32);
            for k in 0..3 {
                d[k] = (d[k] as f32 + (c[k] as f32 - d[k] as f32) * cov.min(1.0)).round() as u8;
            }
        });
    }
}

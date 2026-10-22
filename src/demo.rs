//! `blinker.exe --demo <dir> [en|ru] [record|replay]`: plays a scripted
//! capture (or screen recording, or saving the instant replay) on a made-up
//! desktop in an off-screen window and saves every frame as a PNG (for the
//! README animations). The real mouse and keyboard are never touched.

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

pub fn run(dir: PathBuf, lang: &str, record: bool) {
    let _ = std::fs::create_dir_all(&dir);
    // `tour`: with the first capture walkthrough (to look at it)
    let tour_done = !std::env::args().any(|a| a == "tour");
    let cfg = Config { tools_pos: ToolsPos::Below, magnifier: true, sound_enabled: false, tour_done, ..Config::default() };
    let shared = crate::runtime::Shared::new(cfg);
    crate::i18n::apply(lang);
    // while recording, the chart keeps growing
    let reveal0 = if record { 0.5 } else { 1.0 };
    let image = scene(reveal0);
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
        #[cfg(target_os = "macos")]
        others: Vec::new(),
    };
    let player = Player::new(dir.clone(), 30, if record { record_script() } else { script() });
    let count = player.out.clone();
    crate::overlay::run_demo(&shared, snap, player, record);
    let mut last = count.get();
    if record {
        last = recording(&dir, last, reveal0, Monitor { rect: full, work: full, scale: 1.0 });
    }
    // back to the desktop, with a note about the result
    let image = scene(1.0);
    let mut done = image.clone();
    let note = if record { crate::i18n::l("видео сохранено", "video saved") } else { crate::i18n::l("скопировано в буфер", "copied to clipboard") };
    copied_note(&mut done, note);
    let rest = if record { pos2(560.0, 540.0) } else { pos2(940.0, 330.0) };
    for i in 0..90 {
        let img = if i < 6 { &image } else { &done };
        save(&dir, last + i, img, Some((rest, Cursor::Arrow)));
    }
}

/// The instant replay: the chart takes off, the replay hotkey is pressed and
/// the card of the saved clip slides in. Drawn without the overlay.
pub fn run_replay(dir: PathBuf, lang: &str) {
    let _ = std::fs::create_dir_all(&dir);
    crate::i18n::apply(lang);
    let reveal0 = 0.42;
    for i in 0..30 {
        save(&dir, i, &scene(reveal0), Some((pos2(420.0, 430.0), Cursor::Arrow)));
    }
    let keys_at = 4.2;
    let card_at = 4.7;
    let len = 8.6;
    let pointer = [(0.0, pos2(420.0, 430.0)), (1.2, pos2(520.0, 400.0)), (3.4, pos2(700.0, 352.0)), (len, pos2(700.0, 352.0))];
    let clip = scene(1.0);
    let n = (len * FPS) as usize;
    for i in 0..n {
        let tm = i as f64 / FPS;
        let reveal = reveal0 + (1.0 - reveal0) * ease((tm / 3.8) as f32);
        let mut img = scene(reveal);
        let k = pointer.windows(2).find(|w| tm <= w[1].0).unwrap_or(&pointer[pointer.len() - 2..]);
        let p = k[0].1 + (k[1].1 - k[0].1) * ease(((tm - k[0].0) / (k[1].0 - k[0].0)) as f32);
        if (keys_at..keys_at + 1.1).contains(&tm) {
            let down = tm < keys_at + 0.45;
            keycaps(&mut img, &["Ctrl", "Alt", "PrtSc"], down);
        }
        if tm >= card_at {
            let slide = ease(((tm - card_at) / 0.35) as f32);
            clip_card(&mut img, &clip, (1.0 - slide) * 340.0);
        }
        save(&dir, 30 + i, &img, Some((p, Cursor::Arrow)));
    }
}

/// A hotkey as it's pressed: keycaps in the bottom middle.
fn keycaps(img: &mut image::RgbaImage, keys: &[&str], down: bool) {
    let med = FontRef::try_from_slice(FONT_MED).unwrap();
    let sf = med.as_scaled(PxScale::from(15.0));
    let widths: Vec<f32> = keys.iter().map(|k| k.chars().map(|c| sf.h_advance(sf.glyph_id(c))).sum::<f32>() + 28.0).collect();
    let gap = 10.0;
    let total: f32 = widths.iter().sum::<f32>() + gap * (keys.len() - 1) as f32;
    let (h, y) = (40.0, 548.0);
    let mut x = (W as f32 - total) / 2.0;
    let press = if down { 2.0 } else { 0.0 };
    let mut pm = sk::Pixmap::new(W, H).unwrap();
    let mut spots = Vec::new();
    for w in &widths {
        rrect(&mut pm, x, y + 3.0, *w, h, 8.0, rgba(0x0B0B0A, 200));
        rrect(&mut pm, x - 1.0, y + press - 1.0, w + 2.0, h + 2.0, 9.0, rgba(0x45453F, 255));
        rrect(&mut pm, x, y + press, *w, h, 8.0, rgba(if down { 0x2B2B28 } else { 0x232321 }, 255));
        spots.push(x);
        x += w + gap;
    }
    image::imageops::overlay(img, &crate::theme::pixmap_to_image(&pm), 0, 0);
    for (k, x) in keys.iter().zip(spots) {
        text(img, &med, 15.0, x + 14.0, y + press + 26.0, [0xE6, 0xE4, 0xDE], k);
    }
}

/// The card of the saved replay in the bottom right corner (as the real one
/// looks): a preview with a play button and the length, the red mark of the
/// replay and a caption. `dx` moves it right, off the screen.
fn clip_card(img: &mut image::RgbaImage, clip: &image::RgbaImage, dx: f32) {
    let (pad, caption) = (8.0, 50.0);
    let iw = 284.0;
    let ih = (iw * clip.height() as f32 / clip.width() as f32).round();
    let (w, h) = (iw + pad * 2.0, pad + ih + caption);
    let (x, y) = (W as f32 - w - 20.0 + dx, H as f32 - h - 20.0);
    let mut pm = sk::Pixmap::new(W, H).unwrap();
    for i in 0..8 {
        let e = 1.0 + i as f32 * 2.5;
        rrect(&mut pm, x - e, y - e + 6.0, w + e * 2.0, h + e * 2.0, 14.0 + e, rgba(0, 14));
    }
    rrect(&mut pm, x - 1.0, y - 1.0, w + 2.0, h + 2.0, 15.0, rgba(0x3A3A36, 255));
    rrect(&mut pm, x, y, w, h, 14.0, rgba(0x1C1C1A, 255));
    // the preview, rounded
    let thumb = image::imageops::resize(clip, iw as u32, ih as u32, image::imageops::FilterType::Triangle);
    let (tx, ty) = (x + pad, y + pad);
    // the scene is opaque: its pixels are already premultiplied
    let tp = sk::IntSize::from_wh(thumb.width(), thumb.height()).and_then(|sz| sk::Pixmap::from_vec(thumb.into_raw(), sz));
    if let (Some(tp), Some(path)) = (tp, crate::theme::rounded_rect_path(tx, ty, iw, ih, 8.0)) {
        let mut p = sk::Paint::default();
        p.anti_alias = true;
        p.shader = sk::Pattern::new(tp.as_ref(), sk::SpreadMode::Pad, sk::FilterQuality::Bilinear, 1.0, sk::Transform::from_translate(tx, ty));
        pm.fill_path(&path, &p, sk::FillRule::Winding, sk::Transform::identity(), None);
    }
    let id = sk::Transform::identity();
    // play button
    let (cx, cy) = (tx + iw / 2.0, ty + ih / 2.0);
    pm.fill_path(&sk::PathBuilder::from_circle(cx, cy, 20.0).unwrap(), &paint(rgba(0x121211, 190)), sk::FillRule::Winding, id, None);
    let mut pb = sk::PathBuilder::new();
    pb.move_to(cx - 5.0, cy - 8.0);
    pb.line_to(cx + 9.0, cy);
    pb.line_to(cx - 5.0, cy + 8.0);
    pb.close();
    pm.fill_path(&pb.finish().unwrap(), &paint(rgba(0xE6E4DE, 255)), sk::FillRule::Winding, id, None);
    // the length
    rrect(&mut pm, tx + iw - 46.0, ty + ih - 28.0, 38.0, 20.0, 5.0, rgba(0x121211, 200));
    image::imageops::overlay(img, &crate::theme::pixmap_to_image(&pm), 0, 0);
    let reg = FontRef::try_from_slice(FONT).unwrap();
    let med = FontRef::try_from_slice(FONT_MED).unwrap();
    text(img, &reg, 12.0, tx + iw - 39.0, ty + ih - 14.0, [0xE6, 0xE4, 0xDE], "0:30");
    let mark = crate::theme::mark_image_in(16, crate::theme::DANGER);
    let cap = ty + ih + 10.0;
    image::imageops::overlay(img, &mark, (x + 14.0) as i64, (cap + 3.0) as i64);
    text(img, &med, 13.5, x + 38.0, cap + 15.0, [0xE6, 0xE4, 0xDE], crate::i18n::l("повтор сохранён", "replay saved"));
    text(img, &reg, 12.0, x + 38.0, cap + 32.0, [0x8C, 0x8A, 0x84], crate::i18n::l("последние 30 секунд · mp4", "the last 30 seconds · mp4"));
}

/// The area being recorded (the cards and the chart).
const REC_AREA: (f32, f32, f32, f32) = (268.0, 172.0, 912.0, 480.0);

/// What the screen looks like while recording: the chart grows, the frame and
/// the timer pill sit around the area, the pointer wanders and presses stop.
/// Returns the number of the next frame.
fn recording(dir: &std::path::Path, first: usize, reveal0: f32, monitor: Monitor) -> usize {
    use crate::record::hud;
    let (l, t, r, b) = REC_AREA;
    let area = IRect::new(l as i32, t as i32, (r - l) as i32, (b - t) as i32);
    let (size, _, stop_x) = hud::pill_layout(1.0);
    let pill_at = hud::pill_position(area, &monitor, size, 1.0);
    let stop = pos2(pill_at.0 as f32 + (stop_x.0 + stop_x.1) as f32 / 2.0, pill_at.1 as f32 + size.1 as f32 / 2.0);
    // pointer keyframes: (time, position)
    let keys = [
        (0.0, pos2(l + (r - l) / 2.0, b + 40.0)),
        (0.4, pos2(l + (r - l) / 2.0, b + 40.0)),
        (1.6, pos2(560.0, 400.0)),
        (3.0, pos2(640.0, 350.0)),
        (4.4, pos2(760.0, 330.0)),
        (5.4, stop),
        (6.6, stop),
    ];
    let len = 6.6;
    let click_at = 6.1;
    let n = (len * FPS) as usize;
    for i in 0..n {
        let tm = i as f64 / FPS;
        let k = keys.windows(2).find(|w| tm <= w[1].0).unwrap_or(&keys[keys.len() - 2..]);
        let f = ease(((tm - k[0].0) / (k[1].0 - k[0].0)) as f32);
        let p = k[0].1 + (k[1].1 - k[0].1) * f;
        let reveal = reveal0 + (1.0 - reveal0) * ease((tm / 5.4) as f32);
        let mut img = scene(reveal);
        let mut pm = sk::Pixmap::new(W, H).unwrap();
        hud::draw_frame(&mut pm, (l, t), (r - l, b - t), 1.0);
        let hover = ((p - stop).length() < 12.0).then_some(hud::Btn::Stop);
        let look = hud::PillLook { secs: tm as u64, paused: false, clock: tm as f32, hover };
        image::imageops::overlay(&mut img, &crate::theme::pixmap_to_image(&pm), 0, 0);
        if let Some(pill) = hud::draw_pill(&look, 1.0) {
            image::imageops::overlay(&mut img, &crate::theme::pixmap_to_image(&pill), pill_at.0 as i64, pill_at.1 as i64);
        }
        if tm >= click_at + 0.12 {
            // stopped: the frame and the pill are gone
            img = scene(1.0);
        }
        save(dir, first + i, &img, Some((p, Cursor::Arrow)));
    }
    first + n
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

/// Record mode: select the cards and the chart, press record.
fn record_script() -> Vec<Step> {
    use Step::*;
    use Target::*;
    let (l, t, r, b) = REC_AREA;
    vec![
        Wait(0.5),
        Move(At(l, t), 0.7),
        Wait(0.25),
        Press,
        Move(At(r, b), 1.0),
        Release,
        Wait(0.7),
        Move(Action(0), 0.7),
        Wait(0.25),
        Press,
        Wait(0.08),
        Release,
        Wait(1.0),
    ]
}

fn script() -> Vec<Step> {
    use Step::*;
    use Target::*;
    let click = |v: &mut Vec<Step>| v.extend([Press, Wait(0.08), Release]);
    // `small`: a small selection, to look at the compact panels
    let end = if std::env::args().any(|a| a == "small") { At(560.0, 300.0) } else { At(916.0, 528.0) };
    let mut v = vec![Wait(0.5), Move(At(262.0, 170.0), 0.7), Wait(0.25), Press, Move(end, 1.0), Release, Wait(0.8)];
    // arrow at the chart's peak
    v.extend([Move(Tool(1), 0.6), Wait(0.1)]);
    click(&mut v);
    v.extend([Wait(0.3), Move(At(392.0, 452.0), 0.5), Press, Move(At(548.0, 336.0), 0.5), Release, Wait(0.4)]);
    // numbered steps on two cards
    v.extend([Move(Tool(5), 0.6), Wait(0.1)]);
    click(&mut v);
    v.extend([Wait(0.2), Move(At(452.0, 222.0), 0.5)]);
    click(&mut v);
    v.extend([Wait(0.25), Move(At(664.0, 222.0), 0.4)]);
    click(&mut v);
    v.push(Wait(0.4));
    // blur the contact line
    v.extend([Move(Tool(6), 0.6), Wait(0.1)]);
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
    fn new(dir: PathBuf, first: usize, steps: Vec<Step>) -> Self {
        let start = pos2(180.0, 120.0);
        Self {
            dir,
            out: std::rc::Rc::new(std::cell::Cell::new(first)),
            first,
            frame: 0,
            saved: 0,
            shots: Default::default(),
            steps,
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
            // 42 px buttons 1 px apart after a 6 px margin; a 10 px separator follows the first
            Target::Tool(i) => view.tools.map_or(self.pos, |r| {
                pos2(r.left() + 6.0 + 21.0 + i as f32 * 43.0 + if i > 0 { 10.0 } else { 0.0 }, r.center().y)
            }),
            Target::Action(i) => view.actions.map_or(self.pos, |r| pos2(r.center().x, r.top() + 6.0 + 21.0 + i as f32 * 43.0)),
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

/// `reveal`: how much of the chart is drawn (0..1); the cards follow it.
fn scene(reveal: f32) -> image::RgbaImage {
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
    // only the part up to `reveal` is drawn
    let end_x = main[0].0 + (main[main.len() - 1].0 - main[0].0) * reveal;
    let mut clip = sk::Mask::new(W, H).unwrap();
    clip.fill_path(&sk::PathBuilder::from_rect(sk::Rect::from_ltrb(0.0, 0.0, end_x + 1.5, h).unwrap()), sk::FillRule::Winding, true, id);
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
    pm.fill_path(&area.finish().unwrap(), &p, sk::FillRule::Winding, id, Some(&clip));
    let st = |w: f32| sk::Stroke { width: w, line_cap: sk::LineCap::Round, ..Default::default() };
    pm.stroke_path(&line(&second).finish().unwrap(), &paint(rgba(0x7FA7C9, 255)), &st(2.0), id, Some(&clip));
    pm.stroke_path(&line(&main).finish().unwrap(), &paint(rgba(0xD8D5CD, 255)), &st(2.5), id, Some(&clip));
    // marker: on the newest point while growing, on the peak when complete
    let (px, py) = if reveal >= 1.0 {
        main[10]
    } else {
        let f = reveal * (main.len() - 1) as f32;
        let (i, k) = (f.floor() as usize, f.fract());
        let (a, b) = (main[i], main[(i + 1).min(main.len() - 1)]);
        // the curve is flat at both ends of a segment (smoothstep in y)
        (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k * k * (3.0 - 2.0 * k))
    };
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
    let customers = (700.0 + 584.0 * reveal).round() as u32;
    let values = [
        format!("${:.1}k", 21.0 + 27.2 * reveal),
        format!("+{:.1}%", 1.6 + 10.8 * reveal),
        format!("{},{:03}", customers / 1000, customers % 1000).trim_start_matches("0,").to_string(),
    ];
    for (i, (label, value)) in ["revenue", "growth", "customers"].iter().zip(&values).enumerate() {
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

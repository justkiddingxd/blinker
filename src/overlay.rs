//! Full-desktop selection overlay drawn over a frozen snapshot.

use crate::capture::Snapshot;
use crate::config::Config;
use crate::editor::doc::{self, Kind, Shape};
use crate::editor::preview::{PrimPainter, View};
use crate::icons::Icon;
use crate::runtime::{self, Command, Shared};
use crate::theme::*;
use crate::ui::{self, TiledTexture};
use crate::win::{self, IRect, Monitor};
use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, Key, Pos2, Rect, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Vec2, pos2, vec2,
};
use image::RgbaImage;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Copy,
    Save,
    Edit,
    Pin,
    /// Record a video of the selected area.
    Record,
    /// Framed on a background with shadow (see `beauty.rs`), then copied.
    Beauty,
    /// Upload and copy the link.
    Upload,
}

pub struct Output {
    pub action: Action,
    pub image: RgbaImage,
    /// Captured rectangle, absolute physical px.
    pub rect: IRect,
    /// Monitor (absolute coordinates) the selection was on, for toast placement.
    pub monitor: Monitor,
    /// The selection is exactly a window the user clicked (record mode: record
    /// the window itself).
    pub window: Option<isize>,
    /// Record mode: record a GIF instead of a video.
    pub gif: bool,
}

/// `record`: pick an area for a screen recording (no annotations, one action).
pub fn run(shared: &Arc<Shared>, snap: Snapshot, record: bool, gif: bool) -> Option<Output> {
    run_with(shared, snap, record, gif, None)
}

/// Scripted run for the README animation (see `demo`): off-screen, cloaked,
/// driven by `player` instead of the mouse.
pub fn run_demo(shared: &Arc<Shared>, snap: Snapshot, player: crate::demo::Player) {
    run_with(shared, snap, false, false, Some(player));
}

fn run_with(shared: &Arc<Shared>, snap: Snapshot, record: bool, gif: bool, demo: Option<crate::demo::Player>) -> Option<Output> {
    let out: Rc<RefCell<Option<Output>>> = Rc::default();
    let out2 = out.clone();
    let target = IRect::new(snap.origin.0, snap.origin.1, snap.image.width() as i32, snap.image.height() as i32);
    let primary_scale = win::monitors().first().map(|m| m.scale).unwrap_or(1.0);
    let viewport = egui::ViewportBuilder::default()
        .with_title("blinker")
        .with_decorations(false)
        .with_resizable(false)
        .with_taskbar(false)
        .with_always_on_top()
        .with_active(demo.is_none())
        .with_position([target.x as f32 / primary_scale, target.y as f32 / primary_scale])
        .with_inner_size([target.w as f32 / primary_scale, target.h as f32 / primary_scale]);
    let shared2 = shared.clone();
    runtime::run_window(shared, viewport, false, move |cc| {
        let hwnd = win::hwnd_of(cc);
        if let Some(h) = hwnd {
            win::disable_transitions(h);
            // DWM-cloaked (invisible but rendering) until real frames are on
            // screen: no blank/white window on any monitor.
            win::cloak(h, true);
            win::set_window_rect(h, target, true);
        }
        if demo.is_some() {
            // frames are saved 1:1 with the made-up desktop
            cc.egui_ctx.set_pixels_per_point(1.0);
        }
        let tex = TiledTexture::new(&cc.egui_ctx, "snapshot", &snap.image, TextureOptions::NEAREST);
        let shared2_color = shared2.config().editor_color;
        OverlayApp {
            cfg: shared2.config(),
            shared: shared2,
            snap,
            tex,
            loupe: None,
            revealed: hwnd.is_none() || demo.is_some(),
            stable: 0,
            shown_frame: 0,
            hwnd,
            target,
            frame_no: 0,
            sel: None,
            drag: Drag::None,
            phase: Phase::Selecting,
            out: out2,
            toolbar_rect: None,
            notice: None,
            last_pointer: None,
            keys_down: [false; 2],
            t0: -1.0,
            last_t: 0.0,
            anim: None,
            adjust_at: 0.0,
            was_adjusting: false,
            clock: 0.0,
            tool: Tool::Region,
            shapes: Vec::new(),
            next_id: 1,
            color: {
                let c = shared2_color;
                eframe::egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
            },
            typing: None,
            prims: PrimPainter::default(),
            top_rect: None,
            bar_mon: None,
            last_ppp: 1.0,
            record,
            gif,
            draw_path: Vec::new(),
            clicked_window: None,
            bar_pos: None,
            hover_shape: None,
            picked: None,
            demo,
        }
    });
    out.borrow_mut().take()
}

/// Tools of the top bar: region selection plus annotations drawn before capturing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Region,
    Arrow,
    Pen,
    Highlight,
    Steps,
    Blur,
    Redact,
    TextBox,
    Text,
}

const TOOLS: [(Tool, Icon, &str); 9] = [
    (Tool::Region, Icon::Region, "выделение области  1"),
    (Tool::Arrow, Icon::Arrow, "стрелка  2 · с shift — дугой"),
    (Tool::Pen, Icon::Pen, "карандаш  3"),
    (Tool::Highlight, Icon::Marker, "маркер  4"),
    (Tool::Steps, Icon::Counter, "шаги  5"),
    (Tool::Blur, Icon::Blur, "размытие  6"),
    (Tool::Redact, Icon::Redact, "закрасить чёрным  7"),
    (Tool::TextBox, Icon::TextBox, "текст с фоном  8"),
    (Tool::Text, Icon::Text, "текст  9"),
];

/// Text being typed on the overlay (image px position).
struct TypingText {
    pos: Pos2,
    buf: String,
    plate: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Handle {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Handle {
    fn cursor(self) -> CursorIcon {
        match self {
            Handle::N | Handle::S => CursorIcon::ResizeVertical,
            Handle::E | Handle::W => CursorIcon::ResizeHorizontal,
            Handle::NE | Handle::SW => CursorIcon::ResizeNeSw,
            Handle::NW | Handle::SE => CursorIcon::ResizeNwSe,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Pending { start: Pos2 },
    /// `last` = pointer on the previous frame (Ctrl moves the whole rect).
    Creating { start: Pos2, last: Pos2 },
    Moving { grab: Vec2 },
    Resizing { handle: Handle, orig: Rect },
    /// Drawing an annotation.
    Draw { id: u64, start: Pos2 },
    /// Dragging an existing annotation.
    MoveShape { id: u64, last: Pos2 },
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Selecting,
    Adjusting,
    Finishing { at: f64 },
    /// Cancelled: fading out before the window closes.
    Closing { at: f64 },
}

struct OverlayApp {
    shared: Arc<Shared>,
    cfg: Config,
    snap: Snapshot,
    tex: TiledTexture,
    loupe: Option<TextureHandle>,
    hwnd: Option<win::HWND>,
    target: IRect,
    frame_no: u32,
    /// Uncloaked yet; `stable` counts frames with the window in place.
    revealed: bool,
    stable: u32,
    shown_frame: u32,
    /// Selection in image pixels (always integer-aligned).
    sel: Option<Rect>,
    drag: Drag,
    phase: Phase,
    out: Rc<RefCell<Option<Output>>>,
    toolbar_rect: Option<Rect>,
    notice: Option<(String, f64)>,
    last_pointer: Option<Pos2>,
    /// Edge detection for keys polled directly when Windows withheld focus.
    keys_down: [bool; 2],
    t0: f64,
    last_t: f64,
    /// Animated focus frame (points).
    anim: Option<Rect>,
    adjust_at: f64,
    was_adjusting: bool,
    /// Animation clock: advances by at most one 60 Hz frame per rendered frame,
    /// so a slow first frame (GPU warm-up) can't skip the fade-in.
    clock: f64,
    tool: Tool,
    shapes: Vec<Shape>,
    next_id: u64,
    color: eframe::egui::Color32,
    typing: Option<TypingText>,
    prims: PrimPainter,
    top_rect: Option<Rect>,
    /// Monitor (points) the top bar lives on: where the cursor was at open.
    bar_mon: Option<Rect>,
    last_ppp: f32,
    record: bool,
    gif: bool,
    /// Pointer trail of the arrow being drawn (bends a Shift arrow).
    draw_path: Vec<Pos2>,
    /// Window picked by a click: (its rect in image px, handle).
    clicked_window: Option<(Rect, isize)>,
    /// Animated position of the tools panel (points).
    bar_pos: Option<Pos2>,
    /// Annotation under the pointer (grab to move, Delete to remove).
    hover_shape: Option<u64>,
    picked: Option<u64>,
    /// Scripted README run.
    demo: Option<crate::demo::Player>,
}

/// Loupe magnification steps (points per screen pixel); 0 = hidden.
const LOUPE_STEPS: [f32; 8] = [0.0, 4.0, 5.0, 7.0, 9.0, 12.0, 16.0, 22.0];
/// Approximate side of the loupe image, in points.
const LOUPE_SIDE: f32 = 117.0;
const FLASH: f64 = 0.2;
const APPEAR: f64 = 0.22;
/// First frame on which the overlay may be uncloaked.
const SHOW_FRAME: u32 = 3;
/// Annotation thickness steps for the mouse wheel.
const WIDTH_STEPS: [f32; 7] = [2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0];
const CLOSE: f64 = 0.14;

impl OverlayApp {
    fn img_rect(&self) -> Rect {
        Rect::from_min_size(Pos2::ZERO, self.tex.size)
    }

    fn clamp_rect(&self, r: Rect) -> Rect {
        let r = r.intersect(self.img_rect());
        Rect::from_min_max(r.min.round(), r.max.round())
    }

    /// Window under the pointer, if any.
    fn window_at(&self, p: Pos2) -> Option<(Rect, isize)> {
        let (x, y) = (p.x as i32, p.y as i32);
        self.snap.windows.iter().find(|w| w.rect.contains(x, y)).map(|w| (irect_to_rect(w.rect), w.hwnd))
    }

    /// Window or monitor under the pointer (image px).
    fn hover_target(&self, p: Pos2) -> Rect {
        let (x, y) = (p.x as i32, p.y as i32);
        let r = self
            .snap
            .windows
            .iter()
            .find(|w| w.rect.contains(x, y))
            .map(|w| w.rect)
            .unwrap_or_else(|| self.snap.monitor_at(x, y).rect);
        irect_to_rect(r)
    }

    fn handle_at(&self, p: Pos2, ppp: f32) -> Option<Handle> {
        let s = self.sel?;
        let tol = 8.0 * ppp;
        let near = |a: f32, b: f32| (a - b).abs() <= tol;
        let in_x = p.x >= s.left() - tol && p.x <= s.right() + tol;
        let in_y = p.y >= s.top() - tol && p.y <= s.bottom() + tol;
        let (l, r, t, b) = (near(p.x, s.left()), near(p.x, s.right()), near(p.y, s.top()), near(p.y, s.bottom()));
        Some(match () {
            _ if l && t => Handle::NW,
            _ if r && t => Handle::NE,
            _ if l && b => Handle::SW,
            _ if r && b => Handle::SE,
            _ if t && in_x => Handle::N,
            _ if b && in_x => Handle::S,
            _ if l && in_y => Handle::W,
            _ if r && in_y => Handle::E,
            _ => return None,
        })
    }

    fn finish(&mut self, action: Action, now: f64) {
        let action = if self.record { Action::Record } else { action };
        let Some(s) = self.sel else { return };
        if s.width() < 1.0 || s.height() < 1.0 {
            return;
        }
        self.commit_text();
        let mut image =
            image::imageops::crop_imm(&self.snap.image, s.min.x as u32, s.min.y as u32, s.width() as u32, s.height() as u32)
                .to_image();
        if !self.shapes.is_empty() {
            let d = -s.min.to_vec2();
            let shapes: Vec<Shape> = self
                .shapes
                .iter()
                .cloned()
                .map(|mut sh| {
                    sh.translate(d);
                    sh
                })
                .collect();
            image = crate::editor::render::render(&image, &shapes, None);
        }
        let c = s.center();
        let m = self.snap.monitor_at(c.x as i32, c.y as i32);
        let (ox, oy) = self.snap.origin;
        let monitor = Monitor { rect: m.rect.offset(ox, oy), work: m.work.offset(ox, oy), scale: m.scale };
        let rect = IRect::new(s.min.x as i32 + ox, s.min.y as i32 + oy, s.width() as i32, s.height() as i32);
        let window = self.clicked_window.filter(|(r, _)| *r == s).map(|(_, h)| h);
        *self.out.borrow_mut() = Some(Output { action, image, rect, monitor, window, gif: self.gif });
        self.phase = Phase::Finishing { at: now };
        if !self.record {
            crate::sound::play_capture(&self.shared.config());
        }
    }

    fn cancel(&mut self, ctx: &egui::Context) {
        *self.out.borrow_mut() = None;
        if !matches!(self.phase, Phase::Closing { .. }) {
            self.phase = Phase::Closing { at: ctx.input(|i| i.time) };
            ctx.request_repaint();
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context, ppp: f32, now: f64) {
        if matches!(self.phase, Phase::Finishing { .. } | Phase::Closing { .. }) {
            return;
        }
        let input = ctx.input(|i| i.clone());
        let pointer_pt = input.pointer.hover_pos().or(self.last_pointer);
        if let Some(p) = input.pointer.hover_pos() {
            self.last_pointer = Some(p);
        }
        let p = pointer_pt.map(|p| pos2(p.x * ppp, p.y * ppp));
        let over_toolbar = pointer_pt.is_some_and(|p| {
            self.toolbar_rect.is_some_and(|r| r.contains(p)) || self.top_rect.is_some_and(|r| r.contains(p))
        }) && matches!(self.drag, Drag::None);

        // Typing a label: keys go into the text, rendered live exactly as it will look.
        if self.typing.is_some() {
            let mut commit = false;
            let mut cancel = false;
            for e in &input.raw.events {
                let Some(t) = self.typing.as_mut() else { break };
                match e {
                    egui::Event::Text(txt) => t.buf.extend(txt.chars().filter(|c| !c.is_control())),
                    egui::Event::Paste(txt) => t.buf.push_str(txt),
                    egui::Event::Key { key, pressed: true, modifiers, .. } => match key {
                        Key::Backspace => {
                            t.buf.pop();
                        }
                        Key::Enter if modifiers.shift => t.buf.push('\n'),
                        Key::Enter => commit = true,
                        Key::Escape => cancel = true,
                        Key::V if modifiers.command => {
                            if let Ok(txt) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                                t.buf.push_str(&txt);
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            self.apply_wheel(&input, now);
            if commit {
                self.commit_text();
            } else if cancel {
                self.typing = None;
            } else if input.pointer.primary_pressed() && !over_toolbar {
                self.commit_text();
                if let Some(p) = pointer_pt.map(|p| pos2(p.x * ppp, p.y * ppp))
                    && matches!(self.tool, Tool::Text | Tool::TextBox)
                {
                    self.start_text(p, ppp);
                }
            }
            return;
        }

        // --- keyboard
        // If Windows didn't give us focus, read Esc / Enter straight from the keyboard.
        let (mut esc, mut enter) = (false, false);
        if !input.viewport().focused.unwrap_or(true) {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN};
            let now_down = [win::key_down(VK_ESCAPE as u32), win::key_down(VK_RETURN as u32)];
            esc = now_down[0] && !self.keys_down[0];
            enter = now_down[1] && !self.keys_down[1];
            self.keys_down = now_down;
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        // Mouse wheel: loupe zoom, down to 0 = hidden.
        let wheel: f32 = input
            .raw
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                _ => None,
            })
            .sum();
        if self.tool != Tool::Region {
            self.apply_wheel(&input, now);
        } else if wheel != 0.0 && self.cfg.magnifier {
            let cur = LOUPE_STEPS
                .iter()
                .position(|s| (*s - self.cfg.loupe_cell).abs() < 0.01)
                .unwrap_or(4) as i32;
            let next = (cur + if wheel > 0.0 { 1 } else { -1 }).clamp(0, LOUPE_STEPS.len() as i32 - 1) as usize;
            if next as i32 != cur {
                let v = LOUPE_STEPS[next];
                self.cfg.loupe_cell = v;
                self.shared.update_config(|c| c.loupe_cell = v);
                if v == 0.0 {
                    self.notice = Some((crate::i18n::t("лупа скрыта · колесо вверх вернёт").into(), now));
                }
            }
        }
        let cmd = input.modifiers.command;
        if cmd && input.key_pressed(Key::Z) {
            self.shapes.pop();
        }
        if input.key_pressed(Key::Delete) || input.key_pressed(Key::Backspace) {
            if let Some(id) = self.hover_shape.or(self.picked) {
                self.shapes.retain(|s| s.id != id);
                self.hover_shape = None;
                self.picked = None;
            }
        }
        for (i, k) in [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ]
        .into_iter()
        .enumerate()
        {
            if !cmd && !self.record && input.key_pressed(k) {
                self.tool = TOOLS[i].0;
            }
        }
        if input.key_pressed(Key::Escape) || esc {
            self.cancel(ctx);
            return;
        }
        if self.sel.is_none()
            && self.tool != Tool::Region
            && (input.key_pressed(Key::Enter) || enter)
            && let Some(p) = p
        {
            self.sel = Some(self.clamp_rect(self.hover_target(p)));
            self.phase = Phase::Adjusting;
            return self.finish(Action::Copy, now);
        }
        if self.sel.is_some() && self.phase == Phase::Adjusting {
            if input.key_pressed(Key::Enter) || enter || (cmd && input.key_pressed(Key::C)) {
                return self.finish(Action::Copy, now);
            }
            if cmd && input.key_pressed(Key::S) {
                return self.finish(Action::Save, now);
            }
            if input.key_pressed(Key::E) {
                return self.finish(Action::Edit, now);
            }
            if input.key_pressed(Key::P) {
                return self.finish(Action::Pin, now);
            }
            if input.key_pressed(Key::B) {
                return self.finish(Action::Beauty, now);
            }
            if input.key_pressed(Key::U) && !self.shared.config().upload_url.trim().is_empty() {
                return self.finish(Action::Upload, now);
            }
            let step = if input.modifiers.shift { 10.0 } else { 1.0 };
            let mut d = Vec2::ZERO;
            for (k, v) in [
                (Key::ArrowLeft, vec2(-step, 0.0)),
                (Key::ArrowRight, vec2(step, 0.0)),
                (Key::ArrowUp, vec2(0.0, -step)),
                (Key::ArrowDown, vec2(0.0, step)),
            ] {
                if input.key_pressed(k) {
                    d += v;
                }
            }
            if d != Vec2::ZERO {
                let s = self.sel.unwrap();
                let img = self.img_rect();
                let t = s.translate(d);
                let dx = (img.left() - t.left()).max(0.0) - (t.right() - img.right()).max(0.0);
                let dy = (img.top() - t.top()).max(0.0) - (t.bottom() - img.bottom()).max(0.0);
                self.sel = Some(t.translate(vec2(dx, dy)));
            }
        } else if input.key_pressed(Key::C)
            && let Some(p) = p
        {
            let px = self.snap.image.get_pixel(
                (p.x as u32).min(self.snap.image.width() - 1),
                (p.y as u32).min(self.snap.image.height() - 1),
            );
            let hex = format!("#{:02x}{:02x}{:02x}", px[0], px[1], px[2]);
            crate::output::copy_text(&hex);
            self.notice = Some((lf!("{hex} скопирован", "{hex} copied"), now));
        }
        if cmd && input.key_pressed(Key::A) {
            self.sel = Some(self.img_rect());
            self.phase = Phase::Adjusting;
        }
        if input.key_pressed(Key::Space)
            && let Some(p) = p
        {
            self.sel = Some(irect_to_rect(self.snap.monitor_at(p.x as i32, p.y as i32).rect));
            self.phase = Phase::Adjusting;
            if self.cfg.instant_mode {
                return self.finish(Action::Copy, now);
            }
        }

        if input.pointer.secondary_clicked() {
            if self.sel.is_some() {
                self.sel = None;
                self.phase = Phase::Selecting;
                self.drag = Drag::None;
            } else {
                self.cancel(ctx);
            }
            return;
        }

        let Some(p) = p else { return };

        if self.phase == Phase::Adjusting
            && input.pointer.button_double_clicked(egui::PointerButton::Primary)
            && self.sel.is_some_and(|s| s.contains(p))
            && !over_toolbar
        {
            return self.finish(Action::Copy, now);
        }

        // Annotations can be grabbed and moved with any tool (Ctrl = the selection).
        if let Drag::MoveShape { id, last } = self.drag {
            if input.pointer.primary_down() {
                if let Some(sh) = self.shapes.iter_mut().find(|s| s.id == id) {
                    sh.translate(p - last);
                }
                self.drag = Drag::MoveShape { id, last: p };
            } else {
                self.drag = Drag::None;
            }
            ctx.set_cursor_icon(CursorIcon::Move);
            return;
        }
        self.hover_shape = if matches!(self.drag, Drag::None) && !over_toolbar && !input.modifiers.ctrl {
            self.shape_at(p, ppp)
        } else {
            None
        };
        if let Some(id) = self.hover_shape {
            if input.pointer.primary_pressed() {
                self.drag = Drag::MoveShape { id, last: p };
                self.picked = Some(id);
            }
            ctx.set_cursor_icon(CursorIcon::Move);
            return;
        }

        // Annotation tools draw anywhere (Ctrl still moves the selection).
        if self.tool != Tool::Region && !input.modifiers.ctrl {
            self.draw_input(&input, p, ppp, over_toolbar);
            ctx.set_cursor_icon(if over_toolbar {
                CursorIcon::Default
            } else if matches!(self.tool, Tool::Text | Tool::TextBox) {
                CursorIcon::Text
            } else {
                CursorIcon::Crosshair
            });
            return;
        }

        if input.pointer.primary_pressed() && !over_toolbar {
            let ctrl = input.modifiers.ctrl;
            self.drag = match (self.phase, self.handle_at(p, ppp), self.sel) {
                // Ctrl + drag anywhere moves the existing selection.
                (Phase::Adjusting, _, Some(s)) if ctrl => Drag::Moving { grab: s.min - p },
                (Phase::Adjusting, Some(h), Some(s)) => Drag::Resizing { handle: h, orig: s },
                (Phase::Adjusting, None, Some(s)) if s.contains(p) => Drag::Moving { grab: s.min - p },
                _ => Drag::Pending { start: p },
            };
        }

        if input.pointer.primary_down() {
            match self.drag {
                Drag::Pending { start } if (p - start).length() > 4.0 * ppp => {
                    self.drag = Drag::Creating { start, last: p };
                    self.phase = Phase::Selecting;
                    self.sel = Some(self.clamp_rect(Rect::from_two_pos(start, p)));
                }
                Drag::Creating { mut start, last } => {
                    if input.modifiers.ctrl {
                        // Holding Ctrl: drag the rectangle around instead of resizing it.
                        let img = self.img_rect();
                        let moved = start + (p - last);
                        start = pos2(moved.x.clamp(img.left(), img.right()), moved.y.clamp(img.top(), img.bottom()));
                    }
                    self.drag = Drag::Creating { start, last: p };
                    let mut end = p;
                    if input.modifiers.shift {
                        let d = end - start;
                        let m = d.x.abs().max(d.y.abs());
                        end = start + vec2(m * d.x.signum(), m * d.y.signum());
                    }
                    self.sel = Some(self.clamp_rect(Rect::from_two_pos(start, end)));
                }
                Drag::Moving { grab } => {
                    if let Some(s) = self.sel {
                        let img = self.img_rect();
                        let mut min = (p + grab).round();
                        min.x = min.x.clamp(0.0, img.right() - s.width());
                        min.y = min.y.clamp(0.0, img.bottom() - s.height());
                        self.sel = Some(Rect::from_min_size(min, s.size()));
                    }
                }
                Drag::Resizing { handle, orig } => {
                    let (mut l, mut t, mut r, mut b) = (orig.left(), orig.top(), orig.right(), orig.bottom());
                    match handle {
                        Handle::N => t = p.y,
                        Handle::S => b = p.y,
                        Handle::W => l = p.x,
                        Handle::E => r = p.x,
                        Handle::NW => (l, t) = (p.x, p.y),
                        Handle::NE => (r, t) = (p.x, p.y),
                        Handle::SW => (l, b) = (p.x, p.y),
                        Handle::SE => (r, b) = (p.x, p.y),
                    }
                    self.sel = Some(self.clamp_rect(Rect::from_two_pos(pos2(l, t), pos2(r, b))));
                }
                _ => {}
            }
        }

        if input.pointer.primary_released() {
            match self.drag {
                Drag::Pending { .. } => {
                    self.clicked_window = self.window_at(p).map(|(r, h)| (self.clamp_rect(r), h));
                    self.sel = Some(self.clamp_rect(self.hover_target(p)));
                    self.phase = Phase::Adjusting;
                    if self.cfg.instant_mode {
                        self.drag = Drag::None;
                        return self.finish(Action::Copy, now);
                    }
                }
                Drag::Creating { .. } => {
                    if self.sel.is_some_and(|s| s.width() >= 2.0 && s.height() >= 2.0) {
                        self.phase = Phase::Adjusting;
                        if self.cfg.instant_mode {
                            self.drag = Drag::None;
                            return self.finish(Action::Copy, now);
                        }
                    } else {
                        self.sel = None;
                        self.phase = Phase::Selecting;
                    }
                }
                _ => {}
            }
            self.drag = Drag::None;
        }

        let icon = if over_toolbar {
            CursorIcon::Default
        } else {
            match self.drag {
                Drag::Moving { .. } => CursorIcon::Move,
                Drag::Creating { .. } if input.modifiers.ctrl => CursorIcon::Move,
                Drag::None if self.phase == Phase::Adjusting && input.modifiers.ctrl && self.sel.is_some() => {
                    CursorIcon::Move
                }
                Drag::Resizing { handle, .. } => handle.cursor(),
                Drag::None if self.phase == Phase::Adjusting => match self.handle_at(p, ppp) {
                    Some(h) => h.cursor(),
                    None if self.sel.is_some_and(|s| s.contains(p)) => CursorIcon::Move,
                    None => CursorIcon::Crosshair,
                },
                _ => CursorIcon::Crosshair,
            }
        };
        ctx.set_cursor_icon(icon);
    }

    fn paint(&mut self, ctx: &egui::Context, ppp: f32, now: f64) {
        let painter = ctx.layer_painter(egui::LayerId::background());
        let to_pt = |r: Rect| Rect::from_min_max(pos2(r.min.x / ppp, r.min.y / ppp), pos2(r.max.x / ppp, r.max.y / ppp));
        let full = to_pt(self.img_rect());
        let pointer_pt = self.last_pointer;
        let p_px = pointer_pt.map(|p| pos2(p.x * ppp, p.y * ppp));
        let finishing = match self.phase {
            Phase::Finishing { at } => Some(((now - at) / FLASH).clamp(0.0, 1.0) as f32),
            _ => None,
        };
        let closing = match self.phase {
            Phase::Closing { at } => Some(((now - at) / CLOSE).clamp(0.0, 1.0) as f32),
            _ => None,
        };
        // Overall presence of the overlay chrome: fades in after the hotkey,
        // fades out on cancel and while the capture "blinks".
        let appear = ui::ease_out_cubic((self.clock / APPEAR) as f32);
        let fade = appear * (1.0 - closing.unwrap_or(0.0)) * (1.0 - finishing.unwrap_or(0.0));

        // frozen desktop (identical to the real screen, so the fade is seamless)
        self.tex.paint(&painter, self.img_rect(), full, Color32::WHITE);

        // annotations live on the frozen desktop, under the veil
        let view = View { offset: Vec2::ZERO, zoom: 1.0 / ppp };
        for sh in &self.shapes {
            for prim in sh.prims() {
                self.prims.paint(&painter, ctx, &self.snap.image, &prim, view);
            }
        }
        self.paint_typing(ctx, &painter, ppp);
        self.prims.end_frame();

        // focus frame: glides between windows while hovering, follows the mouse exactly while dragging
        let region_hover = self.tool == Tool::Region
            && self.phase == Phase::Selecting
            && matches!(self.drag, Drag::None | Drag::Pending { .. });
        // Drawing without a selection: the window being drawn on stays lit
        // (locked to where the stroke started while drawing).
        let draw_hover = self.tool != Tool::Region && self.phase == Phase::Selecting && self.sel.is_none();
        let hovering = region_hover || draw_hover;
        let hover_at = match self.drag {
            Drag::Draw { start, .. } if draw_hover => Some(start),
            _ => p_px,
        };
        let target = if hovering { hover_at.map(|p| self.hover_target(p)) } else { self.sel }.map(to_pt);
        let dt = (now - self.last_t).clamp(0.0, 0.1) as f32;
        self.last_t = now;
        self.anim = match (target, self.anim) {
            (Some(t), Some(a)) if hovering => {
                let k = 1.0 - (-dt * 18.0).exp();
                let r = Rect::from_min_max(a.min + (t.min - a.min) * k, a.max + (t.max - a.max) * k);
                if (r.min - t.min).length() + (r.max - t.max).length() < 0.6 {
                    Some(t)
                } else {
                    ctx.request_repaint();
                    Some(r)
                }
            }
            (t, _) => t,
        };

        let veil = Color32::from_rgba_unmultiplied(12, 12, 11, (150.0 * fade) as u8);
        match self.anim {
            Some(f) => {
                for r in [
                    Rect::from_min_max(full.min, pos2(full.max.x, f.min.y)),
                    Rect::from_min_max(pos2(full.min.x, f.max.y), full.max),
                    Rect::from_min_max(pos2(full.min.x, f.min.y), pos2(f.min.x, f.max.y)),
                    Rect::from_min_max(pos2(f.max.x, f.min.y), pos2(full.max.x, f.max.y)),
                ] {
                    if r.is_positive() {
                        painter.rect_filled(r, 0, veil);
                    }
                }
                let strength = if draw_hover {
                    0.3
                } else if hovering {
                    0.55
                } else {
                    1.0
                };
                paint_viewfinder(&painter, f, fade * strength);
                if !hovering
                    && finishing.is_none()
                    && let Some(s) = self.sel
                {
                    self.paint_size_label(ctx, f, s, fade);
                }
            }
            None => {
                painter.rect_filled(full, 0, veil);
            }
        }

        // outline of the annotation under the pointer / being moved
        let outlined = match self.drag {
            Drag::MoveShape { id, .. } => Some(id),
            _ => self.hover_shape,
        };
        if finishing.is_none()
            && let Some(sh) = outlined.and_then(|id| self.shapes.iter().find(|s| s.id == id))
        {
            let b = view.rect(sh.bounds(crate::editor::render::text_box)).expand(4.0);
            let pts = vec![b.left_top(), b.right_top(), b.right_bottom(), b.left_bottom(), b.left_top()];
            painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0, ACCENT.gamma_multiply(0.85 * fade)), 4.0, 3.0));
        }

        // blink
        if let (Some(t), Some(s), false) = (finishing, self.sel, self.record) {
            let a = 0.35 * (1.0 - t);
            painter.rect_filled(to_pt(s), 0, Color32::from_white_alpha((255.0 * a) as u8));
        }

        // loupe
        let precise = self.tool == Tool::Region
            && (matches!(self.drag, Drag::Creating { .. } | Drag::Resizing { .. })
                || (self.phase == Phase::Selecting && !matches!(self.drag, Drag::Moving { .. })));
        if self.cfg.magnifier
            && self.cfg.loupe_cell > 0.0
            && precise
            && finishing.is_none()
            && closing.is_none()
            && let (Some(pt), Some(px)) = (pointer_pt, p_px)
        {
            self.paint_loupe(ctx, pt, px, ppp);
        }

        // transient notice (color copied)
        if let Some((text, at)) = &self.notice {
            if now - at > 1.4 {
                self.notice = None;
            } else {
                let fg = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::unique("ov-notice")));
                let at_pt = pointer_pt.unwrap_or(full.center()) + vec2(14.0, -30.0);
                let galley = fg.layout_no_wrap(text.clone(), font_medium(12.5), TEXT);
                let rect = Rect::from_min_size(at_pt, galley.size() + vec2(16.0, 10.0));
                fg.rect_filled(rect, 4, Color32::from_rgba_unmultiplied(18, 18, 17, 225));
                fg.galley(rect.min + vec2(8.0, 5.0), galley, TEXT);
                ctx.request_repaint();
            }
        }

        if appear < 1.0 || finishing.is_some() || closing.is_some() {
            ctx.request_repaint();
        }
    }

    /// Dark rounded pill with the size above the frame's top-left corner.
    fn paint_size_label(&self, ctx: &egui::Context, r: Rect, s: Rect, alpha: f32) {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("label")));
        let text = format!("{} × {}", s.width() as i32, s.height() as i32);
        let galley = painter.layout_no_wrap(text, font_medium(12.5), Color32::PLACEHOLDER);
        let size = galley.size() + vec2(18.0, 10.0);
        let mut pos = pos2(r.left(), r.top() - 10.0 - size.y);
        if pos.y < 4.0 {
            pos = r.left_top() + vec2(10.0, 10.0);
        }
        let rect = Rect::from_min_size(pos, size);
        painter.rect_filled(rect, 4, Color32::from_rgba_unmultiplied(18, 18, 17, (225.0 * alpha) as u8));
        painter.galley(rect.min + vec2(9.0, 5.0), galley, TEXT.gamma_multiply(alpha));
    }

    fn paint_loupe(&mut self, ctx: &egui::Context, pt: Pos2, px: Pos2, ppp: f32) {
        let cell = self.cfg.loupe_cell.max(1.0);
        let mut cells = (LOUPE_SIDE / cell).floor() as i32;
        if cells % 2 == 0 {
            cells += 1;
        }
        let cells = cells.max(3);
        let img = &self.snap.image;
        let (cx, cy) = (px.x.floor() as i32, px.y.floor() as i32);
        let half = cells / 2;
        let mut pixels = Vec::with_capacity((cells * cells) as usize);
        for y in cy - half..=cy + half {
            for x in cx - half..=cx + half {
                if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
                    let p = img.get_pixel(x as u32, y as u32);
                    pixels.push(Color32::from_rgb(p[0], p[1], p[2]));
                } else {
                    pixels.push(Color32::BLACK);
                }
            }
        }
        let ci = egui::ColorImage::new([cells as usize, cells as usize], pixels);
        match &mut self.loupe {
            Some(t) => t.set(ci, TextureOptions::NEAREST),
            None => self.loupe = Some(ctx.load_texture("loupe", ci, TextureOptions::NEAREST)),
        }
        let tex = self.loupe.as_ref().unwrap();

        let side = cell * cells as f32;
        let pad = 4.0;
        let outer = vec2(side + pad * 2.0, side + pad * 2.0 + 20.0);
        let mon = self.snap.monitor_at(cx, cy).rect;
        let mon_pt = Rect::from_min_max(
            pos2(mon.x as f32 / ppp, mon.y as f32 / ppp),
            pos2(mon.right() as f32 / ppp, mon.bottom() as f32 / ppp),
        );
        let mut pos = pt + vec2(20.0, 20.0);
        if pos.x + outer.x > mon_pt.right() - 8.0 {
            pos.x = pt.x - 20.0 - outer.x;
        }
        if pos.y + outer.y > mon_pt.bottom() - 8.0 {
            pos.y = pt.y - 20.0 - outer.y;
        }
        let frame = Rect::from_min_size(pos, outer);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::unique("loupe")));
        painter.rect_filled(frame, 3, BG);
        painter.rect_stroke(frame, 3, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let img_rect = Rect::from_min_size(pos + vec2(pad, pad), Vec2::splat(side));
        painter.image(tex.id(), img_rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        let grid = Stroke::new(1.0, Color32::from_black_alpha(46));
        let grid_on = cell >= 6.0;
        for i in 1..cells {
            if !grid_on {
                break;
            }
            let o = i as f32 * cell;
            painter.line_segment([pos2(img_rect.left() + o, img_rect.top()), pos2(img_rect.left() + o, img_rect.bottom())], grid);
            painter.line_segment([pos2(img_rect.left(), img_rect.top() + o), pos2(img_rect.right(), img_rect.top() + o)], grid);
        }
        let c = Rect::from_min_size(img_rect.min + Vec2::splat(half as f32 * cell), Vec2::splat(cell));
        painter.rect_stroke(c, 0, Stroke::new(1.5, ACCENT), StrokeKind::Outside);

        let col = if cx >= 0 && cy >= 0 && (cx as u32) < img.width() && (cy as u32) < img.height() {
            *img.get_pixel(cx as u32, cy as u32)
        } else {
            image::Rgba([0, 0, 0, 255])
        };
        let row_y = img_rect.bottom() + 11.0;
        let sw = Rect::from_center_size(pos2(img_rect.left() + 6.0, row_y), Vec2::splat(9.0));
        painter.rect_filled(sw, 0, Color32::from_rgb(col[0], col[1], col[2]));
        painter.text(
            pos2(sw.right() + 6.0, row_y),
            Align2::LEFT_CENTER,
            format!("#{:02x}{:02x}{:02x}", col[0], col[1], col[2]),
            mono(11.0),
            TEXT,
        );
        painter.text(
            pos2(img_rect.right(), row_y),
            Align2::RIGHT_CENTER,
            format!("{cx},{cy}  ×{:.0}", cell * ppp),
            mono(11.0),
            FAINT,
        );
    }

    /// Vertical icon toolbar to the right of the selection (left if no room).
    fn toolbar(&mut self, ctx: &egui::Context, ppp: f32, now: f64) {
        let Some(s) = self.sel else { return };
        if !matches!(self.drag, Drag::None) {
            self.toolbar_rect = None;
            return;
        }
        let r = Rect::from_min_max(pos2(s.min.x / ppp, s.min.y / ppp), pos2(s.max.x / ppp, s.max.y / ppp));
        let c = s.center();
        let mon = self.snap.monitor_at(c.x as i32, c.y as i32).rect;
        let mon = Rect::from_min_max(
            pos2(mon.x as f32 / ppp, mon.y as f32 / ppp),
            pos2(mon.right() as f32 / ppp, mon.bottom() as f32 / ppp),
        );
        let size = self.toolbar_rect.map(|t| t.size()).unwrap_or(vec2(44.0, 214.0));
        let tools_right = !self.record && self.cfg.tools_pos == crate::config::ToolsPos::Right;
        let mut x = if tools_right { r.left() - size.x - 8.0 } else { r.right() + 8.0 };
        if tools_right && x < mon.left() + 8.0 {
            x = r.left() + 8.0;
        } else if !tools_right && x + size.x > mon.right() - 8.0 {
            x = if r.left() - size.x - 8.0 >= mon.left() + 8.0 {
                r.left() - size.x - 8.0
            } else {
                r.right() - size.x - 8.0
            };
        }
        let y = r.top().clamp(mon.top() + 8.0, (mon.bottom() - size.y - 8.0).max(mon.top() + 8.0));

        let t = ui::ease_out_cubic(((now - self.adjust_at) / 0.18) as f32);
        if t < 1.0 {
            ctx.request_repaint();
        }
        let resp = egui::Area::new(Id::unique("toolbar"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(x - (1.0 - t) * 8.0, y))
            .show(ctx, |ui| {
                ui.set_opacity(t);
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, LINE))
                    .corner_radius(3)
                    .inner_margin(egui::Margin::same(3))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.vertical(|ui| {
                            let mut act = None;
                            if self.record {
                                let tip = if self.gif { crate::i18n::t("начать запись gif  enter") } else { crate::i18n::t("начать запись  enter") };
                                if ui::icon_button(ui, Icon::Record, tip, 36.0, false, ui::Kind::Accent).clicked() {
                                    act = Some(Action::Record);
                                }
                                if ui::icon_button(ui, Icon::Close, crate::i18n::t("отмена  esc"), 36.0, false, ui::Kind::Ghost).clicked() {
                                    self.cancel(ctx);
                                }
                                return act;
                            }
                            if ui::icon_button(ui, Icon::Copy, crate::i18n::t("копировать  enter"), 36.0, false, ui::Kind::Accent).clicked() {
                                act = Some(Action::Copy);
                            }
                            if ui::icon_button(ui, Icon::Save, crate::i18n::t("сохранить  ctrl s"), 36.0, false, ui::Kind::Ghost).clicked() {
                                act = Some(Action::Save);
                            }
                            if ui::icon_button(ui, Icon::Edit, crate::i18n::t("править  e"), 36.0, false, ui::Kind::Ghost).clicked() {
                                act = Some(Action::Edit);
                            }
                            if ui::icon_button(ui, Icon::Pin, crate::i18n::t("закрепить поверх окон  p"), 36.0, false, ui::Kind::Ghost).clicked() {
                                act = Some(Action::Pin);
                            }
                            if ui::icon_button(ui, Icon::Frame, crate::i18n::t("красивый снимок  b"), 36.0, false, ui::Kind::Ghost)
                                .clicked()
                            {
                                act = Some(Action::Beauty);
                            }
                            if !self.cfg.upload_url.trim().is_empty()
                                && ui::icon_button(ui, Icon::Upload, crate::i18n::t("загрузить и скопировать ссылку  u"), 36.0, false, ui::Kind::Ghost)
                                    .clicked()
                            {
                                act = Some(Action::Upload);
                            }
                            let (sep, _) = ui.allocate_exact_size(vec2(36.0, 5.0), egui::Sense::hover());
                            ui.painter().hline(sep.shrink2(vec2(4.0, 0.0)).x_range(), sep.center().y, Stroke::new(1.0, LINE));
                            if ui::icon_button(ui, Icon::Close, crate::i18n::t("отмена  esc"), 36.0, false, ui::Kind::Ghost).clicked() {
                                self.cancel(ctx);
                            }
                            act
                        })
                        .inner
                    })
                    .inner
            });
        self.toolbar_rect = Some(resp.response.rect);
        if let Some(a) = resp.inner {
            self.finish(a, now);
        }
    }
}

fn irect_to_rect(r: IRect) -> Rect {
    Rect::from_min_size(pos2(r.x as f32, r.y as f32), vec2(r.w as f32, r.h as f32))
}

impl eframe::App for OverlayApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(d) = &mut self.demo {
            let view = crate::demo::View {
                tools: self.top_rect,
                actions: self.toolbar_rect,
                cursor: ctx.output(|o| o.cursor_icon),
            };
            d.feed(ctx, raw, &view);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame_no += 1;
        let now = ctx.input(|i| i.time);
        if self.t0 < 0.0 {
            self.t0 = now;
            self.last_t = now;
            let (cx, cy) = win::cursor_pos();
            let m = self.snap.monitor_at(cx - self.snap.origin.0, cy - self.snap.origin.1).rect;
            let ppp0 = ctx.pixels_per_point();
            self.bar_mon = Some(Rect::from_min_size(
                pos2(m.x as f32 / ppp0, m.y as f32 / ppp0),
                vec2(m.w as f32 / ppp0, m.h as f32 / ppp0),
            ));
        }
        // The window becomes visible after its first frame: start the clock then,
        // and never let one slow frame jump the animation.
        if self.revealed {
            let dt = ctx.input(|i| i.unstable_dt) as f64;
            self.clock += dt.clamp(0.0, 1.0 / 60.0);
        }
        if let Some(h) = self.hwnd {
            // Guard against DPI-change resizes moving/resizing us across monitors.
            let placed = win::window_rect(h) == self.target;
            if !placed && self.frame_no <= 90 {
                win::set_window_rect(h, self.target, true);
            }
            if self.demo.is_some() {
                // stays cloaked: the frames are taken as screenshots
            } else if !self.revealed {
                // Uncloak only after two frames were drawn at the final size.
                self.stable = if self.frame_no >= SHOW_FRAME && placed { self.stable + 1 } else { 0 };
                if self.stable >= 2 || self.frame_no > 45 {
                    win::cloak(h, false);
                    self.revealed = true;
                    self.shown_frame = self.frame_no;
                    win::force_foreground(h);
                }
                ctx.request_repaint();
            } else if matches!(self.frame_no - self.shown_frame, 4 | 12 | 28) && !win::is_foreground(h) {
                win::force_foreground(h);
            }
        }
        // Hotkeys pressed while selecting are ignored; quitting closes us.
        self.shared.take_where(|c| c.is_capture() || *c == Command::Record);
        if self.shared.peek() == Some(Command::Quit) {
            *self.out.borrow_mut() = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let ppp = ctx.pixels_per_point();
        self.handle_input(&ctx, ppp, now);
        self.paint(&ctx, ppp, now);
        self.tools_bar(&ctx);
        self.last_ppp = ppp;
        let adjusting = self.phase == Phase::Adjusting;
        if adjusting && !self.was_adjusting {
            self.adjust_at = now;
        }
        self.was_adjusting = adjusting;
        if adjusting {
            self.toolbar(&ctx, ppp, now);
        } else {
            self.toolbar_rect = None;
        }
        if let Phase::Closing { at } = self.phase
            && now - at >= CLOSE
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if let Phase::Finishing { at } = self.phase
            && now - at >= FLASH
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if let Some(d) = &self.demo {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(d.shot_request()));
            ctx.request_repaint();
        }
    }
}

/// Viewfinder frame: dashed hairline plus solid corner brackets.
fn paint_viewfinder(painter: &egui::Painter, r: Rect, alpha: f32) {
    if alpha <= 0.0 || r.width() < 1.0 || r.height() < 1.0 {
        return;
    }
    let line = Stroke::new(1.0, ACCENT.gamma_multiply(0.85 * alpha));
    let pts = vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    painter.extend(egui::Shape::dashed_line(&pts, line, 6.0, 4.0));
    let len = 18.0f32.min(r.width() / 2.5).min(r.height() / 2.5).max(5.0);
    let o = r.expand(1.5);
    let bracket = Stroke::new(2.5, ACCENT.gamma_multiply(alpha));
    for (c, dx, dy) in [
        (o.left_top(), 1.0, 1.0),
        (o.right_top(), -1.0, 1.0),
        (o.left_bottom(), 1.0, -1.0),
        (o.right_bottom(), -1.0, -1.0),
    ] {
        painter.line(vec![c + vec2(0.0, dy * len), c, c + vec2(dx * len, 0.0)], bracket);
    }
}

impl OverlayApp {
    fn shape_width(&self, ppp: f32) -> f32 {
        (self.cfg.editor_width * ppp.max(1.0)).round()
    }

    fn start_text(&mut self, p: Pos2, ppp: f32) {
        let lh = crate::editor::render::line_height(doc::text_size(self.shape_width(ppp)));
        self.typing = Some(TypingText {
            pos: p - vec2(0.0, lh * 0.5),
            buf: String::new(),
            plate: self.tool == Tool::TextBox,
        });
    }

    fn commit_text(&mut self) {
        let Some(t) = self.typing.take() else { return };
        let text = t.buf.trim_end().to_string();
        if text.is_empty() {
            return;
        }
        let width = self.cfg.editor_width * self.last_ppp.max(1.0);
        let kind = if t.plate { Kind::Text { pos: t.pos, text } } else { Kind::Label { pos: t.pos, text } };
        self.push_shape(kind, width.round());
    }

    fn push_shape(&mut self, kind: Kind, width: f32) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.shapes.push(Shape { id, kind, color: self.color, width });
        id
    }

    /// Pointer handling for the annotation tools.
    fn draw_input(&mut self, input: &egui::InputState, p: Pos2, ppp: f32, over_toolbar: bool) {
        let w = self.shape_width(ppp);
        if input.pointer.primary_pressed() && !over_toolbar {
            match self.tool {
                Tool::Steps => {
                    let n = self.shapes.iter().filter(|s| matches!(s.kind, Kind::Counter { .. })).count() as u32 + 1;
                    self.push_shape(Kind::Counter { c: p, n }, w);
                }
                Tool::Text | Tool::TextBox => self.start_text(p, ppp),
                Tool::Arrow => {
                    let id = self.push_shape(Kind::Arrow { a: p, b: p }, w);
                    self.draw_path = vec![p];
                    self.drag = Drag::Draw { id, start: p };
                }
                Tool::Pen => {
                    let id = self.push_shape(Kind::Pen { pts: vec![p] }, w);
                    self.drag = Drag::Draw { id, start: p };
                }
                Tool::Highlight | Tool::Blur | Tool::Redact => {
                    let r = Rect::from_two_pos(p, p);
                    let kind = match self.tool {
                        Tool::Highlight => Kind::Highlight { r },
                        Tool::Blur => Kind::Pixelate { r },
                        _ => Kind::Redact { r },
                    };
                    let id = self.push_shape(kind, w);
                    self.drag = Drag::Draw { id, start: p };
                }
                Tool::Region => {}
            }
        }
        if input.pointer.primary_down()
            && let Drag::Draw { id, start } = self.drag
        {
            let img = self.img_rect();
            if self.tool == Tool::Arrow && self.draw_path.last().is_none_or(|l| (*l - p).length() > 2.0) {
                self.draw_path.push(p);
            }
            // Shift bends the arrow along the way the mouse went.
            let curve = input.modifiers.shift.then(|| curve_control(start, p, &self.draw_path));
            let step = ppp.max(1.0);
            if let Some(sh) = self.shapes.iter_mut().find(|s| s.id == id) {
                if matches!(sh.kind, Kind::Arrow { .. } | Kind::Curve { .. }) {
                    sh.kind = match curve {
                        Some(c) => Kind::Curve { a: start, b: p, c },
                        None => Kind::Arrow { a: start, b: p },
                    };
                } else {
                    match &mut sh.kind {
                        Kind::Pen { pts } => {
                            if pts.last().is_none_or(|l| (*l - p).length() >= step) {
                                pts.push(p);
                            }
                        }
                        Kind::Highlight { r } | Kind::Pixelate { r } | Kind::Redact { r } => {
                            *r = Rect::from_two_pos(start, p).intersect(img)
                        }
                        _ => {}
                    }
                }
            }
        }
        if input.pointer.primary_released()
            && let Drag::Draw { id, .. } = self.drag
        {
            let min = 3.0 * ppp;
            let keep = self.shapes.iter().find(|s| s.id == id).is_some_and(|s| match &s.kind {
                Kind::Arrow { a, b } | Kind::Curve { a, b, .. } => (*a - *b).length() > min,
                Kind::Highlight { r } | Kind::Pixelate { r } | Kind::Redact { r } => r.width() > min && r.height() > min,
                _ => true,
            });
            if !keep {
                self.shapes.retain(|s| s.id != id);
            } else if let Some(sh) = self.shapes.iter_mut().find(|s| s.id == id)
                && let Kind::Pen { pts } = &mut sh.kind
            {
                *pts = doc::smooth(&doc::smooth(pts));
            }
            self.draw_path.clear();
            self.drag = Drag::None;
        }
    }

    /// Top bar: tools, colors, undo. Slides down with the overlay.
    fn tools_bar(&mut self, ctx: &egui::Context) {
        let Some(mon) = self.bar_mon else { return };
        let closing = match self.phase {
            Phase::Closing { at } | Phase::Finishing { at } => ((ctx.input(|i| i.time) - at) / CLOSE).clamp(0.0, 1.0) as f32,
            _ => 0.0,
        };
        let t = ui::ease_out_cubic(((self.clock - 0.04) / 0.3) as f32) * (1.0 - closing);
        if t <= 0.0 {
            self.top_rect = None;
            return;
        }
        let ppp = ctx.pixels_per_point();
        let size = self.top_rect.map(|r| r.size()).unwrap_or(vec2(520.0, 46.0));
        // Next to the selection once there is one (the screen top until then).
        let mode = if self.record { crate::config::ToolsPos::Screen } else { self.cfg.tools_pos };
        let area = self
            .sel
            .filter(|_| mode != crate::config::ToolsPos::Screen && self.phase != Phase::Selecting)
            .map(|s| Rect::from_min_max(pos2(s.min.x / ppp, s.min.y / ppp), pos2(s.max.x / ppp, s.max.y / ppp)));
        let vertical = mode == crate::config::ToolsPos::Right && area.is_some();
        let target = match area {
            None => pos2(mon.center().x - size.x / 2.0, mon.top() + 14.0),
            Some(r) => {
                let c = self.snap.monitor_at((r.center().x * ppp) as i32, (r.center().y * ppp) as i32).rect;
                let m = Rect::from_min_max(
                    pos2(c.x as f32 / ppp, c.y as f32 / ppp),
                    pos2(c.right() as f32 / ppp, c.bottom() as f32 / ppp),
                )
                .shrink(8.0);
                let gap = 10.0;
                let below = r.bottom() + gap;
                let above = r.top() - gap - size.y;
                let fits = |y: f32| y >= m.top() && y + size.y <= m.bottom();
                let x = (r.center().x - size.x / 2.0).clamp(m.left(), (m.right() - size.x).max(m.left()));
                match mode {
                    crate::config::ToolsPos::Below | crate::config::ToolsPos::Above => {
                        let order = if mode == crate::config::ToolsPos::Below { [below, above] } else { [above, below] };
                        let y = order.into_iter().find(|y| fits(*y)).unwrap_or(r.bottom() - size.y - gap);
                        pos2(x, y.clamp(m.top(), (m.bottom() - size.y).max(m.top())))
                    }
                    _ => {
                        let right = r.right() + gap;
                        let left = r.left() - gap - size.x;
                        let x = if right + size.x <= m.right() {
                            right
                        } else if left >= m.left() {
                            left
                        } else {
                            r.right() - size.x - gap
                        };
                        pos2(x, r.top().clamp(m.top(), (m.bottom() - size.y).max(m.top())))
                    }
                }
            }
        };
        // glide to the new place instead of jumping
        let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.05);
        let cur = self.bar_pos.unwrap_or(target);
        let k = 1.0 - (-dt * 16.0).exp();
        let next = cur + (target - cur) * k;
        let next = if (next - target).length() < 0.5 { target } else { next };
        if next != target {
            ctx.request_repaint();
        }
        self.bar_pos = Some(next);
        let pos = next - vec2(0.0, (1.0 - t) * 14.0);
        let resp = egui::Area::new(Id::unique("tools-bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, LINE))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::same(4))
                    .show(ui, |ui| {
                        if vertical {
                            self.tools_column(ui);
                            return;
                        }
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if self.record {
                                self.record_bar(ui);
                                return;
                            }
                            for (tool, icon, tip) in TOOLS {
                                if ui::icon_button(ui, icon, crate::i18n::t(tip), 36.0, self.tool == tool, ui::Kind::Ghost).clicked() {
                                    self.commit_text();
                                    self.tool = tool;
                                }
                                if tool == Tool::Region {
                                    bar_sep(ui);
                                }
                            }
                            bar_sep(ui);
                            for (c, name) in crate::editor::PALETTE {
                                if color_chip(ui, c, self.color == c).on_hover_text(crate::i18n::t(name)).clicked() {
                                    self.color = c;
                                    let rgba = c.to_srgba_unmultiplied();
                                    self.shared.update_config(|cfg| cfg.editor_color = rgba);
                                }
                            }
                            bar_sep(ui);
                            ui.add_enabled_ui(!self.shapes.is_empty(), |ui| {
                                if ui::icon_button(ui, Icon::Undo, crate::i18n::t("отменить  ctrl z"), 36.0, false, ui::Kind::Ghost).clicked() {
                                    self.shapes.pop();
                                }
                            });
                        });
                    });
            });
        self.top_rect = Some(resp.response.rect);
    }

    /// The tools panel laid out vertically (right of the selection).
    fn tools_column(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
            for (tool, icon, tip) in TOOLS {
                if ui::icon_button(ui, icon, crate::i18n::t(tip), 36.0, self.tool == tool, ui::Kind::Ghost).clicked() {
                    self.commit_text();
                    self.tool = tool;
                }
                if tool == Tool::Region {
                    bar_sep_h(ui);
                }
            }
            bar_sep_h(ui);
            for pair in crate::editor::PALETTE.chunks(2) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for &(c, name) in pair {
                        if color_chip_small(ui, c, self.color == c).on_hover_text(crate::i18n::t(name)).clicked() {
                            self.color = c;
                            let rgba = c.to_srgba_unmultiplied();
                            self.shared.update_config(|cfg| cfg.editor_color = rgba);
                        }
                    }
                });
            }
            bar_sep_h(ui);
            ui.add_enabled_ui(!self.shapes.is_empty(), |ui| {
                if ui::icon_button(ui, Icon::Undo, crate::i18n::t("отменить  ctrl z"), 36.0, false, ui::Kind::Ghost).clicked() {
                    self.shapes.pop();
                }
            });
        });
    }

    /// Top bar in record mode: what goes into the video.
    fn record_bar(&mut self, ui: &mut egui::Ui) {
        let (r, resp) = ui.allocate_exact_size(vec2(26.0, 36.0), egui::Sense::hover());
        resp.on_hover_text(crate::i18n::t("тяни область, кликни окно или нажми пробел для всего экрана"));
        ui.painter().circle_filled(pos2(r.left() + 14.0, r.center().y), 4.5, DANGER);
        if ui::filter(ui, crate::i18n::t("видео"), !self.gif).clicked() {
            self.gif = false;
        }
        if ui::filter(ui, "gif", self.gif).clicked() {
            self.gif = true;
        }
        bar_sep(ui);
        if self.gif {
            // a GIF has no sound; only its size and the effects matter
            let clicks = self.cfg.rec_show_clicks;
            if ui::icon_button(ui, Icon::Select, crate::i18n::t("показывать клики в записи"), 36.0, clicks, ui::Kind::Ghost).clicked() {
                self.cfg.rec_show_clicks = !clicks;
                self.shared.update_config(|c| c.rec_show_clicks = !clicks);
            }
            ui.add_space(4.0);
            return;
        }
        let desk = self.cfg.rec_desktop_audio;
        let tip = if desk { crate::i18n::t("звук системы пишется") } else { crate::i18n::t("звук системы выключен") };
        if ui::icon_button(ui, if desk { Icon::Sound } else { Icon::Mute }, tip, 36.0, desk, ui::Kind::Ghost).clicked() {
            self.cfg.rec_desktop_audio = !desk;
            self.shared.update_config(|c| c.rec_desktop_audio = !desk);
        }
        let mic = self.cfg.rec_mic;
        let tip = if mic { crate::i18n::t("микрофон пишется") } else { crate::i18n::t("микрофон выключен") };
        if ui::icon_button(ui, if mic { Icon::Mic } else { Icon::MicOff }, tip, 36.0, mic, ui::Kind::Ghost).clicked() {
            self.cfg.rec_mic = !mic;
            self.shared.update_config(|c| c.rec_mic = !mic);
        }
        let clicks = self.cfg.rec_show_clicks;
        let tip = if clicks { crate::i18n::t("клики видны в записи") } else { crate::i18n::t("показывать клики в записи") };
        if ui::icon_button(ui, Icon::Select, tip, 36.0, clicks, ui::Kind::Ghost).clicked() {
            self.cfg.rec_show_clicks = !clicks;
            self.shared.update_config(|c| c.rec_show_clicks = !clicks);
        }
        let keys = self.cfg.rec_show_keys;
        let tip = if keys { crate::i18n::t("сочетания клавиш видны в записи") } else { crate::i18n::t("показывать сочетания клавиш (ctrl + c…)") };
        if ui::icon_button(ui, Icon::Keys, tip, 36.0, keys, ui::Kind::Ghost).clicked() {
            self.cfg.rec_show_keys = !keys;
            self.shared.update_config(|c| c.rec_show_keys = !keys);
        }
        bar_sep(ui);
        for fps in [0u32, 30, 60] {
            let on = self.cfg.rec_fps == fps;
            let label = if fps == 0 { crate::i18n::l("как экран", "native") } else if fps == 30 { "30" } else { "60" };
            if ui::filter(ui, label, on).on_hover_text(crate::i18n::l("кадров в секунду", "frames per second")).clicked() {
                self.cfg.rec_fps = fps;
                self.shared.update_config(|c| c.rec_fps = fps);
            }
        }
        ui.add_space(4.0);
    }

    /// Mouse wheel with a drawing tool: thickness / text size.
    fn apply_wheel(&mut self, input: &egui::InputState, now: f64) {
        let wheel: f32 = input
            .raw
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                _ => None,
            })
            .sum();
        if wheel == 0.0 {
            return;
        }
        let cur = WIDTH_STEPS
            .iter()
            .position(|w| *w >= self.cfg.editor_width - 0.01)
            .unwrap_or(WIDTH_STEPS.len() - 1) as i32;
        let next = (cur + if wheel > 0.0 { 1 } else { -1 }).clamp(0, WIDTH_STEPS.len() as i32 - 1) as usize;
        let v = WIDTH_STEPS[next];
        self.cfg.editor_width = v;
        self.shared.update_config(|c| c.editor_width = v);
        self.notice = Some((lf!("размер {}", "size {}", next + 1), now));
    }

    /// Live preview of the text being typed, plus a blinking caret.
    fn paint_typing(&mut self, ctx: &egui::Context, painter: &egui::Painter, ppp: f32) {
        let Some(t) = &self.typing else { return };
        let width = self.shape_width(ppp);
        let size = doc::text_size(width);
        let view = View { offset: Vec2::ZERO, zoom: 1.0 / ppp };
        let pad = if t.plate { doc::text_pad(size) } else { Vec2::ZERO };
        let lh = crate::editor::render::line_height(size);
        if t.buf.trim().is_empty() {
            if t.plate {
                let plate = Rect::from_min_size(t.pos, vec2(size * 0.6, lh) + pad * 2.0);
                painter.rect_filled(view.rect(plate), (size * 0.22) / ppp, self.color);
            }
        } else {
            let kind = if t.plate {
                Kind::Text { pos: t.pos, text: t.buf.clone() }
            } else {
                Kind::Label { pos: t.pos, text: t.buf.clone() }
            };
            let shape = Shape { id: 0, kind, color: self.color, width };
            for prim in shape.prims() {
                self.prims.paint(painter, ctx, &self.snap.image, &prim, view);
            }
        }
        // caret after the last character
        let lines: Vec<&str> = t.buf.split('\n').collect();
        let last = lines.last().copied().unwrap_or("");
        let last_w = if last.is_empty() { 0.0 } else { crate::editor::render::text_box(last, size).x };
        let top = t.pos + pad + vec2(last_w + size * 0.06, (lines.len() - 1) as f32 * lh + lh * 0.12);
        let on = (self.clock * 1.6).fract() < 0.6;
        if on {
            let caret_col = if t.plate { doc::contrast_on(self.color) } else { self.color };
            let r = Rect::from_min_size(top, vec2((size * 0.07).max(2.0), lh * 0.78));
            painter.rect_filled(view.rect(r), 0, caret_col);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(60));
    }
}

impl OverlayApp {
    /// Topmost annotation under `p` (image px). With a drawing tool, areas
    /// (blur, marker, black box) are grabbed by their edge so drawing inside
    /// them still works.
    fn shape_at(&self, p: Pos2, ppp: f32) -> Option<u64> {
        let tol = 4.0 * ppp;
        let edge = 7.0 * ppp;
        self.shapes
            .iter()
            .rev()
            .find(|s| match &s.kind {
                Kind::Pixelate { r } | Kind::Highlight { r } | Kind::Redact { r } if self.tool != Tool::Region => {
                    r.expand(edge).contains(p) && !r.shrink(edge).contains(p)
                }
                _ => s.hit(p, tol, crate::editor::render::text_box),
            })
            .map(|s| s.id)
    }
}

/// Control point of a curved arrow `a → b`: it bulges as far as the mouse
/// strayed from the straight line, or gently upward for a straight drag.
fn curve_control(a: Pos2, b: Pos2, path: &[Pos2]) -> Pos2 {
    let ab = b - a;
    let len = ab.length().max(1.0);
    let n = vec2(-ab.y, ab.x) / len;
    let dev = path
        .iter()
        .map(|q| (*q - a).dot(n))
        .fold(0.0f32, |best, d| if d.abs() > best.abs() { d } else { best });
    let mid = a + ab * 0.5;
    let apex = if dev.abs() < len * 0.08 {
        let up = if n.y <= 0.0 { n } else { -n };
        mid + up * len * 0.22
    } else {
        mid + n * dev
    };
    doc::bezier_through(a, apex, b)
}

fn bar_sep_h(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(36.0, 7.0), egui::Sense::hover());
    ui.painter().hline(rect.shrink2(vec2(8.0, 0.0)).x_range(), rect.center().y, Stroke::new(1.0, LINE));
}

/// Color chip for the vertical panel (two per row).
fn color_chip_small(ui: &mut egui::Ui, c: eframe::egui::Color32, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(18.0, 22.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 3, HOVER);
    }
    let chip = Rect::from_center_size(rect.center(), Vec2::splat(11.0));
    if selected {
        p.rect_stroke(chip.expand(2.5), 2, Stroke::new(1.0, TEXT), StrokeKind::Inside);
    }
    p.rect_filled(chip, 2, c);
    if c.r() < 40 && c.g() < 40 && c.b() < 40 {
        p.rect_stroke(chip, 2, Stroke::new(1.0, LINE), StrokeKind::Inside);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn bar_sep(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 36.0), egui::Sense::hover());
    ui.painter().vline(rect.center().x, rect.shrink2(vec2(0.0, 9.0)).y_range(), Stroke::new(1.0, LINE));
}

fn color_chip(ui: &mut egui::Ui, c: eframe::egui::Color32, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 36.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect.shrink2(vec2(0.0, 6.0)), 3, HOVER);
    }
    let chip = Rect::from_center_size(rect.center(), Vec2::splat(13.0));
    if selected {
        p.rect_stroke(chip.expand(3.0), 2, Stroke::new(1.0, TEXT), StrokeKind::Inside);
    }
    p.rect_filled(chip, 2, c);
    if c.r() < 40 && c.g() < 40 && c.b() < 40 {
        p.rect_stroke(chip, 2, Stroke::new(1.0, LINE), StrokeKind::Inside);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

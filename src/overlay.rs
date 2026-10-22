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
    self, Color32, CursorIcon, Id, Key, Pos2, Rect, Stroke, StrokeKind, TextureHandle,
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
pub fn run_demo(shared: &Arc<Shared>, snap: Snapshot, player: crate::demo::Player, record: bool) {
    run_with(shared, snap, record, false, Some(player));
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
    // macOS: one more window for every other display (Windows: one window spans all)
    #[cfg(target_os = "macos")]
    let (snap, others) = {
        let mut snap = snap;
        let others = std::mem::take(&mut snap.others);
        (snap, others)
    };
    #[cfg(not(target_os = "macos"))]
    let others: Vec<Snapshot> = Vec::new();
    let shared2 = shared.clone();
    runtime::run_window(shared, viewport, false, move |cc| {
        let hwnd = win::hwnd_of(cc);
        if let Some(h) = hwnd {
            win::disable_transitions(h);
            // DWM-cloaked (invisible but rendering) until real frames are on
            // screen: no blank/white window on any monitor.
            win::cloak(h, true);
            win::set_window_rect(h, target, true);
            // macOS: above the menu bar, the Dock and full-screen apps
            #[cfg(target_os = "macos")]
            win::make_overlay(h);
        }
        if demo.is_some() {
            // frames are saved 1:1 with the made-up desktop
            cc.egui_ctx.set_pixels_per_point(1.0);
        }
        let mut list = vec![OverlayApp::new(&cc.egui_ctx, &shared2, snap, target, hwnd, record, gif, demo, out2.clone(), None)];
        for (i, s) in others.into_iter().enumerate() {
            let t = IRect::new(s.origin.0, s.origin.1, s.image.width() as i32, s.image.height() as i32);
            let title = format!("blinker overlay {}", i + 1);
            list.push(OverlayApp::new(&cc.egui_ctx, &shared2, s, t, None, record, gif, None, out2.clone(), Some(title)));
        }
        Overlays { list, engaged: None, active: 0 }
    });
    out.borrow_mut().take()
}

impl OverlayApp {
    /// The overlay of one snapshot. `title`: an extra display's window, made
    /// by egui as a viewport (found by this title, then set up like the first).
    #[allow(clippy::too_many_arguments)]
    fn new(
        ctx: &egui::Context,
        shared: &Arc<Shared>,
        snap: Snapshot,
        target: IRect,
        hwnd: Option<win::HWND>,
        record: bool,
        gif: bool,
        demo: Option<crate::demo::Player>,
        out: Rc<RefCell<Option<Output>>>,
        title: Option<String>,
    ) -> Self {
        let name = format!("snapshot-{}", title.as_deref().unwrap_or("main"));
        let tex = TiledTexture::new(ctx, &name, &snap.image, TextureOptions::NEAREST);
        let c = shared.config().editor_color;
        let primary = title.is_none();
        let cfg = shared.config();
        let tour = (!cfg.tour_done && !record && !cfg.instant_mode)
            .then_some(Tour { step: TourStep::Select, at: f64::NAN, rect: None, shapes: 0 });
        OverlayApp {
            cfg: shared.config(),
            shared: shared.clone(),
            snap,
            tex,
            loupe: None,
            revealed: (hwnd.is_none() && primary) || demo.is_some(),
            stable: 0,
            shown_frame: 0,
            hwnd,
            target,
            frame_no: 0,
            sel: None,
            drag: Drag::None,
            phase: Phase::Selecting,
            out,
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
            color: eframe::egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]),
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
            primary,
            active: primary,
            title,
            yield_focus: false,
            tour,
            palette_open: false,
            glass: None,
        }
    }

    /// Something started here: a selection, a drag, a drawing or text.
    fn engaged(&self) -> bool {
        self.sel.is_some() || !matches!(self.drag, Drag::None) || !self.shapes.is_empty() || self.typing.is_some()
    }

    /// Back to a plain dimmed display (the work moved to another one).
    fn reset(&mut self) {
        self.sel = None;
        self.drag = Drag::None;
        self.shapes.clear();
        self.typing = None;
        self.clicked_window = None;
        self.hover_shape = None;
        self.picked = None;
        self.toolbar_rect = None;
        if self.phase == Phase::Adjusting {
            self.phase = Phase::Selecting;
        }
    }
}

/// The capture overlay: one window per display on macOS (a single window
/// over all monitors on Windows). The display where the work happens is
/// "active": it has the tool panels; starting a selection on another one
/// moves the work there.
struct Overlays {
    /// The first is the main window (the display under the pointer).
    list: Vec<OverlayApp>,
    engaged: Option<usize>,
    active: usize,
}

impl Overlays {
    fn coordinate(&mut self) {
        let (cx, cy) = win::cursor_pos();
        let under = self.list.iter().position(|o| o.target.contains(cx, cy));
        let engaged: Vec<usize> = (0..self.list.len()).filter(|&i| self.list[i].engaged()).collect();
        // the newest engagement wins: the older one goes back to plain
        let winner = if engaged.len() > 1 { engaged.iter().copied().find(|&i| Some(i) != self.engaged).or(self.engaged) } else { engaged.first().copied() };
        for &i in &engaged {
            if Some(i) != winner {
                self.list[i].reset();
            }
        }
        if winner.is_some() && winner != self.engaged
            && let Some(h) = winner.and_then(|i| self.list[i].hwnd)
        {
            // keys (Enter, Ctrl+C...) go to where the selection is
            win::force_foreground(h);
        }
        self.engaged = winner;
        self.list[0].yield_focus = winner.is_some_and(|w| w != 0);
        let active = winner.or(under).unwrap_or(0);
        if active != self.active {
            let (tool, color) = (self.list[self.active].tool, self.list[self.active].color);
            let o = &mut self.list[active];
            o.tool = tool;
            o.color = color;
            self.active = active;
        }
        for (i, o) in self.list.iter_mut().enumerate() {
            o.active = i == active;
            // no loupe or window highlight left behind where the pointer isn't
            if Some(i) != under && matches!(o.drag, Drag::None) {
                o.last_pointer = None;
            }
        }
    }
}

impl eframe::App for Overlays {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.list[0].raw_input_hook(ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.list.len() > 1 {
            self.coordinate();
        }
        let (main, rest) = self.list.split_first_mut().expect("one overlay");
        main.frame(&ctx);
        for (i, o) in rest.iter_mut().enumerate() {
            #[cfg(target_os = "macos")]
            let (x, y, w, h, _) = win::rect_to_pt(o.target);
            #[cfg(not(target_os = "macos"))]
            let (x, y, w, h) = (o.target.x as f64, o.target.y as f64, o.target.w as f64, o.target.h as f64);
            let builder = egui::ViewportBuilder::default()
                .with_title(o.title.clone().unwrap_or_default())
                .with_decorations(false)
                .with_resizable(false)
                .with_taskbar(false)
                .with_always_on_top()
                .with_visible(o.hwnd.is_some())
                .with_position([x as f32, y as f32])
                .with_inner_size([w as f32, h as f32]);
            ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(("blinker-overlay", i)), builder, |vctx, _| o.frame(vctx));
        }
    }
}

/// Tools of the top bar: region selection plus annotations drawn before capturing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Region,
    Arrow,
    Rect,
    Pen,
    Highlight,
    Steps,
    Blur,
    Redact,
    TextBox,
    Text,
}

/// Tools in toolbar order, with their keys (the rectangle came later: 0).
const TOOLS: [(Tool, Icon, &str, Key); 10] = [
    (Tool::Region, Icon::Region, "выделение области  1", Key::Num1),
    (Tool::Arrow, Icon::Arrow, "стрелка  2 · с shift — дугой", Key::Num2),
    (Tool::Rect, Icon::Rect, "прямоугольник  0", Key::Num0),
    (Tool::Pen, Icon::Pen, "карандаш  3", Key::Num3),
    (Tool::Highlight, Icon::Marker, "маркер  4", Key::Num4),
    (Tool::Steps, Icon::Counter, "шаги  5", Key::Num5),
    (Tool::Blur, Icon::Blur, "размытие  6", Key::Num6),
    (Tool::Redact, Icon::Redact, "закрасить чёрным  7", Key::Num7),
    (Tool::TextBox, Icon::TextBox, "текст с фоном  8", Key::Num8),
    (Tool::Text, Icon::Text, "текст  9", Key::Num9),
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

/// The tools panel: big buttons, the colors folded into one swatch.
/// Button and icon size of the panels: smaller next to a small selection,
/// so the panels don't dwarf it or run into each other.
const BAR_SIZES: [(f32, f32); 3] = [(42.0, 19.0), (36.0, 17.0), (32.0, 15.0)];

thread_local! {
    static BAR: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn bar_btn() -> f32 {
    BAR_SIZES[BAR.with(|b| b.get())].0
}

fn bar_icon() -> f32 {
    BAR_SIZES[BAR.with(|b| b.get())].1
}

/// Panel size step for a selection of `w` × `h` points.
fn bar_step(w: f32, h: f32) -> usize {
    if h < 200.0 || w < 320.0 {
        2
    } else if h < 340.0 || w < 560.0 {
        1
    } else {
        0
    }
}

struct OverlayApp {
    /// macOS 26: a blurred copy of the screenshot the panels are made of,
    /// so they look like Liquid Glass over what's under them.
    glass: Option<TextureHandle>,
    /// The colors are shown in the panel (after a click on the swatch).
    palette_open: bool,
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
    /// The main window (the others don't take the keyboard focus or commands).
    primary: bool,
    /// The display where the work is: it shows the tool panels.
    active: bool,
    /// Extra display windows: the title their egui viewport was made with.
    title: Option<String>,
    /// The work is on another display: the main window leaves the focus there.
    yield_focus: bool,
    /// The first capture's walkthrough: its step, when the step began, and
    /// where its bubble was last frame (points).
    tour: Option<Tour>,
}

/// Steps of the first capture's walkthrough.
#[derive(Clone, Copy, PartialEq)]
enum TourStep {
    /// Nothing selected yet: how to pick an area.
    Select,
    /// The selection is there: the drawing tools.
    Draw,
    /// What to do with the shot.
    Act,
}

struct Tour {
    step: TourStep,
    at: f64,
    rect: Option<Rect>,
    /// Shapes drawn when the step began (one more moves the tour on).
    shapes: usize,
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
        if self.tour.is_some() {
            self.end_tour();
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
            self.toolbar_rect.is_some_and(|r| r.contains(p))
                || self.top_rect.is_some_and(|r| r.contains(p))
                || self.tour.as_ref().and_then(|t| t.rect).is_some_and(|r| r.contains(p))
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
            #[cfg(target_os = "macos")]
            use crate::win::vk::{VK_ESCAPE, VK_RETURN};
            #[cfg(windows)]
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
        for (tool, _, _, k) in TOOLS {
            if !cmd && !self.record && input.key_pressed(k) {
                self.tool = tool;
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
        // Ctrl / Cmd + C: the text field layer turns it into a Copy event
        // before any key press shows, so both are looked for (and plain Ctrl
        // on a Mac too)
        let copy_keys = input.events.iter().any(|e| matches!(e, egui::Event::Copy))
            || ((cmd || input.modifiers.ctrl) && input.key_pressed(Key::C));
        if self.sel.is_some() && self.phase == Phase::Adjusting {
            if input.key_pressed(Key::Enter) || enter || copy_keys {
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
            if input.key_pressed(Key::U) && crate::upload::configured(&self.shared.config()) {
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
            && !copy_keys
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
        self.hover_shape = if matches!(self.drag, Drag::None) && !over_toolbar && !input.modifiers.command {
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
        if self.tool != Tool::Region && !input.modifiers.command {
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
            let ctrl = input.modifiers.command;
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
                    if input.modifiers.command {
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
                Drag::Creating { .. } if input.modifiers.command => CursorIcon::Move,
                Drag::None if self.phase == Phase::Adjusting && input.modifiers.command && self.sel.is_some() => {
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

    /// Points at the top of snapshot monitor `m` that a notch covers (MacBooks;
    /// 0 elsewhere): bars and labels stay below it.
    fn notch(&self, m: IRect, ppp: f32) -> f32 {
        win::safe_top_at(self.snap.origin.0 + m.x + m.w / 2, self.snap.origin.1 + m.y) as f32 / ppp
    }

    /// Dark rounded pill with the size above the frame's top-left corner.
    fn paint_size_label(&self, ctx: &egui::Context, r: Rect, s: Rect, alpha: f32) {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("label")));
        let text = format!("{} × {}", s.width() as i32, s.height() as i32);
        let galley = painter.layout_no_wrap(text, font_medium(12.5), Color32::PLACEHOLDER);
        let size = galley.size() + vec2(18.0, 10.0);
        let ppp = ctx.pixels_per_point();
        let mon = self.snap.monitor_at((r.center().x * ppp) as i32, (r.center().y * ppp) as i32).rect;
        let top = mon.y as f32 / ppp + self.notch(mon, ppp);
        let mut pos = pos2(r.left(), r.top() - 10.0 - size.y);
        if pos.y < top + 4.0 {
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
        // the caption: color, zoom and position on one line when they fit
        // (Retina coordinates are long), the position on a second otherwise
        let col = if cx >= 0 && cy >= 0 && (cx as u32) < img.width() && (cy as u32) < img.height() {
            *img.get_pixel(cx as u32, cy as u32)
        } else {
            image::Rgba([0, 0, 0, 255])
        };
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::unique("loupe")));
        let hex = painter.layout_no_wrap(format!("#{:02x}{:02x}{:02x}", col[0], col[1], col[2]), mono(11.0), TEXT);
        let zoom = painter.layout_no_wrap(format!("×{:.0}", cell * ppp), mono(11.0), FAINT);
        let coords = painter.layout_no_wrap(format!("{cx}, {cy}"), mono(11.0), FAINT);
        let one_line = 15.0 + hex.size().x + 10.0 + coords.size().x + 10.0 + zoom.size().x <= side;
        let caption_h = if one_line { 20.0 } else { 36.0 };
        let outer = vec2(side + pad * 2.0, side + pad * 2.0 + caption_h);
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

        let row_y = img_rect.bottom() + 11.0;
        let sw = Rect::from_center_size(pos2(img_rect.left() + 6.0, row_y), Vec2::splat(9.0));
        painter.rect_filled(sw, 0, Color32::from_rgb(col[0], col[1], col[2]));
        painter.rect_stroke(sw, 0, Stroke::new(1.0, LINE), StrokeKind::Outside);
        let h = hex.size().y;
        painter.galley(pos2(sw.right() + 6.0, row_y - h / 2.0), hex, TEXT);
        let zx = img_rect.right() - zoom.size().x;
        painter.galley(pos2(zx, row_y - h / 2.0), zoom, FAINT);
        if one_line {
            painter.galley(pos2(zx - 10.0 - coords.size().x, row_y - h / 2.0), coords, FAINT);
        } else {
            painter.galley(pos2(img_rect.left(), row_y + 16.0 - h / 2.0), coords, FAINT);
        }
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
        let m = self.snap.monitor_at(c.x as i32, c.y as i32).rect;
        let mon = Rect::from_min_max(
            pos2(m.x as f32 / ppp, m.y as f32 / ppp + self.notch(m, ppp)),
            pos2(m.right() as f32 / ppp, m.bottom() as f32 / ppp),
        );
        let size = self.toolbar_rect.map(|t| t.size()).unwrap_or(vec2(52.0, 250.0));
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
        let at = pos2(x - (1.0 - t) * 8.0, y);
        self.paint_glass(ctx, Rect::from_min_size(at, size), t, ppp, "toolbar-glass");
        let resp = egui::Area::new(Id::unique("toolbar"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                // on glass the panel is clear: the backdrop below shows through
                let glass = self.glass.is_some();
                egui::Frame::new()
                    .fill(if glass { Color32::TRANSPARENT } else { PANEL })
                    .stroke(Stroke::new(1.0, if glass { Color32::TRANSPARENT } else { LINE }))
                    .corner_radius(9)
                    .inner_margin(egui::Margin::same(5))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.vertical(|ui| {
                            let mut act = None;
                            if self.record {
                                let tip = if self.gif { crate::i18n::t("начать запись gif  enter") } else { crate::i18n::t("начать запись  enter") };
                                if ui::icon_button_sized(ui, Icon::Record, tip, bar_btn(), bar_icon(), false, ui::Kind::Accent).clicked() {
                                    act = Some(Action::Record);
                                }
                                if ui::icon_button_sized(ui, Icon::Close, crate::i18n::t("отмена  esc"), bar_btn(), bar_icon(), false, ui::Kind::Ghost).clicked() {
                                    self.cancel(ctx);
                                }
                                return act;
                            }
                            if ui::icon_button_sized(ui, Icon::Copy, crate::i18n::t("копировать  enter"), bar_btn(), bar_icon(), false, ui::Kind::Accent).clicked() {
                                act = Some(Action::Copy);
                            }
                            if ui::icon_button_sized(ui, Icon::Save, &crate::i18n::kb(crate::i18n::t("сохранить  ctrl s")), bar_btn(), bar_icon(), false, ui::Kind::Ghost).clicked() {
                                act = Some(Action::Save);
                            }
                            if ui::icon_button_sized(ui, Icon::Edit, crate::i18n::t("править  e"), bar_btn(), bar_icon(), false, ui::Kind::Ghost).clicked() {
                                act = Some(Action::Edit);
                            }
                            if ui::icon_button_sized(ui, Icon::Pin, crate::i18n::t("закрепить поверх окон  p"), bar_btn(), bar_icon(), false, ui::Kind::Ghost).clicked() {
                                act = Some(Action::Pin);
                            }
                            if ui::icon_button_sized(ui, Icon::Frame, crate::i18n::t("красивый снимок  b"), bar_btn(), bar_icon(), false, ui::Kind::Ghost)
                                .clicked()
                            {
                                act = Some(Action::Beauty);
                            }
                            if crate::upload::configured(&self.cfg)
                                && ui::icon_button_sized(ui, Icon::Upload, crate::i18n::t("загрузить и скопировать ссылку  u"), bar_btn(), bar_icon(), false, ui::Kind::Ghost)
                                    .clicked()
                            {
                                act = Some(Action::Upload);
                            }
                            let (sep, _) = ui.allocate_exact_size(vec2(bar_btn(), 5.0), egui::Sense::hover());
                            ui.painter().hline(sep.shrink2(vec2(4.0, 0.0)).x_range(), sep.center().y, Stroke::new(1.0, LINE));
                            if ui::icon_button_sized(ui, Icon::Close, crate::i18n::t("отмена  esc"), bar_btn(), bar_icon(), false, ui::Kind::Ghost).clicked() {
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

impl OverlayApp {
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

    /// One frame of this display's window.
    fn frame(&mut self, ctx: &egui::Context) {
        let ctx = ctx.clone();
        // an extra display's window: set up once egui has made it
        #[cfg(target_os = "macos")]
        if self.hwnd.is_none()
            && let Some(h) = self.title.as_deref().and_then(win::find_window)
        {
            win::disable_transitions(h);
            win::cloak(h, true);
            win::set_window_rect(h, self.target, true);
            win::make_overlay(h);
            self.hwnd = Some(h);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
        self.frame_no += 1;
        let now = ctx.input(|i| i.time);
        if self.t0 < 0.0 {
            self.t0 = now;
            self.last_t = now;
            let (cx, cy) = win::cursor_pos();
            let m = self.snap.monitor_at(cx - self.snap.origin.0, cy - self.snap.origin.1).rect;
            let ppp0 = ctx.pixels_per_point();
            let notch = self.notch(m, ppp0);
            self.bar_mon = Some(Rect::from_min_max(
                pos2(m.x as f32 / ppp0, m.y as f32 / ppp0 + notch),
                pos2(m.right() as f32 / ppp0, m.bottom() as f32 / ppp0),
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
                    if self.primary {
                        win::force_foreground(h);
                    }
                }
                ctx.request_repaint();
            } else if self.primary && !self.yield_focus && matches!(self.frame_no - self.shown_frame, 4 | 12 | 28) && !win::is_foreground(h) {
                win::force_foreground(h);
            }
        }
        // Hotkeys pressed while selecting are ignored; quitting closes us.
        if self.primary {
            self.shared.take_where(|c| c.is_capture() || *c == Command::Record);
            if matches!(self.shared.peek(), Some(Command::Quit | Command::Restart)) {
                *self.out.borrow_mut() = None;
                close_all(&ctx);
                return;
            }
        }

        let ppp = ctx.pixels_per_point();
        self.handle_input(&ctx, ppp, now);
        self.paint(&ctx, ppp, now);
        if matches!(self.drag, Drag::None) {
            let step = self.sel.filter(|_| self.phase == Phase::Adjusting).map_or(0, |s| bar_step(s.width() / ppp, s.height() / ppp));
            if BAR.with(|b| b.replace(step)) != step {
                // the panels change size: measure them afresh
                self.top_rect = None;
                self.toolbar_rect = None;
                ctx.request_repaint();
            }
        }
        if self.active {
            self.tools_bar(&ctx);
        }
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
        if self.active && self.tour.is_some() {
            self.tour(&ctx, now);
        }
        if let Phase::Closing { at } = self.phase
            && now - at >= CLOSE
        {
            close_all(&ctx);
        }
        if let Phase::Finishing { at } = self.phase
            && now - at >= FLASH
        {
            close_all(&ctx);
        }
        if let Some(d) = &self.demo {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(d.shot_request()));
            ctx.request_repaint();
        }
    }
}

// ---------------------------------------------------------------------------
// The first capture's walkthrough: a bubble per step, next to what it explains

impl OverlayApp {
    fn end_tour(&mut self) {
        self.tour = None;
        if self.demo.is_none() {
            self.shared.update_config(|c| c.tour_done = true);
        }
    }

    fn tour(&mut self, ctx: &egui::Context, now: f64) {
        let Some(mut tour) = self.tour.take() else { return };
        if tour.at.is_nan() {
            tour.at = now;
        }
        // the step follows what the user does
        let want = match (self.phase, tour.step) {
            (Phase::Selecting, _) => TourStep::Select,
            (Phase::Adjusting, TourStep::Select) => TourStep::Draw,
            (_, s) => s,
        };
        let drew = tour.step == TourStep::Draw && self.shapes.len() > tour.shapes && matches!(self.drag, Drag::None);
        let want = if drew { TourStep::Act } else { want };
        if want != tour.step {
            tour = Tour { step: want, at: now, rect: None, shapes: self.shapes.len() };
        }
        if matches!(self.phase, Phase::Finishing { .. } | Phase::Closing { .. }) || !matches!(self.drag, Drag::None) && tour.step == TourStep::Select {
            // out of the way while the area is being dragged out
            tour.rect = None;
            self.tour = Some(tour);
            return;
        }
        let ctrl = |s: &str| crate::i18n::kb(s).into_owned();
        let l = crate::i18n::l;
        let (title, lines, anchor): (&str, Vec<(String, &str)>, Option<Rect>) = match tour.step {
            TourStep::Select => (
                l("выдели, что снять", "select what to capture"),
                vec![
                    (l("тяни", "drag").into(), l("область", "an area")),
                    (l("клик", "click").into(), l("окно под курсором", "the window under the cursor")),
                    (l("пробел", "space").into(), l("весь экран", "the whole screen")),
                    ("esc".into(), l("отмена", "cancel")),
                ],
                None,
            ),
            TourStep::Draw => (
                l("рисуй прямо на снимке", "draw right on the shot"),
                vec![
                    ("1 – 9".into(), l("стрелки, шаги, размытие…", "arrows, steps, blur…")),
                    (ctrl("ctrl z"), l("отменить", "undo")),
                    (l("края", "edges").into(), l("тяни, чтобы поправить рамку", "drag to adjust the frame")),
                ],
                self.top_rect,
            ),
            TourStep::Act => (
                l("что сделать со снимком", "what to do with the shot"),
                vec![
                    (ctrl("ctrl c"), l("копировать", "copy")),
                    (ctrl("ctrl s"), l("сохранить", "save")),
                    ("e".into(), l("открыть в редакторе", "open in the editor")),
                    ("p".into(), l("закрепить поверх окон", "pin on top")),
                ],
                self.toolbar_rect,
            ),
        };
        let index = tour.step as usize;
        let last = tour.step == TourStep::Act;
        let size = tour.rect.map(|r| r.size()).unwrap_or(vec2(290.0, 170.0));
        // where the bubble goes: the screen's top middle, or beside the panel it explains
        let Some(mon) = self.bar_mon else {
            self.tour = Some(tour);
            return;
        };
        let room = mon.shrink(12.0);
        let mut side = None;
        let pos = match anchor {
            None => pos2(mon.center().x - size.x / 2.0, self.top_rect.map_or(mon.top() + 80.0, |r| r.bottom() + 14.0)),
            Some(a) if tour.step == TourStep::Draw => {
                // under the panel; else beside it; over it (on the picture) last
                let mid_y = (a.center().y - size.y / 2.0).clamp(room.top(), (room.bottom() - size.y).max(room.top()));
                if a.bottom() + 14.0 + size.y <= room.bottom() {
                    side = Some((a.center_bottom(), vec2(0.0, -1.0)));
                    pos2(a.center().x - size.x / 2.0, a.bottom() + 14.0)
                } else if a.right() + 14.0 + size.x <= room.right() {
                    side = Some((a.right_center(), vec2(-1.0, 0.0)));
                    pos2(a.right() + 14.0, mid_y)
                } else if a.left() - 14.0 - size.x >= room.left() {
                    side = Some((a.left_center(), vec2(1.0, 0.0)));
                    pos2(a.left() - 14.0 - size.x, mid_y)
                } else {
                    side = Some((a.center_top(), vec2(0.0, 1.0)));
                    pos2(a.center().x - size.x / 2.0, a.top() - 14.0 - size.y)
                }
            }
            Some(a) => {
                // beside the actions bar, away from the selection when there's room
                if a.right() + 14.0 + size.x <= room.right() {
                    side = Some((a.right_center(), vec2(-1.0, 0.0)));
                    pos2(a.right() + 14.0, a.center().y - size.y / 2.0)
                } else {
                    side = Some((a.left_center(), vec2(1.0, 0.0)));
                    pos2(a.left() - 14.0 - size.x, a.center().y - size.y / 2.0)
                }
            }
        };
        let pos = pos2(pos.x.clamp(room.left(), (room.right() - size.x).max(room.left())), pos.y.clamp(room.top(), (room.bottom() - size.y).max(room.top())));
        let t = ui::ease_out_cubic(((now - tour.at - 0.15) / 0.3) as f32);
        if t < 1.0 {
            ctx.request_repaint();
        }
        let slide = side.map_or(vec2(0.0, -8.0), |(_, d)| d * 8.0) * (1.0 - t);
        let mut next = false;
        let mut skip = false;
        let resp = egui::Area::new(Id::unique("tour"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos + slide)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, ACCENT.gamma_multiply(0.55)))
                    .corner_radius(12)
                    .shadow(egui::epaint::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(110) })
                    .inner_margin(egui::Margin::symmetric(16, 14))
                    .show(ui, |ui| {
                        ui.set_width(268.0);
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.label(egui::RichText::new(title).font(font_semibold(15.0)).color(TEXT));
                        ui.add_space(2.0);
                        for (key, what) in &lines {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                keycap(ui, key);
                                ui.label(egui::RichText::new(*what).font(font(12.5)).color(DIM));
                            });
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            // where we are: three dots
                            let (r, _) = ui.allocate_exact_size(vec2(34.0, 20.0), egui::Sense::hover());
                            for i in 0..3 {
                                let c = pos2(r.left() + 4.0 + i as f32 * 12.0, r.center().y);
                                let on = i == index;
                                ui.painter().circle_filled(c, if on { 3.5 } else { 2.5 }, if on { ACCENT } else { LINE });
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if tour.step != TourStep::Select {
                                    let label = if last { l("понятно", "got it") } else { l("дальше", "next") };
                                    next = ui::button_h(ui, label, ui::Kind::Accent, 28.0).clicked();
                                }
                                if !last {
                                    skip = ui::button_h(ui, l("пропустить", "skip"), ui::Kind::Ghost, 28.0).clicked();
                                }
                            });
                        });
                    });
            });
        let rect = resp.response.rect;
        // a small arrow from the bubble towards what it explains
        if let Some((target, dir)) = side {
            let edge = rect.center() - dir * vec2(rect.width() / 2.0, rect.height() / 2.0);
            let tip_at = pos2(
                if dir.x == 0.0 { target.x.clamp(rect.left() + 18.0, rect.right() - 18.0) } else { edge.x },
                if dir.y == 0.0 { target.y.clamp(rect.top() + 18.0, rect.bottom() - 18.0) } else { edge.y },
            );
            let n = -dir;
            let perp = vec2(-n.y, n.x);
            let tri = vec![tip_at + n * 7.0, tip_at + perp * 7.0 - n * 0.5, tip_at - perp * 7.0 - n * 0.5];
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("tour-arrow")));
            painter.add(egui::Shape::convex_polygon(tri.clone(), PANEL.gamma_multiply(t), Stroke::NONE));
            painter.line(vec![tri[1], tri[0], tri[2]], Stroke::new(1.0, ACCENT.gamma_multiply(0.55 * t)));
        }
        tour.rect = Some(rect);
        if skip || (next && last) {
            self.end_tour();
            return;
        }
        if next {
            tour = Tour { step: TourStep::Act, at: now, rect: None, shapes: self.shapes.len() };
        }
        self.tour = Some(tour);
    }
}

/// A key name on a little plate, as on a keyboard.
fn keycap(ui: &mut egui::Ui, key: &str) {
    let g = ui.painter().layout_no_wrap(key.to_string(), mono(11.5), TEXT);
    let w = (g.size().x + 12.0).max(52.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 20.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 5, HOVER);
    ui.painter().rect_stroke(r, 5, Stroke::new(1.0, LINE), StrokeKind::Inside);
    ui.painter().galley(r.center() - g.size() / 2.0, g, TEXT);
}

/// Closes the overlay: the main window, and with it every display's window.
fn close_all(ctx: &egui::Context) {
    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
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
                Tool::Rect | Tool::Highlight | Tool::Blur | Tool::Redact => {
                    let r = Rect::from_two_pos(p, p);
                    let kind = match self.tool {
                        Tool::Rect => Kind::Rect { r },
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
                        Kind::Rect { r } | Kind::Highlight { r } | Kind::Pixelate { r } | Kind::Redact { r } => {
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
                Kind::Rect { r } | Kind::Highlight { r } | Kind::Pixelate { r } | Kind::Redact { r } => r.width() > min && r.height() > min,
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
        let size = self.top_rect.map(|r| r.size()).unwrap_or(vec2(540.0, 52.0));
        // Next to the selection once there is one (the screen top until then).
        // In instant mode the selection is copied the moment it's made: the
        // panel stays at the top instead of setting off after it.
        let mode = if self.record || self.cfg.instant_mode { crate::config::ToolsPos::Screen } else { self.cfg.tools_pos };
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
                    pos2(c.x as f32 / ppp, c.y as f32 / ppp + self.notch(c, ppp)),
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
                        let at = pos2(x, y.clamp(m.top(), (m.bottom() - size.y).max(m.top())));
                        self.clear_of_actions(at, size, m)
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
        let next = if (next - target).length() < 0.5 || self.cfg.instant_mode { target } else { next };
        if next != target {
            ctx.request_repaint();
        }
        self.bar_pos = Some(next);
        let pos = next - vec2(0.0, (1.0 - t) * 14.0);
        self.paint_glass(ctx, Rect::from_min_size(pos, size), t, ppp, "tools-glass");
        let resp = egui::Area::new(Id::unique("tools-bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                // on glass the panel is clear: the backdrop below shows through
                let glass = self.glass.is_some();
                egui::Frame::new()
                    .fill(if glass { Color32::TRANSPARENT } else { PANEL })
                    .stroke(Stroke::new(1.0, if glass { Color32::TRANSPARENT } else { LINE }))
                    .corner_radius(9)
                    .inner_margin(egui::Margin::same(5))
                    .show(ui, |ui| {
                        if vertical {
                            self.tools_column(ui);
                            return;
                        }
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 1.0;
                            if self.record {
                                self.record_bar(ui);
                                return;
                            }
                            for (tool, icon, tip, _) in TOOLS {
                                if ui::icon_button_sized(ui, icon, crate::i18n::t(tip), bar_btn(), bar_icon(), self.tool == tool, ui::Kind::Ghost).clicked() {
                                    self.commit_text();
                                    self.tool = tool;
                                }
                                if tool == Tool::Region {
                                    bar_sep(ui);
                                }
                            }
                            bar_sep(ui);
                            self.palette(ui);
                            bar_sep(ui);
                            ui.add_enabled_ui(!self.shapes.is_empty(), |ui| {
                                if ui::icon_button_sized(ui, Icon::Undo, &crate::i18n::kb(crate::i18n::t("отменить  ctrl z")), bar_btn(), bar_icon(), false, ui::Kind::Ghost).clicked() {
                                    self.shapes.pop();
                                }
                            });
                        });
                    });
            });
        self.top_rect = Some(resp.response.rect);
    }

    /// A pane of glass at `rect` (points): the blurred screenshot under it, a
    /// dark tint, a light rim and a highlight along the top edge.
    fn paint_glass(&mut self, ctx: &egui::Context, rect: Rect, alpha: f32, ppp: f32, id: &str) {
        if !glass() || alpha <= 0.0 {
            return;
        }
        if self.glass.is_none() {
            // an eighth of the size, blurred: cheap, and it's all a pane shows
            let img = &self.snap.image;
            let (w, h) = ((img.width() / 8).max(1), (img.height() / 8).max(1));
            let small = image::imageops::resize(img, w, h, image::imageops::FilterType::Triangle);
            let blurred = image::imageops::blur(&small, 2.2);
            self.glass = Some(ctx.load_texture("glass", color_image(&blurred), TextureOptions::LINEAR));
        }
        let Some(tex) = &self.glass else { return };
        let img = vec2(self.snap.image.width() as f32, self.snap.image.height() as f32);
        let uv = Rect::from_min_max(
            pos2(rect.min.x * ppp / img.x, rect.min.y * ppp / img.y),
            pos2(rect.max.x * ppp / img.x, rect.max.y * ppp / img.y),
        );
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, Id::unique(id)));
        let r = egui::CornerRadius::same(9);
        p.add(egui::epaint::RectShape::filled(rect, r, Color32::WHITE.gamma_multiply(alpha)).with_texture(tex.id(), uv));
        p.rect_filled(rect, r, Color32::from_rgba_unmultiplied(20, 20, 19, (125.0 * alpha) as u8));
        p.rect_stroke(rect, r, Stroke::new(1.0, Color32::from_white_alpha((42.0 * alpha) as u8)), StrokeKind::Inside);
        // light catching the top edge
        let top = Rect::from_min_size(rect.min + vec2(9.0, 1.0), vec2(rect.width() - 18.0, 1.0));
        p.rect_filled(top, 0, Color32::from_white_alpha((36.0 * alpha) as u8));
    }

    /// The current color as one swatch; a click unfolds the others next to it.
    fn palette(&mut self, ui: &mut egui::Ui) {
        if !self.palette_open {
            let name = crate::editor::PALETTE.iter().find(|(c, _)| *c == self.color).map(|(_, n)| crate::i18n::t(n)).unwrap_or("");
            let tip = lf!("цвет: {name}", "color: {name}");
            if swatch(ui, self.color, bar_btn()).on_hover_text(tip).clicked() {
                self.palette_open = true;
            }
            return;
        }
        for (c, name) in crate::editor::PALETTE {
            if color_chip(ui, c, self.color == c).on_hover_text(crate::i18n::t(name)).clicked() {
                self.color = c;
                self.palette_open = false;
                let rgba = c.to_srgba_unmultiplied();
                self.shared.update_config(|cfg| cfg.editor_color = rgba);
            }
        }
    }

    /// Moves the tools panel at `at` so it doesn't run into the actions bar:
    /// sideways first (it stays by the selection), past the bar's end last.
    fn clear_of_actions(&self, at: Pos2, size: Vec2, m: Rect) -> Pos2 {
        let Some(a) = self.toolbar_rect else { return at };
        let gap = 8.0;
        if !Rect::from_min_size(at, size).expand(gap / 2.0).intersects(a) {
            return at;
        }
        let left = a.left() - gap - size.x;
        if left >= m.left() {
            return pos2(left, at.y);
        }
        if a.right() + gap + size.x <= m.right() {
            return pos2(a.right() + gap, at.y);
        }
        let below = a.bottom() + gap;
        if below + size.y <= m.bottom() {
            pos2(at.x, below)
        } else {
            pos2(at.x, (a.top() - gap - size.y).max(m.top()))
        }
    }

    /// The tools panel laid out vertically (right of the selection).
    fn tools_column(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
            for (tool, icon, tip, _) in TOOLS {
                if ui::icon_button_sized(ui, icon, crate::i18n::t(tip), bar_btn(), bar_icon(), self.tool == tool, ui::Kind::Ghost).clicked() {
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
                if ui::icon_button_sized(ui, Icon::Undo, &crate::i18n::kb(crate::i18n::t("отменить  ctrl z")), bar_btn(), bar_icon(), false, ui::Kind::Ghost).clicked() {
                    self.shapes.pop();
                }
            });
        });
    }

    /// Top bar in record mode: what goes into the video.
    fn record_bar(&mut self, ui: &mut egui::Ui) {
        let (r, resp) = ui.allocate_exact_size(vec2(28.0, bar_btn()), egui::Sense::hover());
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
            if ui::icon_button_sized(ui, Icon::Select, crate::i18n::t("показывать клики в записи"), bar_btn(), bar_icon(), clicks, ui::Kind::Ghost).clicked() {
                self.cfg.rec_show_clicks = !clicks;
                self.shared.update_config(|c| c.rec_show_clicks = !clicks);
            }
            ui.add_space(4.0);
            return;
        }
        let desk = self.cfg.rec_desktop_audio;
        let tip = if desk { crate::i18n::t("звук системы пишется") } else { crate::i18n::t("звук системы выключен") };
        if ui::icon_button_sized(ui, if desk { Icon::Sound } else { Icon::Mute }, tip, bar_btn(), bar_icon(), desk, ui::Kind::Ghost).clicked() {
            self.cfg.rec_desktop_audio = !desk;
            self.shared.update_config(|c| c.rec_desktop_audio = !desk);
        }
        let mic = self.cfg.rec_mic;
        let tip = if mic { crate::i18n::t("микрофон пишется") } else { crate::i18n::t("микрофон выключен") };
        if ui::icon_button_sized(ui, if mic { Icon::Mic } else { Icon::MicOff }, tip, bar_btn(), bar_icon(), mic, ui::Kind::Ghost).clicked() {
            self.cfg.rec_mic = !mic;
            self.shared.update_config(|c| c.rec_mic = !mic);
        }
        let clicks = self.cfg.rec_show_clicks;
        let tip = if clicks { crate::i18n::t("клики видны в записи") } else { crate::i18n::t("показывать клики в записи") };
        if ui::icon_button_sized(ui, Icon::Select, tip, bar_btn(), bar_icon(), clicks, ui::Kind::Ghost).clicked() {
            self.cfg.rec_show_clicks = !clicks;
            self.shared.update_config(|c| c.rec_show_clicks = !clicks);
        }
        let keys = self.cfg.rec_show_keys;
        let tip = if keys { crate::i18n::t("сочетания клавиш видны в записи") } else { &crate::i18n::kb(crate::i18n::t("показывать сочетания клавиш (ctrl + c…)")) };
        if ui::icon_button_sized(ui, Icon::Keys, tip, bar_btn(), bar_icon(), keys, ui::Kind::Ghost).clicked() {
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
    let (rect, _) = ui.allocate_exact_size(vec2(bar_btn(), 7.0), egui::Sense::hover());
    ui.painter().hline(rect.shrink2(vec2(8.0, 0.0)).x_range(), rect.center().y, Stroke::new(1.0, LINE));
}

/// Color chip for the vertical panel (two per row).
fn color_chip_small(ui: &mut egui::Ui, c: eframe::egui::Color32, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(bar_btn() / 2.0, 24.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 3, HOVER);
    }
    let chip = Rect::from_center_size(rect.center(), Vec2::splat(12.0));
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
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, bar_btn()), egui::Sense::hover());
    ui.painter().vline(rect.center().x, rect.shrink2(vec2(0.0, 11.0)).y_range(), Stroke::new(1.0, LINE));
}

/// The current color: a round chip in a button.
fn swatch(ui: &mut egui::Ui, c: eframe::egui::Color32, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, (size * 0.12).round() as u8, HOVER);
    }
    let r = size * 0.21;
    p.circle_filled(rect.center(), r, c);
    p.circle_stroke(rect.center(), r + 2.5, Stroke::new(1.2, if resp.hovered() { TEXT } else { DIM }));
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn color_chip(ui: &mut egui::Ui, c: eframe::egui::Color32, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, bar_btn()), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect.shrink2(vec2(0.0, 7.0)), 4, HOVER);
    }
    let chip = Rect::from_center_size(rect.center(), Vec2::splat(16.0));
    if selected {
        p.rect_stroke(chip.expand(3.0), 2, Stroke::new(1.0, TEXT), StrokeKind::Inside);
    }
    p.rect_filled(chip, 2, c);
    if c.r() < 40 && c.g() < 40 && c.b() < 40 {
        p.rect_stroke(chip, 2, Stroke::new(1.0, LINE), StrokeKind::Inside);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

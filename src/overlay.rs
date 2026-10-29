//! Full-desktop selection overlay drawn over a frozen snapshot.

use crate::capture::Snapshot;
use crate::config::{CaptureKey, Config};
use crate::editor::doc::{self, Kind, Shape};
use crate::editor::preview::{PrimPainter, View};
use crate::icons::Icon;
use crate::runtime::{self, Command, Shared};
use crate::theme::{self, *};
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
    /// Read the text in it (`ocr`).
    Text,
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
    run_with(shared, snap, if record { Mode::Record } else { Mode::Capture }, gif, None)
}

/// "Text from the screen": a selection is read for its text right away.
pub fn text(shared: &Arc<Shared>, snap: Snapshot) -> Option<Output> {
    run_with(shared, snap, Mode::Text, false, None)
}

/// The color picker: the frozen screen with the loupe; a click copies the
/// color under the pointer as hex.
pub fn pick(shared: &Arc<Shared>, snap: Snapshot) {
    run_with(shared, snap, Mode::Pick, false, None);
}

/// Scripted run for the README animation (see `demo`): off-screen, cloaked,
/// driven by `player` instead of the mouse.
pub fn run_demo(shared: &Arc<Shared>, snap: Snapshot, player: crate::demo::Player, record: bool) {
    run_with(shared, snap, if record { Mode::Record } else { Mode::Capture }, false, Some(player));
}

/// What the overlay is for.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Capture,
    /// An area for a screen recording (no annotations, one action).
    Record,
    Pick,
    /// A selection is read for its text (no panels, one action).
    Text,
}

fn run_with(shared: &Arc<Shared>, snap: Snapshot, mode: Mode, gif: bool, demo: Option<crate::demo::Player>) -> Option<Output> {
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
    runtime::run_window(shared, "overlay", viewport, false, move |cc| {
        let hwnd = win::hwnd_of(cc);
        if let Some(h) = hwnd {
            win::disable_transitions(h);
            // DWM-cloaked (invisible but rendering) until real frames are on
            // screen: no blank/white window on any monitor.
            win::cloak(h, true);
            // a bare popup (Windows: no caption left on it; macOS: above the
            // menu bar, the Dock and full-screen apps)
            win::make_overlay(h);
            win::set_window_rect(h, target, true);
        }
        if demo.is_some() {
            // frames are saved 1:1 with the made-up desktop
            cc.egui_ctx.set_pixels_per_point(1.0);
        }
        let mut list = vec![OverlayApp::new(&cc.egui_ctx, &shared2, snap, target, hwnd, mode, gif, demo, out2.clone(), None)];
        for (i, s) in others.into_iter().enumerate() {
            let t = IRect::new(s.origin.0, s.origin.1, s.image.width() as i32, s.image.height() as i32);
            let title = format!("blinker overlay {}", i + 1);
            list.push(OverlayApp::new(&cc.egui_ctx, &shared2, s, t, None, mode, gif, None, out2.clone(), Some(title)));
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
        mode: Mode,
        gif: bool,
        demo: Option<crate::demo::Player>,
        out: Rc<RefCell<Option<Output>>>,
        title: Option<String>,
    ) -> Self {
        let name = format!("snapshot-{}", title.as_deref().unwrap_or("main"));
        let tex = TiledTexture::new(ctx, &name, &snap.image, TextureOptions::NEAREST);
        let c = shared.config().editor_color;
        let primary = title.is_none();
        let (record, pick, ocr) = (mode == Mode::Record, mode == Mode::Pick, mode == Mode::Text);
        let mut cfg = shared.config();
        if pick {
            // the picker is the loupe: always there, never zoomed out of sight
            cfg.magnifier = true;
            if cfg.loupe_cell <= 0.0 {
                cfg.loupe_cell = LOUPE_STEPS[4];
            }
        }
        // instant mode copies on release: only the "select" step shows, the
        // first capture ends it
        let tour = (!cfg.tour_done && !record && !pick && !ocr)
            .then_some(Tour { step: TourStep::Select, at: f64::NAN, rect: None, shapes: 0 });
        // QR codes are looked for while the user aims (not in the picker)
        let qr: Arc<std::sync::Mutex<Option<Vec<Qr>>>> = Arc::default();
        if mode == Mode::Capture && demo.is_none() && cfg.read_qr {
            let (qr, img) = (qr.clone(), snap.image.clone());
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let found = find_qr(&img);
                let any = !found.is_empty();
                *qr.lock().unwrap() = Some(found);
                if any {
                    ctx.request_repaint();
                }
            });
        }
        OverlayApp {
            qr,
            qr_rects: Vec::new(),
            qr_seen: None,
            cfg,
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
            size_shown: None,
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
            pick,
            ocr,
            picked_at: None,
            gif,
            draw_path: Vec::new(),
            clicked_window: None,
            whole_window: false,
            bar_pos: None,
            bar_trip: None,
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
            glass_img: None,
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
    Ellipse,
    Pen,
    Highlight,
    Steps,
    Blur,
    Redact,
    TextBox,
    Text,
}

/// A tool's name for its remembered size.
fn tool_key(tool: Tool) -> &'static str {
    match tool {
        Tool::Region => "region",
        Tool::Arrow => "arrow",
        Tool::Rect => "rect",
        Tool::Ellipse => "ellipse",
        Tool::Pen => "pen",
        Tool::Highlight => "marker",
        Tool::Steps => "steps",
        Tool::Blur => "blur",
        Tool::Redact => "redact",
        Tool::TextBox => "text-plate",
        Tool::Text => "text",
    }
}

/// Dark or light, whichever reads on `c`.
fn contrast_on(c: Color32) -> Color32 {
    let l = 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
    if l > 150.0 { Color32::from_rgb(22, 22, 21) } else { Color32::WHITE }
}

/// Tools in toolbar order, with their keys (the rectangle came later: 0).
const TOOLS: [(Tool, Icon, &str, Key); 11] = [
    (Tool::Region, Icon::Region, "выделение области  1", Key::Num1),
    (Tool::Arrow, Icon::Arrow, "стрелка  2 · с shift — дугой", Key::Num2),
    (Tool::Rect, Icon::Rect, "прямоугольник  0", Key::Num0),
    (Tool::Ellipse, Icon::Ellipse, "круг  o · с shift — ровный", Key::O),
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
    /// Drawing an annotation; `last` = pointer on the previous frame (Ctrl
    /// carries what's being drawn, as it carries a selection).
    Draw { id: u64, start: Pos2, last: Pos2 },
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

/// A QR code found on the frozen screen: its box (image px) and what it says.
#[derive(Clone)]
struct Qr {
    rect: Rect,
    text: String,
}

/// The QR codes in `img` (looked for in the background when a capture opens).
fn find_qr(img: &RgbaImage) -> Vec<Qr> {
    let gray = image::DynamicImage::ImageRgba8(img.clone()).to_luma8();
    let (w, h) = gray.dimensions();
    let mut out: Vec<Qr> = Vec::new();
    let mut add = |rect: Rect, text: &str| {
        let text = text.trim();
        let same = |q: &Qr| q.text == text && q.rect.expand(8.0).intersects(rect);
        if !text.is_empty() && !out.iter().any(same) {
            out.push(Qr { rect, text: text.to_string() });
        }
    };
    // a Mac reads them with Vision first, as its camera does
    #[cfg(target_os = "macos")]
    for (text, rect) in vision_qr(img) {
        add(rect, &text);
    }
    // ZXing (rxing): small, tilted, colored and light-on-dark codes; a second
    // look at twice the size finds the tiny ones in thumbnails
    let mut zxing = |luma: Vec<u8>, w: u32, h: u32, k: f32| {
        use rxing::multi::MultipleBarcodeReader;
        let mut hints = rxing::DecodeHints::default().with(rxing::DecodeHintValue::TryHarder(true)).with(rxing::DecodeHintValue::AlsoInverted(true));
        // the generic reader, which cuts the picture up around what it finds
        // (its points: the code's corners), then the one made for several QR
        // codes at once (its points: the finder patterns' middles, about a
        // sixth of the code in from its edges)
        let mut found: Vec<(rxing::RXingResult, f32)> = Vec::new();
        if let Ok(src) = rxing::Luma8LuminanceSource::new(luma.clone(), w, h) {
            let mut bitmap = rxing::BinaryBitmap::new(rxing::common::HybridBinarizer::new(src));
            let multi = rxing::multi::qrcode::QRCodeMultiReader::default().decode_multiple_with_hints(&mut bitmap, &hints).unwrap_or_default();
            found.extend(rxing::helpers::detect_multiple_in_luma_with_hints(luma, w, h, &mut hints).unwrap_or_default().into_iter().map(|r| (r, 0.0)));
            found.extend(multi.into_iter().map(|r| (r, 0.16)));
        }
        for (r, grow) in found {
            if *r.getBarcodeFormat() != rxing::BarcodeFormat::QR_CODE {
                continue;
            }
            let pts = r.getPoints();
            if pts.is_empty() {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for p in pts {
                (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
            }
            let grow = (x1 - x0).max(y1 - y0) * grow;
            let rect = Rect::from_min_max(pos2(x0 - grow, y0 - grow), pos2(x1 + grow, y1 + grow));
            add(Rect::from_min_max(pos2(rect.min.x / k, rect.min.y / k), pos2(rect.max.x / k, rect.max.y / k)), r.getText());
        }
    };
    zxing(gray.as_raw().clone(), w, h, 1.0);
    if w.max(h) <= 2600 {
        let big = image::imageops::resize(&gray, w * 2, h * 2, image::imageops::FilterType::Triangle);
        zxing(big.into_raw(), w * 2, h * 2, 2.0);
    }
    // and rqrr, which reads some that ZXing passes over
    let mut prepared = rqrr::PreparedImage::prepare(gray);
    for grid in prepared.detect_grids() {
        let Ok((_, text)) = grid.decode() else { continue };
        let xs = grid.bounds.iter().map(|p| p.x as f32);
        let ys = grid.bounds.iter().map(|p| p.y as f32);
        let (x0, x1) = (xs.clone().fold(f32::MAX, f32::min), xs.fold(f32::MIN, f32::max));
        let (y0, y1) = (ys.clone().fold(f32::MAX, f32::min), ys.fold(f32::MIN, f32::max));
        add(Rect::from_min_max(pos2(x0, y0), pos2(x1, y1)), &text);
    }
    out
}

/// The QR codes Vision finds in `img`: their text and place in its pixels.
#[cfg(target_os = "macos")]
fn vision_qr(img: &RgbaImage) -> Vec<(String, Rect)> {
    use std::ffi::{CStr, c_char, c_void};
    extern "C" fn found(ctx: *mut c_void, text: *const c_char, x: f64, y: f64, w: f64, h: f64) {
        let out = unsafe { &mut *(ctx as *mut Vec<(String, f64, f64, f64, f64)>) };
        let text = unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned();
        out.push((text, x, y, w, h));
    }
    let mut raw: Vec<(String, f64, f64, f64, f64)> = Vec::new();
    unsafe { win::ffi::bk_qr(img.as_raw().as_ptr(), img.width() as i32, img.height() as i32, &mut raw as *mut _ as *mut c_void, found) };
    let (w, h) = (img.width() as f32, img.height() as f32);
    raw.into_iter()
        .map(|(t, x, y, bw, bh)| (t, Rect::from_min_size(pos2(x as f32 * w, y as f32 * h), vec2(bw as f32 * w, bh as f32 * h))))
        .collect()
}

/// A link as the card shows it: the site, then the rest of the address.
fn split_link(url: &str) -> (String, String) {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    match rest.split_once('/') {
        Some((host, path)) if !path.is_empty() => (host.to_string(), short(&format!("/{path}"), 46)),
        Some((host, _)) => (host.to_string(), String::new()),
        None => (short(rest, 40), String::new()),
    }
}

/// `s` cut to `n` letters with an ellipsis.
fn short(s: &str, n: usize) -> String {
    if s.chars().count() > n { format!("{}…", s.chars().take(n - 1).collect::<String>()) } else { s.to_string() }
}

/// Overshoots a little, then settles (the brackets snapping on).
fn back_out(t: f32) -> f32 {
    let x = t - 1.0;
    1.0 + 2.2 * x.powi(3) + 1.2 * x.powi(2)
}

/// A QR's text that opens in the browser (or another app).
fn qr_link(text: &str) -> Option<String> {
    let t = text.trim();
    let lower = t.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("tg://") || lower.starts_with("mailto:") {
        Some(t.to_string())
    } else if lower.starts_with("www.") && !t.contains(' ') {
        Some(format!("https://{t}"))
    } else {
        None
    }
}

/// A cryptocurrency network.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Chain {
    Evm,
    Btc,
    Tron,
    Solana,
    Ton,
    Ltc,
    Doge,
    Xrp,
}

/// A wallet address in a QR code (bare, or as a `bitcoin:`-style link): its
/// network, the address, the amount asked for, and for EVM the chain's id.
#[derive(Clone, PartialEq, Debug)]
struct Wallet {
    chain: Chain,
    address: String,
    amount: Option<String>,
    chain_id: Option<u64>,
}

impl Wallet {
    /// The network as the card names it (EVM by its chain when the link says).
    fn network(&self) -> &'static str {
        match self.chain {
            Chain::Evm => match self.chain_id {
                Some(56) => "BNB Chain",
                Some(137) => "Polygon",
                Some(8453) => "Base",
                Some(42161) => "Arbitrum",
                Some(10) => "Optimism",
                Some(43114) => "Avalanche",
                _ => "Ethereum · EVM",
            },
            Chain::Btc => "Bitcoin",
            Chain::Tron => "TRON",
            Chain::Solana => "Solana",
            Chain::Ton => "TON",
            Chain::Ltc => "Litecoin",
            Chain::Doge => "Dogecoin",
            Chain::Xrp => "XRP",
        }
    }

    /// The coin's ticker, for the amount.
    fn ticker(&self) -> &'static str {
        match self.chain {
            Chain::Evm => "ETH",
            Chain::Btc => "BTC",
            Chain::Tron => "TRX",
            Chain::Solana => "SOL",
            Chain::Ton => "TON",
            Chain::Ltc => "LTC",
            Chain::Doge => "DOGE",
            Chain::Xrp => "XRP",
        }
    }

    /// The address on the network's explorer.
    fn explorer(&self) -> String {
        let a = &self.address;
        match self.chain {
            Chain::Evm => {
                let host = match self.chain_id {
                    Some(56) => "bscscan.com",
                    Some(137) => "polygonscan.com",
                    Some(8453) => "basescan.org",
                    Some(42161) => "arbiscan.io",
                    Some(10) => "optimistic.etherscan.io",
                    Some(43114) => "snowtrace.io",
                    _ => "etherscan.io",
                };
                format!("https://{host}/address/{a}")
            }
            Chain::Btc => format!("https://mempool.space/address/{a}"),
            Chain::Tron => format!("https://tronscan.org/#/address/{a}"),
            Chain::Solana => format!("https://solscan.io/account/{a}"),
            Chain::Ton => format!("https://tonviewer.com/{a}"),
            Chain::Ltc => format!("https://litecoinspace.org/address/{a}"),
            Chain::Doge => format!("https://blockchair.com/dogecoin/address/{a}"),
            Chain::Xrp => format!("https://xrpscan.com/account/{a}"),
        }
    }

    /// The address shortened in the middle: 0x12ab…9fc3.
    fn short(&self) -> String {
        let n = self.address.chars().count();
        if n <= 16 {
            return self.address.clone();
        }
        let head: String = self.address.chars().take(6).collect();
        let tail: String = self.address.chars().skip(n - 4).collect();
        format!("{head}…{tail}")
    }
}

fn base58(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() && !"0OIl".contains(c))
}

/// The wallet a QR code holds, if it is one.
fn wallet_of(text: &str) -> Option<Wallet> {
    let t = text.trim();
    // a link: "bitcoin:bc1…?amount=0.01", "ethereum:0x…@56", "ton://transfer/EQ…"
    let (scheme, rest) = match t.split_once(':') {
        Some((s, r)) if s.chars().all(|c| c.is_ascii_alphabetic()) && s.len() > 2 => (Some(s.to_ascii_lowercase()), r.trim_start_matches("//")),
        _ => (None, t),
    };
    let rest = rest.strip_prefix("transfer/").unwrap_or(rest);
    let (head, query) = rest.split_once('?').unwrap_or((rest, ""));
    let amount = query.split('&').find_map(|kv| kv.strip_prefix("amount=")).filter(|v| !v.is_empty()).map(|v| v.to_string());
    // EIP-681: ethereum:0x…@chainId/function
    let head = head.split('/').next().unwrap_or(head);
    let (addr, chain_id) = match head.split_once('@') {
        Some((a, id)) => (a, id.parse().ok()),
        None => (head, None),
    };
    let addr = addr.trim();
    let wallet = |chain| Some(Wallet { chain, address: addr.to_string(), amount: amount.clone(), chain_id });
    let hex40 = addr.len() == 42 && addr.starts_with("0x") && addr[2..].chars().all(|c| c.is_ascii_hexdigit());
    let lower = addr.to_ascii_lowercase();
    let len = addr.len();
    let by_shape = || {
        if hex40 {
            wallet(Chain::Evm)
        } else if (lower.starts_with("bc1") && (14..=74).contains(&len) && lower[3..].chars().all(|c| c.is_ascii_alphanumeric()))
            || ((addr.starts_with('1') || addr.starts_with('3')) && (26..=35).contains(&len) && base58(addr))
        {
            wallet(Chain::Btc)
        } else if addr.starts_with('T') && len == 34 && base58(addr) {
            wallet(Chain::Tron)
        } else if (lower.starts_with("ltc1") && (14..=75).contains(&len)) || ((addr.starts_with('L') || addr.starts_with('M')) && (26..=35).contains(&len) && base58(addr)) {
            wallet(Chain::Ltc)
        } else if addr.starts_with('D') && len == 34 && base58(addr) {
            wallet(Chain::Doge)
        } else if addr.starts_with('r') && (25..=35).contains(&len) && base58(addr) {
            wallet(Chain::Xrp)
        } else if len == 48 && ["EQ", "UQ", "kQ", "0Q"].iter().any(|p| addr.starts_with(p)) && addr.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            wallet(Chain::Ton)
        } else if (32..=44).contains(&len)
            && base58(addr)
            && addr.chars().any(|c| c.is_ascii_digit())
            && addr.chars().any(|c| c.is_ascii_uppercase())
            && addr.chars().any(|c| c.is_ascii_lowercase())
        {
            wallet(Chain::Solana)
        } else {
            None
        }
    };
    match scheme.as_deref() {
        // a link names its network itself
        Some("ethereum") if hex40 => wallet(Chain::Evm),
        Some("bitcoin") => wallet(Chain::Btc),
        Some("tron") => wallet(Chain::Tron),
        Some("solana") => wallet(Chain::Solana),
        Some("ton") => wallet(Chain::Ton),
        Some("litecoin") => wallet(Chain::Ltc),
        Some("dogecoin") => wallet(Chain::Doge),
        Some("xrpl" | "xrp" | "ripple") => wallet(Chain::Xrp),
        Some(_) => None,
        // a bare address goes by its shape (no spaces, nothing else around it)
        None if !t.contains(char::is_whitespace) => by_shape(),
        None => None,
    }
}

/// The network's mark on its colors, in a card's tile `r`.
fn paint_chain(p: &egui::Painter, r: Rect, chain: Chain) {
    let c = r.center();
    let white = Color32::WHITE;
    let (bg, fg) = match chain {
        Chain::Evm => (Color32::from_rgb(0x62, 0x7e, 0xea), white),
        Chain::Btc => (Color32::from_rgb(0xf7, 0x93, 0x1a), white),
        Chain::Tron => (Color32::from_rgb(0xeb, 0x00, 0x29), white),
        Chain::Solana => (Color32::from_rgb(0x14, 0x14, 0x1c), white),
        Chain::Ton => (Color32::from_rgb(0x00, 0x98, 0xea), white),
        Chain::Ltc => (Color32::from_rgb(0x34, 0x5d, 0x9d), white),
        Chain::Doge => (Color32::from_rgb(0xc2, 0xa6, 0x33), white),
        Chain::Xrp => (Color32::from_rgb(0x23, 0x29, 0x2f), white),
    };
    p.circle_filled(c, r.width() / 2.0, bg);
    let stroke = Stroke::new(1.8, fg);
    let pt = |x: f32, y: f32| c + vec2(x, y);
    match chain {
        // the diamond: a top half, a bottom chevron
        Chain::Evm => {
            p.add(egui::Shape::convex_polygon(vec![pt(0.0, -10.0), pt(6.5, 0.5), pt(0.0, 4.0), pt(-6.5, 0.5)], fg, Stroke::NONE));
            p.add(egui::Shape::convex_polygon(vec![pt(-6.5, 2.5), pt(0.0, 6.0), pt(6.5, 2.5), pt(0.0, 10.5)], fg.gamma_multiply(0.75), Stroke::NONE));
        }
        // a B with the two strokes through it
        Chain::Btc => {
            p.text(c + vec2(0.5, 0.5), egui::Align2::CENTER_CENTER, "B", font_semibold(19.0), fg);
            for x in [-2.0, 2.0] {
                p.line_segment([pt(x, -11.0), pt(x, -7.5)], Stroke::new(1.8, fg));
                p.line_segment([pt(x, 7.5), pt(x, 11.0)], Stroke::new(1.8, fg));
            }
        }
        // the triangle, its inner lines
        Chain::Tron => {
            let (a, b, d) = (pt(-8.5, -7.0), pt(9.0, -1.0), pt(-1.0, 10.0));
            p.add(egui::Shape::closed_line(vec![a, b, d], stroke));
            p.line_segment([a, pt(1.5, 0.5)], Stroke::new(1.4, fg));
            p.line_segment([pt(1.5, 0.5), d], Stroke::new(1.4, fg));
            p.line_segment([pt(1.5, 0.5), b], Stroke::new(1.4, fg));
        }
        // three slanted bars, purple to green
        Chain::Solana => {
            let cols = [Color32::from_rgb(0x99, 0x45, 0xff), Color32::from_rgb(0x58, 0x9c, 0xd0), Color32::from_rgb(0x14, 0xf1, 0x95)];
            for (k, col) in cols.into_iter().enumerate() {
                let y = -7.0 + k as f32 * 6.0;
                let s = if k == 1 { -1.0 } else { 1.0 };
                p.add(egui::Shape::convex_polygon(vec![pt(-8.0 + 2.0 * s, y), pt(9.0 + 2.0 * s, y), pt(8.0 - 2.0 * s, y + 3.6), pt(-9.0 - 2.0 * s, y + 3.6)], col, Stroke::NONE));
            }
        }
        // the gem: a downward triangle, split down the middle
        Chain::Ton => {
            let (l, rr, b) = (pt(-8.5, -6.5), pt(8.5, -6.5), pt(0.0, 9.0));
            p.add(egui::Shape::closed_line(vec![l, rr, b], stroke));
            p.line_segment([pt(0.0, -6.5), b], stroke);
        }
        Chain::Ltc => {
            p.text(c + vec2(0.5, 0.5), egui::Align2::CENTER_CENTER, "Ł", font_semibold(19.0), fg);
        }
        Chain::Doge => {
            p.text(c + vec2(0.5, 0.5), egui::Align2::CENTER_CENTER, "Ð", font_semibold(19.0), fg);
        }
        // two arcs that cross: an X
        Chain::Xrp => {
            p.line_segment([pt(-7.5, -7.0), pt(7.5, 7.0)], stroke);
            p.line_segment([pt(7.5, -7.0), pt(-7.5, 7.0)], stroke);
        }
    }
}

struct OverlayApp {
    /// QR codes on the screen, once found (under each: where it leads).
    qr: Arc<std::sync::Mutex<Option<Vec<Qr>>>>,
    /// Their cards' rects (points), last frame: clicks there aren't a selection.
    qr_rects: Vec<Rect>,
    /// When they were first drawn (the scan animation starts then).
    qr_seen: Option<f64>,
    /// macOS 26: a blurred copy of the screenshot the panels are made of,
    /// so they look like Liquid Glass over what's under them.
    glass: Option<TextureHandle>,
    /// The same blurred quarter-size picture, to tell how light it is under a pane.
    glass_img: Option<RgbaImage>,
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
    /// When the wheel last changed the tool's size: its preview shows at
    /// the pointer for a moment.
    size_shown: Option<f64>,
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
    /// The color picker (see `pick`).
    pick: bool,
    /// "Text from the screen": the selection is read for its text at once.
    ocr: bool,
    /// When the picker copied: the note shows a moment, then it closes.
    picked_at: Option<f64>,
    gif: bool,
    /// Pointer trail of the arrow being drawn (bends a Shift arrow).
    draw_path: Vec<Pos2>,
    /// Window picked by a click: (its rect in image px, handle).
    clicked_window: Option<(Rect, isize)>,
    /// Alt is held: a click takes the whole window, not the part under it.
    whole_window: bool,
    /// Animated position of the tools panel (points).
    bar_pos: Option<Pos2>,
    /// The bar crossing to another display: where it set off from, and when.
    bar_trip: Option<(Pos2, f64)>,
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

    /// Window or monitor under the pointer (image px): the part of the
    /// window under it (a browser's page…) unless Alt is held or it's for
    /// a recording (a window is recorded whole).
    fn hover_target(&self, p: Pos2) -> Rect {
        let (x, y) = (p.x as i32, p.y as i32);
        let r = self
            .snap
            .windows
            .iter()
            .find(|w| w.rect.contains(x, y))
            .map(|w| {
                // recording a maximized window is recording the screen: the
                // whole display (taskbar included), not the window alone
                if self.record {
                    let m = self.snap.monitor_at(w.rect.x + w.rect.w / 2, w.rect.y + w.rect.h / 2);
                    let covered = w.rect.intersect(&m.work);
                    if covered.w as i64 * covered.h as i64 * 100 >= m.work.w as i64 * m.work.h as i64 * 95 {
                        return m.rect;
                    }
                }
                let part = (!self.record && !self.whole_window).then(|| w.parts.iter().find(|r| r.contains(x, y))).flatten();
                part.copied().unwrap_or(w.rect)
            })
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

    /// The action a finished selection takes at once, with no panel: reading
    /// the text ("text from the screen"), or copying (instant mode).
    fn quick(&self) -> Option<Action> {
        if self.ocr {
            Some(Action::Text)
        } else if self.cfg.instant_mode {
            Some(Action::Copy)
        } else {
            None
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
        if self.whole_window != input.modifiers.alt {
            self.whole_window = input.modifiers.alt;
            ctx.request_repaint();
        }
        let pointer_pt = input.pointer.hover_pos().or(self.last_pointer);
        if let Some(p) = input.pointer.hover_pos() {
            self.last_pointer = Some(p);
        }
        let p = pointer_pt.map(|p| pos2(p.x * ppp, p.y * ppp));
        let over_toolbar = pointer_pt.is_some_and(|p| {
            self.qr_rects.iter().any(|r| r.contains(p))
                || self.toolbar_rect.is_some_and(|r| r.contains(p))
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
            // the picker never zooms the loupe out of sight
            let lowest = if self.pick { 1 } else { 0 };
            let next = (cur + if wheel > 0.0 { 1 } else { -1 }).clamp(lowest, LOUPE_STEPS.len() as i32 - 1) as usize;
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
        // the color picker: a click copies the color under the pointer
        if self.pick {
            if self.picked_at.is_none()
                && input.pointer.primary_released()
                && let Some(p) = p
            {
                let img = &self.snap.image;
                let px = img.get_pixel((p.x.max(0.0) as u32).min(img.width() - 1), (p.y.max(0.0) as u32).min(img.height() - 1));
                let hex = format!("#{:02x}{:02x}{:02x}", px[0], px[1], px[2]);
                crate::output::copy_text(&hex);
                crate::sound::play_capture(&self.shared.config());
                self.notice = Some((lf!("{hex} скопирован", "{hex} copied"), now));
                self.picked_at = Some(now);
            }
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
        // the actions' keys are the user's (the "hotkeys" page)
        let keys = |k: CaptureKey| self.cfg.capture_keys(k).iter().any(|b| crate::hotkey::pressed(&input, b));
        let copy_keys = keys(CaptureKey::Copy);
        if self.sel.is_some() && self.phase == Phase::Adjusting {
            if input.key_pressed(Key::Enter) || enter || copy_keys {
                return self.finish(Action::Copy, now);
            }
            for (k, action) in [
                (CaptureKey::Save, Action::Save),
                (CaptureKey::Edit, Action::Edit),
                (CaptureKey::Pin, Action::Pin),
                (CaptureKey::Beauty, Action::Beauty),
                (CaptureKey::Text, Action::Text),
            ] {
                if keys(k) {
                    return self.finish(action, now);
                }
            }
            if keys(CaptureKey::Upload) && crate::upload::configured(&self.shared.config()) {
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
            if let Some(a) = self.quick() {
                return self.finish(a, now);
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

        // Annotation tools draw anywhere (Ctrl still moves the selection, or
        // what's being drawn while it's drawn).
        let drawing = matches!(self.drag, Drag::Draw { .. });
        if self.tool != Tool::Region && (!input.modifiers.command || drawing) {
            self.draw_input(&input, p, ppp, over_toolbar);
            ctx.set_cursor_icon(if over_toolbar {
                CursorIcon::Default
            } else if drawing && input.modifiers.command {
                CursorIcon::Move
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
                    if let Some(a) = self.quick() {
                        self.drag = Drag::None;
                        return self.finish(a, now);
                    }
                }
                Drag::Creating { .. } => {
                    if self.sel.is_some_and(|s| s.width() >= 2.0 && s.height() >= 2.0) {
                        self.phase = Phase::Adjusting;
                        if let Some(a) = self.quick() {
                            self.drag = Drag::None;
                            return self.finish(a, now);
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
            && !self.pick
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

        // the picker shows the screen as it is
        let veil = if self.pick { Color32::TRANSPARENT } else { theme::look("veil", 0).bg_or(VEIL()).gamma_multiply(fade) };
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
            painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0, ACCENT().gamma_multiply(0.85 * fade)), 4.0, 3.0));
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

        if let (Some(at), Some(pt)) = (self.size_shown, pointer_pt) {
            let age = (now - at) as f32;
            if age > 1.1 {
                self.size_shown = None;
            } else {
                self.paint_size_preview(ctx, pt, ppp, (1.0 - (age - 0.7) / 0.4).clamp(0.0, 1.0));
                ctx.request_repaint();
            }
        }

        // transient notice (color copied)
        if let Some((text, at)) = &self.notice {
            if now - at > 1.4 {
                self.notice = None;
            } else {
                let fg = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::unique("ov-notice")));
                let at_pt = pointer_pt.unwrap_or(full.center()) + vec2(14.0, -30.0);
                let look = theme::look("label", 0);
                let galley = fg.layout_no_wrap(text.clone(), font_medium(12.5), look.fg_or(TEXT()));
                let rect = Rect::from_min_size(at_pt, galley.size() + vec2(16.0, 10.0));
                fg.rect_filled(rect, look.radius_or(4.0), look.bg_or(SHADE()));
                fg.galley(rect.min + vec2(8.0, 5.0), galley, look.fg_or(TEXT()));
                ctx.request_repaint();
            }
        }

        if appear < 1.0 || finishing.is_some() || closing.is_some() {
            ctx.request_repaint();
        }
    }

    /// Under each QR code on the screen: where it leads, and a button to go
    /// there (or to copy what it says). Hidden while a selection is dragged.
    fn qr_cards(&mut self, ctx: &egui::Context, ppp: f32) {
        // last frame's cards: their size, for the glass under them
        let last = std::mem::take(&mut self.qr_rects);
        let found = self.qr.lock().unwrap().clone().unwrap_or_default();
        if found.is_empty() || !matches!(self.drag, Drag::None) || matches!(self.phase, Phase::Finishing { .. } | Phase::Closing { .. }) {
            return;
        }
        // the moment they were found: the scan plays from it
        let now = ctx.input(|i| i.time);
        let seen = *self.qr_seen.get_or_insert(now);
        let age = (now - seen) as f32;
        if age < 1.2 {
            ctx.request_repaint();
        }
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("qr-frames")));
        let mut done = None;
        let mut placed: Vec<Rect> = Vec::new();
        for (i, q) in found.iter().enumerate() {
            let r = Rect::from_min_max(pos2(q.rect.min.x / ppp, q.rect.min.y / ppp), pos2(q.rect.max.x / ppp, q.rect.max.y / ppp));
            let age = (age - i as f32 * 0.08).max(0.0);
            // corner brackets close in on the code
            let t = back_out((age / 0.45).clamp(0.0, 1.0));
            let a = (age / 0.2).clamp(0.0, 1.0);
            let frame = r.expand(6.0 + (1.0 - t) * 22.0);
            let len = (frame.width().min(frame.height()) * 0.24).max(10.0);
            let stroke = Stroke::new(3.0, ACCENT().gamma_multiply(a));
            for (c, dx, dy) in [(frame.left_top(), 1.0, 1.0), (frame.right_top(), -1.0, 1.0), (frame.left_bottom(), 1.0, -1.0), (frame.right_bottom(), -1.0, -1.0)] {
                painter.add(egui::Shape::line(vec![c + vec2(0.0, dy * len), c, c + vec2(dx * len, 0.0)], stroke));
            }
            // a line of light sweeps down it once
            let sweep = (age - 0.1) / 0.55;
            if (0.0..1.0).contains(&sweep) {
                let y = r.top() + r.height() * ui::ease_out_cubic(sweep);
                let fade = (1.0 - sweep).min(sweep * 4.0).clamp(0.0, 1.0);
                let glow = Rect::from_min_max(pos2(r.left(), (y - r.height() * 0.25).max(r.top())), pos2(r.right(), y));
                let mut mesh = egui::Mesh::default();
                let clear = ACCENT().gamma_multiply(0.0);
                let lit = ACCENT().gamma_multiply(0.28 * fade);
                mesh.colored_vertex(glow.left_top(), clear);
                mesh.colored_vertex(glow.right_top(), clear);
                mesh.colored_vertex(glow.right_bottom(), lit);
                mesh.colored_vertex(glow.left_bottom(), lit);
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                painter.add(egui::Shape::mesh(mesh));
                painter.hline(r.x_range(), y, Stroke::new(2.0, ACCENT().gamma_multiply(fade)));
            }
            // the card slides up under it
            let card_t = ui::ease_out_cubic(((age - 0.18) / 0.3).clamp(0.0, 1.0));
            if card_t <= 0.0 {
                continue;
            }
            let wallet = wallet_of(&q.text);
            let link = if wallet.is_some() { None } else { qr_link(&q.text) };
            let (title, sub) = match (&wallet, &link) {
                (Some(w), _) => {
                    let amount = w.amount.as_ref().map(|a| format!(" · {a} {}", w.ticker())).unwrap_or_default();
                    (crate::i18n::l("криптокошелёк", "crypto wallet").to_string(), format!("{} · {}{amount}", w.network(), w.short()))
                }
                (None, Some(url)) => split_link(url),
                (None, None) => (crate::i18n::l("текст в qr-коде", "text in the qr code").to_string(), short(&q.text.replace(['\n', '\r'], " "), 46)),
            };
            // codes side by side: a card that would cover another one's goes
            // under it (sizes from the last frame)
            let size = last.get(i).map_or(vec2(300.0, 52.0), |r| r.shrink(4.0).size());
            let mut at = pos2(r.center().x, frame.bottom() + 12.0 + (1.0 - card_t) * 10.0);
            loop {
                let rect = Rect::from_min_size(at - vec2(size.x / 2.0, 0.0), size);
                match placed.iter().find(|p| p.expand(6.0).intersects(rect)) {
                    Some(p) => at.y = p.bottom() + 8.0,
                    None => {
                        placed.push(rect);
                        break;
                    }
                }
            }
            if last.get(i).is_some() {
                self.paint_glass(ctx, Rect::from_min_size(at - vec2(size.x / 2.0, 0.0), size), card_t, ppp, &format!("qr-glass-{i}"), QR_RADIUS, glass());
            }
            let glass = self.glass.is_some();
            let resp = egui::Area::new(Id::unique(("qr-card", i)))
                .order(egui::Order::Foreground)
                .pivot(egui::Align2::CENTER_TOP)
                .fixed_pos(at)
                .show(ctx, |ui| {
                    ui.set_opacity(card_t);
                    // on glass the card is clear: the pane painted under it shows
                    let frame = if glass {
                        egui::Frame::new().corner_radius(QR_RADIUS)
                    } else {
                        egui::Frame::new()
                            .fill(PANEL())
                            .stroke(Stroke::new(1.0, LINE_HI()))
                            .corner_radius(QR_RADIUS)
                            .shadow(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(110) })
                    };
                    frame.inner_margin(egui::Margin { left: 10, right: 8, top: 8, bottom: 8 }).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            // the kind of code, on an accent tile
                            let (tile, _) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::hover());
                            match &wallet {
                                Some(w) => paint_chain(ui.painter(), tile.shrink(2.0), w.chain),
                                None => {
                                    ui.painter().rect_filled(tile, 10, ACCENT().gamma_multiply(0.15));
                                    crate::icons::paint(ui.painter(), Rect::from_center_size(tile.center(), Vec2::splat(18.0)), if link.is_some() { Icon::Link } else { Icon::Scan }, 15.0, ACCENT());
                                }
                            }
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                ui.set_max_width(250.0);
                                ui.add_space(1.0);
                                ui.add(egui::Label::new(egui::RichText::new(&title).font(font_medium(13.5)).color(TEXT())).truncate());
                                if !sub.is_empty() {
                                    ui.add(egui::Label::new(egui::RichText::new(&sub).font(font(11.5)).color(DIM())).truncate());
                                }
                            })
                            .response
                            .on_hover_text(&q.text);
                            ui.add_space(4.0);
                            ui.spacing_mut().item_spacing.x = 4.0;
                            match (&wallet, &link) {
                                (Some(_), _) => {
                                    if ui::icon_button(ui, Icon::Link, crate::i18n::l("открыть в обозревателе", "open in the explorer"), 34.0, false, ui::Kind::Ghost).clicked() {
                                        done = Some((i, true));
                                    }
                                    if qr_button(ui, crate::i18n::l("копировать", "copy"), Icon::Copy).on_hover_text(crate::i18n::l("копировать адрес", "copy the address")).clicked() {
                                        done = Some((i, false));
                                    }
                                }
                                (None, Some(_)) => {
                                    if ui::icon_button(ui, Icon::Copy, crate::i18n::l("копировать ссылку", "copy the link"), 34.0, false, ui::Kind::Ghost).clicked() {
                                        done = Some((i, false));
                                    }
                                    if qr_button(ui, crate::i18n::l("перейти", "open"), Icon::Arrow).clicked() {
                                        done = Some((i, true));
                                    }
                                }
                                (None, None) => {
                                    if qr_button(ui, crate::i18n::l("копировать", "copy"), Icon::Copy).clicked() {
                                        done = Some((i, false));
                                    }
                                }
                            }
                        });
                    });
                });
            self.qr_rects.push(resp.response.rect.expand(4.0));
        }
        if let Some((i, open)) = done {
            let q = &found[i];
            if let Some(w) = wallet_of(&q.text) {
                // the address alone, or the address on its explorer
                if open {
                    win::open_url(&w.explorer());
                } else {
                    crate::output::copy_text(&w.address);
                }
                self.cancel(ctx);
                return;
            }
            match qr_link(&q.text) {
                Some(url) if open => win::open_url(&url),
                Some(url) => crate::output::copy_text(&url),
                None => crate::output::copy_text(&q.text),
            }
            // done here: the capture goes away
            self.cancel(ctx);
        }
    }

    /// The picker's note at the top of the display: what a click does.
    fn pick_hint(&self, ctx: &egui::Context) {
        let Some(m) = self.bar_mon else { return };
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("pick-hint")));
        let text = crate::i18n::l("пипетка · клик копирует цвет · колесо — масштаб · esc — выход", "color picker · click copies the color · wheel zooms · esc leaves");
        let look = theme::look("label", 0);
        let galley = painter.layout_no_wrap(text.to_string(), font_medium(12.5), look.fg_or(TEXT()));
        let size = galley.size() + vec2(24.0, 14.0);
        let rect = Rect::from_center_size(pos2(m.center().x, m.top() + 16.0 + size.y / 2.0), size);
        painter.rect_filled(rect, look.radius_or(size.y / 2.0), look.bg_or(SHADE()));
        painter.galley(rect.min + vec2(12.0, 7.0), galley, look.fg_or(TEXT()));
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
        let look = theme::look("label", 0);
        let radius = look.radius_or(4.0);
        painter.rect_filled(rect, radius, look.bg_or(SHADE()).gamma_multiply(alpha));
        painter.rect_stroke(rect, radius, look.border.map_or(Stroke::NONE, |b| Stroke::new(b.width, b.color.gamma_multiply(alpha))), StrokeKind::Inside);
        painter.galley(rect.min + vec2(9.0, 5.0), galley, look.fg_or(TEXT()).gamma_multiply(alpha));
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
        let hex = painter.layout_no_wrap(format!("#{:02x}{:02x}{:02x}", col[0], col[1], col[2]), mono(11.0), TEXT());
        let zoom = painter.layout_no_wrap(format!("×{:.0}", cell * ppp), mono(11.0), FAINT());
        let coords = painter.layout_no_wrap(format!("{cx}, {cy}"), mono(11.0), FAINT());
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
        painter.rect_filled(frame, 3, BG());
        painter.rect_stroke(frame, 3, Stroke::new(1.0, LINE()), StrokeKind::Inside);
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
        painter.rect_stroke(c, 0, Stroke::new(1.5, ACCENT()), StrokeKind::Outside);

        let row_y = img_rect.bottom() + 11.0;
        let sw = Rect::from_center_size(pos2(img_rect.left() + 6.0, row_y), Vec2::splat(9.0));
        painter.rect_filled(sw, 0, Color32::from_rgb(col[0], col[1], col[2]));
        painter.rect_stroke(sw, 0, Stroke::new(1.0, LINE()), StrokeKind::Outside);
        let h = hex.size().y;
        painter.galley(pos2(sw.right() + 6.0, row_y - h / 2.0), hex, TEXT());
        let zx = img_rect.right() - zoom.size().x;
        painter.galley(pos2(zx, row_y - h / 2.0), zoom, FAINT());
        if one_line {
            painter.galley(pos2(zx - 10.0 - coords.size().x, row_y - h / 2.0), coords, FAINT());
        } else {
            painter.galley(pos2(img_rect.left(), row_y + 16.0 - h / 2.0), coords, FAINT());
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
        let (frame, glassy) = panel_frame("toolbar");
        self.paint_glass(ctx, Rect::from_min_size(at, size), t, ppp, "toolbar-glass", frame.corner_radius.nw, glassy);
        let resp = egui::Area::new(Id::unique("toolbar"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                let _part = theme::part("toolbar");
                frame
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
                            // "save  cmd s": the name and the key the user set
                            let tip = |k: CaptureKey| {
                                let key = self.cfg.capture_hint(k);
                                if key.is_empty() { k.label().to_string() } else { format!("{}  {key}", k.label()) }
                            };
                            let copy_tip = match self.cfg.capture_hint(CaptureKey::Copy) {
                                k if k.is_empty() => crate::i18n::t("копировать  enter").to_string(),
                                k => format!("{} · {k}", crate::i18n::t("копировать  enter")),
                            };
                            if ui::icon_button_sized(ui, Icon::Copy, &copy_tip, bar_btn(), bar_icon(), false, ui::Kind::Accent).clicked() {
                                act = Some(Action::Copy);
                            }
                            for (k, icon, action) in [
                                (CaptureKey::Save, Icon::Save, Action::Save),
                                (CaptureKey::Edit, Icon::Edit, Action::Edit),
                                (CaptureKey::Pin, Icon::Pin, Action::Pin),
                                (CaptureKey::Beauty, Icon::Frame, Action::Beauty),
                                (CaptureKey::Text, Icon::Scan, Action::Text),
                            ] {
                                if ui::icon_button_sized(ui, icon, &tip(k), bar_btn(), bar_icon(), false, ui::Kind::Ghost).clicked() {
                                    act = Some(action);
                                }
                            }
                            if crate::upload::configured(&self.cfg)
                                && ui::icon_button_sized(ui, Icon::Upload, &tip(CaptureKey::Upload), bar_btn(), bar_icon(), false, ui::Kind::Ghost)
                                    .clicked()
                            {
                                act = Some(Action::Upload);
                            }
                            let (sep, _) = ui.allocate_exact_size(vec2(bar_btn(), 5.0), egui::Sense::hover());
                            ui.painter().hline(sep.shrink2(vec2(4.0, 0.0)).x_range(), sep.center().y, Stroke::new(1.0, LINE()));
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
            // Guard against a caption put back on the window, and against
            // DPI-change resizes moving/resizing us across monitors.
            #[cfg(windows)]
            if win::make_overlay(h) {
                win::set_window_rect(h, self.target, true);
            }
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
        if self.active && !self.pick && !self.ocr {
            self.tools_bar(&ctx);
        }
        if self.pick && self.active {
            self.pick_hint(&ctx);
        }
        self.qr_cards(&ctx, ppp);
        if let Some(at) = self.picked_at {
            if now - at >= 0.6 {
                close_all(&ctx);
            } else {
                ctx.request_repaint();
            }
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
    }

    fn tour(&mut self, ctx: &egui::Context, now: f64) {
        let Some(mut tour) = self.tour.take() else { return };
        if tour.at.is_nan() {
            tour.at = now;
            // shown once is enough, however this capture ends (esc too)
            if self.demo.is_none() {
                self.shared.update_config(|c| c.tour_done = true);
            }
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
                    (l("клик", "click").into(), l("окно или его часть", "a window or a part of it")),
                    ("alt + клик".into(), l("окно целиком", "the whole window")),
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
                // the keys the user set; an action without one is left out
                [
                    (CaptureKey::Copy, l("копировать", "copy")),
                    (CaptureKey::Save, l("сохранить", "save")),
                    (CaptureKey::Edit, l("открыть в редакторе", "open in the editor")),
                    (CaptureKey::Pin, l("закрепить поверх окон", "pin on top")),
                ]
                .into_iter()
                .filter_map(|(k, what)| Some((Some(self.cfg.capture_hint(k)).filter(|h| !h.is_empty())?, what)))
                .collect(),
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
        // a pane of glass like the panels, sized from the last frame
        const TOUR_RADIUS: u8 = 14;
        if let Some(r) = tour.rect {
            self.paint_glass(ctx, Rect::from_min_size(pos + slide, r.size()), t, ctx.pixels_per_point(), "tour-glass", TOUR_RADIUS, glass());
        }
        let glass = self.glass.is_some();
        let mut next = false;
        let mut skip = false;
        let resp = egui::Area::new(Id::unique("tour"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos + slide)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                egui::Frame::new()
                    .fill(if glass { Color32::TRANSPARENT } else { PANEL() })
                    .stroke(if glass { Stroke::NONE } else { Stroke::new(1.0, ACCENT().gamma_multiply(0.55)) })
                    .corner_radius(if glass { TOUR_RADIUS } else { 12 })
                    // a shadow under clear glass would darken the pane itself
                    .shadow(if glass { egui::epaint::Shadow::NONE } else { egui::epaint::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(110) } })
                    .inner_margin(egui::Margin::symmetric(16, 14))
                    .show(ui, |ui| {
                        ui.set_width(268.0);
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.label(egui::RichText::new(title).font(font_semibold(15.0)).color(TEXT()));
                        ui.add_space(2.0);
                        for (key, what) in &lines {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                keycap(ui, key);
                                ui.label(egui::RichText::new(*what).font(font(12.5)).color(DIM()));
                            });
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            // where we are: three dots
                            let (r, _) = ui.allocate_exact_size(vec2(34.0, 20.0), egui::Sense::hover());
                            for i in 0..3 {
                                let c = pos2(r.left() + 4.0 + i as f32 * 12.0, r.center().y);
                                let on = i == index;
                                let off = if glass { Color32::from_white_alpha(46) } else { LINE() };
                                ui.painter().circle_filled(c, if on { 3.5 } else { 2.5 }, if on { ACCENT() } else { off });
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
            // on glass: the tint and rim of the pane it sticks out of
            let (fill, edge) = if glass {
                (Color32::from_rgba_unmultiplied(34, 34, 33, 235), Color32::from_white_alpha(42))
            } else {
                (PANEL(), ACCENT().gamma_multiply(0.55))
            };
            painter.add(egui::Shape::convex_polygon(tri.clone(), fill.gamma_multiply(t), Stroke::NONE));
            painter.line(vec![tri[1], tri[0], tri[2]], Stroke::new(1.0, edge.gamma_multiply(t)));
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

/// How round a QR card is.
const QR_RADIUS: u8 = 14;

/// The card's main action: an accent pill with its label and an icon after
/// it, lighter on hover.
fn qr_button(ui: &mut egui::Ui, label: &str, icon: Icon) -> egui::Response {
    let g = ui.painter().layout_no_wrap(label.to_string(), font_semibold(13.0), ON_ACCENT());
    let size = vec2(14.0 + g.size().x + 6.0 + 14.0 + 12.0, 34.0);
    let (r, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let h = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), 0.1);
    let p = ui.painter();
    p.rect_filled(r, 17, crate::theme::lerp_color(ACCENT(), ACCENT_HI(), h));
    // a highlight along the top edge, like a pressed-in key
    p.rect_filled(Rect::from_min_size(r.min + vec2(12.0, 1.0), vec2(r.width() - 24.0, 1.0)), 0, Color32::from_white_alpha((40.0 + 30.0 * h) as u8));
    p.galley(pos2(r.left() + 14.0, r.center().y - g.size().y / 2.0), g.clone(), ON_ACCENT());
    let ic = Rect::from_center_size(pos2(r.left() + 14.0 + g.size().x + 6.0 + 7.0 + h * 1.5, r.center().y - h * 1.5), Vec2::splat(14.0));
    crate::icons::paint(p, ic, icon, 12.0, ON_ACCENT());
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A key name on a little plate, as on a keyboard.
fn keycap(ui: &mut egui::Ui, key: &str) {
    let g = ui.painter().layout_no_wrap(key.to_string(), mono(11.5), TEXT());
    let w = (g.size().x + 12.0).max(52.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 20.0), egui::Sense::hover());
    let (fill, edge) = if glass() { (Color32::from_white_alpha(14), Color32::from_white_alpha(24)) } else { (HOVER(), LINE()) };
    ui.painter().rect_filled(r, 6, fill);
    ui.painter().rect_stroke(r, 6, Stroke::new(1.0, edge), StrokeKind::Inside);
    ui.painter().galley(r.center() - g.size() / 2.0, g, TEXT());
}

/// Closes the overlay: the main window, and with it every display's window.
fn close_all(ctx: &egui::Context) {
    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
}

/// Viewfinder frame: dashed hairline plus solid corner brackets.
/// The frame of a panel over the screen (`toolbar`, `tools`) as the theme
/// has it, and whether it's glass: then the frame is clear, the glass is
/// painted under it.
fn panel_frame(part: &str) -> (egui::Frame, bool) {
    let look = theme::look(part, 0);
    let glassy = glass() || look.glass();
    let radius = look.radius.map_or(9, |r| r.round() as u8);
    let frame = egui::Frame::new()
        .fill(if glassy { Color32::TRANSPARENT } else { look.bg_or(PANEL()) })
        .stroke(match look.border {
            Some(b) => b,
            None if glassy => Stroke::NONE,
            None => Stroke::new(1.0, LINE()),
        })
        .corner_radius(radius)
        .inner_margin(egui::Margin::same(5));
    (frame, glassy)
}

fn paint_viewfinder(painter: &egui::Painter, r: Rect, alpha: f32) {
    if alpha <= 0.0 || r.width() < 1.0 || r.height() < 1.0 {
        return;
    }
    // the theme: the frame's color and thickness (the corners a bit bolder)
    let look = theme::look("selection", 0);
    let frame = look.border_or(Stroke::new(1.0, ACCENT().gamma_multiply(0.85)));
    let line = Stroke::new(frame.width, frame.color.gamma_multiply(alpha));
    let pts = vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    painter.extend(egui::Shape::dashed_line(&pts, line, 6.0, 4.0));
    let len = 18.0f32.min(r.width() / 2.5).min(r.height() / 2.5).max(5.0);
    let o = r.expand(1.5);
    let corner = look.border.map_or(ACCENT(), |b| b.color);
    let bracket = Stroke::new(frame.width + 1.5, corner.gamma_multiply(alpha));
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
        (self.tool_width(self.tool) * ppp.max(1.0)).round()
    }

    /// `tool`'s own size (points): its thickness, or its text's.
    fn tool_width(&self, tool: Tool) -> f32 {
        self.cfg.tool_widths.get(tool_key(tool)).copied().unwrap_or(self.cfg.editor_width)
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
        let width = self.tool_width(if t.plate { Tool::TextBox } else { Tool::Text }) * self.last_ppp.max(1.0);
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
                    self.drag = Drag::Draw { id, start: p, last: p };
                }
                Tool::Pen => {
                    let id = self.push_shape(Kind::Pen { pts: vec![p] }, w);
                    self.drag = Drag::Draw { id, start: p, last: p };
                }
                Tool::Rect | Tool::Ellipse | Tool::Highlight | Tool::Blur | Tool::Redact => {
                    let r = Rect::from_two_pos(p, p);
                    let kind = match self.tool {
                        Tool::Rect => Kind::Rect { r },
                        Tool::Ellipse => Kind::Ellipse { r },
                        Tool::Highlight => Kind::Highlight { r },
                        Tool::Blur => Kind::Pixelate { r },
                        _ => Kind::Redact { r },
                    };
                    let id = self.push_shape(kind, w);
                    self.drag = Drag::Draw { id, start: p, last: p };
                }
                Tool::Region => {}
            }
        }
        if input.pointer.primary_down()
            && let Drag::Draw { id, start, last } = self.drag
            && input.modifiers.command
        {
            // Ctrl held: the shape follows the mouse whole, as a selection
            // does; letting go of Ctrl goes on drawing from where it is
            let d = p - last;
            if let Some(sh) = self.shapes.iter_mut().find(|s| s.id == id) {
                sh.translate(d);
            }
            for q in &mut self.draw_path {
                *q += d;
            }
            self.drag = Drag::Draw { id, start: start + d, last: p };
        } else if input.pointer.primary_down()
            && let Drag::Draw { id, start, .. } = self.drag
        {
            self.drag = Drag::Draw { id, start, last: p };
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
                        Kind::Rect { r } | Kind::Ellipse { r } => {
                            // Shift: a square, a circle
                            let end = if input.modifiers.shift {
                                let d = p - start;
                                let side = d.x.abs().max(d.y.abs());
                                start + vec2(side * d.x.signum(), side * d.y.signum())
                            } else {
                                p
                            };
                            *r = Rect::from_two_pos(start, end).intersect(img)
                        }
                        Kind::Highlight { r } | Kind::Pixelate { r } | Kind::Redact { r } => *r = Rect::from_two_pos(start, p).intersect(img),
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
                Kind::Rect { r } | Kind::Ellipse { r } | Kind::Highlight { r } | Kind::Pixelate { r } | Kind::Redact { r } => r.width() > min && r.height() > min,
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
        let Some(mut mon) = self.bar_mon else { return };
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
        // at the top of the screen the bar goes to the display the pointer is
        // on, gliding over as it does to a selection
        let at_top = self.sel.is_none() || mode == crate::config::ToolsPos::Screen || self.phase == Phase::Selecting;
        if at_top && let Some(p) = ctx.pointer_latest_pos() {
            let c = self.snap.monitor_at((p.x * ppp) as i32, (p.y * ppp) as i32).rect;
            let m = Rect::from_min_max(pos2(c.x as f32 / ppp, c.y as f32 / ppp + self.notch(c, ppp)), pos2(c.right() as f32 / ppp, c.bottom() as f32 / ppp));
            if m != mon {
                self.bar_mon = Some(m);
                mon = m;
                if let Some(from) = self.bar_pos {
                    self.bar_trip = Some((from, ctx.input(|i| i.time)));
                }
            }
        }
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
        let mut next = if (next - target).length() < 0.5 || self.cfg.instant_mode { target } else { next };
        // to another display: a trip of its own, eased in and out, so the
        // way across is seen (the glide above covers it in a blink)
        if let Some((from, at)) = self.bar_trip {
            let u = ((ctx.input(|i| i.time) - at) / 0.45) as f32;
            if u >= 1.0 {
                self.bar_trip = None;
                next = target;
            } else {
                let e = if u < 0.5 { 4.0 * u * u * u } else { 1.0 - (-2.0 * u + 2.0).powi(3) / 2.0 };
                next = from + (target - from) * e;
            }
        }
        if next != target {
            ctx.request_repaint();
        }
        self.bar_pos = Some(next);
        let pos = next - vec2(0.0, (1.0 - t) * 14.0);
        let (frame, glassy) = panel_frame("tools");
        self.paint_glass(ctx, Rect::from_min_size(pos, size), t, ppp, "tools-glass", frame.corner_radius.nw, glassy);
        let resp = egui::Area::new(Id::unique("tools-bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                let _part = theme::part("tools");
                frame
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

    /// A pane of Liquid Glass at `rect` (points): the screenshot under it,
    /// softly blurred and a little magnified, a light veil, a sheen over its
    /// top, a bright rim and a shadow it floats on.
    /// `on`: glass here (always on Liquid Glass; elsewhere if the theme asks).
    #[allow(clippy::too_many_arguments)]
    fn paint_glass(&mut self, ctx: &egui::Context, rect: Rect, alpha: f32, ppp: f32, id: &str, radius: u8, on: bool) {
        if !on || alpha <= 0.0 {
            return;
        }
        if self.glass.is_none() {
            // a quarter of the size, blurred a little: what's under the pane
            // stays recognizable, as through real glass
            let img = &self.snap.image;
            let (w, h) = ((img.width() / 4).max(1), (img.height() / 4).max(1));
            let small = image::imageops::resize(img, w, h, image::imageops::FilterType::Triangle);
            let blurred = image::imageops::blur(&small, 2.5);
            self.glass = Some(ctx.load_texture("glass", color_image(&blurred), TextureOptions::LINEAR));
            self.glass_img = Some(blurred);
        }
        let Some(tex) = &self.glass else { return };
        let img = vec2(self.snap.image.width() as f32, self.snap.image.height() as f32);
        // a lens: the pane shows a slightly smaller patch, magnified
        let lens = rect.shrink2(rect.size() * 0.04);
        let uv = Rect::from_min_max(
            pos2(lens.min.x * ppp / img.x, lens.min.y * ppp / img.y),
            pos2(lens.max.x * ppp / img.x, lens.max.y * ppp / img.y),
        );
        let a = |v: f32| (v * alpha) as u8;
        // how light it is under the pane (0..1): over a white page the glass
        // darkens, as Apple's does, or the light icons on it would vanish
        let light = self.glass_img.as_ref().map_or(0.0, |small| {
            let k = small.width() as f32 / img.x;
            let (x0, y0) = ((rect.min.x * ppp * k).max(0.0) as u32, (rect.min.y * ppp * k).max(0.0) as u32);
            let (x1, y1) = (((rect.max.x * ppp * k) as u32).min(small.width()), ((rect.max.y * ppp * k) as u32).min(small.height()));
            let (mut sum, mut n) = (0.0f32, 0u32);
            for y in (y0..y1).step_by(2) {
                for x in (x0..x1).step_by(2) {
                    let px = small.get_pixel(x, y);
                    sum += 0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32;
                    n += 1;
                }
            }
            if n == 0 { 0.0 } else { sum / n as f32 / 255.0 }
        });
        let bright = ((light - 0.42) / 0.4).clamp(0.0, 1.0);
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, Id::unique(id)));
        let r = egui::CornerRadius::same(radius);
        p.add(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(a(80.0 + 50.0 * bright)) }.as_shape(rect, r));
        p.add(egui::epaint::RectShape::filled(rect, r, Color32::WHITE.gamma_multiply(alpha)).with_texture(tex.id(), uv));
        // dark enough for the icons, light enough to see through; denser
        // over a light backdrop
        p.rect_filled(rect, r, Color32::from_rgba_unmultiplied(18, 18, 17, a(78.0 + 130.0 * bright)));
        p.rect_filled(rect, r, Color32::from_white_alpha(a(10.0 * (1.0 - bright))));
        // on light: a thin dark edge outside, where the light rim can't show
        if bright > 0.0 {
            p.rect_stroke(rect, r, Stroke::new(1.0, Color32::from_black_alpha(a(70.0 * bright))), StrokeKind::Outside);
        }
        // the sheen: light pooling in the top half
        let inset = radius as f32 * 0.6;
        let sheen = Rect::from_min_max(rect.min + vec2(inset, 1.0), pos2(rect.right() - inset, rect.top() + rect.height() * 0.55));
        let mut mesh = egui::Mesh::default();
        let (top, clear) = (Color32::from_white_alpha(a(20.0)), Color32::TRANSPARENT);
        mesh.colored_vertex(sheen.left_top(), top);
        mesh.colored_vertex(sheen.right_top(), top);
        mesh.colored_vertex(sheen.right_bottom(), clear);
        mesh.colored_vertex(sheen.left_bottom(), clear);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        p.add(mesh);
        // the rim, brightest where the light hits the top edge
        p.rect_stroke(rect, r, Stroke::new(1.0, Color32::from_white_alpha(a(58.0))), StrokeKind::Inside);
        let edge = radius as f32 * 0.8;
        p.rect_filled(Rect::from_min_size(rect.min + vec2(edge, 0.5), vec2(rect.width() - 2.0 * edge, 1.0)), 0, Color32::from_white_alpha(a(120.0)));
        p.rect_filled(Rect::from_min_size(rect.left_bottom() + vec2(edge, -1.5), vec2(rect.width() - 2.0 * edge, 1.0)), 0, Color32::from_white_alpha(a(22.0)));
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
        ui.painter().circle_filled(pos2(r.left() + 14.0, r.center().y), 4.5, DANGER());
        // the whole display this bar is on, in one click (or Space)
        if ui::icon_button_sized(ui, Icon::Screen, crate::i18n::l("весь экран  пробел", "the whole screen  space"), bar_btn(), bar_icon(), false, ui::Kind::Ghost).clicked()
            && let Some(m) = self.bar_mon
        {
            let ppp = ui.ctx().pixels_per_point();
            let c = m.center();
            self.sel = Some(irect_to_rect(self.snap.monitor_at((c.x * ppp) as i32, (c.y * ppp) as i32).rect));
            self.clicked_window = None;
            self.phase = Phase::Adjusting;
        }
        bar_sep(ui);
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
            .position(|w| *w >= self.tool_width(self.tool) - 0.01)
            .unwrap_or(WIDTH_STEPS.len() - 1) as i32;
        let next = (cur + if wheel > 0.0 { 1 } else { -1 }).clamp(0, WIDTH_STEPS.len() as i32 - 1) as usize;
        let v = WIDTH_STEPS[next];
        let key = tool_key(self.tool).to_string();
        self.cfg.tool_widths.insert(key.clone(), v);
        self.shared.update_config(|c| {
            c.tool_widths.insert(key, v);
        });
        // the text being typed takes the new size at once
        self.size_shown = Some(now);
    }

    /// What the tool will draw at its new size, at the pointer `pt`: a dot
    /// as thick as the stroke, a marker band, a step badge, or "Aa" at the
    /// text's size; its number under it.
    fn paint_size_preview(&self, ctx: &egui::Context, pt: Pos2, ppp: f32, alpha: f32) {
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::unique("ov-size")));
        let w = self.shape_width(ppp) / ppp;
        let c = self.color.gamma_multiply(alpha);
        let rim = Stroke::new(1.0, Color32::from_black_alpha((160.0 * alpha) as u8));
        let at = pt + vec2(0.0, -34.0);
        let label = match self.tool {
            Tool::Text | Tool::TextBox => {
                let size = doc::text_size(w);
                let g = p.layout_no_wrap("Aa".into(), font_semibold(size), if self.tool == Tool::TextBox { contrast_on(self.color) } else { c });
                let r = Rect::from_center_size(at - vec2(0.0, size * 0.3), g.size() + doc::text_pad(size) * 2.0);
                if self.tool == Tool::TextBox {
                    p.rect_filled(r, size * 0.22, c);
                } else {
                    p.rect_filled(r.expand(4.0), 8, Color32::from_rgba_unmultiplied(18, 18, 17, (150.0 * alpha) as u8));
                }
                p.galley(r.center() - g.size() / 2.0, g, c);
                format!("{:.0}", size)
            }
            Tool::Steps => {
                let r = doc::counter_radius(w);
                p.circle(at, r, c, rim);
                p.text(at, egui::Align2::CENTER_CENTER, "1", font_semibold(r * 1.15), contrast_on(self.color).gamma_multiply(alpha));
                format!("{w:.0}")
            }
            Tool::Highlight => {
                let band = w * 3.6 + 6.0;
                let r = Rect::from_center_size(at, vec2(band * 2.6, band));
                p.rect_filled(r, band * 0.3, self.color.gamma_multiply(0.45 * alpha));
                format!("{w:.0}")
            }
            _ => {
                // a ring around the dot, so it shows on any background
                p.circle(at, (w / 2.0).max(1.0) + 3.0, Color32::from_rgba_unmultiplied(18, 18, 17, (150.0 * alpha) as u8), Stroke::NONE);
                p.circle(at, (w / 2.0).max(1.0), c, rim);
                format!("{w:.0}")
            }
        };
        let g = p.layout_no_wrap(label, font_medium(11.5), TEXT().gamma_multiply(alpha));
        let tag = Rect::from_center_size(pt + vec2(0.0, 16.0), g.size() + vec2(12.0, 6.0));
        p.rect_filled(tag, tag.height() / 2.0, Color32::from_rgba_unmultiplied(18, 18, 17, (215.0 * alpha) as u8));
        p.galley(tag.center() - g.size() / 2.0, g, TEXT().gamma_multiply(alpha));
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
    ui.painter().hline(rect.shrink2(vec2(8.0, 0.0)).x_range(), rect.center().y, Stroke::new(1.0, LINE()));
}

/// Color chip for the vertical panel (two per row).
fn color_chip_small(ui: &mut egui::Ui, c: eframe::egui::Color32, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(bar_btn() / 2.0, 24.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 3, HOVER());
    }
    let chip = Rect::from_center_size(rect.center(), Vec2::splat(12.0));
    if selected {
        p.rect_stroke(chip.expand(2.5), 2, Stroke::new(1.0, TEXT()), StrokeKind::Inside);
    }
    p.rect_filled(chip, 2, c);
    if c.r() < 40 && c.g() < 40 && c.b() < 40 {
        p.rect_stroke(chip, 2, Stroke::new(1.0, LINE()), StrokeKind::Inside);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn bar_sep(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, bar_btn()), egui::Sense::hover());
    ui.painter().vline(rect.center().x, rect.shrink2(vec2(0.0, 11.0)).y_range(), Stroke::new(1.0, LINE()));
}

/// The current color: a round chip in a button.
fn swatch(ui: &mut egui::Ui, c: eframe::egui::Color32, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, (size * 0.12).round() as u8, HOVER());
    }
    let r = size * 0.21;
    p.circle_filled(rect.center(), r, c);
    p.circle_stroke(rect.center(), r + 2.5, Stroke::new(1.2, if resp.hovered() { TEXT() } else { DIM() }));
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn color_chip(ui: &mut egui::Ui, c: eframe::egui::Color32, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, bar_btn()), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect.shrink2(vec2(0.0, 7.0)), 4, HOVER());
    }
    let chip = Rect::from_center_size(rect.center(), Vec2::splat(16.0));
    if selected {
        p.rect_stroke(chip.expand(3.0), 2, Stroke::new(1.0, TEXT()), StrokeKind::Inside);
    }
    p.rect_filled(chip, 2, c);
    if c.r() < 40 && c.g() < 40 && c.b() < 40 {
        p.rect_stroke(chip, 2, Stroke::new(1.0, LINE()), StrokeKind::Inside);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

#[cfg(test)]
mod qr_tests {
    /// A QR drawn on a big gray "screen" is found where it is, with its text.
    #[test]
    fn finds_a_code() {
        let code = qrcode::QrCode::new(b"https://blinker.cool/update").unwrap();
        let w = code.width();
        let mut img = image::RgbaImage::from_pixel(1920, 1080, image::Rgba([60, 60, 60, 255]));
        let (x0, y0, cell) = (900u32, 400u32, 6u32);
        for y in 0..w + 8 {
            for x in 0..w + 8 {
                let dark = x >= 4 && y >= 4 && x < w + 4 && y < w + 4 && code[(x - 4, y - 4)] == qrcode::Color::Dark;
                let c = if dark { 0 } else { 255 };
                for dy in 0..cell {
                    for dx in 0..cell {
                        img.put_pixel(x0 + x as u32 * cell + dx, y0 + y as u32 * cell + dy, image::Rgba([c, c, c, 255]));
                    }
                }
            }
        }
        let found = super::find_qr(&img);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "https://blinker.cool/update");
        assert!((found[0].rect.min.x - (x0 + 4 * cell) as f32).abs() < 12.0, "{:?}", found[0].rect);
        assert_eq!(super::qr_link("www.example.com").as_deref(), Some("https://www.example.com"));
        assert!(super::qr_link("WIFI:S:home;T:WPA;P:secret;;").is_none());
    }

    /// Wallets by their shape and by their links; other text isn't one.
    #[test]
    fn wallets() {
        use super::{Chain, wallet_of};
        let w = |t: &str| wallet_of(t).map(|w| (w.chain, w.address, w.amount, w.chain_id));
        assert_eq!(w("0x52908400098527886E0F7030069857D2E4169EE7").unwrap().0, Chain::Evm);
        assert_eq!(w("ethereum:0x52908400098527886E0F7030069857D2E4169EE7@56").unwrap().3, Some(56));
        assert_eq!(w("bitcoin:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq?amount=0.01").unwrap(), (Chain::Btc, "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq".into(), Some("0.01".into()), None));
        assert_eq!(w("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2").unwrap().0, Chain::Btc);
        assert_eq!(w("TJCnKsPa7y5okkXvQAidZBzqx3QyQ6sxMW").unwrap().0, Chain::Tron);
        assert_eq!(w("tron:TJCnKsPa7y5okkXvQAidZBzqx3QyQ6sxMW").unwrap().0, Chain::Tron);
        assert_eq!(w("EQDtFpEwcFAEcRe5mLVh2N6C0x-_hJEM7W61_JLnSF74p4q2").unwrap().0, Chain::Ton);
        assert_eq!(w("ton://transfer/EQDtFpEwcFAEcRe5mLVh2N6C0x-_hJEM7W61_JLnSF74p4q2?amount=1000").unwrap().0, Chain::Ton);
        assert_eq!(w("7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU").unwrap().0, Chain::Solana);
        assert_eq!(w("LdP8Qox1VAhCzLJNqrr74YovaWYyNBUWvL").unwrap().0, Chain::Ltc);
        assert_eq!(w("DH5yaieqoZN36fDVciNyRueRGvGLR3mr7L").unwrap().0, Chain::Doge);
        assert!(w("https://blinker.cool/update").is_none());
        assert!(w("hello world").is_none());
        assert!(w("POGODITE: QR RESTORES MISSING DATA").is_none());
        assert!(w("WIFI:S:home;T:WPA;P:secret;;").is_none());
    }

    /// `BLINKER_TEST_QR` = a picture with QR codes: what they say.
    #[test]
    fn find() {
        let Ok(path) = std::env::var("BLINKER_TEST_QR") else { return };
        let img = image::open(path).unwrap().to_rgba8();
        let t = std::time::Instant::now();
        for q in super::find_qr(&img) {
            eprintln!("{:?} {}", q.rect, q.text);
        }
        eprintln!("in {:?}", t.elapsed());
    }
}


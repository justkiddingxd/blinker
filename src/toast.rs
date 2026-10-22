//! Post-capture card: a floating preview in a corner of the screen.
//! Hover reveals actions, the preview can be dragged out as a file.

use crate::config::{Config, Corner};
use crate::icons::{self, Icon};
use crate::runtime::{self, Shared};
use crate::theme::{self, *};
use crate::ui;
use crate::win::{self, IRect, Monitor};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, Rect, Sense, Stroke, StrokeKind, TextureHandle, Vec2, pos2, vec2,
};
use image::RgbaImage;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// What the card shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
    /// A GIF recording (still converting while `saved` is false).
    Gif,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastResult {
    Closed,
    Edit,
}

/// Gap between the card and the screen edge.
const GAP: f32 = 16.0;
const PAD: f32 = 8.0;
const CAPTION_H: f32 = 50.0;
const RADIUS: u8 = 14;
/// Room above the card for a badge ("Clipped!").
const BADGE_H: f32 = 40.0;
/// How long a click's answer (a tick, a chip over the picture) stays.
const FLASH: f64 = 1.3;

struct Layout {
    card: Vec2,
    image: Vec2,
}

fn layout(cfg: &Config, img: &RgbaImage) -> Layout {
    let w = cfg.toast_size.width();
    let iw = w - PAD * 2.0;
    let aspect = img.height() as f32 / img.width().max(1) as f32;
    // the whole picture is shown: a very wide or tall one gets bars around it
    let ih = (iw * aspect).clamp(iw * 0.3, iw).round();
    let h = PAD + ih + if cfg.toast_caption { CAPTION_H } else { PAD };
    Layout { card: vec2(w, h), image: vec2(iw, ih) }
}

pub fn run(
    shared: &Arc<Shared>,
    image: Arc<RgbaImage>,
    title: String,
    path: Option<PathBuf>,
    saved: Option<Arc<AtomicBool>>,
    monitor: Monitor,
    kind: Kind,
    upload: Option<Arc<crate::upload::Job>>,
    badge: Option<&'static str>,
) -> ToastResult {
    let video = kind != Kind::Image;
    let cfg = shared.config();
    let result = Rc::new(Cell::new(ToastResult::Closed));
    let result2 = result.clone();
    let lay = layout(&cfg, &image);
    // Downscale on the CPU so the GPU texture stays small.
    let tw = ((lay.image.x * 2.0) as u32).min(image.width()).max(1);
    let th = ((image.height() as f32 * tw as f32 / image.width() as f32).round() as u32).max(1);
    let thumb = image::imageops::thumbnail(&*image, tw, th);
    // Small PNG used as the cursor image while dragging the file out.
    let drag_thumb = image::imageops::thumbnail(&thumb, 160.min(tw), (160.min(tw) * th / tw).max(1));
    let mut drag_png = Vec::new();
    let _ = image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut drag_png),
        drag_thumb.as_raw(),
        drag_thumb.width(),
        drag_thumb.height(),
        image::ExtendedColorType::Rgba8,
    );
    // The window is exactly the card (and its badge): no invisible margins
    // that catch the mouse.
    let top = if badge.is_some() { BADGE_H } else { 0.0 };
    let window = lay.card + vec2(0.0, top);
    let viewport = egui::ViewportBuilder::default()
        .with_title("blinker")
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_taskbar(false)
        .with_always_on_top()
        .with_active(false)
        .with_inner_size(window);
    let shared2 = shared.clone();
    runtime::run_window(shared, viewport, false, move |cc| {
        let hwnd = win::hwnd_of(cc);
        if let Some(h) = hwnd {
            win::disable_transitions(h);
            win::no_dwm_border(h);
            if badge.is_some() {
                // the badge's room is clear: a window shadow would frame it
                win::no_shadow(h);
            }
            win::cloak(h, true);
            // macOS 26: the card is a pane of glass over the desktop
            #[cfg(target_os = "macos")]
            if theme::glass() {
                win::window_glass(h, (0.0, top, window.x, window.y - top), RADIUS as f32, 0);
            }
        }
        let tex = cc.egui_ctx.load_texture("toast-thumb", theme::color_image(&thumb), egui::TextureOptions::LINEAR);
        let mut app = ToastApp {
            thumb_size: vec2(thumb.width() as f32, thumb.height() as f32),
            image,
            drag_png,
            shared: shared2,
            cfg,
            lay,
            window,
            tex,
            title,
            path,
            saved,
            video,
            gif: kind == Kind::Gif,
            upload,
            note_shown: false,
            hwnd,
            monitor,
            t0: -1.0,
            remaining: 0.0,
            last_t: 0.0,
            closing: None,
            result: result2,
            frame_no: 0,
            size_text: String::new(),
            copied_at: None,
            badge,
            top,
            flash: None,
            pulse: None,
        };
        app.place(app.slide_dir() * 22.0);
        app
    });
    result.get()
}

struct ToastApp {
    image: Arc<RgbaImage>,
    drag_png: Vec<u8>,
    shared: Arc<Shared>,
    cfg: Config,
    lay: Layout,
    window: Vec2,
    tex: TextureHandle,
    thumb_size: Vec2,
    title: String,
    path: Option<PathBuf>,
    saved: Option<Arc<AtomicBool>>,
    /// A screen recording: `path` is the video, `image` its last frame.
    video: bool,
    gif: bool,
    upload: Option<Arc<crate::upload::Job>>,
    /// The note of a first upload to post.blinker.cool is on the card.
    note_shown: bool,
    hwnd: Option<win::HWND>,
    monitor: Monitor,
    t0: f64,
    /// Seconds left before auto-hide (paused while hovered or saving).
    remaining: f32,
    last_t: f64,
    closing: Option<f64>,
    result: Rc<Cell<ToastResult>>,
    frame_no: u32,
    size_text: String,
    copied_at: Option<f64>,
    badge: Option<&'static str>,
    /// Height of the badge's room above the card.
    top: f32,
    /// The answer to the last click: a chip over the picture, and since when.
    flash: Option<(String, f64)>,
    /// The button clicked last (a ring spreads from it), and since when.
    pulse: Option<(Icon, Rect, f64)>,
}

impl ToastApp {
    fn target(&self, scale: f32) -> IRect {
        let w = (self.window.x * scale).round() as i32;
        let h = (self.window.y * scale).round() as i32;
        let gap = (GAP * scale).round() as i32;
        let work = self.monitor.work;
        let (left, top) = match self.cfg.toast_corner {
            Corner::TopLeft => (true, true),
            Corner::TopRight => (false, true),
            Corner::BottomLeft => (true, false),
            Corner::BottomRight => (false, false),
        };
        let x = if left { work.x + gap } else { work.right() - w - gap };
        let y = if top { work.y + gap } else { work.bottom() - h - gap };
        IRect::new(x, y, w, h)
    }

    /// Corner of the work area, shifted by `offset` points (the slide animation
    /// moves the window itself).
    fn place(&mut self, offset: Vec2) {
        if let Some(h) = self.hwnd {
            let scale = win::window_scale(h);
            let t = self.target(scale);
            let t = t.offset((offset.x * scale).round() as i32, (offset.y * scale).round() as i32);
            if win::window_rect(h) != t {
                win::set_window_rect(h, t, true);
            }
        }
    }

    fn close(&mut self, ctx: &egui::Context, result: ToastResult) {
        if self.closing.is_none() {
            self.closing = Some(ctx.input(|i| i.time));
            self.result.set(result);
            ctx.request_repaint();
        }
    }

    fn close_now(&mut self, ctx: &egui::Context, result: ToastResult) {
        self.result.set(result);
        self.closing = Some(f64::NEG_INFINITY);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn is_saving(&self) -> bool {
        self.saved.as_ref().is_some_and(|f| !f.load(Ordering::Acquire))
    }

    fn is_uploading(&self) -> bool {
        self.upload.as_ref().is_some_and(|j| j.result().is_none())
    }

    /// A file on disk for drag-and-drop: the saved capture, or a temp copy.
    fn file_for_drag(&self) -> Option<PathBuf> {
        if let Some(flag) = &self.saved {
            let start = std::time::Instant::now();
            while !flag.load(Ordering::Acquire) && start.elapsed() < std::time::Duration::from_secs(3) {
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
        }
        if let Some(p) = &self.path
            && p.exists()
        {
            return Some(p.clone());
        }
        let dir = crate::config::cache_dir().join("drag");
        let _ = std::fs::create_dir_all(&dir);
        let name = chrono::Local::now().format("blinker %Y-%m-%d %H-%M-%S.png").to_string();
        let path = dir.join(name);
        crate::output::write(&self.image, &path, crate::config::ImageFormat::Png, 90).ok()?;
        Some(path)
    }

    /// A pill over the card that pops in: what just happened ("Clipped!").
    fn paint_badge(&self, painter: &egui::Painter, card: Rect, text: &str, now: f64, alpha: f32) {
        let t = ((now - self.t0 - 0.08) / 0.45) as f32;
        if t <= 0.0 {
            return;
        }
        let pop = ease_out_back(t);
        let a = alpha * t.clamp(0.0, 1.0);
        let galley = painter.layout_no_wrap(text.to_string(), font_semibold(14.0), ON_ACCENT);
        let size = vec2(galley.size().x + 44.0, 30.0) * (0.7 + 0.3 * pop);
        let center = pos2(card.center().x, card.top() - BADGE_H / 2.0 + (1.0 - pop.min(1.0)) * 8.0);
        let pill = Rect::from_center_size(center, size);
        painter.rect_filled(pill.translate(vec2(0.0, 2.0)), 15, Color32::from_black_alpha((60.0 * a) as u8));
        painter.rect_filled(pill, 15, ACCENT.gamma_multiply(a));
        // the logo's sparkle, then the words
        let s = 6.0 * (0.7 + 0.3 * pop);
        let c = pos2(pill.left() + 18.0, pill.center().y);
        let w = s * 0.18;
        let star = vec![
            c + vec2(0.0, -s),
            c + vec2(w, -w),
            c + vec2(s, 0.0),
            c + vec2(w, w),
            c + vec2(0.0, s),
            c + vec2(-w, w),
            c + vec2(-s, 0.0),
            c + vec2(-w, -w),
        ];
        // a 4-point star is concave: two convex diamonds make it
        painter.add(egui::Shape::convex_polygon(vec![star[0], star[1], star[4], star[5]], ON_ACCENT.gamma_multiply(a), Stroke::NONE));
        painter.add(egui::Shape::convex_polygon(vec![star[2], star[3], star[6], star[7]], ON_ACCENT.gamma_multiply(a), Stroke::NONE));
        painter.add(egui::Shape::convex_polygon(vec![star[0], star[2], star[4], star[6]].iter().map(|p| c + (*p - c) * 0.32).collect(), ON_ACCENT.gamma_multiply(a), Stroke::NONE));
        let text_pos = pos2(c.x + s + 8.0, pill.center().y - galley.size().y / 2.0);
        painter.galley(text_pos, galley, ON_ACCENT.gamma_multiply(a));
    }

    /// The answer to a click: a ring spreading from the button and a chip
    /// over the picture saying what happened.
    fn paint_feedback(&mut self, painter: &egui::Painter, img: Rect, now: f64, alpha: f32) {
        if let Some((_, r, at)) = self.pulse {
            let t = ((now - at) / 0.45) as f32;
            if t >= 1.0 {
                self.pulse = None;
            } else {
                let e = ui::ease_out_cubic(t);
                let radius = 16.0 + 14.0 * e;
                painter.circle_stroke(r.center(), radius, Stroke::new(2.0 * (1.0 - e), ACCENT.gamma_multiply(alpha * (1.0 - e))));
            }
        }
        if let Some((text, at)) = &self.flash {
            let t = (now - at) as f32;
            if t as f64 >= FLASH {
                self.flash = None;
                return;
            }
            let a = alpha * (t / 0.12).min(1.0) * ((FLASH as f32 - t) / 0.25).min(1.0);
            let g = painter.layout_no_wrap(text.clone(), font_medium(12.5), TEXT);
            let rise = (1.0 - ui::ease_out_cubic((t / 0.25).min(1.0))) * 6.0;
            let chip = Rect::from_center_size(pos2(img.center().x, img.top() + 22.0 + rise), g.size() + vec2(22.0, 12.0));
            painter.rect_filled(chip, 12, Color32::from_rgba_unmultiplied(18, 18, 17, (235.0 * a) as u8));
            painter.rect_stroke(chip, 12, Stroke::new(1.0, Color32::from_white_alpha((24.0 * a) as u8)), StrokeKind::Inside);
            painter.galley(chip.center() - g.size() / 2.0, g, TEXT.gamma_multiply(a));
        }
    }

    /// Direction the card slides in from (towards its screen edge).
    fn slide_dir(&self) -> Vec2 {
        match self.cfg.toast_corner {
            Corner::TopLeft | Corner::BottomLeft => vec2(-1.0, 0.0),
            Corner::TopRight | Corner::BottomRight => vec2(1.0, 0.0),
        }
    }
}

/// Round action button drawn over the preview: lit on hover, pressed in while
/// held, a tick in place of the icon while `done`.
fn overlay_button(ui: &mut egui::Ui, rect: Rect, icon: Icon, tip: &str, alpha: f32, done: bool) -> egui::Response {
    let resp = ui.interact(rect, ui.scope_id().with(tip), Sense::click());
    let (h, down) = (resp.hovered(), resp.is_pointer_button_down_on());
    let p = ui.painter();
    let r = if down { rect.shrink(1.5) } else { rect };
    let fill = if down {
        ACCENT.gamma_multiply(0.75)
    } else if h || done {
        ACCENT
    } else {
        Color32::TRANSPARENT
    };
    p.rect_filled(r, 8, fill.gamma_multiply(alpha));
    let fg = if h || down || done { ON_ACCENT } else { TEXT };
    if done {
        paint_tick(p, r, fg.gamma_multiply(alpha));
    } else {
        icons::paint(p, r, icon, if down { 13.5 } else { 15.0 }, fg.gamma_multiply(alpha));
    }
    resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip)
}

/// A check mark in the middle of `r`.
fn paint_tick(p: &egui::Painter, r: Rect, c: Color32) {
    let c0 = r.center();
    let pts = vec![c0 + vec2(-5.0, 0.5), c0 + vec2(-1.5, 4.0), c0 + vec2(5.5, -3.5)];
    p.add(egui::Shape::line(pts, Stroke::new(2.0, c)));
}

/// `0..1` overshooting a little past 1 on the way (a pop).
fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let (c1, c3) = (1.70158, 2.70158);
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

impl eframe::App for ToastApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.frame_no += 1;
        if let Some(h) = self.hwnd {
            // Revealed once real frames exist (no blank window flash).
            if self.frame_no == 3 {
                win::cloak(h, false);
            } else if self.frame_no < 3 {
                ctx.request_repaint();
            }
        }
        if self.t0 < 0.0 {
            self.t0 = now;
            self.last_t = now;
            self.remaining = self.cfg.toast_seconds.max(1.0);
        }
        // Any new command (another capture, open window...) dismisses the card.
        if self.shared.peek().is_some() {
            self.close_now(&ctx, ToastResult::Closed);
            return;
        }
        let fade_out = self.closing.map(|c| ((now - c) / 0.18).clamp(0.0, 1.0) as f32).unwrap_or(0.0);
        if fade_out >= 1.0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let saving = self.is_saving();
        if !saving && self.size_text.is_empty() {
            if let Some(p) = &self.path
                && let Ok(meta) = std::fs::metadata(p)
            {
                let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                let kb = meta.len() as f64 / 1024.0;
                self.size_text = if kb < 1000.0 {
                    lf!("{ext} · {kb:.0} кб", "{ext} · {kb:.0} kb")
                } else {
                    lf!("{ext} · {:.1} мб", "{ext} · {:.1} mb", kb / 1024.0).replace('.', ",")
                };
            } else if self.path.is_none() {
                self.size_text = crate::i18n::t("только в буфере").into();
            }
        }

        // geometry with slide-in / slide-out
        let appear = ui::ease_out_cubic(((now - self.t0) / 0.3) as f32);
        let alpha = appear * (1.0 - fade_out);
        let shift = self.slide_dir() * ((1.0 - appear) * 22.0 + fade_out * 12.0);
        self.place(shift);
        let mut card = ctx.content_rect();
        card.min.y += self.top;
        let img_rect = Rect::from_min_size(card.min + Vec2::splat(PAD), self.lay.image);

        let hovered = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| card.contains(p));
        let dt = (now - self.last_t).clamp(0.0, 0.1) as f32;
        self.last_t = now;
        if !hovered && !saving && !self.is_uploading() && self.closing.is_none() {
            self.remaining -= dt;
        }
        if self.remaining <= 0.0 {
            self.close(&ctx, ToastResult::Closed);
        }
        let hover_t = ctx.animate_bool_with_time(egui::Id::unique("toast-hover"), hovered, 0.14);

        let painter = ui.painter().clone();
        // on glass only a dark tint, so the text stays readable
        let (fill, edge) = if theme::glass() { (PANEL.gamma_multiply(0.45 * alpha), Color32::TRANSPARENT) } else { (PANEL.gamma_multiply(alpha), LINE.gamma_multiply(alpha)) };
        painter.rect_filled(card, RADIUS, fill);
        painter.rect_stroke(card, RADIUS, Stroke::new(1.0, edge), StrokeKind::Inside);
        if let Some(text) = self.badge {
            self.paint_badge(&painter, card, text, now, alpha);
        }

        // preview (drag = file, click = edit)
        let img_resp = ui.interact(img_rect, ui.scope_id().with("img"), Sense::click_and_drag());
        let img_radius = CornerRadius::same(RADIUS - PAD as u8 + 2);
        // the whole capture, fitted in (not a cropped part of it)
        let k = (img_rect.width() / self.thumb_size.x).min(img_rect.height() / self.thumb_size.y);
        let fit = Rect::from_center_size(img_rect.center(), self.thumb_size * k);
        if fit.width() < img_rect.width() - 1.0 || fit.height() < img_rect.height() - 1.0 {
            painter.rect_filled(img_rect, img_radius, Color32::from_rgba_unmultiplied(12, 12, 11, (255.0 * alpha) as u8));
        }
        let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        painter.add(egui::epaint::RectShape::filled(fit, img_radius, Color32::WHITE.gamma_multiply(alpha)).with_texture(self.tex.id(), full));
        painter.rect_stroke(img_rect, img_radius, Stroke::new(1.0, Color32::from_white_alpha((14.0 * alpha) as u8)), StrokeKind::Inside);
        if hover_t > 0.0 {
            painter.rect_filled(img_rect, img_radius, Color32::from_black_alpha((120.0 * hover_t * alpha) as u8));
        }
        if self.video && hover_t < 1.0 {
            let a = (1.0 - hover_t) * alpha;
            ui::play_badge(&painter, img_rect.center(), 21.0, a);
        }
        if img_resp.drag_started() {
            if let Some(path) = self.file_for_drag() {
                let png = self.drag_png.clone();
                let _ = drag::start_drag(
                    frame,
                    drag::DragItem::Files(vec![path]),
                    drag::Image::Raw(png),
                    |_, _| {},
                    drag::Options::default(),
                );
            }
            self.close_now(&ctx, ToastResult::Closed);
            return;
        }
        let mut clicked_button = false;
        let mut over_button = false;

        // actions: a row at the bottom of the preview, shown on hover
        if hover_t > 0.01 {
            let a = hover_t * alpha;
            let size = 32.0;
            let gap = 4.0;
            let mut actions: Vec<(Icon, &str)> = if self.gif {
                vec![(Icon::Copy, crate::i18n::t("копировать файл"))]
            } else if self.video {
                vec![(Icon::Edit, crate::i18n::t("обрезать и править")), (Icon::Copy, crate::i18n::t("копировать файл"))]
            } else {
                vec![(Icon::Edit, crate::i18n::t("править")), (Icon::Copy, crate::i18n::t("копировать"))]
            };
            if self.path.is_some() {
                actions.push((Icon::Folder, crate::i18n::t("показать в папке")));
            }
            if !self.video {
                actions.push((Icon::Pin, crate::i18n::t("закрепить поверх окон")));
            }
            if crate::upload::configured(&self.cfg) && self.upload.is_none() {
                actions.push((Icon::Upload, crate::i18n::t("загрузить и скопировать ссылку")));
            }
            let total = actions.len() as f32 * size + (actions.len() as f32 - 1.0) * gap;
            // a dark plate holding the buttons
            let bar = Rect::from_center_size(
                pos2(img_rect.center().x, img_rect.bottom() - 10.0 - size / 2.0 + (1.0 - hover_t) * 6.0),
                vec2(total + 8.0, size + 8.0),
            );
            painter.rect_filled(bar, 11, Color32::from_rgba_unmultiplied(18, 18, 17, (235.0 * a) as u8));
            painter.rect_stroke(bar, 11, Stroke::new(1.0, Color32::from_white_alpha((20.0 * a) as u8)), StrokeKind::Inside);
            let mut x = bar.left() + 4.0;
            for (icon, tip) in actions {
                let r = Rect::from_min_size(pos2(x, bar.top() + 4.0), Vec2::splat(size));
                let done = icon == Icon::Copy && self.copied_at.is_some_and(|t| now - t < FLASH);
                let resp = overlay_button(ui, r, icon, tip, a, done);
                over_button |= resp.hovered();
                if resp.clicked() {
                    clicked_button = true;
                    self.pulse = Some((icon, r, now));
                    match icon {
                        Icon::Edit => self.close(&ctx, ToastResult::Edit),
                        Icon::Copy => {
                            let ok = match (&self.path, self.video) {
                                (Some(p), true) => crate::output::copy_file(p),
                                _ => crate::output::copy_image(&self.image),
                            };
                            if ok {
                                self.copied_at = Some(now);
                            }
                            let msg = if ok { crate::i18n::t("скопировано") } else { crate::i18n::l("не скопировалось", "couldn't copy") };
                            self.flash = Some((msg.to_string(), now));
                        }
                        Icon::Folder => {
                            if let Some(p) = self.path.clone() {
                                win::reveal_in_explorer(&p);
                                self.close(&ctx, ToastResult::Closed);
                            }
                        }
                        Icon::Upload => {
                            if let Some(p) = self.file_for_drag() {
                                self.upload = Some(crate::upload::start(&self.cfg, p, None));
                                self.remaining = self.remaining.max(3.0);
                                self.flash = Some((crate::i18n::l("загружаю…", "uploading…").to_string(), now));
                            }
                        }
                        Icon::Pin => {
                            let (cx, cy) = win::cursor_pos();
                            crate::pin::spawn(&self.image, IRect::new(cx - 60, cy - 60, 0, 0));
                            self.close(&ctx, ToastResult::Closed);
                        }
                        _ => {}
                    }
                }
                x += size + gap;
            }
            if !self.cfg.toast_caption {
                let close_r = Rect::from_min_size(img_rect.right_top() + vec2(-30.0, 8.0), Vec2::splat(22.0));
                let resp = overlay_button(ui, close_r, Icon::Close, crate::i18n::t("закрыть"), a, false);
                over_button |= resp.hovered();
                if resp.clicked() {
                    clicked_button = true;
                    self.close(&ctx, ToastResult::Closed);
                }
            }
        }
        // the picture opens the editor on a click: a hand, as on any link
        // (not the four-way "move" arrows); the buttons keep their own
        let img_resp = if over_button {
            img_resp
        } else {
            img_resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(crate::i18n::t("тяни как файл, клик открывает редактор"))
        };
        if img_resp.is_pointer_button_down_on() && !over_button {
            painter.rect_filled(img_rect, img_radius, Color32::from_black_alpha((50.0 * alpha) as u8));
        }
        if !clicked_button && !over_button && img_resp.clicked() {
            self.close(&ctx, ToastResult::Edit);
        }
        self.paint_feedback(&painter, img_rect, now, alpha);

        // caption: what it is and where it stands, a close button
        if self.cfg.toast_caption {
            let top = img_rect.bottom();
            let title_y = top + CAPTION_H / 2.0 - 8.0;
            let sub_y = top + CAPTION_H / 2.0 + 9.0;
            let left = card.left() + PAD + 4.0;
            let title = if self.copied_at.is_some_and(|t| now - t < 1.5) { crate::i18n::t("скопировано снова") } else { &self.title };
            painter.text(pos2(left, title_y), Align2::LEFT_CENTER, title, font_medium(13.0), TEXT.gamma_multiply(alpha));
            let upload = self.upload.as_ref().map(|j| (j.fraction(), j.result()));
            let sub = match upload.clone() {
                Some((f, None)) => lf!("загрузка {:.0}%", "uploading {:.0}%", f * 100.0),
                Some((_, Some(Ok(_)))) => crate::i18n::t("ссылка в буфере").to_string(),
                Some((_, Some(Err(_)))) => crate::i18n::t("не загрузилось").to_string(),
                None if saving => {
                    let dots = ".".repeat(((now * 3.0) as usize % 3) + 1);
                    lf!("сохраняется{dots}", "saving{dots}")
                }
                None => self.size_text.clone(),
            };
            // the first upload to post.blinker.cool: what it keeps, in place of the size
            let note = self.upload.as_ref().filter(|_| matches!(upload, Some((_, Some(Ok(_)))))).and_then(|j| j.note.clone());
            if note.is_some() && !self.note_shown {
                self.note_shown = true;
                self.remaining = self.remaining.max(7.0);
            }
            let failed = matches!(upload, Some((_, Some(Err(_)))));
            let sc = if failed { DANGER } else { FAINT_TEXT };
            let sr = match note {
                Some(n) => {
                    let w = card.right() - PAD - 34.0 - left;
                    let g = painter.layout(n, font(11.0), ACCENT.gamma_multiply(alpha), w);
                    let r = Rect::from_min_size(pos2(left, sub_y - 7.0), g.size());
                    painter.galley(r.min, g, ACCENT);
                    r
                }
                None => painter.text(pos2(left, sub_y), Align2::LEFT_CENTER, sub, mono(11.0), sc.gamma_multiply(alpha)),
            };
            if let Some((_, Some(Err(e)))) = upload.as_ref() {
                ui.interact(sr, ui.scope_id().with("upload-err"), Sense::hover()).on_hover_text(e);
            }
            // progress of an upload as a thin bar under the text
            let busy = match &upload {
                Some((f, None)) => Some(*f),
                _ => None,
            };
            if let Some(f) = busy {
                let track = Rect::from_min_max(pos2(left, sub_y + 10.0), pos2(card.right() - PAD - 40.0, sub_y + 12.0));
                painter.rect_filled(track, 1, LINE.gamma_multiply(alpha));
                painter.rect_filled(Rect::from_min_size(track.min, vec2(track.width() * f.clamp(0.0, 1.0), 2.0)), 1, ACCENT.gamma_multiply(alpha));
            }
            let close_r = Rect::from_center_size(pos2(card.right() - PAD - 14.0, top + CAPTION_H / 2.0), Vec2::splat(28.0));
            let cresp = ui.interact(close_r, ui.scope_id().with("caption-close"), Sense::click());
            if cresp.hovered() {
                painter.rect_filled(close_r, 8, HOVER.gamma_multiply(alpha));
            }
            icons::paint(&painter, close_r, Icon::Close, 12.0, if cresp.hovered() { TEXT } else { FAINT }.gamma_multiply(alpha));
            if cresp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(crate::i18n::t("закрыть")).clicked() {
                self.close(&ctx, ToastResult::Closed);
            }
        }

        // time left: a hairline along the bottom edge
        let life = (self.remaining / self.cfg.toast_seconds.max(1.0)).clamp(0.0, 1.0);
        let inset = RADIUS as f32;
        let y = card.bottom() - 1.5;
        let track = Rect::from_min_max(pos2(card.left() + inset, y - 0.75), pos2(card.right() - inset, y + 0.75));
        let fill = Rect::from_min_max(track.min, pos2(track.left() + track.width() * life, track.bottom()));
        painter.rect_filled(fill, 1, ACCENT.gamma_multiply(0.5 * alpha * (1.0 - hover_t * 0.7)));

        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

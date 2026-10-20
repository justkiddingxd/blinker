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
const PAD: f32 = 6.0;
const CAPTION_H: f32 = 32.0;

struct Layout {
    card: Vec2,
    image: Vec2,
}

fn layout(cfg: &Config, img: &RgbaImage) -> Layout {
    let w = cfg.toast_size.width();
    let iw = w - PAD * 2.0;
    let aspect = img.height() as f32 / img.width().max(1) as f32;
    let ih = (iw * aspect).clamp(iw * 0.38, iw * 0.75).round();
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
    reencode: Option<Arc<crate::reencode::Job>>,
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
    // The window is exactly the card: no invisible margins that catch the mouse.
    let window = lay.card;
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
            win::cloak(h, true);
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
            reencode,
            tg_copied: false,
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
    /// "For Telegram" re-encode of a recording.
    reencode: Option<Arc<crate::reencode::Job>>,
    tg_copied: bool,
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
            || self.reencode.as_ref().is_some_and(|j| j.result().is_none())
    }

    /// The compressed file once the re-encode finished.
    fn compressed(&self) -> Option<PathBuf> {
        self.reencode.as_ref().and_then(|j| j.result()).and_then(|r| r.ok())
    }

    /// A file on disk for drag-and-drop: the saved capture, or a temp copy.
    fn file_for_drag(&self) -> Option<PathBuf> {
        if let Some(p) = self.compressed() {
            return Some(p);
        }
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

    /// Direction the card slides in from (towards its screen edge).
    fn slide_dir(&self) -> Vec2 {
        match self.cfg.toast_corner {
            Corner::TopLeft | Corner::BottomLeft => vec2(-1.0, 0.0),
            Corner::TopRight | Corner::BottomRight => vec2(1.0, 0.0),
        }
    }
}

/// Round action button drawn over the preview.
fn overlay_button(ui: &mut egui::Ui, rect: Rect, icon: Icon, tip: &str, alpha: f32) -> egui::Response {
    let resp = ui.interact(rect, ui.scope_id().with(tip), Sense::click());
    let h = resp.hovered();
    let p = ui.painter();
    let fill = if h { ACCENT } else { Color32::from_rgba_unmultiplied(18, 18, 17, 225) };
    p.rect_filled(rect, 6, fill.gamma_multiply(alpha));
    let fg = if h { ON_ACCENT } else { TEXT };
    icons::paint(p, rect, icon, 15.0, fg.gamma_multiply(alpha));
    resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip)
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
        let card = ctx.content_rect();
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
        painter.rect_filled(card, 7, PANEL.gamma_multiply(alpha));
        painter.rect_stroke(card, 7, Stroke::new(1.0, LINE.gamma_multiply(alpha)), StrokeKind::Inside);

        // preview (drag = file, click = edit)
        let img_resp = ui.interact(img_rect, ui.scope_id().with("img"), Sense::click_and_drag());
        let uv = ui::cover_uv(img_rect.size(), self.thumb_size);
        painter.add(
            egui::epaint::RectShape::filled(img_rect, CornerRadius::same(4), Color32::WHITE.gamma_multiply(alpha))
                .with_texture(self.tex.id(), uv),
        );
        if hover_t > 0.0 {
            painter.rect_filled(img_rect, 4, Color32::from_black_alpha((105.0 * hover_t * alpha) as u8));
        }
        if self.video && hover_t < 1.0 {
            let a = (1.0 - hover_t) * alpha;
            let c = img_rect.center();
            painter.circle_filled(c, 17.0, Color32::from_rgba_unmultiplied(18, 18, 17, (210.0 * a) as u8));
            icons::paint(&painter, Rect::from_center_size(c + vec2(1.5, 0.0), Vec2::splat(18.0)), Icon::Play, 13.0, TEXT.gamma_multiply(a));
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

        // actions over the preview
        if hover_t > 0.01 {
            let a = hover_t * alpha;
            let size = 32.0;
            let gap = 6.0;
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
            if self.video && !self.gif && self.reencode.is_none() {
                actions.push((Icon::Send, crate::i18n::t("сжать для telegram и скопировать")));
            }
            if crate::upload::configured(&self.cfg) && self.upload.is_none() {
                actions.push((Icon::Upload, crate::i18n::t("загрузить и скопировать ссылку")));
            }
            let total = actions.len() as f32 * size + (actions.len() as f32 - 1.0) * gap;
            let mut x = img_rect.center().x - total / 2.0;
            let y = img_rect.center().y - size / 2.0 + (1.0 - hover_t) * 6.0;
            for (icon, tip) in actions {
                let r = Rect::from_min_size(pos2(x, y), Vec2::splat(size));
                if overlay_button(ui, r, icon, tip, a).clicked() {
                    clicked_button = true;
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
                        }
                        Icon::Folder => {
                            if let Some(p) = self.path.clone() {
                                win::reveal_in_explorer(&p);
                                self.close(&ctx, ToastResult::Closed);
                            }
                        }
                        Icon::Send => {
                            if let Some(p) = self.path.clone() {
                                let c = ctx.clone();
                                self.reencode = Some(crate::reencode::start(p, self.cfg.tg_mode, None, move || c.request_repaint()));
                                self.remaining = self.remaining.max(3.0);
                            }
                        }
                        Icon::Upload => {
                            if let Some(p) = self.file_for_drag() {
                                self.upload = Some(crate::upload::start(&self.cfg, p, None));
                                self.remaining = self.remaining.max(3.0);
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
            let close_r = Rect::from_min_size(img_rect.right_top() + vec2(-28.0, 6.0), Vec2::splat(22.0));
            if overlay_button(ui, close_r, Icon::Close, crate::i18n::t("закрыть"), a).clicked() {
                clicked_button = true;
                self.close(&ctx, ToastResult::Closed);
            }
        }
        if !clicked_button
            && img_resp
                .on_hover_cursor(CursorIcon::Grab)
                .on_hover_text(crate::i18n::t("тяни как файл, клик открывает редактор"))
                .clicked()
        {
            self.close(&ctx, ToastResult::Edit);
        }

        // caption
        let bar_y = if self.cfg.toast_caption {
            let cy = img_rect.bottom() + CAPTION_H / 2.0;
            let title = if self.copied_at.is_some_and(|t| now - t < 1.5) { crate::i18n::t("скопировано снова") } else { &self.title };
            painter.text(pos2(card.left() + 10.0, cy), Align2::LEFT_CENTER, title, font_medium(12.5), TEXT.gamma_multiply(alpha));
            let upload = self.upload.as_ref().map(|j| (j.fraction(), j.result()));
            let tg = self.reencode.as_ref().map(|j| (j.fraction(), j.result()));
            if let Some((_, Some(Ok(p)))) = &tg
                && !self.tg_copied
            {
                // the compressed file replaces the recording on the clipboard
                self.tg_copied = true;
                crate::output::copy_file(p);
                self.remaining = self.remaining.max(4.0);
            }
            let right = match (&tg, upload.clone()) {
                (Some((f, None)), _) => lf!("сжатие {:.0}%", "compressing {:.0}%", f * 100.0),
                (Some((_, Some(Ok(p)))), None) => {
                    let mb = std::fs::metadata(p).map(|m| m.len() as f64 / 1048576.0).unwrap_or(0.0);
                    lf!("в буфере · {mb:.1} мб", "copied · {mb:.1} mb").replace('.', ",")
                }
                (Some((_, Some(Err(_)))), None) => crate::i18n::t("не сжалось").to_string(),
                (_, upload) => match upload {
                Some((f, None)) => lf!("загрузка {:.0}%", "uploading {:.0}%", f * 100.0),
                Some((_, Some(Ok(_)))) => crate::i18n::t("ссылка в буфере").to_string(),
                Some((_, Some(Err(_)))) => crate::i18n::t("не загрузилось").to_string(),
                None if saving => {
                    let dots = ".".repeat(((now * 3.0) as usize % 3) + 1);
                    lf!("сохраняется{dots}", "saving{dots}")
                }
                None => self.size_text.clone(),
                },
            };
            let failed = matches!(upload, Some((_, Some(Err(_))))) || matches!(tg, Some((_, Some(Err(_)))));
            let rc = if failed { DANGER } else { FAINT };
            let rr = painter.text(pos2(card.right() - 10.0, cy), Align2::RIGHT_CENTER, right, mono(11.0), rc.gamma_multiply(alpha));
            if let Some((_, Some(Err(e)))) = upload.as_ref().or(tg.as_ref().map(|(f, r)| (f, r)).map(|(f, r)| (*f, r.clone().map(|x| x.map(|p| p.display().to_string())))).as_ref()) {
                ui.interact(rr, ui.scope_id().with("upload-err"), Sense::hover()).on_hover_text(e);
            }
            card.bottom() - 3.0
        } else {
            img_rect.bottom() - 3.0
        };

        // time left
        let life = (self.remaining / self.cfg.toast_seconds.max(1.0)).clamp(0.0, 1.0);
        let inset = if self.cfg.toast_caption { 10.0 } else { PAD + 6.0 };
        let track = Rect::from_min_max(pos2(card.left() + inset, bar_y - 1.0), pos2(card.right() - inset, bar_y + 1.0));
        let fill = Rect::from_min_max(track.min, pos2(track.left() + track.width() * life, track.bottom()));
        painter.rect_filled(fill, 1, ACCENT.gamma_multiply(0.55 * alpha * (1.0 - hover_t * 0.6)));

        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

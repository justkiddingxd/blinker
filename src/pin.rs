//! "Pin on top": a screenshot floating above all windows. Each pin runs as its
//! own small process (`blinker --pin <file> <x> <y>`), so pins live on while
//! the main app opens and closes its windows.

use crate::icons::Icon;
use crate::theme::{self, *};
use crate::ui::{self, TiledTexture};
use crate::win::{self, IRect};
use eframe::egui::{self, Color32, Key, Rect, Stroke, StrokeKind, TextureOptions, pos2, vec2};
use image::RgbaImage;
use std::path::PathBuf;

/// Saves the image to the pins cache and starts a pin process for it.
pub fn spawn(img: &RgbaImage, at: IRect) {
    let dir = crate::config::cache_dir().join("pins");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("pin-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")));
    let encoder_ok = (|| -> Result<(), String> {
        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let enc = image::codecs::png::PngEncoder::new_with_quality(
            std::io::BufWriter::new(file),
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::NoFilter,
        );
        image::ImageEncoder::write_image(enc, img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8)
            .map_err(|e| e.to_string())
    })();
    if encoder_ok.is_err() {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe)
            .arg("--pin")
            .arg(&path)
            .arg(at.x.to_string())
            .arg(at.y.to_string())
            .spawn();
    }
}

/// Entry point of a pin process.
pub fn run(path: PathBuf, x: i32, y: i32) {
    let Some(img) = crate::output::load(&path) else { return };
    let mon = win::monitor_at(&win::monitors(), x + 1, y + 1);
    let work = mon.map(|m| m.work).unwrap_or(IRect::new(0, 0, 1920, 1040));
    let fit = (work.w as f32 * 0.8 / img.width() as f32).min(work.h as f32 * 0.8 / img.height() as f32).min(1.0);
    let rect = clamp_into(IRect::new(x, y, (img.width() as f32 * fit) as i32, (img.height() as f32 * fit) as i32), work);
    let scale = mon.map(|m| m.scale).unwrap_or(1.0);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(crate::i18n::t("blinker · закреплено"))
            .with_icon(theme::icon_data(64))
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top()
            .with_inner_size([rect.w as f32 / scale, rect.h as f32 / scale]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let _ = eframe::run_native(
        "blinker pin",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            let hwnd = win::hwnd_of(cc);
            if let Some(h) = hwnd {
                win::cloak(h, true);
                win::set_window_rect(h, rect, true);
            }
            let tex = TiledTexture::new(
                &cc.egui_ctx,
                "pin",
                &img,
                TextureOptions {
                    magnification: egui::TextureFilter::Linear,
                    minification: egui::TextureFilter::Linear,
                    wrap_mode: egui::TextureWrapMode::ClampToEdge,
                    mipmap_mode: Some(egui::TextureFilter::Linear),
                },
            );
            Ok(Box::new(PinApp { img, tex, hwnd, rect, zoom: fit, frame_no: 0, notice: None }))
        }),
    );
    let _ = std::fs::remove_file(&path);
}

fn clamp_into(r: IRect, work: IRect) -> IRect {
    let w = r.w.min(work.w).max(32);
    let h = r.h.min(work.h).max(32);
    let x = r.x.clamp(work.x, work.right() - w);
    let y = r.y.clamp(work.y, work.bottom() - h);
    IRect::new(x, y, w, h)
}

struct PinApp {
    img: RgbaImage,
    tex: TiledTexture,
    hwnd: Option<win::HWND>,
    rect: IRect,
    zoom: f32,
    frame_no: u32,
    notice: Option<ui::Notice>,
}

impl PinApp {
    fn set_zoom(&mut self, z: f32) {
        let z = z.clamp(0.1, 4.0);
        self.zoom = z;
        if let Some(h) = self.hwnd {
            let cur = win::window_rect(h);
            let w = ((self.img.width() as f32 * z) as i32).max(32);
            let hgt = ((self.img.height() as f32 * z) as i32).max(32);
            self.rect = IRect::new(cur.x, cur.y, w, hgt);
            win::set_window_rect(h, self.rect, true);
        }
    }

    fn copy(&mut self, ctx: &egui::Context) {
        if crate::output::copy_image(&self.img) {
            self.notice = Some(ui::Notice::new(ctx, crate::i18n::t("скопировано")));
        }
    }

    fn save_as(&mut self, ctx: &egui::Context) {
        let cfg = crate::config::load();
        let name = crate::output::new_path(&cfg);
        if let Some(p) = rfd::FileDialog::new()
            .set_directory(cfg.pics_dir())
            .set_file_name(name.file_name().unwrap_or_default().to_string_lossy())
            .add_filter("png", &["png"])
            .save_file()
            && crate::output::write(&self.img, &p, crate::config::ImageFormat::Png, 90).is_ok()
        {
            self.notice = Some(ui::Notice::new(ctx, crate::i18n::t("сохранено")));
        }
    }
}

impl eframe::App for PinApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame_no += 1;
        if let Some(h) = self.hwnd {
            if self.frame_no == 3 {
                win::cloak(h, false);
            } else if self.frame_no < 3 {
                ctx.request_repaint();
            }
        }
        if self.frame_no <= 10
            && let Some(h) = self.hwnd
            && win::window_rect(h) != self.rect
        {
            win::set_window_rect(h, self.rect, true);
        }
        let full = ctx.content_rect();
        let resp = ui.interact(full, ui.scope_id().with("pin"), egui::Sense::click_and_drag());
        self.tex.paint(ui.painter(), Rect::from_min_size(pos2(0.0, 0.0), self.tex.size), full, Color32::WHITE);
        let border = if resp.hovered() { ACCENT } else { LINE };
        ui.painter().rect_stroke(full, 0, Stroke::new(1.0, border), StrokeKind::Inside);

        if resp.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if resp.double_clicked() || ctx.input(|i| i.key_pressed(Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // Ctrl / Cmd + C arrives as a Copy event, not a key press
        if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)) || (i.modifiers.command && i.key_pressed(Key::C))) {
            self.copy(&ctx);
        }
        let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
        if resp.hovered() && scroll != 0.0 {
            self.set_zoom(self.zoom * (scroll * 0.002).exp());
        }
        resp.context_menu(|ui| {
            ui.set_min_width(180.0);
            if menu_item(ui, Icon::Copy, crate::i18n::t("копировать")) {
                self.copy(&ctx);
                ui.close();
            }
            if menu_item(ui, Icon::Save, crate::i18n::t("сохранить как…")) {
                ui.close();
                self.save_as(&ctx);
            }
            if menu_item(ui, Icon::Region, crate::i18n::t("исходный размер")) {
                self.set_zoom(1.0);
                ui.close();
            }
            if menu_item(ui, Icon::Close, crate::i18n::t("закрыть")) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        if let Some(n) = &self.notice
            && !n.show(&ctx, full)
        {
            self.notice = None;
        }
    }
}

fn menu_item(ui: &mut egui::Ui, icon: Icon, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(180.0), 28.0), egui::Sense::click());
    let h = resp.hovered();
    if h {
        ui.painter().rect_filled(rect, crate::ui::soft_radius(3), crate::ui::hover_fill());
    }
    let ir = Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(16.0, 16.0));
    crate::icons::paint(ui.painter(), ir, icon, 14.0, if h { ACCENT } else { DIM });
    ui.painter().text(
        pos2(rect.left() + 32.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font(13.0),
        if h { TEXT } else { TEXT2 },
    );
    resp.clicked()
}

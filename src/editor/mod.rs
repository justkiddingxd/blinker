//! Annotation editor window.

pub mod doc;
pub mod preview;
pub mod render;

use crate::config::ImageFormat;
use crate::output;
use crate::runtime::{self, Command, Shared};
use crate::theme::{self, *};
use crate::icons::Icon;
use crate::ui::{self, Notice, TiledTexture};
use crate::win;
use doc::{Kind, Prim, Shape, Tool};
use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, Key, KeyboardShortcut, Modifiers, Pos2, Rect, Sense,
    Stroke, StrokeKind, TextureOptions, Vec2, pos2, vec2,
};
use image::RgbaImage;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

pub const PALETTE: [(Color32, &str); 6] = [
    (Color32::from_rgb(0xE3, 0xB3, 0x41), "жёлтый"),
    (Color32::from_rgb(0xE5, 0x48, 0x4D), "красный"),
    (Color32::from_rgb(0x3E, 0x8B, 0xFF), "синий"),
    (Color32::from_rgb(0x46, 0xA7, 0x58), "зелёный"),
    (Color32::from_rgb(0xE6, 0xE4, 0xDE), "белый"),
    (Color32::from_rgb(0x16, 0x16, 0x15), "чёрный"),
];
pub const WIDTHS: [(f32, &str); 3] = [(2.0, "тонкая"), (4.0, "средняя"), (8.0, "толстая")];

#[derive(Clone)]
struct History {
    shapes: Vec<Shape>,
    crop: Option<Rect>,
}

#[derive(Clone)]
pub struct EditorState {
    pub image: Arc<RgbaImage>,
    pub path: Option<PathBuf>,
    shapes: Vec<Shape>,
    crop: Option<Rect>,
    undo: Vec<History>,
    redo: Vec<History>,
    dirty: bool,
    tool: Tool,
    color: Color32,
    width: f32,
    next_id: u64,
}

impl EditorState {
    pub fn new(image: Arc<RgbaImage>, path: Option<PathBuf>, color: [u8; 4], width: f32) -> Self {
        Self {
            image,
            path,
            shapes: Vec::new(),
            crop: None,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            tool: Tool::Arrow,
            color: Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]),
            width,
            next_id: 1,
        }
    }

    fn checkpoint(&mut self) {
        self.undo.push(History { shapes: self.shapes.clone(), crop: self.crop });
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    fn undo(&mut self) {
        if let Some(h) = self.undo.pop() {
            self.redo.push(History { shapes: std::mem::replace(&mut self.shapes, h.shapes), crop: self.crop });
            self.crop = h.crop;
            self.dirty = true;
        }
    }

    fn redo(&mut self) {
        if let Some(h) = self.redo.pop() {
            self.undo.push(History { shapes: std::mem::replace(&mut self.shapes, h.shapes), crop: self.crop });
            self.crop = h.crop;
            self.dirty = true;
        }
    }

    fn shape_mut(&mut self, id: u64) -> Option<&mut Shape> {
        self.shapes.iter_mut().find(|s| s.id == id)
    }

    fn full_rect(&self) -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(self.image.width() as f32, self.image.height() as f32))
    }

    pub fn render(&self) -> RgbaImage {
        render::render(&self.image, &self.shapes, self.crop)
    }
}

pub enum EditorExit {
    Closed,
}

thread_local! {
    /// The window's width, for the glass title bar (macOS 26).
    static GLASS_W: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

pub fn run(shared: &Arc<Shared>, state: EditorState, notice: Option<String>) -> EditorExit {
    let exit: Rc<RefCell<Option<EditorExit>>> = Rc::default();
    let exit2 = exit.clone();
    let mon = win::monitor_at(&win::monitors(), win::cursor_pos().0, win::cursor_pos().1);
    let (work, scale) = mon.map(|m| (m.work, m.scale)).unwrap_or((win::IRect::new(0, 0, 1920, 1040), 1.0));
    let avail = vec2(work.w as f32 / scale * 0.88, work.h as f32 / scale * 0.88);
    let want = vec2(state.image.width() as f32 / scale + 80.0, state.image.height() as f32 / scale + 160.0);
    let size = vec2(want.x.clamp(940.0, avail.x.max(940.0)), want.y.clamp(600.0, avail.y.max(600.0)));
    let title = match &state.path {
        Some(p) => format!("{} — blinker", p.file_name().unwrap_or_default().to_string_lossy()),
        None => crate::i18n::t("новый снимок — blinker").to_string(),
    };
    let viewport = egui::ViewportBuilder::default()
        .with_title(title)
        .with_icon(theme::icon_data(64))
        .with_inner_size(size)
        .with_transparent(theme::glass());
    GLASS_W.with(|w| w.set(size.x));
    let viewport = crate::chrome::style(viewport);
    let shared2 = shared.clone();
    runtime::run_window(shared, viewport, true, move |cc| {
        let hwnd = win::hwnd_of(cc);
        if let Some(h) = hwnd {
            win::style_borderless(h);
            crate::chrome::place_window_buttons(h, crate::chrome::TITLE_H + 4.0);
            crate::chrome::glass_bar(h, GLASS_W.with(|w| w.get()), crate::chrome::TITLE_H + 4.0);
            win::cloak(h, true);
            win::force_foreground(h);
        }
        let tex = TiledTexture::new(
            &cc.egui_ctx,
            "editor-base",
            &state.image,
            TextureOptions {
                magnification: egui::TextureFilter::Nearest,
                minification: egui::TextureFilter::Linear,
                wrap_mode: egui::TextureWrapMode::ClampToEdge,
                mipmap_mode: Some(egui::TextureFilter::Linear),
            },
        );
        let notice = notice.map(|n| Notice::new(&cc.egui_ctx, n));
        let shared2_beauty = shared2.config().beauty;
        EditorApp {
            hwnd,
            frame_no: 0,
            shared: shared2,
            st: state,
            tex,
            zoom: 1.0,
            offset: Vec2::ZERO,
            fit: true,
            drag: EDrag::None,
            selected: None,
            text: None,
            crop_draft: None,
            prims: preview::PrimPainter::default(),
            notice,
            exit: exit2,
            confirm_close: false,
            allow_close: false,
            pan_from: None,
            canvas: Rect::NOTHING,
            wants_focus_text: false,
            beauty_on: false,
            beauty: shared2_beauty,
            beauty_tex: None,
        }
    });
    exit.borrow_mut().take().unwrap_or(EditorExit::Closed)
}

#[derive(Clone, Copy, PartialEq)]
enum EDrag {
    None,
    Draw { id: u64, start: Pos2 },
    Move { id: u64, last: Pos2, moved: bool },
    Crop { start: Pos2 },
    CropMove { grab: Vec2 },
}

struct TextEditing {
    pos: Pos2,
    buf: String,
    id: Option<u64>,
}

struct EditorApp {
    hwnd: Option<win::HWND>,
    /// Frames drawn; the window is uncloaked on the third.
    frame_no: u32,
    shared: Arc<Shared>,
    st: EditorState,
    tex: TiledTexture,
    /// Screen points per image pixel, and screen position of image pixel (0,0).
    zoom: f32,
    offset: Vec2,
    fit: bool,
    drag: EDrag,
    selected: Option<u64>,
    text: Option<TextEditing>,
    crop_draft: Option<Rect>,
    prims: preview::PrimPainter,
    notice: Option<Notice>,
    exit: Rc<RefCell<Option<EditorExit>>>,
    confirm_close: bool,
    allow_close: bool,
    pan_from: Option<Pos2>,
    canvas: Rect,
    wants_focus_text: bool,
    /// "Beautiful shot" mode: the canvas shows the framed result.
    beauty_on: bool,
    beauty: crate::config::Beauty,
    beauty_tex: Option<(String, egui::TextureHandle)>,
}

impl EditorApp {
    fn to_screen(&self, p: Pos2) -> Pos2 {
        (self.offset + p.to_vec2() * self.zoom).to_pos2()
    }
    fn to_img(&self, p: Pos2) -> Pos2 {
        ((p.to_vec2() - self.offset) / self.zoom).to_pos2()
    }
    fn rect_to_screen(&self, r: Rect) -> Rect {
        Rect::from_min_max(self.to_screen(r.min), self.to_screen(r.max))
    }

    fn shown_rect(&self) -> Rect {
        if self.st.tool == Tool::Crop {
            self.st.full_rect()
        } else {
            self.st.crop.unwrap_or(self.st.full_rect())
        }
    }

    fn fit_view(&mut self, ppp: f32) {
        let area = self.canvas.shrink(36.0);
        let r = self.shown_rect();
        let z = (area.width() / r.width()).min(area.height() / r.height()).min(1.0 / ppp).max(0.01);
        self.zoom = z;
        self.offset = self.canvas.center().to_vec2() - r.center().to_vec2() * z;
    }

    fn zoom_at(&mut self, factor: f32, anchor: Pos2, ppp: f32) {
        let new = (self.zoom * factor).clamp(0.05 / ppp, 32.0 / ppp);
        let f = new / self.zoom;
        self.offset = anchor.to_vec2() + (self.offset - anchor.to_vec2()) * f;
        self.zoom = new;
        self.fit = false;
    }

    fn text_size_fn() -> impl Fn(&str, f32) -> Vec2 {
        render::text_box
    }

    fn set_tool(&mut self, t: Tool) {
        self.commit_text();
        if t == Tool::Crop {
            self.crop_draft = self.st.crop;
        } else {
            self.crop_draft = None;
        }
        if self.st.tool == Tool::Crop || t == Tool::Crop {
            self.fit = true;
        }
        self.st.tool = t;
        if t != Tool::Select {
            self.selected = None;
        }
    }

    fn set_color(&mut self, c: Color32) {
        self.st.color = c;
        if let Some(id) = self.selected {
            self.st.checkpoint();
            if let Some(s) = self.st.shape_mut(id) {
                s.color = c;
            }
        }
        let rgba = c.to_srgba_unmultiplied();
        self.shared.update_config(|cfg| cfg.editor_color = rgba);
    }

    fn set_width(&mut self, w: f32) {
        self.st.width = w;
        if let Some(id) = self.selected {
            self.st.checkpoint();
            if let Some(s) = self.st.shape_mut(id) {
                s.width = w;
            }
        }
        self.shared.update_config(|cfg| cfg.editor_width = w);
    }

    fn commit_text(&mut self) {
        let Some(t) = self.text.take() else { return };
        let text = t.buf.trim_end().to_string();
        match t.id {
            Some(id) => {
                if text.is_empty() {
                    self.st.checkpoint();
                    self.st.shapes.retain(|s| s.id != id);
                } else if let Some(s) = self.st.shapes.iter().find(|s| s.id == id) {
                    let (color, width) = (self.st.color, self.st.width);
                    let changed = s.color != color
                        || s.width != width
                        || !matches!(&s.kind, Kind::Text { text: old, .. } if *old == text);
                    if changed {
                        self.st.checkpoint();
                        if let Some(s) = self.st.shape_mut(id) {
                            s.color = color;
                            s.width = width;
                            if let Kind::Text { text: old, .. } = &mut s.kind {
                                *old = text;
                            }
                        }
                    }
                }
            }
            None if !text.is_empty() => {
                self.st.checkpoint();
                let id = self.st.next_id;
                self.st.next_id += 1;
                self.st.shapes.push(Shape {
                    id,
                    kind: Kind::Text { pos: t.pos, text },
                    color: self.st.color,
                    width: self.st.width,
                });
            }
            None => {}
        }
    }

    fn apply_crop(&mut self) {
        if let Some(d) = self.crop_draft {
            let full = self.st.full_rect();
            let d = Rect::from_min_max(d.min.round(), d.max.round()).intersect(full);
            if d.width() >= 2.0 && d.height() >= 2.0 {
                self.st.checkpoint();
                self.st.crop = if d == full { None } else { Some(d) };
            }
        }
        self.crop_draft = None;
        self.st.tool = Tool::Select;
        self.fit = true;
    }

    /// The image as it will be copied / saved (framed in beauty mode).
    fn output(&self) -> RgbaImage {
        let img = self.st.render();
        if self.beauty_on { crate::beauty::render(&img, &self.beauty) } else { img }
    }

    fn copy(&mut self, ctx: &egui::Context) {
        self.commit_text();
        let img = self.output();
        if output::copy_image(&img) {
            self.notice = Some(Notice::new(ctx, crate::i18n::t("скопировано в буфер обмена")));
        }
    }

    fn save(&mut self, ctx: &egui::Context) -> bool {
        self.commit_text();
        let cfg = self.shared.config();
        let img = self.output();
        let path = match &self.st.path {
            Some(p) => {
                let fmt = p
                    .extension()
                    .and_then(|e| ImageFormat::from_extension(&e.to_string_lossy()))
                    .unwrap_or(cfg.format);
                if let Err(e) = output::write(&img, p, fmt, cfg.jpeg_quality) {
                    self.notice = Some(Notice::new(ctx, lf!("не удалось сохранить: {e}", "could not save: {e}")));
                    return false;
                }
                p.clone()
            }
            None => {
                let p = output::new_path(&cfg);
                if let Err(e) = output::write(&img, &p, cfg.format, cfg.jpeg_quality) {
                    self.notice = Some(Notice::new(ctx, lf!("не удалось сохранить: {e}", "could not save: {e}")));
                    return false;
                }
                p
            }
        };
        self.after_save(ctx, path);
        true
    }

    fn save_as(&mut self, ctx: &egui::Context) {
        self.commit_text();
        let cfg = self.shared.config();
        let default = output::new_path(&cfg);
        let Some(path) = rfd::FileDialog::new()
            .set_directory(cfg.pics_dir())
            .set_file_name(default.file_name().unwrap_or_default().to_string_lossy())
            .add_filter("png", &["png"])
            .add_filter(crate::i18n::t("webp (без потерь)"), &["webp"])
            .add_filter("jpeg", &["jpg", "jpeg"])
            .save_file()
        else {
            return;
        };
        let fmt = path
            .extension()
            .and_then(|e| ImageFormat::from_extension(&e.to_string_lossy()))
            .unwrap_or(ImageFormat::Png);
        let img = self.output();
        match output::write(&img, &path, fmt, cfg.jpeg_quality) {
            Ok(()) => self.after_save(ctx, path),
            Err(e) => self.notice = Some(Notice::new(ctx, lf!("не удалось сохранить: {e}", "could not save: {e}"))),
        }
    }

    fn after_save(&mut self, ctx: &egui::Context, path: PathBuf) {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{name} — blinker")));
        self.st.path = Some(path);
        self.st.dirty = false;
        self.notice = Some(Notice::new(ctx, lf!("сохранено · {name}", "saved · {name}")));
    }

    // ------------------------------------------------------------------ UI

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("topbar")
            .exact_size(crate::chrome::TITLE_H + 4.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(crate::chrome::bar_fill()).inner_margin(egui::Margin { left: 14, right: 0, top: 0, bottom: 0 }))
            .show(ui, |ui| {
                let r = ui.max_rect();
                crate::chrome::drag_region(ui, r.expand2(vec2(14.0, 0.0)));
                if !theme::glass() {
                    ui.painter().hline(r.x_range().expand(14.0), r.bottom() + 0.5, Stroke::new(1.0, LINE));
                }
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    crate::chrome::leading_space(ui);
                    crate::chrome::mark(ui);
                    let name = match &self.st.path {
                        Some(p) => p.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                        None => crate::i18n::t("новый снимок").to_string(),
                    };
                    ui.label(egui::RichText::new(name).font(font_medium(13.0)).color(TEXT));
                    ui.add_space(4.0);
                    let state = if self.st.dirty { crate::i18n::t("изменён") } else { crate::i18n::t("сохранён") };
                    ui.label(egui::RichText::new(state).font(mono(11.0)).color(FAINT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        crate::chrome::window_buttons(ui);
                        ui.spacing_mut().item_spacing.x = 4.0;
                        sep(ui);
                        if ui::button(ui, crate::i18n::t("копировать"), ui::Kind::Accent).on_hover_text(crate::i18n::kb("ctrl+c")).clicked() {
                            self.copy(&ctx);
                        }
                        if ui::button(ui, crate::i18n::t("сохранить"), ui::Kind::Ghost).on_hover_text(crate::i18n::kb("ctrl+s")).clicked() {
                            self.save(&ctx);
                        }
                        let more = ui::icon_button(ui, Icon::Settings, crate::i18n::t("ещё"), 30.0, false, ui::Kind::Ghost);
                        egui::Popup::menu(&more).show(|ui| {
                            ui.set_min_width(210.0);
                            if menu_item(ui, Icon::Save, crate::i18n::t("сохранить как…"), &crate::i18n::kb("ctrl+shift+s")) {
                                self.save_as(&ctx);
                                ui.close();
                            }
                            if let Some(p) = self.st.path.clone()
                                && menu_item(ui, Icon::Folder, crate::i18n::t("показать в папке"), "")
                            {
                                win::reveal_in_explorer(&p);
                                ui.close();
                            }
                            if menu_item(ui, Icon::Pin, crate::i18n::t("закрепить поверх окон"), "") {
                                let img = self.output();
                                let (x, y) = win::cursor_pos();
                                crate::pin::spawn(&img, win::IRect::new(x - 40, y - 40, 0, 0));
                                ui.close();
                            }
                            if self.st.crop.is_some() && menu_item(ui, Icon::Crop, crate::i18n::t("сбросить обрезку"), "") {
                                self.st.checkpoint();
                                self.st.crop = None;
                                self.fit = true;
                                ui.close();
                            }
                        });
                        sep(ui);
                        if ui::button(ui, crate::i18n::t("очистить"), ui::Kind::Ghost).clicked()
                            && (!self.st.shapes.is_empty() || self.st.crop.is_some())
                        {
                            self.commit_text();
                            self.st.checkpoint();
                            self.st.shapes.clear();
                            self.st.crop = None;
                            self.selected = None;
                            self.fit = true;
                        }
                        let can_undo = !self.st.undo.is_empty();
                        ui.add_enabled_ui(can_undo, |ui| {
                            if ui::button(ui, crate::i18n::t("отменить"), ui::Kind::Ghost).on_hover_text(crate::i18n::kb("ctrl+z")).clicked() {
                                self.commit_text();
                                self.st.undo();
                            }
                        });
                    });
                });
            });
    }

    fn tool_strip(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("tools")
            .exact_size(50.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(6, 8)))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().vline(r.right() + 6.5, r.y_range().expand(8.0), Stroke::new(1.0, LINE));
                ui.spacing_mut().item_spacing.y = 2.0;
                for t in Tool::ALL {
                    if t == Tool::Crop || t == Tool::Arrow {
                        ui.add_space(6.0);
                    }
                    let tip = format!("{}  {}", t.label(), t.key().name());
                    if ui::icon_button(ui, tool_icon(t), &tip, 38.0, self.st.tool == t, ui::Kind::Ghost).clicked() {
                        self.set_tool(t);
                    }
                }
            });
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("props")
            .exact_size(240.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(12, 14)))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().vline(r.left() - 12.5, r.y_range().expand(14.0), Stroke::new(1.0, LINE));
                ui.spacing_mut().item_spacing.y = 8.0;
                let label = if self.beauty_on { crate::i18n::t("вернуться к рисованию") } else { crate::i18n::t("красивый снимок") };
                let kind = if self.beauty_on { ui::Kind::Ghost } else { ui::Kind::Accent };
                if ui::button_h(ui, label, kind, 30.0)
                    .on_hover_text(crate::i18n::t("фон, тень и скругление"))
                    .clicked()
                {
                    self.commit_text();
                    self.beauty_on = !self.beauty_on;
                }
                ui.add_space(8.0);
                if self.beauty_on {
                    self.beauty_panel(ui);
                    return;
                }
                ui::section_label(ui, crate::i18n::t("цвет"));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (c, name) in PALETTE {
                        if swatch(ui, c, self.st.color == c).on_hover_text(crate::i18n::t(name)).clicked() {
                            self.set_color(c);
                        }
                    }
                });
                ui.add_space(10.0);
                ui::section_label(ui, crate::i18n::t("толщина"));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let w = (ui.available_width() - 4.0) / 3.0;
                    for (i, (px, name)) in WIDTHS.iter().enumerate() {
                        let on = (self.st.width - px).abs() < 0.01;
                        if width_button(ui, w, i, *px, on).on_hover_text(format!("{}  {}", crate::i18n::t(name), i + 1)).clicked() {
                            self.set_width(*px);
                        }
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui::section_label(ui, crate::i18n::t("объекты"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(self.st.shapes.len().to_string()).font(mono(11.0)).color(FAINT));
                    });
                });
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut remove = None;
                let mut select = None;
                egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                    for (i, s) in self.st.shapes.iter().enumerate().rev() {
                        let sel = self.selected == Some(s.id);
                        match object_row(ui, s, sel) {
                            RowAction::Select => select = Some(s.id),
                            RowAction::Remove => remove = Some(i),
                            RowAction::None => {}
                        }
                    }
                    if self.st.shapes.is_empty() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(crate::i18n::t("пока ничего не нарисовано")).font(font(12.0)).color(FAINT));
                    }
                });
                if let Some(id) = select {
                    self.st.tool = Tool::Select;
                    self.selected = Some(id);
                }
                if let Some(i) = remove {
                    self.st.checkpoint();
                    let id = self.st.shapes.remove(i).id;
                    if self.selected == Some(id) {
                        self.selected = None;
                    }
                }
            });
    }

    fn beauty_panel(&mut self, ui: &mut egui::Ui) {
        let before = self.beauty.clone();
        ui::section_label(ui, crate::i18n::t("фон"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (i, bg) in crate::beauty::BACKGROUNDS.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(44.0, 30.0), Sense::click());
                let p = ui.painter();
                match bg.colors {
                    Some((a, b)) => {
                        let ca = Color32::from_rgb(a[0], a[1], a[2]);
                        let cb = Color32::from_rgb(b[0], b[1], b[2]);
                        let mut mesh = egui::Mesh::default();
                        mesh.colored_vertex(rect.left_top(), ca);
                        mesh.colored_vertex(rect.right_top(), crate::theme::lerp_color(ca, cb, 0.5));
                        mesh.colored_vertex(rect.right_bottom(), cb);
                        mesh.colored_vertex(rect.left_bottom(), crate::theme::lerp_color(ca, cb, 0.5));
                        mesh.add_triangle(0, 1, 2);
                        mesh.add_triangle(0, 2, 3);
                        p.add(egui::Shape::mesh(mesh));
                    }
                    None => {
                        // checkerboard = transparent
                        for yy in 0..3 {
                            for xx in 0..4 {
                                let c = if (xx + yy) % 2 == 0 { Color32::from_gray(70) } else { Color32::from_gray(45) };
                                let cell = Rect::from_min_size(rect.min + vec2(xx as f32 * 11.0, yy as f32 * 10.0), vec2(11.0, 10.0));
                                p.rect_filled(cell, 0, c);
                            }
                        }
                    }
                }
                let on = self.beauty.background == i;
                p.rect_stroke(rect, 3, Stroke::new(if on { 2.0 } else { 1.0 }, if on { ACCENT } else { LINE }), StrokeKind::Outside);
                if resp.on_hover_text(crate::i18n::t(bg.name)).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.beauty.background = i;
                }
            }
        });
        ui.add_space(8.0);
        ui::section_label(ui, crate::i18n::t("пропорции"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
            for a in crate::config::Aspect::ALL {
                if ui::filter(ui, a.label(), self.beauty.aspect == a).clicked() {
                    self.beauty.aspect = a;
                }
            }
        });
        ui.add_space(8.0);
        ui.spacing_mut().slider_width = ui.available_width() - 8.0;
        ui::section_label(ui, crate::i18n::t("отступ"));
        ui.add(egui::Slider::new(&mut self.beauty.padding, 0.02..=0.2).show_value(false));
        ui::section_label(ui, crate::i18n::t("скругление"));
        ui.add(egui::Slider::new(&mut self.beauty.radius, 0.0..=40.0).show_value(false));
        ui::section_label(ui, crate::i18n::t("тень"));
        ui.add(egui::Slider::new(&mut self.beauty.shadow, 0.0..=1.0).show_value(false));
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(crate::i18n::t("«копировать» и «сохранить» отдадут снимок уже в оформлении"))
                .font(font(12.0))
                .color(FAINT),
        );
        if before != self.beauty {
            let b = self.beauty.clone();
            self.shared.update_config(|c| c.beauty = b);
        }
    }

    /// Canvas in beauty mode: the framed result (downscaled preview).
    fn beauty_preview(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::hover());
        self.canvas = resp.rect;
        painter.rect_filled(resp.rect, 0, CANVAS);
        let key = format!("{:?}|{}|{}|{:?}", self.beauty, self.st.shapes.len(), self.st.undo.len(), self.st.crop);
        if self.beauty_tex.as_ref().is_none_or(|(k, _)| *k != key) {
            let img = self.st.render();
            let max = 1600u32;
            let small = if img.width().max(img.height()) > max {
                let k = max as f32 / img.width().max(img.height()) as f32;
                image::imageops::resize(&img, (img.width() as f32 * k) as u32, (img.height() as f32 * k) as u32, image::imageops::FilterType::Triangle)
            } else {
                img
            };
            let framed = crate::beauty::render(&small, &self.beauty);
            let tex = ctx.load_texture("beauty", crate::theme::color_image(&framed), egui::TextureOptions::LINEAR);
            self.beauty_tex = Some((key, tex));
        }
        if let Some((_, tex)) = &self.beauty_tex {
            let area = resp.rect.shrink(28.0);
            let size = tex.size_vec2();
            let k = (area.width() / size.x).min(area.height() / size.y).min(1.0 / ctx.pixels_per_point() * 2.0);
            let r = Rect::from_center_size(area.center(), size * k);
            if crate::beauty::background(&self.beauty).colors.is_none() {
                // checkerboard behind a transparent result
                let cell = 12.0;
                let mut y = r.top();
                let mut row = 0;
                while y < r.bottom() {
                    let mut x = r.left() + if row % 2 == 0 { 0.0 } else { cell };
                    while x < r.right() {
                        let c = Rect::from_min_max(pos2(x, y), pos2((x + cell).min(r.right()), (y + cell).min(r.bottom())));
                        painter.rect_filled(c, 0, Color32::from_gray(38));
                        x += cell * 2.0;
                    }
                    y += cell;
                    row += 1;
                }
            }
            painter.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let ppp = ui.ctx().pixels_per_point();
        egui::Panel::bottom("status")
            .exact_size(26.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(14, 0)))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().hline(r.x_range().expand(14.0), r.top() - 0.5, Stroke::new(1.0, LINE));
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 18.0;
                    let shown = self.st.crop.unwrap_or(self.st.full_rect());
                    let dim = |t: String| egui::RichText::new(t).font(mono(11.0)).color(DIM);
                    ui.label(dim(format!("{} × {}", shown.width() as i32, shown.height() as i32)));
                    let pct = format!("{:.0}%", self.zoom * ppp * 100.0);
                    let z = ui.add(egui::Label::new(dim(pct)).sense(Sense::click()));
                    if z.on_hover_text(crate::i18n::kb(crate::i18n::t("клик: 100%  ·  ctrl+0: вписать"))).clicked() {
                        let c = self.canvas.center();
                        self.zoom_at(1.0 / (self.zoom * ppp), c, ppp);
                    }
                    let fmt = self
                        .st
                        .path
                        .as_ref()
                        .and_then(|p| p.extension())
                        .map(|e| e.to_string_lossy().to_lowercase())
                        .unwrap_or_else(|| self.shared.config().format.extension().to_lowercase());
                    ui.label(dim(fmt));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let hint = format!("{}: {}", self.st.tool.label(), self.st.tool.hint());
                        ui.label(egui::RichText::new(hint).font(font(12.0)).color(DIM));
                    });
                });
            });
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.text.is_some() || self.confirm_close {
            return;
        }
        let ppp = ctx.pixels_per_point();
        let typing = ctx.egui_wants_keyboard_input();
        let (cmd, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        // Ctrl / Cmd + C arrives as a Copy event, not a key press; text being
        // typed keeps it to itself
        if ctx.memory(|m| m.focused().is_none()) && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)) || (i.modifiers.command && i.key_pressed(Key::C))) {
            self.copy(ctx);
        }
        if cmd {
            if pressed(Key::Z) {
                if shift { self.st.redo() } else { self.st.undo() }
            }
            if pressed(Key::Y) {
                self.st.redo();
            }
            if pressed(Key::S) {
                if shift { self.save_as(ctx) } else { self.save(ctx); }
            }
            if pressed(Key::Num0) {
                self.fit = true;
            }
            if pressed(Key::Num1) {
                let c = self.canvas.center();
                self.zoom_at(1.0 / (self.zoom * ppp), c, ppp);
            }
            return;
        }
        if typing {
            return;
        }
        for t in Tool::ALL {
            if pressed(t.key()) {
                self.set_tool(t);
            }
        }
        for (i, k) in [Key::Num1, Key::Num2, Key::Num3].into_iter().enumerate() {
            if pressed(k) {
                self.set_width(WIDTHS[i].0);
            }
        }
        if (pressed(Key::Delete) || pressed(Key::Backspace))
            && let Some(id) = self.selected.take()
        {
            self.st.checkpoint();
            self.st.shapes.retain(|s| s.id != id);
        }
        if pressed(Key::Enter) && self.st.tool == Tool::Crop {
            self.apply_crop();
        }
        if pressed(Key::Escape) {
            if self.st.tool == Tool::Crop {
                self.crop_draft = None;
                self.st.tool = Tool::Select;
                self.fit = true;
            } else {
                self.selected = None;
            }
        }
    }

    fn canvas_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        self.canvas = resp.rect;
        if self.fit {
            self.fit_view(ppp);
        }

        // --- zoom & pan
        if resp.hovered() {
            let (scroll, zoom_delta, pointer) =
                ctx.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta(), i.pointer.hover_pos()));
            let anchor = pointer.unwrap_or(self.canvas.center());
            if zoom_delta != 1.0 {
                self.zoom_at(zoom_delta, anchor, ppp);
            } else if scroll != 0.0 {
                self.zoom_at((scroll * 0.0022).exp(), anchor, ppp);
            }
        }
        let space = ctx.input(|i| i.key_down(Key::Space)) && self.text.is_none();
        let middle = ctx.input(|i| i.pointer.middle_down());
        if (space && resp.dragged()) || middle {
            if let Some(p) = ctx.input(|i| i.pointer.latest_pos()) {
                if let Some(from) = self.pan_from {
                    self.offset += p - from;
                    self.fit = false;
                }
                self.pan_from = Some(p);
            }
            ctx.set_cursor_icon(CursorIcon::Grabbing);
        } else {
            self.pan_from = None;
            if !space {
                self.handle_tool_input(&ctx, &resp);
            } else if resp.hovered() {
                ctx.set_cursor_icon(CursorIcon::Grab);
            }
        }

        // --- paint
        painter.rect_filled(self.canvas, 0, CANVAS);
        let shown = self.shown_rect();
        let img_screen = self.rect_to_screen(shown);
        let clip = painter.with_clip_rect(img_screen.intersect(self.canvas));
        self.tex.paint(&clip, shown, img_screen, Color32::WHITE);

        let shapes: Vec<(u64, Vec<Prim>)> = self
            .st
            .shapes
            .iter()
            .filter(|s| self.text.as_ref().and_then(|t| t.id) != Some(s.id))
            .map(|s| (s.id, s.prims()))
            .collect();
        let view = preview::View { offset: self.offset, zoom: self.zoom };
        for (_, prims) in &shapes {
            for p in prims {
                self.prims.paint(&clip, &ctx, &self.st.image, p, view);
            }
        }
        self.prims.end_frame();

        // selection outline
        if let Some(id) = self.selected
            && let Some(s) = self.st.shapes.iter().find(|s| s.id == id)
        {
            let b = self.rect_to_screen(s.bounds(Self::text_size_fn())).expand(4.0);
            let dash = dashed_rect(b);
            painter.extend(dash);
        }

        // crop UI
        if self.st.tool == Tool::Crop {
            self.paint_crop(&painter, img_screen);
        }

        // text editing
        self.text_editor(&ctx);
    }

    fn paint_crop(&self, painter: &egui::Painter, img_screen: Rect) {
        let Some(d) = self.crop_draft else {
            painter.rect_filled(img_screen, 0, Color32::from_black_alpha(90));
            return;
        };
        let f = self.rect_to_screen(d);
        let dim = Color32::from_black_alpha(150);
        let full = img_screen;
        for r in [
            Rect::from_min_max(full.min, pos2(full.max.x, f.min.y)),
            Rect::from_min_max(pos2(full.min.x, f.max.y), full.max),
            Rect::from_min_max(pos2(full.min.x, f.min.y), pos2(f.min.x, f.max.y)),
            Rect::from_min_max(pos2(f.max.x, f.min.y), pos2(full.max.x, f.max.y)),
        ] {
            if r.is_positive() {
                painter.rect_filled(r, 0, dim);
            }
        }
        painter.rect_stroke(f, 0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Outside);
        // rule-of-thirds
        let thirds = Stroke::new(1.0, Color32::from_white_alpha(50));
        for i in 1..3 {
            let x = f.left() + f.width() * i as f32 / 3.0;
            let y = f.top() + f.height() * i as f32 / 3.0;
            painter.vline(x, f.y_range(), thirds);
            painter.hline(f.x_range(), y, thirds);
        }
        let len = 14.0f32.min(f.width() / 3.0).min(f.height() / 3.0);
        let st = Stroke::new(3.0, Color32::WHITE);
        let ff = f.expand(2.0);
        for (c, dx, dy) in [(ff.left_top(), 1.0, 1.0), (ff.right_top(), -1.0, 1.0), (ff.left_bottom(), 1.0, -1.0), (ff.right_bottom(), -1.0, -1.0)] {
            painter.line(vec![c + vec2(0.0, dy * len), c, c + vec2(dx * len, 0.0)], st);
        }
        let label = format!("{} × {}", d.width().round() as i32, d.height().round() as i32);
        let g = painter.layout_no_wrap(label, font_semibold(12.0), TEXT);
        let br = Rect::from_min_size(f.left_top() + vec2(8.0, 8.0), g.size() + vec2(16.0, 8.0));
        painter.rect_filled(br, 6, Color32::from_rgba_unmultiplied(16, 17, 22, 220));
        painter.galley(br.center() - g.size() / 2.0, g, TEXT);
    }

    fn handle_tool_input(&mut self, ctx: &egui::Context, resp: &egui::Response) {
        let Some(pointer) = ctx.input(|i| i.pointer.latest_pos()) else { return };
        let p = self.to_img(pointer);
        let tol = 6.0 / self.zoom;
        let (pressed, down, released, shift) = ctx.input(|i| {
            (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released(), i.modifiers.shift)
        });
        let on_canvas = resp.hovered() || resp.dragged() || !matches!(self.drag, EDrag::None);
        if !on_canvas {
            return;
        }
        let tool = self.st.tool;
        let tsz = Self::text_size_fn();

        // cursor
        let hover_shape = self.st.shapes.iter().rev().find(|s| s.hit(p, tol, &tsz)).map(|s| s.id);
        let cursor = match (tool, self.drag) {
            (_, EDrag::Move { .. }) => CursorIcon::Grabbing,
            (Tool::Select, _) if hover_shape.is_some() => CursorIcon::Grab,
            (Tool::Select, _) => CursorIcon::Default,
            (Tool::Text, _) => CursorIcon::Text,
            (Tool::Crop, _) if self.crop_draft.is_some_and(|d| d.contains(p)) => CursorIcon::Move,
            _ => CursorIcon::Crosshair,
        };
        if resp.hovered() {
            ctx.set_cursor_icon(cursor);
        }

        if pressed && resp.hovered() {
            let had_text = self.text.is_some();
            self.commit_text();
            match tool {
                Tool::Select => {
                    self.selected = hover_shape;
                    if let Some(id) = hover_shape {
                        self.drag = EDrag::Move { id, last: p, moved: false };
                    }
                }
                Tool::Text => {
                    if !had_text {
                        let existing = self.st.shapes.iter().rev().find(|s| {
                            matches!(s.kind, Kind::Text { .. }) && s.hit(p, tol, &tsz)
                        });
                        self.text = Some(match existing {
                            Some(s) => {
                                let Kind::Text { pos, text } = &s.kind else { unreachable!() };
                                self.st.color = s.color;
                                self.st.width = s.width;
                                TextEditing { pos: *pos, buf: text.clone(), id: Some(s.id) }
                            }
                            None => {
                                let lh = render::line_height(doc::text_size(self.st.width));
                                TextEditing { pos: p - vec2(0.0, lh * 0.5), buf: String::new(), id: None }
                            }
                        });
                        self.wants_focus_text = true;
                    }
                }
                Tool::Counter => {
                    self.st.checkpoint();
                    let n = self
                        .st
                        .shapes
                        .iter()
                        .filter_map(|s| if let Kind::Counter { n, .. } = s.kind { Some(n) } else { None })
                        .max()
                        .unwrap_or(0)
                        + 1;
                    let id = self.st.next_id;
                    self.st.next_id += 1;
                    self.st.shapes.push(Shape { id, kind: Kind::Counter { c: p, n }, color: self.st.color, width: self.st.width });
                }
                Tool::Crop => {
                    self.drag = match self.crop_draft {
                        Some(d) if d.contains(p) => EDrag::CropMove { grab: d.min - p },
                        _ => {
                            self.crop_draft = None;
                            EDrag::Crop { start: p }
                        }
                    };
                }
                _ => {
                    self.st.checkpoint();
                    let id = self.st.next_id;
                    self.st.next_id += 1;
                    let kind = match tool {
                        Tool::Arrow => Kind::Arrow { a: p, b: p },
                        Tool::Line => Kind::Line { a: p, b: p },
                        Tool::Rect => Kind::Rect { r: Rect::from_two_pos(p, p) },
                        Tool::Ellipse => Kind::Ellipse { r: Rect::from_two_pos(p, p) },
                        Tool::Pen => Kind::Pen { pts: vec![p] },
                        Tool::Marker => Kind::Marker { pts: vec![p] },
                        _ => Kind::Pixelate { r: Rect::from_two_pos(p, p) },
                    };
                    self.st.shapes.push(Shape { id, kind, color: self.st.color, width: self.st.width });
                    self.drag = EDrag::Draw { id, start: p };
                }
            }
        }

        if down {
            match self.drag {
                EDrag::Draw { id, start } => {
                    let full = self.st.full_rect();
                    if let Some(s) = self.st.shape_mut(id) {
                        match &mut s.kind {
                            Kind::Arrow { b, .. } | Kind::Line { b, .. } => {
                                *b = if shift { doc::snap_45(start, p) } else { p };
                            }
                            Kind::Rect { r } | Kind::Ellipse { r } => {
                                *r = if shift { doc::square_from(start, p) } else { Rect::from_two_pos(start, p) };
                            }
                            Kind::Pixelate { r } => *r = Rect::from_two_pos(start, p).intersect(full),
                            Kind::Pen { pts } => {
                                if pts.last().is_none_or(|l| (*l - p).length() * self.zoom > 1.5) {
                                    pts.push(p);
                                }
                            }
                            Kind::Marker { pts } => {
                                if shift {
                                    *pts = vec![start, pos2(p.x, start.y)];
                                } else if pts.last().is_none_or(|l| (*l - p).length() * self.zoom > 1.5) {
                                    pts.push(p);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                EDrag::Move { id, last, moved } => {
                    let d = p - last;
                    if d != Vec2::ZERO {
                        if !moved {
                            self.st.checkpoint();
                        }
                        if let Some(s) = self.st.shape_mut(id) {
                            s.translate(d);
                        }
                        self.drag = EDrag::Move { id, last: p, moved: true };
                    }
                }
                EDrag::Crop { start } => {
                    let full = self.st.full_rect();
                    self.crop_draft = Some(Rect::from_two_pos(start, p).intersect(full));
                }
                EDrag::CropMove { grab } => {
                    if let Some(d) = self.crop_draft {
                        let full = self.st.full_rect();
                        let mut min = p + grab;
                        min.x = min.x.clamp(0.0, full.right() - d.width());
                        min.y = min.y.clamp(0.0, full.bottom() - d.height());
                        self.crop_draft = Some(Rect::from_min_size(min, d.size()));
                    }
                }
                EDrag::None => {}
            }
        }

        if released {
            if let EDrag::Draw { id, .. } = self.drag {
                let min_len = 3.0 / self.zoom;
                let keep = self.st.shapes.iter().find(|s| s.id == id).is_some_and(|s| match &s.kind {
                    Kind::Arrow { a, b } | Kind::Line { a, b } => (*a - *b).length() > min_len,
                    Kind::Rect { r } | Kind::Ellipse { r } | Kind::Pixelate { r } => {
                        r.width() > min_len && r.height() > min_len
                    }
                    _ => true,
                });
                if keep {
                    if let Some(s) = self.st.shape_mut(id)
                        && let Kind::Pen { pts } = &mut s.kind
                    {
                        *pts = doc::smooth(pts);
                    }
                } else {
                    self.st.shapes.retain(|s| s.id != id);
                    self.st.undo.pop();
                }
            }
            if let EDrag::Crop { .. } = self.drag
                && self.crop_draft.is_some_and(|d| d.width() < 2.0 || d.height() < 2.0)
            {
                self.crop_draft = None;
            }
            self.drag = EDrag::None;
        }
    }

    fn text_editor(&mut self, ctx: &egui::Context) {
        let Some(t) = &self.text else { return };
        // Plain Enter commits; Shift+Enter inserts a newline (TextEdit's return key).
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            self.commit_text();
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.text = None;
            return;
        }
        let size = doc::text_size(self.st.width);
        let screen_size = size * self.zoom;
        let pos = self.to_screen(t.pos + doc::text_pad(size));
        let color = doc::contrast_on(self.st.color);
        let plate_color = self.st.color;
        let box_size = render::text_box(if t.buf.is_empty() { "W" } else { &t.buf }, size) * self.zoom;
        let focus = std::mem::take(&mut self.wants_focus_text);
        let t = self.text.as_mut().unwrap();
        let pad = doc::text_pad(size) * self.zoom;
        egui::Area::new(Id::unique("text-edit"))
            .fixed_pos(pos - vec2(2.0, 2.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let plate = Rect::from_min_size(pos - pad, box_size + pad * 2.0 + vec2(12.0, 0.0));
                ui.painter().rect_filled(plate, 0, plate_color);
                ui.painter().extend(dashed_rect(plate.expand(3.0)));
                let out = egui::TextEdit::multiline(&mut t.buf)
                    .font(egui::FontId::new(screen_size, theme::medium()))
                    .text_color(color)
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::same(2))
                    .desired_rows(1)
                    .desired_width(box_size.x + 20.0)
                    .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                    .show(ui);
                if focus {
                    out.response.request_focus();
                }
            });
    }

    fn close_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_close {
            return;
        }
        let modal = egui::Modal::new(Id::unique("confirm-close"))
            .backdrop_color(Color32::from_black_alpha(140))
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, LINE))
                    .corner_radius(3)
                    .inner_margin(egui::Margin::same(18)),
            )
            .show(ctx, |ui| {
                ui.set_width(360.0);
                ui.label(egui::RichText::new(crate::i18n::t("сохранить изменения?")).font(font_medium(15.0)).color(TEXT));
                ui.add_space(2.0);
                ui.label(egui::RichText::new(crate::i18n::t("аннотации будут потеряны, если закрыть без сохранения.")).color(DIM));
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui::button(ui, crate::i18n::t("сохранить"), ui::Kind::Accent).clicked() && self.save(ctx) {
                        self.allow_close = true;
                    }
                    if ui::button(ui, crate::i18n::t("не сохранять"), ui::Kind::Ghost).clicked() {
                        self.allow_close = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui::button(ui, crate::i18n::t("отмена"), ui::Kind::Ghost).clicked() {
                            self.confirm_close = false;
                        }
                    });
                });
            });
        if modal.should_close() && !self.allow_close {
            self.confirm_close = false;
        }
        if self.allow_close {
            self.confirm_close = false;
            if self.shared.peek() == Some(Command::Quit) || self.exit.borrow().is_none() {
                *self.exit.borrow_mut() = Some(EditorExit::Closed);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn dashed_rect(r: Rect) -> Vec<egui::Shape> {
    let pts = vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    egui::Shape::dashed_line(&pts, Stroke::new(1.0, ACCENT), 5.0, 4.0)
}

fn sep(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 30.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.shrink2(vec2(0.0, 6.0)).y_range(), Stroke::new(1.0, LINE));
}

fn tool_icon(t: Tool) -> Icon {
    match t {
        Tool::Select => Icon::Select,
        Tool::Arrow => Icon::Arrow,
        Tool::Line => Icon::Line,
        Tool::Rect => Icon::Rect,
        Tool::Ellipse => Icon::Ellipse,
        Tool::Pen => Icon::Pen,
        Tool::Marker => Icon::Marker,
        Tool::Text => Icon::Text,
        Tool::Pixelate => Icon::Blur,
        Tool::Counter => Icon::Counter,
        Tool::Crop => Icon::Crop,
    }
}

fn kind_name(k: &Kind) -> String {
    match k {
        Kind::Arrow { .. } | Kind::Curve { .. } => crate::i18n::t("стрелка").into(),
        Kind::Line { .. } => crate::i18n::t("линия").into(),
        Kind::Rect { .. } => crate::i18n::t("рамка").into(),
        Kind::Ellipse { .. } => crate::i18n::t("овал").into(),
        Kind::Pen { .. } => crate::i18n::t("карандаш").into(),
        Kind::Marker { .. } => crate::i18n::t("маркер").into(),
        Kind::Text { text, .. } => {
            let t: String = text.lines().next().unwrap_or("").chars().take(18).collect();
            lf!("текст: {t}", "text: {t}")
        }
        Kind::Pixelate { .. } => crate::i18n::t("размытие").into(),
        Kind::Highlight { .. } => crate::i18n::t("выделение").into(),
        Kind::Redact { .. } => crate::i18n::t("скрыто").into(),
        Kind::Label { text, .. } => {
            let t: String = text.lines().next().unwrap_or("").chars().take(18).collect();
            lf!("надпись: {t}", "label: {t}")
        }
        Kind::Counter { n, .. } => lf!("номер {n}", "number {n}"),
    }
}

fn swatch(ui: &mut egui::Ui, c: Color32, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 3, HOVER);
    }
    let chip = Rect::from_center_size(rect.center(), vec2(14.0, 14.0));
    if selected {
        p.rect_stroke(chip.expand(3.0), 2, Stroke::new(1.0, TEXT), StrokeKind::Inside);
    }
    p.rect_filled(chip, 2, c);
    if c.r() < 40 && c.g() < 40 && c.b() < 40 {
        p.rect_stroke(chip, 2, Stroke::new(1.0, LINE), StrokeKind::Inside);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn width_button(ui: &mut egui::Ui, w: f32, idx: usize, px: f32, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
    let p = ui.painter();
    let fill = if on {
        ACTIVE
    } else if resp.hovered() {
        HOVER
    } else {
        CANVAS
    };
    p.rect_filled(rect, 3, fill);
    let fg = if on || resp.hovered() { TEXT } else { DIM };
    let line_h = [1.0, 2.0, 4.0][idx];
    let line = Rect::from_center_size(pos2(rect.center().x - 8.0, rect.center().y), vec2(14.0, line_h));
    p.rect_filled(line, 0, fg);
    p.text(pos2(rect.center().x + 6.0, rect.center().y), Align2::LEFT_CENTER, format!("{}", px as i32), mono(11.0), fg);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

enum RowAction {
    None,
    Select,
    Remove,
}

fn object_row(ui: &mut egui::Ui, s: &Shape, selected: bool) -> RowAction {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 0, ACTIVE);
    } else if resp.hovered() {
        p.rect_filled(rect, 0, HOVER);
    }
    p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, LINE_SOFT));
    let chip = Rect::from_center_size(pos2(rect.left() + 8.0, rect.center().y), vec2(8.0, 8.0));
    p.rect_filled(chip, 1, s.color);
    p.text(pos2(rect.left() + 20.0, rect.center().y), Align2::LEFT_CENTER, kind_name(&s.kind), font(13.0), TEXT);
    let x_rect = Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), vec2(24.0, 24.0));
    let x_resp = ui.interact(x_rect, resp.id.with("x"), Sense::click());
    let x_col = if x_resp.hovered() { TEXT } else { FAINT };
    if resp.hovered() || x_resp.hovered() || selected {
        crate::icons::paint(ui.painter(), x_rect, Icon::Close, 12.0, x_col);
    }
    if x_resp.on_hover_text(crate::i18n::t("удалить")).clicked() {
        return RowAction::Remove;
    }
    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        return RowAction::Select;
    }
    RowAction::None
}

fn menu_item(ui: &mut egui::Ui, icon: Icon, label: &str, key: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(210.0), 28.0), Sense::click());
    let h = resp.hovered();
    if h {
        ui.painter().rect_filled(rect, 3, HOVER);
    }
    let ir = Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(16.0, 16.0));
    crate::icons::paint(ui.painter(), ir, icon, 14.0, if h { ACCENT } else { DIM });
    ui.painter().text(
        pos2(rect.left() + 32.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font(13.0),
        if h { TEXT } else { TEXT2 },
    );
    if !key.is_empty() {
        ui.painter().text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, key, mono(11.0), FAINT);
    }
    resp.clicked()
}

impl eframe::App for EditorApp {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if theme::glass() { [0.0; 4] } else { visuals.panel_fill.to_normalized_gamma_f32() }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame_no = self.frame_no.saturating_add(1);
        if let Some(h) = self.hwnd {
            if self.frame_no == 3 {
                win::cloak(h, false);
                win::force_foreground(h);
            } else if self.frame_no < 3 {
                ctx.request_repaint();
            }
        }

        // Another command (new capture, home...) — step aside and come back later.
        if let Some(cmd) = self.shared.peek() {
            if cmd == Command::Quit {
                if self.st.dirty && !self.allow_close {
                    self.confirm_close = true;
                } else {
                    *self.exit.borrow_mut() = Some(EditorExit::Closed);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.st.dirty && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }

        self.top_bar(ui);
        self.status_bar(ui);
        self.tool_strip(ui);
        self.side_panel(ui);
        self.shortcuts(&ctx);
        egui::CentralPanel::no_frame().show(ui, |ui| {
            if self.beauty_on {
                self.beauty_preview(ui)
            } else {
                self.canvas_ui(ui)
            }
        });
        self.close_dialog(&ctx);
        crate::chrome::frame_border(&ctx);
        if let Some(n) = &self.notice
            && !n.show(&ctx, self.canvas)
        {
            self.notice = None;
        }
    }
}

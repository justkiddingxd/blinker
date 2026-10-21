//! Small custom-painted widgets shared by all windows (Graphite style).

use crate::icons::{self, Icon};
use crate::theme::{self, *};
use eframe::egui::{
    self, Color32, CursorIcon, Rect, Response, Sense, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Ui, Vec2, pos2, vec2,
};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// Yellow fill, dark text: the one primary action on screen.
    Accent,
    /// Transparent until hovered.
    Ghost,
}

fn hover_t(ui: &Ui, resp: &Response) -> f32 {
    ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), 0.08)
}

/// Square icon button. `active` = selected tool (accent icon on a lighter plate).
pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str, size: f32, active: bool, kind: Kind) -> Response {
    icon_button_sized(ui, icon, tip, size, 16.0, active, kind)
}

/// `icon_button` with an icon of `icon_size` points.
pub fn icon_button_sized(ui: &mut Ui, icon: Icon, tip: &str, size: f32, icon_size: f32, active: bool, kind: Kind) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let h = hover_t(ui, &resp);
    let painter = ui.painter();
    let (fill, fg) = match (kind, active) {
        (Kind::Accent, _) => (theme::lerp_color(ACCENT, ACCENT_HI, h), ON_ACCENT),
        (Kind::Ghost, true) => (ACTIVE, ACCENT),
        (Kind::Ghost, false) => (
            theme::lerp_color(Color32::TRANSPARENT, HOVER, h),
            theme::lerp_color(TEXT2, TEXT, h),
        ),
    };
    let fg = if ui.is_enabled() { fg } else { FAINT.gamma_multiply(0.6) };
    painter.rect_filled(rect, (size * 0.12).round() as u8, fill);
    icons::paint(painter, rect, icon, icon_size, fg);
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// Text button, 30px tall.
pub fn button(ui: &mut Ui, label: &str, kind: Kind) -> Response {
    button_h(ui, label, kind, 30.0)
}

pub fn button_h(ui: &mut Ui, label: &str, kind: Kind, height: f32) -> Response {
    // PLACEHOLDER: the color passed to `galley()` below decides.
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font_medium(13.0), Color32::PLACEHOLDER);
    let size = vec2(galley.size().x + 24.0, height);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let h = hover_t(ui, &resp);
    let (fill, fg) = match kind {
        Kind::Accent => (theme::lerp_color(ACCENT, ACCENT_HI, h), ON_ACCENT),
        Kind::Ghost => (theme::lerp_color(Color32::TRANSPARENT, HOVER, h), theme::lerp_color(TEXT2, TEXT, h)),
    };
    ui.painter().rect_filled(rect, 3, fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// One option of a small choice ("png / webp / jpeg"): the chosen one sits
/// on a raised plate.
pub fn filter(ui: &mut Ui, label: &str, on: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font(12.5), Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 22.0, 28.0), Sense::click());
    let h = hover_t(ui, &resp);
    let p = ui.painter();
    if on {
        p.rect_filled(rect, 6, ACTIVE);
        p.rect_stroke(rect, 6, Stroke::new(1.0, LINE_HI), StrokeKind::Inside);
    } else {
        p.rect_filled(rect, 6, theme::lerp_color(Color32::TRANSPARENT, HOVER, h));
    }
    let fg = if on { TEXT } else { theme::lerp_color(DIM, TEXT, h) };
    p.galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// A pill switch.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.14);
    let h = hover_t(ui, &resp);
    let p = ui.painter();
    let track = theme::lerp_color(theme::lerp_color(LINE, LINE_HI, h), ACCENT, t);
    p.rect_filled(rect, 10, track);
    let x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), t);
    p.circle_filled(pos2(x, rect.center().y), 7.0, theme::lerp_color(DIM, ON_ACCENT, t));
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn section_label(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(font_medium(11.0)).color(FAINT));
}

/// Settings row: title + description left, control right, hairline below.
/// A titled card holding setting rows (see `setting_row`).
pub fn group(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    ui.add_space(18.0);
    if !title.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(title).font(font_medium(12.0)).color(DIM));
        });
        ui.add_space(7.0);
    }
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, LINE_SOFT))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(16, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui);
        });
}

/// Hairline between rows of a card (none above the first one).
fn row_separator(ui: &Ui, top: f32) {
    if top > ui.min_rect().top() + 1.0 {
        let r = ui.max_rect();
        ui.painter().hline(r.x_range(), top - 0.5, Stroke::new(1.0, LINE_SOFT));
    }
}

/// Title (+ description) on the left, vertically centered; controls on the
/// right, laid out right to left. The text gets whatever the controls leave
/// (their width is remembered from the previous frame); when that is too
/// little, the controls move under the text.
pub fn setting_row(ui: &mut Ui, title: &str, desc: &str, control: impl FnOnce(&mut Ui)) {
    let top = ui.cursor().top();
    row_separator(ui, top);
    let w = ui.available_width();
    let key = ui.scope_id().with(("setting-row-ctrl", title));
    let (ctrl_w, ctrl_h): (f32, f32) = ui.data(|d| d.get_temp(key)).unwrap_or((w * 0.35, 26.0));
    let stacked = w - ctrl_w - 16.0 < w * 0.42;
    let text_w = if stacked { w } else { (w - ctrl_w - 16.0).max(80.0) };
    let tg = ui.painter().layout(title.to_owned(), font(13.0), TEXT, text_w);
    let dg = (!desc.is_empty()).then(|| ui.painter().layout(desc.to_owned(), font(11.5), FAINT_TEXT, text_w));
    let text_h = tg.size().y + dg.as_ref().map(|g| g.size().y + 2.0).unwrap_or(0.0);
    let ctrl_h = ctrl_h.max(20.0);
    let h = if stacked { text_h + ctrl_h + 24.0 } else { (text_h + 20.0).max(ctrl_h + 16.0).max(46.0) };
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let mut y = if stacked { rect.top() + 7.0 } else { rect.center().y - text_h / 2.0 };
    let th = tg.size().y;
    ui.painter().galley(pos2(rect.left(), y), tg, TEXT);
    y += th + 2.0;
    if let Some(dg) = dg {
        ui.painter().galley(pos2(rect.left(), y), dg, FAINT_TEXT);
    }
    // Controls are top-aligned in a box of their own (remembered) height, the
    // box centered in the row: measuring a center-aligned layout would
    // report the whole row height and the row would keep growing.
    let ctrl = if stacked {
        Rect::from_min_size(pos2(rect.left(), rect.bottom() - ctrl_h - 7.0), vec2(rect.width(), ctrl_h))
    } else {
        let left = rect.left() + text_w + 8.0;
        Rect::from_min_size(pos2(left, rect.center().y - ctrl_h / 2.0), vec2(rect.right() - left, ctrl_h))
    };
    // right to left in both cases, so option order stays the same
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(ctrl).layout(egui::Layout::right_to_left(egui::Align::Min)));
    control(&mut child);
    let used = child.min_rect().size();
    if (used.x - ctrl_w).abs() > 1.0 || (used.y - ctrl_h).abs() > 1.0 {
        ui.data_mut(|d| d.insert_temp(key, (used.x, used.y)));
        ui.ctx().request_repaint();
    }
}

/// Like `setting_row`, with the controls on their own line under the text
/// (for long lists of options).
pub fn setting_block(ui: &mut Ui, title: &str, desc: &str, control: impl FnOnce(&mut Ui)) {
    let top = ui.cursor().top();
    row_separator(ui, top);
    ui.add_space(11.0);
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.label(egui::RichText::new(title).font(font(13.0)).color(TEXT));
    if !desc.is_empty() {
        ui.label(egui::RichText::new(desc).font(font(11.5)).color(FAINT_TEXT));
    }
    ui.add_space(7.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
        control(ui);
    });
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.add_space(11.0);
}

/// Row of the settings sidebar.
pub fn nav_item(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
    let h = hover_t(ui, &resp);
    let p = ui.painter();
    if !selected {
        p.rect_filled(rect, 7, theme::lerp_color(Color32::TRANSPARENT, HOVER, h));
    }
    let icon_col = if selected { ACCENT } else { theme::lerp_color(FAINT, TEXT2, h) };
    let icon_rect = Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), Vec2::splat(15.0));
    icons::paint(p, icon_rect, icon, 15.0, icon_col);
    let fg = if selected { TEXT } else { theme::lerp_color(TEXT2, TEXT, h) };
    let g = p.layout_no_wrap(label.to_owned(), font(13.0), fg);
    p.galley(pos2(rect.left() + 34.0, rect.center().y - g.size().y / 2.0), g, fg);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Caption of a group of rows in the sidebar.
pub fn nav_caption(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(egui::RichText::new(text).font(font_medium(11.0)).color(FAINT));
    });
    ui.add_space(3.0);
}

pub fn cover_uv(container: Vec2, img: Vec2) -> Rect {
    let ca = container.x / container.y;
    let ia = img.x / img.y;
    if ia > ca {
        let w = ca / ia;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = ia / ca;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
    }
}

/// An image too large for a single GPU texture, split into tiles.
pub struct TiledTexture {
    pub tiles: Vec<(TextureHandle, Rect)>,
    pub size: Vec2,
}

impl TiledTexture {
    pub const TILE: u32 = 2048; // safe minimum every GL driver supports

    pub fn new(ctx: &egui::Context, name: &str, img: &image::RgbaImage, opts: TextureOptions) -> Self {
        let (w, h) = img.dimensions();
        let mut tiles = Vec::new();
        let mut y = 0;
        while y < h {
            let th = (h - y).min(Self::TILE);
            let mut x = 0;
            while x < w {
                let tw = (w - x).min(Self::TILE);
                let mut pixels = Vec::with_capacity((tw * th) as usize);
                for row in y..y + th {
                    let start = ((row * w + x) * 4) as usize;
                    let line = &img.as_raw()[start..start + (tw * 4) as usize];
                    pixels.extend(
                        line.chunks_exact(4)
                            .map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])),
                    );
                }
                let ci = egui::ColorImage::new([tw as usize, th as usize], pixels);
                let tex = ctx.load_texture(format!("{name}-{x}-{y}"), ci, opts);
                tiles.push((tex, Rect::from_min_size(pos2(x as f32, y as f32), vec2(tw as f32, th as f32))));
                x += tw;
            }
            y += th;
        }
        Self { tiles, size: vec2(w as f32, h as f32) }
    }

    /// Paint the part of the image given by `src` (image px) into `dst` (points).
    pub fn paint(&self, painter: &egui::Painter, src: Rect, dst: Rect, tint: Color32) {
        let sx = dst.width() / src.width();
        let sy = dst.height() / src.height();
        for (tex, r) in &self.tiles {
            let part = r.intersect(src);
            if part.width() <= 0.0 || part.height() <= 0.0 {
                continue;
            }
            let uv = Rect::from_min_max(
                pos2((part.min.x - r.min.x) / r.width(), (part.min.y - r.min.y) / r.height()),
                pos2((part.max.x - r.min.x) / r.width(), (part.max.y - r.min.y) / r.height()),
            );
            let d = Rect::from_min_max(
                pos2(dst.min.x + (part.min.x - src.min.x) * sx, dst.min.y + (part.min.y - src.min.y) * sy),
                pos2(dst.min.x + (part.max.x - src.min.x) * sx, dst.min.y + (part.max.y - src.min.y) * sy),
            );
            painter.image(tex.id(), d, uv, tint);
        }
    }
}

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Small panel in the bottom-right corner of `anchor` (editor, home).
pub struct Notice {
    text: String,
    at: f64,
}

impl Notice {
    pub fn new(ctx: &egui::Context, text: impl Into<String>) -> Self {
        Self { text: text.into(), at: ctx.input(|i| i.time) }
    }

    /// Returns false when expired.
    pub fn show(&self, ctx: &egui::Context, anchor: Rect) -> bool {
        const LIFE: f64 = 2.0;
        let age = ctx.input(|i| i.time) - self.at;
        if age > LIFE {
            return false;
        }
        let a = (((LIFE - age) / 0.2).clamp(0.0, 1.0) as f32).min(((age / 0.12) as f32).clamp(0.0, 1.0));
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("notice")));
        let galley = painter.layout_no_wrap(self.text.clone(), font_medium(13.0), Color32::PLACEHOLDER);
        let w = (galley.size().x + 28.0).max(240.0);
        let rect = Rect::from_min_size(anchor.right_bottom() - vec2(w + 16.0, 16.0 + 38.0), vec2(w, 38.0));
        painter.rect_filled(rect, 3, PANEL.gamma_multiply(a));
        painter.rect_stroke(rect, 3, Stroke::new(1.0, LINE.gamma_multiply(a)), StrokeKind::Inside);
        painter.galley(
            pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0 - 1.0),
            galley,
            TEXT.gamma_multiply(a),
        );
        let life_left = (1.0 - age / LIFE) as f32;
        let bar = Rect::from_min_max(
            pos2(rect.left() + 1.0, rect.bottom() - 3.0),
            pos2(rect.right() - 1.0, rect.bottom() - 1.0),
        );
        painter.rect_filled(
            Rect::from_min_max(bar.min, pos2(bar.left() + bar.width() * life_left, bar.bottom())),
            0,
            ACCENT.gamma_multiply(0.8 * a),
        );
        ctx.request_repaint();
        true
    }
}

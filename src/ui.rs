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

// On Liquid Glass (macOS 26) the window is frosted glass: plates are light
// veils over it instead of flat greys, and controls are capsules.

/// A raised plate: the chosen section, option or tab.
pub fn plate() -> Color32 {
    if theme::glass() { Color32::from_white_alpha(20) } else { ACTIVE }
}

/// Under the pointer.
pub fn hover_fill() -> Color32 {
    if theme::glass() { Color32::from_white_alpha(11) } else { HOVER }
}

/// A card of settings.
pub fn card_fill() -> Color32 {
    if theme::glass() { Color32::from_white_alpha(8) } else { PANEL }
}

/// Edges of cards and lines between their rows.
pub fn hairline() -> Color32 {
    if theme::glass() { Color32::from_white_alpha(14) } else { LINE_SOFT }
}

/// A sunken well: key caps, fields, small previews.
pub fn well() -> Color32 {
    if theme::glass() { Color32::from_black_alpha(70) } else { CANVAS }
}

/// The edge of a well or a small plate.
pub fn edge() -> Color32 {
    if theme::glass() { Color32::from_white_alpha(24) } else { LINE }
}

/// Corner radius of rows and small plates: rounder on glass.
pub fn soft_radius(flat: u8) -> u8 {
    if theme::glass() { flat.max(8) } else { flat }
}

/// A side panel of a window (tools, properties, a status line): clear on
/// glass, the frosted window shows through.
pub fn pane_fill() -> Color32 {
    if theme::glass() { Color32::TRANSPARENT } else { PANEL }
}

/// The fill of a card floating over a window (a dialog, a note, a popup).
pub fn float_fill() -> Color32 {
    if theme::glass() { Color32::from_rgba_unmultiplied(30, 30, 29, 236) } else { PANEL }
}

/// A card floating over a window: on glass a dark pane with a light rim,
/// round and with a soft shadow; else the flat panel, `flat_radius` round.
pub fn float_frame(flat_radius: u8) -> egui::Frame {
    if theme::glass() {
        egui::Frame::new()
            .fill(float_fill())
            .stroke(Stroke::new(1.0, Color32::from_white_alpha(30)))
            .corner_radius(16)
            .shadow(egui::epaint::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(90) })
    } else {
        egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0, LINE)).corner_radius(flat_radius)
    }
}

/// `float_frame` painted by hand at `rect`, faded by `alpha`.
pub fn paint_float(p: &egui::Painter, rect: Rect, flat_radius: u8, alpha: f32) {
    if theme::glass() {
        let r = 16.min((rect.height() / 2.0) as u8);
        p.add(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha((80.0 * alpha) as u8) }.as_shape(rect, r));
        p.rect_filled(rect, r, float_fill().gamma_multiply(alpha));
        p.rect_stroke(rect, r, Stroke::new(1.0, Color32::from_white_alpha(30).gamma_multiply(alpha)), StrokeKind::Inside);
        // light catching the top edge
        let inset = r as f32;
        p.rect_filled(Rect::from_min_size(rect.min + vec2(inset, 1.0), vec2(rect.width() - 2.0 * inset, 1.0)), 0, Color32::from_white_alpha(26).gamma_multiply(alpha));
    } else {
        p.rect_filled(rect, flat_radius, PANEL.gamma_multiply(alpha));
        p.rect_stroke(rect, flat_radius, Stroke::new(1.0, LINE.gamma_multiply(alpha)), StrokeKind::Inside);
    }
}

/// Corner radius of a control `height` points tall.
pub fn control_radius(height: f32, flat: f32) -> u8 {
    if theme::glass() { (height / 2.0).round() as u8 } else { flat as u8 }
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
        (Kind::Ghost, true) => (plate(), ACCENT),
        (Kind::Ghost, false) => (
            theme::lerp_color(Color32::TRANSPARENT, hover_fill(), h),
            theme::lerp_color(TEXT2, TEXT, h),
        ),
    };
    let fg = if ui.is_enabled() { fg } else { FAINT.gamma_multiply(0.6) };
    // glass: soft squircles; flat: nearly square
    let radius = if theme::glass() { size * 0.3 } else { size * 0.12 };
    painter.rect_filled(rect, radius.round() as u8, fill);
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
    let size = vec2(galley.size().x + if theme::glass() { 28.0 } else { 24.0 }, height);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let h = hover_t(ui, &resp);
    let (fill, fg) = match kind {
        Kind::Accent => (theme::lerp_color(ACCENT, ACCENT_HI, h), ON_ACCENT),
        Kind::Ghost => (theme::lerp_color(Color32::TRANSPARENT, hover_fill(), h), theme::lerp_color(TEXT2, TEXT, h)),
    };
    ui.painter().rect_filled(rect, control_radius(height, 3.0), fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// One option of a small choice ("png / webp / jpeg"): the chosen one sits
/// on a raised plate.
pub fn filter(ui: &mut Ui, label: &str, on: bool) -> Response {
    // The options of one choice sit side by side: its first option this frame
    // keeps a slot under them for the plate, which glides to the chosen one.
    // Options right after the previous one belong to its choice; anything in
    // between (a separator, a button) starts another choice with its own plate.
    let pass = ui.ctx().cumulative_pass_nr();
    let run_key = ui.scope_id().with("filter-run");
    let at = ui.cursor().min;
    let (group, slot) = match ui.data(|d| d.get_temp::<(u64, u32, egui::Pos2, egui::layers::ShapeIdx)>(run_key)) {
        Some((p, g, end, idx)) if p == pass && (at - end).length() < 1.0 => (g, idx),
        last => {
            let g = last.filter(|(p, ..)| *p == pass).map_or(0, |(_, g, ..)| g + 1);
            (g, ui.painter().add(egui::Shape::Noop))
        }
    };
    let key = ui.scope_id().with(("filter-plate", group));
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font(12.5), Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 22.0, 28.0), Sense::click());
    // where the next option of this choice would start
    ui.data_mut(|d| d.insert_temp(run_key, (pass, group, ui.cursor().min, slot)));
    let h = hover_t(ui, &resp);
    let p = ui.painter();
    let radius = control_radius(rect.height(), 6.0);
    if on {
        let x = ui.ctx().animate_value_with_time(key.with("x"), rect.left(), 0.2);
        let w = ui.ctx().animate_value_with_time(key.with("w"), rect.width(), 0.2);
        let plate_rect = Rect::from_min_size(pos2(x, rect.top()), vec2(w, rect.height()));
        let edge = if theme::glass() { Color32::from_white_alpha(26) } else { LINE_HI };
        p.set(
            slot,
            egui::Shape::Vec(vec![
                egui::epaint::RectShape::filled(plate_rect, radius, plate()).into(),
                egui::epaint::RectShape::stroke(plate_rect, radius, Stroke::new(1.0, edge), StrokeKind::Inside).into(),
            ]),
        );
    } else {
        p.rect_filled(rect, radius, theme::lerp_color(Color32::TRANSPARENT, hover_fill(), h));
    }
    // the label brightens as the plate arrives
    let t = ui.ctx().animate_bool_with_time(resp.id.with("on"), on, 0.2);
    let fg = theme::lerp_color(theme::lerp_color(DIM, TEXT, h), TEXT, t);
    p.galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// A slider `ui.spacing().slider_width` wide: a thin track filled up to a
/// round knob (the way macOS 26 draws them). `step` > 0 snaps to it.
pub fn slider(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, step: f32) -> Response {
    let (lo, hi) = (*range.start(), *range.end());
    let (rect, mut resp) = ui.allocate_exact_size(vec2(ui.spacing().slider_width, 26.0), Sense::click_and_drag());
    const KNOB: f32 = 8.0;
    let track = rect.shrink2(vec2(KNOB, 0.0));
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let f = ((p.x - track.left()) / track.width()).clamp(0.0, 1.0);
        let mut v = lo + f * (hi - lo);
        if step > 0.0 {
            v = (v / step).round() * step;
        }
        let v = v.clamp(lo, hi);
        if v != *value {
            *value = v;
            resp.mark_changed();
        }
    }
    let f = if hi > lo { ((*value - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
    let h = hover_t(ui, &resp);
    let on = ui.is_enabled();
    let p = ui.painter();
    let y = rect.center().y;
    let x = track.left() + f * track.width();
    let rail = Rect::from_min_max(pos2(track.left(), y - 2.0), pos2(track.right(), y + 2.0));
    p.rect_filled(rail, 2, if theme::glass() { Color32::from_white_alpha(34) } else { LINE });
    p.rect_filled(Rect::from_min_max(rail.min, pos2(x, rail.max.y)), 2, if on { ACCENT } else { FAINT });
    let r = KNOB + if resp.dragged() { 1.0 } else { 0.0 };
    p.circle_filled(pos2(x, y + 1.0), r + 1.0, Color32::from_black_alpha(70));
    p.circle_filled(pos2(x, y), r, if on { theme::lerp_color(TEXT, ACCENT_HI, h) } else { DIM });
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
        .fill(card_fill())
        .stroke(Stroke::new(1.0, hairline()))
        .corner_radius(if theme::glass() { 14 } else { 10 })
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
        ui.painter().hline(r.x_range(), top - 0.5, Stroke::new(1.0, hairline()));
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
    // a description wraps and wants a fair share of the row; a bare title
    // only its own width
    let need = if desc.is_empty() {
        ui.painter().layout_no_wrap(title.to_owned(), font(13.0), TEXT).size().x + 8.0
    } else {
        w * 0.42
    };
    let stacked = w - ctrl_w - 16.0 < need;
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

/// Row of the settings sidebar.
pub fn nav_item(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
    let h = hover_t(ui, &resp);
    let p = ui.painter();
    if !selected {
        p.rect_filled(rect, control_radius(rect.height(), 7.0), theme::lerp_color(Color32::TRANSPARENT, hover_fill(), h));
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
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(egui::RichText::new(text).font(font_medium(11.0)).color(FAINT));
    });
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
        paint_float(&painter, rect, 3, a);
        painter.galley(
            pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0 - 1.0),
            galley,
            TEXT.gamma_multiply(a),
        );
        let life_left = (1.0 - age / LIFE) as f32;
        // inside the round corners of the glass card
        let (inset, lift) = if theme::glass() { (14.0, 4.0) } else { (1.0, 1.0) };
        let bar = Rect::from_min_max(
            pos2(rect.left() + inset, rect.bottom() - lift - 2.0),
            pos2(rect.right() - inset, rect.bottom() - lift),
        );
        painter.rect_filled(
            Rect::from_min_max(bar.min, pos2(bar.left() + bar.width() * life_left, bar.bottom())),
            if theme::glass() { 1 } else { 0 },
            ACCENT.gamma_multiply(0.8 * a),
        );
        ctx.request_repaint();
        true
    }
}

/// The round "play" mark over a video's picture: a dark disc with a soft
/// rim and a filled, slightly rounded triangle (set a bit right of the
/// middle, where the eye puts its centre).
pub fn play_badge(p: &egui::Painter, c: egui::Pos2, r: f32, alpha: f32) {
    p.circle_filled(c + vec2(0.0, 1.5), r + 1.0, Color32::from_black_alpha((60.0 * alpha) as u8));
    p.circle_filled(c, r, Color32::from_rgba_unmultiplied(14, 14, 13, (185.0 * alpha) as u8));
    p.circle_stroke(c, r, Stroke::new(1.0, Color32::from_white_alpha((34.0 * alpha) as u8)));
    let s = r * 0.42;
    let o = c + vec2(s * 0.18, 0.0);
    let tri = vec![o + vec2(-s * 0.62, -s * 0.8), o + vec2(s * 0.85, 0.0), o + vec2(-s * 0.62, s * 0.8)];
    let white = Color32::from_rgba_unmultiplied(240, 238, 232, (255.0 * alpha) as u8);
    // the stroke in the same color rounds the corners off a little
    p.add(egui::Shape::convex_polygon(tri.clone(), white, Stroke::NONE));
    p.add(egui::Shape::closed_line(tri, Stroke::new(r * 0.12, white)));
}

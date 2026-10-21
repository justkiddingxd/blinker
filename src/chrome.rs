//! Custom window chrome for undecorated windows: draggable title bar,
//! our own minimize / maximize / close buttons and edge resizing.

use crate::icons::{self, Icon};
use crate::theme::*;
use eframe::egui::{self, CursorIcon, Rect, ResizeDirection, Sense, Stroke, Ui, ViewportCommand, pos2, vec2};

pub const TITLE_H: f32 = 40.0;

/// Makes the whole `rect` a drag handle (double-click toggles maximize).
/// Call before adding widgets on top of it so the widgets keep their clicks.
pub fn drag_region(ui: &mut Ui, rect: Rect) {
    let resp = ui.interact(rect, ui.scope_id().with("chrome-drag"), Sense::click_and_drag());
    if resp.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if resp.double_clicked() {
        let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!max));
    }
}

/// Close / maximize / minimize, laid out right-to-left. Use inside a
/// right-to-left layout at the end of the title bar.
pub fn window_buttons(ui: &mut Ui, can_maximize: bool) {
    ui.spacing_mut().item_spacing.x = 0.0;
    let h = ui.available_height().clamp(28.0, TITLE_H);
    if button(ui, h, Glyph::Close, true).on_hover_text(crate::i18n::t("закрыть")).clicked() {
        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
    }
    if can_maximize {
        let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        let tip = if max { crate::i18n::t("восстановить") } else { crate::i18n::t("развернуть") };
        if button(ui, h, if max { Glyph::Restore } else { Glyph::Maximize }, false).on_hover_text(tip).clicked() {
            ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!max));
        }
    }
    if button(ui, h, Glyph::Minimize, false).on_hover_text(crate::i18n::t("свернуть")).clicked() {
        ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

#[derive(Clone, Copy)]
enum Glyph {
    Close,
    Maximize,
    Restore,
    Minimize,
}

fn button(ui: &mut Ui, h: f32, glyph: Glyph, danger: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(44.0, h), Sense::click());
    let p = ui.painter();
    let hovered = resp.hovered();
    if hovered {
        p.rect_filled(rect, 0, if danger { DANGER } else { HOVER });
    }
    let fg = if hovered { TEXT } else { DIM };
    let c = rect.center();
    let s = Stroke::new(1.0, fg);
    match glyph {
        Glyph::Close => icons::paint(p, rect, Icon::Close, 14.0, fg),
        Glyph::Minimize => {
            p.hline((c.x - 5.0)..=(c.x + 5.0), c.y.round() + 0.5, s);
        }
        Glyph::Maximize => {
            p.rect_stroke(Rect::from_center_size(c, vec2(10.0, 10.0)), 0, s, egui::StrokeKind::Inside);
        }
        Glyph::Restore => {
            p.rect_stroke(Rect::from_center_size(c + vec2(-1.5, 1.5), vec2(8.0, 8.0)), 0, s, egui::StrokeKind::Inside);
            p.line(vec![c + vec2(-3.5, -3.5), c + vec2(4.5, -3.5), c + vec2(4.5, 4.5)], s);
            p.line(vec![pos2(c.x - 3.5, c.y - 3.5), pos2(c.x - 3.5, c.y - 2.5)], s);
        }
    }
    resp
}

/// The edge under `pos` (window coordinates), if any.
fn edge_at(ctx: &egui::Context, pos: egui::Pos2) -> Option<ResizeDirection> {
    let r = ctx.content_rect();
    const E: f32 = 5.0;
    let (l, t, rr, b) = (pos.x - r.left() < E, pos.y - r.top() < E, r.right() - pos.x < E, r.bottom() - pos.y < E);
    Some(match (l, t, rr, b) {
        (true, true, _, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, _, true, true) => ResizeDirection::SouthEast,
        (true, _, _, _) => ResizeDirection::West,
        (_, _, true, _) => ResizeDirection::East,
        (_, true, _, _) => ResizeDirection::North,
        (_, _, _, true) => ResizeDirection::South,
        _ => return None,
    })
}

fn resize_cursor(dir: ResizeDirection) -> CursorIcon {
    match dir {
        ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
        ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
    }
}

/// Lets the user resize the undecorated window by its edges. Call once per frame.
///
/// macOS can't hand an edge drag to the system (winit has no
/// `drag_resize_window` there), so the window follows the pointer itself.
#[cfg(target_os = "macos")]
pub fn resize_edges(ctx: &egui::Context) {
    #[derive(Clone, Copy)]
    struct Drag {
        dir: ResizeDirection,
        /// Pointer at the start, in screen points.
        start: egui::Pos2,
        /// Window at the start, in screen points.
        rect: Rect,
    }
    let id = egui::Id::unique("chrome-resize");
    let (outer, maximized) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().maximized.unwrap_or(false)));
    let Some(outer) = outer else { return };
    if maximized {
        return;
    }
    let (pos, pressed, down) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.primary_pressed(), i.pointer.primary_down()));
    if let Some(d) = ctx.data(|m| m.get_temp::<Drag>(id)) {
        if !down {
            ctx.data_mut(|m| m.remove::<Drag>(id));
            return;
        }
        ctx.set_cursor_icon(resize_cursor(d.dir));
        let Some(p) = pos else { return };
        let delta = outer.min + p.to_vec2() - d.start;
        let mut r = d.rect;
        let min = vec2(560.0, 380.0);
        use ResizeDirection::*;
        if matches!(d.dir, West | NorthWest | SouthWest) {
            r.min.x = (r.min.x + delta.x).min(r.max.x - min.x);
        }
        if matches!(d.dir, East | NorthEast | SouthEast) {
            r.max.x = (r.max.x + delta.x).max(r.min.x + min.x);
        }
        if matches!(d.dir, North | NorthWest | NorthEast) {
            r.min.y = (r.min.y + delta.y).min(r.max.y - min.y);
        }
        if matches!(d.dir, South | SouthWest | SouthEast) {
            r.max.y = (r.max.y + delta.y).max(r.min.y + min.y);
        }
        if r != outer {
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(r.min));
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(r.size()));
        }
        return;
    }
    let Some(p) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let Some(dir) = edge_at(ctx, p) else { return };
    ctx.set_cursor_icon(resize_cursor(dir));
    if pressed {
        ctx.data_mut(|m| m.insert_temp(id, Drag { dir, start: outer.min + p.to_vec2(), rect: outer }));
    }
}

/// Lets the user resize the undecorated window by its edges. Call once per frame.
#[cfg(not(target_os = "macos"))]
pub fn resize_edges(ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let Some(dir) = edge_at(ctx, pos) else { return };
    ctx.set_cursor_icon(resize_cursor(dir));
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
    }
}

/// 1px frame around the whole window (undecorated windows have none).
pub fn frame_border(ctx: &egui::Context) {
    // macOS draws its own hairline around the (rounded) window.
    if cfg!(target_os = "macos") || ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("chrome-border")));
    painter.rect_stroke(ctx.content_rect(), 0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
}

/// The mark used in title bars: the logo sparkle.
pub fn mark(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    // rasterized at the screen's pixel size, once per size
    let px = (r.width() * ui.ctx().pixels_per_point()).round() as u32;
    let key = egui::Id::unique(("logo-mark", px));
    let tex = ui.data(|d| d.get_temp::<egui::TextureHandle>(key)).unwrap_or_else(|| {
        let img = crate::theme::color_image(&crate::theme::mark_image(px));
        let t = ui.ctx().load_texture("logo-mark", img, egui::TextureOptions::LINEAR);
        ui.data_mut(|d| d.insert_temp(key, t.clone()));
        t
    });
    let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    ui.painter().image(tex.id(), r, uv, egui::Color32::WHITE);
}


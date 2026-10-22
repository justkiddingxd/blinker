//! Window chrome: a draggable title bar of our own, the system window buttons
//! on macOS and our close / minimize buttons on Windows.

use crate::icons::{self, Icon};
use crate::theme::*;
use eframe::egui::{self, Rect, Sense, Stroke, Ui, ViewportCommand, vec2};

pub const TITLE_H: f32 = 40.0;

/// Width the macOS window buttons (close, minimize, zoom) take at the left
/// of a title bar, with room around them.
pub const MAC_BUTTONS_W: f32 = 78.0;

/// Our app windows have one size, chosen when they open: no resizing, no
/// maximize, no full screen. macOS keeps its own window buttons over our
/// title bar (transparent, the content drawn under it); Windows gets ours.
pub fn style(b: egui::ViewportBuilder) -> egui::ViewportBuilder {
    let b = b.with_resizable(false).with_maximize_button(false);
    if cfg!(target_os = "macos") {
        b.with_decorations(true).with_title_shown(false).with_titlebar_shown(false).with_fullsize_content_view(true)
    } else {
        b.with_decorations(false)
    }
}

/// After the window exists: on macOS the window buttons move to the middle
/// of our title bar of height `bar_h`.
pub fn place_window_buttons(hwnd: crate::win::HWND, bar_h: f32) {
    #[cfg(target_os = "macos")]
    crate::win::window_buttons_at(hwnd, 14.0, bar_h / 2.0);
    #[cfg(not(target_os = "macos"))]
    let _ = (hwnd, bar_h);
}

/// macOS 26: the window as frosted glass and its title bar a pane of clear
/// glass floating at its top (the bar itself is drawn clear over it).
pub fn glass_bar(hwnd: crate::win::HWND, width: f32, bar_h: f32) {
    #[cfg(target_os = "macos")]
    if crate::theme::glass() {
        crate::win::window_backdrop(hwnd, 0x121211, crate::home::GLASS_TINT);
        crate::win::window_glass(hwnd, (6.0, 5.0, width - 12.0, bar_h - 8.0), 16.0, 0);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (hwnd, width, bar_h);
}

/// The fill of a title bar: clear over the glass pane, else the panel color.
pub fn bar_fill() -> egui::Color32 {
    if crate::theme::glass() { egui::Color32::TRANSPARENT } else { crate::theme::PANEL }
}

/// Room for the macOS window buttons at the start of a title bar.
pub fn leading_space(ui: &mut Ui) {
    if cfg!(target_os = "macos") {
        ui.add_space(MAC_BUTTONS_W - ui.spacing().item_spacing.x);
    }
}

/// Makes the whole `rect` a drag handle. Call before adding widgets on top
/// of it so the widgets keep their clicks.
pub fn drag_region(ui: &mut Ui, rect: Rect) {
    let resp = ui.interact(rect, ui.scope_id().with("chrome-drag"), Sense::click_and_drag());
    if resp.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

/// Close and minimize, laid out right-to-left. Use inside a right-to-left
/// layout at the end of the title bar. macOS has its own buttons on the left.
pub fn window_buttons(ui: &mut Ui) {
    if cfg!(target_os = "macos") {
        return;
    }
    ui.spacing_mut().item_spacing.x = 0.0;
    let h = ui.available_height().clamp(28.0, TITLE_H);
    if button(ui, h, Glyph::Close, true).on_hover_text(crate::i18n::t("закрыть")).clicked() {
        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
    }
    if button(ui, h, Glyph::Minimize, false).on_hover_text(crate::i18n::t("свернуть")).clicked() {
        ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

#[derive(Clone, Copy)]
enum Glyph {
    Close,
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
    }
    resp
}

/// 1px frame around the whole window (undecorated windows have none).
pub fn frame_border(ctx: &egui::Context) {
    // macOS draws its own hairline around the (rounded) window.
    if cfg!(target_os = "macos") {
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


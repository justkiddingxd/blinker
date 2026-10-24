//! Main window = settings. Custom chrome, sections on the left.

use crate::chrome;
use crate::config::{self, Config, Corner, HotkeyAction, ImageFormat, SoundKind, ToastSize, TrayAction};
use crate::hotkey;
use crate::i18n::{l, t};
use crate::icons::{self, Icon};
use crate::runtime::{self, HomeTab, Shared};
use crate::theme::{self, *};
use crate::ui::{self, Notice};
use crate::win;
use eframe::egui::{self, CursorIcon, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Runs the settings window (its own process) until it is closed.
pub const TITLE: &str = "blinker · настройки";
/// Named event the tray sets to show an already running settings process.
pub const SHOW_EVENT: &str = "Local\\BlinkerSettingsShow";
/// The settings window has one size.
const SIZE: (f32, f32) = (820.0, 620.0);
/// The strip at the top that drags the window (the window buttons sit in it).
const TITLE_BAR_H: f32 = 48.0;
const NAV_W: f32 = 204.0;
/// How far the glass sidebar (macOS 26) stands off the window's edges.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const GLASS_INSET: f32 = 7.0;
/// How much of the graphite tint lies over the blurred desktop (macOS 26).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const GLASS_TINT: f32 = 0.8;

/// Tab requested by the tray for the next show (read when the event fires).
pub fn request_file() -> std::path::PathBuf {
    config::cache_dir().join("settings-tab")
}

/// Settings window. Closing only hides it: the process stays (idle, trimmed),
/// so reopening is instant and GPU overlays (NVIDIA "Alt+Z") don't treat every
/// open as a new game launch. It exits once the tray is gone.
pub fn run(shared: &Arc<Shared>, tab: HomeTab) {
    let event = win::create_event(SHOW_EVENT);
    let viewport = egui::ViewportBuilder::default()
        .with_title(TITLE)
        .with_icon(theme::icon_data(64))
        .with_inner_size([SIZE.0, SIZE.1])
        // Liquid Glass shows through what egui leaves clear
        .with_transparent(theme::glass());
    let viewport = chrome::style(viewport);
    let shared2 = shared.clone();
    runtime::run_smooth_window(shared, viewport, true, move |cc| {
        let hwnd = win::hwnd_of(cc);
        if let Some(h) = hwnd {
            win::style_borderless(h);
            chrome::place_window_buttons(h, TITLE_BAR_H);
            #[cfg(target_os = "macos")]
            if theme::glass() {
                // the window is frosted glass, the sidebar a pane of clear
                // glass floating in it, like Finder's
                win::window_backdrop(h, 0x161615, GLASS_TINT);
                let (x, y) = (GLASS_INSET, GLASS_INSET);
                win::window_glass(h, (x, y, NAV_W - 2.0 * x, SIZE.1 - 2.0 * y), 18.0, 0);
            }
            win::cloak(h, true);
            win::force_foreground(h);
        }
        let hidden = Arc::new(AtomicBool::new(false));
        let request = Arc::new(Mutex::new(None));
        if let (Some(h), Some(ev)) = (hwnd, event) {
            watch_show_requests(ev, h as isize, hidden.clone(), request.clone(), cc.egui_ctx.clone());
        }
        let replay_min = replay_minutes(shared2.config().replay_secs);
        HomeApp {
            hwnd,
            frame_no: 0,
            hidden,
            request,
            upload_test: None,
            cfg: shared2.config(),
            update: {
                let slot = Arc::new(Mutex::new(None));
                crate::update::watch(&cc.egui_ctx, &slot);
                slot
            },
            check: None,
            install: None,
            speech_install: None,
            speech_error: None,
            ocr_install: None,
            ocr_error: None,
            shared: shared2,
            tab,
            recording: None,
            notice: None,
            failed: config::load_failed_hotkeys(),
            failed_at: 0.0,
            content_rect: Rect::NOTHING,
            replay_min,
            upload_mode: None,
            sharex: None,
            nvidia: win::nvidia_overlay(),
            nvidia_open: false,
            notes: None,
            all_notes: None,
            changelog_open: false,
            mics: None,
            shown: None,
            tray_drag: None,
            page_at: 0.0,
        }
    });
}

/// Waits for "show settings" from the tray; also ends the hidden process
/// once the tray has quit.
fn watch_show_requests(
    event: isize,
    hwnd: isize,
    hidden: Arc<AtomicBool>,
    request: Arc<Mutex<Option<HomeTab>>>,
    ctx: egui::Context,
) {
    std::thread::spawn(move || {
        loop {
            if win::wait_event(event, 2000) {
                let tab = std::fs::read_to_string(request_file()).ok().map(|t| tab_from_arg(t.trim()));
                *request.lock().unwrap() = Some(tab.unwrap_or(HomeTab::Capture));
                let h = hwnd as win::HWND;
                if hidden.load(Ordering::SeqCst) {
                    // Cloaked until fresh frames are drawn (no stale page or white flash).
                    win::cloak(h, true);
                    win::show_window(h, true);
                } else {
                    // open already, maybe minimized or behind others: a
                    // minimized window draws no frames, so not left to `ui`
                    win::raise(h);
                }
                ctx.request_repaint();
            } else if hidden.load(Ordering::SeqCst) && !win::window_class_exists("BlinkerShell") {
                std::process::exit(0);
            }
        }
    });
}

pub fn tab_arg(tab: HomeTab) -> &'static str {
    match tab {
        HomeTab::Capture => "capture",
        HomeTab::Notify => "notify",
        HomeTab::Text => "text",
        HomeTab::Record => "record",
        HomeTab::Replay => "replay",
        HomeTab::Editor => "editor",
        HomeTab::Storage => "storage",
        HomeTab::Upload => "upload",
        HomeTab::Hotkeys => "hotkeys",
        HomeTab::Tray => "tray",
        HomeTab::Integrations => "integrations",
        HomeTab::System => "system",
    }
}

/// The request (`--settings update`, the request file) that opens the
/// settings and installs the new version (`blinker://update`).
pub const UPDATE_ARG: &str = "update";
/// That request came: the settings start the update on their next frame.
pub static UPDATE_ASKED: AtomicBool = AtomicBool::new(false);

pub fn tab_from_arg(arg: &str) -> HomeTab {
    if arg == UPDATE_ARG {
        UPDATE_ASKED.store(true, Ordering::SeqCst);
        return HomeTab::System;
    }
    match arg {
        // the shutter sound lives with the card now
        "notify" | "sound" => HomeTab::Notify,
        "text" => HomeTab::Text,
        "record" => HomeTab::Record,
        "replay" => HomeTab::Replay,
        "editor" => HomeTab::Editor,
        "storage" => HomeTab::Storage,
        "upload" => HomeTab::Upload,
        "hotkeys" => HomeTab::Hotkeys,
        "tray" => HomeTab::Tray,
        "integrations" => HomeTab::Integrations,
        "system" => HomeTab::System,
        _ => HomeTab::Capture,
    }
}

/// Whose keys are being changed: a system-wide hotkey, or an action's key
/// inside a capture.
#[derive(Clone, Copy, PartialEq)]
enum KeyTarget {
    Global(HotkeyAction),
    Capture(config::CaptureKey),
}

impl KeyTarget {
    fn keys(self, cfg: &Config) -> &Vec<String> {
        match self {
            KeyTarget::Global(a) => cfg.bindings(a),
            KeyTarget::Capture(k) => cfg.capture_keys(k),
        }
    }

    fn keys_mut(self, cfg: &mut Config) -> &mut Vec<String> {
        match self {
            KeyTarget::Global(a) => cfg.bindings_mut(a),
            KeyTarget::Capture(k) => cfg.capture_keys_mut(k),
        }
    }

    fn all() -> impl Iterator<Item = KeyTarget> {
        HotkeyAction::ALL.into_iter().map(KeyTarget::Global).chain(config::CaptureKey::ALL.into_iter().map(KeyTarget::Capture))
    }
}

/// Something picked up in the tray menu editor.
#[derive(Clone, Copy, PartialEq)]
enum TrayPiece {
    Item(crate::tray::TrayItem),
    Separator,
}

impl TrayPiece {
    fn icon(self) -> Icon {
        match self {
            TrayPiece::Item(i) => i.icon(),
            TrayPiece::Separator => Icon::Line,
        }
    }

    fn label(self) -> &'static str {
        match self {
            TrayPiece::Item(i) => i.label(),
            TrayPiece::Separator => l("разделитель", "separator"),
        }
    }
}

/// Where it was picked up: a tile (row, place), a row of the list, or the
/// spare pieces.
#[derive(Clone, Copy, PartialEq)]
enum TrayFrom {
    Tile(usize, usize),
    List(usize),
    Spare,
}

/// Where it would land.
#[derive(Clone, Copy, PartialEq)]
enum TrayDrop {
    Tile(usize, usize),
    NewRow,
    List(usize),
    /// Back among the spare pieces: out of the menu.
    Remove,
}

#[derive(Clone, Copy)]
struct TrayDrag {
    piece: TrayPiece,
    from: TrayFrom,
}

/// `drag`'s piece moved to `to` in `menu` (rows of tiles left empty go).
fn tray_move(menu: &mut crate::tray::TrayMenu, drag: TrayDrag, to: TrayDrop) {
    use crate::tray::{Entry, TileRow};
    let mut to = to;
    match drag.from {
        TrayFrom::Tile(r, i) => {
            menu.tiles[r].items.remove(i);
            if let TrayDrop::Tile(tr, ti) = to
                && tr == r
                && ti > i
            {
                to = TrayDrop::Tile(tr, ti - 1);
            }
        }
        TrayFrom::List(i) => {
            menu.list.remove(i);
            if let TrayDrop::List(ti) = to
                && ti > i
            {
                to = TrayDrop::List(ti - 1);
            }
        }
        TrayFrom::Spare => {}
    }
    match (to, drag.piece) {
        (TrayDrop::Tile(r, i), TrayPiece::Item(item)) => {
            let row = &mut menu.tiles[r].items;
            row.insert(i.min(row.len()), item);
        }
        (TrayDrop::NewRow, TrayPiece::Item(item)) => menu.tiles.push(TileRow { title: String::new(), items: vec![item] }),
        (TrayDrop::List(i), piece) => {
            let entry = match piece {
                TrayPiece::Item(item) => Entry::Item(item),
                TrayPiece::Separator => Entry::Separator,
            };
            menu.list.insert(i.min(menu.list.len()), entry);
        }
        _ => {}
    }
    menu.tiles.retain(|r| !r.items.is_empty());
}

impl HomeApp {
    /// The tray menu drawn as it opens, and beside it what else can go in:
    /// pieces are dragged between the tiles, the list and the spare ones.
    fn tray_editor(&mut self, ui: &mut egui::Ui) {
        use crate::tray::{Entry, TileRow, TrayItem, TrayMenu};
        const TILE: f32 = 56.0;
        const GAP: f32 = 4.0;
        const PAD: f32 = 10.0;
        const ROW_H: f32 = 30.0;
        const SEP_H: f32 = 11.0;
        const HEAD_H: f32 = 18.0;
        const NEW_ROW_H: f32 = 30.0;
        let menu = self.cfg.tray_menu.clone();
        let drag = self.tray_drag;
        let pointer = ui.ctx().pointer_latest_pos();
        let released = ui.input(|i| i.pointer.primary_released() || !i.pointer.primary_down());

        // the menu's size, the spare pieces laid out beside it
        let menu_w = PAD * 2.0 + TileRow::MAX as f32 * TILE + (TileRow::MAX - 1) as f32 * GAP;
        let list_h: f32 = menu.list.iter().map(|e| if *e == Entry::Separator { SEP_H } else { ROW_H }).sum();
        let menu_h = PAD + menu.tiles.len() as f32 * (HEAD_H + TILE + 8.0) + NEW_ROW_H + 8.0 + SEP_H + list_h + PAD;
        let width = ui.available_width();
        let spare_w = (width - 16.0 - menu_w - 20.0 - 16.0).max(140.0);
        let p = ui.painter().clone();
        let spare: Vec<TrayPiece> = TrayItem::ALL.into_iter().filter(|i| !menu.has(*i)).map(TrayPiece::Item).chain([TrayPiece::Separator]).collect();
        let mut chips = Vec::new();
        let (mut cx, mut cy) = (0.0f32, 26.0f32);
        for piece in &spare {
            let g = p.layout_no_wrap(piece.label().to_string(), font(12.0), TEXT2);
            let w = 34.0 + g.size().x;
            if cx + w > spare_w && cx > 0.0 {
                cx = 0.0;
                cy += 34.0;
            }
            chips.push((Rect::from_min_size(pos2(cx, cy), vec2(w, 28.0)), *piece, g));
            cx += w + 6.0;
        }
        // under them: room for a note, and the reset at the bottom
        let spare_h = cy + 28.0 + 30.0 + 40.0;
        let (area, _) = ui.allocate_exact_size(vec2(width, menu_h.max(spare_h)), Sense::hover());
        let panel = Rect::from_min_size(area.min + vec2(16.0, 0.0), vec2(menu_w, menu_h));
        let spare_zone = Rect::from_min_max(pos2(panel.right() + 20.0, area.top()), pos2(area.right() - 16.0, area.bottom()));

        let mut start = None;
        let picked = |from: TrayFrom| drag.is_some_and(|d| d.from == from);
        let ctx = ui.ctx().clone();
        let mut grab = |resp: &egui::Response, piece: TrayPiece, from: TrayFrom| {
            if resp.drag_started() {
                start = Some(TrayDrag { piece, from });
            }
            let hot = resp.hovered() && drag.is_none();
            if hot {
                ctx.set_cursor_icon(CursorIcon::Grab);
            }
            hot
        };

        // the menu, as dark as the real one; on glass (macOS 26) a pane of
        // dark glass with a light rim, as the menu bar's menu is there
        let glass = theme::glass();
        let (rim, line) = if glass { (egui::Color32::from_white_alpha(34), egui::Color32::from_white_alpha(22)) } else { (LINE_HI, LINE) };
        if glass {
            ui::paint_float(&p, panel, 10, 1.0);
        } else {
            p.rect_filled(panel, 10, BG);
            p.rect_stroke(panel, 10, Stroke::new(1.0, rim), StrokeKind::Inside);
        }
        let mut y = panel.top() + PAD;
        let mut rows: Vec<(Rect, Vec<Rect>)> = Vec::new();
        let mut titles = Vec::new();
        for (r, row) in menu.tiles.iter().enumerate() {
            // the heading, changed in place
            let head = Rect::from_min_size(pos2(panel.left() + PAD + 2.0, y), vec2(menu_w - 2.0 * PAD - 2.0, HEAD_H - 2.0));
            let mut text = row.heading().to_string();
            let edit = egui::TextEdit::singleline(&mut text)
                .id_salt(("tray-head", r))
                .frame(egui::Frame::NONE)
                .font(font_medium(11.0))
                .text_color(DIM)
                .hint_text(l("без заголовка", "no heading"));
            if ui.put(head, edit).changed() {
                titles.push((r, text));
            }
            y += HEAD_H;
            let mut rects = Vec::new();
            for (i, item) in row.items.iter().enumerate() {
                let rect = Rect::from_min_size(pos2(panel.left() + PAD + i as f32 * (TILE + GAP), y), egui::Vec2::splat(TILE));
                rects.push(rect);
                let from = TrayFrom::Tile(r, i);
                let resp = ui.interact(rect, ui.scope_id().with(("tray-tile", r, i)), Sense::drag());
                let hot = grab(&resp, TrayPiece::Item(*item), from);
                let a = if picked(from) { 0.25 } else { 1.0 };
                p.rect_filled(rect, 8, if hot { ui::hover_fill().gamma_multiply(2.2) } else { ui::hover_fill() }.gamma_multiply(a));
                icons::paint(&p, Rect::from_center_size(rect.center() - vec2(0.0, 8.0), egui::Vec2::splat(18.0)), item.icon(), 16.0, TEXT2.gamma_multiply(a));
                p.text(pos2(rect.center().x, rect.bottom() - 12.0), egui::Align2::CENTER_CENTER, item.tile_label(), font(11.0), DIM.gamma_multiply(a));
            }
            rows.push((Rect::from_min_max(pos2(panel.left(), y - 4.0), pos2(panel.right(), y + TILE + 4.0)), rects));
            y += TILE + 8.0;
        }
        // room for one more row of tiles
        let new_row = Rect::from_min_size(pos2(panel.left() + PAD, y), vec2(menu_w - 2.0 * PAD, NEW_ROW_H));
        let carrying_item = drag.is_some_and(|d| matches!(d.piece, TrayPiece::Item(_)));
        let edge = if carrying_item { DIM } else { rim };
        p.extend(egui::Shape::dashed_line(
            &[new_row.left_top(), new_row.right_top(), new_row.right_bottom(), new_row.left_bottom(), new_row.left_top()],
            Stroke::new(1.0, edge),
            4.0,
            3.0,
        ));
        p.text(new_row.center(), egui::Align2::CENTER_CENTER, l("+ новый ряд плиток", "+ a new row of tiles"), font(11.5), if carrying_item { DIM } else { FAINT });
        y += NEW_ROW_H + 8.0;
        p.hline(panel.x_range().shrink(PAD), y + SEP_H / 2.0, Stroke::new(1.0, line));
        y += SEP_H;
        // the list
        let list_top = y;
        let mut list = Vec::new();
        for (i, e) in menu.list.iter().enumerate() {
            let h = if *e == Entry::Separator { SEP_H } else { ROW_H };
            let rect = Rect::from_min_size(pos2(panel.left() + 4.0, y), vec2(menu_w - 8.0, h));
            list.push(rect);
            let from = TrayFrom::List(i);
            let piece = match e {
                Entry::Item(item) => TrayPiece::Item(*item),
                Entry::Separator => TrayPiece::Separator,
            };
            let resp = ui.interact(rect, ui.scope_id().with(("tray-row", i)), Sense::drag());
            let hot = grab(&resp, piece, from);
            let a = if picked(from) { 0.25 } else { 1.0 };
            if hot {
                p.rect_filled(rect, 6, ui::hover_fill());
            }
            match e {
                Entry::Separator => {
                    p.hline(rect.x_range().shrink(8.0), rect.center().y, Stroke::new(1.0, if hot { DIM } else { rim }.gamma_multiply(a)));
                }
                Entry::Item(item) => {
                    // the replay's row shows only while the replay is on
                    let off = *item == TrayItem::Replay && !self.cfg.replay_enabled;
                    icons::paint(&p, Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), egui::Vec2::splat(15.0)), item.icon(), 14.0, if off { FAINT } else { TEXT2 }.gamma_multiply(a));
                    let label = if off { lf!("{} · при включённом повторе", "{} · with the replay on", item.label()) } else { item.label().to_string() };
                    p.text(pos2(rect.left() + 34.0, rect.center().y), egui::Align2::LEFT_CENTER, label, font(12.5), if off { FAINT } else { TEXT }.gamma_multiply(a));
                    let key = item.hotkey().map(|k| crate::hotkey::display_parts(self.cfg.primary_binding(k)).join(" ")).unwrap_or_default();
                    if !off && !key.is_empty() {
                        p.text(pos2(rect.right() - 10.0, rect.center().y), egui::Align2::RIGHT_CENTER, key, mono(10.5), FAINT.gamma_multiply(a));
                    }
                }
            }
            y += h;
        }

        // the spare pieces
        p.text(spare_zone.left_top(), egui::Align2::LEFT_TOP, l("ещё можно добавить", "can be added"), font_medium(12.0), DIM);
        for (rel, piece, g) in chips {
            let rect = rel.translate(spare_zone.min.to_vec2());
            let resp = ui.interact(rect, ui.scope_id().with(("tray-spare", piece.label())), Sense::drag());
            let hot = grab(&resp, piece, TrayFrom::Spare);
            p.rect_filled(rect, 14, if hot { ui::hover_fill().gamma_multiply(2.2) } else { ui::hover_fill() });
            p.rect_stroke(rect, 14, Stroke::new(1.0, line), StrokeKind::Inside);
            icons::paint(&p, Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), egui::Vec2::splat(13.0)), piece.icon(), 12.0, DIM);
            p.galley(pos2(rect.left() + 26.0, rect.center().y - g.size().y / 2.0), g, TEXT2);
        }

        // where it would land
        let target = drag.zip(pointer).and_then(|(d, pt)| {
            let item = match d.piece {
                TrayPiece::Item(i) => Some(i),
                TrayPiece::Separator => None,
            };
            if spare_zone.contains(pt) {
                let can = d.from != TrayFrom::Spare && !item.is_some_and(|i| i.locked());
                return can.then_some(TrayDrop::Remove);
            }
            if !panel.expand(14.0).contains(pt) {
                return None;
            }
            if item.is_some() {
                for (r, (band, rects)) in rows.iter().enumerate() {
                    if band.contains(pt) {
                        let same_row = matches!(d.from, TrayFrom::Tile(sr, _) if sr == r);
                        if rects.len() >= TileRow::MAX && !same_row {
                            return None;
                        }
                        return Some(TrayDrop::Tile(r, rects.iter().filter(|t| t.center().x < pt.x).count()));
                    }
                }
                if new_row.expand(5.0).contains(pt) {
                    return Some(TrayDrop::NewRow);
                }
            }
            (pt.y >= list_top - 8.0).then(|| TrayDrop::List(list.iter().filter(|r| r.center().y < pt.y).count()))
        });
        let fg = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("tray-drop")));
        match target {
            Some(TrayDrop::Tile(r, i)) => {
                let (band, rects) = &rows[r];
                let x = match (rects.get(i), rects.last()) {
                    (Some(t), _) => t.left() - GAP / 2.0,
                    (None, Some(last)) => last.right() + GAP / 2.0,
                    (None, None) => panel.left() + PAD,
                };
                fg.vline(x, band.y_range().shrink(4.0), Stroke::new(2.0, ACCENT));
            }
            Some(TrayDrop::NewRow) => {
                fg.rect_filled(new_row, 6, ACCENT.gamma_multiply(0.1));
                fg.rect_stroke(new_row, 6, Stroke::new(1.0, ACCENT), StrokeKind::Inside);
            }
            Some(TrayDrop::List(i)) => {
                let y = match (list.get(i), list.last()) {
                    (Some(r), _) => r.top(),
                    (None, Some(last)) => last.bottom(),
                    (None, None) => list_top,
                };
                fg.hline(panel.x_range().shrink(8.0), y, Stroke::new(2.0, ACCENT));
            }
            Some(TrayDrop::Remove) => {
                fg.rect_stroke(spare_zone.expand(6.0), 10, Stroke::new(1.0, ACCENT.gamma_multiply(0.7)), StrokeKind::Inside);
            }
            None => {}
        }
        // settings and quit don't leave
        if let (Some(d), Some(pt)) = (drag, pointer)
            && spare_zone.contains(pt)
            && matches!(d.piece, TrayPiece::Item(i) if i.locked())
        {
            p.text(pos2(spare_zone.left(), spare_zone.top() + cy + 28.0 + 16.0), egui::Align2::LEFT_CENTER, l("это убрать нельзя", "this one stays"), font(12.0), DANGER);
        }
        // back to the menu blinker comes with
        let reset = Rect::from_min_max(pos2(spare_zone.left(), spare_zone.bottom() - 30.0), spare_zone.right_bottom());
        let mut row = ui.new_child(egui::UiBuilder::new().max_rect(reset).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let changed = self.cfg.tray_menu != TrayMenu::default();
        row.add_enabled_ui(changed && drag.is_none(), |ui| {
            if ui::button(ui, l("сбросить", "reset"), ui::Kind::Ghost)
                .on_hover_text(l("вернуть меню, как было в blinker", "back to blinker's own menu"))
                .clicked()
            {
                self.cfg.tray_menu = TrayMenu::default();
            }
        });
        // the piece in hand follows the pointer
        if let (Some(d), Some(pt)) = (drag, pointer) {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            let tip = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("tray-ghost")));
            let g = tip.layout_no_wrap(d.piece.label().to_string(), font(12.0), TEXT);
            let r = Rect::from_min_size(pt + vec2(12.0, 10.0), vec2(34.0 + g.size().x, 28.0));
            tip.add(egui::epaint::Shadow { offset: [0, 4], blur: 14, spread: 0, color: egui::Color32::from_black_alpha(120) }.as_shape(r, 14));
            tip.rect_filled(r, 14, ui::float_fill());
            tip.rect_stroke(r, 14, Stroke::new(1.0, ACCENT.gamma_multiply(0.6)), StrokeKind::Inside);
            icons::paint(&tip, Rect::from_center_size(pos2(r.left() + 15.0, r.center().y), egui::Vec2::splat(13.0)), d.piece.icon(), 12.0, ACCENT);
            tip.galley(pos2(r.left() + 26.0, r.center().y - g.size().y / 2.0), g, TEXT);
        }

        for (r, text) in titles {
            if let Some(row) = self.cfg.tray_menu.tiles.get_mut(r) {
                row.title = text;
            }
        }
        if released {
            // let go: it lands (or goes back where it was)
            if let (Some(d), Some(t)) = (drag, target) {
                tray_move(&mut self.cfg.tray_menu, d, t);
            }
            self.tray_drag = None;
        } else if let Some(s) = start {
            self.tray_drag = Some(s);
        }
    }
}

/// Size of everything under `dir` (date / pics / videos subfolders included).
/// A binding being recorded: which action, and which slot (None = new one).
#[derive(Clone, Copy, PartialEq)]
struct Recording {
    action: KeyTarget,
    index: Option<usize>,
    started: f64,
}

struct HomeApp {
    hwnd: Option<win::HWND>,
    /// Frames since the window was (re)shown; it is revealed on frame 3.
    frame_no: u32,
    /// Closed = hidden, waiting for the next show request.
    hidden: Arc<AtomicBool>,
    request: Arc<Mutex<Option<HomeTab>>>,
    upload_test: Option<Arc<Mutex<Option<Result<String, String>>>>>,
    shared: Arc<Shared>,
    cfg: Config,
    tab: HomeTab,
    recording: Option<Recording>,
    /// Bindings the tray process couldn't register (refreshed from disk).
    failed: std::collections::HashSet<String>,
    failed_at: f64,
    notice: Option<Notice>,
    /// A newer version on GitHub, once known.
    update: Arc<Mutex<Option<String>>>,
    /// An update being installed (after "update blinker").
    install: Option<Arc<crate::update::Install>>,
    /// A speech model downloading (subtitles from speech).
    speech_install: Option<(crate::vedit::speech::Model, Arc<crate::vedit::speech::Job>)>,
    speech_error: Option<String>,
    /// The text model coming down (Windows).
    ocr_install: Option<Arc<crate::paddle::Install>>,
    ocr_error: Option<String>,
    /// "Check for updates" being asked or answered.
    check: Option<Arc<Mutex<crate::update::Check>>>,
    content_rect: Rect,
    /// The replay length field, in minutes, as typed.
    replay_min: String,
    /// Where uploads go, as picked on the page (empty fields still mean "own").
    upload_mode: Option<UploadMode>,
    /// ShareX settings on this computer (looked up once).
    sharex: Option<Option<crate::sharex::Source>>,
    /// The NVIDIA overlay is installed (looked up once).
    nvidia: bool,
    /// The steps to quiet it are open (in "system").
    nvidia_open: bool,
    /// This version's notes, read once.
    notes: Option<Vec<Note>>,
    /// Every version's notes, for the changelog.
    all_notes: Option<Vec<Note>>,
    /// The changelog is open (a click on the version).
    changelog_open: bool,
    /// The microphones there are (read when the page opens).
    mics: Option<Vec<(String, String)>>,
    /// A piece of the tray menu being dragged in its editor.
    tray_drag: Option<TrayDrag>,
    /// Page currently on screen and when it started appearing.
    shown: Option<HomeTab>,
    page_at: f64,
}

impl HomeApp {
    /// The sidebar, the whole height of the window: the window buttons and
    /// the name at the top, sections in groups, the version at the bottom.
    fn nav(&mut self, ui: &mut egui::Ui) {
        let glass = theme::glass();
        // on glass the sidebar is clear: the pane behind shows through
        let fill = if glass { egui::Color32::TRANSPARENT } else { PANEL };
        egui::Panel::left("nav")
            .exact_size(NAV_W)
            .resizable(false)
            // the glass pane is its own edge; the flat look draws its own line below
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(fill).inner_margin(egui::Margin { left: 12, right: 12, top: 0, bottom: 14 }))
            .show(ui, |ui| {
                let r = ui.max_rect();
                if !glass {
                    ui.painter().vline(r.right() + 12.5, r.y_range().expand(14.0), Stroke::new(1.0, LINE_SOFT));
                }
                let top = Rect::from_min_size(pos2(r.left() - 10.0, r.top()), vec2(NAV_W, TITLE_BAR_H));
                chrome::drag_region(ui, top);
                // the name: in the title strip on Windows, under the window
                // buttons on a Mac
                if cfg!(target_os = "macos") {
                    ui.add_space(TITLE_BAR_H - 4.0);
                }
                let h = if cfg!(target_os = "macos") { 36.0 } else { TITLE_BAR_H };
                ui.allocate_ui_with_layout(vec2(r.width(), h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 9.0;
                    ui.add_space(6.0);
                    brand_mark(ui, 30.0);
                    ui.label(egui::RichText::new("blinker").font(font_semibold(17.0)).color(TEXT));
                });
                // the footer keeps its place at the bottom; the sections
                // scroll above it when the window is too short for them
                egui::Panel::bottom("nav-footer")
                    .resizable(false)
                    .show_separator_line(false)
                    .frame(egui::Frame::new().fill(egui::Color32::TRANSPARENT).inner_margin(egui::Margin { left: 0, right: 0, top: 10, bottom: 0 }))
                    .show(ui, |ui| self.nav_footer(ui));
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.nav_sections(ui));
            });
    }

    /// The sections of the sidebar, in groups, with the plate under the open one.
    fn nav_sections(&mut self, ui: &mut egui::Ui) {
                ui.spacing_mut().item_spacing.y = 2.0;
                let plate = ui.painter().add(egui::Shape::Noop);
                let mut selected_rect = None;
                let groups: [(&str, &[(HomeTab, Icon, &str)]); 3] = [
                    (
                        l("снимки", "screenshots"),
                        &[(HomeTab::Capture, Icon::Region, t("захват")), (HomeTab::Text, Icon::Scan, l("текст и qr-коды", "text and qr codes"))],
                    ),
                    (
                        l("видео", "video"),
                        &[
                            (HomeTab::Record, Icon::Record, t("запись")),
                            (HomeTab::Replay, Icon::Repeat, t("повтор")),
                            (HomeTab::Editor, Icon::Cut, l("редактор видео", "video editor")),
                        ],
                    ),
                    (
                        l("общее", "general"),
                        &[
                            (HomeTab::Notify, Icon::Bell, t("уведомление")),
                            (HomeTab::Storage, Icon::Save, t("сохранение")),
                            (HomeTab::Upload, Icon::Upload, t("загрузка")),
                            (HomeTab::Hotkeys, Icon::Keys, t("горячие клавиши")),
                            (HomeTab::Tray, Icon::Menu, l("меню трея", "tray menu")),
                            (HomeTab::Integrations, Icon::Link, l("интеграции", "integrations")),
                            (HomeTab::System, Icon::Settings, t("система")),
                        ],
                    ),
                ];
                for (caption, items) in groups {
                    ui::nav_caption(ui, caption);
                    for &(tab, icon, label) in items {
                        let resp = ui::nav_item(ui, icon, label, self.tab == tab);
                        // a section just opened (from the tray, say): bring it into view
                        if self.tab == tab && self.shown != Some(tab) {
                            resp.scroll_to_me(None);
                        }
                        if resp.clicked() {
                            self.tab = tab;
                        }
                        if self.tab == tab {
                            selected_rect = Some(resp.rect);
                        }
                    }
                }
                // The selection plate glides between sections.
                if let Some(r) = selected_rect {
                    let y = ui.ctx().animate_value_with_time(egui::Id::unique("nav-plate"), r.top(), 0.16);
                    let rect = Rect::from_min_size(pos2(r.left(), y), r.size());
                    ui.painter().set(plate, egui::Shape::rect_filled(rect, ui::control_radius(rect.height(), 7.0), ui::plate()));
                }
    }

    /// Bottom of the sections: the version, a link to GitHub and, when a
    /// newer release is out, the way to it (checking by hand is on the
    /// "system" page).
    fn nav_footer(&mut self, ui: &mut egui::Ui) {
        let newer = self.update.lock().unwrap().clone();
        ui.spacing_mut().item_spacing.y = 8.0;
        use crate::update::Stage;
        let stage = self.install.as_ref().map(|j| j.stage());
        let note = |ui: &mut egui::Ui, text: String, color: egui::Color32| {
            ui.add(egui::Label::new(egui::RichText::new(text).font(mono(10.5)).color(color)).wrap());
        };
        match (stage, newer) {
            (Some(Stage::Downloading(done, total)), _) => {
                let f = if total > 0 { (done as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 };
                let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width() - 8.0, 3.0), Sense::hover());
                let bar = bar.translate(vec2(8.0, 0.0));
                ui.painter().rect_filled(bar, 1.5, LINE);
                ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * f, bar.height())), 1.5, ACCENT);
                note(ui, lf!("  загрузка {:.0}%", "  downloading {:.0}%", f * 100.0), DIM);
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
            }
            (Some(Stage::Installing), _) => note(ui, format!("  {}", l("установка…", "installing…")), DIM),
            (Some(Stage::Restarting), _) => note(ui, format!("  {}", l("перезапуск…", "restarting…")), DIM),
            (Some(Stage::Failed(e)), _) if e == crate::update::LATEST => {
                note(ui, format!("  {}", l("у тебя последняя версия", "this is the latest version")), DIM);
            }
            (Some(Stage::Failed(e)), _) => {
                note(ui, format!("  {e}"), DANGER);
                if ui::button_h(ui, l("скачать с сайта", "get it from the site"), ui::Kind::Ghost, 26.0).clicked() {
                    win::open_url(crate::update::SITE_URL);
                }
            }
            (None, Some(v)) => {
                let tip = l("скачать и перезапустить", "download it and restart");
                if update_row(ui, &v).on_hover_text(tip).clicked() {
                    self.install = Some(crate::update::install(ui.ctx()));
                }
            }
            (None, None) => {}
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add_space(8.0);
            // the version opens the changelog
            let version = ui.painter().layout_no_wrap(format!("blinker {}", crate::update::current()), mono(10.5), egui::Color32::PLACEHOLDER);
            let (rect, resp) = ui.allocate_exact_size(version.size(), Sense::click());
            let col = if resp.hovered() { TEXT } else { FAINT };
            ui.painter().galley(rect.min, version, col);
            if resp.hovered() {
                ui.painter().hline(rect.x_range(), rect.bottom() + 0.5, Stroke::new(1.0, col));
            }
            if resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(l("история версий", "release notes")).clicked() {
                self.changelog_open = true;
            }
        });
        // the bottom line: where blinker lives
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.add_space(8.0);
            if brand_link(ui, crate::brand::Mark::GitHub, "github", crate::update::REPO_URL).clicked() {
                win::open_url(crate::update::REPO_URL);
            }
            if brand_link(ui, crate::brand::Mark::Discord, "discord", l("сервер blinker в discord", "blinker's discord server")).clicked() {
                crate::discord::open_invite();
            }
        });
    }

    fn content(&mut self, ui: &mut egui::Ui) {
        let before = self.cfg.clone();
        let now = ui.ctx().input(|i| i.time);
        if self.shown != Some(self.tab) {
            self.shown = Some(self.tab);
            self.page_at = now;
        }
        let t = ui::ease_out_cubic(((now - self.page_at) / 0.24) as f32);
        if t < 1.0 {
            ui.ctx().request_repaint();
        }
        // on glass the page is clear: the frosted window shows through
        let fill = if theme::glass() { egui::Color32::TRANSPARENT } else { BG };
        egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(fill)).show(ui, |ui| {
            self.content_rect = ui.max_rect();
            let r = ui.max_rect();
            // the top strip drags the window; Windows has its buttons there
            chrome::drag_region(ui, Rect::from_min_size(r.min, vec2(r.width(), TITLE_BAR_H)));
            let mut buttons = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_size(r.min, vec2(r.width(), 40.0)))
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
            );
            chrome::window_buttons(&mut buttons);
            // Windows: the pages scroll below the strip with the window
            // buttons instead of over them
            let strip = if cfg!(target_os = "macos") { 0.0 } else { 40.0 };
            let mut page = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(r.min + vec2(0.0, strip), r.max)));
            let ui = &mut page;
            ui.set_opacity(t);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let top = if strip > 0.0 { 0 } else { 26 };
                egui::Frame::new().inner_margin(egui::Margin { left: 32, right: 32, top, bottom: 28 }).show(ui, |ui| {
                    ui.set_max_width(600.0f32.min(ui.available_width()));
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.add_space((1.0 - t) * 14.0);
                    self.nvidia_note(ui);
                    match self.tab {
                        HomeTab::Capture => self.capture_page(ui),
                        HomeTab::Notify => self.notify_page(ui),
                        HomeTab::Text => self.text_page(ui),
                        HomeTab::Record => self.record_page(ui),
                        HomeTab::Replay => self.replay_page(ui),
                        HomeTab::Editor => self.editor_page(ui),
                        HomeTab::Storage => self.storage_page(ui),
                        HomeTab::Upload => self.upload_page(ui),
                        HomeTab::Hotkeys => self.hotkeys_page(ui),
                        HomeTab::Tray => self.tray_page(ui),
                        HomeTab::Integrations => self.integrations_page(ui),
                        HomeTab::System => self.system_page(ui),
                    }
                });
            });
        });
        self.apply_config(before);
    }

    /// Once, with the NVIDIA overlay installed: why "Press Alt+Z to open
    /// NVIDIA Overlay" pops up with blinker, and the way to the steps.
    fn nvidia_note(&mut self, ui: &mut egui::Ui) {
        if !self.nvidia || self.cfg.nvidia_tip_seen {
            return;
        }
        let mut go = false;
        let mut done = false;
        egui::Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, ACCENT.gamma_multiply(0.45)))
            .corner_radius(10)
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new(l("видишь «press alt+z to open nvidia overlay»?", "seeing “press alt+z to open nvidia overlay”?")).font(font_semibold(13.5)).color(TEXT));
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(l(
                        "это оверлей nvidia: он считает игрой всё, что рисует через видеокарту и не стоит в его списке. discord и браузеры nvidia вписала туда сама, а программа вписаться не может. надпись выключается в настройках оверлея за 10 секунд",
                        "that's the nvidia overlay: it takes for a game anything drawn on the graphics card that isn't on its list. nvidia put discord and browsers there itself; an app can't. the note is turned off in the overlay's settings in 10 seconds",
                    ))
                    .font(font(12.5))
                    .color(DIM),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    go = ui::button(ui, l("как убрать", "how to"), ui::Kind::Accent).clicked();
                    done = ui::button(ui, l("понятно", "got it"), ui::Kind::Ghost).clicked();
                });
            });
        ui.add_space(14.0);
        if go {
            self.tab = HomeTab::Integrations;
            self.nvidia_open = true;
        }
        if go || done {
            self.cfg.nvidia_tip_seen = true;
        }
    }

    fn page_title(ui: &mut egui::Ui, title: &str, sub: &str) {
        ui.label(egui::RichText::new(title).font(font_semibold(21.0)).color(TEXT));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(sub).font(font(12.5)).color(DIM));
        ui.add_space(4.0);
    }

    fn capture_page(&mut self, ui: &mut egui::Ui) {
        Self::page_title(ui, t("захват"), t("выделение области"));
        ui::group(ui, l("выделение", "selection"), |ui| {
            ui::setting_row(ui, l("панель инструментов", "tools panel"), "", |ui| {
                egui::ComboBox::from_id_salt("tools-pos")
                    .selected_text(egui::RichText::new(self.cfg.tools_pos.label()).font(font(12.5)).color(TEXT))
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        for p in config::ToolsPos::ALL {
                            ui.selectable_value(&mut self.cfg.tools_pos, p, egui::RichText::new(p.label()).font(font(12.5)));
                        }
                    });
            });
            ui::setting_row(ui, t("мгновенный режим"), t("копировать без панели кнопок"), |ui| {
                ui::toggle(ui, &mut self.cfg.instant_mode);
            });
            let (sub, label) = if self.cfg.tour_done {
                (l("подсказки по шагам при следующем снимке", "step-by-step hints on the next capture"), l("показать снова", "show again"))
            } else {
                (l("покажется при следующем снимке", "shows on the next capture"), l("не показывать", "don't show"))
            };
            ui::setting_row(ui, l("знакомство", "walkthrough"), sub, |ui| {
                if ui::button(ui, label, ui::Kind::Ghost).clicked() {
                    self.cfg.tour_done = !self.cfg.tour_done;
                }
            });
            // a Mac tells the parts of a window apart through Accessibility
            #[cfg(target_os = "macos")]
            {
                let allowed = win::ax_trusted(false);
                let sub = if allowed {
                    l("клик выделяет часть окна: страницу, список. alt + клик — окно целиком", "a click takes part of a window: the page, a list. alt + click takes the whole window")
                } else {
                    l("чтобы клик выделял часть окна (например, страницу без вкладок), нужен «универсальный доступ»", "for a click to take part of a window (say, the page without the tabs), blinker needs Accessibility")
                };
                ui::setting_row(ui, l("части окон", "parts of windows"), sub, |ui| {
                    if allowed {
                        ui.label(egui::RichText::new(l("разрешено", "allowed")).font(font(12.5)).color(FAINT));
                    } else if ui::button(ui, l("разрешить", "allow"), ui::Kind::Ghost).clicked() {
                        win::ax_trusted(true);
                    }
                });
            }
        });
        ui::group(ui, t("лупа"), |ui| {
            ui::setting_row(ui, t("лупа"), t("увеличенный кусок экрана у курсора"), |ui| {
                ui::toggle(ui, &mut self.cfg.magnifier);
            });
            let loupe = if self.cfg.loupe_cell > 0.0 {
                lf!("×{:.0}, меняется колесом мыши при выделении", "×{:.0}, change it with the wheel while selecting",
                    self.cfg.loupe_cell
                )
            } else {
                t("скрыта. колесо вверх при выделении вернёт её").to_string()
            };
            ui::setting_row(ui, t("масштаб лупы"), &loupe, |_| {});
        });
        self.sound_group(ui);
    }

    fn record_page(&mut self, ui: &mut egui::Ui) {
        let hk = crate::hotkey::display_parts(self.cfg.primary_binding(HotkeyAction::Record)).join(" ");
        let how = if hk.is_empty() {
            t("клавиша не задана, запись есть в меню трея").to_string()
        } else {
            lf!("старт и стоп: {hk}", "start and stop: {hk}"
            )
        };
        Self::page_title(ui, t("запись экрана"), &how);

        ui::group(ui, l("видео", "video"), |ui| {
            let hz = {
                let (x, y) = win::cursor_pos();
                win::refresh_rate(x, y)
            };
            let fps_label = |fps: u32| -> String {
                if fps == 0 {
                    lf!("как экран · {hz}", "native · {hz}")
                } else {
                    lf!("{fps} fps", "{fps} fps")
                }
            };
            ui::setting_row(ui, t("частота кадров"), "", |ui| {
                egui::ComboBox::from_id_salt("rec-fps")
                    .selected_text(egui::RichText::new(fps_label(self.cfg.rec_fps)).font(font(12.5)).color(TEXT))
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        for fps in [0u32, 30, 60, 120, 144, 240] {
                            ui.selectable_value(&mut self.cfg.rec_fps, fps, egui::RichText::new(fps_label(fps)).font(font(12.5)));
                        }
                    });
            });
            let hevc = crate::record::encoder::hevc_available();
            let codec_desc = match (self.cfg.rec_codec, hevc) {
                (_, false) => l("видеокарта не умеет h.265", "this gpu can't do h.265"),
                (config::VideoCodec::H264, _) => l("открывается везде", "plays everywhere"),
                (config::VideoCodec::Hevc, _) => l("файл меньше, но открывается не везде", "smaller files, but not everything plays them"),
            };
            ui::setting_row(ui, l("кодек", "codec"), codec_desc, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_enabled_ui(hevc, |ui| {
                    if ui::filter(ui, "h.265", self.cfg.rec_codec == config::VideoCodec::Hevc).clicked() {
                        self.cfg.rec_codec = config::VideoCodec::Hevc;
                    }
                });
                if ui::filter(ui, "h.264", self.cfg.rec_codec == config::VideoCodec::H264 || !hevc).clicked() {
                    self.cfg.rec_codec = config::VideoCodec::H264;
                }
            });
            let q = lf!("около {:.0} мб в минуту для full hd", "about {:.0} mb a minute in full hd",
                self.cfg.rec_quality.typical_mb_min()
            );
            ui::setting_row(ui, t("качество"), &q, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for rq in config::RecQuality::ALL.into_iter().rev() {
                    if ui::filter(ui, rq.label(), self.cfg.rec_quality == rq).clicked() {
                        self.cfg.rec_quality = rq;
                    }
                }
            });
            ui::setting_row(ui, t("курсор в записи"), "", |ui| {
                ui::toggle(ui, &mut self.cfg.rec_cursor);
            });
        });

        ui::group(ui, t("звук"), |ui| {
            ui::setting_row(ui, t("звук системы"), t("дорожка 1"), |ui| {
                ui::toggle(ui, &mut self.cfg.rec_desktop_audio);
            });
            ui::setting_row(ui, t("микрофон"), t("дорожка 2"), |ui| {
                ui::toggle(ui, &mut self.cfg.rec_mic);
            });
            if self.cfg.rec_mic {
                self.mic_row(ui);
            }
        });

        ui::group(ui, l("в кадре", "on screen"), |ui| {
            ui::setting_row(ui, t("показывать клики"), "", |ui| {
                ui::toggle(ui, &mut self.cfg.rec_show_clicks);
            });
            ui::setting_row(
                ui,
                t("показывать сочетания клавиш"),
                t("только сочетания, обычный текст не видно"),
                |ui| {
                    ui::toggle(ui, &mut self.cfg.rec_show_keys);
                },
            );
        });

        ui::group(ui, "gif", |ui| {
            ui::setting_row(ui, l("ширина gif", "gif width"), "", |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for w in [1080u32, 800, 640, 480] {
                    if ui::filter(ui, &w.to_string(), self.cfg.gif_width == w).clicked() {
                        self.cfg.gif_width = w;
                    }
                }
            });
            ui::setting_row(ui, l("кадров в секунду", "frames per second"), "", |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for f in [24u32, 15, 10] {
                    if ui::filter(ui, &f.to_string(), self.cfg.gif_fps == f).clicked() {
                        self.cfg.gif_fps = f;
                    }
                }
            });
        });

        ui::group(ui, l("плашка записи", "recording pill"), |ui| {
            ui.add_space(10.0);
            ui.columns(2, |cols| {
                let (left, right) = cols.split_at_mut(1);
                pill_column(&mut left[0], l("область или окно", "an area or a window"), &mut self.cfg.rec_pill, false);
                pill_column(&mut right[0], l("весь экран", "the whole screen"), &mut self.cfg.rec_pill_screen, true);
            });
            ui.add_space(8.0);
        });

    }

    fn replay_page(&mut self, ui: &mut egui::Ui) {
        let rhk = crate::hotkey::display_parts(self.cfg.primary_binding(HotkeyAction::Replay)).join(" ");
        let how = if rhk.is_empty() {
            l("последние минуты экрана всегда под рукой. сохранить: меню трея", "the last minutes of the screen, always at hand. save: the tray menu").to_string()
        } else {
            lf!("последние минуты экрана всегда под рукой. сохранить: {rhk}", "the last minutes of the screen, always at hand. save: {rhk}")
        };
        Self::page_title(ui, t("повтор"), &how);
        ui::group(ui, "", |ui| {
            let desc = lf!("пишет главный монитор, ~{} мб на диске, иконка в трее красная", "records the main monitor, ~{} mb on disk, the tray icon turns red",
                (self.cfg.replay_secs as f32 / 60.0 * self.cfg.rec_quality.typical_mb_min() * 2.0).ceil().max(5.0)
            );
            ui::setting_row(ui, l("включить повтор", "replay on"), &desc, |ui| {
                ui::toggle(ui, &mut self.cfg.replay_enabled);
            });
            if self.cfg.replay_enabled {
                let max = crate::record::replay::MAX_SECS;
                let sub = lf!("от {MIN_REPLAY} до {max} секунд · это {}", "{MIN_REPLAY} to {max} seconds · that's {}", approx_length(self.cfg.replay_secs));
                ui::setting_row(ui, t("сколько хранить"), &sub, |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(egui::RichText::new(l("сек", "sec")).font(font(12.5)).color(DIM));
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.replay_min)
                            .desired_width(52.0)
                            .align(egui::Align2::CENTER_CENTER)
                            .char_limit(4)
                            .font(mono(12.5)),
                    );
                    if resp.changed()
                        && let Ok(s) = self.replay_min.trim().parse::<u32>()
                    {
                        self.cfg.replay_secs = s.clamp(MIN_REPLAY, max);
                    }
                    if !resp.has_focus() {
                        self.replay_min = replay_minutes(self.cfg.replay_secs);
                    }
                });
                ui::setting_row(
                    ui,
                    l("сохранение начинает повтор заново", "saving starts the replay over"),
                    l("следующий повтор будет с момента сохранения", "the next replay starts where the last one was saved"),
                    |ui| {
                        ui::toggle(ui, &mut self.cfg.replay_reset_on_save);
                    },
                );
            }
        });
        if self.cfg.replay_enabled {
            ui::group(ui, t("звук"), |ui| {
                ui::setting_row(ui, t("звук системы"), "", |ui| {
                    ui::toggle(ui, &mut self.cfg.replay_desktop_audio);
                });
                ui::setting_row(ui, t("микрофон"), t("микрофон будет записываться всё время"), |ui| {
                    ui::toggle(ui, &mut self.cfg.replay_mic);
                });
                if self.cfg.replay_mic {
                    self.mic_row(ui);
                }
            });
        }
    }

    /// Which microphone (records and the replay use the same one).
    fn mic_row(&mut self, ui: &mut egui::Ui) {
        let mics = self.mics.get_or_insert_with(crate::record::audio::mics).clone();
        let default = l("как в системе", "system default");
        let current = mics.iter().find(|(id, _)| *id == self.cfg.mic_device).map(|(_, n)| n.clone());
        let shown = match (&current, self.cfg.mic_device.is_empty()) {
            (_, true) => default.to_string(),
            (Some(n), _) => n.clone(),
            (None, false) => l("отключён, пишет микрофон системы", "unplugged, the default one records").to_string(),
        };
        ui::setting_row(ui, l("какой микрофон", "which microphone"), "", |ui| {
            egui::ComboBox::from_id_salt(("mic", ui.scope_id()))
                .selected_text(egui::RichText::new(&shown).font(font(12.5)).color(TEXT))
                .width(220.0)
                .height(400.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.cfg.mic_device, String::new(), egui::RichText::new(default).font(font(12.5)));
                    for (id, name) in &mics {
                        ui.selectable_value(&mut self.cfg.mic_device, id.clone(), egui::RichText::new(name).font(font(12.5)));
                    }
                });
        });
    }

    fn upload_page(&mut self, ui: &mut egui::Ui) {
        Self::page_title(ui, t("загрузка"), l("файл уходит на сервер, в буфер попадает ссылка", "the file goes to a server, the link goes to the clipboard"));
        ui::group(ui, "", |ui| {
            ui::setting_row(
                ui,
                l("загрузка", "uploads"),
                l("кнопки загрузки в уведомлении и редакторах, клавиша u при выделении", "the upload buttons on the card and in the editors, the u key while selecting"),
                |ui| {
                    ui::toggle(ui, &mut self.cfg.upload_enabled);
                },
            );
        });
        if !self.cfg.upload_enabled {
            return;
        }
        if self.upload_mode.is_none() {
            self.upload_mode = Some(UploadMode::of(&self.cfg));
        }
        let mut mode = self.upload_mode.unwrap_or(UploadMode::Default);
        let field = |ui: &mut egui::Ui, value: &mut String, hint: &str, password: bool| {
            ui.add(
                egui::TextEdit::singleline(value)
                    .hint_text(hint)
                    .password(password)
                    .desired_width(280.0)
                    .font(mono(12.5)),
            )
        };
        ui::group(ui, l("сервер", "server"), |ui| {
            // a ShareX uploader (brought over or from a .sxcu) takes the place of both
            if let Some(c) = self.cfg.upload_custom.clone() {
                let title = if c.name.is_empty() { l("загрузчик sharex", "sharex uploader").to_string() } else { lf!("загрузчик «{}»", "the “{}” uploader", c.name) };
                let mut off = false;
                ui::setting_row(ui, &title, &c.url, |ui| {
                    off = ui::button(ui, l("убрать", "remove"), ui::Kind::Ghost).clicked();
                });
                if off {
                    self.cfg.upload_custom = None;
                    mode = UploadMode::of(&self.cfg);
                }
            } else {
                ui::setting_row(ui, l("куда загружать", "upload to"), mode.desc(), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for m in [UploadMode::Own, UploadMode::Default] {
                        if ui::filter(ui, m.label(), mode == m).clicked() && mode != m {
                            mode = m;
                            if m == UploadMode::Default {
                                self.cfg.upload_url.clear();
                            }
                        }
                    }
                });
                if mode == UploadMode::Own {
                    ui::setting_row(ui, t("адрес сервера"), "", |ui| {
                        field(ui, &mut self.cfg.upload_url, "https://share.example.com/api/upload", false);
                    });
                    ui::setting_row(ui, t("токен"), "", |ui| {
                        field(ui, &mut self.cfg.upload_token, t("секретный ключ"), true);
                    });
                }
            }
            let status = match self.upload_test.as_ref().map(|r| r.lock().unwrap().clone()) {
                None => t("отправить тестовую картинку").to_string(),
                Some(None) => t("проверяю…").to_string(),
                Some(Some(Ok(url))) => lf!("работает: {url}", "works: {url}"),
                Some(Some(Err(e))) => lf!("ошибка: {e}", "error: {e}"),
            };
            let busy = self.upload_test.as_ref().is_some_and(|r| r.lock().unwrap().is_none());
            ui::setting_row(ui, t("проверка"), &status, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    if ui::button(ui, t("проверить"), ui::Kind::Ghost).clicked() {
                        let slot = Arc::new(Mutex::new(None));
                        let (s2, cfg, ctx) = (slot.clone(), self.cfg.clone(), ui.ctx().clone());
                        std::thread::spawn(move || {
                            *s2.lock().unwrap() = Some(crate::upload::test(&cfg));
                            ctx.request_repaint();
                        });
                        self.upload_test = Some(slot);
                    }
                });
            });
        });
        self.upload_mode = Some(mode);
        ui::group(ui, l("когда загружать", "when to upload"), |ui| {
            ui::setting_row(ui, t("загружать каждый снимок"), t("в буфер попадёт ссылка вместо файла"), |ui| {
                ui::toggle(ui, &mut self.cfg.upload_auto);
            });
            ui::setting_row(
                ui,
                l("мои загрузки", "my uploads"),
                l("ссылки, сколько им жить, удаление с сервера", "the links, how long they live, deleting from the server"),
                |ui| {
                    if ui::button(ui, l("открыть", "open"), ui::Kind::Ghost).clicked()
                        && let Ok(exe) = std::env::current_exe()
                    {
                        let _ = std::process::Command::new(exe).args(["--history", "uploads"]).spawn();
                    }
                },
            );
        });
        ui.add_space(16.0);
        let note = match (mode, self.cfg.upload_custom.is_some()) {
            (_, true) => l(
                "загрузчик перенесён из sharex. другой подключается в разделе «интеграции».",
                "the uploader came from sharex. another one is picked in “integrations”.",
            ),
            (UploadMode::Default, _) => l(
                "post.blinker.cool — наш сервер: без регистрации, файлы живут 3 дня, видео до 100 мб. свой такой же можно поднять из папки server в репозитории.",
                "post.blinker.cool is our server: no sign-up, files live 3 days, videos up to 100 mb. you can run your own from the server folder of the repository.",
            ),
            (UploadMode::Own, _) => l(
                "сервер получает post с файлом в теле и отвечает {\"url\": \"…\"}. готовый сервер — в папке server репозитория.",
                "the server gets a post with the file as the body and answers {\"url\": \"…\"}. a ready one is in the server folder of the repository.",
            ),
        };
        ui.add(egui::Label::new(egui::RichText::new(note).font(font(12.0)).color(FAINT)).wrap());
    }

    /// A .sxcu file as the uploader.
    fn pick_sxcu(&mut self, ctx: &egui::Context) {
        let Some(p) = rfd::FileDialog::new().add_filter("sharex", &["sxcu", "json"]).pick_file() else { return };
        match crate::sharex::read_sxcu(&p) {
            Ok(c) => {
                self.notice = Some(Notice::new(ctx, lf!("загрузчик «{}» подключён", "the “{}” uploader is on", if c.name.is_empty() { &c.url } else { &c.name })));
                self.cfg.upload_custom = Some(c);
            }
            Err(e) => self.notice = Some(Notice::new(ctx, e)),
        }
    }

    /// ShareX settings into ours (the system page's button, or a file).
    fn import_sharex(&mut self, ctx: &egui::Context, src: crate::sharex::Source) {
        let mut cfg = self.cfg.clone();
        match crate::sharex::import(&src, &mut cfg) {
            Ok(report) => {
                self.cfg = cfg;
                self.upload_mode = None;
                self.replay_min = replay_minutes(self.cfg.replay_secs);
                self.notice = Some(Notice::new(ctx, report.summary()));
            }
            Err(e) => self.notice = Some(Notice::new(ctx, e)),
        }
    }

    /// The card after a capture or a recording.
    fn notify_page(&mut self, ui: &mut egui::Ui) {
        Self::page_title(ui, t("уведомление"), l("превью после снимка и записи", "a preview after a capture or a recording"));
        ui::group(ui, "", |ui| {
            ui::setting_row(
                ui,
                l("показывать превью", "show the preview"),
                l("его можно перетащить как файл, правый клик закрывает", "drag it out as a file, a right click dismisses it"),
                |ui| {
                    ui::toggle(ui, &mut self.cfg.show_toast);
                },
            );
            if !self.cfg.show_toast {
                return;
            }
            ui::setting_row(ui, t("где появляться"), corner_name(self.cfg.toast_corner), |ui| {
                corner_picker(ui, &mut self.cfg.toast_corner);
            });
            ui::setting_row(ui, t("размер превью"), "", |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (s, label) in [(ToastSize::Large, t("большое")), (ToastSize::Medium, t("среднее")), (ToastSize::Small, t("маленькое"))] {
                    if ui::filter(ui, label, self.cfg.toast_size == s).clicked() {
                        self.cfg.toast_size = s;
                    }
                }
            });
            ui::setting_row(ui, t("подпись под превью"), t("формат и размер файла"), |ui| {
                ui::toggle(ui, &mut self.cfg.toast_caption);
            });
            ui::setting_row(ui, t("длительность"), l("при наведении не пропадает", "stays while hovered"), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let label = lf!("{:.0} с", "{:.0} s", self.cfg.toast_seconds);
                ui.add_sized(vec2(36.0, 26.0), egui::Label::new(egui::RichText::new(label).font(mono(11.5)).color(DIM)));
                ui.spacing_mut().slider_width = 130.0;
                ui::slider(ui, &mut self.cfg.toast_seconds, 2.0..=15.0, 1.0);
            });
        });
    }

    /// The click when a capture is taken.
    fn sound_group(&mut self, ui: &mut egui::Ui) {
        let mut preview = false;
        ui::group(ui, t("звук"), |ui| {
            ui::setting_row(ui, t("звук при снимке"), "", |ui| {
                ui::toggle(ui, &mut self.cfg.sound_enabled);
            });
            if !self.cfg.sound_enabled {
                return;
            }
            ui::setting_row(ui, l("какой звук", "which sound"), "", |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let mut kinds: Vec<SoundKind> = SoundKind::BUILTIN.to_vec();
                kinds.push(SoundKind::Custom);
                // right to left
                for k in kinds.into_iter().rev() {
                    if ui::filter(ui, k.label(), self.cfg.sound_kind == k).clicked() {
                        self.cfg.sound_kind = k;
                        if k == SoundKind::Custom && self.cfg.sound_file.is_none() {
                            self.pick_sound_file();
                        }
                        preview = true;
                    }
                }
            });
            if self.cfg.sound_kind == SoundKind::Custom {
                let desc = match &self.cfg.sound_file {
                    Some(p) => p.display().to_string(),
                    None => t("файл не выбран").into(),
                };
                ui::setting_row(ui, t("свой файл"), &desc, |ui| {
                    if ui::button(ui, t("выбрать…"), ui::Kind::Ghost).clicked() {
                        self.pick_sound_file();
                        preview = true;
                    }
                });
            }
            ui::setting_row(ui, t("громкость"), "", |ui| {
                // right to left: play, value, slider
                ui.spacing_mut().item_spacing.x = 8.0;
                if ui::icon_button(ui, Icon::Play, t("проверить"), 26.0, false, ui::Kind::Ghost).clicked() {
                    preview = true;
                }
                let mut pct = (self.cfg.sound_volume * 100.0).round();
                // as tall as the button and the slider: all three on one line
                ui.add_sized(
                    vec2(36.0, 26.0),
                    egui::Label::new(egui::RichText::new(format!("{pct:.0}%")).font(mono(11.5)).color(DIM)),
                );
                ui.spacing_mut().slider_width = 130.0;
                let resp = ui::slider(ui, &mut pct, 0.0..=100.0, 1.0);
                self.cfg.sound_volume = pct / 100.0;
                if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                    preview = true;
                }
            });
        });
        if preview {
            crate::sound::play(self.cfg.sound_kind, self.cfg.sound_file.clone(), self.cfg.sound_volume);
        }
    }

    fn pick_sound_file(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter(t("звук"), &["wav", "mp3", "ogg", "flac"]).pick_file() {
            self.cfg.sound_file = Some(p);
            self.cfg.sound_kind = SoundKind::Custom;
        } else if self.cfg.sound_file.is_none() {
            self.cfg.sound_kind = SoundKind::Shutter;
        }
    }

    fn storage_page(&mut self, ui: &mut egui::Ui) {
        Self::page_title(ui, t("сохранение"), t("папка и формат"));
        ui::group(ui, t("папка"), |ui| {
            let dir = self.cfg.save_dir.display().to_string();
            ui::setting_row(ui, t("папка"), &dir, |ui| {
                if ui::icon_button(ui, Icon::Folder, t("открыть папку"), 30.0, false, ui::Kind::Ghost).clicked() {
                    let _ = std::fs::create_dir_all(&self.cfg.save_dir);
                    win::open_path(&self.cfg.save_dir);
                }
                if ui::button(ui, t("изменить"), ui::Kind::Ghost).clicked()
                    && let Some(d) = rfd::FileDialog::new().set_directory(&self.cfg.save_dir).pick_folder()
                {
                    self.cfg.save_dir = d;
                }
            });
            let today = chrono::Local::now().format("%Y-%m-%d");
            let layout = lf!("blinker / {today} / pics и videos", "blinker / {today} / pics and videos");
            ui::setting_row(ui, t("раскладывать по датам"), &layout, |ui| {
                ui::toggle(ui, &mut self.cfg.sort_by_date);
            });
            // games are recognized on Windows only
            if cfg!(windows) {
                ui::setting_row(
                    ui,
                    l("клипы из игр — в папку игры", "clips from games in the game's folder"),
                    l("blinker / games / Dota 2, и в названии файла игра", "blinker / games / Dota 2, and the game in the file's name"),
                    |ui| {
                        ui::toggle(ui, &mut self.cfg.sort_by_game);
                    },
                );
            }
        });
        ui::group(ui, l("снимки", "screenshots"), |ui| {
            let fmt_desc = match self.cfg.format {
                ImageFormat::Png => t("без потерь"),
                ImageFormat::Webp => t("без потерь, легче png"),
                ImageFormat::Jpeg => t("с потерями, самый лёгкий"),
            };
            ui::setting_row(ui, t("формат"), fmt_desc, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (f, label) in [(ImageFormat::Jpeg, "jpeg"), (ImageFormat::Webp, "webp"), (ImageFormat::Png, "png")] {
                    if ui::filter(ui, label, self.cfg.format == f).clicked() {
                        self.cfg.format = f;
                    }
                }
            });
            if self.cfg.format == ImageFormat::Jpeg {
                ui::setting_row(ui, t("качество jpeg"), "", |ui| {
                    // right to left: value, slider
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.add_sized(
                        vec2(36.0, 26.0),
                        egui::Label::new(egui::RichText::new(format!("{}%", self.cfg.jpeg_quality)).font(mono(11.5)).color(DIM)),
                    );
                    ui.spacing_mut().slider_width = 130.0;
                    let mut q = self.cfg.jpeg_quality as f32;
                    if ui::slider(ui, &mut q, 50.0..=100.0, 1.0).changed() {
                        self.cfg.jpeg_quality = q as u8;
                    }
                });
            }
            ui::setting_row(ui, t("сохранять каждый снимок"), t("даже если нажал «копировать»"), |ui| {
                ui::toggle(ui, &mut self.cfg.auto_save);
            });
            ui::setting_row(ui, t("копировать при сохранении"), "", |ui| {
                ui::toggle(ui, &mut self.cfg.copy_on_save);
            });
        });
    }

    fn hotkeys_page(&mut self, ui: &mut egui::Ui) {
        Self::page_title(
            ui,
            t("горячие клавиши"),
            l("клик по сочетанию, чтобы поменять. на одно действие можно несколько", "click a shortcut to change it. one action can have several"),
        );
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        if now - self.failed_at > 0.5 {
            self.failed = config::load_failed_hotkeys();
            self.failed_at = now;
        }
        let failed = self.failed.clone();
        let groups: [(&str, &[HotkeyAction]); 2] = [
            (
                l("скриншоты", "screenshots"),
                &[HotkeyAction::Region, HotkeyAction::Screen, HotkeyAction::Window, HotkeyAction::Last, HotkeyAction::Pick, HotkeyAction::Text],
            ),
            (
                t("запись"),
                &[HotkeyAction::Record, HotkeyAction::RecordGif, HotkeyAction::Pause, HotkeyAction::Replay],
            ),
        ];
        for (title, actions) in groups {
            ui::group(ui, title, |ui| {
                for &action in actions {
                    self.keys_row(ui, KeyTarget::Global(action), action.label(), action.desc(), now, &failed);
                }
            });
        }
        // inside a capture, with an area selected
        let capture = if self.cfg.instant_mode {
            l("в захвате · не работают в мгновенном режиме", "in a capture · off in instant mode")
        } else {
            l("в захвате", "in a capture")
        };
        ui::group(ui, capture, |ui| {
            for k in config::CaptureKey::ALL {
                self.keys_row(ui, KeyTarget::Capture(k), k.label(), k.desc(), now, &failed);
            }
        });
        ui.add_space(14.0);
        let note = if self.recording.is_some() {
            t("нажми сочетание. esc отменяет, backspace удаляет это сочетание.")
        } else if cfg!(windows) && self.cfg.hotkey_region.iter().any(|b| b == "PrintScreen") {
            t("prtsc не работает? в параметрах windows выключи «клавиша print screen открывает ножницы»")
        } else {
            ""
        };
        if !note.is_empty() {
            ui.label(egui::RichText::new(note).font(font(12.0)).color(FAINT));
        }
        self.poll_recorder(&ctx);
    }

    /// One action's key combinations: chips (click to change, × to remove)
    /// and a "+" for one more.
    fn keys_row(&mut self, ui: &mut egui::Ui, target: KeyTarget, label: &str, desc: &str, now: f64, failed: &std::collections::HashSet<String>) {
        ui::setting_row(ui, label, desc, |ui| {
            // right to left: "+" first, then the shortcuts (last first)
            ui.spacing_mut().item_spacing.x = 6.0;
            let adding = self.recording.is_some_and(|r| r.action == target && r.index.is_none());
            if adding {
                recording_chip(ui, now);
            } else if add_button(ui).clicked() {
                self.start_recording(Recording { action: target, index: None, started: now });
            }
            let bindings = target.keys(&self.cfg).clone();
            let mut remove = None;
            for (i, b) in bindings.iter().enumerate().rev() {
                if self.recording.is_some_and(|r| r.action == target && r.index == Some(i)) {
                    recording_chip(ui, now);
                    continue;
                }
                // a capture's keys are never registered: they can't fail
                let failed = matches!(target, KeyTarget::Global(_)) && failed.contains(b);
                match chip(ui, b, failed) {
                    ChipAction::Edit => self.start_recording(Recording { action: target, index: Some(i), started: now }),
                    ChipAction::Remove => remove = Some(i),
                    ChipAction::None => {}
                }
            }
            if let Some(i) = remove {
                target.keys_mut(&mut self.cfg).remove(i);
            }
        });
    }

    fn sharex_group(&mut self, ui: &mut egui::Ui, found: Option<crate::sharex::Source>) {
        ui::group(ui, l("переезд с sharex", "moving from sharex"), |ui| {
            let desc = match &found {
                Some(s) => lf!("нашлись в {}: клавиши, папка, формат, уведомление, загрузчик", "found in {}: keys, folder, format, notification, uploader", s.path().display()),
                None => l("на этом компьютере не найдены, можно выбрать бэкап .sxb или папку", "not found on this computer, you can pick an .sxb backup or the folder").to_string(),
            };
            let mut action = None;
            ui::setting_row(ui, l("настройки sharex", "sharex settings"), &desc, |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if found.is_some() {
                    if ui::button(ui, l("перенести", "bring over"), ui::Kind::Accent).clicked() {
                        action = found.clone();
                    }
                    if ui::button(ui, l("файл…", "file…"), ui::Kind::Ghost).clicked() {
                        action = pick_sharex();
                    }
                } else if ui::button(ui, l("выбрать…", "choose…"), ui::Kind::Ghost).clicked() {
                    action = pick_sharex();
                }
            });
            if let Some(src) = action {
                self.import_sharex(ui.ctx(), src);
            }
            let mut pick = false;
            ui::setting_row(
                ui,
                l("загрузчик sharex", "sharex uploader"),
                l("файл .sxcu: снимки уходят на твой zipline, xbackbone и другие", "an .sxcu file: captures go to your zipline, xbackbone and the like"),
                |ui| {
                    pick = ui::button(ui, l("выбрать .sxcu…", "choose .sxcu…"), ui::Kind::Ghost).clicked();
                },
            );
            if pick {
                self.pick_sxcu(ui.ctx());
            }
        });
    }

    /// Subtitles from speech in the video editor: the models, put in or taken
    /// out here (nothing is downloaded until asked).
    fn speech_group(&mut self, ui: &mut egui::Ui) {
        use crate::vedit::speech::{self, Model};
        let done = self.speech_install.as_ref().map(|(_, job)| job.result.lock().unwrap().take());
        match done {
            Some(Some(r)) => {
                self.speech_error = r.err();
                self.speech_install = None;
            }
            Some(None) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(200)),
            None => {}
        }
        ui::group(ui, l("субтитры из речи", "subtitles from speech"), |ui| {
            for m in Model::ALL {
                let installing = self.speech_install.as_ref().filter(|(im, _)| *im == m).map(|(_, j)| j.fraction());
                let state = match installing {
                    Some(f) => lf!("скачивается {:.0}%", "downloading {:.0}%", f * 100.0),
                    None if m.ready() => l("установлена", "installed").to_string(),
                    None => l("не установлена", "not installed").to_string(),
                };
                ui::setting_row(ui, m.label(), &format!("{} · {state}", m.desc()), |ui| {
                    if installing.is_some() {
                        return;
                    }
                    if m.ready() {
                        if ui::button(ui, l("удалить", "remove"), ui::Kind::Ghost).clicked() {
                            speech::remove(m);
                        }
                    } else {
                        ui.add_enabled_ui(self.speech_install.is_none(), |ui| {
                            if ui::button(ui, l("установить", "install"), ui::Kind::Ghost).clicked() {
                                self.speech_error = None;
                                self.speech_install = Some((m, speech::install(ui.ctx(), m)));
                            }
                        });
                    }
                });
            }
            let hint = match &self.speech_error {
                Some(e) => lf!("не установилась: {e}", "didn't install: {e}"),
                None => l(
                    "нужна только для «распознать речь» в редакторе видео. работает на этом компьютере, без интернета",
                    "only for “recognize speech” in the video editor. it runs on this computer, offline",
                )
                .to_string(),
            };
            ui.label(egui::RichText::new(hint).font(font(12.0)).color(FAINT));
        });
    }

    /// The text recognition model (Windows; a Mac has Vision).
    fn ocr_group(&mut self, ui: &mut egui::Ui) {
        let done = self.ocr_install.as_ref().map(|job| job.result.lock().unwrap().take());
        match done {
            Some(Some(r)) => {
                self.ocr_error = r.err();
                self.ocr_install = None;
            }
            Some(None) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(200)),
            None => {}
        }
        ui::group(ui, l("распознавание текста", "text recognition"), |ui| {
            let ready = crate::paddle::ready();
            let state = match &self.ocr_install {
                Some(job) => lf!("скачивается {:.0}%", "downloading {:.0}%", job.fraction() * 100.0),
                None if ready => l("установлена", "installed").to_string(),
                None => l("не установлена", "not installed").to_string(),
            };
            let title = lf!("paddleocr · {} мб", "paddleocr · {} mb", crate::paddle::SIZE_MB);
            let desc = lf!("русский, украинский, белорусский, английский · {state}", "Russian, Ukrainian, Belarusian, English · {state}");
            ui::setting_row(ui, &title, &desc, |ui| {
                if self.ocr_install.is_some() {
                    return;
                }
                if ready {
                    if ui::button(ui, l("удалить", "remove"), ui::Kind::Ghost).clicked() {
                        crate::paddle::remove();
                    }
                } else if ui::button(ui, l("установить", "install"), ui::Kind::Ghost).clicked() {
                    self.ocr_error = None;
                    self.ocr_install = Some(crate::paddle::install(ui.ctx()));
                }
            });
            let hint = match &self.ocr_error {
                Some(e) => lf!("не установилась: {e}", "didn't install: {e}"),
                None => l(
                    "без неё текст не распознаётся. всё считается на этом компьютере, без интернета",
                    "text isn't read without it. it all runs on this computer, offline",
                )
                .to_string(),
            };
            ui.label(egui::RichText::new(hint).font(font(12.0)).color(FAINT));
        });
    }

    /// Text and QR codes read off the screen.
    fn text_page(&mut self, ui: &mut egui::Ui) {
        let hk = crate::hotkey::display_parts(self.cfg.primary_binding(HotkeyAction::Text)).join(" ");
        let sub = if hk.is_empty() {
            l("«распознать текст» в панели захвата", "“recognize text” in the capture panel").to_string()
        } else {
            lf!("{hk} или «распознать текст» в панели захвата", "{hk} or “recognize text” in the capture panel")
        };
        Self::page_title(ui, l("текст и qr-коды", "text and qr codes"), &sub);
        if cfg!(windows) {
            self.ocr_group(ui);
        } else {
            ui::group(ui, l("распознавание текста", "text recognition"), |ui| {
                ui::setting_row(ui, l("распознаёт macos", "macos reads it"), l("vision встроен в систему: ставить ничего не нужно", "vision is part of the system: nothing to install"), |_| {});
            });
        }
        ui::group(ui, l("qr-коды", "qr codes"), |ui| {
            ui::setting_row(
                ui,
                l("читать qr-коды", "read qr codes"),
                l("при снимке под каждым кодом — куда он ведёт и кнопка «перейти»", "on a capture, under each code: where it leads, and an \"open\" button"),
                |ui| {
                    ui::toggle(ui, &mut self.cfg.read_qr);
                },
            );
        });
    }

    /// The video editor: whether it opens after a recording, and its
    /// subtitles from speech.
    fn editor_page(&mut self, ui: &mut egui::Ui) {
        Self::page_title(ui, l("редактор видео", "video editor"), l("обрезка, монтаж, текст и субтитры", "trimming, cutting, text and subtitles"));
        ui::group(ui, l("после записи", "after a recording"), |ui| {
            ui::setting_row(ui, t("открывать редактор"), l("сразу после остановки записи", "as soon as a recording stops"), |ui| {
                ui::toggle(ui, &mut self.cfg.rec_open_editor);
            });
        });
        self.speech_group(ui);
    }

    /// The tray: what clicks on its icon do, and its menu, laid out by
    /// dragging.
    fn tray_page(&mut self, ui: &mut egui::Ui) {
        Self::page_title(ui, l("меню трея", "tray menu"), l("клики по значку и что в меню", "clicks on the icon and what's in the menu"));
        ui::group(ui, l("клики по значку", "clicks on the icon"), |ui| {
            // a Mac menu bar icon has no double click: the first click opens its menu
            let mut rows = vec![(l("левый клик", "left click"), "tray-left", &mut self.cfg.tray_click)];
            if !cfg!(target_os = "macos") {
                rows.push((l("двойной клик", "double click"), "tray-double", &mut self.cfg.tray_double_click));
            }
            rows.push((l("правый клик", "right click"), "tray-right", &mut self.cfg.tray_right_click));
            for (title, id, value) in rows {
                ui::setting_row(ui, title, "", |ui| {
                    egui::ComboBox::from_id_salt(id)
                        .selected_text(egui::RichText::new(value.label()).font(font(12.5)).color(TEXT))
                        .width(170.0)
                        .height(400.0)
                        .show_ui(ui, |ui| {
                            for a in TrayAction::ALL {
                                ui.selectable_value(value, a, egui::RichText::new(a.label()).font(font(12.5)));
                            }
                        });
                });
            }
            if self.cfg.tray_click != TrayAction::Menu && self.cfg.tray_right_click != TrayAction::Menu {
                ui::setting_row(
                    ui,
                    l("меню трея не открыть", "the tray menu is unreachable"),
                    l("поставь «показать меню» на одну из кнопок", "set “show menu” on one of the buttons"),
                    |_| {},
                );
            }
        });
        ui::group(ui, l("меню", "the menu"), |ui| {
            ui.add_space(12.0);
            self.tray_editor(ui);
            ui.add_space(12.0);
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(l(
                        "перетаскивай пункты: в плитки сверху, в список или вправо, чтобы убрать. заголовок ряда меняется по клику. настройки и выход остаются всегда. во время записи вместо плиток — «стоп» и «пауза». «сбросить» вернёт меню, как было",
                        "drag items into the tiles on top, into the list, or to the right to take them out. click a row's heading to change it. settings and quit always stay. while recording, the tiles give way to “stop” and “pause”. “reset” brings the menu back as it was",
                    ))
                    .font(font(12.0))
                    .color(FAINT),
                )
                .wrap(),
            );
        });
    }

    /// Other apps: Discord, ShareX, NVIDIA.
    fn integrations_page(&mut self, ui: &mut egui::Ui) {
        Self::page_title(ui, l("интеграции", "integrations"), l("discord, sharex и другие программы", "discord, sharex and other apps"));
        let found = self.sharex.get_or_insert_with(crate::sharex::detect).clone();
        // found ShareX goes first: a new user brings it over in one click
        if found.is_some() {
            self.sharex_group(ui, found.clone());
        }
        ui::group(ui, "discord", |ui| {
            ui::setting_row(
                ui,
                l("статус в discord", "discord status"),
                l("логотип blinker и кнопка на blinker.cool, пока blinker запущен", "the blinker logo and a blinker.cool button while blinker runs"),
                |ui| {
                    ui::toggle(ui, &mut self.cfg.discord_presence);
                },
            );
            if self.cfg.discord_presence {
                ui::setting_row(ui, l("показывать игру", "show the game"), l("«clipping dota 2» и значок игры, пока она запущена", "“clipping dota 2” and the game's icon while it runs"), |ui| {
                    ui::toggle(ui, &mut self.cfg.discord_game);
                });
                ui::setting_row(ui, l("показывать статистику", "show the numbers"), l("сколько снимков и клипов за сегодня", "how many screenshots and clips today"), |ui| {
                    ui::toggle(ui, &mut self.cfg.discord_stats);
                });
            }
        });
        if self.nvidia {
            ui::group(ui, "nvidia", |ui| {
                ui::setting_row(
                    ui,
                    l("надпись «press alt+z»", "the “press alt+z” note"),
                    l("оверлей nvidia принимает blinker за игру. убрать её можно только в его настройках", "the nvidia overlay takes blinker for a game. only its own settings can quiet it"),
                    |ui| {
                        let label = if self.nvidia_open { l("скрыть", "hide") } else { l("как убрать", "how to") };
                        if ui::button(ui, label, ui::Kind::Ghost).clicked() {
                            self.nvidia_open = !self.nvidia_open;
                        }
                    },
                );
                if self.nvidia_open {
                    nvidia_steps(ui);
                }
            });
        }
        if found.is_none() {
            self.sharex_group(ui, None);
        }
    }

    fn system_page(&mut self, ui: &mut egui::Ui) {
        Self::page_title(ui, t("система"), l("язык, запуск, обновления", "language, startup, updates"));
        ui::group(ui, l("основное", "basics"), |ui| {
            // the language names stay in their own language
            ui::setting_row(ui, "язык · language", "", |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let system = if cfg!(target_os = "macos") { l("как в системе", "system") } else { l("как в windows", "system") };
                for (code, label) in [("en", "english"), ("ru", "русский"), ("", system)] {
                    if ui::filter(ui, label, self.cfg.lang == code).clicked() {
                        self.cfg.lang = code.to_string();
                        crate::i18n::apply(code);
                    }
                }
            });
            let title = if cfg!(target_os = "macos") { t("открывать при входе в систему") } else { t("запуск вместе с windows") };
            ui::setting_row(ui, title, "", |ui| {
                ui::toggle(ui, &mut self.cfg.autostart);
            });
        });
        ui::group(ui, l("обновления", "updates"), |ui| {
            let newer = self.update.lock().unwrap().clone();
            let state = self.check.as_ref().map(|c| c.lock().unwrap().clone());
            use crate::update::Check;
            let desc = match (&state, &newer) {
                (Some(Check::Asking), _) => l("проверяю…", "checking…").to_string(),
                (_, Some(v)) => lf!("вышла версия {v}", "version {v} is out"),
                (Some(Check::Failed(e)), _) => lf!("не проверилось: {e}", "the check failed: {e}"),
                (Some(Check::Latest), _) => lf!("у тебя последняя версия, {}", "you have the latest version, {}", crate::update::current()),
                _ => lf!("сейчас стоит {}", "this is {}", crate::update::current()),
            };
            let busy = matches!(state, Some(Check::Asking)) || self.install.is_some();
            let mut install = false;
            ui::setting_row(ui, l("проверить обновление", "check for an update"), &desc, |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add_enabled_ui(!busy, |ui| {
                    if newer.is_some() {
                        install = ui::button(ui, l("обновить", "update"), ui::Kind::Accent).clicked();
                    }
                    if ui::button(ui, l("проверить", "check"), ui::Kind::Ghost).clicked() {
                        self.check = Some(crate::update::check_now(ui.ctx(), &self.update));
                    }
                });
            });
            if install {
                self.install = Some(crate::update::install(ui.ctx()));
            }
            ui::setting_row(
                ui,
                l("обновляться самому", "update by itself"),
                l("новая версия ставится в фоне, blinker перезапускается (не во время записи)", "a new version is put in place in the background and blinker restarts (never while recording)"),
                |ui| {
                    ui::toggle(ui, &mut self.cfg.auto_update);
                },
            );
        });
        ui.add_space(18.0);
        ui.label(egui::RichText::new(format!("blinker {}", env!("CARGO_PKG_VERSION"))).font(mono(11.0)).color(FAINT));
        // the config file: a click shows it in its folder
        let path = config::app_dir().join("config.toml");
        let resp = ui.add(egui::Label::new(egui::RichText::new(path.display().to_string()).font(mono(11.0)).color(FAINT)).sense(egui::Sense::click()));
        if resp.hovered() {
            ui.painter().hline(resp.rect.x_range(), resp.rect.bottom(), egui::Stroke::new(1.0, FAINT));
        }
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(l("показать в папке", "show in the folder")).clicked() {
            if path.exists() {
                win::reveal_in_explorer(&path);
            } else {
                win::open_path(&config::app_dir());
            }
        }
    }

    /// After an update: what the new version brings, once, over the settings.
    /// And the whole changelog after a click on the version.
    fn whats_new(&mut self, ctx: &egui::Context) {
        let version = crate::update::current();
        let fresh = self.cfg.seen_version != version;
        if !fresh && !self.changelog_open {
            return;
        }
        let (title, notes) = if fresh {
            (l("что нового", "what's new"), self.notes.get_or_insert_with(whats_new).clone())
        } else {
            (l("история версий", "release notes"), self.all_notes.get_or_insert_with(changelog).clone())
        };
        let mut close = notes.is_empty();
        if !close {
            let screen = ctx.content_rect();
            egui::Area::new(egui::Id::unique("whats-new-shade")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
                let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter().rect_filled(r, 0, egui::Color32::from_black_alpha(150));
                // a click beside the card closes it
                if resp.clicked() {
                    close = true;
                }
            });
            let w = 500f32.min(screen.width() - 60.0);
            let h = 440f32.min(screen.height() - 60.0);
            let card = Rect::from_center_size(screen.center(), vec2(w, h));
            egui::Area::new(egui::Id::unique("whats-new")).order(egui::Order::Tooltip).fixed_pos(card.min).show(ctx, |ui| {
                let frame = if theme::glass() { ui::float_frame(14) } else { egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0, LINE_HI)).corner_radius(14) };
                frame
                    .inner_margin(egui::Margin { left: 24, right: 24, top: 22, bottom: 18 })
                    .show(ui, |ui| {
                        ui.set_width(w - 48.0);
                        ui.set_height(h - 40.0);
                        ui.horizontal(|ui| {
                            brand_mark(ui, 36.0);
                            ui.add_space(4.0);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                ui.label(egui::RichText::new(title).font(font_semibold(19.0)).color(TEXT));
                                ui.label(egui::RichText::new(format!("blinker {version}")).font(mono(11.5)).color(FAINT));
                            });
                        });
                        ui.add_space(12.0);
                        let button_h = 34.0 + 14.0;
                        egui::ScrollArea::vertical().max_height(ui.available_height() - button_h).auto_shrink([false, true]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 6.0;
                            for (i, note) in notes.iter().enumerate() {
                                match note {
                                    Note::Version(v) => {
                                        if i > 0 {
                                            ui.add_space(14.0);
                                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                                            ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, ui::hairline()));
                                            ui.add_space(6.0);
                                        }
                                        ui.label(egui::RichText::new(v).font(mono(12.5)).color(ACCENT));
                                    }
                                    Note::Heading(text) => {
                                        ui.add_space(8.0);
                                        ui.label(egui::RichText::new(text).font(font_medium(13.5)).color(TEXT));
                                    }
                                    Note::Item(text) => {
                                        ui.horizontal_top(|ui| {
                                            ui.spacing_mut().item_spacing.x = 8.0;
                                            let (dot, _) = ui.allocate_exact_size(vec2(6.0, 18.0), Sense::hover());
                                            ui.painter().circle_filled(pos2(dot.center().x, dot.top() + 9.0), 2.0, FAINT);
                                            ui.add(egui::Label::new(egui::RichText::new(text).font(font(13.0)).color(TEXT2)).wrap());
                                        });
                                    }
                                }
                            }
                        });
                        ui.add_space(14.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui::button_h(ui, l("понятно", "got it"), ui::Kind::Accent, 34.0).clicked() {
                                close = true;
                            }
                        });
                    });
            });
            if ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter)) {
                close = true;
            }
        }
        if close {
            self.changelog_open = false;
            if fresh {
                self.cfg.seen_version = version.to_string();
                let v = version.to_string();
                self.shared.update_config(|c| c.seen_version = v);
            }
        }
    }

    fn start_recording(&mut self, r: Recording) {
        self.recording = Some(r);
        self.shared.suspend_hotkeys();
    }

    fn apply_config(&mut self, before: Config) {
        if same_config(&before, &self.cfg) {
            return;
        }
        if before.autostart != self.cfg.autostart && !win::set_autostart(self.cfg.autostart) {
            self.cfg.autostart = before.autostart;
        }
        let hotkeys_changed = HotkeyAction::ALL.iter().any(|a| before.bindings(*a) != self.cfg.bindings(*a))
            || crate::record::replay::config_key(&before) != crate::record::replay::config_key(&self.cfg)
            || before.lang != self.cfg.lang;
        let new = self.cfg.clone();
        self.shared.update_config(|c| *c = new);
        if hotkeys_changed && self.recording.is_none() {
            self.shared.reload_hotkeys();
        }
    }

    fn poll_recorder(&mut self, ctx: &egui::Context) {
        let Some(rec) = self.recording else { return };
        ctx.request_repaint();
        // Ignore the click that started recording.
        if ctx.input(|i| i.time) - rec.started < 0.25 {
            return;
        }
        let Some(c) = hotkey::poll_pressed() else { return };
        let backspace = hotkey::is_backspace(c);
        if hotkey::is_escape(c) {
            // cancel
        } else if backspace {
            if let Some(i) = rec.index {
                rec.action.keys_mut(&mut self.cfg).remove(i);
            }
        } else {
            let combo = hotkey::format(c);
            // One combination belongs to one action: take it away from the others.
            // (a system-wide hotkey would never let a capture see it)
            for a in KeyTarget::all() {
                if a != rec.action {
                    a.keys_mut(&mut self.cfg).retain(|b| *b != combo);
                }
            }
            let list = rec.action.keys_mut(&mut self.cfg);
            match rec.index {
                Some(i) if i < list.len() => list[i] = combo.clone(),
                _ => list.push(combo.clone()),
            }
            let mut seen = std::collections::HashSet::new();
            list.retain(|b| seen.insert(b.clone()));
        }
        self.recording = None;
        let new = self.cfg.clone();
        self.shared.update_config(|c| *c = new);
        self.shared.reload_hotkeys();
    }
}

#[derive(Clone, Copy, PartialEq)]
enum UploadMode {
    Default,
    Own,
}

impl UploadMode {
    fn of(cfg: &Config) -> UploadMode {
        if cfg.upload_url.trim().is_empty() { UploadMode::Default } else { UploadMode::Own }
    }

    fn label(self) -> &'static str {
        match self {
            UploadMode::Default => "blinker.cool",
            UploadMode::Own => l("свой сервер", "own server"),
        }
    }

    fn desc(self) -> &'static str {
        match self {
            UploadMode::Default => l("бесплатно, 3 дня, до 100 мб", "free, 3 days, up to 100 mb"),
            UploadMode::Own => l("адрес и токен своего сервера", "your own server's address and token"),
        }
    }
}

/// A link with another brand's mark before its name.
fn brand_link(ui: &mut egui::Ui, mark: crate::brand::Mark, name: &str, tip: &str) -> egui::Response {
    let g = ui.painter().layout_no_wrap(name.into(), mono(10.5), egui::Color32::PLACEHOLDER);
    let icon = 12.0;
    let size = vec2(icon + 5.0 + g.size().x, g.size().y.max(icon));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let col = if resp.hovered() { TEXT } else { DIM };
    let px = (icon * ui.ctx().pixels_per_point()).round() as u32;
    let key = egui::Id::unique(("brand", mark as u8, px));
    let tex = ui.data(|d| d.get_temp::<egui::TextureHandle>(key)).unwrap_or_else(|| {
        let t = ui.ctx().load_texture("brand", theme::color_image(&mark.image(px)), egui::TextureOptions::LINEAR);
        ui.data_mut(|d| d.insert_temp(key, t.clone()));
        t
    });
    let ir = Rect::from_min_size(pos2(rect.left(), rect.center().y - icon / 2.0), vec2(icon, icon));
    ui.painter().image(tex.id(), ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), col);
    let tx = pos2(ir.right() + 5.0, rect.center().y - g.size().y / 2.0);
    let text_w = g.size().x;
    ui.painter().galley(tx, g, col);
    if resp.hovered() {
        ui.painter().hline(tx.x..=tx.x + text_w, rect.bottom() + 0.5, Stroke::new(1.0, col));
    }
    resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip)
}

/// The logo tile of `size` points.
fn brand_mark(ui: &mut egui::Ui, size: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let px = (size * ui.ctx().pixels_per_point()).round() as u32;
    let key = egui::Id::unique(("brand-logo", px));
    let tex = ui.data(|d| d.get_temp::<egui::TextureHandle>(key)).unwrap_or_else(|| {
        let t = ui.ctx().load_texture("brand-logo", theme::color_image(&theme::logo(px)), egui::TextureOptions::LINEAR);
        ui.data_mut(|d| d.insert_temp(key, t.clone()));
        t
    });
    ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
}

/// A line of release notes.
#[derive(Clone, PartialEq)]
enum Note {
    /// "blinker 0.3.0"
    Version(String),
    Heading(String),
    Item(String),
}

const CHANGELOG: &str = include_str!("../CHANGELOG.md");

/// One version's section of CHANGELOG.md (built in): its Russian half or
/// its English one.
fn section_notes(lines: &[&str]) -> Vec<Note> {
    let halves: Vec<&[&str]> = lines.split(|l| l.trim() == "---").collect();
    let ru = crate::i18n::l("ru", "en") == "ru";
    let half = if ru || halves.len() < 2 { halves[0] } else { halves[1] };
    half.iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|l| if l.starts_with("**") && l.ends_with("**") { Note::Heading(plain(l)) } else { Note::Item(plain(l.trim_start_matches("- "))) })
        .collect()
}

/// The notes of this version.
fn whats_new() -> Vec<Note> {
    let head = format!("## {}", crate::update::current());
    let Some(start) = CHANGELOG.lines().position(|l| l.trim() == head) else { return Vec::new() };
    let section: Vec<&str> = CHANGELOG.lines().skip(start + 1).take_while(|l| !l.starts_with("## ")).collect();
    section_notes(&section)
}

/// Every version, newest first.
fn changelog() -> Vec<Note> {
    let mut out = Vec::new();
    let lines: Vec<&str> = CHANGELOG.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if let Some(v) = lines[i].strip_prefix("## ") {
            let end = lines[i + 1..].iter().position(|l| l.starts_with("## ")).map(|p| i + 1 + p).unwrap_or(lines.len());
            out.push(Note::Version(format!("blinker {}", v.trim())));
            out.extend(section_notes(&lines[i + 1..end]));
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// Markdown down to text: no `**`, no backticks, [links](…) as their words.
fn plain(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('[') {
        let Some(j) = rest[i..].find("](") else { break };
        let Some(k) = rest[i + j..].find(')') else { break };
        out.push_str(&rest[..i]);
        out.push_str(&rest[i + 1..i + j]);
        rest = &rest[i + j + k + 1..];
    }
    out.push_str(rest);
    out.replace("**", "").replace('`', "")
}

const MIN_REPLAY: u32 = crate::record::replay::MIN_SECS;

/// The replay length as typed: seconds.
fn replay_minutes(secs: u32) -> String {
    secs.max(MIN_REPLAY).to_string()
}

/// "45 sec", "5 min", "≈ 5.5 min": the length the way people say it.
fn approx_length(secs: u32) -> String {
    let (m, s) = (secs / 60, secs % 60);
    match (m, s) {
        (0, s) => lf!("{s} сек", "{s} sec"),
        (m, 0) => lf!("{m} мин", "{m} min"),
        (m, s) if s < 15 => lf!("≈ {m} мин", "≈ {m} min"),
        (m, s) if s > 45 => lf!("≈ {} мин", "≈ {} min", m + 1),
        (m, _) => lf!("≈ {m},5 мин", "≈ {m}.5 min"),
    }
}

/// An .sxb backup or the ShareX folder, picked by the user.
fn pick_sharex() -> Option<crate::sharex::Source> {
    let p = rfd::FileDialog::new().add_filter("sharex", &["sxb", "zip", "json"]).pick_file()?;
    crate::sharex::source_of(&p)
}

fn corner_name(c: Corner) -> &'static str {
    match c {
        Corner::TopLeft => crate::i18n::t("слева сверху"),
        Corner::TopRight => crate::i18n::t("справа сверху"),
        Corner::BottomLeft => crate::i18n::t("слева снизу"),
        Corner::BottomRight => crate::i18n::t("справа снизу"),
    }
}

/// A tiny screen with four clickable corners.
fn corner_picker(ui: &mut egui::Ui, value: &mut Corner) {
    let (rect, _) = ui.allocate_exact_size(vec2(84.0, 52.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, ui::soft_radius(4), ui::well());
    p.rect_stroke(rect, ui::soft_radius(4), Stroke::new(1.0, ui::edge()), StrokeKind::Inside);
    let card = vec2(22.0, 13.0);
    for (corner, anchor) in [
        (Corner::TopLeft, rect.left_top() + vec2(5.0, 5.0)),
        (Corner::TopRight, rect.right_top() + vec2(-5.0 - card.x, 5.0)),
        (Corner::BottomLeft, rect.left_bottom() + vec2(5.0, -5.0 - card.y)),
        (Corner::BottomRight, rect.right_bottom() + vec2(-5.0 - card.x, -5.0 - card.y)),
    ] {
        let r = Rect::from_min_size(anchor, card);
        let resp = ui.interact(r.expand(3.0), ui.scope_id().with(corner_name(corner)), Sense::click());
        let selected = *value == corner;
        let t = ui.ctx().animate_bool_with_time(resp.id, selected || resp.hovered(), 0.12);
        let fill = theme::lerp_color(SURFACE_IDLE, if selected { ACCENT } else { DIM }, t);
        ui.painter().rect_filled(r, 2, fill);
        if resp.on_hover_text(corner_name(corner)).on_hover_cursor(CursorIcon::PointingHand).clicked() {
            *value = corner;
        }
    }
}

const SURFACE_IDLE: egui::Color32 = egui::Color32::from_rgb(0x30, 0x30, 0x2D);

fn same_config(a: &Config, b: &Config) -> bool {
    toml::to_string(a).ok() == toml::to_string(b).ok()
}

enum ChipAction {
    None,
    Edit,
    Remove,
}

/// A key combination: click to re-record, × to remove.
fn chip(ui: &mut egui::Ui, binding: &str, failed: bool) -> ChipAction {
    let parts = hotkey::display_parts(binding);
    let galleys: Vec<_> = parts.iter().map(|p| ui.painter().layout_no_wrap(p.clone(), mono(11.0), TEXT)).collect();
    let caps_w: f32 = galleys.iter().map(|g| g.size().x + 12.0).sum::<f32>() + (galleys.len().saturating_sub(1)) as f32 * 4.0;
    let size = vec2(caps_w + 10.0 + 18.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let x_rect = Rect::from_min_size(pos2(rect.right() - 18.0, rect.top()), vec2(18.0, rect.height()));
    let x_resp = ui.interact(x_rect, resp.id.with("x"), Sense::click());
    let hovered = resp.hovered() || x_resp.hovered();
    let p = ui.painter();
    let radius = ui::control_radius(rect.height(), 5.0);
    if hovered {
        p.rect_filled(rect, radius, ui::hover_fill());
    }
    if failed {
        p.rect_stroke(rect, radius, Stroke::new(1.0, DANGER), StrokeKind::Inside);
    }
    // keycaps
    let mut x = rect.left() + 5.0;
    for g in galleys {
        let cap = Rect::from_min_size(pos2(x, rect.center().y - 9.5), vec2(g.size().x + 12.0, 19.0));
        let edge = if failed { DANGER } else { ui::edge() };
        p.rect_filled(cap, ui::soft_radius(4).min(6), ui::well());
        p.rect_stroke(cap, ui::soft_radius(4).min(6), Stroke::new(1.0, edge), StrokeKind::Inside);
        // a slightly thicker bottom edge, like a real key
        p.hline(cap.x_range().shrink(5.0), cap.bottom() - 0.5, Stroke::new(1.0, edge));
        let col = if failed { DANGER } else { TEXT };
        p.galley(cap.center() - g.size() / 2.0, g, col);
        x = cap.right() + 4.0;
    }
    if hovered {
        let x_col = if x_resp.hovered() { TEXT } else { DIM };
        icons::paint(p, x_rect, Icon::Close, 10.0, x_col);
    }
    if x_resp.on_hover_text(t("удалить")).on_hover_cursor(CursorIcon::PointingHand).clicked() {
        return ChipAction::Remove;
    }
    let tip = if failed { t("занято другой программой. клик, чтобы задать другое") } else { t("клик, чтобы перезаписать") };
    if resp.on_hover_text(tip).on_hover_cursor(CursorIcon::PointingHand).clicked() {
        return ChipAction::Edit;
    }
    ChipAction::None
}

fn recording_chip(ui: &mut egui::Ui, now: f64) {
    let galley = ui.painter().layout_no_wrap(crate::i18n::t("нажми сочетание…").into(), mono(11.0), ACCENT);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 18.0, 26.0), Sense::hover());
    let t = ((now * 4.0).sin() as f32) * 0.5 + 0.5;
    let p = ui.painter();
    let radius = ui::control_radius(rect.height(), 3.0);
    p.rect_filled(rect, radius, ui::well());
    p.rect_stroke(rect, radius, Stroke::new(1.0, theme::lerp_color(ui::edge(), ACCENT, t)), StrokeKind::Inside);
    p.galley(pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0), galley, ACCENT);
}

fn add_button(ui: &mut egui::Ui) -> egui::Response {
    let galley = ui.painter().layout_no_wrap("+".into(), font_medium(14.0), DIM);
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let p = ui.painter();
    let h = resp.hovered();
    if theme::glass() {
        // a round glass button
        p.circle_filled(rect.center(), 13.0, if h { ui::plate() } else { ui::hover_fill() });
        p.circle_stroke(rect.center(), 12.5, Stroke::new(1.0, ui::edge()));
    } else {
        if h {
            p.rect_filled(rect, 3, HOVER);
        }
        let pts = dashed(rect.shrink(0.5));
        p.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0, if h { DIM } else { LINE }), 4.0, 3.0));
    }
    p.galley(rect.center() - galley.size() / 2.0, galley, if h { TEXT } else { DIM });
    resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(t("+ добавить"))
}

/// "update to 0.3.4" at the bottom of the sidebar: a row like the sections,
/// quiet, with the download arrow.
fn update_row(ui: &mut egui::Ui, version: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let h = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), 0.08);
    let p = ui.painter();
    let radius = ui::control_radius(rect.height(), 7.0);
    p.rect_filled(rect, radius, theme::lerp_color(ui::plate(), ui::hover_fill().gamma_multiply(2.6), h));
    let icon = Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), vec2(15.0, 15.0));
    icons::paint(p, icon, Icon::Save, 15.0, ACCENT);
    let g = p.layout_no_wrap(lf!("обновить до {version}", "update to {version}"), font(12.5), TEXT);
    p.galley(pos2(rect.left() + 34.0, rect.center().y - g.size().y / 2.0), g, TEXT);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn dashed(r: Rect) -> Vec<egui::Pos2> {
    vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()]
}

impl eframe::App for HomeApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if theme::glass() { [0.0; 4] } else { BG.to_normalized_gamma_f32() }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // blinker://update: the footer shows how it goes (or that this is
        // the latest version)
        if UPDATE_ASKED.swap(false, Ordering::SeqCst) && self.install.is_none() {
            self.install = Some(crate::update::install(&ctx));
        }
        if let Some(tab) = self.request.lock().unwrap().take() {
            self.tab = tab;
            self.cfg = self.shared.config();
            self.failed = config::load_failed_hotkeys();
            if self.hidden.swap(false, Ordering::SeqCst) {
                self.frame_no = 0;
                self.shown = None;
                crate::update::watch(&ctx, &self.update);
                self.check = None;
                self.mics = None;
            } else if let Some(h) = self.hwnd {
                win::force_foreground(h);
            }
        }
        if self.hidden.load(Ordering::SeqCst) {
            return;
        }
        self.frame_no += 1;
        if let Some(h) = self.hwnd {
            // Revealed only once real frames exist: no white flash on open.
            if self.frame_no == 3 {
                win::cloak(h, false);
                win::force_foreground(h);
            } else if self.frame_no < 3 {
                ctx.request_repaint();
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.recording.take().is_some() {
                self.shared.reload_hotkeys();
            }
            // With the tray running, closing just hides the window (see `run`).
            if let Some(h) = self.hwnd
                && win::window_class_exists("BlinkerShell")
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.notice = None;
                self.hidden.store(true, Ordering::SeqCst);
                win::show_window(h, false);
                win::trim_memory();
                return;
            }
        }
        // other processes change these (a capture ends the walkthrough, the
        // loupe and the pen remember their last setting): never write them
        // back stale
        let disk = self.shared.config();
        self.cfg.tour_done = disk.tour_done;
        self.cfg.loupe_cell = disk.loupe_cell;
        self.cfg.editor_color = disk.editor_color;
        self.cfg.editor_width = disk.editor_width;
        self.nav(ui);
        self.content(ui);
        chrome::frame_border(&ctx);
        self.whats_new(&ctx);
        if let Some(n) = &self.notice
            && !n.show(&ctx, self.content_rect)
        {
            self.notice = None;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(2));
    }
}

/// One of the pill's two places: a heading, the choice in words, the picker.
fn pill_column(ui: &mut egui::Ui, title: &str, place: &mut config::PillPlace, whole: bool) {
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(title).font(font_medium(13.0)).color(TEXT));
        ui.add_space(2.0);
        let words = if whole { place.label_screen() } else { place.label() };
        ui.label(egui::RichText::new(words).font(font(12.0)).color(DIM));
        ui.add_space(6.0);
        pill_picker(ui, place, whole);
    });
}

/// A small screen with a taskbar and what's recorded (an area, or all of
/// it): the pill sits where it will during a recording; a click on another
/// spot moves it there.
fn pill_picker(ui: &mut egui::Ui, place: &mut config::PillPlace, whole: bool) {
    use config::PillPlace;
    let w = ui.available_width().clamp(200.0, 360.0);
    let h = (w * 0.62).round();
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), h + 8.0), Sense::hover());
    let screen = Rect::from_center_size(row.center(), vec2(w, h));
    let p = ui.painter().clone();
    p.rect_filled(screen, 9, egui::Color32::from_rgb(0x19, 0x19, 0x18));
    // what is recorded: the whole screen, or an area in it
    let recorded = if whole { screen.shrink(1.5) } else { Rect::from_min_size(screen.min + vec2(w * 0.24, h * 0.2), vec2(w * 0.42, h * 0.4)) };
    let edge = if whole { recorded.shrink(2.0) } else { recorded };
    let dash = egui::Shape::dashed_line(&[edge.left_top(), edge.right_top(), edge.right_bottom(), edge.left_bottom(), edge.left_top()], Stroke::new(1.0, ACCENT.gamma_multiply(0.7)), 5.0, 3.0);
    p.extend(dash);
    if !whole {
        p.rect_stroke(screen, 9, Stroke::new(1.0, LINE), StrokeKind::Inside);
    }
    // a Mac: the menu bar on top, the Dock below; Windows: the taskbar with
    // the apps in the middle, the tray and the clock on the right
    let work = if cfg!(target_os = "macos") {
        let menu = Rect::from_min_max(screen.min, pos2(screen.right(), screen.top() + 8.0));
        p.rect_filled(menu, egui::CornerRadius { nw: 9, ne: 9, sw: 0, se: 0 }, egui::Color32::from_rgb(0x24, 0x24, 0x22));
        for i in 0..3 {
            p.circle_filled(pos2(menu.right() - 30.0 + i as f32 * 7.0, menu.center().y), 1.5, FAINT);
        }
        let dock = Rect::from_center_size(pos2(screen.center().x, screen.bottom() - 9.0), vec2(56.0, 11.0));
        p.rect_filled(dock, 4, egui::Color32::from_rgb(0x2A, 0x2A, 0x27));
        for i in -2..=2 {
            p.rect_filled(Rect::from_center_size(pos2(dock.center().x + i as f32 * 10.0, dock.center().y), egui::Vec2::splat(6.0)), 2, egui::Color32::from_rgb(0x46, 0x46, 0x42));
        }
        Rect::from_min_max(pos2(screen.left(), menu.bottom()), pos2(screen.right(), dock.top())).shrink(6.0)
    } else {
        let bar = Rect::from_min_max(pos2(screen.left(), screen.bottom() - 13.0), screen.max);
        p.rect_filled(bar, egui::CornerRadius { nw: 0, ne: 0, sw: 9, se: 9 }, egui::Color32::from_rgb(0x24, 0x24, 0x22));
        for i in -2..=2 {
            p.rect_filled(Rect::from_center_size(pos2(bar.center().x + i as f32 * 10.0, bar.center().y), egui::Vec2::splat(6.0)), 2, egui::Color32::from_rgb(0x46, 0x46, 0x42));
        }
        for i in 0..3 {
            p.circle_filled(pos2(bar.right() - 38.0 + i as f32 * 6.0, bar.center().y), 1.5, FAINT);
        }
        p.rect_filled(Rect::from_center_size(pos2(bar.right() - 13.0, bar.center().y), vec2(13.0, 4.0)), 1, FAINT);
        Rect::from_min_max(screen.min, pos2(screen.right(), bar.top())).shrink(6.0)
    };

    // where each choice puts the pill (spots relative to the little screen,
    // so scrolling the page doesn't send the pill gliding)
    // the real pill, about a third of the little screen wide
    let pw = (w * 0.3).round();
    let size = vec2(pw, (pw * 40.0 / 170.0).round());
    let spot = |pl: PillPlace| -> Rect {
        match pl.anchor() {
            Some((fx, fy)) => Rect::from_min_size(pos2(work.left() + (work.width() - size.x) * fx, work.top() + (work.height() - size.y) * fy), size),
            // by the tray (a Mac: the corner over the Dock)
            None if whole => Rect::from_min_size(work.right_bottom() - size, size),
            None => Rect::from_center_size(pos2(recorded.center().x, recorded.bottom() + 5.0 + size.y / 2.0), size),
        }
    };
    // recording the whole screen, "auto" already is the bottom right corner
    let choices: Vec<PillPlace> = PillPlace::ALL.into_iter().filter(|pl| !(whole && *pl == PillPlace::BottomRight)).collect();
    let words = |pl: PillPlace| if whole { pl.label_screen() } else { pl.label() };
    let mut hovered = None;
    for pl in choices {
        let r = spot(pl);
        let resp = ui.interact(r.expand(4.0), ui.scope_id().with(("pill-spot", whole, words(pl))), Sense::click());
        if pl != *place {
            p.rect_stroke(r, size.y * 0.3, Stroke::new(1.0, if resp.hovered() { DIM } else { LINE }), StrokeKind::Inside);
        }
        if resp.hovered() {
            hovered = Some(r);
        }
        if resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(words(pl)).clicked() {
            *place = pl;
        }
    }
    // the pill glides to the chosen spot; a ghost of it under the pointer
    let to = spot(*place);
    let x = ui.ctx().animate_value_with_time(ui.scope_id().with(("pill-x", whole)), to.left() - screen.left(), 0.25);
    let y = ui.ctx().animate_value_with_time(ui.scope_id().with(("pill-y", whole)), to.top() - screen.top(), 0.25);
    if let Some(r) = hovered.filter(|r| *r != to) {
        mini_pill(ui, &p, r, 0.4);
    }
    mini_pill(ui, &p, Rect::from_min_size(screen.min + vec2(x, y), size), 1.0);
}

/// The recording pill as it looks during a recording (drawn by the same
/// code), at the size of `r`: rasterized for this screen's pixels, kept.
fn mini_pill(ui: &egui::Ui, p: &egui::Painter, r: Rect, a: f32) {
    let ppp = ui.ctx().pixels_per_point();
    let s = r.width() * ppp / 170.0;
    let key = egui::Id::unique(("mini-pill", (s * 1000.0).round() as i64));
    let tex = ui.data(|d| d.get_temp::<egui::TextureHandle>(key)).or_else(|| {
        let look = crate::record::hud::PillLook { secs: 12, paused: false, clock: 0.0, hover: None, glass: false };
        let pm = crate::record::hud::draw_pill(&look, s)?;
        let img = theme::pixmap_to_image(&pm);
        let t = ui.ctx().load_texture("mini-pill", theme::color_image(&img), egui::TextureOptions::LINEAR);
        ui.data_mut(|d| d.insert_temp(key, t.clone()));
        Some(t)
    });
    match tex {
        Some(t) => {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            p.image(t.id(), r, uv, egui::Color32::WHITE.gamma_multiply(a));
        }
        None => {
            p.rect_filled(r, r.height() / 2.0, egui::Color32::from_rgba_unmultiplied(22, 22, 21, (245.0 * a) as u8));
        }
    }
}

/// How to quiet the NVIDIA overlay's note, step by step.
fn nvidia_steps(ui: &mut egui::Ui) {
    let steps = [
        l("нажми alt+z: откроется оверлей nvidia", "press alt+z: the nvidia overlay opens"),
        l("справа нажми шестерёнку (настройки оверлея, не nvidia app)", "on the right, click the gear (the overlay's settings, not the nvidia app)"),
        l("открой «уведомления»", "open “notifications”"),
        l("выключи уведомление об открытии оверлея", "turn off the note about opening the overlay"),
    ];
    egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 16, top: 4, bottom: 14 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        for (i, step) in steps.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 10.0, HOVER);
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, (i + 1).to_string(), font_medium(11.5), TEXT);
                ui.label(egui::RichText::new(*step).font(font(12.5)).color(TEXT));
            });
        }
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(l(
                "alt+z продолжит работать, пропадёт только надпись. совсем выключить оверлей: nvidia app → настройки → функции → оверлей nvidia",
                "alt+z keeps working, only the note goes. to turn the overlay off altogether: nvidia app → settings → features → nvidia overlay",
            ))
            .font(font(12.0))
            .color(FAINT),
        );
    });
}

#[cfg(test)]
mod tray_editor_tests {
    use super::{TrayDrag, TrayDrop, TrayFrom, TrayPiece, tray_move};
    use crate::tray::{Entry, TrayItem, TrayMenu};

    /// Moving pieces around the tray menu: within a row, from a row to the
    /// list, from the spare ones, out of the menu; an emptied row goes.
    #[test]
    fn pieces_move() {
        let mut m = TrayMenu::default();
        // "window" before "area" in the first row
        tray_move(&mut m, TrayDrag { piece: TrayPiece::Item(TrayItem::Window), from: TrayFrom::Tile(0, 2) }, TrayDrop::Tile(0, 0));
        assert_eq!(m.tiles[0].items, [TrayItem::Window, TrayItem::Region, TrayItem::Screen]);
        // "area" to the end of its own row
        tray_move(&mut m, TrayDrag { piece: TrayPiece::Item(TrayItem::Region), from: TrayFrom::Tile(0, 1) }, TrayDrop::Tile(0, 3));
        assert_eq!(m.tiles[0].items, [TrayItem::Window, TrayItem::Screen, TrayItem::Region]);
        // the color picker from the spare ones to the top of the list
        tray_move(&mut m, TrayDrag { piece: TrayPiece::Item(TrayItem::Pick), from: TrayFrom::Spare }, TrayDrop::List(0));
        assert_eq!(m.list[0], Entry::Item(TrayItem::Pick));
        // a list row down past two others
        tray_move(&mut m, TrayDrag { piece: TrayPiece::Item(TrayItem::Pick), from: TrayFrom::List(0) }, TrayDrop::List(3));
        assert_eq!(m.list[2], Entry::Item(TrayItem::Pick));
        // the video row emptied tile by tile: it goes
        for _ in 0..3 {
            let item = m.tiles[1].items[0];
            tray_move(&mut m, TrayDrag { piece: TrayPiece::Item(item), from: TrayFrom::Tile(1, 0) }, TrayDrop::Remove);
        }
        assert_eq!(m.tiles.len(), 1);
        // a tile into a row of its own
        tray_move(&mut m, TrayDrag { piece: TrayPiece::Item(TrayItem::Region), from: TrayFrom::Tile(0, 2) }, TrayDrop::NewRow);
        assert_eq!(m.tiles.len(), 2);
        assert_eq!(m.tiles[1].items, [TrayItem::Region]);
        // a separator from the spare ones
        let n = m.list.len();
        tray_move(&mut m, TrayDrag { piece: TrayPiece::Separator, from: TrayFrom::Spare }, TrayDrop::List(n));
        assert_eq!(m.list.last(), Some(&Entry::Separator));
    }
}

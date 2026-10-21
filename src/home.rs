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
const SIZE: (f32, f32) = (820.0, 580.0);
/// The strip at the top that drags the window (the window buttons sit in it).
const TITLE_BAR_H: f32 = 48.0;
const NAV_W: f32 = 204.0;

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
        .with_inner_size([SIZE.0, SIZE.1]);
    let viewport = chrome::style(viewport);
    let shared2 = shared.clone();
    runtime::run_window(shared, viewport, true, move |cc| {
        let hwnd = win::hwnd_of(cc);
        if let Some(h) = hwnd {
            win::style_borderless(h);
            chrome::place_window_buttons(h, TITLE_BAR_H);
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
            update: crate::update::watch(&cc.egui_ctx),
            install: None,
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
            notes: None,
            shown: None,
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
        HomeTab::Record => "record",
        HomeTab::Replay => "replay",
        HomeTab::Upload => "upload",
        HomeTab::Notify => "notify",
        HomeTab::Sound => "sound",
        HomeTab::Storage => "storage",
        HomeTab::Hotkeys => "hotkeys",
        HomeTab::System => "system",
    }
}

pub fn tab_from_arg(arg: &str) -> HomeTab {
    match arg {
        "notify" => HomeTab::Notify,
        "record" => HomeTab::Record,
        "replay" => HomeTab::Replay,
        "upload" => HomeTab::Upload,
        "sound" => HomeTab::Sound,
        "storage" => HomeTab::Storage,
        "hotkeys" => HomeTab::Hotkeys,
        "system" => HomeTab::System,
        _ => HomeTab::Capture,
    }
}

/// Size of everything under `dir` (date / pics / videos subfolders included).
/// A binding being recorded: which action, and which slot (None = new one).
#[derive(Clone, Copy, PartialEq)]
struct Recording {
    action: HotkeyAction,
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
    content_rect: Rect,
    /// The replay length field, in minutes, as typed.
    replay_min: String,
    /// Where uploads go, as picked on the page (empty fields still mean "own").
    upload_mode: Option<UploadMode>,
    /// ShareX settings on this computer (looked up once).
    sharex: Option<Option<crate::sharex::Source>>,
    /// This version's notes, read once.
    notes: Option<Vec<(bool, String)>>,
    /// Page currently on screen and when it started appearing.
    shown: Option<HomeTab>,
    page_at: f64,
}

impl HomeApp {
    /// The sidebar, the whole height of the window: the window buttons and
    /// the name at the top, sections in groups, the version at the bottom.
    fn nav(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("nav")
            .exact_size(NAV_W)
            .resizable(false)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin { left: 10, right: 10, top: 0, bottom: 12 }))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().vline(r.right() + 10.5, r.y_range().expand(12.0), Stroke::new(1.0, LINE_SOFT));
                let top = Rect::from_min_size(pos2(r.left() - 10.0, r.top()), vec2(NAV_W, TITLE_BAR_H));
                chrome::drag_region(ui, top);
                // the name: in the title strip on Windows, under the window
                // buttons on a Mac
                if cfg!(target_os = "macos") {
                    ui.add_space(TITLE_BAR_H - 4.0);
                }
                let h = if cfg!(target_os = "macos") { 30.0 } else { TITLE_BAR_H };
                ui.allocate_ui_with_layout(vec2(r.width(), h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 7.0;
                    ui.add_space(8.0);
                    brand_mark(ui, 20.0);
                    ui.label(egui::RichText::new("blinker").font(font_semibold(15.0)).color(TEXT));
                });
                ui.spacing_mut().item_spacing.y = 2.0;
                let plate = ui.painter().add(egui::Shape::Noop);
                let mut selected_rect = None;
                let groups: [(&str, &[(HomeTab, Icon, &str)]); 3] = [
                    (
                        l("снимки", "screenshots"),
                        &[
                            (HomeTab::Capture, Icon::Region, t("захват")),
                            (HomeTab::Notify, Icon::Bell, t("уведомление")),
                            (HomeTab::Sound, Icon::Sound, t("звук")),
                        ],
                    ),
                    (
                        l("видео", "video"),
                        &[(HomeTab::Record, Icon::Record, t("запись")), (HomeTab::Replay, Icon::Repeat, t("повтор"))],
                    ),
                    (
                        l("общее", "general"),
                        &[
                            (HomeTab::Storage, Icon::Save, t("сохранение")),
                            (HomeTab::Upload, Icon::Upload, t("загрузка")),
                            (HomeTab::Hotkeys, Icon::Keys, t("горячие клавиши")),
                            (HomeTab::System, Icon::Settings, t("система")),
                        ],
                    ),
                ];
                for (i, (caption, items)) in groups.into_iter().enumerate() {
                    if i > 0 {
                        ui.add_space(6.0);
                    }
                    ui::nav_caption(ui, caption);
                    for &(tab, icon, label) in items {
                        let resp = ui::nav_item(ui, icon, label, self.tab == tab);
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
                    ui.painter().set(plate, egui::Shape::rect_filled(rect, 7, ACTIVE));
                }
                self.nav_footer(ui);
            });
    }

    /// Bottom of the sections: the version, a link to GitHub and, when a
    /// newer release is out, the way to it.
    fn nav_footer(&mut self, ui: &mut egui::Ui) {
        let newer = self.update.lock().unwrap().clone();
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.add_space(8.0);
                let version = ui.painter().layout_no_wrap(format!("blinker {}", crate::update::current()), mono(10.5), FAINT);
                let (rect, _) = ui.allocate_exact_size(version.size(), Sense::hover());
                ui.painter().galley(rect.min, version, FAINT);
                let g = ui.painter().layout_no_wrap("github".into(), mono(10.5), egui::Color32::PLACEHOLDER);
                let (rect, resp) = ui.allocate_exact_size(g.size(), Sense::click());
                let col = if resp.hovered() { TEXT } else { DIM };
                ui.painter().galley(rect.min, g, col);
                if resp.hovered() {
                    ui.painter().hline(rect.x_range(), rect.bottom() + 0.5, Stroke::new(1.0, col));
                }
                if resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(crate::update::REPO_URL).clicked() {
                    win::open_url(crate::update::REPO_URL);
                }
            });
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
                (Some(Stage::Failed(e)), _) => {
                    if ui::button_h(ui, l("скачать с сайта", "get it from the site"), ui::Kind::Ghost, 26.0).clicked() {
                        win::open_url(crate::update::SITE_URL);
                    }
                    note(ui, format!("  {e}"), DANGER);
                }
                (None, Some(v)) => {
                    let tip = lf!("вышла версия {v}: скачать и перезапустить", "version {v} is out: download it and restart");
                    if ui::button_h(ui, l("обновить blinker", "update blinker"), ui::Kind::Accent, 28.0).on_hover_text(tip).clicked() {
                        self.install = Some(crate::update::install(ui.ctx()));
                    }
                }
                (None, None) => {}
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
        egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(BG)).show(ui, |ui| {
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
            ui.set_opacity(t);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Frame::new().inner_margin(egui::Margin { left: 32, right: 32, top: 26, bottom: 28 }).show(ui, |ui| {
                    ui.set_max_width(600.0f32.min(ui.available_width()));
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.add_space((1.0 - t) * 14.0);
                    match self.tab {
                        HomeTab::Capture => self.capture_page(ui),
                        HomeTab::Record => self.record_page(ui),
                        HomeTab::Replay => self.replay_page(ui),
                        HomeTab::Upload => self.upload_page(ui),
                        HomeTab::Notify => self.notify_page(ui),
                        HomeTab::Sound => self.sound_page(ui),
                        HomeTab::Storage => self.storage_page(ui),
                        HomeTab::Hotkeys => self.hotkeys_page(ui),
                        HomeTab::System => self.system_page(ui),
                    }
                });
            });
        });
        self.apply_config(before);
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

        ui::group(ui, l("после остановки", "after stopping"), |ui| {
            ui::setting_row(ui, t("открывать редактор"), "", |ui| {
                ui::toggle(ui, &mut self.cfg.rec_open_editor);
            });
        });

        ui::group(ui, t("для telegram"), |ui| {
            ui::setting_row(ui, t("режим"), self.cfg.tg_mode.desc(), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for m in config::TgMode::ALL.into_iter().rev() {
                    if ui::filter(ui, m.label(), self.cfg.tg_mode == m).clicked() {
                        self.cfg.tg_mode = m;
                    }
                }
            });
            ui::setting_row(ui, t("чем сжимается"), &crate::reencode::engine_label(self.cfg.tg_mode), |_| {});
            ui::setting_row(
                ui,
                t("сжимать каждую запись"),
                t("копия ложится в буфер, оригинал остаётся"),
                |ui| {
                    ui::toggle(ui, &mut self.cfg.tg_auto);
                },
            );
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
                let max_min = crate::record::replay::MAX_SECS / 60;
                ui::setting_row(ui, t("сколько хранить"), &lf!("от 1 до {max_min} минут", "1 to {max_min} minutes"), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(egui::RichText::new(l("мин", "min")).font(font(12.5)).color(DIM));
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.replay_min)
                            .desired_width(44.0)
                            .align(egui::Align2::CENTER_CENTER)
                            .char_limit(2)
                            .font(mono(12.5)),
                    );
                    if resp.changed()
                        && let Ok(m) = self.replay_min.trim().parse::<u32>()
                    {
                        self.cfg.replay_secs = m.clamp(1, max_min) * 60;
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
            });
        }
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
        });
        ui.add_space(16.0);
        let note = match (mode, self.cfg.upload_custom.is_some()) {
            (_, true) => l(
                "загрузчик перенесён из sharex. другой подключается в разделе «система».",
                "the uploader came from sharex. another one is picked in “system”.",
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

    fn notify_page(&mut self, ui: &mut egui::Ui) {
        Self::page_title(ui, t("уведомление"), t("превью после снимка"));
        ui::group(ui, "", |ui| {
            ui::setting_row(ui, t("показывать после снимка"), t("его можно перетащить как файл"), |ui| {
                ui::toggle(ui, &mut self.cfg.show_toast);
            });
        });
        if !self.cfg.show_toast {
            return;
        }
        ui::group(ui, l("вид", "look"), |ui| {
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
        });
        ui::group(ui, l("время", "timing"), |ui| {
            ui::setting_row(ui, t("длительность"), l("при наведении не пропадает", "stays while hovered"), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let label = lf!("{:.0} с", "{:.0} s", self.cfg.toast_seconds);
                ui.add_sized(vec2(34.0, 20.0), egui::Label::new(egui::RichText::new(label).font(mono(11.5)).color(DIM)));
                ui.spacing_mut().slider_width = 130.0;
                ui.add(egui::Slider::new(&mut self.cfg.toast_seconds, 2.0..=15.0).step_by(1.0).show_value(false));
            });
        });
    }

    fn sound_page(&mut self, ui: &mut egui::Ui) {
        Self::page_title(ui, t("звук"), t("щелчок при снимке"));
        ui::group(ui, "", |ui| {
            ui::setting_row(ui, t("звук при снимке"), "", |ui| {
                ui::toggle(ui, &mut self.cfg.sound_enabled);
            });
        });
        if !self.cfg.sound_enabled {
            return;
        }
        let mut preview = false;
        ui::group(ui, "", |ui| {
            ui::setting_block(ui, l("какой звук", "which sound"), "", |ui| {
                let mut kinds: Vec<SoundKind> = SoundKind::BUILTIN.to_vec();
                kinds.push(SoundKind::Custom);
                for k in kinds {
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
                ui.add_sized(
                    vec2(34.0, 20.0),
                    egui::Label::new(egui::RichText::new(format!("{pct:.0}%")).font(mono(11.5)).color(DIM)),
                );
                ui.spacing_mut().slider_width = 130.0;
                let resp = ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).step_by(1.0).show_value(false));
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
                    ui.add(egui::Slider::new(&mut self.cfg.jpeg_quality, 50..=100).suffix("%"));
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
                &[HotkeyAction::Region, HotkeyAction::Screen, HotkeyAction::Window, HotkeyAction::Last],
            ),
            (
                t("запись"),
                &[HotkeyAction::Record, HotkeyAction::RecordGif, HotkeyAction::Pause, HotkeyAction::Replay],
            ),
        ];
        for (title, actions) in groups {
            ui::group(ui, title, |ui| {
                for &action in actions {
                    ui::setting_row(ui, action.label(), action.desc(), |ui| {
                        // right to left: "+" first, then the shortcuts (last first)
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let adding = self.recording.is_some_and(|r| r.action == action && r.index.is_none());
                        if adding {
                            recording_chip(ui, now);
                        } else if add_button(ui).clicked() {
                            self.start_recording(Recording { action, index: None, started: now });
                        }
                        let bindings = self.cfg.bindings(action).clone();
                        let mut remove = None;
                        for (i, b) in bindings.iter().enumerate().rev() {
                            if self.recording.is_some_and(|r| r.action == action && r.index == Some(i)) {
                                recording_chip(ui, now);
                                continue;
                            }
                            match chip(ui, b, failed.contains(b)) {
                                ChipAction::Edit => self.start_recording(Recording { action, index: Some(i), started: now }),
                                ChipAction::Remove => remove = Some(i),
                                ChipAction::None => {}
                            }
                        }
                        if let Some(i) = remove {
                            self.cfg.bindings_mut(action).remove(i);
                        }
                    });
                }
            });
        }
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

    fn system_page(&mut self, ui: &mut egui::Ui) {
        Self::page_title(ui, t("система"), t("запуск, трей, язык"));
        let found = self.sharex.get_or_insert_with(crate::sharex::detect).clone();
        // found ShareX goes first: a new user brings it over in one click
        if found.is_some() {
            self.sharex_group(ui, found.clone());
        }
        ui::group(ui, "язык · language", |ui| {
            // the language names stay in their own language
            ui::setting_row(ui, l("язык интерфейса", "interface language"), "", |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let system = if cfg!(target_os = "macos") { l("как в системе", "system") } else { l("как в windows", "system") };
                for (code, label) in [("en", "english"), ("ru", "русский"), ("", system)] {
                    if ui::filter(ui, label, self.cfg.lang == code).clicked() {
                        self.cfg.lang = code.to_string();
                        crate::i18n::apply(code);
                    }
                }
            });
        });
        ui::group(ui, l("запуск", "startup"), |ui| {
            let title = if cfg!(target_os = "macos") { t("открывать при входе в систему") } else { t("запуск вместе с windows") };
            ui::setting_row(ui, title, "", |ui| {
                ui::toggle(ui, &mut self.cfg.autostart);
            });
            ui::setting_row(
                ui,
                l("обновляться самому", "update by itself"),
                l("новая версия ставится в фоне, blinker перезапускается (не во время записи)", "a new version is put in place in the background and blinker restarts (never while recording)"),
                |ui| {
                    ui::toggle(ui, &mut self.cfg.auto_update);
                },
            );
        });
        ui::group(ui, l("значок в трее", "tray icon"), |ui| {
            for (title, id, value) in [
                (l("левый клик", "left click"), "tray-left", &mut self.cfg.tray_click),
                (l("двойной клик", "double click"), "tray-double", &mut self.cfg.tray_double_click),
                (l("правый клик", "right click"), "tray-right", &mut self.cfg.tray_right_click),
            ] {
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
        if found.is_none() {
            self.sharex_group(ui, None);
        }
        ui.add_space(18.0);
        ui.label(
            egui::RichText::new(format!(
                "blinker {}\n{}",
                env!("CARGO_PKG_VERSION"),
                config::app_dir().join("config.toml").display()
            ))
            .font(mono(11.0))
            .color(FAINT),
        );
    }

    /// After an update: what the new version brings, once, over the settings.
    fn whats_new(&mut self, ctx: &egui::Context) {
        let version = crate::update::current();
        if self.cfg.seen_version == version {
            return;
        }
        let notes = self.notes.get_or_insert_with(whats_new).clone();
        let mut close = notes.is_empty();
        if !close {
            let screen = ctx.content_rect();
            egui::Area::new(egui::Id::unique("whats-new-shade")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
                let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter().rect_filled(r, 0, egui::Color32::from_black_alpha(150));
                let _ = resp;
            });
            let w = 500f32.min(screen.width() - 60.0);
            let h = 440f32.min(screen.height() - 60.0);
            let card = Rect::from_center_size(screen.center(), vec2(w, h));
            egui::Area::new(egui::Id::unique("whats-new")).order(egui::Order::Tooltip).fixed_pos(card.min).show(ctx, |ui| {
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, LINE_HI))
                    .corner_radius(14)
                    .inner_margin(egui::Margin { left: 24, right: 24, top: 22, bottom: 18 })
                    .show(ui, |ui| {
                        ui.set_width(w - 48.0);
                        ui.set_height(h - 40.0);
                        ui.horizontal(|ui| {
                            brand_mark(ui, 26.0);
                            ui.add_space(4.0);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                ui.label(egui::RichText::new(l("что нового", "what's new")).font(font_semibold(19.0)).color(TEXT));
                                ui.label(egui::RichText::new(format!("blinker {version}")).font(mono(11.5)).color(FAINT));
                            });
                        });
                        ui.add_space(12.0);
                        let button_h = 34.0 + 14.0;
                        egui::ScrollArea::vertical().max_height(ui.available_height() - button_h).auto_shrink([false, true]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 6.0;
                            for (heading, text) in &notes {
                                if *heading {
                                    ui.add_space(8.0);
                                    ui.label(egui::RichText::new(text).font(font_medium(13.5)).color(TEXT));
                                } else {
                                    ui.horizontal_top(|ui| {
                                        ui.spacing_mut().item_spacing.x = 8.0;
                                        let (dot, _) = ui.allocate_exact_size(vec2(6.0, 18.0), Sense::hover());
                                        ui.painter().circle_filled(pos2(dot.center().x, dot.top() + 9.0), 2.0, FAINT);
                                        ui.add(egui::Label::new(egui::RichText::new(text).font(font(13.0)).color(TEXT2)).wrap());
                                    });
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
            self.cfg.seen_version = version.to_string();
            let v = version.to_string();
            self.shared.update_config(|c| c.seen_version = v);
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
                self.cfg.bindings_mut(rec.action).remove(i);
            }
        } else {
            let combo = hotkey::format(c);
            // One combination belongs to one action: take it away from the others.
            for a in HotkeyAction::ALL {
                if a != rec.action {
                    self.cfg.bindings_mut(a).retain(|b| *b != combo);
                }
            }
            let list = self.cfg.bindings_mut(rec.action);
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

/// The logo tile (sparkle on graphite) of `size` points.
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

/// The release notes of this version, from CHANGELOG.md (built in): the
/// Russian half or the English one, as (is a heading, text) lines.
fn whats_new() -> Vec<(bool, String)> {
    const LOG: &str = include_str!("../CHANGELOG.md");
    let head = format!("## {}", crate::update::current());
    let Some(start) = LOG.lines().position(|l| l.trim() == head) else { return Vec::new() };
    let section: Vec<&str> = LOG.lines().skip(start + 1).take_while(|l| !l.starts_with("## ")).collect();
    let halves: Vec<&[&str]> = section.split(|l| l.trim() == "---").collect();
    let ru = crate::i18n::l("ru", "en") == "ru";
    let half = if ru || halves.len() < 2 { halves[0] } else { halves[1] };
    half.iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|l| {
            let heading = l.starts_with("**") && l.ends_with("**");
            (heading, plain(l.trim_start_matches("- ")))
        })
        .collect()
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

fn replay_minutes(secs: u32) -> String {
    (secs.div_ceil(60)).max(1).to_string()
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
    p.rect_filled(rect, 4, CANVAS);
    p.rect_stroke(rect, 4, Stroke::new(1.0, LINE), StrokeKind::Inside);
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
    if hovered {
        p.rect_filled(rect, 5, HOVER);
    }
    if failed {
        p.rect_stroke(rect, 5, Stroke::new(1.0, DANGER), StrokeKind::Inside);
    }
    // keycaps
    let mut x = rect.left() + 5.0;
    for g in galleys {
        let cap = Rect::from_min_size(pos2(x, rect.center().y - 9.5), vec2(g.size().x + 12.0, 19.0));
        p.rect_filled(cap, 4, CANVAS);
        p.rect_stroke(cap, 4, Stroke::new(1.0, if failed { DANGER } else { LINE }), StrokeKind::Inside);
        // a slightly thicker bottom edge, like a real key
        p.hline(cap.x_range().shrink(3.0), cap.bottom() - 0.5, Stroke::new(1.0, if failed { DANGER } else { LINE }));
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
    p.rect_filled(rect, 3, CANVAS);
    p.rect_stroke(rect, 3, Stroke::new(1.0, theme::lerp_color(LINE, ACCENT, t)), StrokeKind::Inside);
    p.galley(pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0), galley, ACCENT);
}

fn add_button(ui: &mut egui::Ui) -> egui::Response {
    let galley = ui.painter().layout_no_wrap("+".into(), font_medium(14.0), DIM);
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let p = ui.painter();
    let h = resp.hovered();
    if h {
        p.rect_filled(rect, 3, HOVER);
    }
    let pts = dashed(rect.shrink(0.5));
    p.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0, if h { DIM } else { LINE }), 4.0, 3.0));
    p.galley(rect.center() - galley.size() / 2.0, galley, if h { TEXT } else { DIM });
    resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(t("+ добавить"))
}

fn dashed(r: Rect) -> Vec<egui::Pos2> {
    vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()]
}

impl eframe::App for HomeApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        BG.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if let Some(tab) = self.request.lock().unwrap().take() {
            self.tab = tab;
            self.cfg = self.shared.config();
            self.failed = config::load_failed_hotkeys();
            if self.hidden.swap(false, Ordering::SeqCst) {
                self.frame_no = 0;
                self.shown = None;
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

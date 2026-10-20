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
use eframe::egui::{self, Align2, CursorIcon, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Runs the settings window (its own process) until it is closed.
pub const TITLE: &str = "blinker · настройки";
/// Named event the tray sets to show an already running settings process.
pub const SHOW_EVENT: &str = "Local\\BlinkerSettingsShow";

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
        .with_decorations(false)
        .with_inner_size([700.0, 500.0])
        .with_min_inner_size([560.0, 380.0]);
    let shared2 = shared.clone();
    runtime::run_window(shared, viewport, true, move |cc| {
        let hwnd = win::hwnd_of(cc);
        if let Some(h) = hwnd {
            win::style_borderless(h);
            win::cloak(h, true);
            win::force_foreground(h);
        }
        let hidden = Arc::new(AtomicBool::new(false));
        let request = Arc::new(Mutex::new(None));
        if let (Some(h), Some(ev)) = (hwnd, event) {
            watch_show_requests(ev, h as isize, hidden.clone(), request.clone(), cc.egui_ctx.clone());
        }
        HomeApp {
            hwnd,
            frame_no: 0,
            hidden,
            request,
            upload_test: None,
            cfg: shared2.config(),
            disk_bytes: folder_size(&shared2.config().save_dir),
            shared: shared2,
            tab,
            recording: None,
            notice: None,
            failed: config::load_failed_hotkeys(),
            failed_at: 0.0,
            content_rect: Rect::NOTHING,
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
        "upload" => HomeTab::Upload,
        "sound" => HomeTab::Sound,
        "storage" => HomeTab::Storage,
        "hotkeys" => HomeTab::Hotkeys,
        "system" => HomeTab::System,
        _ => HomeTab::Capture,
    }
}

/// Size of everything under `dir` (date / pics / videos subfolders included).
fn folder_size(dir: &std::path::Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => folder_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

fn human_size(b: u64) -> String {
    let kb = b as f64 / 1024.0;
    if kb < 1000.0 { lf!("{kb:.0} кб", "{kb:.0} kb") } else { lf!("{:.1} мб", "{:.1} mb", kb / 1024.0).replace('.', ",") }
}

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
    disk_bytes: u64,
    content_rect: Rect,
    /// Page currently on screen and when it started appearing.
    shown: Option<HomeTab>,
    page_at: f64,
}

impl HomeApp {
    fn title_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("title")
            .exact_size(32.0)
            .frame(egui::Frame::new().fill(PANEL))
            .show(ui, |ui| {
                let r = ui.max_rect();
                chrome::drag_region(ui, r);
                ui.painter().hline(r.x_range(), r.bottom() + 0.5, Stroke::new(1.0, LINE));
                ui.horizontal_centered(|ui| {
                    ui.add_space(10.0);
                    chrome::mark(ui);
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new("blinker").font(font_semibold(12.5)).color(TEXT));
                    ui.label(egui::RichText::new(crate::i18n::t("настройки")).font(font(12.5)).color(FAINT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        chrome::window_buttons(ui, true);
                    });
                });
            });
    }

    fn nav(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("nav")
            .exact_size(150.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(6, 8)))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().vline(r.right() + 6.5, r.y_range().expand(8.0), Stroke::new(1.0, LINE));
                ui.spacing_mut().item_spacing.y = 2.0;
                let plate = ui.painter().add(egui::Shape::Noop);
                let mut selected_rect = None;
                for (tab, icon, label) in [
                    (HomeTab::Capture, Icon::Region, crate::i18n::t("захват")),
                    (HomeTab::Record, Icon::Record, crate::i18n::t("запись")),
                    (HomeTab::Upload, Icon::Upload, crate::i18n::t("загрузка")),
                    (HomeTab::Notify, Icon::Bell, crate::i18n::t("уведомление")),
                    (HomeTab::Sound, Icon::Sound, crate::i18n::t("звук")),
                    (HomeTab::Storage, Icon::Save, crate::i18n::t("сохранение")),
                    (HomeTab::Hotkeys, Icon::Keys, crate::i18n::t("горячие клавиши")),
                    (HomeTab::System, Icon::Settings, crate::i18n::t("система")),
                ] {
                    let resp = ui::nav_item(ui, icon, label, self.tab == tab);
                    if resp.clicked() {
                        self.tab = tab;
                    }
                    if self.tab == tab {
                        selected_rect = Some(resp.rect);
                    }
                }
                // The selection plate glides between sections.
                if let Some(r) = selected_rect {
                    let y = ui.ctx().animate_value_with_time(egui::Id::unique("nav-plate"), r.top(), 0.16);
                    let rect = Rect::from_min_size(pos2(r.left(), y), r.size());
                    ui.painter().set(plate, egui::Shape::rect_filled(rect, 3, ACTIVE));
                }
                let mb = win::memory_usage() as f64 / 1024.0 / 1024.0;
                let text = lf!("озу {mb:.0} мб\nна диске {}", "ram {mb:.0} mb\non disk {}", human_size(self.disk_bytes));
                ui.painter().text(pos2(r.left() + 8.0, r.bottom()), Align2::LEFT_BOTTOM, text, mono(10.0), FAINT);
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
            ui.set_opacity(t);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Frame::new().inner_margin(egui::Margin { left: 18, right: 18, top: 14, bottom: 18 }).show(ui, |ui| {
                    ui.set_max_width(560.0f32.min(ui.available_width()));
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.add_space((1.0 - t) * 14.0);
                    match self.tab {
                        HomeTab::Capture => self.capture_page(ui),
                        HomeTab::Record => self.record_page(ui),
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
        ui.label(egui::RichText::new(title).font(font_medium(15.0)).color(TEXT));
        ui.add_space(3.0);
        ui.label(egui::RichText::new(sub).font(font(11.5)).color(DIM));
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

        ui::group(ui, t("повтор"), |ui| {
            let rhk = crate::hotkey::display_parts(self.cfg.primary_binding(HotkeyAction::Replay)).join(" ");
            let desc = lf!("держит последние секунды главного монитора, ~{} мб на диске. сохранить: {}", "keeps the last seconds of the main monitor, ~{} mb on disk. save: {}",
                (self.cfg.replay_secs as f32 / 60.0 * self.cfg.rec_quality.typical_mb_min() * 2.0).ceil().max(5.0),
                if rhk.is_empty() { t("меню трея") } else { &rhk }
            );
            ui::setting_row(ui, t("повтор"), &desc, |ui| {
                ui::toggle(ui, &mut self.cfg.replay_enabled);
            });
            if self.cfg.replay_enabled {
                ui::setting_row(
                    ui,
                    t("сколько хранить"),
                    "",
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for secs in [300u32, 120, 60, 30] {
                            let label = if secs >= 60 { lf!("{} мин", "{} min", secs / 60) } else { lf!("{secs} с", "{secs} s") };
                            if ui::filter(ui, &label, self.cfg.replay_secs == secs).clicked() {
                                self.cfg.replay_secs = secs;
                            }
                        }
                    },
                );
                ui::setting_row(ui, t("звук системы в повторе"), "", |ui| {
                    ui::toggle(ui, &mut self.cfg.replay_desktop_audio);
                });
                ui::setting_row(ui, t("микрофон в повторе"), t("микрофон будет записываться всё время"), |ui| {
                    ui::toggle(ui, &mut self.cfg.replay_mic);
                });
            }
        });
    }

    fn upload_page(&mut self, ui: &mut egui::Ui) {
        Self::page_title(ui, t("загрузка"), t("отправка на свой сервер, в буфер попадает ссылка"));
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
            ui::setting_row(ui, t("адрес сервера"), "", |ui| {
                field(ui, &mut self.cfg.upload_url, "https://share.example.com/api/upload", false);
            });
            ui::setting_row(ui, t("токен"), "", |ui| {
                field(ui, &mut self.cfg.upload_token, t("секретный ключ"), true);
            });
            if crate::upload::configured(&self.cfg) {
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
            }
        });
        if crate::upload::configured(&self.cfg) {
            ui::group(ui, l("когда загружать", "when to upload"), |ui| {
                ui::setting_row(ui, t("загружать каждый снимок"), t("в буфер попадёт ссылка вместо файла"), |ui| {
                    ui::toggle(ui, &mut self.cfg.upload_auto);
                });
            });
        }
        ui.add_space(14.0);
        ui.label(
            egui::RichText::new(l("сервер получает post с файлом в теле и отвечает {\"url\": \"…\"}", "the server gets a post with the file as the body and answers {\"url\": \"…\"}"))
            .font(font(12.0))
            .color(FAINT),
        );
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
        } else if self.cfg.hotkey_region.iter().any(|b| b == "PrintScreen") {
            t("prtsc не работает? в параметрах windows выключи «клавиша print screen открывает ножницы»")
        } else {
            ""
        };
        if !note.is_empty() {
            ui.label(egui::RichText::new(note).font(font(12.0)).color(FAINT));
        }
        self.poll_recorder(&ctx);
    }

    fn system_page(&mut self, ui: &mut egui::Ui) {
        Self::page_title(ui, t("система"), t("запуск, трей, язык"));
        ui::group(ui, "язык · language", |ui| {
            // the language names stay in their own language
            ui::setting_row(ui, l("язык интерфейса", "interface language"), "", |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (code, label) in [("en", "english"), ("ru", "русский"), ("", l("как в windows", "system"))] {
                    if ui::filter(ui, label, self.cfg.lang == code).clicked() {
                        self.cfg.lang = code.to_string();
                        crate::i18n::apply(code);
                    }
                }
            });
        });
        ui::group(ui, l("запуск", "startup"), |ui| {
            ui::setting_row(ui, t("запуск вместе с windows"), "", |ui| {
                ui::toggle(ui, &mut self.cfg.autostart);
            });
        });
        ui::group(ui, l("значок в трее", "tray icon"), |ui| {
            for (title, id, value) in [
                (l("левый клик", "left click"), "tray-left", &mut self.cfg.tray_click),
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
        if before.save_dir != self.cfg.save_dir {
            self.disk_bytes = folder_size(&self.cfg.save_dir);
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
        let backspace = c.mods == 0 && c.vk == windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_BACK as u32;
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
                self.disk_bytes = folder_size(&self.cfg.save_dir);
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
        chrome::resize_edges(&ctx);
        self.title_bar(ui);
        self.nav(ui);
        self.content(ui);
        chrome::frame_border(&ctx);
        if let Some(n) = &self.notice
            && !n.show(&ctx, self.content_rect)
        {
            self.notice = None;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(2));
    }
}

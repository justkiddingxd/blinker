//! Video editor for recordings: preview, trimming on a filmstrip timeline,
//! per-track mute / volume (desktop sound and microphone are separate), and
//! saving as MP4 or GIF.

mod engine;
pub mod export;

use crate::chrome;
use crate::config;
use crate::icons::{self, Icon};
use crate::media::{self, MediaInfo};
use crate::record::encoder::HNS;
use crate::runtime::{self, Shared};
use crate::theme::{self, *};
use crate::ui::{self, Notice};
use crate::win;
use eframe::egui::{self, Align2, Color32, CursorIcon, Key, Rect, Sense, Stroke, StrokeKind, TextureHandle, Vec2, pos2, vec2};
use engine::{Engine, Gain, Thumbs};
use export::{Job, Kind as ExportKind};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const THUMBS: usize = 18;
const WAVE_BUCKETS: usize = 1600;
const LABEL_W: f32 = 176.0;
const STRIP_H: f32 = 54.0;
const TRACK_H: f32 = 46.0;

pub fn run(path: PathBuf) {
    let info = match media::probe(&path) {
        Ok(i) if i.video_stream.is_some() && i.duration > 0 => i,
        _ => {
            win::message_box("blinker", crate::i18n::t("не удалось открыть видео."));
            return;
        }
    };
    let shared = Shared::new(config::load());
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let viewport = egui::ViewportBuilder::default()
        .with_title(format!("{name} — blinker"))
        .with_icon(theme::icon_data(64))
        .with_decorations(false)
        .with_inner_size([1120.0, 760.0])
        .with_min_inner_size([820.0, 560.0]);
    let shared2 = shared.clone();
    runtime::run_window(&shared, viewport, true, move |cc| {
        let hwnd = win::hwnd_of(cc);
        if let Some(h) = hwnd {
            win::style_borderless(h);
            win::cloak(h, true);
            win::force_foreground(h);
        }
        VideoApp::new(&cc.egui_ctx, shared2, path, info, hwnd)
    });
}

struct Track {
    stream: u32,
    label: &'static str,
    icon: Icon,
    muted_icon: Icon,
    gain: Gain,
    wave: Arc<Mutex<Option<Vec<f32>>>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Playhead,
    In,
    Out,
}

struct Done {
    path: PathBuf,
    text: String,
}

struct VideoApp {
    shared: Arc<Shared>,
    hwnd: Option<win::HWND>,
    frame_no: u32,
    path: PathBuf,
    info: MediaInfo,
    file_size: u64,
    engine: Engine,
    tex: Option<TextureHandle>,
    tex_seq: u64,
    thumbs: Thumbs,
    thumb_tex: Vec<Option<TextureHandle>>,
    thumb_ver: u64,
    tracks: Vec<Track>,
    playing: bool,
    play_t0: Instant,
    play_from: i64,
    pos: i64,
    trim: (i64, i64),
    drag: Drag,
    mix_down: bool,
    gif_width: u32,
    gif_fps: u32,
    job: Option<Job>,
    done: Option<Done>,
    notice: Option<Notice>,
    preview_rect: Rect,
    upload: Option<Arc<crate::upload::Job>>,
    /// "For Telegram" re-encode in progress.
    tg: Option<Arc<crate::reencode::Job>>,
    tg_mode: config::TgMode,
}

impl VideoApp {
    fn new(ctx: &egui::Context, shared: Arc<Shared>, path: PathBuf, info: MediaInfo, hwnd: Option<win::HWND>) -> Self {
        let streams = info.audio_streams.clone();
        let engine = Engine::new(ctx, &path, (1280, 720), streams.clone());
        engine.show(0);
        let two = streams.len() >= 2;
        let tracks = streams
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let (label, icon, muted_icon) = match (two, i) {
                    (true, 0) => (crate::i18n::t("звук системы"), Icon::Sound, Icon::Mute),
                    (true, 1) => (crate::i18n::t("микрофон"), Icon::Mic, Icon::MicOff),
                    (true, _) => (crate::i18n::t("дорожка"), Icon::Sound, Icon::Mute),
                    (false, _) => (crate::i18n::t("звук"), Icon::Sound, Icon::Mute),
                };
                Track {
                    stream: *s,
                    label,
                    icon,
                    muted_icon,
                    gain: Gain { muted: false, volume: 1.0 },
                    wave: engine::waveform(ctx, &path, *s, info.duration, WAVE_BUCKETS),
                }
            })
            .collect();
        let thumbs = engine::thumbnails(ctx, &path, info.duration, THUMBS, (192, 108));
        let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let gif_width = if info.width > 800 { 800 } else { info.width };
        Self {
            shared,
            hwnd,
            frame_no: 0,
            trim: (0, info.duration),
            path,
            file_size,
            engine,
            tex: None,
            tex_seq: 0,
            thumbs,
            thumb_tex: vec![None; THUMBS],
            thumb_ver: 0,
            tracks,
            playing: false,
            play_t0: Instant::now(),
            play_from: 0,
            pos: 0,
            drag: Drag::None,
            mix_down: true,
            gif_width,
            gif_fps: 15,
            job: None,
            done: None,
            notice: None,
            preview_rect: Rect::NOTHING,
            upload: None,
            tg: None,
            tg_mode: config::load().tg_mode,
            info,
        }
    }

    fn dur(&self) -> i64 {
        self.info.duration.max(1)
    }

    fn trimmed(&self) -> bool {
        self.trim.0 > HNS / 50 || self.trim.1 < self.dur() - HNS / 50
    }

    // ------------------------------------------------------------ playback

    fn play(&mut self) {
        if self.pos >= self.trim.1 - HNS / 20 || self.pos < self.trim.0 {
            self.pos = self.trim.0;
        }
        self.sync_gains();
        self.engine.play(self.pos, self.trim.1);
        self.playing = true;
        self.play_t0 = Instant::now() + std::time::Duration::from_millis(60);
        self.play_from = self.pos;
    }

    fn pause(&mut self) {
        if self.playing {
            self.playing = false;
            self.engine.pause();
            self.engine.show(self.pos);
        }
    }

    fn seek(&mut self, t: i64) {
        if self.playing {
            self.playing = false;
            self.engine.pause();
        }
        self.pos = t.clamp(0, self.dur());
        self.engine.show(self.pos);
    }

    fn sync_gains(&self) {
        let mut g = self.engine.gains.lock().unwrap();
        for (i, t) in self.tracks.iter().enumerate() {
            if let Some(slot) = g.get_mut(i) {
                *slot = t.gain;
            }
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        if self.playing {
            let el = Instant::now().saturating_duration_since(self.play_t0);
            self.pos = (self.play_from + (el.as_nanos() / 100) as i64).min(self.trim.1);
            if self.engine.ended.load(Ordering::SeqCst) || self.pos >= self.trim.1 {
                self.playing = false;
                self.engine.pause();
                self.pos = self.trim.1;
            }
            ctx.request_repaint();
        }
        // newest decoded frame → texture
        let fresh = {
            let g = self.engine.frame.lock().unwrap();
            g.as_ref().filter(|f| f.seq != self.tex_seq).map(|f| (f.seq, theme::color_image(&f.img)))
        };
        if let Some((seq, img)) = fresh {
            self.tex_seq = seq;
            match &mut self.tex {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => self.tex = Some(ctx.load_texture("video-frame", img, egui::TextureOptions::LINEAR)),
            }
        }
        let ver = self.thumbs.version.load(Ordering::SeqCst);
        if ver != self.thumb_ver {
            self.thumb_ver = ver;
            let items = self.thumbs.items.lock().unwrap();
            for (i, it) in items.iter().enumerate() {
                if self.thumb_tex[i].is_none()
                    && let Some(img) = it
                {
                    self.thumb_tex[i] = Some(ctx.load_texture(format!("thumb{i}"), theme::color_image(img), egui::TextureOptions::LINEAR));
                }
            }
        }
    }

    // ------------------------------------------------------------ export

    fn export(&mut self, ctx: &egui::Context, kind: ExportKind) {
        if self.job.is_some() {
            return;
        }
        self.pause();
        let cfg = self.shared.config();
        let opt = export::Options {
            kind,
            from: self.trim.0,
            to: self.trim.1,
            tracks: self
                .tracks
                .iter()
                .filter(|t| !t.gain.muted && t.gain.volume > 0.0)
                .map(|t| (t.stream, t.gain.volume))
                .collect(),
            mix_down: self.mix_down,
            bitrate_quality: cfg.rec_quality,
            gif_width: self.gif_width,
            gif_fps: self.gif_fps,
            quality: None,
            codec: crate::record::encoder::Codec::H264,
        };
        let dst = export::target_path(&self.path, kind, self.trimmed());
        self.done = None;
        self.job = Some(export::start(ctx, self.path.clone(), dst, self.info.clone(), opt));
    }

    fn start_tg(&mut self, ctx: &egui::Context) {
        if self.job.is_some() || self.tg.is_some() {
            return;
        }
        self.pause();
        self.done = None;
        let edit = crate::reencode::Edit {
            from: self.trim.0,
            to: self.trim.1,
            tracks: self
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.gain.muted && t.gain.volume > 0.0)
                .map(|(i, t)| (i, t.gain.volume))
                .collect(),
            mix_down: true,
        };
        let c = ctx.clone();
        self.tg = Some(crate::reencode::start(self.path.clone(), self.tg_mode, Some(edit), move || c.request_repaint()));
        let mode = self.tg_mode;
        self.shared.update_config(|c| c.tg_mode = mode);
    }

    fn poll_tg(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.tg else { return };
        match job.result() {
            None => ctx.request_repaint_after(std::time::Duration::from_millis(150)),
            Some(Ok(path)) => {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                // straight to the clipboard: paste into Telegram
                let copied = crate::output::copy_file(&path);
                let text = format!(
                    "{} → {}{}",
                    human_size(self.file_size),
                    human_size(size),
                    if copied { crate::i18n::t(" · файл в буфере") } else { "" }
                );
                self.done = Some(Done { text, path });
                self.tg = None;
            }
            Some(Err(e)) => {
                self.notice = Some(Notice::new(ctx, if e == crate::i18n::t("отменено") { e } else { lf!("не сжалось: {e}", "compression failed: {e}") }));
                self.tg = None;
            }
        }
    }

    fn poll_job(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else { return };
        let res = job.result.lock().unwrap().take();
        match res {
            None => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            Some(Ok(path)) => {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let what = if job.kind == ExportKind::Gif { crate::i18n::t("gif сохранён") } else { crate::i18n::t("видео сохранено") };
                self.done = Some(Done { text: format!("{what} · {}", human_size(size)), path });
                self.job = None;
            }
            Some(Err(e)) => {
                self.notice = Some(Notice::new(ctx, if e == crate::i18n::t("отменено") { e } else { lf!("не удалось сохранить: {e}", "could not save: {e}") }));
                self.job = None;
            }
        }
    }

    // ------------------------------------------------------------ UI

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("topbar")
            .exact_size(chrome::TITLE_H + 4.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin { left: 14, right: 0, top: 0, bottom: 0 }))
            .show(ui, |ui| {
                let r = ui.max_rect();
                chrome::drag_region(ui, r.expand2(vec2(14.0, 0.0)));
                ui.painter().hline(r.x_range().expand(14.0), r.bottom() + 0.5, Stroke::new(1.0, LINE));
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    chrome::mark(ui);
                    let name = self.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    ui.label(egui::RichText::new(name).font(font_medium(13.0)).color(TEXT));
                    ui.add_space(4.0);
                    let meta = lf!(
                        "{} · {} × {} · {:.0} fps · {}", "{} · {} × {} · {:.0} fps · {}",
                        fmt_time(self.dur(), false),
                        self.info.width,
                        self.info.height,
                        self.info.fps,
                        human_size(self.file_size)
                    );
                    ui.label(egui::RichText::new(meta).font(mono(11.0)).color(FAINT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        chrome::window_buttons(ui, true);
                        ui.spacing_mut().item_spacing.x = 4.0;
                        sep(ui);
                        let busy = self.job.is_some() || self.tg.is_some();
                        ui.add_enabled_ui(!busy, |ui| {
                            if ui::button(ui, crate::i18n::t("сохранить mp4"), ui::Kind::Accent).on_hover_text(crate::i18n::kb("ctrl+s")).clicked() {
                                self.export(&ctx, ExportKind::Mp4);
                            }
                            let tg = ui::button(ui, crate::i18n::t("для telegram"), ui::Kind::Ghost);
                            egui::Popup::menu(&tg).show(|ui| {
                                ui.set_min_width(300.0);
                                ui.spacing_mut().item_spacing.y = 6.0;
                                caption(ui, crate::i18n::t("сжатие"));
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 2.0;
                                    for m in config::TgMode::ALL {
                                        if ui::filter(ui, m.label(), self.tg_mode == m).clicked() {
                                            self.tg_mode = m;
                                        }
                                    }
                                });
                                ui.label(egui::RichText::new(self.tg_mode.desc()).font(font(12.0)).color(DIM));
                                ui.label(
                                    egui::RichText::new(crate::reencode::engine_label(self.tg_mode))
                                        .font(mono(11.0))
                                        .color(FAINT),
                                );
                                ui.label(
                                    egui::RichText::new(crate::i18n::t("с обрезкой и громкостью дорожек, звук в одну дорожку"))
                                        .font(font(12.0))
                                        .color(DIM),
                                );
                                if ui::button(ui, crate::i18n::t("сжать и скопировать"), ui::Kind::Accent).clicked() {
                                    self.start_tg(&ctx);
                                    ui.close();
                                }
                            });
                            let gif = ui::button(ui, "gif", ui::Kind::Ghost);
                            egui::Popup::menu(&gif).show(|ui| {
                                ui.set_min_width(250.0);
                                ui.spacing_mut().item_spacing.y = 6.0;
                                caption(ui, crate::i18n::t("ширина"));
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 2.0;
                                    for w in [480u32, 640, 800] {
                                        let w = w.min(self.info.width);
                                        if ui::filter(ui, &w.to_string(), self.gif_width == w).clicked() {
                                            self.gif_width = w;
                                        }
                                    }
                                    if ui::filter(ui, crate::i18n::t("оригинал"), self.gif_width == self.info.width).clicked() {
                                        self.gif_width = self.info.width;
                                    }
                                });
                                caption(ui, crate::i18n::t("кадров в секунду"));
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 2.0;
                                    for f in [10u32, 15, 24] {
                                        if ui::filter(ui, &f.to_string(), self.gif_fps == f).clicked() {
                                            self.gif_fps = f;
                                        }
                                    }
                                });
                                let secs = (self.trim.1 - self.trim.0) as f64 / HNS as f64;
                                let est = gif_estimate(self.gif_width, &self.info, self.gif_fps, secs);
                                ui.label(
                                    egui::RichText::new(lf!("без звука, около {}", "no sound, about {}", human_size(est)))
                                        .font(font(12.0))
                                        .color(DIM),
                                );
                                if ui::button(ui, crate::i18n::t("сохранить gif"), ui::Kind::Accent).clicked() {
                                    self.export(&ctx, ExportKind::Gif);
                                    ui.close();
                                }
                            });
                        });
                        let more = ui::icon_button(ui, Icon::Settings, crate::i18n::t("ещё"), 30.0, false, ui::Kind::Ghost);
                        egui::Popup::menu(&more).show(|ui| {
                            ui.set_min_width(250.0);
                            if self.tracks.len() > 1 {
                                caption(ui, crate::i18n::t("звук в сохранённом mp4"));
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 2.0;
                                    if ui::filter(ui, crate::i18n::t("одна дорожка"), self.mix_down).clicked() {
                                        self.mix_down = true;
                                    }
                                    if ui::filter(ui, crate::i18n::t("раздельно"), !self.mix_down).clicked() {
                                        self.mix_down = false;
                                    }
                                });
                                let hint = if self.mix_down {
                                    crate::i18n::t("играет везде")
                                } else {
                                    crate::i18n::t("дорожки отдельно, для монтажа")
                                };
                                ui.label(egui::RichText::new(hint).font(font(12.0)).color(DIM));
                                ui.add_space(6.0);
                            }
                            if menu_item(ui, Icon::Copy, crate::i18n::t("копировать исходный файл")) {
                                if crate::output::copy_file(&self.path) {
                                    self.notice = Some(Notice::new(&ctx, crate::i18n::t("файл скопирован")));
                                }
                                ui.close();
                            }
                            if crate::upload::configured(&self.shared.config())
                                && menu_item(ui, Icon::Upload, crate::i18n::t("загрузить исходный и скопировать ссылку"))
                            {
                                self.upload = Some(crate::upload::start(&self.shared.config(), self.path.clone(), None));
                                ui.close();
                            }
                            if menu_item(ui, Icon::Folder, crate::i18n::t("показать в папке")) {
                                win::reveal_in_explorer(&self.path);
                                ui.close();
                            }
                        });
                    });
                });
            });
    }

    fn timeline(&mut self, ui: &mut egui::Ui) {
        let h = 16.0 + 40.0 + 8.0 + STRIP_H + 10.0 + self.tracks.len() as f32 * (TRACK_H + 4.0) + 12.0;
        egui::Panel::bottom("timeline")
            .exact_size(h)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin { left: 14, right: 16, top: 10, bottom: 12 }))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().hline(egui::Rangef::new(r.left() - 14.0, r.right() + 16.0), r.top() - 10.5, Stroke::new(1.0, LINE));
                self.transport(ui);
                ui.add_space(8.0);
                let top = ui.cursor().top();
                let x0 = r.left() + LABEL_W;
                let x1 = r.right();
                let strip = Rect::from_min_max(pos2(x0, top), pos2(x1, top + STRIP_H));
                let tracks_top = strip.bottom() + 10.0;
                let area = Rect::from_min_max(
                    pos2(x0, strip.top()),
                    pos2(x1, tracks_top + self.tracks.len() as f32 * (TRACK_H + 4.0)),
                );
                // left labels
                let p = ui.painter().clone();
                p.text(pos2(r.left() + 2.0, strip.center().y - 8.0), Align2::LEFT_CENTER, crate::i18n::t("видео"), font_medium(13.0), TEXT2);
                let sub = format!("{} × {}", self.info.width, self.info.height);
                p.text(pos2(r.left() + 2.0, strip.center().y + 10.0), Align2::LEFT_CENTER, sub, mono(11.0), FAINT);
                self.filmstrip(&p, strip);
                for i in 0..self.tracks.len() {
                    let y = tracks_top + i as f32 * (TRACK_H + 4.0);
                    let row = Rect::from_min_max(pos2(r.left(), y), pos2(x1, y + TRACK_H));
                    self.track_row(ui, i, row, x0);
                }
                self.timeline_input(ui, area, strip);
                self.paint_trim_and_playhead(&p, strip, area);
            });
    }

    fn x_of(&self, t: i64, area: Rect) -> f32 {
        area.left() + area.width() * (t as f64 / self.dur() as f64) as f32
    }

    fn t_of(&self, x: f32, area: Rect) -> i64 {
        (((x - area.left()) / area.width()).clamp(0.0, 1.0) as f64 * self.dur() as f64) as i64
    }

    fn transport(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.set_height(40.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            let (icon, tip) = if self.playing { (Icon::Pause, crate::i18n::t("пауза  пробел")) } else { (Icon::Play, crate::i18n::t("играть  пробел")) };
            if ui::icon_button(ui, icon, tip, 36.0, false, ui::Kind::Accent).clicked() {
                if self.playing { self.pause() } else { self.play() }
            }
            ui.add_space(4.0);
            let now = egui::RichText::new(fmt_time(self.pos, true)).font(mono(14.0)).color(TEXT);
            ui.label(now);
            ui.label(egui::RichText::new(format!("/ {}", fmt_time(self.dur(), true))).font(mono(12.0)).color(FAINT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if self.trimmed() {
                    if ui::button(ui, crate::i18n::t("сбросить"), ui::Kind::Ghost).on_hover_text(crate::i18n::t("вернуть всю длину")).clicked() {
                        self.trim = (0, self.dur());
                    }
                    let len = fmt_time(self.trim.1 - self.trim.0, true);
                    ui.label(egui::RichText::new(lf!("фрагмент {len}", "clip {len}")).font(mono(12.0)).color(ACCENT));
                } else {
                    ui.label(
                        egui::RichText::new(crate::i18n::t("края полосы обрезают · i и o ставят начало и конец"))
                            .font(font(12.0))
                            .color(FAINT),
                    );
                }
            });
        });
    }

    fn filmstrip(&self, p: &egui::Painter, strip: Rect) {
        p.rect_filled(strip, 3, CANVAS);
        let n = self.thumb_tex.len().max(1);
        let w = strip.width() / n as f32;
        for (i, t) in self.thumb_tex.iter().enumerate() {
            let cell = Rect::from_min_size(pos2(strip.left() + i as f32 * w, strip.top()), vec2(w, strip.height()));
            if let Some(tex) = t {
                let uv = ui::cover_uv(cell.size(), tex.size_vec2());
                p.image(tex.id(), cell, uv, Color32::WHITE);
            }
        }
    }

    fn track_row(&mut self, ui: &mut egui::Ui, i: usize, row: Rect, x0: f32) {
        let p = ui.painter().clone();
        let wave_rect = Rect::from_min_max(pos2(x0, row.top()), row.max);
        p.rect_filled(wave_rect, 3, CANVAS);
        // controls on the left
        let t = &self.tracks[i];
        let (label, icon, muted_icon, muted) = (t.label, t.icon, t.muted_icon, t.gain.muted);
        let btn = Rect::from_min_size(pos2(row.left(), row.center().y - 15.0), vec2(30.0, 30.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(btn));
        let tip = if muted { crate::i18n::t("включить звук дорожки") } else { crate::i18n::t("убрать звук дорожки") };
        if ui::icon_button(&mut child, if muted { muted_icon } else { icon }, tip, 30.0, false, ui::Kind::Ghost).clicked() {
            self.tracks[i].gain.muted = !muted;
            self.sync_gains();
        }
        let col = if muted { FAINT } else { TEXT2 };
        p.text(pos2(row.left() + 38.0, row.top() + 12.0), Align2::LEFT_CENTER, label, font_medium(12.5), col);
        let slider = Rect::from_min_max(pos2(row.left() + 36.0, row.top() + 22.0), pos2(x0 - 12.0, row.bottom() - 2.0));
        let mut sc = ui.new_child(egui::UiBuilder::new().max_rect(slider).layout(egui::Layout::left_to_right(egui::Align::Center)));
        sc.spacing_mut().slider_width = slider.width() - 44.0;
        sc.spacing_mut().item_spacing.x = 6.0;
        sc.add_enabled_ui(!muted, |ui| {
            let mut v = self.tracks[i].gain.volume;
            let resp = ui.add(egui::Slider::new(&mut v, 0.0..=2.0).show_value(false));
            ui.label(egui::RichText::new(format!("{:.0}%", v * 100.0)).font(mono(10.5)).color(FAINT));
            if resp.changed() {
                self.tracks[i].gain.volume = v;
                self.sync_gains();
            }
        });
        // waveform
        let wave = self.tracks[i].wave.lock().unwrap().clone();
        match wave {
            Some(peaks) => {
                let color = if muted { LINE } else { theme::lerp_color(TEXT2, ACCENT, 0.3) };
                let vol = if muted { 1.0 } else { self.tracks[i].gain.volume.min(1.6) };
                let mid = wave_rect.center().y;
                let half = wave_rect.height() / 2.0 - 3.0;
                let cols = wave_rect.width().max(1.0) as usize;
                let mut mesh = egui::Mesh::default();
                for c in 0..cols {
                    let a = c * peaks.len() / cols;
                    let b = ((c + 1) * peaks.len() / cols).max(a + 1).min(peaks.len());
                    let pk = peaks[a..b].iter().cloned().fold(0.0f32, f32::max);
                    let hgt = (pk.sqrt() * vol).min(1.0) * half;
                    let x = wave_rect.left() + c as f32;
                    mesh.add_colored_rect(Rect::from_min_max(pos2(x, mid - hgt - 0.5), pos2(x + 1.0, mid + hgt + 0.5)), color);
                }
                p.add(egui::Shape::mesh(mesh));
            }
            None => {
                p.text(wave_rect.center(), Align2::CENTER_CENTER, crate::i18n::t("читаю звук…"), font(11.5), FAINT);
            }
        }
    }

    fn timeline_input(&mut self, ui: &mut egui::Ui, area: Rect, strip: Rect) {
        let resp = ui.interact(area, ui.scope_id().with("timeline"), Sense::click_and_drag());
        let near = |x: f32, px: f32| (x - px).abs() <= 8.0;
        let (xi, xo) = (self.x_of(self.trim.0, area), self.x_of(self.trim.1, area));
        if let Some(pt) = resp.hover_pos() {
            let on_strip = pt.y <= strip.bottom() + 4.0;
            if on_strip && (near(pt.x, xi) || near(pt.x, xo)) || matches!(self.drag, Drag::In | Drag::Out) {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            } else {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
        }
        if resp.drag_started() || resp.clicked() {
            if let Some(pt) = resp.interact_pointer_pos() {
                let on_strip = pt.y <= strip.bottom() + 4.0;
                self.drag = if on_strip && near(pt.x, xi) && (pt.x <= xi || !near(pt.x, xo)) {
                    Drag::In
                } else if on_strip && near(pt.x, xo) {
                    Drag::Out
                } else {
                    Drag::Playhead
                };
            }
        }
        if let Some(pt) = resp.interact_pointer_pos()
            && (resp.dragged() || resp.clicked() || resp.drag_started())
        {
            let t = self.t_of(pt.x, area);
            let min_len = HNS / 5;
            match self.drag {
                Drag::In => {
                    self.trim.0 = t.min(self.trim.1 - min_len).max(0);
                    self.seek(self.trim.0);
                }
                Drag::Out => {
                    self.trim.1 = t.max(self.trim.0 + min_len).min(self.dur());
                    self.seek(self.trim.1);
                }
                Drag::Playhead => self.seek(t),
                Drag::None => {}
            }
        }
        if resp.drag_stopped() || resp.clicked() {
            self.drag = Drag::None;
        }
    }

    fn paint_trim_and_playhead(&self, p: &egui::Painter, strip: Rect, area: Rect) {
        let (xi, xo) = (self.x_of(self.trim.0, area), self.x_of(self.trim.1, area));
        let shade = Color32::from_black_alpha(165);
        if xi > area.left() {
            p.rect_filled(Rect::from_min_max(area.min, pos2(xi, area.bottom())), 0, shade);
        }
        if xo < area.right() {
            p.rect_filled(Rect::from_min_max(pos2(xo, area.top()), area.max), 0, shade);
        }
        // selected range on the filmstrip: accent frame with grips
        let sel = Rect::from_min_max(pos2(xi, strip.top()), pos2(xo, strip.bottom()));
        p.rect_stroke(sel, 3, Stroke::new(2.0, ACCENT), StrokeKind::Inside);
        for (x, left) in [(xi, true), (xo, false)] {
            let g = if left {
                Rect::from_min_max(pos2(x, strip.top()), pos2(x + 9.0, strip.bottom()))
            } else {
                Rect::from_min_max(pos2(x - 9.0, strip.top()), pos2(x, strip.bottom()))
            };
            p.rect_filled(g, 3, ACCENT);
            let cx = g.center().x;
            for dy in [-5.0, 0.0, 5.0] {
                p.hline((cx - 2.0)..=(cx + 2.0), g.center().y + dy, Stroke::new(1.0, ON_ACCENT));
            }
        }
        // playhead
        let x = self.x_of(self.pos, area).round() + 0.5;
        p.vline(x, (area.top() - 4.0)..=area.bottom(), Stroke::new(2.0, TEXT));
        let knob = Rect::from_center_size(pos2(x, area.top() - 5.0), vec2(10.0, 8.0));
        p.rect_filled(knob, 2, TEXT);
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let r = ui.max_rect();
        self.preview_rect = r;
        let p = ui.painter().clone();
        p.rect_filled(r, 0, CANVAS);
        let area = r.shrink(18.0);
        let aspect = self.info.width.max(1) as f32 / self.info.height.max(1) as f32;
        let mut size = vec2(area.width(), area.width() / aspect);
        if size.y > area.height() {
            size = vec2(area.height() * aspect, area.height());
        }
        let ppp = ui.ctx().pixels_per_point();
        // Never upscale beyond 1:1 physical pixels.
        let max = vec2(self.info.width as f32, self.info.height as f32) / ppp;
        if size.x > max.x {
            size = max;
        }
        let rect = Rect::from_center_size(area.center(), size);
        let resp = ui.interact(rect, ui.scope_id().with("preview"), Sense::click());
        if let Some(tex) = &self.tex {
            p.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        p.rect_stroke(rect, 0, Stroke::new(1.0, LINE), StrokeKind::Outside);
        if resp.clicked() && self.job.is_none() {
            if self.playing { self.pause() } else { self.play() }
        }
        if !self.playing && self.job.is_none() && self.tg.is_none() {
            let h = resp.hovered();
            let c = rect.center();
            let a = if h { 1.0 } else { 0.75 };
            p.circle_filled(c, 28.0, Color32::from_rgba_unmultiplied(18, 18, 17, (215.0 * a) as u8));
            icons::paint(&p, Rect::from_center_size(c + vec2(2.0, 0.0), Vec2::splat(26.0)), Icon::Play, 22.0, TEXT.gamma_multiply(a));
            if h {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
        }
        self.job_overlay(ui, rect);
        self.done_card(ui, r);
    }

    fn job_overlay(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let (what, f, cancel, engine) = match (&self.job, &self.tg) {
            (Some(job), _) => {
                let w = if job.kind == ExportKind::Gif { crate::i18n::t("сохраняю gif") } else { crate::i18n::t("сохраняю видео") };
                (w.to_string(), job.fraction(), job.cancel.clone(), None)
            }
            (None, Some(tg)) => (
                lf!("сжимаю для telegram · {}", "compressing for telegram · {}", self.tg_mode.label()),
                tg.fraction(),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                Some(tg.engine.lock().unwrap().clone()),
            ),
            _ => return,
        };
        let p = ui.painter().clone();
        p.rect_filled(rect, 0, Color32::from_black_alpha(150));
        let card = Rect::from_center_size(rect.center(), vec2(360.0, 104.0));
        p.rect_filled(card, 4, PANEL);
        p.rect_stroke(card, 4, Stroke::new(1.0, LINE), StrokeKind::Inside);
        if let Some(e) = &engine {
            p.text(card.left_bottom() + vec2(16.0, -27.0), Align2::LEFT_CENTER, e, mono(10.5), FAINT);
        }
        p.text(card.left_top() + vec2(16.0, 22.0), Align2::LEFT_CENTER, what, font_medium(13.5), TEXT);
        p.text(card.right_top() + vec2(-16.0, 22.0), Align2::RIGHT_CENTER, format!("{:.0}%", f * 100.0), mono(12.0), DIM);
        let bar = Rect::from_min_size(card.left_top() + vec2(16.0, 42.0), vec2(card.width() - 32.0, 4.0));
        p.rect_filled(bar, 2, LINE);
        p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * f, bar.height())), 2, ACCENT);
        let btn = Rect::from_min_size(pos2(card.right() - 16.0 - 80.0, card.bottom() - 42.0), vec2(80.0, 30.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(btn));
        if ui::button(&mut child, crate::i18n::t("отмена"), ui::Kind::Ghost).clicked() {
            cancel.store(true, Ordering::SeqCst);
            if let Some(tg) = &self.tg {
                tg.cancel.store(true, Ordering::SeqCst);
            }
        }
    }

    fn done_card(&mut self, ui: &mut egui::Ui, r: Rect) {
        let Some(done) = &self.done else { return };
        let card = Rect::from_min_size(r.right_bottom() - vec2(16.0 + 380.0, 16.0 + 50.0), vec2(380.0, 50.0));
        let p = ui.painter().clone();
        p.rect_filled(card, 4, PANEL);
        p.rect_stroke(card, 4, Stroke::new(1.0, LINE), StrokeKind::Inside);
        p.text(pos2(card.left() + 14.0, card.center().y), Align2::LEFT_CENTER, &done.text, font_medium(13.0), TEXT);
        let path = done.path.clone();
        let mut close = false;
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(card.shrink2(vec2(8.0, 10.0)))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = 2.0;
        if ui::icon_button(&mut child, Icon::Close, crate::i18n::t("скрыть"), 30.0, false, ui::Kind::Ghost).clicked() {
            close = true;
        }
        if ui::icon_button(&mut child, Icon::Folder, crate::i18n::t("показать в папке"), 30.0, false, ui::Kind::Ghost).clicked() {
            win::reveal_in_explorer(&path);
        }
        if crate::upload::configured(&self.shared.config())
            && ui::icon_button(&mut child, Icon::Upload, crate::i18n::t("загрузить и скопировать ссылку"), 30.0, false, ui::Kind::Ghost).clicked()
        {
            self.upload = Some(crate::upload::start(&self.shared.config(), path.clone(), None));
        }
        if ui::icon_button(&mut child, Icon::Copy, crate::i18n::t("копировать файл"), 30.0, false, ui::Kind::Ghost).clicked()
            && crate::output::copy_file(&path)
        {
            self.notice = Some(Notice::new(ui.ctx(), crate::i18n::t("файл скопирован")));
        }
        if close {
            self.done = None;
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.job.is_some() || self.tg.is_some() {
            return;
        }
        let (cmd, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        if pressed(Key::Space) {
            if self.playing { self.pause() } else { self.play() }
        }
        if cmd && pressed(Key::S) {
            self.export(ctx, ExportKind::Mp4);
        }
        let step = if shift { 5 * HNS } else { HNS };
        if pressed(Key::ArrowLeft) {
            self.seek(self.pos - step);
        }
        if pressed(Key::ArrowRight) {
            self.seek(self.pos + step);
        }
        if pressed(Key::Home) {
            self.seek(self.trim.0);
        }
        if pressed(Key::End) {
            self.seek(self.trim.1);
        }
        if pressed(Key::I) && self.pos < self.trim.1 - HNS / 5 {
            self.trim.0 = self.pos;
        }
        if pressed(Key::O) && self.pos > self.trim.0 + HNS / 5 {
            self.trim.1 = self.pos;
        }
    }
}

impl eframe::App for VideoApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        CANVAS.to_normalized_gamma_f32()
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
        if ctx.input(|i| i.viewport().close_requested()) && (self.job.is_some() || self.tg.is_some()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.notice = Some(Notice::new(&ctx, crate::i18n::t("ещё сохраняется")));
        }
        self.tick(&ctx);
        self.poll_job(&ctx);
        self.poll_tg(&ctx);
        if let Some(job) = &self.upload {
            match job.result() {
                None => {
                    let pct = job.fraction() * 100.0;
                    if self.notice.is_none() || ctx.input(|i| i.time) as i64 % 1 == 0 {
                        self.notice = Some(Notice::new(&ctx, lf!("загрузка {pct:.0}%", "uploading {pct:.0}%")));
                    }
                    ctx.request_repaint_after(std::time::Duration::from_millis(200));
                }
                Some(Ok(_)) => {
                    self.notice = Some(Notice::new(&ctx, crate::i18n::t("ссылка скопирована")));
                    self.upload = None;
                }
                Some(Err(e)) => {
                    self.notice = Some(Notice::new(&ctx, lf!("не загрузилось: {e}", "upload failed: {e}")));
                    self.upload = None;
                }
            }
        }
        chrome::resize_edges(&ctx);
        self.top_bar(ui);
        self.timeline(ui);
        self.shortcuts(&ctx);
        egui::CentralPanel::no_frame().show(ui, |ui| self.preview(ui));
        chrome::frame_border(&ctx);
        if let Some(n) = &self.notice
            && !n.show(&ctx, self.preview_rect)
        {
            self.notice = None;
        }
    }
}

// ---------------------------------------------------------------------------

fn fmt_time(t: i64, fraction: bool) -> String {
    let t = t.max(0);
    let secs = t / HNS;
    let (m, s) = (secs / 60, secs % 60);
    if fraction {
        format!("{m}:{s:02}.{}", (t % HNS) / (HNS / 10))
    } else {
        format!("{m}:{s:02}")
    }
}

fn human_size(b: u64) -> String {
    let kb = b as f64 / 1024.0;
    if kb < 1000.0 {
        lf!("{kb:.0} кб", "{kb:.0} kb")
    } else if kb < 1024.0 * 1000.0 {
        lf!("{:.1} мб", "{:.1} mb", kb / 1024.0).replace('.', ",")
    } else {
        lf!("{:.2} гб", "{:.2} gb", kb / 1024.0 / 1024.0).replace('.', ",")
    }
}

/// Very rough GIF size guess, so 4K-at-24-fps surprises show up front.
fn gif_estimate(width: u32, info: &MediaInfo, fps: u32, secs: f64) -> u64 {
    let h = width as f64 * info.height as f64 / info.width.max(1) as f64;
    (width as f64 * h * 0.12 * fps as f64 * secs) as u64
}

fn sep(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 30.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.shrink2(vec2(0.0, 6.0)).y_range(), Stroke::new(1.0, LINE));
}

fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(font_medium(12.0)).color(TEXT2));
}

fn menu_item(ui: &mut egui::Ui, icon: Icon, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(230.0), 28.0), Sense::click());
    let h = resp.hovered();
    if h {
        ui.painter().rect_filled(rect, 3, HOVER);
    }
    let ir = Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(16.0, 16.0));
    icons::paint(ui.painter(), ir, icon, 14.0, if h { ACCENT } else { DIM });
    ui.painter().text(pos2(rect.left() + 32.0, rect.center().y), Align2::LEFT_CENTER, label, font(13.0), TEXT);
    resp.clicked()
}

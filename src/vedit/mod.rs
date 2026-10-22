//! Video editor for recordings: preview, trimming on a filmstrip timeline,
//! per-track mute / volume (desktop sound and microphone are separate),
//! markers with notes (they go along with an upload), and saving as MP4,
//! GIF or, with the video track off, the sound alone (MP3 / M4A / WAV).

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
use export::{AudioFormat, Job, Kind as ExportKind};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const THUMBS: usize = 18;
const WAVE_BUCKETS: usize = 1600;
const LABEL_W: f32 = 176.0;
const STRIP_H: f32 = 54.0;
const TRACK_H: f32 = 46.0;
/// The lane of markers above the filmstrip.
const MARK_H: f32 = 20.0;
/// Trim grips: their width, and how far outside the range they still grab.
const GRIP_W: f32 = 12.0;
const GRIP_REACH: f32 = 14.0;

thread_local! {
    /// The window's width, for the glass title bar (macOS 26).
    static GLASS_W: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

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
        .with_inner_size(window_size())
        .with_transparent(theme::glass());
    GLASS_W.with(|w| w.set(window_size()[0]));
    let viewport = chrome::style(viewport);
    let shared2 = shared.clone();
    runtime::run_window(&shared, viewport, true, move |cc| {
        let hwnd = win::hwnd_of(cc);
        if let Some(h) = hwnd {
            win::style_borderless(h);
            chrome::place_window_buttons(h, chrome::TITLE_H + 4.0);
            chrome::glass_bar(h, GLASS_W.with(|w| w.get()), chrome::TITLE_H + 4.0);
            win::cloak(h, true);
            win::force_foreground(h);
        }
        VideoApp::new(&cc.egui_ctx, shared2, path, info, hwnd)
    });
}

/// 1120 × 760, or less on a small screen (the window can't be resized).
fn window_size() -> [f32; 2] {
    let (x, y) = win::cursor_pos();
    let mon = win::monitor_at(&win::monitors(), x, y);
    let (work, scale) = mon.map(|m| (m.work, m.scale)).unwrap_or((win::IRect::new(0, 0, 1920, 1040), 1.0));
    let avail = (work.w as f32 / scale * 0.94, work.h as f32 / scale * 0.92);
    [1120f32.min(avail.0).max(820.0), 760f32.min(avail.1).max(560.0)]
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
    /// The markers inside the saved clip, retimed to it (for an upload).
    markers: Vec<crate::upload::Marker>,
}

/// A point on the timeline with a note.
#[derive(Clone)]
struct Marker {
    /// Source time.
    t: i64,
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
    /// The video track is kept; off = the sound alone is saved.
    video_on: bool,
    audio_format: AudioFormat,
    markers: Vec<Marker>,
    /// The marker whose note is being edited.
    marker_edit: Option<usize>,
    /// A marker being dragged along the timeline.
    marker_drag: Option<usize>,
    /// The note field takes the keyboard on its next frame.
    marker_focus: bool,
    /// "Upload" on an edited clip: it is saved first, then the saved file goes up.
    upload_after_save: bool,
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
            video_on: true,
            audio_format: AudioFormat::Mp3,
            markers: Vec::new(),
            marker_edit: None,
            marker_drag: None,
            marker_focus: false,
            upload_after_save: false,
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

    /// Anything changed: what is uploaded has to be saved first.
    fn edited(&self) -> bool {
        self.trimmed() || !self.video_on || self.tracks.iter().any(|t| t.gain.muted || (t.gain.volume - 1.0).abs() > 0.01)
    }

    /// "Upload": the source as it is, or the edited clip once it's saved.
    fn upload(&mut self, ctx: &egui::Context) {
        if self.upload.is_some() || self.job.is_some() {
            return;
        }
        if self.edited() {
            self.upload_after_save = true;
            self.export(ctx, self.save_kind());
        } else {
            let markers = self.source_markers();
            self.upload = Some(crate::upload::start_with(&self.shared.config(), self.path.clone(), None, markers));
        }
    }

    /// The primary "save": MP4, or the sound alone with the video track off.
    fn save_kind(&self) -> ExportKind {
        if self.video_on { ExportKind::Mp4 } else { ExportKind::Audio(self.audio_format) }
    }

    /// Markers inside the trimmed range, in seconds from its start.
    fn clip_markers(&self) -> Vec<crate::upload::Marker> {
        let mut out: Vec<_> = self
            .markers
            .iter()
            .filter(|m| m.t >= self.trim.0 && m.t <= self.trim.1)
            .map(|m| crate::upload::Marker { t: (m.t - self.trim.0) as f64 / HNS as f64, text: m.text.trim().to_string() })
            .collect();
        out.sort_by(|a, b| a.t.total_cmp(&b.t));
        out
    }

    /// Markers of the whole source file, in seconds.
    fn source_markers(&self) -> Vec<crate::upload::Marker> {
        let mut out: Vec<_> = self
            .markers
            .iter()
            .map(|m| crate::upload::Marker { t: m.t as f64 / HNS as f64, text: m.text.trim().to_string() })
            .collect();
        out.sort_by(|a, b| a.t.total_cmp(&b.t));
        out
    }

    fn add_marker(&mut self) {
        // one marker per moment: a second one nearby is the same marker
        self.marker_focus = true;
        if let Some(i) = self.markers.iter().position(|m| (m.t - self.pos).abs() < HNS / 10) {
            self.marker_edit = Some(i);
            return;
        }
        self.markers.push(Marker { t: self.pos, text: String::new() });
        self.marker_edit = Some(self.markers.len() - 1);
    }

    fn poll_job(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else { return };
        let res = job.result.lock().unwrap().take();
        match res {
            None => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            Some(Ok(path)) => {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let what = match job.kind {
                    ExportKind::Gif => crate::i18n::t("gif сохранён"),
                    ExportKind::Audio(_) => crate::i18n::l("звук сохранён", "sound saved"),
                    ExportKind::Mp4 => crate::i18n::t("видео сохранено"),
                };
                let markers = if job.kind == ExportKind::Mp4 { self.clip_markers() } else { Vec::new() };
                if std::mem::take(&mut self.upload_after_save) {
                    self.upload = Some(crate::upload::start_with(&self.shared.config(), path.clone(), None, markers.clone()));
                }
                self.done = Some(Done { text: format!("{what} · {}", human_size(size)), path, markers });
                self.job = None;
            }
            Some(Err(e)) => {
                self.notice = Some(Notice::new(ctx, if e == crate::i18n::t("отменено") { e } else { lf!("не удалось сохранить: {e}", "could not save: {e}") }));
                self.job = None;
                self.upload_after_save = false;
            }
        }
    }

    // ------------------------------------------------------------ UI

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("topbar")
            .exact_size(chrome::TITLE_H + 4.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(chrome::bar_fill()).inner_margin(egui::Margin { left: 14, right: 0, top: 0, bottom: 0 }))
            .show(ui, |ui| {
                let r = ui.max_rect();
                chrome::drag_region(ui, r.expand2(vec2(14.0, 0.0)));
                if !theme::glass() {
                    ui.painter().hline(r.x_range().expand(14.0), r.bottom() + 0.5, Stroke::new(1.0, LINE));
                }
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    chrome::leading_space(ui);
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
                        chrome::window_buttons(ui);
                        ui.spacing_mut().item_spacing.x = 4.0;
                        sep(ui);
                        let busy = self.job.is_some();
                        ui.add_enabled_ui(!busy, |ui| {
                            if !self.video_on {
                                let label = lf!("сохранить {}", "save {}", self.audio_format.ext());
                                if ui::button(ui, &label, ui::Kind::Accent).on_hover_text(crate::i18n::kb("ctrl+s")).clicked() {
                                    self.export(&ctx, self.save_kind());
                                }
                                self.upload_button(ui, &ctx);
                                let fmt = ui::button(ui, crate::i18n::l("формат", "format"), ui::Kind::Ghost);
                                egui::Popup::menu(&fmt).show(|ui| {
                                    ui.set_min_width(260.0);
                                    ui.spacing_mut().item_spacing.y = 6.0;
                                    caption(ui, crate::i18n::l("только звук", "sound only"));
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 2.0;
                                        for f in AudioFormat::ALL {
                                            if ui::filter(ui, f.ext(), self.audio_format == f).clicked() {
                                                self.audio_format = f;
                                            }
                                        }
                                    });
                                    ui.label(egui::RichText::new(self.audio_format.desc()).font(font(12.0)).color(DIM));
                                    ui.label(
                                        egui::RichText::new(crate::i18n::l(
                                            "включённые дорожки сводятся в одну, с их громкостью",
                                            "the tracks that are on are mixed into one, at their volume",
                                        ))
                                        .font(font(12.0))
                                        .color(DIM),
                                    );
                                });
                                return;
                            }
                            if ui::button(ui, crate::i18n::t("сохранить mp4"), ui::Kind::Accent).on_hover_text(crate::i18n::kb("ctrl+s")).clicked() {
                                self.export(&ctx, ExportKind::Mp4);
                            }
                            self.upload_button(ui, &ctx);
                            sep(ui);
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
                                let markers = self.source_markers();
                                self.upload = Some(crate::upload::start_with(&self.shared.config(), self.path.clone(), None, markers));
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

    /// "Upload" next to "save" (when uploads are on).
    fn upload_button(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !crate::upload::configured(&self.shared.config()) {
            return;
        }
        let tip = if self.edited() {
            crate::i18n::l("сохранить правку, загрузить и скопировать ссылку · метки тоже", "save the edit, upload it and copy the link · markers too")
        } else {
            crate::i18n::l("загрузить и скопировать ссылку · метки тоже", "upload and copy the link · markers too")
        };
        ui.add_enabled_ui(self.upload.is_none(), |ui| {
            if ui::button(ui, crate::i18n::l("загрузить", "upload"), ui::Kind::Ghost).on_hover_text(tip).clicked() {
                self.upload(ctx);
            }
        });
    }

    fn timeline(&mut self, ui: &mut egui::Ui) {
        let h = 16.0 + 40.0 + 8.0 + MARK_H + 4.0 + STRIP_H + 10.0 + self.tracks.len() as f32 * (TRACK_H + 4.0) + 12.0;
        egui::Panel::bottom("timeline")
            .exact_size(h)
            .frame(egui::Frame::new().fill(ui::pane_fill()).inner_margin(egui::Margin { left: 14, right: 16, top: 10, bottom: 12 }))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().hline(egui::Rangef::new(r.left() - 14.0, r.right() + 16.0), r.top() - 10.5, Stroke::new(1.0, ui::hairline()));
                self.transport(ui);
                ui.add_space(8.0);
                let lane_top = ui.cursor().top();
                let x0 = r.left() + LABEL_W;
                let x1 = r.right();
                let lane = Rect::from_min_max(pos2(x0, lane_top), pos2(x1, lane_top + MARK_H));
                let top = lane.bottom() + 4.0;
                let strip = Rect::from_min_max(pos2(x0, top), pos2(x1, top + STRIP_H));
                let tracks_top = strip.bottom() + 10.0;
                let area = Rect::from_min_max(
                    pos2(x0, strip.top()),
                    pos2(x1, tracks_top + self.tracks.len() as f32 * (TRACK_H + 4.0)),
                );
                // left labels
                let p = ui.painter().clone();
                self.video_label(ui, Rect::from_min_max(pos2(r.left(), strip.top()), pos2(x0, strip.bottom())));
                self.filmstrip(&p, strip);
                if !self.video_on {
                    p.rect_filled(strip, 3, Color32::from_black_alpha(170));
                    p.text(strip.center(), Align2::CENTER_CENTER, crate::i18n::l("видео выключено: сохранится только звук", "video off: only the sound is saved"), font(12.0), DIM);
                }
                for i in 0..self.tracks.len() {
                    let y = tracks_top + i as f32 * (TRACK_H + 4.0);
                    let row = Rect::from_min_max(pos2(r.left(), y), pos2(x1, y + TRACK_H));
                    self.track_row(ui, i, row, x0);
                }
                self.timeline_input(ui, area, strip);
                self.paint_trim_and_playhead(&p, strip, area);
                let label = Rect::from_min_max(pos2(r.left(), lane.top()), pos2(x0 - 12.0, lane.bottom()));
                self.markers_lane(ui, lane, label, area);
            });
    }

    /// "video" with its size, and the switch that leaves it out of the save.
    fn video_label(&mut self, ui: &mut egui::Ui, row: Rect) {
        let p = ui.painter().clone();
        let btn = Rect::from_min_size(pos2(row.left(), row.center().y - 15.0), vec2(30.0, 30.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(btn));
        let (icon, tip) = if self.video_on {
            (Icon::Video, crate::i18n::l("убрать видео: сохранить только звук", "drop the video: save the sound alone"))
        } else {
            (Icon::VideoOff, crate::i18n::l("вернуть видео", "bring the video back"))
        };
        if ui::icon_button(&mut child, icon, tip, 30.0, false, ui::Kind::Ghost).clicked() {
            self.video_on = !self.video_on;
        }
        let col = if self.video_on { TEXT2 } else { FAINT };
        p.text(pos2(row.left() + 38.0, row.center().y - 8.0), Align2::LEFT_CENTER, crate::i18n::t("видео"), font_medium(12.5), col);
        let sub = if self.video_on { format!("{} × {}", self.info.width, self.info.height) } else { crate::i18n::l("выключено", "off").to_string() };
        p.text(pos2(row.left() + 38.0, row.center().y + 9.0), Align2::LEFT_CENTER, sub, mono(11.0), FAINT);
    }

    /// Markers: flags above the filmstrip with a line through the tracks.
    /// Click = go there and edit the note, drag = move, M adds one.
    fn markers_lane(&mut self, ui: &mut egui::Ui, lane: Rect, label: Rect, area: Rect) {
        let p = ui.painter().clone();
        // the left label is the "add" button
        let add = ui.interact(label, ui.scope_id().with("marker-add"), Sense::click());
        let h = add.hovered();
        icons::paint(&p, Rect::from_center_size(pos2(label.left() + 15.0, label.center().y), Vec2::splat(14.0)), Icon::Flag, 13.0, if h { ACCENT } else { FAINT });
        let text = if self.markers.is_empty() { crate::i18n::l("метка · m", "marker · m") } else { crate::i18n::l("+ метка · m", "+ marker · m") };
        p.text(pos2(label.left() + 38.0, label.center().y), Align2::LEFT_CENTER, text, font(12.0), if h { TEXT } else { FAINT });
        if add.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(crate::i18n::l("метка на месте курсора", "a marker at the playhead")).clicked() {
            self.add_marker();
        }
        p.rect_filled(lane, 3, CANVAS);
        let mut order: Vec<usize> = (0..self.markers.len()).collect();
        order.sort_by_key(|&i| self.markers[i].t);
        let mut clicked = None;
        for i in order {
            let x = self.x_of(self.markers[i].t, area).round() + 0.5;
            let selected = self.marker_edit == Some(i);
            let flag = Rect::from_min_size(pos2(x - 1.0, lane.top() + 3.0), vec2(12.0, lane.height() - 6.0));
            let resp = ui.interact(flag.expand2(vec2(4.0, 2.0)), ui.scope_id().with(("marker", i)), Sense::click_and_drag());
            let hot = selected || resp.hovered() || self.marker_drag == Some(i);
            let col = if hot { ACCENT } else { theme::lerp_color(DIM, ACCENT, 0.4) };
            p.vline(x, lane.top()..=area.bottom(), Stroke::new(1.0, col.gamma_multiply(if hot { 0.8 } else { 0.35 })));
            let pts = vec![flag.left_top(), flag.right_top(), pos2(flag.right() - 3.0, flag.center().y), flag.right_bottom(), flag.left_bottom()];
            p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
            if resp.drag_started() {
                self.marker_drag = Some(i);
            }
            if self.marker_drag == Some(i)
                && let Some(pt) = resp.interact_pointer_pos()
            {
                self.markers[i].t = self.t_of(pt.x, area);
            }
            if resp.drag_stopped() {
                self.marker_drag = None;
                clicked = Some(i);
            }
            let tip = if self.markers[i].text.trim().is_empty() { crate::i18n::l("метка без текста", "a marker without a note") } else { self.markers[i].text.as_str() };
            let tip = format!("{} · {}", fmt_time(self.markers[i].t, false), tip);
            if resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip).clicked() {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked {
            self.marker_edit = Some(i);
            self.marker_focus = true;
            let t = self.markers[i].t;
            self.seek(t);
        }
        self.marker_editor(ui, lane, area);
    }

    /// The note of the selected marker, in a card under its flag.
    fn marker_editor(&mut self, ui: &mut egui::Ui, lane: Rect, area: Rect) {
        let Some(i) = self.marker_edit else { return };
        if i >= self.markers.len() {
            self.marker_edit = None;
            return;
        }
        let x = self.x_of(self.markers[i].t, area);
        let w = 300.0;
        let left = (x - 14.0).clamp(area.left(), area.right() - w);
        let pos = pos2(left, lane.top() - 8.0 - 84.0);
        let mut remove = false;
        let mut close = false;
        let id = egui::Id::unique("marker-editor");
        egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| {
            ui::float_frame(5)
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(w - 20.0);
                    ui.horizontal(|ui| {
                        icons::paint(ui.painter(), Rect::from_min_size(ui.cursor().min + vec2(0.0, 1.0), Vec2::splat(16.0)), Icon::Flag, 13.0, ACCENT);
                        ui.add_space(20.0);
                        ui.label(egui::RichText::new(lf!("метка · {}", "marker · {}", fmt_time(self.markers[i].t, true))).font(font_medium(12.5)).color(TEXT));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if ui::icon_button(ui, Icon::Close, crate::i18n::l("готово", "done"), 24.0, false, ui::Kind::Ghost).clicked() {
                                close = true;
                            }
                            if ui::icon_button(ui, Icon::Trash, crate::i18n::l("удалить метку", "delete the marker"), 24.0, false, ui::Kind::Ghost).clicked() {
                                remove = true;
                            }
                        });
                    });
                    ui.add_space(4.0);
                    let edit = egui::TextEdit::singleline(&mut self.markers[i].text)
                        .hint_text(crate::i18n::l("что здесь происходит", "what happens here"))
                        .char_limit(200)
                        .desired_width(f32::INFINITY)
                        .font(font(12.5));
                    let resp = ui.add(edit);
                    if std::mem::take(&mut self.marker_focus) {
                        resp.request_focus();
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Escape)) {
                        close = true;
                    }
                });
        });
        if remove {
            self.markers.remove(i);
            self.marker_edit = None;
        } else if close {
            self.marker_edit = None;
            ui.ctx().memory_mut(|m| {
                if let Some(f) = m.focused() {
                    m.surrender_focus(f);
                }
            });
        }
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
                    let range = format!("{} – {}", fmt_time(self.trim.0, true), fmt_time(self.trim.1, true));
                    ui.label(egui::RichText::new(lf!("останется {range} · {len}", "keeps {range} · {len}")).font(mono(12.0)).color(ACCENT));
                } else {
                    ui.label(
                        egui::RichText::new(crate::i18n::l("тяни края рамки, чтобы обрезать · i и o ставят начало и конец · m метка", "drag the frame's edges to trim · i and o set the start and end · m adds a marker"))
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
            let resp = ui::slider(ui, &mut v, 0.0..=2.0, 0.0);
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

    /// The trim edge `pt` is on: within the grip (it sits inside the range)
    /// or a little outside it, on the filmstrip or the tracks.
    fn edge_at(&self, pt: egui::Pos2, area: Rect) -> Option<Drag> {
        if !area.expand2(vec2(GRIP_REACH, 0.0)).contains(pt) {
            return None;
        }
        let (xi, xo) = (self.x_of(self.trim.0, area), self.x_of(self.trim.1, area));
        let di = if pt.x >= xi - GRIP_REACH && pt.x <= xi + GRIP_W + 4.0 { Some((pt.x - xi).abs()) } else { None };
        let do_ = if pt.x <= xo + GRIP_REACH && pt.x >= xo - GRIP_W - 4.0 { Some((pt.x - xo).abs()) } else { None };
        match (di, do_) {
            (Some(a), Some(b)) => Some(if a <= b { Drag::In } else { Drag::Out }),
            (Some(_), None) => Some(Drag::In),
            (None, Some(_)) => Some(Drag::Out),
            (None, None) => None,
        }
    }

    fn timeline_input(&mut self, ui: &mut egui::Ui, area: Rect, strip: Rect) {
        let _ = strip;
        let resp = ui.interact(area.expand2(vec2(GRIP_REACH, 0.0)), ui.scope_id().with("timeline"), Sense::click_and_drag());
        // what a press grabs is decided where it went down, not where the
        // pointer is once the move counts as a drag
        if resp.is_pointer_button_down_on() && self.drag == Drag::None {
            if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                self.drag = self.edge_at(origin, area).unwrap_or(Drag::Playhead);
            }
        }
        if let Some(pt) = resp.hover_pos() {
            if self.edge_at(pt, area).is_some() || matches!(self.drag, Drag::In | Drag::Out) {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            } else {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
        }
        if let Some(pt) = resp.interact_pointer_pos()
            && (resp.dragged() || resp.clicked() || resp.drag_started() || resp.is_pointer_button_down_on())
        {
            let t = self.t_of(pt.x, area);
            let min_len = HNS / 5;
            // held still: nothing to do (a seek decodes a frame)
            match self.drag {
                Drag::In => {
                    let v = t.min(self.trim.1 - min_len).max(0);
                    if v != self.trim.0 || self.pos != v {
                        self.trim.0 = v;
                        self.seek(v);
                    }
                }
                Drag::Out => {
                    let v = t.max(self.trim.0 + min_len).min(self.dur());
                    if v != self.trim.1 || self.pos != v {
                        self.trim.1 = v;
                        self.seek(v);
                    }
                }
                Drag::Playhead if t != self.pos || self.playing => self.seek(t),
                _ => {}
            }
        }
        if !ui.input(|i| i.pointer.primary_down()) {
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
        // the kept range: an accent frame over the filmstrip and a thin one
        // round the tracks, grips on both ends
        let sel = Rect::from_min_max(pos2(xi, strip.top()), pos2(xo, strip.bottom()));
        p.rect_stroke(Rect::from_min_max(pos2(xi, strip.top()), pos2(xo, area.bottom())), 4, Stroke::new(1.0, ACCENT.gamma_multiply(0.35)), StrokeKind::Inside);
        p.rect_stroke(sel, 4, Stroke::new(2.0, ACCENT), StrokeKind::Inside);
        for (x, left, which) in [(xi, true, Drag::In), (xo, false, Drag::Out)] {
            let g = if left {
                Rect::from_min_max(pos2(x, strip.top()), pos2(x + GRIP_W, strip.bottom()))
            } else {
                Rect::from_min_max(pos2(x - GRIP_W, strip.top()), pos2(x, strip.bottom()))
            };
            let hot = self.drag == which;
            p.rect_filled(g, 4, if hot { ACCENT_HI } else { ACCENT });
            let cx = g.center().x;
            for dx in [-1.5, 1.5] {
                p.vline(cx + dx, (g.center().y - 7.0)..=(g.center().y + 7.0), Stroke::new(1.2, ON_ACCENT));
            }
            // the time of the edge, above it while it's dragged
            if hot {
                let t = if left { self.trim.0 } else { self.trim.1 };
                let text = fmt_time(t, true);
                let g2 = p.layout_no_wrap(text, mono(11.0), ON_ACCENT);
                let bubble = Rect::from_center_size(pos2(x, strip.top() - 13.0), g2.size() + vec2(12.0, 6.0));
                p.rect_filled(bubble, 5, ACCENT);
                p.galley(bubble.center() - g2.size() / 2.0, g2, ON_ACCENT);
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
        if !self.playing && self.job.is_none() {
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
        let (what, f, cancel, engine): (String, f32, _, Option<String>) = match &self.job {
            Some(job) => {
                let w = match job.kind {
                    ExportKind::Gif => crate::i18n::t("сохраняю gif"),
                    ExportKind::Audio(_) => crate::i18n::l("сохраняю звук", "saving the sound"),
                    ExportKind::Mp4 => crate::i18n::t("сохраняю видео"),
                };
                (w.to_string(), job.fraction(), job.cancel.clone(), None)
            }
            None => return,
        };
        let p = ui.painter().clone();
        p.rect_filled(rect, 0, Color32::from_black_alpha(150));
        let card = Rect::from_center_size(rect.center(), vec2(360.0, 104.0));
        ui::paint_float(&p, card, 4, 1.0);
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
        }
    }

    fn done_card(&mut self, ui: &mut egui::Ui, r: Rect) {
        let Some(done) = &self.done else { return };
        let card = Rect::from_min_size(r.right_bottom() - vec2(16.0 + 380.0, 16.0 + 50.0), vec2(380.0, 50.0));
        let p = ui.painter().clone();
        ui::paint_float(&p, card, 4, 1.0);
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
            let markers = done.markers.clone();
            self.upload = Some(crate::upload::start_with(&self.shared.config(), path.clone(), None, markers));
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
        // typing a marker's note: letters are text, not shortcuts
        if self.job.is_some() || ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let (cmd, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        if pressed(Key::Space) {
            if self.playing { self.pause() } else { self.play() }
        }
        if cmd && pressed(Key::S) {
            self.export(ctx, self.save_kind());
        }
        if !cmd && pressed(Key::M) {
            self.add_marker();
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
        if theme::glass() { [0.0; 4] } else { CANVAS.to_normalized_gamma_f32() }
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
        if ctx.input(|i| i.viewport().close_requested()) && (self.job.is_some()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.notice = Some(Notice::new(&ctx, crate::i18n::t("ещё сохраняется")));
        }
        self.tick(&ctx);
        self.poll_job(&ctx);
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
                    let text = match &job.note {
                        Some(n) => format!("{} · {n}", crate::i18n::t("ссылка скопирована")),
                        None => crate::i18n::t("ссылка скопирована").to_string(),
                    };
                    self.notice = Some(Notice::new(&ctx, text));
                    self.upload = None;
                }
                Some(Err(e)) => {
                    self.notice = Some(Notice::new(&ctx, lf!("не загрузилось: {e}", "upload failed: {e}")));
                    self.upload = None;
                }
            }
        }
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
    ui.painter().vline(rect.center().x, rect.shrink2(vec2(0.0, 6.0)).y_range(), Stroke::new(1.0, ui::hairline()));
}

fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(font_medium(12.0)).color(TEXT2));
}

fn menu_item(ui: &mut egui::Ui, icon: Icon, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(230.0), 28.0), Sense::click());
    let h = resp.hovered();
    if h {
        ui.painter().rect_filled(rect, ui::soft_radius(3), ui::hover_fill());
    }
    let ir = Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(16.0, 16.0));
    icons::paint(ui.painter(), ir, icon, 14.0, if h { ACCENT } else { DIM });
    ui.painter().text(pos2(rect.left() + 32.0, rect.center().y), Align2::LEFT_CENTER, label, font(13.0), TEXT);
    resp.clicked()
}

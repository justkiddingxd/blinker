//! Screen recording: Windows Graphics Capture → Media Foundation MP4 (macOS:
//! ScreenCaptureKit → AVAssetWriter) with separate desktop-audio and
//! microphone tracks; pause, single-window recording, click / shortcut
//! effects, and an always-on instant replay.

#[cfg_attr(target_os = "macos", path = "../mac/record/audio.rs")]
pub mod audio;
#[cfg_attr(target_os = "macos", path = "../mac/record/encoder.rs")]
pub mod encoder;
#[cfg_attr(target_os = "macos", path = "../mac/record/fx.rs")]
pub mod fx;
#[cfg_attr(target_os = "macos", path = "../mac/record/hud.rs")]
pub mod hud;
mod pill;
pub mod replay;
#[cfg_attr(target_os = "macos", path = "../mac/record/video.rs")]
pub mod video;

use crate::config::{Config, log};
use crate::win::{IRect, Monitor};
use audio::{AudioCapture, Source};
use encoder::{Encoder, HNS, VideoSpec};
use image::RgbaImage;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use video::{Capture, Frame, Gpu, Pacer};

/// Recording clock in 100 ns: QPC minus the start and all paused stretches.
pub struct Clock {
    t0: i64,
    paused: AtomicBool,
    paused_at: AtomicI64,
    paused_total: AtomicI64,
}

impl Clock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            t0: video::qpc_now(),
            paused: AtomicBool::new(false),
            paused_at: AtomicI64::new(0),
            paused_total: AtomicI64::new(0),
        })
    }

    /// Recording time of a QPC timestamp; `None` while paused.
    pub fn map(&self, qpc: i64) -> Option<i64> {
        if self.paused.load(Ordering::SeqCst) {
            return None;
        }
        Some(qpc - self.t0 - self.paused_total.load(Ordering::SeqCst))
    }

    /// Current recording time (stands still while paused).
    pub fn now(&self) -> i64 {
        let q = if self.paused.load(Ordering::SeqCst) { self.paused_at.load(Ordering::SeqCst) } else { video::qpc_now() };
        q - self.t0 - self.paused_total.load(Ordering::SeqCst)
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn set_paused(&self, on: bool) {
        if on == self.is_paused() {
            return;
        }
        if on {
            self.paused_at.store(video::qpc_now(), Ordering::SeqCst);
            self.paused.store(true, Ordering::SeqCst);
        } else {
            let d = video::qpc_now() - self.paused_at.load(Ordering::SeqCst);
            self.paused_total.fetch_add(d, Ordering::SeqCst);
            self.paused.store(false, Ordering::SeqCst);
        }
    }
}

/// Where captured audio goes: an MP4 track, or the replay's PCM ring.
pub trait AudioSink: Send + Sync {
    fn write_audio(&self, track: usize, pcm: &[i16], t: i64);
}

impl AudioSink for Encoder {
    fn write_audio(&self, track: usize, pcm: &[i16], t: i64) {
        let _ = Encoder::write_audio(self, track, pcm, t);
    }
}

/// Where paced video frames go.
pub enum VideoOut {
    File(Arc<Encoder>),
    Replay(Arc<replay::Segmenter>),
}

unsafe impl Send for VideoOut {}

impl VideoOut {
    fn write_video(&self, tex: &Frame, t: i64, dur: i64) {
        match self {
            VideoOut::File(e) => {
                let _ = e.write_texture(tex, t, dur);
            }
            VideoOut::Replay(s) => s.write_video(tex, t, dur),
        }
    }

    /// Housekeeping between frames (replay: drop old segments).
    fn maintain(&self, now: i64, _last: Option<&Frame>) {
        if let VideoOut::Replay(s) = self {
            s.maintain(now);
        }
    }
}

/// What the user picked in the overlay.
#[derive(Clone, Copy)]
pub struct Selection {
    /// Absolute physical px.
    pub rect: IRect,
    pub monitor: Monitor,
    /// A window was clicked: record that window itself.
    pub window: Option<isize>,
}

pub struct Recording {
    enc: Arc<Encoder>,
    capture: Capture,
    pacer: Pacer,
    audio: Vec<AudioCapture>,
    end: Arc<AtomicI64>,
    clock: Arc<Clock>,
    fx: Option<Arc<fx::Fx>>,
    path: PathBuf,
    pub rect: IRect,
    pub monitor: Monitor,
    hud: hud::Hud,
    /// Track labels in file order.
    tracks: Vec<Source>,
    /// Tracks whose device failed to start: filled with silence at the end.
    failed: Vec<usize>,
    pub gif: bool,
}

#[allow(dead_code)]
pub struct Finished {
    pub path: PathBuf,
    /// Recorded for a GIF: `path` is a temporary MP4 to convert.
    pub gif: bool,
    pub thumb: Option<RgbaImage>,
    pub duration: f64,
    pub rect: IRect,
    pub monitor: Monitor,
    pub tracks: Vec<Source>,
}

pub fn new_path(cfg: &Config, prefix: &str) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
    let dir = cfg.videos_dir();
    let mut path = dir.join(format!("{prefix} {stamp}.mp4"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{prefix} {stamp} ({n}).mp4"));
        n += 1;
    }
    path
}

#[cfg(windows)]
pub fn hmonitor_at(x: i32, y: i32) -> windows::Win32::Graphics::Gdi::HMONITOR {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
    unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) }
}

/// Audio tracks to record (and whose devices exist).
pub fn audio_sources(desktop: bool, mic: bool) -> Vec<Source> {
    let mut tracks = Vec::new();
    if desktop && audio::available(Source::Desktop) {
        tracks.push(Source::Desktop);
    }
    if mic && audio::available(Source::Mic) {
        tracks.push(Source::Mic);
    }
    tracks
}

/// Frame rate for a recording on `monitor` (0 in the settings = its refresh rate).
pub fn resolve_fps(cfg: &Config, monitor: &Monitor) -> u32 {
    if cfg.rec_fps == 0 {
        let r = monitor.rect;
        crate::win::refresh_rate(r.x + r.w / 2, r.y + r.h / 2).clamp(24, 360)
    } else {
        cfg.rec_fps.clamp(10, 360)
    }
}

pub fn video_spec(cfg: &Config, w: u32, h: u32, keyframe_secs: u32, monitor: &Monitor) -> VideoSpec {
    let fps = resolve_fps(cfg, monitor);
    let codec = if cfg.rec_codec == crate::config::VideoCodec::Hevc && encoder::hevc_available() {
        encoder::Codec::Hevc
    } else {
        encoder::Codec::H264
    };
    VideoSpec {
        width: w,
        height: h,
        fps,
        bitrate: cfg.rec_quality.bitrate(w, h, fps),
        codec,
        quality: Some(cfg.rec_quality.quality()),
        keyframe_secs,
    }
}

/// Opens an encoder for `spec`, stepping down when the hardware refuses:
/// constant quality → VBR, very high frame rates → 120 → 60, H.265 → H.264.
/// Returns the settings actually used.
pub fn open_encoder(path: &std::path::Path, spec: &VideoSpec, tracks: usize, gpu: &Gpu) -> Result<(Encoder, VideoSpec), String> {
    let mut tries = vec![*spec, VideoSpec { quality: None, ..*spec }];
    for fps in [120, 60] {
        if spec.fps > fps {
            tries.push(VideoSpec { fps, ..*spec });
        }
    }
    if spec.codec != encoder::Codec::H264 {
        tries.push(VideoSpec { codec: encoder::Codec::H264, fps: spec.fps.min(120), ..*spec });
    }
    let mut last = String::new();
    for s in tries {
        match Encoder::new(path, Some(&s), tracks, Some(&gpu.device)) {
            Ok(e) => return Ok((e, s)),
            Err(e) => {
                last = e.message().to_string();
                let _ = std::fs::remove_file(path);
            }
        }
    }
    Err(lf!("кодировщик: {last}", "encoder: {last}"))
}

/// Starts recording. A monitor area loses what lies outside the monitor
/// holding its center; sizes are rounded to even.
pub fn start(
    cfg: &Config,
    sel: Selection,
    gif: bool,
    on_stop: impl Fn() + Send + 'static,
    on_pause: impl Fn() + Send + 'static,
) -> Result<Recording, String> {
    let monitor = sel.monitor;
    let (target, r, origin) = match sel.window {
        Some(h) => {
            let (target, b) = video::window_target(h);
            (target, b, (b.x, b.y))
        }
        None => {
            let r = sel.rect.intersect(&monitor.rect);
            let (w, h) = ((r.w as u32) & !1, (r.h as u32) & !1);
            let r = IRect::new(r.x, r.y, w as i32, h as i32);
            (video::monitor_target(r, &monitor), r, (r.x, r.y))
        }
    };
    let (w, h) = target.size();
    if w < 16 || h < 16 {
        return Err(crate::i18n::t("слишком маленькая область").into());
    }
    // A GIF has no sound and is written to a temporary MP4 first.
    let tracks = if gif { Vec::new() } else { audio_sources(cfg.rec_desktop_audio, cfg.rec_mic) };
    let gpu = Arc::new(Gpu::new().map_err(|e| format!("{}: {e}", video::GPU_API))?);
    let path = if gif {
        let dir = crate::config::cache_dir().join("gif");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(format!("rec-{}.mp4", chrono::Local::now().format("%Y%m%d-%H%M%S")))
    } else {
        new_path(cfg, "blinker")
    };
    let (enc, spec) = open_encoder(&path, &video_spec(cfg, w, h, 2, &monitor), tracks.len(), &gpu)?;
    let enc = Arc::new(enc);

    let clock = Clock::new();
    let end = Arc::new(AtomicI64::new(0));
    let capture = Capture::start(&gpu, target, spec.fps, cfg.rec_cursor, clock.clone()).map_err(|e| {
        let _ = std::fs::remove_file(&path);
        lf!("захват экрана: {e}", "screen capture: {e}")
    })?;
    let fx = fx::Fx::start(cfg.rec_show_clicks, cfg.rec_show_keys, monitor.scale);
    let pacer = Pacer::start(gpu, &capture, target, VideoOut::File(enc.clone()), spec.fps, clock.clone(), fx.clone(), origin);
    let mut audio = Vec::new();
    let mut failed = Vec::new();
    for (i, src) in tracks.iter().enumerate() {
        match AudioCapture::start(*src, enc.clone(), i, clock.clone(), end.clone()) {
            Ok(a) => audio.push(a),
            Err(e) => {
                log::error(format!("audio {src:?}: {e}"));
                failed.push(i);
            }
        }
    }
    let hud = hud::Hud::show(r, &monitor, sel.window.is_none(), sel.window, clock.clone(), on_stop, on_pause);
    Ok(Recording { enc, capture, pacer, audio, end, clock, fx, path, rect: r, monitor, hud, tracks, failed, gif })
}

impl Recording {
    pub fn toggle_pause(&self) {
        self.clock.set_paused(!self.clock.is_paused());
    }

    /// Stops and finalizes the file (blocks until it is written).
    pub fn stop(self) -> Option<Finished> {
        self.hud.close();
        if let Some(f) = &self.fx {
            f.stop();
        }
        self.clock.set_paused(false);
        self.capture.close();
        let end = self.clock.now().max(1);
        self.end.store(end, Ordering::SeqCst);
        let thumb = self.pacer.stop();
        for a in self.audio {
            a.join();
        }
        for &track in &self.failed {
            let second = vec![0i16; (encoder::AUDIO_RATE * encoder::AUDIO_CHANNELS) as usize];
            let mut t = 0;
            while t < end {
                let _ = self.enc.write_audio(track, &second, t);
                t += HNS;
            }
        }
        if let Err(e) = self.enc.finish() {
            log::error(format!("finalize: {e}"));
            return None;
        }
        Some(Finished {
            path: self.path,
            gif: self.gif,
            thumb,
            duration: end as f64 / HNS as f64,
            rect: self.rect,
            monitor: self.monitor,
            tracks: self.tracks,
        })
    }
}

#[cfg(test)]
mod tests {
    /// Records 1.5 s of a small corner of the primary monitor (desktop sound
    /// on, microphone off, with a 0.5 s pause), checks the file, deletes it.
    /// `BLINKER_TEST_RECORD` = scratch folder.
    #[test]
    fn record_smoke() {
        let Ok(dir) = std::env::var("BLINKER_TEST_RECORD") else { return };
        crate::media::mark_main_thread();
        let mut cfg = crate::config::Config { save_dir: dir.into(), ..Default::default() };
        cfg.rec_mic = false;
        cfg.rec_desktop_audio = true;
        let mon = crate::win::monitors()[0];
        let rect = crate::win::IRect::new(mon.rect.x + 40, mon.rect.y + 60, 320, 200);
        let sel = super::Selection { rect, monitor: mon, window: None };
        let rec = super::start(&cfg, sel, false, || {}, || {}).expect("start");
        std::thread::sleep(std::time::Duration::from_millis(800));
        rec.toggle_pause();
        std::thread::sleep(std::time::Duration::from_millis(500));
        rec.toggle_pause();
        std::thread::sleep(std::time::Duration::from_millis(700));
        let f = rec.stop().expect("stop");
        let info = crate::media::probe(&f.path).expect("probe");
        eprintln!("{info:?} thumb={:?}", f.thumb.as_ref().map(|t| t.dimensions()));
        let _ = std::fs::remove_file(&f.path);
        assert_eq!((info.width, info.height), (320, 200));
        // 1.5 s recorded, the paused half second is not in the file
        assert!((13_000_000..17_500_000).contains(&info.duration), "{}", info.duration);
        assert_eq!(info.audio_tracks, f.tracks.len());
    }

    /// HEVC at the monitor's refresh rate.
    #[test]
    fn record_hevc_native() {
        let Ok(dir) = std::env::var("BLINKER_TEST_RECORD") else { return };
        crate::media::mark_main_thread();
        let mut cfg = crate::config::Config { save_dir: dir.into(), ..Default::default() };
        cfg.rec_mic = false;
        cfg.rec_desktop_audio = false;
        cfg.rec_codec = crate::config::VideoCodec::Hevc;
        cfg.rec_fps = 0;
        let mon = crate::win::monitors()[0];
        let rect = crate::win::IRect::new(mon.rect.x + 40, mon.rect.y + 60, 320, 200);
        let sel = super::Selection { rect, monitor: mon, window: None };
        let rec = super::start(&cfg, sel, false, || {}, || {}).expect("start");
        std::thread::sleep(std::time::Duration::from_millis(1200));
        let f = rec.stop().expect("stop");
        let info = crate::media::probe(&f.path).expect("probe");
        let out = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "stream=codec_name,r_frame_rate", "-of", "csv=p=0"])
            .arg(&f.path)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        eprintln!("{info:?} · ffprobe: {} · monitor {} hz", out.trim(), crate::win::refresh_rate(mon.rect.x + 1, mon.rect.y + 1));
        let _ = std::fs::remove_file(&f.path);
        assert!(info.duration > 9_000_000);
    }
}

#[cfg(all(test, windows))]
mod window_test {
    /// Records a window (`BLINKER_TEST_WINDOW` = HWND, decimal) for 1.5 s.
    #[test]
    fn record_window() {
        let (Ok(dir), Ok(h)) = (std::env::var("BLINKER_TEST_RECORD"), std::env::var("BLINKER_TEST_WINDOW")) else { return };
        crate::media::mark_main_thread();
        let mut cfg = crate::config::Config { save_dir: dir.into(), ..Default::default() };
        cfg.rec_mic = false;
        cfg.rec_desktop_audio = false;
        let hwnd: isize = h.parse().unwrap();
        let b = crate::win::frame_bounds(hwnd as _);
        let mon = crate::win::monitor_at(&crate::win::monitors(), b.x + b.w / 2, b.y + b.h / 2).unwrap();
        let sel = super::Selection { rect: b, monitor: mon, window: Some(hwnd) };
        let rec = super::start(&cfg, sel, false, || {}, || {}).expect("start");
        std::thread::sleep(std::time::Duration::from_millis(500));
        // move the window: the pill must follow it
        let pill_x = || unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowRect};
            let class = crate::win::wide("BlinkerRecHud");
            let h = FindWindowW(class.as_ptr(), std::ptr::null());
            let mut r = std::mem::zeroed();
            GetWindowRect(h, &mut r);
            (r.left + r.right) / 2
        };
        let before = pill_x();
        let wr = crate::win::window_rect(hwnd as _);
        crate::win::set_window_rect(hwnd as _, wr.offset(200, 0), false);
        std::thread::sleep(std::time::Duration::from_millis(400));
        let after = pill_x();
        crate::win::set_window_rect(hwnd as _, wr, false);
        eprintln!("pill center x: {before} -> {after}");
        assert!((after - before - 200).abs() <= 2, "pill did not follow");
        std::thread::sleep(std::time::Duration::from_millis(600));
        let f = rec.stop().expect("stop");
        let info = crate::media::probe(&f.path).expect("probe");
        eprintln!("{info:?} window {}x{}", b.w, b.h);
        if let Some(t) = &f.thumb {
            let _ = t.save(f.path.with_extension("png"));
        }
        let _ = std::fs::remove_file(&f.path);
        assert_eq!((info.width as i32, info.height as i32), (b.w & !1, b.h & !1));
    }
}

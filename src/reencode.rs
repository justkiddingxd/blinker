//! "For Telegram" re-encode of a recording. With ffmpeg installed this is the
//! user's tuned recipe: NVENC (preset p7, tune hq, constant-quality VBR, 3
//! B-frames, lookahead 20, spatial + temporal AQ) or libx264/libx265 `slow`
//! CRF as a CPU fallback; AAC audio; even size; faststart; `hvc1` tag for HEVC.
//! Without ffmpeg the built-in Media Foundation encoder does the job.
//!
//! Our recordings keep desktop sound and microphone apart; Telegram plays only
//! the first track, so they are mixed into one here.

use crate::config::TgMode;
use crate::media::MediaInfo;
use crate::record::encoder::HNS;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

pub struct Job {
    /// 0..=1000
    pub progress: AtomicU32,
    pub cancel: AtomicBool,
    pub result: Mutex<Option<Result<PathBuf, String>>>,
    /// What actually encodes ("h264_nvenc · видеокарта"...).
    pub engine: Mutex<String>,
}

impl Job {
    pub fn fraction(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }
    pub fn result(&self) -> Option<Result<PathBuf, String>> {
        self.result.lock().unwrap().clone()
    }
}

/// Trim / per-track gain / mixing chosen in the video editor.
#[derive(Clone)]
pub struct Edit {
    pub from: i64,
    pub to: i64,
    /// (audio track ordinal, gain); muted tracks are left out.
    pub tracks: Vec<(usize, f32)>,
    pub mix_down: bool,
}

impl TgMode {
    pub const ALL: [TgMode; 4] = [TgMode::Min, TgMode::Balanced, TgMode::Hq, TgMode::Hevc];

    pub fn label(self) -> &'static str {
        match self {
            TgMode::Balanced => crate::i18n::t("баланс"),
            TgMode::Min => crate::i18n::t("минимум"),
            TgMode::Hq => crate::i18n::t("качество"),
            TgMode::Hevc => "hevc",
        }
    }

    pub fn desc(self) -> &'static str {
        match self {
            TgMode::Balanced => crate::i18n::t("h.264, нормальное качество"),
            TgMode::Min => crate::i18n::t("h.264, меньше всего весит, есть артефакты"),
            TgMode::Hq => crate::i18n::t("h.264, почти без потерь"),
            TgMode::Hevc => crate::i18n::t("h.265, меньше, но открывается не везде"),
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            TgMode::Balanced => "-telegram",
            TgMode::Min => "-telegram-min",
            TgMode::Hq => "-telegram-hq",
            TgMode::Hevc => "-telegram-hevc",
        }
    }

    fn hevc(self) -> bool {
        self == TgMode::Hevc
    }

    /// NVENC `-cq` / x264 `-crf`.
    fn cq(self) -> u32 {
        match self {
            TgMode::Balanced => 23,
            TgMode::Min => 30,
            TgMode::Hq => 19,
            TgMode::Hevc => 28,
        }
    }

    fn audio_kbps(self) -> u32 {
        if self == TgMode::Min { 128 } else { 160 }
    }

    /// Equivalent constant quality for the built-in encoder (1..100).
    fn mf_quality(self) -> u32 {
        match self {
            TgMode::Balanced => 62,
            TgMode::Min => 44,
            TgMode::Hq => 76,
            TgMode::Hevc => 58,
        }
    }
}

/// ffmpeg from C:\ffmpeg\bin or PATH (looked up once). On macOS also the
/// Homebrew / MacPorts folders: apps started from Finder get a bare PATH.
pub fn ffmpeg() -> Option<&'static Path> {
    static FF: OnceLock<Option<PathBuf>> = OnceLock::new();
    FF.get_or_init(|| {
        let (name, known): (&str, &[&str]) = if cfg!(windows) {
            ("ffmpeg.exe", &[r"C:\ffmpeg\bin\ffmpeg.exe"])
        } else {
            ("ffmpeg", &["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg", "/opt/local/bin/ffmpeg"])
        };
        if let Some(k) = known.iter().map(PathBuf::from).find(|p| p.is_file()) {
            return Some(k);
        }
        std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(name)).find(|f| f.is_file()))
    })
    .as_deref()
}

#[cfg(windows)]
fn quiet(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW)
}

#[cfg(not(windows))]
fn quiet(cmd: &mut Command) -> &mut Command {
    cmd
}

/// Whether ffmpeg was built with this encoder (cached).
fn has_encoder(ff: &Path, name: &str) -> bool {
    static LIST: OnceLock<String> = OnceLock::new();
    LIST.get_or_init(|| {
        quiet(Command::new(ff).args(["-hide_banner", "-encoders"]))
            .stderr(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    })
    .contains(name)
}

/// Short description of the engine that would be used.
pub fn engine_label(mode: TgMode) -> String {
    match ffmpeg() {
        Some(ff) => {
            let gpu = if mode.hevc() { "hevc_nvenc" } else { "h264_nvenc" };
            if has_encoder(ff, gpu) {
                lf!("ffmpeg · {gpu} (видеокарта)", "ffmpeg · {gpu} (gpu)")
            } else {
                lf!("ffmpeg · {} (процессор)", "ffmpeg · {} (cpu)", if mode.hevc() { "libx265" } else { "libx264" })
            }
        }
        None => crate::i18n::t("встроенный кодировщик (ffmpeg не найден)").to_string(),
    }
}

pub fn target_path(src: &Path, mode: TgMode) -> PathBuf {
    let stem = src.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let dir = src.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut p = dir.join(format!("{stem}{}.mp4", mode.suffix()));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stem}{} {n}.mp4", mode.suffix()));
        n += 1;
    }
    p
}

/// Starts the re-encode in the background. `edit`: trim and audio choices
/// from the editor; `None` = whole file, every audio track mixed in.
/// `repaint` is called when progress moves.
pub fn start(src: PathBuf, mode: TgMode, edit: Option<Edit>, repaint: impl Fn() + Send + 'static) -> Arc<Job> {
    let job = Arc::new(Job {
        progress: AtomicU32::new(0),
        cancel: AtomicBool::new(false),
        result: Mutex::new(None),
        engine: Mutex::new(engine_label(mode)),
    });
    let j = job.clone();
    std::thread::spawn(move || {
        crate::media::startup();
        let dst = target_path(&src, mode);
        let r = match crate::media::probe(&src) {
            Err(e) => Err(lf!("не открылось видео: {}", "the video did not open: {}", e.message())),
            Ok(info) => {
                let edit = edit.unwrap_or_else(|| Edit {
                    from: 0,
                    to: info.duration,
                    tracks: (0..info.audio_tracks).map(|i| (i, 1.0)).collect(),
                    mix_down: true,
                });
                match ffmpeg() {
                    Some(ff) => run_ffmpeg(ff, &src, &dst, mode, &edit, &info, &j, &repaint),
                    None => run_builtin(&src, &dst, mode, &edit, &info, &j),
                }
            }
        };
        let r = if j.cancel.load(Ordering::SeqCst) { Err(crate::i18n::t("отменено").to_string()) } else { r };
        if r.is_err() {
            let _ = std::fs::remove_file(&dst);
        }
        *j.result.lock().unwrap() = Some(r.map(|_| dst));
        repaint();
    });
    job
}

fn secs(t: i64) -> String {
    format!("{:.3}", t as f64 / HNS as f64)
}

#[allow(clippy::too_many_arguments)]
fn run_ffmpeg(
    ff: &Path,
    src: &Path,
    dst: &Path,
    mode: TgMode,
    edit: &Edit,
    info: &MediaInfo,
    job: &Job,
    repaint: &dyn Fn(),
) -> Result<(), String> {
    let (gpu, cpu) = if mode.hevc() { ("hevc_nvenc", "libx265") } else { ("h264_nvenc", "libx264") };
    let cq = mode.cq().to_string();
    let video = |use_gpu: bool| -> Vec<String> {
        let mut v: Vec<&str> = if use_gpu {
            vec![
                "-c:v", gpu, "-preset", "p7", "-tune", "hq", "-rc", "vbr", "-cq", &cq, "-b:v", "0", "-bf", "3", "-rc-lookahead", "20",
                "-spatial-aq", "1", "-temporal-aq", "1", "-pix_fmt", "yuv420p",
            ]
        } else {
            vec!["-c:v", cpu, "-preset", "slow", "-crf", &cq, "-pix_fmt", "yuv420p"]
        };
        // h.264 high profile decodes everywhere; hvc1 so HEVC plays in Telegram / on Apple
        v.extend(if mode.hevc() { ["-tag:v", "hvc1"] } else { ["-profile:v", "high"] });
        v.into_iter().map(String::from).collect()
    };

    let len = (edit.to - edit.from).max(1);
    let mut base: Vec<String> = vec!["-hide_banner".into(), "-y".into(), "-nostats".into(), "-progress".into(), "pipe:1".into()];
    if edit.from > 0 {
        base.extend(["-ss".into(), secs(edit.from)]);
    }
    base.extend(["-i".into(), src.display().to_string(), "-t".into(), secs(len)]);
    // audio: per-track volume, mixed into one track (or kept apart from the editor)
    let tracks: Vec<&(usize, f32)> = edit.tracks.iter().filter(|(i, g)| *i < info.audio_tracks && *g > 0.0).collect();
    let mut filters: Vec<String> = Vec::new();
    let mut maps: Vec<String> = vec!["-map".into(), "0:v:0".into()];
    filters.push("[0:v:0]scale=trunc(iw/2)*2:trunc(ih/2)*2[v]".into());
    maps[1] = "[v]".into();
    if !tracks.is_empty() {
        for (k, (i, g)) in tracks.iter().enumerate() {
            filters.push(format!("[0:a:{i}]volume={g:.3}[a{k}]"));
        }
        if edit.mix_down && tracks.len() > 1 {
            let ins: String = (0..tracks.len()).map(|k| format!("[a{k}]")).collect();
            filters.push(format!("{ins}amix=inputs={}:normalize=0:dropout_transition=0[aout]", tracks.len()));
            maps.extend(["-map".into(), "[aout]".into()]);
        } else {
            for k in 0..tracks.len() {
                maps.extend(["-map".into(), format!("[a{k}]")]);
            }
        }
    }
    let audio: Vec<String> = if tracks.is_empty() {
        vec!["-an".into()]
    } else {
        vec!["-c:a".into(), "aac".into(), "-b:a".into(), format!("{}k", mode.audio_kbps())]
    };

    let attempt = |use_gpu: bool| -> Result<(), String> {
        let mut args = base.clone();
        args.extend(["-filter_complex".into(), filters.join(";")]);
        args.extend(maps.clone());
        args.extend(video(use_gpu));
        args.extend(audio.clone());
        args.extend(["-movflags".into(), "+faststart".into(), dst.display().to_string()]);
        let mut child = quiet(Command::new(ff).args(&args))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| lf!("ffmpeg не запустился: {e}", "ffmpeg did not start: {e}"))?;
        // stderr drained on the side (the last lines explain failures)
        let err_tail = Arc::new(Mutex::new(Vec::<String>::new()));
        if let Some(se) = child.stderr.take() {
            let tail = err_tail.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(se).lines().map_while(Result::ok) {
                    let mut t = tail.lock().unwrap();
                    t.push(line);
                    if t.len() > 12 {
                        t.remove(0);
                    }
                }
            });
        }
        if let Some(out) = child.stdout.take() {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if job.cancel.load(Ordering::SeqCst) {
                    let _ = child.kill();
                    break;
                }
                if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<i64>().ok()) {
                    let f = (us * 10) as f64 / len as f64;
                    job.progress.store((f.clamp(0.0, 1.0) * 990.0) as u32, Ordering::Relaxed);
                    repaint();
                }
            }
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        if job.cancel.load(Ordering::SeqCst) {
            return Ok(());
        }
        if status.success() {
            job.progress.store(1000, Ordering::Relaxed);
            Ok(())
        } else {
            let tail = err_tail.lock().unwrap().join("\n");
            let last = tail.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").to_string();
            Err(format!("ffmpeg: {last}"))
        }
    };

    if has_encoder(ff, gpu) {
        match attempt(true) {
            Ok(()) => return Ok(()),
            // NVENC listed but not usable (no NVIDIA GPU / old driver): CPU then.
            Err(e) if !job.cancel.load(Ordering::SeqCst) => {
                crate::config::log::error(format!("{gpu} failed, falling back: {e}"));
                *job.engine.lock().unwrap() = lf!("ffmpeg · {cpu} (процессор)", "ffmpeg · {cpu} (cpu)");
                job.progress.store(0, Ordering::Relaxed);
            }
            Err(e) => return Err(e),
        }
    }
    attempt(false)
}

fn run_builtin(src: &Path, dst: &Path, mode: TgMode, edit: &Edit, info: &MediaInfo, job: &Job) -> Result<(), String> {
    use crate::record::encoder::Codec;
    let codec = if mode.hevc() && crate::record::encoder::has_codec(Codec::Hevc, true) { Codec::Hevc } else { Codec::H264 };
    let opt = crate::vedit::export::Options {
        kind: crate::vedit::export::Kind::Mp4,
        from: edit.from,
        to: edit.to,
        tracks: edit.tracks.iter().filter_map(|(i, g)| info.audio_streams.get(*i).map(|s| (*s, *g))).collect(),
        mix_down: edit.mix_down,
        bitrate_quality: crate::config::RecQuality::Balanced,
        gif_width: 0,
        gif_fps: 0,
        quality: Some(mode.mf_quality()),
        codec,
    };
    crate::vedit::export::mp4(src, dst, info, &opt, &job.progress, &job.cancel)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Re-encodes `BLINKER_TEST_EXPORT` with ffmpeg (two modes, one trimmed)
    /// and with the built-in encoder; checks one mixed audio track.
    #[test]
    fn reencode_tg() {
        let Ok(src) = std::env::var("BLINKER_TEST_EXPORT") else { return };
        let src = PathBuf::from(src);
        crate::media::startup();
        let info = crate::media::probe(&src).unwrap();
        for (mode, edit) in [
            (TgMode::Balanced, None),
            (TgMode::Hevc, Some(Edit { from: HNS / 2, to: HNS * 2, tracks: vec![(0, 1.0), (1, 0.5)], mix_down: true })),
        ] {
            let t = std::time::Instant::now();
            let job = start(src.clone(), mode, edit, || {});
            while job.result().is_none() {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let out = job.result().unwrap().expect("reencode");
            let o = crate::media::probe(&out).unwrap();
            eprintln!("{mode:?} via {}: {} bytes in {:?} · {o:?}", job.engine.lock().unwrap(), std::fs::metadata(&out).unwrap().len(), t.elapsed());
            assert_eq!(o.audio_tracks, 1);
            // faststart: the index comes before the media data
            let head = std::fs::read(&out).unwrap();
            let pos = |tag: &[u8]| head.windows(4).position(|w| w == tag).unwrap_or(usize::MAX);
            assert!(pos(b"moov") < pos(b"mdat"), "moov after mdat");
            let _ = std::fs::remove_file(&out);
        }
        // built-in path
        let dst = src.with_file_name("builtin-tg.mp4");
        let edit = Edit { from: 0, to: info.duration, tracks: vec![(0, 1.0), (1, 1.0)], mix_down: true };
        let job = Job { progress: AtomicU32::new(0), cancel: AtomicBool::new(false), result: Mutex::new(None), engine: Mutex::new(String::new()) };
        run_builtin(&src, &dst, TgMode::Balanced, &edit, &info, &job).unwrap();
        let o = crate::media::probe(&dst).unwrap();
        eprintln!("builtin: {} bytes · {o:?}", std::fs::metadata(&dst).unwrap().len());
        assert_eq!(o.audio_tracks, 1);
        let _ = std::fs::remove_file(&dst);
    }
}

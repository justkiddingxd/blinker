//! Saving an edited clip: trimmed MP4 (tracks kept apart, or mixed into one
//! for messengers and players that only play the first track) or a GIF.

use crate::media::{AudioReader, MediaInfo, VideoReader};
use crate::record::encoder::{AUDIO_CHANNELS, AUDIO_RATE, Encoder, HNS, VideoSpec};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Mp4,
    Gif,
    /// The sound alone (the video track switched off).
    Audio(AudioFormat),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AudioFormat {
    Mp3,
    M4a,
    Wav,
}

impl AudioFormat {
    pub const ALL: [AudioFormat; 3] = [AudioFormat::Mp3, AudioFormat::M4a, AudioFormat::Wav];

    pub fn ext(self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::M4a => "m4a",
            AudioFormat::Wav => "wav",
        }
    }

    pub fn desc(self) -> &'static str {
        match self {
            AudioFormat::Mp3 => crate::i18n::l("192 кбит/с, играет везде", "192 kbit/s, plays everywhere"),
            AudioFormat::M4a => crate::i18n::l("aac, меньше mp3 при том же звуке", "aac, smaller than mp3 for the same sound"),
            AudioFormat::Wav => crate::i18n::l("без сжатия, для монтажа", "uncompressed, for editing"),
        }
    }
}

#[derive(Clone)]
pub struct Options {
    pub kind: Kind,
    pub from: i64,
    pub to: i64,
    /// (audio stream, gain); muted tracks are left out.
    pub tracks: Vec<(u32, f32)>,
    /// Mix the kept tracks into one.
    pub mix_down: bool,
    pub bitrate_quality: crate::config::RecQuality,
    pub gif_width: u32,
    pub gif_fps: u32,
    /// Constant quality override (else from `bitrate_quality`).
    pub quality: Option<u32>,
    pub codec: crate::record::encoder::Codec,
}

pub struct Job {
    /// 0..=1000
    pub progress: Arc<AtomicU32>,
    pub cancel: Arc<AtomicBool>,
    pub result: Arc<Mutex<Option<Result<PathBuf, String>>>>,
    pub kind: Kind,
}

impl Job {
    pub fn fraction(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }
}

/// Free name next to the source: "name (обрезка).mp4", "name.gif", ...
pub fn target_path(src: &Path, kind: Kind, trimmed: bool) -> PathBuf {
    let stem = src.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let dir = src.parent().map(Path::to_path_buf).unwrap_or_default();
    let (base, ext) = match kind {
        Kind::Mp4 if trimmed => (lf!("{stem} (обрезка)", "{stem} (trimmed)"), "mp4"),
        Kind::Mp4 => (lf!("{stem} (правка)", "{stem} (edited)"), "mp4"),
        Kind::Gif => (stem, "gif"),
        Kind::Audio(f) => (stem, f.ext()),
    };
    let mut p = dir.join(format!("{base}.{ext}"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{base} {n}.{ext}"));
        n += 1;
    }
    p
}

pub fn start(ctx: &eframe::egui::Context, src: PathBuf, dst: PathBuf, info: MediaInfo, opt: Options) -> Job {
    let job = Job {
        progress: Arc::new(AtomicU32::new(0)),
        cancel: Arc::new(AtomicBool::new(false)),
        result: Arc::new(Mutex::new(None)),
        kind: opt.kind,
    };
    let (progress, cancel, result, ctx) = (job.progress.clone(), job.cancel.clone(), job.result.clone(), ctx.clone());
    std::thread::spawn(move || {
        crate::media::startup();
        let r = match opt.kind {
            Kind::Mp4 => mp4(&src, &dst, &info, &opt, &progress, &cancel),
            Kind::Gif => gif(&src, &dst, &opt, &progress, &cancel),
            Kind::Audio(f) => audio(&src, &dst, f, &opt, &progress, &cancel),
        };
        if r.is_err() || cancel.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&dst);
        }
        let r = if cancel.load(Ordering::SeqCst) { Err(crate::i18n::t("отменено").to_string()) } else { r.map(|_| dst) };
        *result.lock().unwrap() = Some(r);
        ctx.request_repaint();
    });
    job
}

fn err(e: crate::media::Error) -> String {
    e.message().to_string()
}

fn set_progress(p: &AtomicU32, done: i64, total: i64, lo: u32, hi: u32) {
    let f = (done as f64 / total.max(1) as f64).clamp(0.0, 1.0);
    p.store(lo + ((hi - lo) as f64 * f) as u32, Ordering::Relaxed);
}

pub fn mp4(src: &Path, dst: &Path, info: &MediaInfo, opt: &Options, progress: &AtomicU32, cancel: &AtomicBool) -> Result<(), String> {
    let (from, to) = (opt.from, opt.to);
    let len = (to - from).max(1);
    let fps = info.fps.round().clamp(1.0, 120.0) as u32;
    let (w, h) = (info.width & !1, info.height & !1);
    let audio_out = if opt.tracks.is_empty() { 0 } else if opt.mix_down { 1 } else { opt.tracks.len() };
    let spec = VideoSpec {
        width: w,
        height: h,
        fps,
        bitrate: opt.bitrate_quality.bitrate(w, h, fps),
        codec: opt.codec,
        quality: Some(opt.quality.unwrap_or(opt.bitrate_quality.quality())),
        keyframe_secs: 2,
    };
    let enc = Encoder::new(dst, Some(&spec), audio_out, None)
        .or_else(|_| {
            let _ = std::fs::remove_file(dst);
            Encoder::new(dst, Some(&VideoSpec { quality: None, ..spec }), audio_out, None)
        })
        .map_err(err)?;

    // audio first (decoded into memory), so it can be interleaved with the video
    let total = (len as i128 * AUDIO_RATE as i128 / HNS as i128) as usize * AUDIO_CHANNELS as usize;
    let mut out_tracks: Vec<Vec<i16>> = Vec::new();
    let mut mixed: Vec<f32> = if opt.mix_down { vec![0.0; total] } else { Vec::new() };
    for (i, (stream, gain)) in opt.tracks.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let pcm = read_range(src, *stream, from, total).map_err(err)?;
        if opt.mix_down {
            for (m, s) in mixed.iter_mut().zip(&pcm) {
                *m += *s as f32 * gain;
            }
        } else {
            out_tracks.push(pcm.iter().map(|s| (*s as f32 * gain).clamp(-32768.0, 32767.0) as i16).collect());
        }
        set_progress(progress, i as i64 + 1, opt.tracks.len() as i64, 0, 100);
    }
    if opt.mix_down && !opt.tracks.is_empty() {
        out_tracks.push(mixed.iter().map(|s| s.clamp(-32768.0, 32767.0) as i16).collect());
        drop(mixed);
    }
    let chunk = (AUDIO_RATE / 10 * AUDIO_CHANNELS) as usize; // 100 ms
    let mut k = 0usize;
    // The sink writer stalls when one stream runs far ahead: feed audio up to `until`.
    let audio_until = |k: &mut usize, until: i64| -> Result<(), String> {
        loop {
            let t = (*k * chunk / 2) as i64 * HNS / AUDIO_RATE as i64;
            if out_tracks.first().is_none_or(|p| *k * chunk >= p.len()) {
                return enc.end_audio().map_err(err);
            }
            if t > until {
                return Ok(());
            }
            for (i, pcm) in out_tracks.iter().enumerate() {
                let c = &pcm[*k * chunk..((*k + 1) * chunk).min(pcm.len())];
                enc.write_audio(i, c, t).map_err(err)?;
            }
            *k += 1;
        }
    };

    // video: every frame shown inside the range, retimed to start at 0
    let (reader, _) = VideoReader::open(src, w, h).map_err(err)?;
    reader.seek(from).map_err(err)?;
    let mut bgra = Vec::new();
    let mut write = |ts: i64, dur: i64, img: &image::RgbaImage, k: &mut usize| -> Result<(), String> {
        audio_until(k, ts + crate::record::encoder::AUDIO_LEAD)?;
        bgra.clear();
        bgra.extend(img.as_raw().chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], 255]));
        enc.write_bgra(&bgra, ts, dur).map_err(err)
    };
    let mut pending: Option<(i64, image::RgbaImage)> = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let next = reader.next().map_err(err)?;
        let Some((ts, img)) = next else { break };
        if ts <= from {
            pending = Some((ts, img));
            continue;
        }
        if let Some((pt, pimg)) = pending.take() {
            let at = (pt - from).max(0);
            let end = (ts.min(to) - from).max(at + 1);
            write(at, end - at, &pimg, &mut k)?;
        }
        if ts >= to {
            pending = None;
            break;
        }
        set_progress(progress, ts - from, len, 100, 990);
        pending = Some((ts, img));
    }
    if let Some((pt, pimg)) = pending {
        let at = (pt - from).max(0);
        write(at, (len - at).max(HNS / fps as i64), &pimg, &mut k)?;
    }
    audio_until(&mut k, i64::MAX)?;
    enc.finish().map_err(err)?;
    progress.store(1000, Ordering::Relaxed);
    Ok(())
}

/// The kept tracks of `from..to`, mixed (with their gains) into one stereo
/// track and saved as `format`.
pub fn audio(src: &Path, dst: &Path, format: AudioFormat, opt: &Options, progress: &AtomicU32, cancel: &AtomicBool) -> Result<(), String> {
    if opt.tracks.is_empty() {
        return Err(crate::i18n::l("все дорожки звука выключены", "every sound track is off").into());
    }
    let len = (opt.to - opt.from).max(1);
    let total = (len as i128 * AUDIO_RATE as i128 / HNS as i128) as usize * AUDIO_CHANNELS as usize;
    let mut mixed = vec![0f32; total];
    for (i, (stream, gain)) in opt.tracks.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let pcm = read_range(src, *stream, opt.from, total).map_err(err)?;
        for (m, s) in mixed.iter_mut().zip(&pcm) {
            *m += *s as f32 * gain;
        }
        set_progress(progress, i as i64 + 1, opt.tracks.len() as i64, 0, 400);
    }
    let pcm: Vec<i16> = mixed.iter().map(|s| s.clamp(-32768.0, 32767.0) as i16).collect();
    drop(mixed);
    let step = |f: f32| progress.store(400 + (590.0 * f.clamp(0.0, 1.0)) as u32, Ordering::Relaxed);
    match format {
        AudioFormat::Wav => write_wav(dst, &pcm).map_err(|e| e.to_string())?,
        AudioFormat::Mp3 => crate::record::encoder::write_mp3(dst, &pcm, &step)?,
        AudioFormat::M4a => {
            let enc = Encoder::new(dst, None, 1, None).map_err(err)?;
            let chunk = (AUDIO_RATE / 10 * AUDIO_CHANNELS) as usize; // 100 ms
            let n = pcm.len().div_ceil(chunk).max(1);
            for (i, c) in pcm.chunks(chunk).enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(());
                }
                enc.write_audio(0, c, i as i64 * HNS / 10).map_err(err)?;
                step((i + 1) as f32 / n as f32);
            }
            enc.end_audio().map_err(err)?;
            enc.finish().map_err(err)?;
        }
    }
    progress.store(1000, Ordering::Relaxed);
    Ok(())
}

/// 16-bit stereo PCM WAV at 48 kHz.
fn write_wav(dst: &Path, pcm: &[i16]) -> std::io::Result<()> {
    use std::io::Write;
    let data = (pcm.len() * 2) as u32;
    let mut f = std::io::BufWriter::new(std::fs::File::create(dst)?);
    let (rate, ch) = (AUDIO_RATE, AUDIO_CHANNELS as u16);
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&ch.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&(rate * ch as u32 * 2).to_le_bytes())?;
    f.write_all(&(ch * 2).to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data.to_le_bytes())?;
    for s in pcm {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()
}

/// `total` interleaved samples of a track starting exactly at `from`.
fn read_range(src: &Path, stream: u32, from: i64, total: usize) -> crate::media::Result<Vec<i16>> {
    let r = AudioReader::open(src, stream)?;
    r.seek(from)?;
    let mut out: Vec<i16> = Vec::with_capacity(total);
    let mut started = false;
    while out.len() < total {
        let Some((ts, pcm)) = r.next()? else { break };
        if !started {
            started = true;
            let rel = ts - from;
            if rel > 0 {
                let pad = (rel as i128 * AUDIO_RATE as i128 / HNS as i128) as usize * 2;
                out.extend(std::iter::repeat_n(0, pad.min(total)));
            } else {
                let skip = ((-rel) as i128 * AUDIO_RATE as i128 / HNS as i128) as usize * 2;
                if skip < pcm.len() {
                    out.extend_from_slice(&pcm[skip..]);
                } else {
                    started = false;
                }
                continue;
            }
        }
        out.extend_from_slice(&pcm);
    }
    out.resize(total, 0);
    Ok(out)
}

/// GIF of `from..to`: one palette for the whole clip, and each frame stores
/// only the rectangle that changed (see `gif_builtin`). On screen recordings
/// this is several times smaller than full frames with their own palettes.
pub fn gif(src: &Path, dst: &Path, opt: &Options, progress: &AtomicU32, cancel: &AtomicBool) -> Result<(), String> {
    gif_builtin(src, dst, opt, progress, cancel)
}

/// Calls `f(id, frame)` for every output slot (`from`, `from + 1/fps`, ...):
/// the frame on screen at that moment; `id` changes when the source frame does.
fn for_each_slot(
    src: &Path,
    opt: &Options,
    cancel: &AtomicBool,
    mut f: impl FnMut(u64, &image::RgbaImage, i64) -> Result<(), String>,
) -> Result<(), String> {
    let (from, to) = (opt.from, opt.to);
    let (reader, _) = VideoReader::open(src, opt.gif_width.max(64), 4096).map_err(err)?;
    reader.seek(from).map_err(err)?;
    let step = HNS / opt.gif_fps.clamp(1, 50) as i64;
    let mut slot = from;
    let mut cur: Option<(u64, image::RgbaImage)> = None;
    let mut id = 0u64;
    'outer: loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let Some((ts, img)) = reader.next().map_err(err)? else { break };
        while slot < ts.min(to) {
            if let Some((cid, cimg)) = &cur {
                f(*cid, cimg, slot)?;
            }
            slot += step;
            if slot >= to {
                break 'outer;
            }
        }
        id += 1;
        cur = Some((id, img));
    }
    while slot < to {
        if let Some((cid, cimg)) = &cur {
            f(*cid, cimg, slot)?;
        }
        slot += step;
    }
    Ok(())
}

/// Built-in GIF: one global palette (NeuQuant over sampled frames), no
/// dithering, each frame stores only the rectangle that changed (the rest is
/// transparent), identical frames merge into longer ones.
fn gif_builtin(src: &Path, dst: &Path, opt: &Options, progress: &AtomicU32, cancel: &AtomicBool) -> Result<(), String> {
    use std::collections::HashMap;
    let len = (opt.to - opt.from).max(1);
    let step = HNS / opt.gif_fps.clamp(1, 50) as i64;
    let slots = (len / step).max(1);

    // pass 1: palette from up to ~24 frames, every 3rd pixel
    let every = (slots / 24).max(1);
    let mut sample: Vec<u8> = Vec::new();
    let mut n = 0i64;
    let mut last_id = 0;
    for_each_slot(src, opt, cancel, |id, img, _| {
        if id != last_id && n % every == 0 {
            for px in img.as_raw().chunks_exact(4).step_by(3) {
                sample.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
        }
        last_id = id;
        n += 1;
        Ok(())
    })?;
    if sample.is_empty() {
        return Err("empty clip".into());
    }
    set_progress(progress, 1, 10, 0, 1000);
    let nq = color_quant::NeuQuant::new(10, 255, &sample);
    drop(sample);
    let mut palette = nq.color_map_rgb();
    palette.extend_from_slice(&[0, 0, 0]); // index 255: transparent
    const CLEAR: u8 = 255;

    // pass 2: index frames and write the changed rectangles
    let (reader, _) = VideoReader::open(src, opt.gif_width.max(64), 4096).map_err(err)?;
    let (w, h) = (reader.width as usize, reader.height as usize);
    drop(reader);
    let file = std::fs::File::create(dst).map_err(|e| e.to_string())?;
    let mut enc = gif::Encoder::new(std::io::BufWriter::new(file), w as u16, h as u16, &palette).map_err(|e| e.to_string())?;
    enc.set_repeat(gif::Repeat::Infinite).map_err(|e| e.to_string())?;
    let mut cache: HashMap<u32, u8> = HashMap::new();
    let mut prev: Option<Vec<u8>> = None;
    let mut pending: Option<gif::Frame<'static>> = None;
    let mut pending_cs: u32 = 0; // pending delay in 1/100 s ×1000 (sub-centisecond accuracy)
    let step_cs = (step / 100) as u32; // 100 ns → 1/100 s ×1000
    let mut last_id = 0u64;
    let write = |frame: Option<gif::Frame<'static>>, delay: u32, enc: &mut gif::Encoder<_>| -> Result<(), String> {
        if let Some(mut f) = frame {
            f.delay = ((delay + 500) / 1000).clamp(2, 65_535) as u16;
            enc.write_frame(&f).map_err(|e| e.to_string())?;
        }
        Ok(())
    };
    for_each_slot(src, opt, cancel, |id, img, slot| {
        set_progress(progress, slot - opt.from, len, 100, 990);
        if id == last_id {
            pending_cs += step_cs;
            return Ok(());
        }
        last_id = id;
        let idx: Vec<u8> = img
            .as_raw()
            .chunks_exact(4)
            .map(|p| {
                let key = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
                *cache.entry(key).or_insert_with(|| nq.index_of(&[p[0], p[1], p[2], 255]) as u8)
            })
            .collect();
        // bounding box of what changed since the last written frame
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
        if let Some(p) = &prev {
            for y in 0..h {
                let row = y * w;
                for x in 0..w {
                    if p[row + x] != idx[row + x] {
                        x0 = x0.min(x);
                        x1 = x1.max(x + 1);
                        y0 = y0.min(y);
                        y1 = y1.max(y + 1);
                    }
                }
            }
            if x1 == 0 {
                // looks the same after quantizing: just a longer frame
                pending_cs += step_cs;
                return Ok(());
            }
        } else {
            (x0, y0, x1, y1) = (0, 0, w, h);
        }
        let (fw, fh) = (x1 - x0, y1 - y0);
        let mut buf = Vec::with_capacity(fw * fh);
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * w + x;
                let same = prev.as_ref().is_some_and(|p| p[i] == idx[i]);
                buf.push(if same { CLEAR } else { idx[i] });
            }
        }
        let frame = gif::Frame {
            left: x0 as u16,
            top: y0 as u16,
            width: fw as u16,
            height: fh as u16,
            buffer: std::borrow::Cow::Owned(buf),
            transparent: prev.is_some().then_some(CLEAR),
            dispose: gif::DisposalMethod::Keep,
            ..Default::default()
        };
        write(pending.take(), pending_cs, &mut enc)?;
        pending = Some(frame);
        pending_cs = step_cs;
        prev = Some(idx);
        Ok(())
    })?;
    write(pending.take(), pending_cs, &mut enc)?;
    drop(enc);
    progress.store(1000, Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GIF size of `BLINKER_TEST_GIF` (mp4).
    #[test]
    fn gif_sizes() {
        let Ok(src) = std::env::var("BLINKER_TEST_GIF") else { return };
        let src = PathBuf::from(src);
        crate::media::startup();
        let info = crate::media::probe(&src).unwrap();
        let opt = Options {
            kind: Kind::Gif,
            from: 0,
            to: info.duration,
            tracks: vec![],
            mix_down: false,
            bitrate_quality: crate::config::RecQuality::Balanced,
            gif_width: 800,
            gif_fps: 15,
            quality: None,
            codec: crate::record::encoder::Codec::H264,
        };
        let (p, c) = (AtomicU32::new(0), AtomicBool::new(false));
        for name in ["builtin.gif"] {
            let dst = src.with_file_name(name);
            let t = std::time::Instant::now();
            let r = gif(&src, &dst, &opt, &p, &c);
            r.unwrap();
            eprintln!("{name}: {:.2} MB in {:?}", std::fs::metadata(&dst).unwrap().len() as f64 / 1e6, t.elapsed());
        }
    }

    /// Saves 0.5..2.5 s of the sound of `BLINKER_TEST_EXPORT` as MP3, M4A, WAV.
    #[test]
    fn export_audio() {
        let Ok(src) = std::env::var("BLINKER_TEST_EXPORT") else { return };
        let src = PathBuf::from(src);
        crate::media::startup();
        let info = crate::media::probe(&src).unwrap();
        let tracks: Vec<(u32, f32)> = info.audio_streams.iter().map(|s| (*s, 1.0)).collect();
        for f in AudioFormat::ALL {
            let dst = src.with_file_name(format!("sound.{}", f.ext()));
            let _ = std::fs::remove_file(&dst);
            let opt = Options {
                kind: Kind::Audio(f),
                from: HNS / 2,
                to: HNS * 5 / 2,
                tracks: tracks.clone(),
                mix_down: true,
                bitrate_quality: crate::config::RecQuality::Balanced,
                gif_width: 320,
                gif_fps: 15,
                quality: None,
                codec: crate::record::encoder::Codec::H264,
            };
            let (p, c) = (AtomicU32::new(0), AtomicBool::new(false));
            audio(&src, &dst, f, &opt, &p, &c).unwrap();
            let size = std::fs::metadata(&dst).unwrap().len();
            eprintln!("{}: {size} bytes", f.ext());
            assert!(size > 1000);
            assert_eq!(p.load(Ordering::Relaxed), 1000);
        }
    }

    /// Trims `BLINKER_TEST_EXPORT` (mp4 with audio) to 0.5..2.5 s as MP4
    /// (mixed and separate tracks) and GIF, then checks the results.
    #[test]
    fn export_trim() {
        let Ok(src) = std::env::var("BLINKER_TEST_EXPORT") else { return };
        let src = PathBuf::from(src);
        let info = crate::media::probe(&src).unwrap();
        let tracks: Vec<(u32, f32)> = info.audio_streams.iter().map(|s| (*s, 1.0)).collect();
        for (kind, mix, name) in [(Kind::Mp4, true, "mix.mp4"), (Kind::Mp4, false, "sep.mp4"), (Kind::Gif, true, "out.gif")] {
            let dst = src.with_file_name(name);
            let _ = std::fs::remove_file(&dst);
            let opt = Options {
                kind,
                from: HNS / 2,
                to: HNS * 5 / 2,
                tracks: tracks.clone(),
                mix_down: mix,
                bitrate_quality: crate::config::RecQuality::Balanced,
                gif_width: 320,
                gif_fps: 15,
                quality: None,
                codec: crate::record::encoder::Codec::H264,
            };
            let (p, c) = (AtomicU32::new(0), AtomicBool::new(false));
            let t = std::time::Instant::now();
            let r = match kind {
                Kind::Mp4 => mp4(&src, &dst, &info, &opt, &p, &c),
                Kind::Gif | Kind::Audio(_) => gif(&src, &dst, &opt, &p, &c),
            };
            r.unwrap();
            let size = std::fs::metadata(&dst).unwrap().len();
            eprintln!("{name}: {size} bytes in {:?}", t.elapsed());
            if kind == Kind::Mp4 {
                let out = crate::media::probe(&dst).unwrap();
                eprintln!("  {out:?}");
                assert_eq!(out.audio_tracks, if mix { 1 } else { tracks.len() });
                assert!((out.duration - 2 * HNS).abs() < HNS / 10, "{}", out.duration);
            }
        }
    }
}

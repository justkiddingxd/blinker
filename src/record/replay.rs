//! Instant replay: the screen is recorded all the time into short MP4 segments
//! (hardware encoder, keyframe every second) and the sound into raw PCM ring
//! files, both on disk, so RAM stays flat. Saving glues the segments covering
//! the last N seconds without re-encoding the video and encodes the sound.

use super::audio::{AudioCapture, Source};
#[cfg(windows)]
use super::encoder::audio_pcm_type;
#[cfg(windows)]
use super::encoder::AUDIO_CHANNELS;
use super::encoder::{AUDIO_RATE, Encoder, HNS, VideoSpec};
use super::video::{Capture, Frame, Gpu, Pacer};
use super::{AudioSink, Clock, Finished, VideoOut};
use crate::config::{Config, log};
use crate::win::{IRect, Monitor};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(windows)]
use windows::Win32::Media::MediaFoundation::*;

/// Length of one video segment.
const SEG: i64 = 5 * HNS;

/// How long the replay can be: 10 s .. 30 min.
pub const MIN_SECS: u32 = 10;
pub const MAX_SECS: u32 = 30 * 60;

struct Segment {
    path: PathBuf,
    start: i64,
    end: i64,
}

struct Current {
    enc: Arc<Encoder>,
    path: PathBuf,
    start: i64,
    last: i64,
}

pub struct Segmenter {
    dir: PathBuf,
    spec: VideoSpec,
    gpu: Arc<Gpu>,
    keep: AtomicI64,
    cur: Mutex<Option<Current>>,
    done: Arc<Mutex<VecDeque<Segment>>>,
    finalizing: Arc<AtomicUsize>,
    n: AtomicUsize,
}

unsafe impl Send for Segmenter {}
unsafe impl Sync for Segmenter {}

impl Segmenter {
    fn close_current(&self, cur: &mut Option<Current>) {
        if let Some(c) = cur.take() {
            self.finalizing.fetch_add(1, Ordering::SeqCst);
            let (done, fin) = (self.done.clone(), self.finalizing.clone());
            std::thread::spawn(move || {
                crate::media::startup();
                if c.enc.finish().is_ok() {
                    done.lock().unwrap().push_back(Segment { path: c.path, start: c.start, end: c.last });
                } else {
                    let _ = std::fs::remove_file(&c.path);
                }
                fin.fetch_sub(1, Ordering::SeqCst);
            });
        }
    }

    pub fn write_video(&self, tex: &Frame, t: i64, dur: i64) {
        let mut cur = self.cur.lock().unwrap();
        if cur.as_ref().is_some_and(|c| t - c.start >= SEG) {
            self.close_current(&mut cur);
        }
        if cur.is_none() {
            let n = self.n.fetch_add(1, Ordering::SeqCst);
            let path = self.dir.join(format!("seg-{n:06}.mp4"));
            match super::open_encoder(&path, &self.spec, 0, &self.gpu) {
                Ok((e, _)) => *cur = Some(Current { enc: Arc::new(e), path, start: t, last: t }),
                Err(e) => {
                    log::error(format!("replay segment: {e}"));
                    return;
                }
            }
        }
        if let Some(c) = cur.as_mut() {
            let _ = c.enc.write_texture(tex, t - c.start, dur);
            c.last = t + dur;
        }
    }

    /// Drops segments that fell out of the buffer.
    pub fn maintain(&self, now: i64) {
        let mut done = self.done.lock().unwrap();
        let keep = self.keep.load(Ordering::Relaxed);
        while done.front().is_some_and(|s| s.end < now - keep - SEG) {
            if let Some(s) = done.pop_front() {
                let _ = std::fs::remove_file(&s.path);
            }
        }
    }

    /// Finishes the open segment and waits until every segment is on disk.
    fn flush(&self) {
        self.close_current(&mut self.cur.lock().unwrap());
        let t = std::time::Instant::now();
        while self.finalizing.load(Ordering::SeqCst) > 0 && t.elapsed() < std::time::Duration::from_secs(8) {
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    }
}

/// One track's ring: `cap` frames in a file, frame `f` lives at `f % cap`.
struct Ring {
    file: File,
    cap: i64,
}

impl Ring {
    /// Frames `lo..hi` (all inside the ring) appended to `out` at `out_off`.
    fn read_into(&mut self, lo: i64, hi: i64, out: &mut [i16], out_off: usize) {
        let mut f0 = lo;
        while f0 < hi {
            let pos = f0 % self.cap;
            let run = (hi - f0).min(self.cap - pos);
            let mut buf = vec![0u8; (run * 4) as usize];
            if self.file.seek(SeekFrom::Start((pos * 4) as u64)).is_ok() && self.file.read_exact(&mut buf).is_ok() {
                let o = out_off + ((f0 - lo) * 2) as usize;
                for (k, ch) in buf.chunks_exact(2).enumerate() {
                    out[o + k] = i16::from_le_bytes([ch[0], ch[1]]);
                }
            }
            f0 += run;
        }
    }

    fn write_at(&mut self, f0: i64, bytes: &[u8]) {
        let mut off = 0usize;
        let mut cur = f0;
        while off < bytes.len() {
            let pos = cur % self.cap;
            let run = ((bytes.len() - off) as i64 / 4).min(self.cap - pos);
            if self.file.seek(SeekFrom::Start((pos * 4) as u64)).is_err()
                || self.file.write_all(&bytes[off..off + (run * 4) as usize]).is_err()
            {
                return;
            }
            off += (run * 4) as usize;
            cur += run;
        }
    }
}

/// PCM ring file per track (48 kHz stereo 16-bit), sized to the buffer.
pub struct PcmRings {
    rings: Vec<Mutex<Ring>>,
    /// Newest frame index written per track.
    heads: Vec<AtomicI64>,
}

fn ring_frames(secs: u32) -> i64 {
    (secs as i64 + 20) * AUDIO_RATE as i64
}

impl PcmRings {
    fn new(dir: &Path, tracks: usize, secs: u32) -> std::io::Result<Self> {
        let cap = ring_frames(secs);
        let mut rings = Vec::new();
        for i in 0..tracks {
            let f = File::options().create(true).truncate(true).read(true).write(true).open(dir.join(format!("pcm-{i}.raw")))?;
            f.set_len((cap * 4) as u64)?;
            rings.push(Mutex::new(Ring { file: f, cap }));
        }
        Ok(Self { rings, heads: (0..tracks).map(|_| AtomicI64::new(0)).collect() })
    }

    /// Interleaved samples of `track` for frames `from..to` (zeros where missing).
    fn read(&self, track: usize, from: i64, to: i64) -> Vec<i16> {
        let n = (to - from).max(0) as usize;
        let mut out = vec![0i16; n * 2];
        let mut r = self.rings[track].lock().unwrap();
        let head = self.heads[track].load(Ordering::SeqCst);
        let lo = from.max(head - r.cap).max(0);
        let hi = to.min(head);
        if hi > lo {
            r.read_into(lo, hi, &mut out, ((lo - from) * 2) as usize);
        }
        out
    }

    /// New buffer length; the sound already recorded is kept.
    fn resize(&self, secs: u32) {
        let cap = ring_frames(secs);
        for (track, ring) in self.rings.iter().enumerate() {
            let mut r = ring.lock().unwrap();
            if r.cap == cap {
                continue;
            }
            let head = self.heads[track].load(Ordering::SeqCst);
            let lo = (head - r.cap.min(cap)).max(0);
            let mut keep = vec![0i16; ((head - lo) * 2) as usize];
            r.read_into(lo, head, &mut keep, 0);
            let _ = r.file.set_len((cap * 4) as u64);
            r.cap = cap;
            let bytes: Vec<u8> = keep.iter().flat_map(|s| s.to_le_bytes()).collect();
            r.write_at(lo, &bytes);
        }
    }
}

impl AudioSink for PcmRings {
    fn write_audio(&self, track: usize, pcm: &[i16], t: i64) {
        let Some(ring) = self.rings.get(track) else { return };
        let mut r = ring.lock().unwrap();
        let mut f0 = t * AUDIO_RATE as i64 / HNS;
        let mut bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
        let frames = (pcm.len() / 2) as i64;
        if frames > r.cap {
            let skip = frames - r.cap;
            bytes.drain(..(skip * 4) as usize);
            f0 += skip;
        }
        r.write_at(f0, &bytes);
        self.heads[track].fetch_max(t * AUDIO_RATE as i64 / HNS + frames, Ordering::SeqCst);
    }
}

pub struct Replay {
    capture: Capture,
    pacer: Option<Pacer>,
    audio: Vec<AudioCapture>,
    end: Arc<AtomicI64>,
    clock: Arc<Clock>,
    seg: Arc<Segmenter>,
    rings: Arc<PcmRings>,
    tracks: Vec<Source>,
    secs: std::sync::atomic::AtomicU32,
    /// A save never reaches back before this (replay clock): the moment of
    /// the previous save, when saving starts the replay over.
    floor: AtomicI64,
    pub monitor: Monitor,
    size: (u32, u32),
    /// Settings this replay was started with (restart when they change).
    pub key: String,
}

pub fn dir() -> PathBuf {
    crate::config::cache_dir().join("replay")
}

/// The settings that require restarting the replay when changed.
pub fn config_key(cfg: &Config) -> String {
    format!(
        "{}|{}|{}|{}|{:?}|{}|{:?}|{}",
        cfg.replay_enabled, cfg.replay_desktop_audio, cfg.replay_mic, cfg.rec_fps, cfg.rec_quality, cfg.rec_cursor, cfg.rec_codec, cfg.mic_device
    )
}

impl Replay {
    /// Starts buffering the primary monitor.
    pub fn start(cfg: &Config) -> Result<Replay, String> {
        let dir = dir();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let monitor = crate::win::monitors()
            .into_iter()
            .find(|m| m.rect.contains(0, 0))
            .or_else(|| crate::win::monitors().into_iter().next())
            .ok_or(crate::i18n::t("нет монитора"))?;
        let (w, h) = ((monitor.rect.w as u32) & !1, (monitor.rect.h as u32) & !1);
        let target = super::video::monitor_target(IRect::new(monitor.rect.x, monitor.rect.y, w as i32, h as i32), &monitor);
        let gpu = Arc::new(Gpu::new().map_err(|e| format!("{}: {e}", super::video::GPU_API))?);
        let spec = super::video_spec(cfg, w, h, 1, &monitor);
        let secs = cfg.replay_secs.clamp(MIN_SECS, MAX_SECS);
        let seg = Arc::new(Segmenter {
            dir: dir.clone(),
            spec,
            gpu: gpu.clone(),
            keep: AtomicI64::new(secs as i64 * HNS),
            cur: Mutex::new(None),
            done: Arc::new(Mutex::new(VecDeque::new())),
            finalizing: Arc::new(AtomicUsize::new(0)),
            n: AtomicUsize::new(0),
        });
        super::audio::use_mic(&cfg.mic_device);
        let tracks = super::audio_sources(cfg.replay_desktop_audio, cfg.replay_mic);
        let rings = Arc::new(PcmRings::new(&dir, tracks.len(), secs).map_err(|e| e.to_string())?);
        let clock = Clock::new();
        let end = Arc::new(AtomicI64::new(0));
        let capture = Capture::start(&gpu, target, spec.fps, cfg.rec_cursor, clock.clone()).map_err(|e| lf!("захват экрана: {e}", "screen capture: {e}"))?;
        let pacer = Pacer::start(gpu, &capture, target, VideoOut::Replay(seg.clone()), spec.fps, clock.clone(), None, (monitor.rect.x, monitor.rect.y));
        let mut audio = Vec::new();
        for (i, src) in tracks.iter().enumerate() {
            match AudioCapture::start(*src, rings.clone(), i, clock.clone(), end.clone()) {
                Ok(a) => audio.push(a),
                Err(e) => log::error(format!("replay audio {src:?}: {e}")),
            }
        }
        Ok(Replay {
            capture,
            pacer: Some(pacer),
            audio,
            end,
            clock,
            seg,
            rings,
            tracks,
            secs: std::sync::atomic::AtomicU32::new(secs),
            floor: AtomicI64::new(0),
            monitor,
            size: (w, h),
            key: config_key(cfg),
        })
    }

    /// Changes the buffer length without dropping what is already recorded.
    pub fn set_secs(&self, secs: u32) {
        let secs = secs.clamp(MIN_SECS, MAX_SECS);
        if self.secs.swap(secs, Ordering::SeqCst) != secs {
            self.seg.keep.store(secs as i64 * HNS, Ordering::SeqCst);
            self.rings.resize(secs);
        }
    }

    pub fn stop(mut self) {
        self.capture.close();
        self.end.store(self.clock.now().max(1), Ordering::SeqCst);
        if let Some(p) = self.pacer.take() {
            p.stop();
        }
        for a in std::mem::take(&mut self.audio) {
            a.join();
        }
        self.seg.flush();
        let _ = std::fs::remove_dir_all(dir());
    }

    /// Saves the last `secs` seconds as an MP4 in the captures folder (with
    /// `replay_reset_on_save`, only what came after the previous save).
    pub fn save(&self, cfg: &Config) -> Result<Finished, String> {
        let now = self.clock.now();
        self.seg.flush();
        let from = (now - self.secs.load(Ordering::SeqCst) as i64 * HNS).max(self.floor.load(Ordering::SeqCst)).max(0);
        if now - from < HNS {
            return Err(crate::i18n::t("буфер повтора ещё пуст").into());
        }
        let segs: Vec<(PathBuf, i64)> = self
            .seg
            .done
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.end > from && s.start < now)
            .map(|s| (s.path.clone(), s.start))
            .collect();
        if segs.is_empty() {
            return Err(crate::i18n::t("буфер повтора ещё пуст").into());
        }
        let path = super::clip_path(cfg, crate::i18n::t("blinker повтор"), "mp4", super::game_now().as_deref());
        let duration = remux(&segs, &path, from, now, &self.rings, self.tracks.len()).map_err(|e| {
            let _ = std::fs::remove_file(&path);
            lf!("не удалось сохранить повтор: {e}", "could not save the replay: {e}")
        })?;
        crate::strip::file(&path);
        let thumb = crate::media::VideoReader::open(&path, 640, 360)
            .ok()
            .and_then(|(r, _)| {
                let _ = r.seek(duration.saturating_sub(HNS / 2));
                let mut last = None;
                while let Ok(Some((_, img))) = r.next() {
                    last = Some(img);
                }
                last
            });
        if cfg.replay_reset_on_save {
            self.floor.store(now, Ordering::SeqCst);
        }
        let m = self.monitor;
        Ok(Finished {
            path,
            gif: false,
            thumb,
            duration: duration as f64 / HNS as f64,
            rect: IRect::new(m.rect.x, m.rect.y, self.size.0 as i32, self.size.1 as i32),
            monitor: m,
            tracks: self.tracks.clone(),
        })
    }
}

/// Glues `segs` (path, start on the replay clock) into `dst`, starting at the
/// first keyframe at or after `from`; the sound comes from the PCM rings.
/// Returns the duration.
#[cfg(target_os = "macos")]
fn remux(segs: &[(PathBuf, i64)], dst: &Path, from: i64, to: i64, rings: &PcmRings, tracks: usize) -> crate::media::Result<i64> {
    // The video is copied as is; the sound covers the same stretch.
    let (base, len) = super::encoder::scan_segments(segs, from, to)?;
    let f0 = base * AUDIO_RATE as i64 / HNS;
    let f1 = (base + len) * AUDIO_RATE as i64 / HNS;
    let pcm: Vec<Vec<i16>> = (0..tracks).map(|t| rings.read(t, f0, f1)).collect();
    super::encoder::remux_segments(segs, dst, base, base + len, &pcm)?;
    Ok(len)
}

#[cfg(windows)]
fn remux(segs: &[(PathBuf, i64)], dst: &Path, from: i64, to: i64, rings: &PcmRings, tracks: usize) -> windows::core::Result<i64> {
    crate::media::startup();
    unsafe {
        let first = crate::media::open_reader(&segs[0].0, false)?;
        let vtype = first.GetNativeMediaType(0, 0)?;
        drop(first);
        let mut a = None;
        MFCreateAttributes(&mut a, 2)?;
        let a = a.unwrap();
        a.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
        a.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
        let w = crate::media::wide(dst);
        let writer = MFCreateSinkWriterFromURL(windows::core::PCWSTR(w.as_ptr()), None, &a)?;
        // video: compressed samples pass straight through
        let v = writer.AddStream(&vtype)?;
        writer.SetInputMediaType(v, &vtype, None)?;
        let mut audio_idx = Vec::new();
        for t in 0..tracks {
            let out = MFCreateMediaType()?;
            out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            out.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
            out.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            out.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, AUDIO_RATE)?;
            out.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, AUDIO_CHANNELS)?;
            out.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, if t == 0 { 16_000 } else { 12_000 })?;
            let idx = writer.AddStream(&out)?;
            writer.SetInputMediaType(idx, &audio_pcm_type(AUDIO_RATE, AUDIO_CHANNELS)?, None)?;
            audio_idx.push(idx);
        }
        writer.BeginWriting()?;

        // Compressed video samples of the stretch (small: no decoding).
        let mut base: Option<i64> = None;
        let mut video: Vec<IMFSample> = Vec::new();
        let mut last_end = 0i64;
        for (path, start) in segs {
            let r = crate::media::open_reader(path, false)?;
            loop {
                let mut flags = 0u32;
                let mut ts = 0i64;
                let mut sample = None;
                r.ReadSample(0, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample))?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    break;
                }
                let Some(s) = sample else { continue };
                let abs = start + ts;
                if abs >= to {
                    break;
                }
                if base.is_none() {
                    let key = s.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0;
                    if abs < from || !key {
                        continue;
                    }
                    base = Some(abs);
                }
                let b = base.unwrap();
                s.SetSampleTime(abs - b)?;
                let dur = s.GetSampleDuration().unwrap_or(HNS / 60);
                last_end = last_end.max(abs - b + dur);
                video.push(s);
            }
        }
        let Some(b) = base else {
            return Err(windows::core::Error::from_hresult(MF_E_INVALIDREQUEST));
        };
        let len = (to - b).max(last_end);
        let f0 = b * AUDIO_RATE as i64 / HNS;
        let f1 = (b + len) * AUDIO_RATE as i64 / HNS;
        let pcm: Vec<Vec<i16>> = (0..audio_idx.len()).map(|t| rings.read(t, f0, f1)).collect();
        // Interleave by time: the sink writer stalls when one stream runs far ahead.
        let chunk = (AUDIO_RATE / 10 * AUDIO_CHANNELS) as usize; // 100 ms
        let mut k = 0usize;
        let write_audio_until = |k: &mut usize, until: i64| -> windows::core::Result<()> {
            loop {
                let t = (*k * chunk / 2) as i64 * HNS / AUDIO_RATE as i64;
                if t > until || pcm.first().is_none_or(|p| *k * chunk >= p.len()) {
                    return Ok(());
                }
                for (tr, idx) in audio_idx.iter().enumerate() {
                    let c = &pcm[tr][*k * chunk..((*k + 1) * chunk).min(pcm[tr].len())];
                    let bytes = c.len() * 2;
                    let buf = MFCreateMemoryBuffer(bytes as u32)?;
                    let mut p = std::ptr::null_mut();
                    buf.Lock(&mut p, None, None)?;
                    std::ptr::copy_nonoverlapping(c.as_ptr() as *const u8, p, bytes);
                    buf.Unlock()?;
                    buf.SetCurrentLength(bytes as u32)?;
                    let s = MFCreateSample()?;
                    s.AddBuffer(&buf)?;
                    s.SetSampleTime(t)?;
                    s.SetSampleDuration((c.len() / 2) as i64 * HNS / AUDIO_RATE as i64)?;
                    writer.WriteSample(*idx, &s)?;
                }
                *k += 1;
            }
        };
        for s in &video {
            let t = s.GetSampleTime()?;
            write_audio_until(&mut k, t)?;
            writer.WriteSample(v, s)?;
        }
        write_audio_until(&mut k, i64::MAX)?;
        writer.Finalize()?;
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Growing and shrinking the PCM ring keeps the newest sound in place.
    #[test]
    fn ring_resize_keeps_audio() {
        let dir = std::env::temp_dir().join(format!("blinker-ring-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rings = PcmRings::new(&dir, 1, 10).unwrap();
        // 25 s of a counting signal (wraps the 30 s ring? no: cap = 10 + 20 s)
        let sample = |f: i64| (f % 30000) as i16;
        let chunk = 4800i64;
        for k in 0..(25 * 48_000 / chunk) {
            let f0 = k * chunk;
            let pcm: Vec<i16> = (0..chunk).flat_map(|i| [sample(f0 + i), sample(f0 + i)]).collect();
            rings.write_audio(0, &pcm, f0 * HNS / 48_000);
        }
        let head = 25 * 48_000;
        let check = |rings: &PcmRings, secs: i64| {
            let from = head - secs * 48_000;
            let got = rings.read(0, from, head);
            for (i, pair) in got.chunks_exact(2).enumerate() {
                assert_eq!(pair[0], sample(from + i as i64), "frame {}", from + i as i64);
            }
        };
        check(&rings, 20);
        rings.resize(300); // grow: everything stays
        check(&rings, 25);
        rings.resize(10); // shrink to 30 s ring: still all 25 s
        check(&rings, 25);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Buffers ~7 s of the primary monitor, saves the replay, checks it,
    /// deletes it. `BLINKER_TEST_RECORD` = scratch folder.
    #[test]
    fn replay_smoke() {
        let Ok(dir) = std::env::var("BLINKER_TEST_RECORD") else { return };
        crate::media::mark_main_thread();
        let mut cfg = crate::config::Config { save_dir: dir.into(), ..Default::default() };
        cfg.replay_secs = 10;
        cfg.replay_mic = false;
        cfg.replay_desktop_audio = true;
        let t0 = std::time::Instant::now();
        let r = super::Replay::start(&cfg).expect("start");
        eprintln!("started in {:?}", t0.elapsed());
        std::thread::sleep(std::time::Duration::from_millis(7200));
        let t = std::time::Instant::now();
        let f = r.save(&cfg).expect("save");
        eprintln!("saved in {:?}", t.elapsed());
        let info = crate::media::probe(&f.path).expect("probe");
        eprintln!("{info:?} thumb={:?}", f.thumb.as_ref().map(|t| t.dimensions()));
        let _ = std::fs::remove_file(&f.path);
        r.stop();
        assert!(info.duration > 50_000_000, "{}", info.duration);
        assert_eq!(info.audio_tracks, f.tracks.len());
    }
}

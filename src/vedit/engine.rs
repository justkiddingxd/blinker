//! Background work for the video editor: frame decoding / paced playback,
//! audio playback mixed from the separate tracks, filmstrip thumbnails and
//! waveforms. Everything talks to the UI through small shared states.

use crate::media::{AudioReader, VideoReader};
use crate::record::encoder::HNS;
use eframe::egui;
use image::RgbaImage;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct Frame {
    #[allow(dead_code)]
    pub t: i64,
    pub img: RgbaImage,
    pub seq: u64,
}

/// Per-track playback settings, shared with the audio thread.
#[derive(Clone, Copy)]
pub struct Gain {
    pub muted: bool,
    pub volume: f32,
}

impl Gain {
    pub fn factor(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume }
    }
}

enum Cmd {
    /// Show the frame at `t` (paused).
    Show(i64),
    /// Play `from..to` at `rate` (2 = twice as fast), `from` maps to `start`.
    Play { from: i64, to: i64, start: Instant, rate: f32 },
    Pause,
}

pub struct Engine {
    cmd: Sender<Cmd>,
    pub frame: Arc<Mutex<Option<Frame>>>,
    /// Set by the video thread when playback reached its end.
    pub ended: Arc<AtomicBool>,
    pub gains: Arc<Mutex<Vec<Gain>>>,
    audio_stop: Option<Arc<AtomicBool>>,
    path: PathBuf,
    streams: Vec<u32>,
}

impl Engine {
    pub fn new(ctx: &egui::Context, path: &Path, preview: (u32, u32), streams: Vec<u32>) -> Self {
        let (tx, rx) = channel();
        let frame = Arc::new(Mutex::new(None));
        let ended = Arc::new(AtomicBool::new(false));
        {
            let (frame, ended, ctx, path) = (frame.clone(), ended.clone(), ctx.clone(), path.to_path_buf());
            std::thread::spawn(move || video_thread(&path, preview, rx, frame, ended, ctx));
        }
        let gains = Arc::new(Mutex::new(vec![Gain { muted: false, volume: 1.0 }; streams.len()]));
        Self { cmd: tx, frame, ended, gains, audio_stop: None, path: path.to_path_buf(), streams }
    }

    pub fn show(&self, t: i64) {
        let _ = self.cmd.send(Cmd::Show(t));
    }

    /// Plays `from..to` at `rate`; `music`: a music file mixed in, from its
    /// position (100 ns; negative: it starts that much later) and at its gain.
    pub fn play(&mut self, from: i64, to: i64, rate: f32, music: Option<(PathBuf, i64, f32)>) {
        self.pause();
        self.ended.store(false, Ordering::SeqCst);
        let start = Instant::now() + Duration::from_millis(60);
        let _ = self.cmd.send(Cmd::Play { from, to, start, rate });
        if !self.streams.is_empty() || music.is_some() {
            let stop = Arc::new(AtomicBool::new(false));
            let (p, s, g, st) = (self.path.clone(), self.streams.clone(), self.gains.clone(), stop.clone());
            std::thread::spawn(move || audio_thread(&p, &s, from, to, start, rate, music, g, st));
            self.audio_stop = Some(stop);
        }
    }

    pub fn pause(&mut self) {
        let _ = self.cmd.send(Cmd::Pause);
        if let Some(s) = self.audio_stop.take() {
            s.store(true, Ordering::SeqCst);
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.pause();
    }
}

fn video_thread(
    path: &Path,
    preview: (u32, u32),
    rx: Receiver<Cmd>,
    frame: Arc<Mutex<Option<Frame>>>,
    ended: Arc<AtomicBool>,
    ctx: egui::Context,
) {
    let Ok((reader, _)) = VideoReader::open(path, preview.0, preview.1) else { return };
    let mut seq = 0u64;
    // Frame on screen, and the one decoded after it (reader is past both).
    let mut last: Option<(i64, RgbaImage)> = None;
    let mut ahead: Option<(i64, RgbaImage)> = None;
    let mut publish = |t: i64, img: &RgbaImage| {
        seq += 1;
        *frame.lock().unwrap() = Some(Frame { t, img: img.clone(), seq });
        ctx.request_repaint();
    };
    // Decodes to the frame on screen at `t`: the last one with ts <= t.
    let goto = |reader: &VideoReader, last: &mut Option<(i64, RgbaImage)>, ahead: &mut Option<(i64, RgbaImage)>, t: i64| -> bool {
        let near = last.as_ref().is_some_and(|(lt, _)| *lt <= t && t - *lt < 2 * HNS);
        if !near {
            let _ = reader.seek(t);
            *last = None;
            *ahead = None;
        }
        loop {
            let next = match ahead.take() {
                Some(f) => Some(f),
                None => reader.next().ok().flatten(),
            };
            match next {
                Some((ts, img)) => {
                    if ts > t && last.is_some() {
                        *ahead = Some((ts, img));
                        return true;
                    }
                    *last = Some((ts, img));
                    if ts >= t {
                        return true;
                    }
                }
                None => return last.is_some(),
            }
        }
    };
    let mut pending: Option<Cmd> = None;
    loop {
        let cmd = match pending.take() {
            Some(c) => c,
            None => match rx.recv() {
                Ok(c) => c,
                Err(_) => return,
            },
        };
        match cmd {
            Cmd::Pause => {}
            Cmd::Show(mut t) => {
                // Only the newest position matters while scrubbing.
                while let Ok(c) = rx.try_recv() {
                    match c {
                        Cmd::Show(t2) => t = t2,
                        other => {
                            pending = Some(other);
                            break;
                        }
                    }
                }
                if goto(&reader, &mut last, &mut ahead, t)
                    && let Some((lt, img)) = &last
                {
                    publish(*lt, img);
                }
            }
            Cmd::Play { from, to, start, rate } => {
                let _ = reader.seek(from);
                last = None;
                ahead = None;
                'play: loop {
                    let next = match reader.next() {
                        Ok(Some(f)) => f,
                        _ => break,
                    };
                    let (ts, img) = next;
                    if ts >= to {
                        break;
                    }
                    if ts + HNS / 30 < from {
                        last = Some((ts, img));
                        continue;
                    }
                    let due = start + Duration::from_nanos((((ts - from).max(0) * 100) as f64 / rate.max(0.1) as f64) as u64);
                    loop {
                        match rx.try_recv() {
                            Ok(c) => {
                                pending = Some(c);
                                break 'play;
                            }
                            Err(TryRecvError::Disconnected) => return,
                            Err(TryRecvError::Empty) => {}
                        }
                        let now = Instant::now();
                        if now >= due {
                            break;
                        }
                        std::thread::sleep((due - now).min(Duration::from_millis(5)));
                    }
                    publish(ts, &img);
                    last = Some((ts, img));
                }
                if pending.is_none() {
                    ended.store(true, Ordering::SeqCst);
                    ctx.request_repaint();
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Audio

/// Pull-based rodio source fed by the decoding thread.
struct Stream {
    rx: Receiver<Vec<f32>>,
    buf: Vec<f32>,
    pos: usize,
    done: bool,
}

impl Iterator for Stream {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.pos >= self.buf.len() {
            match self.rx.try_recv() {
                Ok(b) => {
                    self.buf = b;
                    self.pos = 0;
                }
                Err(TryRecvError::Empty) if !self.done => return Some(0.0),
                Err(TryRecvError::Disconnected) => {
                    self.done = true;
                    return None;
                }
                Err(_) => return None,
            }
            if self.buf.is_empty() {
                return Some(0.0);
            }
        }
        let v = self.buf[self.pos];
        self.pos += 1;
        Some(v)
    }
}

impl rodio::Source for Stream {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> rodio::ChannelCount {
        std::num::NonZero::new(2).unwrap()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        std::num::NonZero::new(48_000).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Track reader that yields samples starting exactly at `from`.
struct TrackCursor {
    reader: AudioReader,
    buf: std::collections::VecDeque<i16>,
    skip: usize,
    eof: bool,
}

impl TrackCursor {
    fn open(path: &Path, stream: u32, from: i64) -> Option<Self> {
        let reader = AudioReader::open(path, stream).ok()?;
        let _ = reader.seek(from);
        Some(Self { reader, buf: Default::default(), skip: 0, eof: false }).map(|mut c| {
            c.skip = usize::MAX; // decided by the first chunk's timestamp
            c.prime(from);
            c
        })
    }

    fn prime(&mut self, from: i64) {
        while self.skip == usize::MAX {
            match self.reader.next() {
                Ok(Some((ts, pcm))) => {
                    let lead = (((from - ts).max(0) as i128 * 48_000 / HNS as i128) as usize) * 2;
                    if lead >= pcm.len() {
                        continue;
                    }
                    // Leading silence when the chunk starts after `from`.
                    let pad = (((ts - from).max(0) as i128 * 48_000 / HNS as i128) as usize) * 2;
                    self.buf.extend(std::iter::repeat_n(0, pad));
                    self.buf.extend(&pcm[lead..]);
                    self.skip = 0;
                }
                _ => {
                    self.eof = true;
                    self.skip = 0;
                }
            }
        }
    }

    /// Fills `out` (interleaved stereo) with up to `out.len()` samples; zeros past the end.
    fn take(&mut self, n: usize) -> Vec<i16> {
        while self.buf.len() < n && !self.eof {
            match self.reader.next() {
                Ok(Some((_, pcm))) => self.buf.extend(pcm),
                _ => self.eof = true,
            }
        }
        let k = n.min(self.buf.len());
        let mut v: Vec<i16> = self.buf.drain(..k).collect();
        v.resize(n, 0);
        v
    }
}

#[allow(clippy::too_many_arguments)]
fn audio_thread(
    path: &Path,
    streams: &[u32],
    from: i64,
    to: i64,
    start: Instant,
    rate: f32,
    music: Option<(PathBuf, i64, f32)>,
    gains: Arc<Mutex<Vec<Gain>>>,
    stop: Arc<AtomicBool>,
) {
    crate::media::startup();
    let mut tracks: Vec<TrackCursor> = streams.iter().filter_map(|s| TrackCursor::open(path, *s, from)).collect();
    // the music runs at its own pace, whatever the clip's speed
    let mut tune = music.as_ref().and_then(|(p, at, _)| crate::media::probe(p).ok().and_then(|i| i.audio_streams.first().copied()).and_then(|s| TrackCursor::open(p, s, *at)));
    let tune_gain = music.as_ref().map_or(0.0, |m| m.2);
    if tracks.is_empty() && tune.is_none() {
        return;
    }
    let Ok(mut sink) = rodio::DeviceSinkBuilder::open_default_sink() else { return };
    sink.log_on_drop(false);
    let (tx, rx) = channel();
    let player = rodio::Player::connect_new(sink.mixer());
    // Start in sync with the video clock.
    let now = Instant::now();
    if start > now {
        std::thread::sleep(start - now);
    }
    player.append(Stream { rx, buf: Vec::new(), pos: 0, done: false });
    let chunk = 4800usize; // 50 ms of stereo frames
    // what's heard: the clip's span at `rate`
    let rate = rate.clamp(0.1, 8.0);
    let total = (((to - from).max(0) as f64 / rate as f64) as i128 * 48_000 / HNS as i128) as usize * 2;
    let mut sent = 0usize;
    let began = Instant::now();
    while sent < total && !stop.load(Ordering::SeqCst) {
        // Stay ~300 ms ahead of the speaker.
        let ahead = sent as f64 / 96_000.0 - began.elapsed().as_secs_f64();
        if ahead > 0.3 {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        let n = chunk.min(total - sent);
        let g = gains.lock().unwrap().clone();
        let mut mix = vec![0f32; n];
        // a faster piece: more of the clip squeezed into the same time (the
        // saved file keeps the pitch; here it just plays faster)
        let need = if (rate - 1.0).abs() < 0.01 { n } else { (((n / 2) as f32 * rate).ceil() as usize).max(1) * 2 };
        for (i, t) in tracks.iter_mut().enumerate() {
            let k = g.get(i).map(|g| g.factor()).unwrap_or(1.0);
            let pcm = t.take(need);
            let pcm = if need == n { pcm } else { resample(&pcm, n) };
            if k > 0.0 {
                for (m, s) in mix.iter_mut().zip(pcm) {
                    *m += s as f32 / 32768.0 * k;
                }
            }
        }
        if let Some(t) = &mut tune {
            for (m, s) in mix.iter_mut().zip(t.take(n)) {
                *m += s as f32 / 32768.0 * tune_gain;
            }
        }
        for m in &mut mix {
            *m = m.clamp(-1.0, 1.0);
        }
        if tx.send(mix).is_err() {
            break;
        }
        sent += n;
    }
    drop(tx);
    if stop.load(Ordering::SeqCst) {
        player.stop();
    } else {
        player.sleep_until_end();
    }
}

/// Interleaved stereo `pcm` stretched or squeezed to `n` samples (linear).
fn resample(pcm: &[i16], n: usize) -> Vec<i16> {
    let (src, dst) = (pcm.len() / 2, n / 2);
    if src == 0 || dst == 0 {
        return vec![0; n];
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..dst {
        let f = i as f32 * (src - 1) as f32 / (dst.max(2) - 1) as f32;
        let (a, k) = (f as usize, f.fract());
        let b = (a + 1).min(src - 1);
        for c in 0..2 {
            let v = pcm[a * 2 + c] as f32 * (1.0 - k) + pcm[b * 2 + c] as f32 * k;
            out.push(v as i16);
        }
    }
    out.resize(n, 0);
    out
}

// ---------------------------------------------------------------------------
// Filmstrip and waveforms

pub struct Thumbs {
    pub items: Arc<Mutex<Vec<Option<RgbaImage>>>>,
    pub version: Arc<AtomicU64>,
}

pub fn thumbnails(ctx: &egui::Context, path: &Path, duration: i64, count: usize, size: (u32, u32)) -> Thumbs {
    let items = Arc::new(Mutex::new(vec![None; count]));
    let version = Arc::new(AtomicU64::new(0));
    let (it, ver, ctx, path) = (items.clone(), version.clone(), ctx.clone(), path.to_path_buf());
    std::thread::spawn(move || {
        let Ok((r, _)) = VideoReader::open(&path, size.0, size.1) else { return };
        for i in 0..count {
            let t = duration * (2 * i as i64 + 1) / (2 * count as i64);
            let _ = r.seek(t);
            if let Ok(Some((_, img))) = r.next() {
                it.lock().unwrap()[i] = Some(img);
                ver.fetch_add(1, Ordering::SeqCst);
                ctx.request_repaint();
            }
        }
    });
    Thumbs { items, version }
}

/// Peak level (0..1) per bucket across the whole track.
pub fn waveform(ctx: &egui::Context, path: &Path, stream: u32, duration: i64, buckets: usize) -> Arc<Mutex<Option<Vec<f32>>>> {
    let out = Arc::new(Mutex::new(None));
    let (o, ctx, path) = (out.clone(), ctx.clone(), path.to_path_buf());
    std::thread::spawn(move || {
        let Ok(r) = AudioReader::open(&path, stream) else { return };
        let total = (duration as f64 / HNS as f64 * 48_000.0).max(1.0);
        let mut peaks = vec![0f32; buckets];
        let mut frame = 0usize;
        while let Ok(Some((_, pcm))) = r.next() {
            for pair in pcm.chunks_exact(2) {
                let b = ((frame as f64 / total) * buckets as f64) as usize;
                if b < buckets {
                    let v = (pair[0].unsigned_abs().max(pair[1].unsigned_abs())) as f32 / 32768.0;
                    if v > peaks[b] {
                        peaks[b] = v;
                    }
                }
                frame += 1;
            }
        }
        *o.lock().unwrap() = Some(peaks);
        ctx.request_repaint();
    });
    out
}

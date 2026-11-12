//! The instant replay's video: one encoder running all the time, its packets
//! in a ring in memory. A save writes the stretch asked for into an MP4; the
//! encoder doesn't notice. Nothing is opened and closed every few seconds and
//! nothing goes to the disk until then.

use super::encoder::{HNS, VideoSpec};
use super::live::{self, LiveEncoder, Packet};
use super::video::{Frame, Gpu};
use crate::config::log;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Media::MediaFoundation::IMFMediaType;

/// How long after a failure a new encoder is tried.
const RETRY: Duration = Duration::from_secs(5);
/// Kept beyond the replay's length: the stretch starts at a keyframe.
const MARGIN: i64 = 2 * HNS;

/// A packet and the encoder it came from.
pub struct Stored {
    pub p: Packet,
    generation: u32,
}

#[derive(Default)]
struct Store {
    list: VecDeque<Arc<Stored>>,
    bytes: usize,
    generation: u32,
    /// The description of the current encoder's stream.
    vtype: Option<IMFMediaType>,
    /// The replay was cut short to fit in memory (said once in the log).
    cut: bool,
}

unsafe impl Send for Store {}

impl Store {
    fn push(&mut self, s: Arc<Stored>, keep: i64, budget: usize) {
        let newest = s.p.t;
        self.bytes += s.p.data.len();
        self.list.push_back(s);
        let over_time = |st: &Store| st.list.front().is_some_and(|f| newest - f.p.t > keep + MARGIN);
        while over_time(self) || self.bytes > budget {
            if self.bytes > budget && !over_time(self) && !self.cut {
                self.cut = true;
                log::info(format!("replay: {} MB in memory at most: it holds less than the length set", budget >> 20));
            }
            self.drop_group();
            if self.list.len() < 2 {
                break;
            }
        }
    }

    /// The oldest packet and everything up to the next keyframe: what's left
    /// always starts with a picture that can be started from.
    fn drop_group(&mut self) {
        if let Some(f) = self.list.pop_front() {
            self.bytes -= f.p.data.len();
        }
        while self.list.front().is_some_and(|f| !f.p.key) {
            if let Some(f) = self.list.pop_front() {
                self.bytes -= f.p.data.len();
            }
        }
    }

    /// What a save of `from..to` takes: from the first keyframe at or after
    /// `from`, only what the current encoder made (another one's packets
    /// don't go with its description).
    fn stretch(&self, from: i64, to: i64) -> Vec<Arc<Stored>> {
        let mut out = Vec::new();
        for s in &self.list {
            if s.generation != self.generation || s.p.t >= to {
                continue;
            }
            if out.is_empty() && (s.p.t < from || !s.p.key) {
                continue;
            }
            out.push(s.clone());
        }
        out
    }
}

/// Memory the ring may take: a quarter of the RAM, 2 GB at most.
fn budget() -> usize {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    let ram = if unsafe { GlobalMemoryStatusEx(&mut m) } != 0 { m.ullTotalPhys } else { 8 << 30 };
    ((ram / 4) as usize).clamp(256 << 20, 2 << 30)
}

pub struct RingWriter {
    gpu: Arc<Gpu>,
    spec: VideoSpec,
    keep: Arc<AtomicI64>,
    enc: Mutex<Option<Arc<LiveEncoder>>>,
    store: Arc<Mutex<Store>>,
    budget: usize,
    tried: Mutex<Instant>,
    said_dropped: Mutex<Instant>,
}

unsafe impl Send for RingWriter {}
unsafe impl Sync for RingWriter {}

impl RingWriter {
    /// Starts it with an encoder for `spec`; None if there's no hardware one
    /// for it. With the settings the encoder took.
    pub fn start(gpu: &Arc<Gpu>, spec: &VideoSpec, secs: u32) -> Option<(Arc<RingWriter>, VideoSpec)> {
        let (enc, taken) = live::open_ladder(gpu, spec)?;
        let writer = Arc::new(RingWriter {
            gpu: gpu.clone(),
            spec: taken,
            keep: Arc::new(AtomicI64::new(secs as i64 * HNS)),
            enc: Mutex::new(None),
            store: Arc::new(Mutex::new(Store::default())),
            budget: budget(),
            tried: Mutex::new(Instant::now()),
            said_dropped: Mutex::new(Instant::now()),
        });
        writer.attach(enc);
        Some((writer, taken))
    }

    /// `enc` takes over: its packets go in the ring from now on.
    fn attach(&self, enc: LiveEncoder) {
        let generation = {
            let mut s = self.store.lock().unwrap();
            s.generation += 1;
            s.vtype = Some(enc.out_type().clone());
            s.generation
        };
        let (store, keep, budget) = (self.store.clone(), self.keep.clone(), self.budget);
        enc.start(move |p| {
            let k = keep.load(Ordering::Relaxed);
            store.lock().unwrap().push(Arc::new(Stored { p, generation }), k, budget);
        });
        *self.enc.lock().unwrap() = Some(Arc::new(enc));
    }

    pub fn set_keep(&self, secs: u32) {
        self.keep.store(secs as i64 * HNS, Ordering::SeqCst);
    }

    pub fn write_video(&self, tex: &Frame, t: i64, dur: i64) {
        let cur = self.enc.lock().unwrap().clone();
        match cur {
            Some(e) if !e.broken() => {
                e.write(tex, t, dur);
                let mut said = self.said_dropped.lock().unwrap();
                if e.dropped() > 0 && said.elapsed() > Duration::from_secs(600) {
                    *said = Instant::now();
                    log::info(format!("replay: the encoder is behind, {} frames dropped so far", e.dropped()));
                }
            }
            Some(e) => {
                // stuck or failed: a new one takes over. This one is finished
                // on the side, it may never return
                log::error(format!("replay: the encoder ({}) stopped working, a new one takes over", e.name));
                *self.enc.lock().unwrap() = None;
                *self.tried.lock().unwrap() = Instant::now() - RETRY;
                std::thread::spawn(move || {
                    e.finish(Duration::from_secs(5));
                });
            }
            None => {
                let mut tried = self.tried.lock().unwrap();
                if tried.elapsed() >= RETRY {
                    *tried = Instant::now();
                    drop(tried);
                    if let Some((enc, _)) = live::open_ladder(&self.gpu, &self.spec) {
                        self.attach(enc);
                    }
                }
            }
        }
    }

    /// The stream's description and the packets of `from..to`.
    pub fn stretch(&self, from: i64, to: i64) -> Option<(IMFMediaType, Vec<Arc<Stored>>)> {
        let s = self.store.lock().unwrap();
        let list = s.stretch(from, to);
        if list.is_empty() {
            return None;
        }
        Some((s.vtype.clone()?, list))
    }

    pub fn finish(&self) {
        if let Some(e) = self.enc.lock().unwrap().take() {
            e.finish(Duration::from_secs(3));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(t: i64, key: bool, n: usize) -> Arc<Stored> {
        Arc::new(Stored { p: Packet { data: vec![0; n], t, dur: HNS / 10, key }, generation: 1 })
    }

    fn filled(budget: usize, keep: i64, secs: i64) -> Store {
        let mut s = Store { generation: 1, ..Default::default() };
        // 10 frames a second, a keyframe every second
        for i in 0..secs * 10 {
            s.push(packet(i * HNS / 10, i % 10 == 0, 1000), keep, budget);
        }
        s
    }

    /// Old packets go by whole groups: it always starts on a keyframe, and
    /// holds the length asked for.
    #[test]
    fn keeps_the_length() {
        let s = filled(usize::MAX, 10 * HNS, 60);
        assert!(s.list.front().unwrap().p.key);
        let span = s.list.back().unwrap().p.t - s.list.front().unwrap().p.t;
        assert!(span >= 10 * HNS && span <= 10 * HNS + MARGIN + HNS, "{span}");
        assert_eq!(s.bytes, s.list.len() * 1000);
    }

    /// Past its memory budget it holds less, never more.
    #[test]
    fn stays_in_budget() {
        let s = filled(50_000, 1000 * HNS, 60);
        assert!(s.bytes <= 50_000, "{}", s.bytes);
        assert!(s.list.front().unwrap().p.key);
    }

    /// A stretch starts at the first keyframe in it, and only the current
    /// encoder's packets go in.
    #[test]
    fn stretch_starts_on_a_keyframe() {
        let mut s = filled(usize::MAX, 100 * HNS, 20);
        let got = s.stretch(5 * HNS + HNS / 2, 8 * HNS);
        assert!(got[0].p.key);
        assert_eq!(got[0].p.t, 6 * HNS);
        assert!(got.last().unwrap().p.t < 8 * HNS);
        s.generation = 2;
        assert!(s.stretch(0, 100 * HNS).is_empty());
    }
}

//! Screen frames from ScreenCaptureKit. A monitor area (cropped by the
//! system) or a single window is captured into a "latest frame" slot; a pacer
//! thread turns that into a steady stream for the encoder: fresh frames as
//! they come, repeats while click / key effects animate, and a 1 fps
//! heartbeat on a still screen. Frames stay in IOSurfaces all the way to the
//! hardware encoder.

use super::encoder::HNS;
use super::fx::Fx;
use super::{Clock, VideoOut};
use crate::media::Error;
use crate::win::{self, IRect, Monitor, ffi};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// A retained BGRA `CVPixelBuffer`.
pub struct PixelBuffer(*mut c_void);

unsafe impl Send for PixelBuffer {}
unsafe impl Sync for PixelBuffer {}

impl PixelBuffer {
    /// Takes a +0 reference (retains it).
    fn retain(p: *mut c_void) -> Self {
        unsafe { ffi::bk_cf_retain(p) };
        PixelBuffer(p)
    }

    pub fn as_ptr(&self) -> *mut c_void {
        self.0
    }

    /// A new buffer with the same pixels (to draw on).
    pub fn duplicate(&self) -> Option<PixelBuffer> {
        let p = unsafe { ffi::bk_pixbuf_copy(self.0) };
        (!p.is_null()).then_some(PixelBuffer(p))
    }

    /// Runs `f(pixels, stride, width, height)` with the BGRA memory.
    pub fn with_pixels<R>(&self, write: bool, f: impl FnOnce(&mut [u8], usize, u32, u32) -> R) -> Option<R> {
        let (mut stride, mut w, mut h) = (0usize, 0, 0);
        let base = unsafe { ffi::bk_pixbuf_lock(self.0, (!write) as i32, &mut stride, &mut w, &mut h) };
        if base.is_null() {
            return None;
        }
        let data = unsafe { std::slice::from_raw_parts_mut(base, stride * h.max(0) as usize) };
        let r = f(data, stride, w.max(0) as u32, h.max(0) as u32);
        unsafe { ffi::bk_pixbuf_unlock(self.0, (!write) as i32) };
        Some(r)
    }

    pub fn to_rgba(&self) -> Option<image::RgbaImage> {
        self.with_pixels(false, |data, stride, w, h| {
            let mut img = image::RgbaImage::new(w, h);
            for y in 0..h as usize {
                let row = &data[y * stride..y * stride + w as usize * 4];
                let out = &mut img.as_mut()[y * w as usize * 4..(y + 1) * w as usize * 4];
                for (o, p) in out.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                    o.copy_from_slice(&[p[2], p[1], p[0], 255]);
                }
            }
            img
        })
    }
}

impl Clone for PixelBuffer {
    fn clone(&self) -> Self {
        PixelBuffer::retain(self.0)
    }
}

impl Drop for PixelBuffer {
    fn drop(&mut self) {
        unsafe { ffi::bk_cf_release(self.0) };
    }
}

/// One captured frame as it travels from capture to the encoder.
pub type Frame = PixelBuffer;

/// Name of the capture API (for error messages).
pub const GPU_API: &str = "ScreenCaptureKit";

/// Nothing to share on macOS: the system hands out GPU-backed frames.
pub struct Gpu {
    pub device: (),
}

impl Gpu {
    pub fn new() -> Result<Self, String> {
        Ok(Gpu { device: () })
    }
}

/// Host clock in 100 ns units — the clock ScreenCaptureKit stamps samples with.
pub fn qpc_now() -> i64 {
    unsafe { ffi::bk_host_time_hns() }
}

/// What to capture.
#[derive(Clone, Copy)]
pub enum Target {
    /// `crop` = (x, y, w, h) px relative to the monitor's top-left; even size.
    Monitor { display: u32, scale: f64, crop: (u32, u32, u32, u32) },
    /// Follows the window wherever it is, even when covered.
    Window { id: u32, size: (u32, u32) },
}

impl Target {
    pub fn size(&self) -> (u32, u32) {
        match self {
            Target::Monitor { crop, .. } => (crop.2, crop.3),
            Target::Window { size, .. } => *size,
        }
    }
}

/// Capture target for one window (`CGWindowID`). Returns the target and the
/// window's bounds (even size).
pub fn window_target(h: isize) -> (Target, IRect) {
    let id = h as u32;
    let b = win::window_bounds(id).unwrap_or_default();
    let size = ((b.w as u32) & !1, (b.h as u32) & !1);
    (Target::Window { id, size }, IRect::new(b.x, b.y, size.0 as i32, size.1 as i32))
}

/// Capture target for `r` (absolute px, even size) on `monitor`.
pub fn monitor_target(r: IRect, monitor: &Monitor) -> Target {
    let d = win::display_at(r.x + r.w / 2, r.y + r.h / 2);
    let (display, scale) = d.map(|d| (d.id, d.scale)).unwrap_or((0, monitor.scale as f64));
    let crop = ((r.x - monitor.rect.x).max(0) as u32, (r.y - monitor.rect.y).max(0) as u32, r.w as u32, r.h as u32);
    Target::Monitor { display, scale, crop }
}

/// The newest captured frame.
struct Latest {
    tex: Option<Frame>,
    t: i64,
    fresh: bool,
}

struct Slot {
    latest: Arc<Mutex<Latest>>,
    clock: Arc<Clock>,
}

extern "C" fn on_frame(ctx: *mut c_void, pixbuf: *mut c_void, t: i64) {
    let slot = unsafe { &*(ctx as *const Slot) };
    let Some(t) = slot.clock.map(t) else { return };
    if t < 0 {
        return;
    }
    let frame = PixelBuffer::retain(pixbuf);
    let mut l = slot.latest.lock().unwrap();
    l.tex = Some(frame);
    l.t = t;
    l.fresh = true;
}

extern "C" fn no_audio(_: *mut c_void, _: *const i16, _: i32, _: i64) {}

pub struct Capture {
    stream: Mutex<*mut c_void>,
    slot: *mut Slot,
    latest: Arc<Mutex<Latest>>,
}

unsafe impl Send for Capture {}
unsafe impl Sync for Capture {}

impl Capture {
    pub fn start(_gpu: &Arc<Gpu>, target: Target, fps: u32, cursor: bool, clock: Arc<Clock>) -> Result<Self, Error> {
        let mut cfg = ffi::StreamCfg { fps: fps.clamp(1, 240) as i32, cursor: cursor as i32, kind: 0, ..Default::default() };
        match target {
            Target::Monitor { display, scale, crop } => {
                cfg.display = display;
                cfg.sx = crop.0 as f64 / scale;
                cfg.sy = crop.1 as f64 / scale;
                cfg.sw = crop.2 as f64 / scale;
                cfg.sh = crop.3 as f64 / scale;
                cfg.width = crop.2 as i32;
                cfg.height = crop.3 as i32;
            }
            Target::Window { id, size } => {
                cfg.window = id;
                cfg.width = size.0 as i32;
                cfg.height = size.1 as i32;
            }
        }
        let latest = Arc::new(Mutex::new(Latest { tex: None, t: 0, fresh: false }));
        let slot = Box::into_raw(Box::new(Slot { latest: latest.clone(), clock }));
        let mut err = ffi::ErrBuf::new();
        let stream = unsafe { ffi::bk_stream_start(&cfg, on_frame, no_audio, slot as *mut c_void, err.ptr(), ffi::ErrBuf::LEN) };
        if stream.is_null() {
            drop(unsafe { Box::from_raw(slot) });
            crate::capture::no_permission_notice();
            return Err(Error::new(err.text()));
        }
        Ok(Self { stream: Mutex::new(stream), slot, latest })
    }

    pub fn close(&self) {
        let s = std::mem::replace(&mut *self.stream.lock().unwrap(), std::ptr::null_mut());
        unsafe { ffi::bk_stream_stop(s) };
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.close();
        drop(unsafe { Box::from_raw(self.slot) });
    }
}

pub struct Pacer {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<Option<image::RgbaImage>>,
}

impl Pacer {
    /// Feeds `out` from `capture` at up to `fps`, with click / key effects
    /// from `fx` drawn on top. `origin`: screen position of the frame's top-left
    /// (for effects); a window target reads it live.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        _gpu: Arc<Gpu>,
        capture: &Capture,
        target: Target,
        out: VideoOut,
        fps: u32,
        clock: Arc<Clock>,
        fx: Option<Arc<Fx>>,
        origin: (i32, i32),
    ) -> Pacer {
        let stop = Arc::new(AtomicBool::new(false));
        let (st, latest) = (stop.clone(), capture.latest.clone());
        let thread = std::thread::spawn(move || pace(&latest, target, &out, fps, &clock, fx.as_deref(), origin, &st));
        Pacer { stop, thread }
    }

    /// Writes the closing frame and returns the last frame as RGBA.
    pub fn stop(self) -> Option<image::RgbaImage> {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.join().ok().flatten()
    }
}

#[allow(clippy::too_many_arguments)]
fn pace(
    latest: &Mutex<Latest>,
    target: Target,
    out: &VideoOut,
    fps: u32,
    clock: &Clock,
    fx: Option<&Fx>,
    origin: (i32, i32),
    stop: &AtomicBool,
) -> Option<image::RgbaImage> {
    let (w, h) = target.size();
    let interval = HNS / fps.max(1) as i64;
    let tick = Duration::from_nanos((interval * 100) as u64);
    let mut base: Option<Frame> = None; // newest captured frame
    let mut last_t = -1i64;
    let mut next = Instant::now();
    loop {
        let stopping = stop.load(Ordering::SeqCst);
        if clock.is_paused() && !stopping {
            std::thread::sleep(Duration::from_millis(20));
            next = Instant::now();
            continue;
        }
        let (fresh, t_cap) = {
            let mut l = latest.lock().unwrap();
            let f = l.fresh;
            if f {
                base = l.tex.clone();
                l.fresh = false;
            }
            (f, l.t)
        };
        let now = clock.now();
        let animating = fx.is_some_and(|f| f.active());
        let heartbeat = now - last_t >= HNS;
        if let Some(b) = &base
            && (fresh || animating || heartbeat || stopping)
        {
            let mut t = if fresh && !animating { t_cap } else { now };
            if t <= last_t {
                t = last_t + 1;
            }
            let frame = match fx {
                Some(f) if animating => {
                    let o = match target {
                        Target::Window { id, .. } => win::window_bounds(id).map(|r| (r.x, r.y)).unwrap_or(origin),
                        _ => origin,
                    };
                    b.duplicate().inspect(|c| f.draw(c, (w, h), o))
                }
                _ => None,
            };
            out.write_video(frame.as_ref().unwrap_or(b), t, interval);
            last_t = t;
        }
        if stopping {
            return base.and_then(|b| b.to_rgba());
        }
        out.maintain(clock.now(), base.as_ref());
        next += tick;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
    }
}

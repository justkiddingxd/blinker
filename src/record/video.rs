//! Screen frames from Windows Graphics Capture. A monitor (cropped on the GPU)
//! or a single window is captured into a "latest frame" slot; a pacer thread
//! turns that into a steady stream for the encoder: fresh frames as they
//! come, repeats while click / key effects animate, and a 1 fps heartbeat on
//! a still screen. Frames never pass through system memory.

use super::encoder::HNS;
use super::fx::Fx;
use super::{Clock, VideoOut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{Interface, Result};

pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
}

unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

impl Gpu {
    pub fn new() -> Result<Self> {
        unsafe {
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
            let device = device.unwrap();
            // Capture callbacks, the pacer and the encoder share the device.
            if let Ok(mt) = device.cast::<ID3D11Multithread>() {
                let _ = mt.SetMultithreadProtected(true);
            }
            Ok(Self { device, context: context.unwrap() })
        }
    }

    pub fn texture(&self, w: u32, h: u32, staging: bool) -> Result<ID3D11Texture2D> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: if staging { D3D11_USAGE_STAGING } else { D3D11_USAGE_DEFAULT },
            BindFlags: if staging { 0 } else { (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32 },
            CPUAccessFlags: if staging { D3D11_CPU_ACCESS_READ.0 as u32 } else { 0 },
            MiscFlags: 0,
        };
        let mut t = None;
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut t))? };
        Ok(t.unwrap())
    }

    pub fn copy(&self, dst: &ID3D11Texture2D, src: &ID3D11Texture2D) {
        unsafe { self.context.CopyResource(dst, src) };
    }

    /// Reads a texture back as RGBA (for the thumbnail).
    pub fn read_rgba(&self, tex: &ID3D11Texture2D, w: u32, h: u32) -> Option<image::RgbaImage> {
        let staging = self.texture(w, h, true).ok()?;
        unsafe {
            self.context.CopyResource(&staging, tex);
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m)).ok()?;
            let mut img = image::RgbaImage::new(w, h);
            for y in 0..h {
                let row = std::slice::from_raw_parts((m.pData as *const u8).add((y * m.RowPitch) as usize), (w * 4) as usize);
                for x in 0..w as usize {
                    let p = &row[x * 4..x * 4 + 4];
                    img.put_pixel(x as u32, y, image::Rgba([p[2], p[1], p[0], 255]));
                }
            }
            self.context.Unmap(&staging, 0);
            Some(img)
        }
    }
}

/// QPC time in 100 ns units — the clock WGC and WASAPI timestamps use.
pub fn qpc_now() -> i64 {
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    let (mut c, mut f) = (0i64, 0i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut c);
        let _ = QueryPerformanceFrequency(&mut f);
    }
    (c as i128 * HNS as i128 / f.max(1) as i128) as i64
}

/// What to capture.
#[derive(Clone, Copy)]
pub enum Target {
    /// `crop` = (x, y, w, h) relative to the monitor's top-left; even size.
    Monitor { hmon: HMONITOR, crop: (u32, u32, u32, u32) },
    /// Follows the window wherever it is, even when covered.
    Window { hwnd: HWND, size: (u32, u32) },
}

unsafe impl Send for Target {}
unsafe impl Sync for Target {}

impl Target {
    pub fn size(&self) -> (u32, u32) {
        match self {
            Target::Monitor { crop, .. } => (crop.2, crop.3),
            Target::Window { size, .. } => *size,
        }
    }
}

/// WinRT device handle used from the capture callback thread.
struct SendDevice(IDirect3DDevice);
unsafe impl Send for SendDevice {}
unsafe impl Sync for SendDevice {}

impl SendDevice {
    fn get(&self) -> &IDirect3DDevice {
        &self.0
    }
}

/// The newest captured frame (already cropped to the output size).
struct Latest {
    tex: Option<ID3D11Texture2D>,
    t: i64,
    fresh: bool,
}

unsafe impl Send for Latest {}

pub struct Capture {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    latest: Arc<Mutex<Latest>>,
}

unsafe impl Send for Capture {}

impl Capture {
    pub fn start(gpu: &Arc<Gpu>, target: Target, fps: u32, cursor: bool, clock: Arc<Clock>) -> Result<Self> {
        unsafe {
            let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            let item: GraphicsCaptureItem = match target {
                Target::Monitor { hmon, .. } => interop.CreateForMonitor(hmon)?,
                Target::Window { hwnd, .. } => interop.CreateForWindow(hwnd)?,
            };
            let dxgi: IDXGIDevice = gpu.device.cast()?;
            let d3d: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
            let size = item.Size()?;
            let fmt = DirectXPixelFormat::B8G8R8A8UIntNormalized;
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&d3d, fmt, 2, size)?;
            let session = pool.CreateCaptureSession(&item)?;
            let _ = session.SetIsBorderRequired(false);
            let _ = session.SetIsCursorCaptureEnabled(cursor);
            let interval = HNS / fps.max(1) as i64;
            let _ = session.SetMinUpdateInterval(windows::Foundation::TimeSpan { Duration: interval });

            let (ox, oy, ow, oh) = match target {
                Target::Monitor { crop, .. } => crop,
                Target::Window { size, .. } => (0, 0, size.0, size.1),
            };
            let latest = Arc::new(Mutex::new(Latest { tex: None, t: 0, fresh: false }));
            let slot = latest.clone();
            let g = gpu.clone();
            let pool_size = Mutex::new(size);
            let device = SendDevice(d3d.clone());
            pool.FrameArrived(&windows::Foundation::TypedEventHandler::new(
                move |pool: windows::core::Ref<Direct3D11CaptureFramePool>, _| {
                    let Some(pool) = pool.as_ref() else { return Ok(()) };
                    let frame = pool.TryGetNextFrame()?;
                    let content = frame.ContentSize()?;
                    // A resized window: grow the pool so frames aren't clipped.
                    {
                        let mut ps = pool_size.lock().unwrap();
                        if content.Width != ps.Width || content.Height != ps.Height {
                            *ps = content;
                            let _ = pool.Recreate(device.get(), fmt, 2, content);
                        }
                    }
                    let Some(t) = clock.map(frame.SystemRelativeTime()?.Duration) else { return Ok(()) };
                    if t < 0 {
                        return Ok(());
                    }
                    let src: ID3D11Texture2D = frame.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()?;
                    let dst = g.texture(ow, oh, false)?;
                    let b = D3D11_BOX {
                        left: ox,
                        top: oy,
                        front: 0,
                        right: (ox + ow).min(content.Width.max(0) as u32),
                        bottom: (oy + oh).min(content.Height.max(0) as u32),
                        back: 1,
                    };
                    if b.right > b.left && b.bottom > b.top {
                        g.context.CopySubresourceRegion(&dst, 0, 0, 0, 0, &src, 0, Some(&b));
                    }
                    drop(frame);
                    let mut l = slot.lock().unwrap();
                    l.tex = Some(dst);
                    l.t = t;
                    l.fresh = true;
                    Ok(())
                },
            ))?;
            session.StartCapture()?;
            Ok(Self { pool, session, latest })
        }
    }

    pub fn close(&self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.close();
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
    pub fn start(
        gpu: Arc<Gpu>,
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
        let thread = std::thread::spawn(move || {
            crate::media::startup();
            pace(&gpu, &latest, target, &out, fps, &clock, fx.as_deref(), origin, &st)
        });
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
    gpu: &Gpu,
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
    let mut base: Option<ID3D11Texture2D> = None; // newest captured frame
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
                        Target::Window { hwnd, .. } => {
                            let r = crate::win::frame_bounds(hwnd.0 as _);
                            (r.x, r.y)
                        }
                        _ => origin,
                    };
                    gpu.texture(w, h, false).ok().inspect(|c| {
                        gpu.copy(c, b);
                        f.draw(c, (w, h), o);
                    })
                }
                _ => None,
            };
            out.write_video(frame.as_ref().unwrap_or(b), t, interval);
            last_t = t;
        }
        if stopping {
            return base.and_then(|b| gpu.read_rgba(&b, w, h));
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

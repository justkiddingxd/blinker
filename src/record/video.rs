//! Screen frames: a monitor through Desktop Duplication (cropped on the GPU),
//! a single window through Windows Graphics Capture (and a monitor too when
//! duplication isn't to be had). Either fills a "latest frame" slot; a pacer thread
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
use windows::Win32::Graphics::Dxgi::Common::{DXGI_MODE_ROTATION_IDENTITY, DXGI_MODE_ROTATION_UNSPECIFIED};
use windows::Win32::Graphics::Dxgi::{DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, IDXGIDevice, IDXGIKeyedMutex, IDXGIOutput1, IDXGIResource};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{Interface, Result};

/// One captured frame as it travels from capture to the encoder.
pub type Frame = ID3D11Texture2D;

/// Name of the graphics API (for error messages).
pub const GPU_API: &str = "Direct3D";

/// Capture target for one window: it is followed wherever it goes. Returns
/// the target and the window's bounds (even size).
pub fn window_target(h: isize) -> (Target, crate::win::IRect) {
    let hwnd = HWND(h as _);
    let b = crate::win::frame_bounds(h as _);
    let size = ((b.w as u32) & !1, (b.h as u32) & !1);
    (Target::Window { hwnd, size }, crate::win::IRect::new(b.x, b.y, size.0 as i32, size.1 as i32))
}

/// Capture target for `r` (absolute px, even size) on `monitor`.
pub fn monitor_target(r: crate::win::IRect, monitor: &crate::win::Monitor) -> Target {
    let crop = ((r.x - monitor.rect.x) as u32, (r.y - monitor.rect.y) as u32, r.w as u32, r.h as u32);
    Target::Monitor { hmon: super::hmonitor_at(r.x + r.w / 2, r.y + r.h / 2), crop }
}

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

/// The newest captured frame (already cropped to the output size): a
/// texture on the taker's own device, or, from a monitor's feed (on a device
/// of its own), the shared texture it waits in.
struct Latest {
    tex: Option<ID3D11Texture2D>,
    shared: Option<(ID3D11Texture2D, IDXGIKeyedMutex)>,
    t: i64,
    fresh: bool,
}

/// A keyed mutex with a timeout (its wrapper takes the timeout for success).
fn lock_shared(m: &IDXGIKeyedMutex, ms: u32) -> bool {
    unsafe { (Interface::vtable(m).AcquireSync)(Interface::as_raw(m), 0, ms) }.0 == 0
}

impl Latest {
    /// The frame, on `gpu`; None if the shared one is busy this moment.
    fn take(&self, gpu: &Gpu, w: u32, h: u32) -> Option<ID3D11Texture2D> {
        let Some((tex, mutex)) = &self.shared else { return self.tex.clone() };
        if !lock_shared(mutex, 4) {
            return None;
        }
        let dst = gpu.texture(w, h, false).ok();
        if let Some(d) = &dst {
            gpu.copy(d, tex);
        }
        unsafe {
            let _ = mutex.ReleaseSync(0);
        }
        dst
    }
}

unsafe impl Send for Latest {}

pub struct Capture {
    how: How,
    latest: Arc<Mutex<Latest>>,
    /// The pointer goes into the frames (drawn by the pacer, `pointer.rs`).
    cursor: bool,
}

unsafe impl Send for Capture {}

/// Where the frames come from.
enum How {
    /// Windows Graphics Capture. Any session of it hides the pointer in
    /// some fullscreen games (Minecraft): only for windows, and for a
    /// monitor duplication can't take.
    Wgc { pool: Direct3D11CaptureFramePool, session: GraphicsCaptureSession },
    /// The monitor's Desktop Duplication feed, shared with whatever else
    /// captures that monitor (the replay and a recording): `id` in its list.
    Duplication { feed: Arc<Feed>, id: u64 },
}

/// COM objects of the duplication thread (made on one thread, used on it).
struct SendCom<T>(T);
unsafe impl<T> Send for SendCom<T> {}

/// Someone taking frames from a feed: their piece of the monitor, the shared
/// texture it goes to, their clock and slot.
struct Sub {
    id: u64,
    crop: (u32, u32, u32, u32),
    tex: ID3D11Texture2D,
    mutex: IDXGIKeyedMutex,
    clock: Arc<Clock>,
    slot: Arc<Mutex<Latest>>,
}

/// One monitor through Desktop Duplication, on a device of its own. Windows
/// duplicates a monitor once per process, so the replay and a recording share
/// the feed; each gets its piece through a shared texture on its own device.
/// No encoder ever sits on this device, so nothing that happens in a driver's
/// encoder can hold the capture up, or the other way round.
pub struct Feed {
    hmon: isize,
    /// The monitor on the screen (px).
    rect: crate::win::IRect,
    gpu: Arc<Gpu>,
    subs: Mutex<Vec<Sub>>,
    /// A recent picture of the whole monitor (a newcomer starts from it: a
    /// still screen gives no new ones).
    last: Mutex<Option<ID3D11Texture2D>>,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

unsafe impl Send for Feed {}
unsafe impl Sync for Feed {}

/// The feeds running now, by monitor.
static FEEDS: Mutex<Vec<std::sync::Weak<Feed>>> = Mutex::new(Vec::new());

impl Feed {
    /// The feed of `hmon`: the running one, or a new one.
    fn of(hmon: HMONITOR) -> Result<Arc<Feed>> {
        let mut feeds = FEEDS.lock().unwrap();
        feeds.retain(|f| f.strong_count() > 0);
        if let Some(f) = feeds.iter().filter_map(|f| f.upgrade()).find(|f| f.hmon == hmon.0 as isize) {
            return Ok(f);
        }
        let gpu = Arc::new(Gpu::new()?);
        let output = unsafe { output_of(&gpu, hmon)? };
        let d = unsafe { output.GetDesc()? }.DesktopCoordinates;
        let rect = crate::win::IRect::new(d.left, d.top, d.right - d.left, d.bottom - d.top);
        // the feed before this one may still be letting go of the monitor
        let mut tries = 0;
        let dup = loop {
            match unsafe { output.DuplicateOutput(&gpu.device) } {
                Ok(d) => break d,
                Err(e) if tries >= 4 => return Err(e),
                Err(_) => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(60));
                }
            }
        };
        let feed = Arc::new(Feed {
            hmon: hmon.0 as isize,
            rect,
            gpu,
            subs: Mutex::new(Vec::new()),
            last: Mutex::new(None),
            stop: Arc::new(AtomicBool::new(false)),
            thread: Mutex::new(None),
        });
        let weak = Arc::downgrade(&feed);
        let stop = feed.stop.clone();
        let (output, dup) = (SendCom(output), SendCom(dup));
        let thread = std::thread::spawn(move || {
            let (output, dup) = (output, dup);
            feed_loop(&weak, &stop, output.0, dup.0);
        });
        *feed.thread.lock().unwrap() = Some(thread);
        feeds.push(Arc::downgrade(&feed));
        Ok(feed)
    }

    /// Frames of `crop` into `slot` from now on, as textures `gpu` can read,
    /// on `clock`; the recent picture at once. The id to leave with.
    fn join(&self, gpu: &Gpu, crop: (u32, u32, u32, u32), clock: Arc<Clock>, slot: Arc<Mutex<Latest>>) -> Result<u64> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let desc = D3D11_TEXTURE2D_DESC {
            Width: crop.2,
            Height: crop.3,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0 as u32,
        };
        let (tex, mine, theirs) = unsafe {
            let mut t = None;
            self.gpu.device.CreateTexture2D(&desc, None, Some(&mut t))?;
            let tex = t.unwrap();
            let handle = tex.cast::<IDXGIResource>()?.GetSharedHandle()?;
            let mut opened: Option<ID3D11Texture2D> = None;
            gpu.device.OpenSharedResource(handle, &mut opened)?;
            let opened = opened.unwrap();
            let (mine, theirs) = (tex.cast::<IDXGIKeyedMutex>()?, opened.cast::<IDXGIKeyedMutex>()?);
            slot.lock().unwrap().shared = Some((opened, theirs.clone()));
            (tex, mine, theirs)
        };
        drop(theirs);
        let id = NEXT.fetch_add(1, Ordering::SeqCst);
        let sub = Sub { id, crop, tex, mutex: mine, clock, slot };
        match self.last.lock().unwrap().as_ref() {
            Some(last) => self.deliver(&sub, last, qpc_now()),
            None => self.first_picture(&sub),
        }
        self.subs.lock().unwrap().push(sub);
        Ok(id)
    }

    /// No picture from the duplication yet: a still screen may not give one
    /// for a long time (and its very first is empty). What's on the screen
    /// now, by the plain way, until it does.
    fn first_picture(&self, sub: &Sub) {
        let (ox, oy, ow, oh) = sub.crop;
        let r = crate::win::IRect::new(self.rect.x + ox as i32, self.rect.y + oy as i32, ow as i32, oh as i32);
        let Some(img) = crate::capture::grab_rect(r) else { return };
        let Some(t) = sub.clock.map(qpc_now()).filter(|t| *t >= 0) else { return };
        if !lock_shared(&sub.mutex, 50) {
            return;
        }
        let bgra: Vec<u8> = img.as_raw().chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], 255]).collect();
        unsafe {
            self.gpu.context.UpdateSubresource(&sub.tex, 0, None, bgra.as_ptr() as _, ow * 4, 0);
            let _ = sub.mutex.ReleaseSync(0);
        }
        let mut l = sub.slot.lock().unwrap();
        l.t = t;
        l.fresh = true;
    }

    fn leave(&self, id: u64) {
        self.subs.lock().unwrap().retain(|s| s.id != id);
    }

    /// `sub`'s piece of the picture `full`, taken at `at` (QPC, 100 ns). If
    /// the taker has the shared texture this moment, it skips a frame.
    fn deliver(&self, sub: &Sub, full: &ID3D11Texture2D, at: i64) {
        let Some(t) = sub.clock.map(at).filter(|t| *t >= 0) else { return };
        let (ox, oy, ow, oh) = sub.crop;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { full.GetDesc(&mut desc) };
        let b = D3D11_BOX { left: ox, top: oy, front: 0, right: (ox + ow).min(desc.Width), bottom: (oy + oh).min(desc.Height), back: 1 };
        if b.right <= b.left || b.bottom <= b.top || !lock_shared(&sub.mutex, 2) {
            return;
        }
        unsafe {
            self.gpu.context.CopySubresourceRegion(&sub.tex, 0, 0, 0, 0, full, 0, Some(&b));
            let _ = sub.mutex.ReleaseSync(0);
        }
        let mut l = sub.slot.lock().unwrap();
        l.t = t;
        l.fresh = true;
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.get_mut().unwrap().take()
            && t.thread().id() != std::thread::current().id()
        {
            let _ = t.join();
        }
    }
}

/// The DXGI output showing `hmon`, among those of `gpu`'s adapter.
unsafe fn output_of(gpu: &Gpu, hmon: HMONITOR) -> Result<IDXGIOutput1> {
    unsafe {
        let adapter = gpu.device.cast::<IDXGIDevice>()?.GetAdapter()?;
        let mut i = 0;
        loop {
            let o = adapter.EnumOutputs(i)?;
            let desc = o.GetDesc()?;
            if desc.Monitor == hmon {
                if !matches!(desc.Rotation, DXGI_MODE_ROTATION_IDENTITY | DXGI_MODE_ROTATION_UNSPECIFIED) {
                    return Err(windows::core::Error::new(windows::core::HRESULT(0x887A0004u32 as i32), "a rotated monitor"));
                }
                return o.cast();
            }
            i += 1;
        }
    }
}

/// The duplication thread: a new picture whenever the monitor changes,
/// handed to everyone on the feed; till `stop` (or the feed is gone).
fn feed_loop(feed: &std::sync::Weak<Feed>, stop: &AtomicBool, output: IDXGIOutput1, dup: windows::Win32::Graphics::Dxgi::IDXGIOutputDuplication) {
    let mut qpf = 0i64;
    unsafe {
        let _ = windows::Win32::System::Performance::QueryPerformanceFrequency(&mut qpf);
    }
    let mut dup = Some(dup);
    let mut kept = Instant::now() - Duration::from_secs(1);
    while !stop.load(Ordering::SeqCst) {
        let Some(f) = feed.upgrade() else { return };
        // lost (another resolution, the secure desktop, a game taking the
        // screen): made again once it can be
        let Some(d) = &dup else {
            drop(f);
            std::thread::sleep(Duration::from_millis(200));
            let Some(f) = feed.upgrade() else { return };
            dup = unsafe { output.DuplicateOutput(&f.gpu.device) }.ok();
            continue;
        };
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut res = None;
        match unsafe { d.AcquireNextFrame(50, &mut info, &mut res) } {
            Ok(()) => {
                // no present time: only the pointer moved, nothing new in the
                // picture (the first of a duplication can be empty, too)
                let at = (info.LastPresentTime != 0).then(|| (info.LastPresentTime as i128 * HNS as i128 / qpf.max(1) as i128) as i64);
                if let Some(at) = at
                    && let Some(src) = res.and_then(|r| r.cast::<ID3D11Texture2D>().ok())
                {
                    for sub in f.subs.lock().unwrap().iter() {
                        f.deliver(sub, &src, at);
                    }
                    // the picture for newcomers: twice a second is plenty
                    if kept.elapsed() > Duration::from_millis(500) {
                        kept = Instant::now();
                        let mut desc = D3D11_TEXTURE2D_DESC::default();
                        unsafe { src.GetDesc(&mut desc) };
                        let mut last = f.last.lock().unwrap();
                        let fits = last.as_ref().is_some_and(|l| {
                            let mut have = D3D11_TEXTURE2D_DESC::default();
                            unsafe { l.GetDesc(&mut have) };
                            (have.Width, have.Height) == (desc.Width, desc.Height)
                        });
                        if !fits {
                            *last = f.gpu.texture(desc.Width, desc.Height, false).ok();
                        }
                        if let Some(l) = last.as_ref() {
                            f.gpu.copy(l, &src);
                        }
                    }
                }
                let _ = unsafe { d.ReleaseFrame() };
            }
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {}
            Err(_) => dup = None,
        }
    }
}

impl Capture {
    pub fn start(gpu: &Arc<Gpu>, target: Target, fps: u32, cursor: bool, clock: Arc<Clock>) -> Result<Self> {
        let latest = Arc::new(Mutex::new(Latest { tex: None, shared: None, t: 0, fresh: false }));
        if let Target::Monitor { hmon, crop } = target {
            match Feed::of(hmon).and_then(|feed| feed.join(gpu, crop, clock.clone(), latest.clone()).map(|id| (feed, id))) {
                Ok((feed, id)) => return Ok(Self { how: How::Duplication { feed, id }, latest, cursor }),
                Err(e) => crate::config::log::info(format!("screen capture: no desktop duplication ({e}), Windows Graphics Capture instead")),
            }
        }
        Self::wgc(gpu, target, fps, cursor, clock, latest)
    }

    fn wgc(gpu: &Arc<Gpu>, target: Target, fps: u32, cursor: bool, clock: Arc<Clock>, latest: Arc<Mutex<Latest>>) -> Result<Self> {
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
            // never Windows' own pointer capture: it makes the pointer a
            // software one, which some fullscreen games (Minecraft) lose;
            // the pacer draws it instead
            let _ = session.SetIsCursorCaptureEnabled(false);
            let interval = HNS / fps.max(1) as i64;
            let _ = session.SetMinUpdateInterval(windows::Foundation::TimeSpan { Duration: interval });

            let (ox, oy, ow, oh) = match target {
                Target::Monitor { crop, .. } => crop,
                Target::Window { size, .. } => (0, 0, size.0, size.1),
            };
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
            Ok(Self { how: How::Wgc { pool, session }, latest, cursor })
        }
    }

    pub fn close(&self) {
        match &self.how {
            How::Wgc { pool, session } => {
                let _ = session.Close();
                let _ = pool.Close();
            }
            How::Duplication { feed, id } => feed.leave(*id),
        }
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
    /// from `fx` and the pointer drawn on top. `origin`: screen position of
    /// the frame's top-left (for both); a window target reads it live.
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
        let (st, latest, cursor) = (stop.clone(), capture.latest.clone(), capture.cursor);
        let thread = std::thread::spawn(move || {
            crate::media::startup();
            let pointer = if cursor { super::pointer::Pointer::new() } else { None };
            pace(&gpu, &latest, target, &out, fps, &clock, fx.as_deref(), pointer, origin, &st)
        });
        Pacer { stop, thread }
    }

    /// Writes the closing frame and returns the last frame as RGBA. A thread
    /// stuck on its device (in a driver) is left behind, not waited for:
    /// nobody who called this hangs with it.
    pub fn stop(self) -> Option<image::RgbaImage> {
        self.stop.store(true, Ordering::SeqCst);
        let t0 = Instant::now();
        while !self.thread.is_finished() {
            if t0.elapsed() > Duration::from_secs(5) {
                crate::config::log::error("recording: the frame thread is stuck, left behind");
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
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
    mut pointer: Option<super::pointer::Pointer>,
    origin: (i32, i32),
    stop: &AtomicBool,
) -> Option<image::RgbaImage> {
    let (w, h) = target.size();
    let interval = HNS / fps.max(1) as i64;
    let tick = Duration::from_nanos((interval * 100) as u64);
    let mut base: Option<ID3D11Texture2D> = None; // newest captured frame
    let mut last_t = -1i64;
    let mut next = Instant::now();
    // the pointer drawn last: a new frame when it moves or changes
    let mut drawn: Option<super::pointer::Seen> = None;
    loop {
        let stopping = stop.load(Ordering::SeqCst);
        if clock.is_paused() && !stopping {
            std::thread::sleep(Duration::from_millis(20));
            next = Instant::now();
            continue;
        }
        let (fresh, t_cap) = {
            let mut l = latest.lock().unwrap();
            let f = l.fresh && l.take(gpu, w, h).map(|t| base = Some(t)).is_some();
            if f {
                l.fresh = false;
            }
            (f, l.t)
        };
        let now = clock.now();
        let animating = fx.is_some_and(|f| f.active());
        let heartbeat = now - last_t >= HNS;
        let o = match target {
            Target::Window { hwnd, .. } if animating || pointer.is_some() => {
                let r = crate::win::frame_bounds(hwnd.0 as _);
                (r.x, r.y)
            }
            _ => origin,
        };
        let seen = pointer.as_mut().and_then(|p| super::pointer::Pointer::now().filter(|s| p.within(*s, (w, h), o)));
        let moved = seen != drawn;
        if let Some(b) = &base
            && (fresh || animating || heartbeat || stopping || moved)
        {
            let mut t = if fresh && !animating && !moved { t_cap } else { now };
            if t <= last_t {
                t = last_t + 1;
            }
            let frame = if animating || seen.is_some() {
                gpu.texture(w, h, false).ok().inspect(|c| {
                    gpu.copy(c, b);
                    if let Some(f) = fx.filter(|_| animating) {
                        f.draw(c, (w, h), o);
                    }
                    if let (Some(p), Some(s)) = (pointer.as_mut(), seen) {
                        p.draw(c, o, s);
                    }
                })
            } else {
                None
            };
            out.write_video(frame.as_ref().unwrap_or(b), t, interval);
            last_t = t;
            drawn = seen;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A monitor goes through Desktop Duplication (not WGC, which hides the
    /// pointer in some games), each taker on a device of its own, and frames
    /// come with something on them.
    #[test]
    fn monitor_by_duplication() {
        let m = crate::win::monitors()[0];
        let r = crate::win::IRect::new(m.rect.x, m.rect.y, m.rect.w & !1, m.rect.h & !1);
        let gpu = Arc::new(Gpu::new().unwrap());
        let cap = Capture::start(&gpu, monitor_target(r, &m), 30, true, super::super::Clock::new()).unwrap();
        assert!(matches!(cap.how, How::Duplication { .. }), "fell back to WGC");
        // a frame in hand from a taker's own device
        let frame = |c: &Capture, g: &Gpu, w: u32, h: u32| {
            let t0 = Instant::now();
            loop {
                {
                    let l = c.latest.lock().unwrap();
                    if l.fresh && let Some(t) = l.take(g, w, h) {
                        break t;
                    }
                }
                assert!(t0.elapsed() < Duration::from_secs(3), "no frame");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        let tex = frame(&cap, &gpu, r.w as u32, r.h as u32);
        let img = gpu.read_rgba(&tex, r.w as u32, r.h as u32).unwrap();
        assert!(img.pixels().any(|p| p[0] > 8 || p[1] > 8 || p[2] > 8), "all black");
        // a recording while the replay runs: the same monitor again, another
        // device, a piece of it
        std::thread::sleep(Duration::from_millis(600));
        let gpu2 = Arc::new(Gpu::new().unwrap());
        let small = crate::win::IRect::new(m.rect.x + 40, m.rect.y + 40, 320, 200);
        let second = Capture::start(&gpu2, monitor_target(small, &m), 30, true, super::super::Clock::new()).unwrap();
        assert!(matches!(second.how, How::Duplication { .. }), "the second one fell back to WGC");
        let piece = frame(&second, &gpu2, 320, 200);
        let img = gpu2.read_rgba(&piece, 320, 200).unwrap();
        assert_eq!(img.dimensions(), (320, 200));
        second.close();
        cap.close();
    }
}

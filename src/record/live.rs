//! The live video encoder: the graphics card's own encoder, driven directly.
//!
//! The sink writer picks an encoder by itself, and when the vendor's refuses
//! at the last moment it takes Microsoft's D3D12 one, which closes the whole
//! app with an assertion. Here the encoder is chosen by hand: the one of the
//! device's adapter, never Microsoft's. It stays open for as long as the
//! recording or the replay runs (every opening of NVIDIA's leaves handles
//! behind for good), and what it makes goes to `sink` as packets.
//!
//! Nothing here can hold up the frame pacer: frames wait in a short queue
//! that the encoder's own threads take from, and when it's full (the encoder
//! is behind or stuck) a new frame is dropped.

use super::encoder::{Codec, VideoSpec, set_size};
use super::video::Gpu;
use crate::config::log;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_UI4};
use windows::core::{Error, Interface, PWSTR, Result};

/// Frames waiting for the encoder, at most.
const QUEUE: usize = 4;
/// Frames in it without anything coming out: it's stuck or far behind.
const IN_FLIGHT: u64 = 16;
/// No packet for this long with frames in it: stuck.
const STALL: Duration = Duration::from_secs(10);

/// One piece of the compressed stream.
pub struct Packet {
    pub data: Vec<u8>,
    /// Time and duration, 100 ns.
    pub t: i64,
    pub dur: i64,
    /// A picture that can be started from.
    pub key: bool,
}

/// A hardware encoder for `codec` on the adapter of `gpu`: its activator and
/// name. Microsoft's D3D12 one doesn't count.
fn pick(gpu: &Gpu, codec: Codec) -> Result<(IMFActivate, String)> {
    unsafe {
        let vendor = gpu.device.cast::<IDXGIDevice>()?.GetAdapter()?.GetDesc()?.VendorId;
        let want = format!("VEN_{vendor:04X}");
        let info = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: codec.subtype() };
        let (mut list, mut n) = (std::ptr::null_mut(), 0u32);
        MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER, None, Some(&info), &mut list, &mut n)?;
        let all: Vec<Option<IMFActivate>> = (0..n as usize).map(|i| std::ptr::read(list.add(i))).collect();
        windows::Win32::System::Com::CoTaskMemFree(Some(list as _));
        let text = |a: &IMFActivate, key: &windows::core::GUID| {
            let (mut p, mut len) = (PWSTR::null(), 0);
            if a.GetAllocatedString(key, &mut p, &mut len).is_err() {
                return String::new();
            }
            let s = p.to_string().unwrap_or_default();
            windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _));
            s
        };
        for a in all.into_iter().flatten() {
            let (id, name) = (text(&a, &MFT_ENUM_HARDWARE_VENDOR_ID_Attribute), text(&a, &MFT_FRIENDLY_NAME_Attribute));
            if id.eq_ignore_ascii_case("VEN_1414") || name.contains("DX12") || name.contains("D3D12") {
                continue;
            }
            if id.is_empty() || id.eq_ignore_ascii_case(&want) {
                return Ok((a, name));
            }
        }
        Err(Error::new(MF_E_TOPO_CODEC_NOT_FOUND, format!("no hardware encoder for {want}")))
    }
}

fn u4(v: u32) -> VARIANT {
    let mut x = VARIANT::default();
    unsafe {
        let inner = &mut *x.Anonymous.Anonymous;
        inner.vt = VT_UI4;
        inner.Anonymous.ulVal = v;
    }
    x
}

fn yes() -> VARIANT {
    let mut x = VARIANT::default();
    unsafe {
        let inner = &mut *x.Anonymous.Anonymous;
        inner.vt = VT_BOOL;
        inner.Anonymous.boolVal = windows::Win32::Foundation::VARIANT_TRUE;
    }
    x
}

/// The colors of a video this size, by the rule players go by when a stream
/// doesn't say: BT.709 from 720p up, BT.601 below. The stream doesn't say: a
/// matrix on the media type reaches NVIDIA's bitstream with a wrong code.
fn is_hd(v: &VideoSpec) -> bool {
    v.width >= 1280 || v.height >= 720
}

/// BGRA frames into NV12 ones, on the GPU (video range, see `is_hd`).
struct Converter {
    video: ID3D11VideoDevice,
    context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
}

impl Converter {
    fn new(gpu: &Gpu, v: &VideoSpec) -> Result<Converter> {
        unsafe {
            let video: ID3D11VideoDevice = gpu.device.cast()?;
            let context: ID3D11VideoContext = gpu.context.cast()?;
            let rate = DXGI_RATIONAL { Numerator: v.fps, Denominator: 1 };
            let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: rate,
                InputWidth: v.width,
                InputHeight: v.height,
                OutputFrameRate: rate,
                OutputWidth: v.width,
                OutputHeight: v.height,
                Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
            };
            let enumerator = video.CreateVideoProcessorEnumerator(&desc)?;
            let processor = video.CreateVideoProcessor(&enumerator, 0)?;
            // the bits: RGB_Range 0 = full; YCbCr_Matrix (bit 2) 1 = BT.709,
            // 0 = BT.601; Nominal_Range (bits 4-5) 1 = 16-235
            context.VideoProcessorSetStreamColorSpace(&processor, 0, &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 });
            let m = if is_hd(v) { 1 << 2 } else { 0 };
            context.VideoProcessorSetOutputColorSpace(&processor, &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: m | (1 << 4) });
            context.VideoProcessorSetStreamFrameFormat(&processor, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
            context.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
            Ok(Converter { video, context, enumerator, processor })
        }
    }

    /// `src` (BGRA) as a new NV12 texture.
    fn convert(&self, gpu: &Gpu, src: &ID3D11Texture2D, w: u32, h: u32) -> Result<ID3D11Texture2D> {
        unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_NV12,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_VIDEO_ENCODER.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut dst = None;
            gpu.device.CreateTexture2D(&desc, None, Some(&mut dst))?;
            let dst = dst.unwrap();
            let mut iv = None;
            let idesc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: 0 } },
            };
            self.video.CreateVideoProcessorInputView(src, &self.enumerator, &idesc, Some(&mut iv))?;
            let mut ov = None;
            let odesc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 } },
            };
            self.video.CreateVideoProcessorOutputView(&dst, &self.enumerator, &odesc, Some(&mut ov))?;
            let stream = D3D11_VIDEO_PROCESSOR_STREAM { Enable: true.into(), pInputSurface: std::mem::ManuallyDrop::new(iv), ..Default::default() };
            let mut streams = [stream];
            let r = self.context.VideoProcessorBlt(&self.processor, ov.as_ref(), 0, &streams);
            std::mem::ManuallyDrop::drop(&mut streams[0].pInputSurface);
            r?;
            Ok(dst)
        }
    }
}

/// What the threads of an encoder share.
struct Shared {
    mft: IMFTransform,
    input: u32,
    output: u32,
    /// The encoder allocates its output samples itself; else their size.
    own_size: Option<u32>,
    queue: Mutex<Queue>,
    wake: Condvar,
    submitted: AtomicU64,
    came_out: AtomicU64,
    /// When the last packet came out.
    last_out: Mutex<Instant>,
    failed: AtomicBool,
    finished: Mutex<bool>,
    finished_cv: Condvar,
    dropped: AtomicU64,
}

unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

#[derive(Default)]
struct Queue {
    frames: VecDeque<IMFSample>,
    /// How many inputs the encoder has asked for and not got.
    need: u32,
    closing: bool,
}

pub struct LiveEncoder {
    shared: Arc<Shared>,
    activate: IMFActivate,
    converter: Converter,
    gpu: Arc<Gpu>,
    spec: VideoSpec,
    /// The stream as the encoder describes it (what the MP4 needs).
    out_type: IMFMediaType,
    pub name: String,
    started: AtomicBool,
}

unsafe impl Send for LiveEncoder {}
unsafe impl Sync for LiveEncoder {}

impl LiveEncoder {
    /// Opens the encoder of `gpu`'s adapter for `v`; an error if it won't take it.
    pub fn open(gpu: &Arc<Gpu>, v: &VideoSpec) -> Result<LiveEncoder> {
        crate::media::startup();
        let (activate, name) = pick(gpu, v.codec)?;
        let r = Self::configure(gpu, v, &activate, &name);
        if r.is_err() {
            unsafe {
                let _ = activate.ShutdownObject();
            }
        }
        r
    }

    fn configure(gpu: &Arc<Gpu>, v: &VideoSpec, activate: &IMFActivate, name: &str) -> Result<LiveEncoder> {
        unsafe {
            let mft: IMFTransform = activate.ActivateObject()?;
            let attrs = mft.GetAttributes()?;
            // a hardware encoder is asynchronous, and locked until told it's understood
            attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?;
            let (mut token, mut mgr) = (0u32, None);
            MFCreateDXGIDeviceManager(&mut token, &mut mgr)?;
            let mgr = mgr.unwrap();
            mgr.ResetDevice(&gpu.device, token)?;
            mft.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, mgr.as_raw() as usize)?;

            let out = MFCreateMediaType()?;
            out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            out.SetGUID(&MF_MT_SUBTYPE, &v.codec.subtype())?;
            out.SetUINT32(&MF_MT_AVG_BITRATE, v.bitrate)?;
            out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, v.codec.profile())?;
            out.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
            set_size(&out.cast()?, &MF_MT_FRAME_SIZE, v.width, v.height)?;
            set_size(&out.cast()?, &MF_MT_FRAME_RATE, v.fps, 1)?;
            set_size(&out.cast()?, &MF_MT_PIXEL_ASPECT_RATIO, 1, 1)?;

            // no stream ids of its own: 0 and 0
            let (mut ins, mut outs) = ([0u32], [0u32]);
            let (input, output) = match mft.GetStreamIDs(&mut ins, &mut outs) {
                Ok(()) => (ins[0], outs[0]),
                Err(_) => (0, 0),
            };

            // the rate and the keyframes go in before the types are set
            let api: ICodecAPI = mft.cast()?;
            match v.quality {
                Some(q) => {
                    api.SetValue(&CODECAPI_AVEncCommonRateControlMode, &u4(eAVEncCommonRateControlMode_Quality.0 as u32))?;
                    api.SetValue(&CODECAPI_AVEncCommonQuality, &u4(q.clamp(1, 100)))?;
                }
                None => {
                    api.SetValue(&CODECAPI_AVEncCommonRateControlMode, &u4(eAVEncCommonRateControlMode_UnconstrainedVBR.0 as u32))?;
                    api.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &u4(v.bitrate))?;
                }
            }
            let _ = api.SetValue(&CODECAPI_AVEncMPVGOPSize, &u4(v.fps * v.keyframe_secs.max(1)));
            // no frames held back and no reordering: what goes in comes right out
            let _ = api.SetValue(&CODECAPI_AVLowLatencyMode, &yes());

            mft.SetOutputType(output, &out, 0)?;
            let inp = MFCreateMediaType()?;
            inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            inp.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            inp.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            inp.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
            set_size(&inp.cast()?, &MF_MT_FRAME_SIZE, v.width, v.height)?;
            set_size(&inp.cast()?, &MF_MT_FRAME_RATE, v.fps, 1)?;
            set_size(&inp.cast()?, &MF_MT_PIXEL_ASPECT_RATIO, 1, 1)?;
            mft.SetInputType(input, &inp, 0)?;
            let out_type = mft.GetOutputCurrentType(output)?;

            let info = mft.GetOutputStreamInfo(output)?;
            let own = info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32 != 0;
            let own_size = (!own).then_some(info.cbSize.max(v.width * v.height * 2));

            let converter = Converter::new(gpu, v)?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            Ok(LiveEncoder {
                shared: Arc::new(Shared {
                    mft,
                    input,
                    output,
                    own_size,
                    queue: Mutex::new(Queue::default()),
                    wake: Condvar::new(),
                    submitted: AtomicU64::new(0),
                    came_out: AtomicU64::new(0),
                    last_out: Mutex::new(Instant::now()),
                    failed: AtomicBool::new(false),
                    finished: Mutex::new(false),
                    finished_cv: Condvar::new(),
                    dropped: AtomicU64::new(0),
                }),
                activate: activate.clone(),
                converter,
                gpu: gpu.clone(),
                spec: *v,
                out_type,
                name: name.to_string(),
                started: AtomicBool::new(false),
            })
        }
    }

    pub fn out_type(&self) -> &IMFMediaType {
        &self.out_type
    }

    /// The encoder runs from here on; its packets go to `sink`.
    pub fn start(&self, sink: impl FnMut(Packet) + Send + 'static) {
        self.started.store(true, Ordering::SeqCst);
        let s = self.shared.clone();
        std::thread::spawn(move || {
            crate::media::startup();
            feed(&s);
        });
        let s = self.shared.clone();
        let fallback = super::encoder::HNS / self.spec.fps.max(1) as i64;
        std::thread::spawn(move || {
            crate::media::startup();
            events(&s, fallback, sink);
            *s.finished.lock().unwrap() = true;
            s.finished_cv.notify_all();
        });
    }

    /// One frame (BGRA texture of the video size). Dropped if the encoder is
    /// behind. Never waits for it.
    pub fn write(&self, tex: &ID3D11Texture2D, t: i64, dur: i64) {
        let s = &self.shared;
        let behind = s.submitted.load(Ordering::SeqCst).saturating_sub(s.came_out.load(Ordering::SeqCst)) >= IN_FLIGHT;
        if behind || s.queue.lock().unwrap().frames.len() >= QUEUE {
            s.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let sample = (|| -> Result<IMFSample> {
            unsafe {
                let nv12 = self.converter.convert(&self.gpu, tex, self.spec.width, self.spec.height)?;
                let buf = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, &nv12, 0, false)?;
                buf.SetCurrentLength(buf.cast::<IMF2DBuffer>()?.GetContiguousLength()?)?;
                let sample = MFCreateSample()?;
                sample.AddBuffer(&buf)?;
                sample.SetSampleTime(t)?;
                sample.SetSampleDuration(dur.max(1))?;
                Ok(sample)
            }
        })();
        match sample {
            Ok(sample) => {
                s.queue.lock().unwrap().frames.push_back(sample);
                s.wake.notify_all();
            }
            Err(e) => log::error(format!("video encoder: a frame: {e}")),
        }
    }

    /// It reported an error, or has frames and gives nothing back: of no
    /// more use (a new one takes over).
    pub fn broken(&self) -> bool {
        let s = &self.shared;
        s.failed.load(Ordering::SeqCst)
            || (s.submitted.load(Ordering::SeqCst) > s.came_out.load(Ordering::SeqCst) && s.last_out.lock().unwrap().elapsed() > STALL)
    }

    /// Frames dropped so far because the encoder was behind.
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    /// Everything still in it comes out through the sink. Waits `within` at
    /// most: an encoder stuck in its driver is left behind, not waited for.
    /// True if it finished.
    pub fn finish(&self, within: Duration) -> bool {
        if !self.started.load(Ordering::SeqCst) {
            unsafe {
                let _ = self.activate.ShutdownObject();
            }
            return true;
        }
        let s = &self.shared;
        s.queue.lock().unwrap().closing = true;
        s.wake.notify_all();
        let mut done = s.finished.lock().unwrap();
        let t0 = Instant::now();
        while !*done && t0.elapsed() < within {
            done = s.finished_cv.wait_timeout(done, Duration::from_millis(50)).unwrap().0;
        }
        let ok = *done;
        drop(done);
        if ok {
            unsafe {
                let _ = s.mft.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
                let _ = self.activate.ShutdownObject();
            }
        } else {
            log::error(format!("video encoder: {} didn't finish in {} s, left behind", self.name, within.as_secs()));
        }
        ok
    }
}

/// Gives the encoder the frames it has asked for; at the end, the drain.
fn feed(s: &Shared) {
    loop {
        let sample = {
            let mut q = s.queue.lock().unwrap();
            loop {
                if s.failed.load(Ordering::SeqCst) {
                    return;
                }
                if q.need > 0 && !q.frames.is_empty() {
                    q.need -= 1;
                    break q.frames.pop_front();
                }
                if q.closing && q.frames.is_empty() {
                    break None;
                }
                q = s.wake.wait_timeout(q, Duration::from_millis(200)).unwrap().0;
            }
        };
        let Some(sample) = sample else {
            unsafe {
                let _ = s.mft.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, s.input as usize);
                if s.mft.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, s.output as usize).is_err() {
                    s.failed.store(true, Ordering::SeqCst);
                }
            }
            return;
        };
        match unsafe { s.mft.ProcessInput(s.input, &sample, 0) } {
            Ok(()) => {
                s.submitted.fetch_add(1, Ordering::SeqCst);
            }
            Err(e) => {
                log::error(format!("video encoder: input: {e}"));
                s.failed.store(true, Ordering::SeqCst);
                s.wake.notify_all();
                return;
            }
        }
    }
}

/// The encoder's events: it wants a frame, it has a packet, it's drained.
fn events(s: &Shared, fallback: i64, mut sink: impl FnMut(Packet)) {
    let Ok(gen_) = s.mft.cast::<IMFMediaEventGenerator>() else {
        s.failed.store(true, Ordering::SeqCst);
        return;
    };
    loop {
        let event = match unsafe { gen_.GetEvent(MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0)) } {
            Ok(e) => e,
            Err(e) => {
                log::error(format!("video encoder: events: {e}"));
                s.failed.store(true, Ordering::SeqCst);
                s.wake.notify_all();
                return;
            }
        };
        let kind = unsafe { event.GetType() }.unwrap_or(0);
        if kind == METransformNeedInput.0 as u32 {
            s.queue.lock().unwrap().need += 1;
            s.wake.notify_all();
        } else if kind == METransformHaveOutput.0 as u32 {
            match take_output(s, fallback) {
                Ok(Some(p)) => {
                    s.came_out.fetch_add(1, Ordering::SeqCst);
                    *s.last_out.lock().unwrap() = Instant::now();
                    sink(p);
                }
                Ok(None) => {}
                Err(e) => {
                    log::error(format!("video encoder: output: {e}"));
                    s.failed.store(true, Ordering::SeqCst);
                    s.wake.notify_all();
                    return;
                }
            }
        } else if kind == METransformDrainComplete.0 as u32 {
            return;
        } else if kind == MEError.0 as u32 {
            let status = unsafe { event.GetStatus() }.map(|h| h.0).unwrap_or(0);
            log::error(format!("video encoder: error event 0x{status:08x}"));
            s.failed.store(true, Ordering::SeqCst);
            s.wake.notify_all();
            return;
        }
    }
}

/// One packet out of the encoder.
fn take_output(s: &Shared, fallback: i64) -> Result<Option<Packet>> {
    unsafe {
        let mut own = None;
        if let Some(size) = s.own_size {
            let b = MFCreateMemoryBuffer(size)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&b)?;
            own = Some(sample);
        }
        let mut out = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: s.output, pSample: std::mem::ManuallyDrop::new(own), dwStatus: 0, pEvents: std::mem::ManuallyDrop::new(None) }];
        let mut status = 0u32;
        let r = s.mft.ProcessOutput(0, &mut out, &mut status);
        let sample = std::mem::ManuallyDrop::take(&mut out[0].pSample);
        drop(std::mem::ManuallyDrop::take(&mut out[0].pEvents));
        match r {
            Ok(()) => {}
            // no more for now; the stream's description changed: taken as it is
            Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
            Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                let t = s.mft.GetOutputAvailableType(s.output, 0)?;
                s.mft.SetOutputType(s.output, &t, 0)?;
                return Ok(None);
            }
            Err(e) => return Err(e),
        }
        let Some(sample) = sample else { return Ok(None) };
        let buf = sample.ConvertToContiguousBuffer()?;
        let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
        buf.Lock(&mut p, None, Some(&mut len))?;
        let data = std::slice::from_raw_parts(p, len as usize).to_vec();
        buf.Unlock()?;
        let t = sample.GetSampleTime()?;
        let dur = sample.GetSampleDuration().ok().filter(|d| *d > 0).unwrap_or(fallback);
        let key = sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0;
        Ok(Some(Packet { data, t, dur, key }))
    }
}

/// The encoder for `spec`, stepping down when it refuses: constant quality →
/// VBR, then 120 and 60 fps, H.265 → H.264. With the settings it took.
pub fn open_ladder(gpu: &Arc<Gpu>, spec: &VideoSpec) -> Option<(LiveEncoder, VideoSpec)> {
    let vbr = |s: VideoSpec| VideoSpec { quality: None, ..s };
    let mut tries = vec![*spec, vbr(*spec)];
    for fps in [120, 60] {
        if spec.fps > fps {
            tries.push(vbr(VideoSpec { fps, ..*spec }));
        }
    }
    if spec.codec != Codec::H264 {
        tries.push(vbr(VideoSpec { codec: Codec::H264, fps: spec.fps.min(60), ..*spec }));
    }
    let mut last = String::new();
    for s in tries {
        match LiveEncoder::open(gpu, &s) {
            Ok(e) => {
                log::info(format!("video encoder: {}, {}x{} at {} fps{}", e.name, s.width, s.height, s.fps, if s.quality.is_some() { ", constant quality" } else { "" }));
                return Some((e, s));
            }
            Err(e) => last = e.message().to_string(),
        }
    }
    log::info(format!("video encoder: no hardware one for {}x{} ({last})", spec.width, spec.height));
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::encoder::{Encoder, HNS};

    fn spec(w: u32, h: u32, codec: Codec) -> VideoSpec {
        VideoSpec { width: w, height: h, fps: 60, bitrate: 6_000_000, codec, quality: Some(60), keyframe_secs: 1 }
    }

    /// A texture all of one color, by copying it from memory.
    fn solid(gpu: &Gpu, w: u32, h: u32, bgra: [u8; 4]) -> ID3D11Texture2D {
        let t = gpu.texture(w, h, false).unwrap();
        let row: Vec<u8> = bgra.iter().copied().cycle().take((w * 4) as usize).collect();
        let all: Vec<u8> = row.iter().copied().cycle().take((w * h * 4) as usize).collect();
        unsafe { gpu.context.UpdateSubresource(&t, 0, None, all.as_ptr() as _, w * 4, 0) };
        t
    }

    /// The encoder is never Microsoft's D3D12 one.
    #[test]
    fn picks_the_vendors() {
        crate::media::startup();
        let gpu = Arc::new(Gpu::new().unwrap());
        for codec in [Codec::H264, Codec::Hevc] {
            if let Ok((a, name)) = pick(&gpu, codec) {
                eprintln!("{name}");
                assert!(!name.contains("DX12") && !name.contains("D3D12"), "{name}");
                unsafe {
                    let _ = a.ShutdownObject();
                }
            }
        }
    }

    /// Frames in, packets out, into an MP4 that opens, with the right size,
    /// length and colors. H.264 and H.265, small and HD.
    #[test]
    fn encodes_a_file() {
        crate::media::startup();
        for (codec, name, w, h) in [(Codec::H264, "h264", 640, 360), (Codec::H264, "h264", 1920, 1080), (Codec::Hevc, "hevc", 640, 360), (Codec::Hevc, "hevc", 1920, 1080)] {
            let gpu = Arc::new(Gpu::new().unwrap());
            let Ok(enc) = LiveEncoder::open(&gpu, &spec(w, h, codec)) else {
                eprintln!("no hardware {name} here");
                continue;
            };
            let path = std::env::temp_dir().join(format!("blinker-live-{name}-{}.mp4", std::process::id()));
            let file = Arc::new(Encoder::passthrough(&path, enc.out_type(), 0).expect("sink"));
            let (f2, keys) = (file.clone(), Arc::new(AtomicU64::new(0)));
            let k2 = keys.clone();
            enc.start(move |p| {
                if p.key {
                    k2.fetch_add(1, Ordering::SeqCst);
                }
                f2.write_packet(&p).unwrap();
            });
            // pure red in BGRA
            let tex = solid(&gpu, w, h, [0, 0, 255, 255]);
            for i in 0..120 {
                enc.write(&tex, i * HNS / 60, HNS / 60);
                std::thread::sleep(Duration::from_millis(8));
            }
            assert!(enc.finish(Duration::from_secs(5)), "{name}: didn't finish");
            file.finish_part().unwrap();
            assert_eq!(enc.dropped(), 0, "{name}: frames dropped");
            assert!(keys.load(Ordering::SeqCst) >= 2, "{name}: keyframes every second");
            let info = crate::media::probe(&path).expect("probe");
            assert_eq!((info.width, info.height), (w, h));
            assert!((info.duration - 2 * HNS).abs() < HNS / 5, "{name}: {}", info.duration);
            let (r, _) = crate::media::VideoReader::open(&path, w, h).expect("reader");
            let (_, img) = r.next().unwrap().expect("a frame");
            let px = img.get_pixel(w / 2, h / 2);
            eprintln!("{name}: {:?}", px.0);
            assert!(px[0] > 245 && px[1] < 12 && px[2] < 12, "{name} {w}x{h}: colors off: {:?}", px.0);
            drop(r);
            let _ = std::fs::remove_file(&path);
        }
    }

    /// Nothing is left behind by an encoder that runs on and on: handles
    /// stay put while frames go through.
    #[test]
    fn stays_open_without_leaking() {
        crate::media::startup();
        let gpu = Arc::new(Gpu::new().unwrap());
        let Ok(enc) = LiveEncoder::open(&gpu, &spec(1280, 720, Codec::H264)) else { return };
        let got = Arc::new(AtomicU64::new(0));
        let g2 = got.clone();
        enc.start(move |_| {
            g2.fetch_add(1, Ordering::SeqCst);
        });
        let tex = solid(&gpu, 1280, 720, [40, 80, 120, 255]);
        let handles = || {
            let mut n = 0u32;
            unsafe {
                windows_sys::Win32::System::Threading::GetProcessHandleCount(windows_sys::Win32::System::Threading::GetCurrentProcess(), &mut n);
            }
            n
        };
        for i in 0..60 {
            enc.write(&tex, i * HNS / 60, HNS / 60);
            std::thread::sleep(Duration::from_millis(16));
        }
        let before = handles();
        for i in 60..360 {
            enc.write(&tex, i * HNS / 60, HNS / 60);
            std::thread::sleep(Duration::from_millis(16));
        }
        let after = handles();
        assert!(got.load(Ordering::SeqCst) > 200, "packets: {}", got.load(Ordering::SeqCst));
        assert!(after <= before + 8, "handles {before} → {after}");
        assert!(!enc.broken());
        assert!(enc.finish(Duration::from_secs(5)));
    }
}

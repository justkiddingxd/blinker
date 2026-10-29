//! MP4 writer on Media Foundation: one H.264 video stream (hardware encoder
//! when available) plus any number of AAC audio streams.

use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows::Win32::Media::MediaFoundation::*;
use windows::core::{Interface, Result};

pub const AUDIO_RATE: u32 = 48_000;
pub const AUDIO_CHANNELS: u32 = 2;

/// 100 ns units per second (Media Foundation time base).
pub const HNS: i64 = 10_000_000;

/// How far ahead of the video the sound is fed when a file is written
/// offline: the sink writer stalls when one stream runs far ahead.
pub const AUDIO_LEAD: i64 = 0;

/// H.265 / AV1 are kept for comparison (`codec_bench`): on screen content
/// they were not smaller than H.264 at the same look, and H.264 plays everywhere.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

impl Codec {
    fn subtype(self) -> windows::core::GUID {
        match self {
            Codec::H264 => MFVideoFormat_H264,
            Codec::Hevc => MFVideoFormat_HEVC,
            Codec::Av1 => MFVideoFormat_AV1,
        }
    }
    fn profile(self) -> u32 {
        match self {
            Codec::H264 => eAVEncH264VProfile_High.0 as u32,
            Codec::Hevc => eAVEncH265VProfile_Main_420_8.0 as u32,
            Codec::Av1 => eAVEncAV1VProfile_Main_420_8.0 as u32,
        }
    }
}

/// A hardware H.265 encoder exists (checked once).
pub fn hevc_available() -> bool {
    static HEVC: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HEVC.get_or_init(|| has_codec(Codec::Hevc, true))
}

/// Hardware encoder / any decoder present for this codec.
#[allow(dead_code)]
pub fn has_codec(codec: Codec, encoder: bool) -> bool {
    crate::media::startup();
    let info = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: codec.subtype() };
    let (cat, flags) = if encoder {
        (MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER)
    } else {
        (MFT_CATEGORY_VIDEO_DECODER, MFT_ENUM_FLAG(MFT_ENUM_FLAG_SYNCMFT.0 | MFT_ENUM_FLAG_ASYNCMFT.0 | MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SORTANDFILTER.0))
    };
    let (inp, out) = if encoder { (None, Some(&info as *const _)) } else { (Some(&info as *const _), None) };
    let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut n = 0u32;
    unsafe {
        if MFTEnumEx(cat, flags, inp, out, &mut list, &mut n).is_err() {
            return false;
        }
        for i in 0..n as usize {
            drop(std::ptr::read(list.add(i)));
        }
        windows::Win32::System::Com::CoTaskMemFree(Some(list as _));
    }
    n > 0
}

/// The hardware encoder the sink writer would take first for this codec
/// accepts `v`'s size and rate. When it doesn't (too big for it, all its
/// sessions taken, none at all), the sink writer goes on down its list, where
/// Microsoft's "AVC DX12 Encoder" from the OpenCL, OpenGL and Vulkan
/// Compatibility Pack may sit: a debug build that ends the whole process with
/// an assertion box on what it can't do. Then the software encoder is used
/// instead. Asked once per codec, size and rate.
fn hardware_takes(v: &VideoSpec) -> bool {
    /// Answers so far: (profile, width, height, fps) and whether it took them.
    type Answers = Vec<((u32, u32, u32, u32), bool)>;
    static SEEN: std::sync::Mutex<Answers> = std::sync::Mutex::new(Vec::new());
    let key = (v.codec.profile(), v.width, v.height, v.fps);
    if let Some(&(_, ok)) = SEEN.lock().unwrap().iter().find(|(k, _)| *k == key) {
        return ok;
    }
    let ok = unsafe { probe_hardware(v) };
    SEEN.lock().unwrap().push((key, ok));
    ok
}

unsafe fn probe_hardware(v: &VideoSpec) -> bool {
    unsafe {
        let info = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: v.codec.subtype() };
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut n = 0u32;
        if MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER, None, Some(&info), &mut list, &mut n).is_err() {
            return false;
        }
        let all: Vec<Option<IMFActivate>> = (0..n as usize).map(|i| std::ptr::read(list.add(i))).collect();
        windows::Win32::System::Com::CoTaskMemFree(Some(list as _));
        let Some(Some(first)) = all.first() else { return false };
        // the D3D12 one is never the vendor's own
        let mut vendor = windows::core::PWSTR::null();
        let mut len = 0;
        if first.GetAllocatedString(&MFT_ENUM_HARDWARE_VENDOR_ID_Attribute, &mut vendor, &mut len).is_ok() {
            let id = vendor.to_string().unwrap_or_default();
            windows::Win32::System::Com::CoTaskMemFree(Some(vendor.0 as _));
            if id.eq_ignore_ascii_case("VEN_1414") {
                return false;
            }
        }
        let takes = (|| -> Result<()> {
            let mft: IMFTransform = first.ActivateObject()?;
            if let Ok(a) = mft.GetAttributes() {
                let _ = a.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1);
            }
            let out = MFCreateMediaType()?;
            out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            out.SetGUID(&MF_MT_SUBTYPE, &v.codec.subtype())?;
            out.SetUINT32(&MF_MT_AVG_BITRATE, v.bitrate)?;
            out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, v.codec.profile())?;
            set_size(&out.cast()?, &MF_MT_FRAME_SIZE, v.width, v.height)?;
            set_size(&out.cast()?, &MF_MT_FRAME_RATE, v.fps, 1)?;
            set_size(&out.cast()?, &MF_MT_PIXEL_ASPECT_RATIO, 1, 1)?;
            mft.SetOutputType(0, &out, 0)?;
            let inp = MFCreateMediaType()?;
            inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            inp.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            inp.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            set_size(&inp.cast()?, &MF_MT_FRAME_SIZE, v.width, v.height)?;
            set_size(&inp.cast()?, &MF_MT_FRAME_RATE, v.fps, 1)?;
            set_size(&inp.cast()?, &MF_MT_PIXEL_ASPECT_RATIO, 1, 1)?;
            mft.SetInputType(0, &inp, 0)?;
            Ok(())
        })();
        let _ = first.ShutdownObject();
        if let Err(e) = &takes {
            crate::config::log::info(format!("video encoder: the hardware one won't take {}x{} at {} fps ({}); the software one writes it", v.width, v.height, v.fps, e.message()));
        }
        takes.is_ok()
    }
}

#[derive(Clone, Copy)]
pub struct VideoSpec {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Mean bitrate (VBR), or the ceiling in quality mode; bits per second.
    pub bitrate: u32,
    pub codec: Codec,
    /// Constant quality 1..100 (like CRF): size follows how much moves.
    pub quality: Option<u32>,
    /// Keyframe interval (seeking / trimming precision).
    pub keyframe_secs: u32,
}

pub struct Encoder {
    writer: IMFSinkWriter,
    /// The file, for its metadata to go once it's done.
    path: std::path::PathBuf,
    video: Option<u32>,
    audio: Vec<u32>,
    width: u32,
    height: u32,
}

// The sink writer is free-threaded (its methods are serialized internally).
unsafe impl Send for Encoder {}
unsafe impl Sync for Encoder {}

fn attrs(n: u32) -> Result<IMFAttributes> {
    let mut a = None;
    unsafe { MFCreateAttributes(&mut a, n)? };
    Ok(a.unwrap())
}

pub fn set_size(t: &IMFAttributes, key: &windows::core::GUID, a: u32, b: u32) -> Result<()> {
    unsafe { t.SetUINT64(key, ((a as u64) << 32) | b as u64) }
}

pub fn audio_pcm_type(rate: u32, channels: u32) -> Result<IMFMediaType> {
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels)?;
        t.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, channels * 2)?;
        t.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * channels * 2)?;
        t.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        Ok(t)
    }
}

impl Encoder {
    /// `device`: frames come as GPU textures from this device (zero-copy into
    /// the hardware encoder). Without it, frames are written from memory.
    pub fn new(
        path: &std::path::Path,
        video: Option<&VideoSpec>,
        audio_tracks: usize,
        device: Option<&ID3D11Device>,
    ) -> Result<Self> {
        crate::media::startup();
        unsafe {
            let a = attrs(4)?;
            a.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
            // the hardware encoder only when it's sure to take this video (see
            // `hardware_takes`): the sink writer must not fall back on its own
            let hardware = video.is_none_or(hardware_takes);
            a.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, hardware as u32)?;
            if let Some(dev) = device {
                let mut token = 0u32;
                let mut mgr = None;
                MFCreateDXGIDeviceManager(&mut token, &mut mgr)?;
                let mgr = mgr.unwrap();
                mgr.ResetDevice(dev, token)?;
                a.SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, &mgr)?;
            }
            let wide: Vec<u16> = path.as_os_str().encode_wide_null();
            let writer = MFCreateSinkWriterFromURL(windows::core::PCWSTR(wide.as_ptr()), None, &a)?;

            let (mut width, mut height) = (0, 0);
            let video_index = match video {
                Some(v) => {
                    width = v.width;
                    height = v.height;
                    let out = MFCreateMediaType()?;
                    out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                    out.SetGUID(&MF_MT_SUBTYPE, &v.codec.subtype())?;
                    out.SetUINT32(&MF_MT_AVG_BITRATE, v.bitrate)?;
                    out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                    out.SetUINT32(&MF_MT_MPEG2_PROFILE, v.codec.profile())?;
                    set_size(&out.cast()?, &MF_MT_FRAME_SIZE, v.width, v.height)?;
                    set_size(&out.cast()?, &MF_MT_FRAME_RATE, v.fps, 1)?;
                    set_size(&out.cast()?, &MF_MT_PIXEL_ASPECT_RATIO, 1, 1)?;
                    let idx = writer.AddStream(&out)?;

                    let inp = MFCreateMediaType()?;
                    inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                    inp.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
                    inp.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                    inp.SetUINT32(&MF_MT_DEFAULT_STRIDE, v.width * 4)?;
                    set_size(&inp.cast()?, &MF_MT_FRAME_SIZE, v.width, v.height)?;
                    set_size(&inp.cast()?, &MF_MT_FRAME_RATE, v.fps, 1)?;
                    set_size(&inp.cast()?, &MF_MT_PIXEL_ASPECT_RATIO, 1, 1)?;
                    // Encoder settings: VBR around the mean bitrate (still
                    // screens cost almost nothing), keyframe every 2 s for seeking.
                    let enc = attrs(4)?;
                    match v.quality {
                        Some(q) => {
                            enc.SetUINT32(&CODECAPI_AVEncCommonRateControlMode, eAVEncCommonRateControlMode_Quality.0 as u32)?;
                            enc.SetUINT32(&CODECAPI_AVEncCommonQuality, q.clamp(1, 100))?;
                        }
                        None => {
                            enc.SetUINT32(&CODECAPI_AVEncCommonRateControlMode, eAVEncCommonRateControlMode_UnconstrainedVBR.0 as u32)?;
                            enc.SetUINT32(&CODECAPI_AVEncCommonMeanBitRate, v.bitrate)?;
                        }
                    }
                    enc.SetUINT32(&CODECAPI_AVEncMPVGOPSize, v.fps * v.keyframe_secs.max(1))?;
                    writer.SetInputMediaType(idx, &inp, &enc)?;
                    Some(idx)
                }
                None => None,
            };

            let mut audio = Vec::new();
            for t in 0..audio_tracks {
                let out = MFCreateMediaType()?;
                out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                out.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
                out.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                out.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, AUDIO_RATE)?;
                out.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, AUDIO_CHANNELS)?;
                // Track 1 (desktop / mixed sound) 128 kbit/s, the microphone 96 kbit/s.
                out.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, if t == 0 { 16_000 } else { 12_000 })?;
                let idx = writer.AddStream(&out)?;
                writer.SetInputMediaType(idx, &audio_pcm_type(AUDIO_RATE, AUDIO_CHANNELS)?, None)?;
                audio.push(idx);
            }
            writer.BeginWriting()?;
            Ok(Self { writer, path: path.to_path_buf(), video: video_index, audio, width, height })
        }
    }

    fn write(&self, stream: u32, buffer: &IMFMediaBuffer, t: i64, dur: i64) -> Result<()> {
        unsafe {
            let s = MFCreateSample()?;
            s.AddBuffer(buffer)?;
            s.SetSampleTime(t)?;
            s.SetSampleDuration(dur.max(1))?;
            self.writer.WriteSample(stream, &s)
        }
    }

    /// One frame from a GPU texture (BGRA, exactly the video size).
    pub fn write_texture(&self, tex: &ID3D11Texture2D, t: i64, dur: i64) -> Result<()> {
        let Some(v) = self.video else { return Ok(()) };
        unsafe {
            let buf = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, tex, 0, false)?;
            let len = buf.cast::<IMF2DBuffer>()?.GetContiguousLength()?;
            buf.SetCurrentLength(len)?;
            self.write(v, &buf, t, dur)
        }
    }

    /// One frame from memory: top-down BGRA rows of `width * 4` bytes.
    pub fn write_bgra(&self, data: &[u8], t: i64, dur: i64) -> Result<()> {
        let Some(v) = self.video else { return Ok(()) };
        let len = (self.width * self.height * 4) as usize;
        unsafe {
            let buf = MFCreateMemoryBuffer(len as u32)?;
            let mut p = std::ptr::null_mut();
            buf.Lock(&mut p, None, None)?;
            std::ptr::copy_nonoverlapping(data.as_ptr(), p, len.min(data.len()));
            buf.Unlock()?;
            buf.SetCurrentLength(len as u32)?;
            self.write(v, &buf, t, dur)
        }
    }

    /// Interleaved stereo 16-bit PCM at 48 kHz starting at `t`.
    pub fn write_audio(&self, track: usize, pcm: &[i16], t: i64) -> Result<()> {
        let Some(&idx) = self.audio.get(track) else { return Ok(()) };
        if pcm.is_empty() {
            return Ok(());
        }
        let bytes = pcm.len() * 2;
        let frames = (pcm.len() / AUDIO_CHANNELS as usize) as i64;
        unsafe {
            let buf = MFCreateMemoryBuffer(bytes as u32)?;
            let mut p = std::ptr::null_mut();
            buf.Lock(&mut p, None, None)?;
            std::ptr::copy_nonoverlapping(pcm.as_ptr() as *const u8, p, bytes);
            buf.Unlock()?;
            buf.SetCurrentLength(bytes as u32)?;
            self.write(idx, &buf, t, frames * HNS / AUDIO_RATE as i64)
        }
    }

    /// No more sound will come (the macOS writer stops waiting for it).
    pub fn end_audio(&self) -> Result<()> {
        Ok(())
    }

    pub fn finish(&self) -> Result<()> {
        unsafe { self.writer.Finalize() }?;
        crate::strip::file(&self.path);
        Ok(())
    }
}

/// Sound only, as MP3 (the Media Foundation MP3 encoder, 192 kbit/s):
/// interleaved stereo 16-bit PCM at 48 kHz. `progress` gets 0..1.
pub fn write_mp3(dst: &std::path::Path, pcm: &[i16], progress: &dyn Fn(f32)) -> std::result::Result<(), String> {
    crate::media::startup();
    let r = (|| -> Result<()> {
        unsafe {
            let a = attrs(2)?;
            a.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MP3)?;
            let wide: Vec<u16> = dst.as_os_str().encode_wide_null();
            let writer = MFCreateSinkWriterFromURL(windows::core::PCWSTR(wide.as_ptr()), None, &a)?;
            let out = MFCreateMediaType()?;
            out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            out.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_MP3)?;
            out.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, AUDIO_RATE)?;
            out.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, AUDIO_CHANNELS)?;
            out.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 24_000)?;
            out.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 1)?;
            let idx = writer.AddStream(&out)?;
            writer.SetInputMediaType(idx, &audio_pcm_type(AUDIO_RATE, AUDIO_CHANNELS)?, None)?;
            writer.BeginWriting()?;
            let enc = Encoder { writer, path: dst.to_path_buf(), video: None, audio: vec![idx], width: 0, height: 0 };
            let chunk = (AUDIO_RATE / 10 * AUDIO_CHANNELS) as usize; // 100 ms
            let n = pcm.len().div_ceil(chunk).max(1);
            for (i, c) in pcm.chunks(chunk).enumerate() {
                enc.write_audio(0, c, i as i64 * HNS / 10)?;
                progress((i + 1) as f32 / n as f32);
            }
            enc.finish()
        }
    })();
    r.map_err(|e| e.message().to_string())
}

trait WideNull {
    fn encode_wide_null(&self) -> Vec<u16>;
}

impl WideNull for std::ffi::OsStr {
    fn encode_wide_null(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain(std::iter::once(0)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a 3 s synthetic clip with two audio tracks and reads it back.
    /// `BLINKER_TEST_MP4` = output path.
    #[test]
    fn encoder_smoke() {
        let Ok(path) = std::env::var("BLINKER_TEST_MP4") else { return };
        let path = std::path::PathBuf::from(path);
        let _ = std::fs::remove_file(&path);
        let (w, h, fps) = (640u32, 360u32, 30u32);
        let enc = Encoder::new(&path, Some(&VideoSpec { width: w, height: h, fps, bitrate: 2_000_000, codec: Codec::H264, quality: None, keyframe_secs: 2 }), 2, None)
            .expect("encoder");
        let mut frame = vec![0u8; (w * h * 4) as usize];
        for i in 0..fps * 3 {
            for y in 0..h {
                for x in 0..w {
                    let o = ((y * w + x) * 4) as usize;
                    frame[o] = ((x + i * 4) % 256) as u8;
                    frame[o + 1] = (y % 256) as u8;
                    frame[o + 2] = if (x / 40 + i / 10) % 2 == 0 { 220 } else { 30 };
                    frame[o + 3] = 255;
                }
            }
            let t = i as i64 * HNS / fps as i64;
            enc.write_bgra(&frame, t, HNS / fps as i64).unwrap();
        }
        // two tones, 100 ms chunks
        for track in 0..2 {
            let freq = if track == 0 { 440.0 } else { 660.0 };
            for chunk in 0..30 {
                let n = (AUDIO_RATE / 10) as usize;
                let mut pcm = Vec::with_capacity(n * 2);
                for k in 0..n {
                    let s = (chunk * n + k) as f32 / AUDIO_RATE as f32;
                    let v = ((s * freq * std::f32::consts::TAU).sin() * 8000.0) as i16;
                    pcm.push(v);
                    pcm.push(v);
                }
                enc.write_audio(track, &pcm, chunk as i64 * HNS / 10).unwrap();
            }
        }
        enc.finish().unwrap();
        let info = crate::media::probe(&path).expect("probe");
        assert_eq!(info.audio_tracks, 2, "{info:?}");
        assert_eq!((info.width, info.height), (w, h));
        assert!(info.duration > HNS * 29 / 10, "{info:?}");
    }

    /// GPU path: D3D11 textures straight into the (hardware) encoder.
    /// `BLINKER_TEST_MP4_GPU` = output path.
    #[test]
    fn encoder_gpu() {
        use windows::Win32::Graphics::Direct3D11::D3D11_BOX;
        let Ok(path) = std::env::var("BLINKER_TEST_MP4_GPU") else { return };
        let path = std::path::PathBuf::from(path);
        let _ = std::fs::remove_file(&path);
        let gpu = crate::record::video::Gpu::new().expect("d3d");
        let (w, h, fps) = (1280u32, 720u32, 60u32);
        let enc = Encoder::new(&path, Some(&VideoSpec { width: w, height: h, fps, bitrate: 6_000_000, codec: Codec::H264, quality: None, keyframe_secs: 2 }), 0, Some(&gpu.device))
            .expect("encoder");
        let mut frame = vec![0u8; (w * h * 4) as usize];
        let started = std::time::Instant::now();
        for i in 0..fps * 2 {
            for y in 0..h {
                for x in 0..w {
                    let o = ((y * w + x) * 4) as usize;
                    frame[o] = ((x + i * 8) % 256) as u8;
                    frame[o + 1] = (y % 256) as u8;
                    frame[o + 2] = if (x / 64 + i / 15) % 2 == 0 { 230 } else { 20 };
                    frame[o + 3] = 255;
                }
            }
            let tex = gpu.texture(w, h, false).unwrap();
            unsafe {
                gpu.context.UpdateSubresource(&tex, 0, None::<*const D3D11_BOX>, frame.as_ptr() as _, w * 4, 0);
            }
            enc.write_texture(&tex, i as i64 * HNS / fps as i64, HNS / fps as i64).unwrap();
        }
        enc.finish().unwrap();
        eprintln!("gpu encode: {:?}", started.elapsed());
        let info = crate::media::probe(&path).expect("probe");
        assert_eq!((info.width, info.height), (w, h), "{info:?}");
        assert!(info.duration > HNS * 19 / 10, "{info:?}");
    }

    /// Size benchmark on screen-like content (a text document scrolled and
    /// typed into, then idle). `BLINKER_TEST_BENCH` = scratch folder.
    #[test]
    fn codec_bench() {
        use crate::editor::render::rasterize_text;
        use eframe::egui::Color32;
        let Ok(dir) = std::env::var("BLINKER_TEST_BENCH") else { return };
        let dir = std::path::PathBuf::from(dir);
        for c in [Codec::H264, Codec::Hevc, Codec::Av1] {
            eprintln!("{c:?}: hw encoder {} · decoder {}", has_codec(c, true), has_codec(c, false));
        }
        // a long "document": dark sidebar + light page with text lines
        let (w, h, fps) = (1920u32, 1080u32, 60u32);
        let page_h = 4000u32;
        let mut doc = image::RgbaImage::from_pixel(w, page_h, image::Rgba([250, 250, 248, 255]));
        for y in 0..page_h {
            for x in 0..300 {
                doc.put_pixel(x, y, image::Rgba([32, 33, 36, 255]));
            }
        }
        let words = "Отчёт квартал выручка рост клиенты договор поставка сроки оплата склад заказ доставка счёт";
        let ws: Vec<&str> = words.split(' ').collect();
        for i in 0..(page_h / 28) {
            let line: String = (0..9).map(|k| ws[((i * 7 + k * 3) as usize) % ws.len()]).collect::<Vec<_>>().join(" ");
            if let Some(rt) = rasterize_text(&line, 17.0, Color32::from_rgb(30, 30, 32), false, false) {
                image::imageops::overlay(&mut doc, &rt.img, 360 + rt.offset.x as i64, (i * 28 + 10) as i64 + rt.offset.y as i64);
            }
            if i % 3 == 0
                && let Some(rt) = rasterize_text(&format!("Раздел {i}"), 14.0, Color32::from_rgb(200, 200, 205), false, false)
            {
                image::imageops::overlay(&mut doc, &rt.img, 24 + rt.offset.x as i64, (i * 28 + 10) as i64 + rt.offset.y as i64);
            }
        }
        // timeline: 6 s scrolling, 4 s idle (no frames: like WGC), 4 s small changes
        let mut frames: Vec<(i64, i64)> = Vec::new(); // (time, scroll offset)
        for i in 0..fps * 6 {
            frames.push((i as i64 * HNS / fps as i64, (i as i64 * 7) % (page_h - h) as i64));
        }
        let last = frames.last().unwrap().1;
        for i in 0..8 {
            frames.push(((10 * fps + i * fps / 2) as i64 * HNS / fps as i64, last + (i as i64 % 2)));
        }
        let total_secs = 14.0;
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        let cases: Vec<(&str, Codec, Option<u32>, u32)> = vec![
            ("h264-q45", Codec::H264, Some(45), 12_000_000),
            ("h264-q60", Codec::H264, Some(60), 12_000_000),
            ("h264-q72", Codec::H264, Some(72), 12_000_000),
            ("hevc-q50", Codec::Hevc, Some(50), 12_000_000),
            ("av1-q60", Codec::Av1, Some(60), 12_000_000),
            ("av1-vbr-2M", Codec::Av1, None, 2_000_000),
        ];
        for (name, codec, q, br) in cases {
            if !has_codec(codec, true) {
                continue;
            }
            let path = dir.join(format!("bench-{name}.mp4"));
            let _ = std::fs::remove_file(&path);
            let spec = VideoSpec { width: w, height: h, fps, bitrate: br, codec, quality: q, keyframe_secs: 2 };
            let enc = match Encoder::new(&path, Some(&spec), 0, None) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("{name}: {e}");
                    continue;
                }
            };
            let t0 = std::time::Instant::now();
            for (k, (t, off)) in frames.iter().enumerate() {
                for y in 0..h {
                    let src = &doc.as_raw()[(((y as i64 + off) as u32 * w) * 4) as usize..][..(w * 4) as usize];
                    let dst = &mut bgra[(y * w * 4) as usize..][..(w * 4) as usize];
                    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                        d[0] = s[2];
                        d[1] = s[1];
                        d[2] = s[0];
                        d[3] = 255;
                    }
                }
                let dur = frames.get(k + 1).map(|n| n.0 - t).unwrap_or(HNS / fps as i64);
                if let Err(e) = enc.write_bgra(&bgra, *t, dur) {
                    eprintln!("{name}: write {e}");
                    break;
                }
            }
            let _ = enc.write_bgra(&bgra, (total_secs * HNS as f64) as i64, HNS / fps as i64);
            if let Err(e) = enc.finish() {
                eprintln!("{name}: finish {e}");
                continue;
            }
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            eprintln!(
                "{name:>14}: {:>6.2} MB · {:>5.1} MB/min · {:?}",
                size as f64 / 1e6,
                size as f64 / 1e6 / total_secs * 60.0,
                t0.elapsed()
            );
            // frame mid-scroll for a visual check
            if let Ok((r, _)) = crate::media::VideoReader::open(&path, w, h) {
                let _ = r.seek(3 * HNS);
                let mut best = None;
                while let Ok(Some((ts, img))) = r.next() {
                    best = Some(img);
                    if ts >= 3 * HNS {
                        break;
                    }
                }
                if let Some(img) = best {
                    let crop = image::imageops::crop_imm(&img, 300, 200, 700, 300).to_image();
                    let _ = crop.save(dir.join(format!("bench-{name}.png")));
                }
            }
        }
    }
}


#[cfg(test)]
mod hardware_tests {
    use super::*;

    /// The hardware encoder answers for usual sizes (when there is one), and a
    /// size no encoder takes is turned down instead of handed on.
    #[test]
    fn hardware_probe() {
        crate::media::startup();
        let spec = |w, h, fps| VideoSpec { width: w, height: h, fps, bitrate: 8_000_000, codec: Codec::H264, quality: None, keyframe_secs: 2 };
        let usual = hardware_takes(&spec(2560, 1440, 60));
        eprintln!("2560x1440@60: {usual}, 1920x1080@144: {}, 16384x16384@60: {}", hardware_takes(&spec(1920, 1080, 144)), hardware_takes(&spec(16384, 16384, 60)));
        if has_codec(Codec::H264, true) {
            assert!(usual, "a usual size must stay on the hardware encoder");
        }
        assert!(!hardware_takes(&spec(16384, 16384, 60)));
    }

    /// A size the hardware encoder turns down (wider than NVENC's H.264
    /// 4096) is written by the software one or refused, never handed to
    /// whatever encoder comes next.
    #[test]
    fn too_wide_for_hardware() {
        crate::media::startup();
        let spec = VideoSpec { width: 5120, height: 1440, fps: 30, bitrate: 12_000_000, codec: Codec::H264, quality: None, keyframe_secs: 2 };
        let dst = std::env::temp_dir().join(format!("blinker-wide-{}.mp4", std::process::id()));
        let r = Encoder::new(&dst, Some(&spec), 0, None).and_then(|enc| {
            let frame = vec![90u8; 5120 * 1440 * 4];
            for k in 0..5 {
                enc.write_bgra(&frame, k * HNS / 30, HNS / 30)?;
            }
            enc.finish()
        });
        eprintln!("5120x1440 h264: {:?}, {} bytes", r.as_ref().map(|_| ()), std::fs::metadata(&dst).map(|m| m.len()).unwrap_or(0));
        let _ = std::fs::remove_file(&dst);
    }

    /// The same from GPU textures, as screen recording writes them.
    #[test]
    fn too_wide_for_hardware_textures() {
        use windows::Win32::Graphics::Direct3D11::D3D11_BOX;
        crate::media::startup();
        let gpu = crate::record::video::Gpu::new().expect("d3d");
        let (w, h, fps) = (5120u32, 1440u32, 30u32);
        let spec = VideoSpec { width: w, height: h, fps, bitrate: 12_000_000, codec: Codec::H264, quality: None, keyframe_secs: 2 };
        let dst = std::env::temp_dir().join(format!("blinker-wide-gpu-{}.mp4", std::process::id()));
        let r = Encoder::new(&dst, Some(&spec), 0, Some(&gpu.device)).and_then(|enc| {
            let frame = vec![120u8; (w * h * 4) as usize];
            for k in 0..10 {
                let tex = gpu.texture(w, h, false).unwrap();
                unsafe { gpu.context.UpdateSubresource(&tex, 0, None::<*const D3D11_BOX>, frame.as_ptr() as _, w * 4, 0) };
                enc.write_texture(&tex, k * HNS / fps as i64, HNS / fps as i64)?;
            }
            enc.finish()
        });
        let info = crate::media::probe(&dst).ok();
        eprintln!("5120x1440 h264 from textures: {:?}, {:?}", r.as_ref().map(|_| ()), info.map(|i| (i.width, i.height, i.duration)));
        let _ = std::fs::remove_file(&dst);
    }
}

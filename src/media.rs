//! Media Foundation basics shared by the recorder and the video editor.

use std::path::Path;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::{GUID, Interface, PCWSTR};
pub use windows::core::Result;

/// Error of the media layer (the macOS build has its own with the same `message()`).
pub type Error = windows::core::Error;

static MAIN_THREAD: std::sync::OnceLock<std::thread::ThreadId> = std::sync::OnceLock::new();

/// Call first thing in `main`: the UI thread must stay single-threaded COM
/// (winit needs OLE there for drag and drop).
pub fn mark_main_thread() {
    let _ = MAIN_THREAD.set(std::thread::current().id());
}

/// COM for this thread (STA on the UI thread, MTA on workers) + Media
/// Foundation for the process. Cheap to repeat.
pub fn startup() {
    thread_local!(static COM: () = unsafe {
        let main = MAIN_THREAD.get() == Some(&std::thread::current().id());
        let _ = CoInitializeEx(None, if main { COINIT_APARTMENTTHREADED } else { COINIT_MULTITHREADED });
    });
    COM.with(|_| ());
    static MF: std::sync::Once = std::sync::Once::new();
    MF.call_once(|| unsafe {
        let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
    });
}

pub fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

#[derive(Debug, Clone, Default)]
pub struct MediaInfo {
    pub width: u32,
    pub height: u32,
    /// Frames per second (nominal).
    pub fps: f64,
    /// Duration in 100 ns units.
    pub duration: i64,
    /// Source-reader stream index of the video stream.
    pub video_stream: Option<u32>,
    /// Source-reader stream indices of the audio streams, in file order.
    pub audio_streams: Vec<u32>,
    pub audio_tracks: usize,
}

pub fn open_reader(path: &Path, video_processing: bool) -> Result<IMFSourceReader> {
    startup();
    unsafe {
        let mut a = None;
        MFCreateAttributes(&mut a, 2)?;
        let a = a.unwrap();
        if video_processing {
            a.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        }
        let w = wide(path);
        MFCreateSourceReaderFromURL(PCWSTR(w.as_ptr()), &a)
    }
}

pub fn probe(path: &Path) -> Result<MediaInfo> {
    let reader = open_reader(path, false)?;
    probe_reader(&reader)
}

pub fn probe_reader(reader: &IMFSourceReader) -> Result<MediaInfo> {
    let mut info = MediaInfo::default();
    let mut rates = Vec::new();
    unsafe {
        let mut i = 0u32;
        while let Ok(t) = reader.GetNativeMediaType(i, 0) {
            let major = t.GetGUID(&MF_MT_MAJOR_TYPE)?;
            if major == MFMediaType_Video && info.video_stream.is_none() {
                info.video_stream = Some(i);
                let v = t.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
                info.width = (v >> 32) as u32;
                info.height = v as u32;
                // H.265 frames are coded in 16/32 px blocks: the visible size
                // is the display aperture (e.g. 1920x1080 inside 1920x1088).
                let mut area = MFVideoArea::default();
                let blob = std::slice::from_raw_parts_mut(&mut area as *mut MFVideoArea as *mut u8, std::mem::size_of::<MFVideoArea>());
                if t.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, blob, None).is_ok() && area.Area.cx > 0 && area.Area.cy > 0 {
                    info.width = area.Area.cx as u32;
                    info.height = area.Area.cy as u32;
                }
                let r = t.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or(0);
                let (n, d) = ((r >> 32) as f64, (r as u32) as f64);
                info.fps = if d > 0.0 { n / d } else { 30.0 };
            } else if major == MFMediaType_Audio {
                info.audio_streams.push(i);
                rates.push(t.GetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND).unwrap_or(0));
            }
            i += 1;
        }
        // Our recordings: the desktop sound (128 kbit/s) is track 1, the
        // microphone (96 kbit/s) track 2. The file source may list them the
        // other way round: the bitrate tells which is which.
        if let ([a, b], [ra, rb]) = (info.audio_streams.as_mut_slice(), rates.as_slice())
            && *ra == 12_000
            && *rb == 16_000
        {
            std::mem::swap(a, b);
        }
        info.audio_tracks = info.audio_streams.len();
        if let Ok(pv) = reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION) {
            info.duration = pv.Anonymous.Anonymous.Anonymous.uhVal as i64;
        }
    }
    Ok(info)
}

/// Seeks a source reader to `t` (100 ns).
pub fn seek(reader: &IMFSourceReader, t: i64) -> Result<()> {
    unsafe {
        let mut pv = PROPVARIANT::default();
        (*pv.Anonymous.Anonymous).vt = windows::Win32::System::Variant::VT_I8;
        (*pv.Anonymous.Anonymous).Anonymous.hVal = t.max(0);
        reader.SetCurrentPosition(&GUID::zeroed(), &pv)
    }
}

/// Decodes video frames as RGBA, scaled by the source reader's video processor.
pub struct VideoReader {
    reader: IMFSourceReader,
    stream: u32,
    pub width: u32,
    pub height: u32,
}

impl VideoReader {
    /// Opens the first video stream, output fitted inside `max_w × max_h`.
    pub fn open(path: &Path, max_w: u32, max_h: u32) -> Result<(Self, MediaInfo)> {
        let reader = open_reader(path, true)?;
        let info = probe_reader(&reader)?;
        let stream = info.video_stream.ok_or_else(|| windows::core::Error::from_hresult(MF_E_INVALIDSTREAMNUMBER))?;
        let k = (max_w as f64 / info.width.max(1) as f64).min(max_h as f64 / info.height.max(1) as f64).min(1.0);
        let w = (((info.width as f64 * k) as u32) & !1).max(2);
        let h = (((info.height as f64 * k) as u32) & !1).max(2);
        unsafe {
            reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
            reader.SetStreamSelection(stream, true)?;
            let t = MFCreateMediaType()?;
            t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
            t.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64)?;
            reader.SetCurrentMediaType(stream, None, &t)?;
        }
        Ok((Self { reader, stream, width: w, height: h }, info))
    }

    pub fn seek(&self, t: i64) -> Result<()> {
        seek(&self.reader, t)
    }

    /// Next decoded frame: (timestamp, image). `None` at the end.
    pub fn next(&self) -> Result<Option<(i64, image::RgbaImage)>> {
        unsafe {
            loop {
                let mut flags = 0u32;
                let mut ts = 0i64;
                let mut sample = None;
                self.reader.ReadSample(self.stream, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample))?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    return Ok(None);
                }
                let Some(sample) = sample else { continue };
                let buf = sample.ConvertToContiguousBuffer()?;
                let (w, h) = (self.width, self.height);
                let mut img = image::RgbaImage::new(w, h);
                let dst = img.as_mut();
                let mut copy = |src: *const u8, pitch: isize| {
                    for y in 0..h as isize {
                        let row = std::slice::from_raw_parts(src.offset(y * pitch), (w * 4) as usize);
                        let out = &mut dst[(y as usize) * (w as usize) * 4..][..(w * 4) as usize];
                        for (o, i) in out.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                            o[0] = i[2];
                            o[1] = i[1];
                            o[2] = i[0];
                            o[3] = 255;
                        }
                    }
                };
                if let Ok(b2) = buf.cast::<IMF2DBuffer>() {
                    let mut p = std::ptr::null_mut();
                    let mut pitch = 0i32;
                    b2.Lock2D(&mut p, &mut pitch)?;
                    copy(p, pitch as isize);
                    b2.Unlock2D()?;
                } else {
                    let mut p = std::ptr::null_mut();
                    buf.Lock(&mut p, None, None)?;
                    copy(p, (w * 4) as isize);
                    buf.Unlock()?;
                }
                return Ok(Some((ts, img)));
            }
        }
    }
}

/// Decodes one audio stream to interleaved 16-bit PCM (48 kHz stereo).
pub struct AudioReader {
    reader: IMFSourceReader,
    stream: u32,
}

impl AudioReader {
    pub fn open(path: &Path, stream: u32) -> Result<Self> {
        let reader = open_reader(path, false)?;
        unsafe {
            reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
            reader.SetStreamSelection(stream, true)?;
            let t = crate::record::encoder::audio_pcm_type(48_000, 2)?;
            reader.SetCurrentMediaType(stream, None, &t)?;
        }
        Ok(Self { reader, stream })
    }

    pub fn seek(&self, t: i64) -> Result<()> {
        seek(&self.reader, t)
    }

    /// Next chunk: (timestamp, samples). `None` at the end.
    pub fn next(&self) -> Result<Option<(i64, Vec<i16>)>> {
        unsafe {
            loop {
                let mut flags = 0u32;
                let mut ts = 0i64;
                let mut sample = None;
                self.reader.ReadSample(self.stream, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample))?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    return Ok(None);
                }
                let Some(sample) = sample else { continue };
                let buf = sample.ConvertToContiguousBuffer()?;
                let mut p = std::ptr::null_mut();
                let mut len = 0u32;
                buf.Lock(&mut p, None, Some(&mut len))?;
                let pcm = std::slice::from_raw_parts(p as *const i16, len as usize / 2).to_vec();
                buf.Unlock()?;
                return Ok(Some((ts, pcm)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decodes a frame from `BLINKER_TEST_DECODE` (mp4) into `<path>.png`.
    #[test]
    fn decode_frame() {
        let Ok(path) = std::env::var("BLINKER_TEST_DECODE") else { return };
        let path = std::path::PathBuf::from(path);
        let (r, info) = VideoReader::open(&path, 640, 640).expect("open");
        eprintln!("{info:?} -> {}x{}", r.width, r.height);
        r.seek(10_000_000).unwrap();
        let (ts, img) = r.next().unwrap().expect("frame");
        eprintln!("frame at {ts}");
        img.save(path.with_extension("png")).unwrap();
        for s in &info.audio_streams {
            let a = AudioReader::open(&path, *s).expect("audio");
            let mut total = 0;
            while let Some((_, pcm)) = a.next().unwrap() {
                total += pcm.len();
            }
            eprintln!("audio stream {s}: {} s", total as f64 / 96_000.0);
        }
    }
}

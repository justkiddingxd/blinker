//! Reading media files on macOS (AVFoundation, hardware decoding): the same
//! API the Windows build has over Media Foundation.

use crate::win::ffi;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::{Path, PathBuf};

/// Error of the media layer.
#[derive(Debug, Clone)]
pub struct Error(String);

impl Error {
    pub fn new(msg: impl Into<String>) -> Self {
        Error(msg.into())
    }
    pub fn message(&self) -> String {
        self.0.clone()
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Nothing to set up on macOS (kept for the shared code).
pub fn mark_main_thread() {}

pub fn startup() {}

#[derive(Debug, Clone, Default)]
pub struct MediaInfo {
    pub width: u32,
    pub height: u32,
    /// Frames per second (nominal).
    pub fps: f64,
    /// Duration in 100 ns units.
    pub duration: i64,
    /// Track index of the video.
    pub video_stream: Option<u32>,
    /// Track indices of the audio tracks, in file order.
    pub audio_streams: Vec<u32>,
    pub audio_tracks: usize,
}

pub fn probe(path: &Path) -> Result<MediaInfo> {
    let p = ffi::cpath(path);
    let mut raw = ffi::MediaInfo { width: 0, height: 0, fps: 0.0, duration: 0, video_stream: -1, audio_streams: [0; 16], audio_count: 0 };
    let mut err = ffi::ErrBuf::new();
    if unsafe { ffi::bk_probe(p.as_ptr(), &mut raw, err.ptr(), ffi::ErrBuf::LEN) } == 0 {
        return Err(Error(err.text()));
    }
    let audio: Vec<u32> = raw.audio_streams[..raw.audio_count.clamp(0, 16) as usize].iter().map(|s| *s as u32).collect();
    Ok(MediaInfo {
        width: raw.width.max(0) as u32,
        height: raw.height.max(0) as u32,
        fps: if raw.fps > 0.0 { raw.fps } else { 30.0 },
        duration: raw.duration,
        video_stream: (raw.video_stream >= 0).then_some(raw.video_stream as u32),
        audio_tracks: audio.len(),
        audio_streams: audio,
    })
}

/// Decodes video frames as RGBA, scaled to fit.
pub struct VideoReader {
    path: PathBuf,
    native: Cell<*mut c_void>,
    buf: RefCell<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

unsafe impl Send for VideoReader {}

impl VideoReader {
    /// Opens the first video track, output fitted inside `max_w × max_h`.
    pub fn open(path: &Path, max_w: u32, max_h: u32) -> Result<(Self, MediaInfo)> {
        let info = probe(path)?;
        if info.video_stream.is_none() {
            return Err(Error::new("no video"));
        }
        let k = (max_w as f64 / info.width.max(1) as f64).min(max_h as f64 / info.height.max(1) as f64).min(1.0);
        let w = (((info.width as f64 * k) as u32) & !1).max(2);
        let h = (((info.height as f64 * k) as u32) & !1).max(2);
        let r = Self { path: path.to_path_buf(), native: Cell::new(std::ptr::null_mut()), buf: RefCell::new(Vec::new()), width: w, height: h };
        r.reopen(0)?;
        Ok((r, info))
    }

    fn reopen(&self, from: i64) -> Result<()> {
        let old = self.native.replace(std::ptr::null_mut());
        unsafe { ffi::bk_vreader_free(old) };
        let p = ffi::cpath(&self.path);
        let mut err = ffi::ErrBuf::new();
        let n = unsafe { ffi::bk_vreader_open(p.as_ptr(), self.width as i32, self.height as i32, from.max(0), err.ptr(), ffi::ErrBuf::LEN) };
        if n.is_null() {
            return Err(Error(err.text()));
        }
        self.native.set(n);
        Ok(())
    }

    pub fn seek(&self, t: i64) -> Result<()> {
        self.reopen(t)
    }

    /// Next decoded frame: (timestamp, image). `None` at the end.
    pub fn next(&self) -> Result<Option<(i64, image::RgbaImage)>> {
        let n = self.native.get();
        if n.is_null() {
            return Ok(None);
        }
        let mut buf = self.buf.borrow_mut();
        buf.resize((self.width * self.height * 4) as usize, 0);
        let mut ts = 0i64;
        match unsafe { ffi::bk_vreader_next(n, buf.as_mut_ptr(), &mut ts) } {
            1 => Ok(image::RgbaImage::from_raw(self.width, self.height, buf.clone()).map(|img| (ts, img))),
            0 => Ok(None),
            _ => Err(Error::new("decoding failed")),
        }
    }
}

impl Drop for VideoReader {
    fn drop(&mut self) {
        unsafe { ffi::bk_vreader_free(self.native.get()) };
    }
}

/// Decodes one audio track to interleaved 16-bit PCM (48 kHz stereo).
pub struct AudioReader {
    path: PathBuf,
    stream: u32,
    native: Cell<*mut c_void>,
}

unsafe impl Send for AudioReader {}

impl AudioReader {
    pub fn open(path: &Path, stream: u32) -> Result<Self> {
        let r = Self { path: path.to_path_buf(), stream, native: Cell::new(std::ptr::null_mut()) };
        r.reopen(0)?;
        Ok(r)
    }

    fn reopen(&self, from: i64) -> Result<()> {
        let old = self.native.replace(std::ptr::null_mut());
        unsafe { ffi::bk_areader_free(old) };
        let p = ffi::cpath(&self.path);
        let mut err = ffi::ErrBuf::new();
        let n = unsafe { ffi::bk_areader_open(p.as_ptr(), self.stream as i32, from.max(0), err.ptr(), ffi::ErrBuf::LEN) };
        if n.is_null() {
            return Err(Error(err.text()));
        }
        self.native.set(n);
        Ok(())
    }

    pub fn seek(&self, t: i64) -> Result<()> {
        self.reopen(t)
    }

    /// Next chunk: (timestamp, samples). `None` at the end.
    pub fn next(&self) -> Result<Option<(i64, Vec<i16>)>> {
        let n = self.native.get();
        if n.is_null() {
            return Ok(None);
        }
        let (mut pcm, mut frames, mut ts) = (std::ptr::null(), 0, 0i64);
        match unsafe { ffi::bk_areader_next(n, &mut pcm, &mut frames, &mut ts) } {
            1 => Ok(Some((ts, unsafe { std::slice::from_raw_parts(pcm, frames.max(0) as usize * 2) }.to_vec()))),
            0 => Ok(None),
            _ => Err(Error::new("decoding failed")),
        }
    }
}

impl Drop for AudioReader {
    fn drop(&mut self) {
        unsafe { ffi::bk_areader_free(self.native.get()) };
    }
}

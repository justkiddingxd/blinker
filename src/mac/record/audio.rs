//! Audio from ScreenCaptureKit: the desktop mix (everything playing, except
//! blinker's own sounds) and the microphone (the chosen one, or the
//! default), each into its own track, as 48 kHz stereo 16-bit.

use super::encoder::{AUDIO_CHANNELS, AUDIO_RATE, HNS};
use super::{AudioSink, Clock};
use crate::media::Error;
use crate::win::ffi;
use std::ffi::c_void;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Desktop,
    Mic,
}

/// Records from this microphone from now on (`""` = the system default).
pub fn use_mic(id: &str) {
    let c = ffi::cstr(id);
    unsafe { ffi::bk_set_mic(c.as_ptr()) };
}

/// The microphones there are: (device id, name).
pub fn mics() -> Vec<(String, String)> {
    extern "C" fn add(ctx: *mut c_void, id: *const std::ffi::c_char, name: *const std::ffi::c_char) {
        let out = unsafe { &mut *(ctx as *mut Vec<(String, String)>) };
        let s = |p: *const std::ffi::c_char| unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned();
        out.push((s(id), s(name)));
    }
    let mut out: Vec<(String, String)> = Vec::new();
    unsafe { ffi::bk_mic_list(add, &mut out as *mut _ as *mut c_void) };
    out
}

/// Whether a source can be recorded (the microphone needs macOS 15).
pub fn available(source: Source) -> bool {
    match source {
        Source::Desktop => true,
        Source::Mic => unsafe { ffi::bk_mic_available() != 0 },
    }
}

struct Track {
    sink: Arc<dyn AudioSink>,
    track: usize,
    clock: Arc<Clock>,
    /// Frames on the track so far.
    written: Mutex<i64>,
}

/// Frames → 100 ns.
fn frames_to_hns(f: i64) -> i64 {
    f * HNS / AUDIO_RATE as i64
}

impl Track {
    fn silence(&self, written: &mut i64, frames: i64) {
        let ch = AUDIO_CHANNELS as usize;
        let mut left = frames;
        while left > 0 {
            let n = left.min(AUDIO_RATE as i64 / 10);
            let z = vec![0i16; n as usize * ch];
            self.sink.write_audio(self.track, &z, frames_to_hns(*written));
            *written += n;
            left -= n;
        }
    }

    /// A packet stamped `t` (host clock). Gaps of more than 20 ms (nothing
    /// playing) are filled with silence so the track stays in sync.
    fn packet(&self, pcm: &[i16], t: i64) {
        let ch = AUDIO_CHANNELS as usize;
        let frames = (pcm.len() / ch) as i64;
        // dropped while paused
        let Some(at) = self.clock.map(t) else { return };
        let mut written = self.written.lock().unwrap();
        let skip = if at < 0 { (-at * AUDIO_RATE as i64 / HNS).min(frames) } else { 0 };
        let start_frame = (at.max(0) * AUDIO_RATE as i64) / HNS;
        if start_frame > *written + AUDIO_RATE as i64 / 50 {
            let gap = start_frame - *written;
            self.silence(&mut written, gap);
        }
        let count = frames - skip;
        if count > 0 {
            self.sink.write_audio(self.track, &pcm[skip as usize * ch..], frames_to_hns(*written));
            *written += count;
        }
    }
}

extern "C" fn on_audio(ctx: *mut c_void, pcm: *const i16, frames: i32, t: i64) {
    let track = unsafe { &*(ctx as *const Track) };
    let pcm = unsafe { std::slice::from_raw_parts(pcm, frames.max(0) as usize * AUDIO_CHANNELS as usize) };
    track.packet(pcm, t);
}

extern "C" fn no_video(_: *mut c_void, _: *mut c_void, _: i64) {}

pub struct AudioCapture {
    stream: *mut c_void,
    track: *mut Track,
    end: Arc<AtomicI64>,
}

unsafe impl Send for AudioCapture {}

impl AudioCapture {
    /// Starts capturing into `track` of `sink`.
    /// `end`: 0 while recording; set to the stop time (recording clock) before `join`.
    pub fn start(source: Source, sink: Arc<dyn AudioSink>, track: usize, clock: Arc<Clock>, end: Arc<AtomicI64>) -> Result<Self, Error> {
        let display = crate::win::displays().first().map(|d| d.id).unwrap_or(0);
        let cfg = ffi::StreamCfg { display, kind: if source == Source::Desktop { 1 } else { 2 }, ..Default::default() };
        let t = Box::into_raw(Box::new(Track { sink, track, clock, written: Mutex::new(0) }));
        let mut err = ffi::ErrBuf::new();
        let stream = unsafe { ffi::bk_stream_start(&cfg, no_video, on_audio, t as *mut c_void, err.ptr(), ffi::ErrBuf::LEN) };
        if stream.is_null() {
            drop(unsafe { Box::from_raw(t) });
            return Err(Error::new(err.text()));
        }
        Ok(Self { stream, track: t, end })
    }

    /// Stops and pads the track with silence up to the stop moment, so it is
    /// as long as the video.
    pub fn join(mut self) {
        unsafe { ffi::bk_stream_stop(std::mem::replace(&mut self.stream, std::ptr::null_mut())) };
        let track = unsafe { Box::from_raw(std::mem::replace(&mut self.track, std::ptr::null_mut())) };
        let end_t = self.end.load(Ordering::SeqCst);
        let end_t = if end_t > 0 { end_t } else { track.clock.now() };
        let end = end_t * AUDIO_RATE as i64 / HNS;
        let mut written = track.written.lock().unwrap();
        if end > *written {
            let gap = end - *written;
            track.silence(&mut written, gap);
        }
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        if !self.stream.is_null() {
            unsafe { ffi::bk_stream_stop(self.stream) };
        }
        if !self.track.is_null() {
            drop(unsafe { Box::from_raw(self.track) });
        }
    }
}

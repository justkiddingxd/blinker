//! MP4 writer on AVAssetWriter: one H.264 / HEVC video track (VideoToolbox,
//! the hardware encoder) plus any number of AAC audio tracks.

use crate::media::{Error, Result};
use crate::win::ffi;
use std::ffi::c_void;
use std::path::{Path, PathBuf};

pub const AUDIO_RATE: u32 = 48_000;
pub const AUDIO_CHANNELS: u32 = 2;

/// 100 ns units per second (the time base shared with the Windows build).
pub const HNS: i64 = 10_000_000;

/// How far ahead of the video the sound is fed when a file is written
/// offline: AVAssetWriter interleaves in about one-second chunks and holds
/// the video back until the sound has reached the next one.
pub const AUDIO_LEAD: i64 = 2 * HNS;

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Codec {
    H264,
    Hevc,
    /// No AV1 encoder in VideoToolbox: written as H.264.
    Av1,
}

/// A hardware H.265 encoder exists (checked once).
pub fn hevc_available() -> bool {
    static HEVC: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HEVC.get_or_init(|| unsafe { ffi::bk_hevc_available() != 0 })
}

#[allow(dead_code)]
pub fn has_codec(codec: Codec, _encoder: bool) -> bool {
    match codec {
        Codec::H264 => true,
        Codec::Hevc => hevc_available(),
        Codec::Av1 => false,
    }
}

#[derive(Clone, Copy)]
pub struct VideoSpec {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Mean bitrate, bits per second (unused in quality mode).
    pub bitrate: u32,
    pub codec: Codec,
    /// Constant quality 1..100 (like CRF): size follows how much moves.
    pub quality: Option<u32>,
    /// Keyframe interval (seeking / trimming precision).
    pub keyframe_secs: u32,
}

pub struct Encoder {
    native: *mut c_void,
    /// The file, for its metadata to go once it's done.
    path: std::path::PathBuf,
}

// The native writer serializes appends internally.
unsafe impl Send for Encoder {}
unsafe impl Sync for Encoder {}

fn fail(what: &str) -> Error {
    Error::new(what)
}

impl Encoder {
    /// `device`: frames come live from a capture (a slow frame is dropped
    /// rather than waited for). Without it, every frame is written.
    pub fn new(path: &Path, video: Option<&VideoSpec>, audio_tracks: usize, device: Option<&()>) -> Result<Self> {
        let p = ffi::cpath(path);
        let spec = video.map(|v| ffi::VideoSpec {
            width: v.width as i32,
            height: v.height as i32,
            fps: v.fps as i32,
            bitrate: v.bitrate.min(i32::MAX as u32) as i32,
            codec: (v.codec == Codec::Hevc) as i32,
            quality: v.quality.unwrap_or(0) as i32,
            keyframe_secs: v.keyframe_secs as i32,
        });
        let mut err = ffi::ErrBuf::new();
        let native = unsafe {
            ffi::bk_writer_new(
                p.as_ptr(),
                spec.as_ref().map_or(std::ptr::null(), |s| s as *const _),
                audio_tracks as i32,
                device.is_some() as i32,
                err.ptr(),
                ffi::ErrBuf::LEN,
            )
        };
        if native.is_null() {
            return Err(Error::new(err.text()));
        }
        Ok(Self { native, path: path.to_path_buf() })
    }

    /// One captured frame (BGRA pixel buffer, exactly the video size).
    pub fn write_texture(&self, frame: &super::video::Frame, t: i64, dur: i64) -> Result<()> {
        match unsafe { ffi::bk_writer_video_pixbuf(self.native, frame.as_ptr(), t, dur) } {
            0 => Err(fail("frame not written")),
            _ => Ok(()),
        }
    }

    /// One frame from memory: top-down BGRA rows of `width * 4` bytes.
    pub fn write_bgra(&self, data: &[u8], t: i64, dur: i64) -> Result<()> {
        match unsafe { ffi::bk_writer_video_bgra(self.native, data.as_ptr(), t, dur) } {
            0 => Err(fail("frame not written")),
            _ => Ok(()),
        }
    }

    /// Interleaved stereo 16-bit PCM at 48 kHz starting at `t`.
    pub fn write_audio(&self, track: usize, pcm: &[i16], t: i64) -> Result<()> {
        if pcm.is_empty() {
            return Ok(());
        }
        let frames = (pcm.len() / AUDIO_CHANNELS as usize) as i32;
        match unsafe { ffi::bk_writer_audio(self.native, track as i32, pcm.as_ptr(), frames, t) } {
            0 => Err(fail("audio not written")),
            _ => Ok(()),
        }
    }

    /// No more sound will come: AVAssetWriter stops holding the video back for it.
    pub fn end_audio(&self) -> Result<()> {
        unsafe { ffi::bk_writer_end_audio(self.native) };
        Ok(())
    }

    pub fn finish(&self) -> Result<()> {
        let mut err = ffi::ErrBuf::new();
        match unsafe { ffi::bk_writer_finish(self.native, err.ptr(), ffi::ErrBuf::LEN) } {
            0 => Err(Error::new(err.text())),
            _ => {
                crate::strip::file(&self.path);
                Ok(())
            }
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe { ffi::bk_writer_free(self.native) };
    }
}

fn paths(segs: &[(PathBuf, i64)]) -> (Vec<std::ffi::CString>, Vec<i64>) {
    (segs.iter().map(|(p, _)| ffi::cpath(p)).collect(), segs.iter().map(|(_, s)| *s).collect())
}

/// Instant replay: the stretch of `segs` (path, start on the replay clock) to
/// save: (first keyframe at or after `from`, length up to `to`).
pub fn scan_segments(segs: &[(PathBuf, i64)], from: i64, to: i64) -> Result<(i64, i64)> {
    let (names, starts) = paths(segs);
    let ptrs: Vec<_> = names.iter().map(|n| n.as_ptr()).collect();
    let (mut base, mut len) = (0i64, 0i64);
    let mut err = ffi::ErrBuf::new();
    let ok = unsafe {
        ffi::bk_scan_segments(ptrs.as_ptr(), starts.as_ptr(), ptrs.len() as i32, from, to, &mut base, &mut len, err.ptr(), ffi::ErrBuf::LEN)
    };
    if ok == 0 { Err(Error::new(err.text())) } else { Ok((base, len)) }
}

/// Writes `dst` from the segments' video (copied, not re-encoded) between
/// `base` and `end`, with one AAC track per `pcm` (stereo, starting at `base`).
pub fn remux_segments(segs: &[(PathBuf, i64)], dst: &Path, base: i64, end: i64, pcm: &[Vec<i16>]) -> Result<()> {
    let (names, starts) = paths(segs);
    let ptrs: Vec<_> = names.iter().map(|n| n.as_ptr()).collect();
    let d = ffi::cpath(dst);
    let tracks: Vec<*const i16> = pcm.iter().map(|p| p.as_ptr()).collect();
    let frames: Vec<usize> = pcm.iter().map(|p| p.len() / 2).collect();
    let mut err = ffi::ErrBuf::new();
    let ok = unsafe {
        ffi::bk_remux(
            ptrs.as_ptr(),
            starts.as_ptr(),
            ptrs.len() as i32,
            base,
            end,
            d.as_ptr(),
            tracks.as_ptr(),
            frames.as_ptr(),
            tracks.len() as i32,
            err.ptr(),
            ffi::ErrBuf::LEN,
        )
    };
    if ok == 0 { Err(Error::new(err.text())) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes 2 s of a moving gradient with a 440 Hz tone, reads it back.
    #[test]
    fn encoder_roundtrip() {
        let dir = std::env::temp_dir().join(format!("blinker-enc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.mp4");
        let (w, h, fps) = (320u32, 200u32, 30u32);
        let ntracks: usize = std::env::var("BK_TRACKS").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
        let qs: Vec<Option<u32>> = match std::env::var("BK_Q").as_deref() { Ok("none") => vec![None], Ok(_) => vec![Some(60)], _ => vec![Some(60), None] };
        for quality in qs {
            let spec = VideoSpec { width: w, height: h, fps, bitrate: 2_000_000, codec: Codec::H264, quality, keyframe_secs: 1 };
            let enc = Encoder::new(&path, Some(&spec), ntracks, None).expect("encoder");
            let frame_t = HNS / fps as i64;
            let chunk = (AUDIO_RATE / 10) as usize;
            let mut a = 0usize;
            for f in 0..(2 * fps) as i64 {
                let t = f * frame_t;
                while (a as i64) * HNS / AUDIO_RATE as i64 <= t + AUDIO_LEAD && a < 2 * AUDIO_RATE as usize {
                    let pcm: Vec<i16> = (0..chunk)
                        .flat_map(|i| {
                            let s = (((a + i) as f32 / AUDIO_RATE as f32 * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16;
                            [s, s]
                        })
                        .collect();
                    let at = a as i64 * HNS / AUDIO_RATE as i64;
                    for tr in 0..ntracks {
                        enc.write_audio(tr, &pcm, at).unwrap();
                    }
                    a += chunk;
                }
                if a >= 2 * AUDIO_RATE as usize {
                    enc.end_audio().unwrap();
                }
                let bgra: Vec<u8> = (0..w * h).flat_map(|i| [(i % w) as u8, (f * 4) as u8, (i / w) as u8, 255]).collect();
                if let Err(e) = enc.write_bgra(&bgra, t, frame_t) { panic!("frame {f} at {t}: {e}"); }
            }
            enc.finish().expect("finish");
            let info = crate::media::probe(&path).expect("probe");
            eprintln!("quality {quality:?}: {info:?} {} bytes", std::fs::metadata(&path).unwrap().len());
            assert_eq!((info.width, info.height), (w, h));
            assert_eq!(info.audio_tracks, 2);
            assert!((19_000_000..21_500_000).contains(&info.duration), "{}", info.duration);
            let (r, _) = crate::media::VideoReader::open(&path, 160, 100).unwrap();
            r.seek(HNS).unwrap();
            let (ts, img) = r.next().unwrap().expect("frame");
            assert!(ts >= HNS - frame_t, "{ts}");
            assert_eq!(img.dimensions(), (160, 100));
            let ar = crate::media::AudioReader::open(&path, info.audio_streams[1]).unwrap();
            let mut total = 0;
            while let Some((_, pcm)) = ar.next().unwrap() {
                total += pcm.len();
            }
            assert!(total > 2 * 90_000, "{total}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Writes a 4 s sample clip (moving bars, two tones on two tracks) to
    /// `BLINKER_TEST_SAMPLE`, for the export / GIF / replay tests.
    #[test]
    fn make_sample() {
        let Ok(path) = std::env::var("BLINKER_TEST_SAMPLE") else { return };
        let (w, h, fps) = (640u32, 360u32, 30u32);
        let spec = VideoSpec { width: w, height: h, fps, bitrate: 3_000_000, codec: Codec::H264, quality: Some(60), keyframe_secs: 1 };
        let enc = Encoder::new(Path::new(&path), Some(&spec), 2, None).expect("encoder");
        let (secs, frame_t, chunk) = (4usize, HNS / fps as i64, (AUDIO_RATE / 10) as usize);
        let mut a = 0usize;
        for f in 0..(secs as i64 * fps as i64) {
            let t = f * frame_t;
            while a < secs * AUDIO_RATE as usize && (a as i64) * HNS / AUDIO_RATE as i64 <= t + AUDIO_LEAD {
                for (tr, hz) in [(0usize, 440.0f32), (1, 660.0)] {
                    let pcm: Vec<i16> = (0..chunk)
                        .flat_map(|i| {
                            let s = (((a + i) as f32 / AUDIO_RATE as f32 * hz * std::f32::consts::TAU).sin() * 6000.0) as i16;
                            [s, s]
                        })
                        .collect();
                    enc.write_audio(tr, &pcm, a as i64 * HNS / AUDIO_RATE as i64).unwrap();
                }
                a += chunk;
            }
            if a >= secs * AUDIO_RATE as usize {
                enc.end_audio().unwrap();
            }
            let x0 = (f as u32 * 8) % w;
            let bgra: Vec<u8> = (0..w * h)
                .flat_map(|i| {
                    let (x, y) = (i % w, i / w);
                    let bar = (x + w - x0) % w < 80;
                    if bar { [40, 200, 240, 255] } else { [(y / 2) as u8, 60, (x / 3) as u8, 255] }
                })
                .collect();
            enc.write_bgra(&bgra, t, frame_t).unwrap();
        }
        enc.finish().unwrap();
        eprintln!("{:?}", crate::media::probe(Path::new(&path)).unwrap());
    }

    /// Three 2 s segments glued from 1.5 s to 5.5 s: starts at the keyframe
    /// at 2 s, copies the video, encodes the sound.
    #[test]
    fn replay_remux() {
        let dir = std::env::temp_dir().join(format!("blinker-remux-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h, fps) = (320u32, 192u32, 30u32);
        let spec = VideoSpec { width: w, height: h, fps, bitrate: 1_000_000, codec: Codec::H264, quality: None, keyframe_secs: 1 };
        let frame_t = HNS / fps as i64;
        let mut segs = Vec::new();
        for s in 0..3i64 {
            let path = dir.join(format!("seg-{s}.mp4"));
            let enc = Encoder::new(&path, Some(&spec), 0, None).unwrap();
            for f in 0..(2 * fps) as i64 {
                let shade = ((s * 60 + f) * 2) as u8;
                let bgra: Vec<u8> = (0..w * h).flat_map(|_| [shade, 128, 255 - shade, 255]).collect();
                enc.write_bgra(&bgra, f * frame_t, frame_t).unwrap();
            }
            enc.finish().unwrap();
            segs.push((path, s * 2 * HNS));
        }
        let (base, len) = scan_segments(&segs, 15 * HNS / 10, 55 * HNS / 10).unwrap();
        eprintln!("base {base} len {len}");
        assert!((base - 2 * HNS).abs() <= frame_t, "{base}");
        let pcm = vec![vec![1000i16; (len * AUDIO_RATE as i64 / HNS) as usize * 2]];
        let dst = dir.join("replay.mp4");
        remux_segments(&segs, &dst, base, base + len, &pcm).unwrap();
        let info = crate::media::probe(&dst).unwrap();
        eprintln!("{info:?}");
        assert_eq!(info.audio_tracks, 1);
        assert!((info.duration - len).abs() < HNS / 5, "{} vs {len}", info.duration);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Sound only, as MP3 (LAME, 192 kbit/s): interleaved stereo 16-bit PCM at
/// 48 kHz. `progress` gets 0..1.
pub fn write_mp3(dst: &Path, pcm: &[i16], progress: &dyn Fn(f32)) -> std::result::Result<(), String> {
    use mp3lame_encoder::{Bitrate, Builder, FlushNoGap, InterleavedPcm, Quality};
    let mut enc = Builder::new()
        .ok_or("lame")?
        .with_num_channels(AUDIO_CHANNELS as u8)
        .and_then(|b| b.with_sample_rate(AUDIO_RATE))
        .and_then(|b| b.with_brate(Bitrate::Kbps192))
        .and_then(|b| b.with_quality(Quality::Best))
        .and_then(|b| b.build())
        .map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(pcm.len() / 3);
    let chunk = AUDIO_RATE as usize * AUDIO_CHANNELS as usize; // one second
    let n = pcm.len().div_ceil(chunk).max(1);
    for (i, c) in pcm.chunks(chunk).enumerate() {
        out.reserve(mp3lame_encoder::max_required_buffer_size(c.len()));
        enc.encode_to_vec(InterleavedPcm(c), &mut out).map_err(|e| e.to_string())?;
        progress((i + 1) as f32 / n as f32);
    }
    out.reserve(7200);
    enc.flush_to_vec::<FlushNoGap>(&mut out).map_err(|e| e.to_string())?;
    std::fs::write(dst, out).map_err(|e| e.to_string())?;
    crate::strip::file(dst);
    Ok(())
}

//! Audio from WASAPI: the desktop mix (loopback of the default output) and the
//! default microphone, each into its own track. Windows converts both to
//! 48 kHz stereo 16-bit for us (AUTOCONVERTPCM).

use super::encoder::{AUDIO_CHANNELS, AUDIO_RATE, HNS};
use super::{AudioSink, Clock};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::thread::JoinHandle;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::core::Result;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Desktop,
    Mic,
}

struct Client {
    client: IAudioClient,
    capture: IAudioCaptureClient,
}

fn open(source: Source) -> Result<Client> {
    crate::media::startup();
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let flow = if source == Source::Desktop { eRender } else { eCapture };
        let dev = en.GetDefaultAudioEndpoint(flow, eConsole)?;
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
        let fmt = WAVEFORMATEX {
            wFormatTag: 1, // WAVE_FORMAT_PCM
            nChannels: AUDIO_CHANNELS as u16,
            nSamplesPerSec: AUDIO_RATE,
            nAvgBytesPerSec: AUDIO_RATE * AUDIO_CHANNELS * 2,
            nBlockAlign: (AUDIO_CHANNELS * 2) as u16,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        let mut flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        if source == Source::Desktop {
            flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
        }
        // 300 ms buffer; we drain it every 10 ms.
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 3_000_000, 0, &fmt, None)?;
        let capture: IAudioCaptureClient = client.GetService()?;
        Ok(Client { client, capture })
    }
}

/// Probes whether a source can be opened (device present).
pub fn available(source: Source) -> bool {
    open(source).is_ok()
}

pub struct AudioCapture {
    thread: JoinHandle<()>,
}

impl AudioCapture {
    /// Starts capturing into `track` of `enc`. Gaps (loopback produces no
    /// packets while nothing plays) are filled with silence, so the track
    /// stays in sync with the video clock `t0`.
    /// `end`: 0 while recording; set to the stop time (recording clock) to finish.
    pub fn start(source: Source, sink: Arc<dyn AudioSink>, track: usize, clock: Arc<Clock>, end: Arc<AtomicI64>) -> Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let c = match open(source) {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            if let Err(e) = unsafe { c.client.Start() } {
                let _ = tx.send(Err(e));
                return;
            }
            let _ = tx.send(Ok(()));
            run(&c, sink.as_ref(), track, &clock, &end);
            unsafe {
                let _ = c.client.Stop();
            }
        });
        rx.recv().unwrap_or(Err(windows::core::Error::empty()))?;
        Ok(Self { thread })
    }

    pub fn join(self) {
        let _ = self.thread.join();
    }
}

/// Frames → 100 ns.
fn frames_to_hns(f: i64) -> i64 {
    f * HNS / AUDIO_RATE as i64
}

fn run(c: &Client, enc: &dyn AudioSink, track: usize, clock: &Clock, end_at: &AtomicI64) {
    let ch = AUDIO_CHANNELS as usize;
    let mut written: i64 = 0; // frames on the track so far
    let mut buf: Vec<i16> = Vec::with_capacity(AUDIO_RATE as usize / 10 * ch);
    let silence = |enc: &dyn AudioSink, written: &mut i64, frames: i64| {
        let mut left = frames;
        while left > 0 {
            let n = left.min(AUDIO_RATE as i64 / 10);
            let z = vec![0i16; n as usize * ch];
            enc.write_audio(track, &z, frames_to_hns(*written));
            *written += n;
            left -= n;
        }
    };
    loop {
        let end_t = end_at.load(Ordering::SeqCst);
        unsafe {
            while let Ok(n) = c.capture.GetNextPacketSize() {
                if n == 0 {
                    break;
                }
                let mut data = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                let mut qpc = 0u64;
                if c.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc)).is_err() {
                    break;
                }
                // Packet start on the recording clock; dropped while paused.
                let Some(at) = clock.map(qpc as i64) else {
                    let _ = c.capture.ReleaseBuffer(frames);
                    continue;
                };
                let mut skip = 0i64;
                if at < 0 {
                    skip = (-at * AUDIO_RATE as i64 / HNS).min(frames as i64);
                }
                let start_frame = (at.max(0) * AUDIO_RATE as i64) / HNS;
                // Fill a gap of more than 20 ms with silence.
                if start_frame > written + AUDIO_RATE as i64 / 50 {
                    let gap = start_frame - written;
                    silence(enc, &mut written, gap);
                }
                let count = frames as i64 - skip;
                if count > 0 {
                    buf.clear();
                    if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                        buf.resize(count as usize * ch, 0);
                    } else {
                        let src = std::slice::from_raw_parts(data as *const i16, frames as usize * ch);
                        buf.extend_from_slice(&src[skip as usize * ch..]);
                    }
                    enc.write_audio(track, &buf, frames_to_hns(written));
                    written += count;
                }
                let _ = c.capture.ReleaseBuffer(frames);
            }
        }
        if end_t > 0 {
            // Pad to the stop moment so the track is as long as the video.
            let end = end_t * AUDIO_RATE as i64 / HNS;
            if end > written {
                let gap = end - written;
                silence(enc, &mut written, gap);
            }
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

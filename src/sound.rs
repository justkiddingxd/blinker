//! Capture sound. Built-in sounds are synthesized (no bundled files); a custom
//! file (wav / mp3 / ogg / flac) is decoded by rodio. The audio device is only
//! opened while a sound plays.

use crate::config::{Config, SoundKind};
use rodio::Source;
use std::num::NonZero;
use std::path::PathBuf;
use std::time::Duration;

const RATE: u32 = 44_100;

/// Plays the configured capture sound, if enabled.
pub fn play_capture(cfg: &Config) {
    if cfg.sound_enabled {
        play(cfg.sound_kind, cfg.sound_file.clone(), cfg.sound_volume);
    }
}

/// The default output. rodio asks for a ~50 ms buffer; some devices (MacBook
/// speakers: 15..=960 frames) refuse that, then the device picks its own.
fn open_sink() -> Option<rodio::MixerDeviceSink> {
    let first = match rodio::DeviceSinkBuilder::open_default_sink() {
        Ok(sink) => return Some(sink),
        Err(e) => e,
    };
    match rodio::DeviceSinkBuilder::from_default_device().and_then(|b| b.with_buffer_size(rodio::cpal::BufferSize::Default).open_stream()) {
        Ok(sink) => Some(sink),
        Err(e) => {
            crate::config::log::error(format!("sound: {first}; {e}"));
            None
        }
    }
}

/// Fire-and-forget playback on a short-lived thread.
pub fn play(kind: SoundKind, file: Option<PathBuf>, volume: f32) {
    std::thread::spawn(move || {
        let Some(mut sink) = open_sink() else { return };
        sink.log_on_drop(false);
        let player = rodio::Player::connect_new(sink.mixer());
        player.set_volume(volume.clamp(0.0, 1.0));
        match kind {
            SoundKind::Custom => {
                let Some(path) = file else { return };
                let Ok(f) = std::fs::File::open(path) else { return };
                let Ok(dec) = rodio::Decoder::try_from(f) else { return };
                // Long files are cut: this is a shutter sound, not a playlist.
                player.append(dec.take_duration(Duration::from_secs(4)));
            }
            k => {
                let samples = synth(k);
                player.append(rodio::buffer::SamplesBuffer::new(
                    NonZero::new(1).unwrap(),
                    NonZero::new(RATE).unwrap(),
                    samples,
                ));
            }
        }
        player.sleep_until_end();
        std::thread::sleep(Duration::from_millis(60));
    });
}

/// Tiny deterministic noise source (no rand dependency needed).
struct Noise(u32);
impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn synth(kind: SoundKind) -> Vec<f32> {
    let sr = RATE as f32;
    let tau = std::f32::consts::TAU;
    let n = |secs: f32| (secs * sr) as usize;
    match kind {
        // Two short mechanical clacks: shutter opening and closing.
        SoundKind::Shutter => {
            let mut out = vec![0.0; n(0.16)];
            let mut noise = Noise(0x9E37_79B9);
            let mut lp = 0.0;
            for (start, len, tone, gain) in [(0.0, 0.028, 2400.0, 0.55), (0.062, 0.05, 1500.0, 0.75)] {
                let s0 = n(start);
                for i in 0..n(len) {
                    let t = i as f32 / sr;
                    let env = (-t * 120.0).exp() * (1.0 - (-t * 4000.0).exp());
                    let raw = noise.next();
                    lp += (raw - lp) * 0.35; // soften the hiss
                    let body = (tau * tone * t).sin() * 0.35;
                    out[s0 + i] += (lp * 0.8 + body) * env * gain;
                }
            }
            out
        }
        // Very short, dry tick.
        SoundKind::Click => (0..n(0.035))
            .map(|i| {
                let t = i as f32 / sr;
                let env = (-t * 220.0).exp() * (1.0 - (-t * 6000.0).exp());
                ((tau * 2100.0 * t).sin() * 0.7 + (tau * 4300.0 * t).sin() * 0.2) * env
            })
            .collect(),
        // Two soft rising notes.
        SoundKind::Blink => {
            let mut out = vec![0.0; n(0.2)];
            for (start, freq) in [(0.0, 880.0), (0.07, 1318.5)] {
                let s0 = n(start);
                for i in 0..n(0.12) {
                    if s0 + i >= out.len() {
                        break;
                    }
                    let t = i as f32 / sr;
                    let env = (-t * 28.0).exp() * (1.0 - (-t * 900.0).exp());
                    out[s0 + i] += (tau * freq * t).sin() * env * 0.45;
                }
            }
            out
        }
        // Falling "drop" sweep.
        SoundKind::Drop => {
            let mut phase = 0.0f32;
            (0..n(0.12))
                .map(|i| {
                    let t = i as f32 / sr;
                    let f = 1300.0 * (-t * 9.0).exp() + 420.0;
                    phase += tau * f / sr;
                    let env = (-t * 26.0).exp() * (1.0 - (-t * 1500.0).exp());
                    phase.sin() * env * 0.6
                })
                .collect()
        }
        SoundKind::Custom => Vec::new(),
    }
}

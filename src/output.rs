//! Encoding to disk and clipboard.

use crate::config::{Config, ImageFormat, log};
use image::{ExtendedColorType, ImageEncoder, RgbaImage};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

static PENDING: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());

pub fn new_path(cfg: &Config) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
    let ext = cfg.format.extension();
    let dir = cfg.pics_dir();
    let mut path = dir.join(format!("blinker {stamp}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("blinker {stamp} ({n}).{ext}"));
        n += 1;
    }
    path
}

/// Encodes on a background thread. The path is final immediately; the flag
/// flips to true once the file is on disk.
pub fn save_async(img: Arc<RgbaImage>, cfg: &Config) -> (PathBuf, Arc<AtomicBool>) {
    let path = new_path(cfg);
    let (format, quality) = (cfg.format, cfg.jpeg_quality);
    let p = path.clone();
    let done = Arc::new(AtomicBool::new(false));
    let done2 = done.clone();
    let handle = std::thread::spawn(move || {
        if let Err(e) = write(&img, &p, format, quality) {
            log::error(format!("save failed: {e}"));
        }
        done2.store(true, Ordering::Release);
    });
    let mut pending = PENDING.lock().unwrap();
    pending.retain(|h| !h.is_finished());
    pending.push(handle);
    (path, done)
}

/// Blocks until background saves are done (called before exit).
pub fn wait_pending() {
    let handles: Vec<_> = std::mem::take(&mut *PENDING.lock().unwrap());
    for h in handles {
        let _ = h.join();
    }
}

pub fn write(img: &RgbaImage, path: &Path, format: ImageFormat, jpeg_quality: u8) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("partial");
    let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut w = BufWriter::new(file);
    let (width, height) = img.dimensions();
    // Screenshots are opaque: dropping alpha saves ~10-25% after compression.
    let opaque = img.pixels().all(|p| p.0[3] == 255);
    let rgb = || -> Vec<u8> { img.pixels().flat_map(|p| [p.0[0], p.0[1], p.0[2]]).collect() };
    let res = match format {
        ImageFormat::Png => {
            let enc = image::codecs::png::PngEncoder::new_with_quality(
                &mut w,
                image::codecs::png::CompressionType::Best,
                image::codecs::png::FilterType::Adaptive,
            );
            if opaque {
                enc.write_image(&rgb(), width, height, ExtendedColorType::Rgb8)
            } else {
                enc.write_image(img.as_raw(), width, height, ExtendedColorType::Rgba8)
            }
        }
        ImageFormat::Webp => {
            let enc = image::codecs::webp::WebPEncoder::new_lossless(&mut w);
            if opaque {
                enc.write_image(&rgb(), width, height, ExtendedColorType::Rgb8)
            } else {
                enc.write_image(img.as_raw(), width, height, ExtendedColorType::Rgba8)
            }
        }
        ImageFormat::Jpeg => {
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, jpeg_quality.clamp(30, 100));
            enc.write_image(&rgb(), width, height, ExtendedColorType::Rgb8)
        }
    };
    res.map_err(|e| e.to_string())?;
    drop(w);
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    crate::strip::file(path);
    Ok(())
}

/// Puts a file on the clipboard (paste into Explorer, Telegram, Discord...).
pub fn copy_file(path: &Path) -> bool {
    match arboard::Clipboard::new().and_then(|mut c| c.set().file_list(&[path])) {
        Ok(()) => true,
        Err(e) => {
            log::error(format!("clipboard: {e}"));
            false
        }
    }
}

pub fn copy_image(img: &RgbaImage) -> bool {
    let data = arboard::ImageData {
        width: img.width() as usize,
        height: img.height() as usize,
        bytes: std::borrow::Cow::Borrowed(img.as_raw()),
    };
    match arboard::Clipboard::new().and_then(|mut c| c.set_image(data)) {
        Ok(()) => true,
        Err(e) => {
            log::error(format!("clipboard: {e}"));
            false
        }
    }
}

pub fn copy_text(text: &str) {
    if let Ok(mut c) = arboard::Clipboard::new() {
        let _ = c.set_text(text.to_owned());
    }
}

pub fn load(path: &Path) -> Option<RgbaImage> {
    image::open(path).ok().map(|i| i.to_rgba8())
}

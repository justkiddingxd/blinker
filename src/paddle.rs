//! Text recognition on Windows: PaddleOCR (PP-OCRv5), the two small models
//! PaddlePaddle publishes in ONNX: one finds the lines of text in a picture,
//! the other reads each of them (Russian, Ukrainian, Belarusian and English
//! in one go). Windows ML, part of Windows, runs them, so blinker carries no
//! runtime of its own. The models (13 MB) are downloaded once, in settings or
//! from the text window, from PaddlePaddle's page on Hugging Face and checked
//! by their SHA-256.

#[cfg(windows)]
pub use read::recognize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// A model file: its name here, where it comes from, its size and hash.
struct Part {
    file: &'static str,
    url: &'static str,
    size: u64,
    sha256: &'static str,
}

const DETECT: Part = Part {
    file: "det.onnx",
    url: "https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_det_onnx/resolve/main/inference.onnx",
    size: 4_826_518,
    sha256: "a431985659dc921974177a95adcfbb90fd9e51989a5e04d70d0b75f597b6e61d",
};

const READ: Part = Part {
    file: "rec-eslav.onnx",
    url: "https://huggingface.co/PaddlePaddle/eslav_PP-OCRv5_mobile_rec_onnx/resolve/main/inference.onnx",
    size: 7_887_627,
    sha256: "b3018ef2b09a0250b6e0c8e871c927098363e5fd4df890cc68e8358eb0aaf1bd",
};

const PARTS: [Part; 2] = [DETECT, READ];

/// All of it, in megabytes (for the buttons).
pub const SIZE_MB: u64 = 13;

fn path(part: &Part) -> PathBuf {
    crate::config::cache_dir().join("ocr").join(part.file)
}

/// The models are here.
pub fn ready() -> bool {
    PARTS.iter().all(|p| std::fs::metadata(path(p)).is_ok_and(|m| m.len() == p.size))
}

/// Deletes the models.
pub fn remove() {
    for p in &PARTS {
        let _ = std::fs::remove_file(path(p));
    }
}

/// A download of the models.
pub struct Install {
    /// 0..=1000 of all of it.
    pub progress: AtomicU32,
    pub result: Mutex<Option<Result<(), String>>>,
}

impl Install {
    pub fn fraction(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }
}

/// Downloads what's missing in the background; `ctx` repaints as it goes.
pub fn install(ctx: &eframe::egui::Context) -> Arc<Install> {
    let job = Arc::new(Install { progress: AtomicU32::new(0), result: Mutex::new(None) });
    let (j, ctx) = (job.clone(), ctx.clone());
    std::thread::spawn(move || {
        let r = fetch(&j, &ctx);
        *j.result.lock().unwrap() = Some(r);
        ctx.request_repaint();
    });
    job
}

fn fetch(job: &Install, ctx: &eframe::egui::Context) -> Result<(), String> {
    use sha2::Digest;
    let total: u64 = PARTS.iter().map(|p| p.size).sum();
    let mut before = 0;
    for part in &PARTS {
        let dst = path(part);
        if std::fs::metadata(&dst).is_ok_and(|m| m.len() == part.size) {
            before += part.size;
            continue;
        }
        std::fs::create_dir_all(dst.parent().ok_or("no folder")?).map_err(|e| e.to_string())?;
        let tmp = dst.with_extension("part");
        let _ = std::fs::remove_file(&tmp);
        crate::update::download(part.url, &tmp, part.size, |got| {
            job.progress.store(((before + got) * 1000 / total).min(1000) as u32, Ordering::Relaxed);
            ctx.request_repaint();
        })
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        let mut hash = sha2::Sha256::new();
        let mut f = std::fs::File::open(&tmp).map_err(|e| e.to_string())?;
        std::io::copy(&mut f, &mut hash).map_err(|e| e.to_string())?;
        drop(f);
        let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if got != part.sha256 {
            let _ = std::fs::remove_file(&tmp);
            return Err(crate::i18n::l("модель скачалась с ошибкой, попробуй ещё раз", "the model came down damaged, try again").into());
        }
        std::fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
        before += part.size;
    }
    Ok(())
}

/// Reading with the models, by Windows ML.
#[cfg(windows)]
mod read {
    use super::{DETECT, Part, READ, path, ready};
    use crate::ocr::Line;
    use image::RgbaImage;
    use windows::AI::MachineLearning::{LearningModel, LearningModelBinding, LearningModelDevice, LearningModelDeviceKind, LearningModelSession, TensorFloat};
    use windows::core::{HSTRING, Interface};

    /// The reading model's letters, in the order of its outputs (after the CTC
    /// blank; a space comes last). From the model's own inference.yml.
    const LETTERS: &str = include_str!("../assets/ocr/eslav_dict.txt");

    /// A model loaded into Windows ML, on the processor.
    struct Net {
        session: LearningModelSession,
    }

    impl Net {
        fn load(part: &Part) -> windows::core::Result<Self> {
            let model = LearningModel::LoadFromFilePath(&HSTRING::from(path(part).as_os_str()))?;
            let device = LearningModelDevice::Create(LearningModelDeviceKind::Cpu)?;
            Ok(Self { session: LearningModelSession::CreateFromModelOnDevice(&model, &device)? })
        }

        /// One picture in (`shape`: 1, 3, h, w), the model's one output back with
        /// its shape.
        fn run(&self, shape: [i64; 4], data: &[f32]) -> windows::core::Result<(Vec<i64>, Vec<f32>)> {
            let binding = LearningModelBinding::CreateFromSession(&self.session)?;
            let input = TensorFloat::CreateFromArray(&windows_collections::IIterable::<i64>::from(shape.to_vec()), data)?;
            binding.Bind(&HSTRING::from("x"), &input)?;
            let result = self.session.Evaluate(&binding, &HSTRING::new())?;
            let out = result.Outputs()?.Lookup(&HSTRING::from("fetch_name_0"))?.cast::<TensorFloat>()?;
            let shape: Vec<i64> = out.Shape()?.into_iter().collect();
            let view = out.GetAsVectorView()?;
            let mut values = vec![0f32; view.Size()? as usize];
            view.GetMany(0, &mut values)?;
            Ok((shape, values))
        }
    }

    /// The text in `img`: its lines where they are, in reading order.
    pub fn recognize(img: &RgbaImage) -> Result<Vec<Line>, String> {
        if !ready() {
            return Err(crate::i18n::l("нет модели распознавания", "the recognition model isn't installed").into());
        }
        let e = |e: windows::core::Error| e.message().to_string();
        unsafe {
            use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        let (det, rec) = std::thread::scope(|s| {
            let det = s.spawn(|| Net::load(&DETECT));
            let rec = Net::load(&READ);
            (det.join().unwrap_or_else(|_| Err(windows::core::Error::empty())), rec)
        });
        let (det, rec) = (det.map_err(e)?, rec.map_err(e)?);
        let letters: Vec<char> = LETTERS.lines().filter_map(|l| l.chars().next()).collect();
        let mut out = Vec::new();
        for b in find_lines(&det, img).map_err(e)? {
            let crop = image::imageops::crop_imm(img, b.0, b.1, b.2, b.3).to_image();
            if let Some(text) = read_line(&rec, &crop, &letters).map_err(e)? {
                let (x, y, w, h) = b.4;
                out.push(Line { text: crate::ocr::one_script(&text), x, y, w, h });
            }
        }
        crate::ocr::sort(&mut out);
        Ok(out)
    }

    /// `img` as the models take it: blue, green, red planes, each `(v / 255 -
    /// mean) / std`, `width` wide. Paddle's ImageNet mean and std go on the
    /// planes in its own order, blue first, as in PaddleOCR.
    fn planes(img: &RgbaImage, mean: [f32; 3], std: [f32; 3], width: u32) -> Vec<f32> {
        let (h, width) = (img.height() as usize, width as usize);
        let mut out = vec![0f32; 3 * h * width];
        for (x, y, p) in img.enumerate_pixels() {
            let (x, y) = (x as usize, y as usize);
            for (c, ch) in [2usize, 1, 0].into_iter().enumerate() {
                out[c * h * width + y * width + x] = (p[ch] as f32 / 255.0 - mean[c]) / std[c];
            }
        }
        // beyond the picture (a short line padded to the width): zero, as Paddle pads
        out
    }

    /// A line found: the box to read (x, y, width, height in pixels), with a
    /// margin, and the snugger one the text sits in (for showing it).
    type Found = (u32, u32, u32, u32, (f32, f32, f32, f32));

    /// The text lines in `img`: the detector's map of where text is, its blobs,
    /// each grown back by a margin as PaddleOCR does ("unclip"), the faint ones
    /// dropped.
    fn find_lines(det: &Net, img: &RgbaImage) -> windows::core::Result<Vec<Found>> {
        let (w, h) = (img.width(), img.height());
        // the detector wants sides of a multiple of 32; huge pictures go smaller,
        // tiny ones bigger
        let long = w.max(h) as f32;
        let k = if long > 2560.0 { 2560.0 / long } else if (w.min(h) as f32) < 64.0 { 64.0 / w.min(h) as f32 } else { 1.0 };
        let round = |v: f32| ((v / 32.0).round() as u32).max(1) * 32;
        let (dw, dh) = (round(w as f32 * k), round(h as f32 * k));
        let scaled = if (dw, dh) == (w, h) { img.clone() } else { image::imageops::resize(img, dw, dh, image::imageops::FilterType::Triangle) };
        let data = planes(&scaled, [0.485, 0.456, 0.406], [0.229, 0.224, 0.225], dw);
        let (shape, map) = det.run([1, 3, dh as i64, dw as i64], &data)?;
        let (mh, mw) = (shape[2] as usize, shape[3] as usize);
        let (sx, sy) = (w as f32 / mw as f32, h as f32 / mh as f32);
        // blobs of the map above 0.3, 8-connected
        let mut seen = vec![false; mw * mh];
        let mut boxes = Vec::new();
        let mut stack = Vec::new();
        for start in 0..mw * mh {
            if seen[start] || map[start] <= 0.3 {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
            seen[start] = true;
            stack.push(start);
            while let Some(i) = stack.pop() {
                let (x, y) = (i % mw, i / mw);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                        if nx < 0 || ny < 0 || nx >= mw as i64 || ny >= mh as i64 {
                            continue;
                        }
                        let j = ny as usize * mw + nx as usize;
                        if !seen[j] && map[j] > 0.3 {
                            seen[j] = true;
                            stack.push(j);
                        }
                    }
                }
            }
            let (bw, bh) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
            if bw.min(bh) < 3.0 {
                continue;
            }
            // sure enough: the mean of the map over its box
            let mut sum = 0.0;
            for y in y0..=y1 {
                sum += map[y * mw + x0..=y * mw + x1].iter().sum::<f32>();
            }
            if sum / (bw * bh) < 0.6 {
                continue;
            }
            // the detector draws text shrunk: grown back by area × 1.5 / perimeter
            let grow = bw * bh * 1.5 / (2.0 * (bw + bh));
            let (fx0, fy0) = (((x0 as f32 - grow) * sx).max(0.0), ((y0 as f32 - grow) * sy).max(0.0));
            let (fx1, fy1) = (((x1 as f32 + 1.0 + grow) * sx).min(w as f32), ((y1 as f32 + 1.0 + grow) * sy).min(h as f32));
            // the text itself: grown less (the detector shrinks it by about this)
            let snug = bw * bh * 0.85 / (2.0 * (bw + bh));
            let (tx0, ty0) = (((x0 as f32 - snug) * sx).max(0.0), ((y0 as f32 - snug) * sy).max(0.0));
            let (tx1, ty1) = (((x1 as f32 + 1.0 + snug) * sx).min(w as f32), ((y1 as f32 + 1.0 + snug) * sy).min(h as f32));
            let b = (fx0 as u32, fy0 as u32, (fx1 - fx0) as u32, (fy1 - fy0) as u32, (tx0, ty0, tx1 - tx0, ty1 - ty0));
            if b.2 >= 2 && b.3 >= 2 {
                boxes.push(b);
            }
        }
        Ok(boxes)
    }

    /// The text of one line (`crop`), or nothing when the reader isn't sure.
    fn read_line(rec: &Net, crop: &RgbaImage, letters: &[char]) -> windows::core::Result<Option<String>> {
        const H: u32 = 48;
        let w = ((crop.width() as f32 * H as f32 / crop.height().max(1) as f32).round() as u32).clamp(1, 3200);
        let resized = image::imageops::resize(crop, w, H, image::imageops::FilterType::Triangle);
        // at least 320 wide, the rest empty
        let width = w.max(320);
        let data = planes(&resized, [0.5; 3], [0.5; 3], width);
        let (shape, probs) = rec.run([1, 3, H as i64, width as i64], &data)?;
        let (steps, classes) = (shape[1] as usize, shape[2] as usize);
        // CTC: the likeliest class at each step, repeats and blanks dropped
        let mut text = String::new();
        let (mut conf, mut n, mut prev) = (0.0f32, 0, 0usize);
        for t in 0..steps {
            let row = &probs[t * classes..(t + 1) * classes];
            let (best, p) = row.iter().enumerate().fold((0, f32::MIN), |a, (i, &p)| if p > a.1 { (i, p) } else { a });
            if best != 0 && best != prev {
                text.push(letters.get(best - 1).copied().unwrap_or(' '));
                conf += p;
                n += 1;
            }
            prev = best;
        }
        let text = text.trim().to_string();
        // a stray mark ("." in a chart) is no text
        if n == 0 || !text.chars().any(char::is_alphanumeric) || conf / (n as f32) < 0.5 {
            return Ok(None);
        }
        Ok(Some(text))
    }
}

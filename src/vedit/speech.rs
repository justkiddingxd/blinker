//! Subtitles from speech: whisper.cpp on this computer (no service, no
//! upload). The model is fetched once, on the first use, from the
//! whisper.cpp project's page on Hugging Face and checked by its SHA-256.

use super::shorts::Caption;
use crate::record::encoder::{AUDIO_CHANNELS, AUDIO_RATE, HNS};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Which whisper model: quicker and lighter, or the one that hears better.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Model {
    #[default]
    Small,
    Turbo,
}

impl Model {
    pub const ALL: [Model; 2] = [Model::Small, Model::Turbo];

    fn file(self) -> &'static str {
        match self {
            Model::Small => "ggml-small-q5_1.bin",
            Model::Turbo => "ggml-large-v3-turbo-q5_0.bin",
        }
    }

    fn size(self) -> u64 {
        match self {
            Model::Small => 190_085_487,
            Model::Turbo => 574_041_195,
        }
    }

    fn sha256(self) -> &'static str {
        match self {
            Model::Small => "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
            Model::Turbo => "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Model::Small => crate::i18n::l("быстрее · 190 мб", "quicker · 190 mb"),
            Model::Turbo => crate::i18n::l("точнее · 570 мб", "more accurate · 570 mb"),
        }
    }

    pub fn desc(self) -> &'static str {
        match self {
            Model::Small => crate::i18n::l("whisper small: чёткую речь пишет хорошо, путается в шуме", "whisper small: clear speech comes out well, noise confuses it"),
            Model::Turbo => crate::i18n::l("whisper large-v3-turbo: почти без ошибок, но в несколько раз дольше", "whisper large-v3-turbo: hardly a mistake, but several times slower"),
        }
    }

    pub fn path(self) -> PathBuf {
        crate::config::cache_dir().join("whisper").join(self.file())
    }

    /// Downloaded already.
    pub fn ready(self) -> bool {
        std::fs::metadata(self.path()).is_ok_and(|m| m.len() == self.size())
    }

    fn url(self) -> String {
        format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}", self.file())
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Stage {
    Download,
    Listen,
}

pub struct Job {
    /// 0..=1000 of the current stage.
    pub progress: AtomicU32,
    pub stage: Mutex<Stage>,
    pub cancel: AtomicBool,
    pub result: Mutex<Option<Result<Vec<Caption>, String>>>,
}

impl Job {
    pub fn fraction(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }
}

/// Installs `model` alone (from the settings): the job ends with no lines.
pub fn install(ctx: &eframe::egui::Context, model: Model) -> Arc<Job> {
    let job = Arc::new(Job { progress: AtomicU32::new(0), stage: Mutex::new(Stage::Download), cancel: AtomicBool::new(false), result: Mutex::new(None) });
    let (j, ctx) = (job.clone(), ctx.clone());
    std::thread::spawn(move || {
        let r = if model.ready() { Ok(()) } else { fetch(model, &j, &ctx) };
        *j.result.lock().unwrap() = Some(r.map(|_| Vec::new()));
        ctx.request_repaint();
    });
    job
}

/// Deletes a downloaded model.
pub fn remove(model: Model) {
    let _ = std::fs::remove_file(model.path());
}

/// Recognizes the speech in `from..to` of `src`: the `tracks` (stream, gain)
/// mixed, as subtitles in source time. Fetches the model first if needed.
pub fn start(ctx: &eframe::egui::Context, model: Model, src: PathBuf, tracks: Vec<(u32, f32)>, from: i64, to: i64) -> Arc<Job> {
    let job = Arc::new(Job { progress: AtomicU32::new(0), stage: Mutex::new(Stage::Listen), cancel: AtomicBool::new(false), result: Mutex::new(None) });
    let (j, ctx) = (job.clone(), ctx.clone());
    std::thread::spawn(move || {
        crate::media::startup();
        let r = run(&j, &ctx, model, &src, &tracks, from, to);
        let r = if j.cancel.load(Ordering::SeqCst) { Err(crate::i18n::t("отменено").to_string()) } else { r };
        *j.result.lock().unwrap() = Some(r);
        ctx.request_repaint();
    });
    job
}

fn run(job: &Arc<Job>, ctx: &eframe::egui::Context, model: Model, src: &Path, tracks: &[(u32, f32)], from: i64, to: i64) -> Result<Vec<Caption>, String> {
    if tracks.is_empty() {
        return Err(crate::i18n::l("все дорожки звука выключены", "every sound track is off").into());
    }
    if !model.ready() {
        *job.stage.lock().unwrap() = Stage::Download;
        fetch(model, job, ctx)?;
    }
    if job.cancel.load(Ordering::SeqCst) {
        return Ok(Vec::new());
    }
    *job.stage.lock().unwrap() = Stage::Listen;
    job.progress.store(0, Ordering::Relaxed);
    ctx.request_repaint();
    let pcm = speech_audio(src, tracks, from, to)?;
    transcribe(model, &pcm, job, ctx).map(|segs| {
        segs.into_iter().map(|(a, b, text)| Caption { from: from + a, to: (from + b).min(to), text }).filter(|c| c.to > c.from).collect()
    })
}

/// The model into the cache folder: downloaded next to it, checked, renamed.
fn fetch(model: Model, job: &Arc<Job>, ctx: &eframe::egui::Context) -> Result<(), String> {
    use sha2::Digest;
    let path = model.path();
    let dir = path.parent().ok_or("no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let part = path.with_extension("part");
    let _ = std::fs::remove_file(&part);
    let size = model.size();
    crate::update::download(&model.url(), &part, size, |got| {
        job.progress.store((got * 1000 / size.max(1)).min(1000) as u32, Ordering::Relaxed);
        ctx.request_repaint();
    })
    .inspect_err(|_| {
        let _ = std::fs::remove_file(&part);
    })?;
    let mut hash = sha2::Sha256::new();
    let mut f = std::fs::File::open(&part).map_err(|e| e.to_string())?;
    std::io::copy(&mut f, &mut hash).map_err(|e| e.to_string())?;
    let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != model.sha256() {
        let _ = std::fs::remove_file(&part);
        return Err(crate::i18n::l("модель скачалась с ошибкой, попробуй ещё раз", "the model came down damaged, try again").into());
    }
    std::fs::rename(&part, &path).map_err(|e| e.to_string())
}

/// What whisper hears: the tracks mixed to mono, 16 kHz, -1..1.
fn speech_audio(src: &Path, tracks: &[(u32, f32)], from: i64, to: i64) -> Result<Vec<f32>, String> {
    let total = ((to - from).max(0) as i128 * AUDIO_RATE as i128 / HNS as i128) as usize * AUDIO_CHANNELS as usize;
    let mut mix = vec![0f32; total / 2];
    for (stream, gain) in tracks {
        let pcm = super::export::read_range(src, *stream, from, total).map_err(|e| e.message().to_string())?;
        for (m, lr) in mix.iter_mut().zip(pcm.chunks_exact(2)) {
            *m += (lr[0] as f32 + lr[1] as f32) * 0.5 / 32768.0 * gain;
        }
    }
    // 48 kHz to 16: the mean of every three (a gentle low-pass on the way)
    let step = (AUDIO_RATE / 16_000).max(1) as usize;
    let mut out: Vec<f32> = mix.chunks(step).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    // a quiet microphone brought up: its loud parts near full scale
    let mut peaks: Vec<f32> = out.iter().step_by(7).map(|v| v.abs()).collect();
    if !peaks.is_empty() {
        let at = ((peaks.len() as f32 * 0.999) as usize).min(peaks.len() - 1);
        let top = *peaks.select_nth_unstable_by(at, |a, b| a.total_cmp(b)).1;
        let gain = (0.8 / top.max(1e-4)).clamp(1.0, 30.0);
        for v in &mut out {
            *v = (*v * gain).clamp(-1.0, 1.0);
        }
    }
    Ok(out)
}

/// Languages whisper may pick: on noise or quiet speech it otherwise guesses
/// a rare one and writes nonsense in its script.
const LANGS: [&str; 18] = ["ru", "en", "uk", "be", "kk", "de", "fr", "es", "it", "pt", "pl", "tr", "nl", "sv", "cs", "ja", "ko", "zh"];

/// The clip's language: whisper's guess over its first minute and a half,
/// among `LANGS`; when it isn't sure, the interface's.
fn language(state: &mut whisper_rs::WhisperState, pcm: &[f32], threads: usize) -> &'static str {
    let fallback = crate::i18n::l("ru", "en");
    if state.pcm_to_mel(pcm, threads).is_err() {
        return fallback;
    }
    let secs = pcm.len() / 16_000;
    let mut sum = vec![0f32; whisper_rs::get_lang_max_id() as usize + 1];
    for offset in (0..secs.max(1)).step_by(30).take(3) {
        if let Ok((_, probs)) = state.lang_detect(offset * 1000, threads) {
            for (s, p) in sum.iter_mut().zip(probs) {
                *s += p;
            }
        }
    }
    let total: f32 = sum.iter().sum::<f32>().max(1e-6);
    LANGS
        .iter()
        .filter_map(|l| whisper_rs::get_lang_id(l).map(|id| (*l, sum.get(id as usize).copied().unwrap_or(0.0) / total)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|(_, p)| *p > 0.15)
        .map_or(fallback, |(l, _)| l)
}

/// The line is in the language's letters (Latin always passes: names, words
/// from games); one mostly in another script is noise read as speech.
fn in_script(text: &str, lang: &str) -> bool {
    let (mut ok, mut all) = (0, 0);
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        all += 1;
        let cyrillic = ('\u{0400}'..='\u{04FF}').contains(&c);
        let cjk = ('\u{3040}'..='\u{30FF}').contains(&c) || ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{AC00}'..='\u{D7AF}').contains(&c);
        let latin = c.is_ascii_alphabetic() || ('\u{00C0}'..='\u{024F}').contains(&c);
        let fits = latin
            || match lang {
                "ru" | "uk" | "be" | "kk" => cyrillic,
                "ja" | "ko" | "zh" => cjk,
                _ => false,
            };
        if fits {
            ok += 1;
        }
    }
    all == 0 || ok * 10 >= all * 8
}

/// What whisper writes when it hears nothing it knows: film subtitle
/// credits it learned from, and sound labels.
fn made_up(text: &str) -> bool {
    let t = text.to_lowercase();
    let starts_bracket = matches!(t.trim().chars().next(), Some('[' | '(' | '*' | '♪'));
    starts_bracket
        || ["субтитр", "dimatorzok", "продолжение следует", "спасибо за просмотр", "подписывайтесь на канал", "редактор субтитров", "amara.org"]
            .iter()
            .any(|w| t.contains(w))
}

/// Runs whisper: (start, end, text) per line, times from the audio's start.
fn transcribe(model: Model, pcm: &[f32], job: &Arc<Job>, ctx: &eframe::egui::Context) -> Result<Vec<(i64, i64, String)>, String> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};
    static QUIET: std::sync::Once = std::sync::Once::new();
    QUIET.call_once(whisper_rs::install_logging_hooks);
    let fail = |e: whisper_rs::WhisperError| lf!("распознавание: {e}", "recognition: {e}");
    let path = model.path();
    // whisper.cpp opens files by narrow names: a folder with non-Latin
    // letters in its name (a Cyrillic user name) is read into memory instead
    let cp = WhisperContextParameters::default();
    let wctx = if path.to_string_lossy().is_ascii() {
        WhisperContext::new_with_params(&path, cp)
    } else {
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        WhisperContext::new_from_buffer_with_params(&bytes, cp)
    }
    .map_err(fail)?;
    let mut state = wctx.create_state().map_err(fail)?;
    let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 2 });
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
    p.set_n_threads(threads as i32);
    let lang = language(&mut state, pcm, threads);
    p.set_language(Some(lang));
    p.set_translate(false);
    p.set_print_progress(false);
    p.set_print_realtime(false);
    p.set_print_special(false);
    p.set_print_timestamps(false);
    p.set_suppress_nst(true);
    // short lines, cut between words: a subtitle reads at a glance
    p.set_token_timestamps(true);
    p.set_max_len(42);
    p.set_split_on_word(true);
    // whisper hears in 30 s windows: a shorter clip gets a window its size,
    // quicker (small only: turbo's timing goes off with it)
    let secs = pcm.len() as f32 / 16_000.0;
    if model == Model::Small && secs < 28.0 {
        p.set_audio_ctx(((secs / 30.0 * 1500.0) as i32 + 128).min(1500));
    }
    {
        let (job, ctx) = (job.clone(), ctx.clone());
        p.set_progress_callback_safe(move |pct: i32| {
            job.progress.store((pct.clamp(0, 100) * 10) as u32, Ordering::Relaxed);
            ctx.request_repaint();
        });
    }
    state.full(p, pcm).map_err(fail)?;
    let mut out = Vec::new();
    for seg in state.as_iter() {
        if seg.no_speech_probability() > 0.6 {
            continue;
        }
        let text = seg.to_str_lossy().map(|t| t.trim().to_string()).unwrap_or_default();
        if text.is_empty() || made_up(&text) || !in_script(&text, lang) {
            continue;
        }
        // centiseconds
        let (a, b) = (seg.start_timestamp() * HNS / 100, seg.end_timestamp() * HNS / 100);
        // under a line whisper made up from silence there's next to nothing
        let (i0, i1) = ((seg.start_timestamp() as usize * 160).min(pcm.len()), (seg.end_timestamp() as usize * 160).min(pcm.len()));
        if i1 > i0 {
            let rms = (pcm[i0..i1].iter().map(|v| v * v).sum::<f32>() / (i1 - i0) as f32).sqrt();
            if rms < 0.012 {
                continue;
            }
        }
        out.push((a, b.max(a + HNS / 5), text));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn hallucinations() {
        assert!(super::made_up("Субтитры сделал DimaTorzok"));
        assert!(super::made_up("[музыка]"));
        assert!(!super::made_up("ну что, пошли на мид"));
        assert!(super::in_script("ну что, пошли на мид", "ru"));
        assert!(super::in_script("gg wp", "ru"));
        assert!(!super::in_script("Ḉᴇ ᴇ ᴇ ᴇ ɱა", "ru"));
        assert!(!super::in_script("忍中國 ḅἜʔ", "en"));
    }

    /// `BLINKER_TEST_SPEECH` = a video with speech (the small model must be downloaded).
    #[test]
    fn real_clip() {
        let Ok(src) = std::env::var("BLINKER_TEST_SPEECH") else { return };
        crate::media::startup();
        let src = std::path::PathBuf::from(src);
        let info = crate::media::probe(&src).unwrap();
        // BLINKER_TEST_SPEECH_TRACK: just that audio track; _SECS: just the start
        let only: Option<usize> = std::env::var("BLINKER_TEST_SPEECH_TRACK").ok().and_then(|v| v.parse().ok());
        let tracks: Vec<(u32, f32)> = info.audio_streams.iter().enumerate().filter(|(i, _)| only.is_none_or(|o| o == *i)).map(|(_, s)| (*s, 1.0)).collect();
        let secs: Option<i64> = std::env::var("BLINKER_TEST_SPEECH_SECS").ok().and_then(|v| v.parse().ok());
        let end = secs.map_or(info.duration, |s| (s * crate::record::encoder::HNS).min(info.duration));
        let model = if std::env::var("BLINKER_TEST_TURBO").is_ok() { super::Model::Turbo } else { super::Model::Small };
        let ctx = eframe::egui::Context::default();
        let job = std::sync::Arc::new(super::Job {
            progress: Default::default(),
            stage: std::sync::Mutex::new(super::Stage::Listen),
            cancel: Default::default(),
            result: Default::default(),
        });
        let t = std::time::Instant::now();
        let caps = super::run(&job, &ctx, model, &src, &tracks, 0, end).unwrap();
        eprintln!("{} lines in {:?}", caps.len(), t.elapsed());
        for c in caps {
            eprintln!("{:>6.2} – {:>6.2}  {}", c.from as f64 / 1e7, c.to as f64 / 1e7, c.text);
        }
    }
}

//! "Recognize text": the words in a capture, read by the system (Windows.Media.Ocr,
//! macOS Vision), shown in a window over the very spot they came from: each
//! line on a dark plate where it was, selectable, all of it on the clipboard.
//! The window runs as its own process (`blinker --text <file> <x> <y>`),
//! like a pin.

use crate::icons::Icon;
use crate::theme::{self, *};
use crate::ui::{self, TiledTexture};
use crate::win::{self, IRect};
use eframe::egui::{self, Color32, Key, Rect, Stroke, StrokeKind, TextureOptions, pos2, vec2};
use image::RgbaImage;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// A line of text and where it is in the image (pixels).
#[derive(Clone, Debug)]
pub struct Line {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The text of `lines` in reading order, one line each.
pub fn joined(lines: &[Line]) -> String {
    lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")
}

/// Cyrillic letters a reader puts for Latin ones that look alike (and the
/// ones it mixes up: г for r, т for m, п for n, и for u).
const AS_LATIN: [(char, char); 31] = [
    ('а', 'a'), ('е', 'e'), ('о', 'o'), ('р', 'p'), ('с', 'c'), ('у', 'y'), ('х', 'x'), ('к', 'k'), ('і', 'i'), ('ј', 'j'), ('ѕ', 's'),
    ('г', 'r'), ('т', 'm'), ('п', 'n'), ('и', 'u'), ('ь', 'b'),
    ('А', 'A'), ('В', 'B'), ('Е', 'E'), ('К', 'K'), ('М', 'M'), ('Н', 'H'), ('О', 'O'), ('Р', 'P'), ('С', 'C'), ('Т', 'T'), ('Х', 'X'), ('У', 'Y'), ('І', 'I'), ('Ј', 'J'), ('Ѕ', 'S'),
];

/// Latin letters that look like Cyrillic ones.
const AS_CYRILLIC: [(char, char); 20] = [
    ('a', 'а'), ('e', 'е'), ('o', 'о'), ('p', 'р'), ('c', 'с'), ('y', 'у'), ('x', 'х'), ('k', 'к'),
    ('A', 'А'), ('B', 'В'), ('E', 'Е'), ('K', 'К'), ('M', 'М'), ('H', 'Н'), ('O', 'О'), ('P', 'Р'), ('C', 'С'), ('T', 'Т'), ('X', 'Х'), ('Y', 'У'),
];

fn cyrillic(c: char) -> bool {
    ('\u{0400}'..='\u{04FF}').contains(&c)
}

/// Which script a word is in by its letters that can't be the other's:
/// Some(true) Latin, Some(false) Cyrillic, None when it can't tell.
fn script_of(word: &str) -> Option<bool> {
    let latin = word.chars().any(|c| c.is_ascii_alphabetic() && !AS_CYRILLIC.iter().any(|p| p.0 == c));
    let cyr = word.chars().any(|c| cyrillic(c) && !AS_LATIN.iter().any(|p| p.0 == c));
    match (latin, cyr) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

/// `line` with each word in one script: a reader that knows both alphabets
/// writes "Аррlе" (Cyrillic А and р) or "тасОS" for "Apple" and "macOS",
/// which look right and copy wrong. A word of only look-alike letters goes
/// by its neighbours.
pub fn one_script(line: &str) -> String {
    // runs of letters and what's between them
    let mut parts: Vec<(String, bool)> = Vec::new();
    for c in line.chars() {
        let letter = c.is_alphabetic();
        match parts.last_mut() {
            Some((s, l)) if *l == letter => s.push(c),
            _ => parts.push((c.to_string(), letter)),
        }
    }
    let scripts: Vec<Option<bool>> = parts.iter().map(|(s, l)| if *l { script_of(s) } else { None }).collect();
    // the nearest word with a script, going `step` from `from`
    let decided = |from: usize, step: isize| {
        let mut i = from as isize + step;
        while i >= 0 && (i as usize) < parts.len() {
            if let Some(s) = scripts[i as usize] {
                return Some(s);
            }
            i += step;
        }
        None
    };
    let mut out = String::with_capacity(line.len());
    for (i, (s, letter)) in parts.iter().enumerate() {
        let by_neighbours = || match (decided(i, -1), decided(i, 1)) {
            (Some(a), Some(b)) if a == b => Some(a),
            (Some(a), None) | (None, Some(a)) => Some(a),
            _ => None,
        };
        let (script, sure) = match (*letter, scripts[i]) {
            (false, _) => (None, false),
            (true, Some(s)) => (Some(s), true),
            (true, None) => (by_neighbours(), false),
        };
        // by its neighbours only the letters that look exactly alike: "и"
        // between two English words is still the Russian "and"
        let latin = if sure { &AS_LATIN[..] } else { &AS_LATIN[..11] };
        let latin = latin.iter().chain(&AS_LATIN[16..]);
        match script {
            Some(true) => out.extend(s.chars().map(|c| latin.clone().find(|p| p.0 == c).map_or(c, |p| p.1))),
            Some(false) => out.extend(s.chars().map(|c| AS_CYRILLIC.iter().find(|p| p.0 == c).map_or(c, |p| p.1))),
            None => out.push_str(s),
        }
    }
    out
}

/// Reading order: top to bottom, left to right along a row.
pub fn sort(lines: &mut [Line]) {
    lines.sort_by(|a, b| {
        let same_row = (a.y - b.y).abs() < a.h.min(b.h) * 0.5;
        if same_row { a.x.total_cmp(&b.x) } else { a.y.total_cmp(&b.y) }
    });
}

/// Small text reads better larger: the image as the recognizer gets it, and
/// how much bigger it is.
#[cfg(target_os = "macos")]
fn prepared(img: &RgbaImage, max_side: u32) -> (RgbaImage, f32) {
    let side = img.width().max(img.height()).max(1);
    let k = if side < 1500 { 2.0f32 } else { 1.0 }.min(max_side as f32 / side as f32);
    if (k - 1.0).abs() < 0.01 {
        return (img.clone(), 1.0);
    }
    let (w, h) = (((img.width() as f32 * k) as u32).max(1), ((img.height() as f32 * k) as u32).max(1));
    (image::imageops::resize(img, w, h, image::imageops::FilterType::CatmullRom), k)
}

/// The text in `img`: PaddleOCR's models (see `paddle`).
#[cfg(windows)]
pub fn recognize(img: &RgbaImage) -> Result<Vec<Line>, String> {
    crate::paddle::recognize(img)
}

/// The recognizer still has to be downloaded: PaddleOCR's models on
/// Windows (a Mac reads with Vision, part of the system).
pub fn needs_model() -> bool {
    cfg!(windows) && !crate::paddle::ready()
}

/// The text in `img`, by Vision (Russian and English, the language guessed).
#[cfg(target_os = "macos")]
pub fn recognize(img: &RgbaImage) -> Result<Vec<Line>, String> {
    use std::ffi::{CStr, c_char, c_void};
    extern "C" fn found(ctx: *mut c_void, text: *const c_char, x: f64, y: f64, w: f64, h: f64) {
        let out = unsafe { &mut *(ctx as *mut Vec<(String, f64, f64, f64, f64)>) };
        let text = unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned();
        out.push((one_script(&text), x, y, w, h));
    }
    let (scaled, _) = prepared(img, 4096);
    let mut raw: Vec<(String, f64, f64, f64, f64)> = Vec::new();
    let n = unsafe {
        win::ffi::bk_ocr(scaled.as_raw().as_ptr(), scaled.width() as i32, scaled.height() as i32, &mut raw as *mut _ as *mut c_void, found)
    };
    if n < 0 {
        return Err(crate::i18n::l("macos не смог прочитать текст", "macos couldn't read the text").into());
    }
    // Vision's boxes are fractions of the picture
    let (w, h) = (img.width() as f64, img.height() as f64);
    let mut out: Vec<Line> = raw
        .into_iter()
        .filter(|(t, ..)| !t.trim().is_empty())
        .map(|(text, x, y, bw, bh)| Line { text, x: (x * w) as f32, y: (y * h) as f32, w: (bw * w) as f32, h: (bh * h) as f32 })
        .collect();
    sort(&mut out);
    Ok(out)
}

// ---------------------------------------------------------------------------
// The window

/// The header above the picture (points).
const HEADER: f32 = 46.0;

/// Saves the capture and starts a text window for it over `at`.
pub fn spawn(img: &RgbaImage, at: IRect) {
    let dir = crate::config::cache_dir().join("text");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("text-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")));
    if crate::output::write(img, &path, crate::config::ImageFormat::Png, 90).is_err() {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--text").arg(&path).arg(at.x.to_string()).arg(at.y.to_string()).spawn();
    }
}

/// Set by a new text window: the one already open closes (one at a time).
const CLOSE_EVENT: &str = "Local\\BlinkerTextClose";

/// Closes the text window if one is open, and waits for it to be gone (a
/// capture right after shouldn't have it in the picture). True if there was one.
pub fn close_open() -> bool {
    let open = win::signal_event(CLOSE_EVENT);
    if open {
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    open
}

/// What the recognizer found, once it's done.
type Found = Arc<Mutex<Option<Result<Vec<Line>, String>>>>;

/// Entry point of a text window process.
pub fn run(path: PathBuf, x: i32, y: i32) {
    // the interface language (it also picks the recognizer's)
    let _ = crate::config::load();
    let Some(img) = crate::output::load(&path) else { return };
    let _ = std::fs::remove_file(&path);
    let mon = win::monitor_at(&win::monitors(), x + 1, y + 1);
    let work = mon.map(|m| m.work).unwrap_or(IRect::new(0, 0, 1920, 1040));
    let scale = mon.map(|m| m.scale).unwrap_or(1.0);
    // the picture stays where it was taken, the header above it
    let header = (HEADER * scale).round() as i32;
    // without the model: room for the card that offers it
    let waiting = needs_model();
    let (min_w, min_h) = if waiting { (380.0, 230.0) } else { (300.0, 0.0) };
    let w = (img.width() as i32).max((min_w * scale) as i32);
    let h = (img.height() as i32).max((min_h * scale) as i32) + header;
    let rect = IRect::new(x.clamp(work.x, (work.right() - w).max(work.x)), (y - header).clamp(work.y, (work.bottom() - h).max(work.y)), w.min(work.w), h.min(work.h));
    let found: Found = Arc::default();
    if !waiting {
        read_in_background(&found, &img);
    }
    // one text window at a time: the one open goes, and lets go of the name
    // before this one takes it
    close_open();
    let close = win::create_event(CLOSE_EVENT);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(crate::i18n::l("blinker · текст", "blinker · text"))
            .with_icon(theme::icon_data(64))
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top()
            .with_transparent(theme::glass())
            .with_inner_size([rect.w as f32 / scale, rect.h as f32 / scale]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let _ = eframe::run_native(
        "blinker text",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            let hwnd = win::hwnd_of(cc);
            if let Some(h) = hwnd {
                // macOS 26: the header a pane of glass right on the desktop,
                // the window clear around it and the picture
                #[cfg(target_os = "macos")]
                if theme::glass() {
                    win::window_glass(h, (6.0, 5.0, rect.w as f32 / scale - 12.0, HEADER - 9.0), 15.0, 0);
                }
                win::cloak(h, true);
                win::set_window_rect(h, rect, true);
            }
            if let Some(ev) = close {
                let ctx = cc.egui_ctx.clone();
                std::thread::spawn(move || {
                    loop {
                        if win::wait_event(ev, 2000) {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            ctx.request_repaint();
                        }
                    }
                });
            }
            let tex = TiledTexture::new(&cc.egui_ctx, "text-shot", &img, TextureOptions::LINEAR);
            Ok(Box::new(TextApp {
                size: vec2(img.width() as f32, img.height() as f32),
                tex,
                img,
                hwnd,
                rect,
                frame_no: 0,
                found,
                waiting,
                install: None,
                install_error: None,
                card_h: 200.0,
                lines: None,
                error: None,
                notice: None,
                shown_at: 0.0,
            }))
        }),
    );
}

/// Reads `img` on a thread; the result lands in `found`.
fn read_in_background(found: &Found, img: &RgbaImage) {
    let (found, img) = (found.clone(), img.clone());
    std::thread::spawn(move || {
        let r = recognize(&img);
        *found.lock().unwrap() = Some(r);
    });
}

struct TextApp {
    /// The picture's pixels.
    size: egui::Vec2,
    tex: TiledTexture,
    img: RgbaImage,
    /// The model isn't there yet: the window offers it before reading.
    waiting: bool,
    install: Option<Arc<crate::paddle::Install>>,
    install_error: Option<String>,
    /// The offer's height last frame (it's centered by it).
    card_h: f32,
    hwnd: Option<win::HWND>,
    rect: IRect,
    frame_no: u32,
    found: Found,
    lines: Option<Vec<Line>>,
    error: Option<String>,
    notice: Option<ui::Notice>,
    /// When the text came in (the lines appear one after another from it).
    shown_at: f64,
}

impl TextApp {
    fn copy_all(&mut self, ctx: &egui::Context) {
        let Some(lines) = &self.lines else { return };
        if !lines.is_empty() {
            crate::output::copy_text(&joined(lines));
            self.notice = Some(ui::Notice::new(ctx, crate::i18n::l("текст скопирован", "text copied")));
        }
    }

    fn header(&mut self, ui: &mut egui::Ui, bar: Rect) {
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        if !theme::glass() {
            p.rect_filled(bar, 0, PANEL);
            p.hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, LINE));
        }
        // dragging the header moves the window
        let drag = ui.interact(bar, ui.scope_id().with("text-drag"), egui::Sense::drag());
        if drag.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        let dot = pos2(bar.left() + 25.0, bar.center().y);
        p.circle_filled(dot, 14.0, ACCENT.gamma_multiply(0.16));
        crate::icons::paint(&p, Rect::from_center_size(dot, egui::Vec2::splat(16.0)), Icon::Scan, 14.0, ACCENT);
        let status = match (&self.lines, &self.error) {
            _ if self.install.is_some() => crate::i18n::l("скачиваю модель…", "downloading the model…").to_string(),
            _ if self.waiting => crate::i18n::l("нужна модель распознавания", "needs the recognition model").to_string(),
            (_, Some(e)) => e.clone(),
            (None, None) => crate::i18n::l("читаю…", "reading…").to_string(),
            (Some(l), None) if l.is_empty() => crate::i18n::l("текста не нашлось", "no text found").to_string(),
            (Some(l), None) => {
                let n = l.len();
                let word = match (n % 10, n % 100) {
                    (1, x) if x != 11 => "строка",
                    (2..=4, x) if !(12..=14).contains(&x) => "строки",
                    _ => "строк",
                };
                let en = if n == 1 { "line" } else { "lines" };
                lf!("{n} {word} · всё уже в буфере", "{n} {en} · all of it is on the clipboard")
            }
        };
        // the buttons first, right to left; the text gets what they leave
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(8.0, 6.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        child.spacing_mut().item_spacing.x = 4.0;
        if ui::icon_button(&mut child, Icon::Close, crate::i18n::l("закрыть  esc", "close  esc"), 30.0, false, ui::Kind::Ghost).clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.lines.as_ref().is_some_and(|l| !l.is_empty()) {
            let label = crate::i18n::l("копировать всё", "copy all");
            // a narrow window: the icon alone
            let copy = if bar.width() < 380.0 {
                ui::icon_button(&mut child, Icon::Copy, label, 30.0, false, ui::Kind::Accent)
            } else {
                ui::button(&mut child, label, ui::Kind::Accent)
            };
            if copy.clicked() {
                self.copy_all(&ctx);
            }
        }
        let text_right = child.min_rect().left() - 10.0;
        let max_w = (text_right - (bar.left() + 48.0)).max(20.0);
        let title = elided(&p, crate::i18n::l("текст с картинки", "text from the picture"), font_medium(13.0), TEXT, max_w);
        let status = elided(&p, &status, font(11.5), DIM, max_w);
        p.galley(pos2(bar.left() + 48.0, bar.center().y - 8.0 - title.size().y / 2.0), title, TEXT);
        p.galley(pos2(bar.left() + 48.0, bar.center().y + 9.0 - status.size().y / 2.0), status, DIM);
    }
}

/// `text` on one line, cut with "…" to fit `max_w`.
fn elided(p: &egui::Painter, text: &str, font: egui::FontId, color: Color32, max_w: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
    p.layout_job(job)
}

impl TextApp {
    /// The card asking for the model, in the middle of `area`.
    fn offer(&mut self, ui: &mut egui::Ui, area: Rect) {
        let w = (area.width() - 24.0).min(360.0);
        let card = Rect::from_center_size(area.center(), vec2(w, self.card_h));
        ui::paint_float(ui.painter(), card, 12, 1.0);
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt("offer").max_rect(card.shrink2(vec2(18.0, 16.0))).layout(egui::Layout::top_down(egui::Align::Min)));
        child.spacing_mut().item_spacing.y = 6.0;
        child.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let (tile, _) = ui.allocate_exact_size(vec2(38.0, 38.0), egui::Sense::hover());
            ui.painter().rect_filled(tile, 10, ACCENT.gamma_multiply(0.15));
            crate::icons::paint(ui.painter(), Rect::from_center_size(tile.center(), egui::Vec2::splat(18.0)), Icon::Scan, 15.0, ACCENT);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(egui::RichText::new(crate::i18n::l("модель для текста", "the text model")).font(font_medium(14.0)).color(TEXT));
                ui.label(egui::RichText::new(lf!("paddleocr · {} мб, один раз", "paddleocr · {} mb, once", crate::paddle::SIZE_MB)).font(font(12.0)).color(DIM));
            });
        });
        child.add_space(2.0);
        child.label(
            egui::RichText::new(crate::i18n::l(
                "читает русский, украинский, белорусский и английский прямо на этом компьютере. картинка никуда не уходит",
                "reads Russian, Ukrainian, Belarusian and English right on this computer. the picture goes nowhere",
            ))
            .font(font(12.5))
            .color(TEXT2),
        );
        child.add_space(6.0);
        match &self.install {
            Some(job) => {
                // the download: a bar filling up
                let f = job.fraction();
                let (bar, _) = child.allocate_exact_size(vec2(child.available_width(), 6.0), egui::Sense::hover());
                child.painter().rect_filled(bar, 3, ui::hover_fill().gamma_multiply(2.0));
                child.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * f.max(0.02), bar.height())), 3, ACCENT);
                child.label(egui::RichText::new(lf!("скачиваю {:.0}%", "downloading {:.0}%", f * 100.0)).font(font(12.0)).color(DIM));
            }
            None => {
                if let Some(e) = &self.install_error {
                    child.label(egui::RichText::new(lf!("не скачалась: {e}", "didn't download: {e}")).font(font(12.0)).color(DANGER));
                }
                let label = if self.install_error.is_some() { crate::i18n::l("ещё раз", "try again") } else { crate::i18n::l("скачать и распознать", "download and read") };
                if ui::button(&mut child, label, ui::Kind::Accent).clicked() {
                    self.install_error = None;
                    self.install = Some(crate::paddle::install(ui.ctx()));
                }
            }
        }
        // the card fits what's in it next frame
        let used = child.min_rect().height() + 32.0;
        if (used - self.card_h).abs() > 0.5 {
            self.card_h = used;
            ui.ctx().request_repaint();
        }
    }
}

/// A line of light running down a picture (reading, scanning).
pub fn scan_line(p: &egui::Painter, r: Rect, y: f32, alpha: f32) {
    let glow = Rect::from_min_max(pos2(r.left(), (y - 40.0).max(r.top())), pos2(r.right(), y));
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(glow.left_top(), Color32::TRANSPARENT);
    mesh.colored_vertex(glow.right_top(), Color32::TRANSPARENT);
    mesh.colored_vertex(glow.right_bottom(), ACCENT.gamma_multiply(0.22 * alpha));
    mesh.colored_vertex(glow.left_bottom(), ACCENT.gamma_multiply(0.22 * alpha));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(egui::Shape::mesh(mesh));
    p.hline(r.x_range(), y, Stroke::new(2.0, ACCENT.gamma_multiply(0.8 * alpha)));
}

impl eframe::App for TextApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if theme::glass() { [0.0; 4] } else { BG.to_normalized_gamma_f32() }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.frame_no += 1;
        if let Some(h) = self.hwnd {
            if self.frame_no == 3 {
                win::cloak(h, false);
                win::force_foreground(h);
            } else if self.frame_no < 3 {
                ctx.request_repaint();
            }
            if self.frame_no <= 10 && win::window_rect(h) != self.rect {
                win::set_window_rect(h, self.rect, true);
            }
        }
        // the model came down: now it reads
        let installed = self.install.as_ref().and_then(|j| j.result.lock().unwrap().take());
        match installed {
            Some(Ok(())) => {
                self.install = None;
                self.waiting = false;
                read_in_background(&self.found, &self.img);
            }
            Some(Err(e)) => {
                self.install = None;
                self.install_error = Some(e);
            }
            None if self.install.is_some() => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            None => {}
        }
        if !self.waiting && self.lines.is_none() && self.error.is_none() {
            let found = self.found.lock().unwrap().take();
            match found {
                Some(Ok(lines)) => {
                    self.lines = Some(lines);
                    self.shown_at = now;
                    // the whole text goes to the clipboard right away
                    self.copy_all(&ctx);
                }
                Some(Err(e)) => self.error = Some(e),
                None => ctx.request_repaint(),
            }
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let full = ctx.content_rect();
        let ppp = ctx.pixels_per_point();
        let bar = Rect::from_min_size(full.min, vec2(full.width(), HEADER));
        let pic = Rect::from_min_size(pos2(full.left(), bar.bottom()), self.size / ppp);
        let p = ui.painter().clone();
        let glass = theme::glass();
        if !glass {
            p.rect_filled(full, 0, BG);
        }
        self.tex.paint(&p, Rect::from_min_size(pos2(0.0, 0.0), self.tex.size), pic, Color32::WHITE);
        let mut copied_line = None;
        match &self.lines {
            None if self.waiting => {
                p.rect_filled(Rect::from_min_max(pic.min, full.max), 0, Color32::from_black_alpha(170));
                self.offer(ui, Rect::from_min_max(pos2(full.left(), bar.bottom()), full.max));
            }
            None => {
                // reading: a line of light runs down the picture
                p.rect_filled(pic, 0, Color32::from_black_alpha(110));
                let t = ((now * 0.8) % 1.0) as f32;
                scan_line(&p, pic, pic.top() + pic.height() * t, 1.0);
            }
            Some(lines) if lines.is_empty() => {
                p.rect_filled(pic, 0, Color32::from_black_alpha(150));
                let g = p.layout_no_wrap(crate::i18n::l("текста не нашлось", "no text found").to_string(), font_medium(13.0), TEXT);
                let r = Rect::from_center_size(pic.center(), g.size() + vec2(28.0, 14.0));
                p.rect_filled(r, r.height() / 2.0, ui::float_fill());
                if glass {
                    p.rect_stroke(r, r.height() / 2.0, Stroke::new(1.0, Color32::from_white_alpha(30)), StrokeKind::Inside);
                }
                p.galley(r.min + vec2(14.0, 7.0), g, TEXT);
            }
            Some(lines) => {
                // the picture dimmed, each line over its own words, one after another
                p.rect_filled(pic, 0, Color32::from_black_alpha(160));
                let pointer = ctx.input(|i| i.pointer.hover_pos());
                for (i, l) in lines.iter().enumerate() {
                    let age = (now - self.shown_at) as f32 - i as f32 * 0.025;
                    let t = ui::ease_out_cubic((age / 0.22).clamp(0.0, 1.0));
                    if t < 1.0 {
                        ctx.request_repaint();
                    }
                    if t <= 0.0 {
                        continue;
                    }
                    let r = Rect::from_min_size(pic.min + vec2(l.x, l.y) / ppp + vec2(0.0, (1.0 - t) * 5.0), vec2(l.w, l.h) / ppp);
                    // as wide as the words it covers, within reason of their height
                    let probe = p.layout_no_wrap(l.text.clone(), font_medium(r.height()), TEXT);
                    let size = (r.height() * r.width() / probe.size().x.max(1.0)).clamp(r.height() * 0.6, r.height() * 1.5).clamp(8.0, 72.0);
                    let plate = r.expand2(vec2(5.0, 3.0)).union(Rect::from_min_size(r.min, vec2(probe.size().x * size / r.height().max(1.0), r.height())).expand2(vec2(5.0, 3.0)));
                    let hot = pointer.is_some_and(|pt| plate.expand2(vec2(26.0, 0.0)).contains(pt));
                    // on glass: dark glass plates with a light rim, rounder
                    let (fill, rim, radius) = if glass {
                        (if hot { Color32::from_rgba_unmultiplied(44, 44, 42, 240) } else { ui::float_fill() }, Color32::from_white_alpha(34), 7)
                    } else {
                        (if hot { Color32::from_rgb(40, 40, 38) } else { Color32::from_rgb(24, 24, 23) }, LINE, 5)
                    };
                    p.rect_filled(plate, radius, fill.gamma_multiply(t));
                    p.rect_stroke(plate, radius, Stroke::new(1.0, if hot { ACCENT.gamma_multiply(0.5) } else { rim.gamma_multiply(t) }), StrokeKind::Inside);
                    let mut child = ui.new_child(egui::UiBuilder::new().id_salt(("line", i)).max_rect(r.expand2(vec2(40.0, 2.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                    child.set_clip_rect(full);
                    child.set_opacity(t);
                    child.add_space(r.left() - child.max_rect().left());
                    child.add(egui::Label::new(egui::RichText::new(&l.text).font(font_medium(size)).color(TEXT)).selectable(true).extend());
                    // a copy button at the line's end on hover
                    if hot {
                        let b = Rect::from_min_size(pos2(plate.right() + 3.0, plate.center().y - 11.0), vec2(22.0, 22.0));
                        let resp = ui.interact(b, ui.scope_id().with(("copy-line", i)), egui::Sense::click());
                        p.rect_filled(b, if glass { 11 } else { 5 }, if resp.hovered() { ACCENT } else { ui::float_fill() });
                        crate::icons::paint(&p, b, Icon::Copy, 12.0, if resp.hovered() { ON_ACCENT } else { DIM });
                        if resp.on_hover_text(crate::i18n::l("скопировать строку", "copy the line")).clicked() {
                            copied_line = Some(l.text.clone());
                        }
                    }
                }
            }
        }
        if let Some(text) = copied_line {
            crate::output::copy_text(&text);
            self.notice = Some(ui::Notice::new(&ctx, crate::i18n::l("строка скопирована", "line copied")));
        }
        if !glass {
            p.rect_stroke(full, 0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        }
        self.header(ui, bar);
        if let Some(n) = &self.notice
            && !n.show(&ctx, pic)
        {
            self.notice = None;
        }
    }
}

#[cfg(test)]
mod tests {
    /// Mixed alphabets come out in one per word.
    #[test]
    fn words_in_one_script() {
        let fix = super::one_script;
        assert_eq!(fix("В марте Аррlе добавила в тасОS защиту от СlісkFiх"), "В марте Apple добавила в macOS защиту от ClickFix");
        assert_eq!(fix("Settings + Сарtuге → гead QR соdes · читать qг-коды"), "Settings + Capture → read QR codes · читать qr-коды");
        assert_eq!(fix("Скачай blinkеr для Windows и mасОS"), "Скачай blinker для Windows и macOS");
        assert_eq!(fix("от атак в стиле"), "от атак в стиле");
        assert_eq!(fix("кoды и тeкст"), "коды и текст");
    }

    /// `BLINKER_TEST_OCR` = a picture with text: its lines, printed.
    #[test]
    #[cfg(windows)]
    fn read_picture() {
        let Ok(path) = std::env::var("BLINKER_TEST_OCR") else { return };
        let img = image::open(path).unwrap().to_rgba8();
        let t = std::time::Instant::now();
        let lines = crate::paddle::recognize(&img).unwrap();
        eprintln!("{} lines in {:?}", lines.len(), t.elapsed());
        for l in &lines {
            eprintln!("{:>5.0} {:>5.0} {:>4.0}×{:<3.0} {}", l.x, l.y, l.w, l.h, l.text);
        }
    }
}

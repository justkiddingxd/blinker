//! The history window: the latest screenshots and recordings from the
//! captures folder, grouped by day, and what was uploaded (the link, how long
//! it lives, "delete from the server"). Its own process, opened from the tray;
//! closing it ends the process.

use crate::chrome;
use crate::config::{self, Config};
use crate::i18n::l;
use crate::icons::{self, Icon};
use crate::runtime::{self, Shared};
use crate::theme::{self, *};
use crate::ui::{self, Notice};
use crate::upload::Uploaded;
use crate::win;
use eframe::egui::{self, Align2, Color32, CursorIcon, Rect, Sense, Stroke, StrokeKind, TextureHandle, Vec2, pos2, vec2};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::SystemTime;

pub const TITLE: &str = "blinker · история";
/// Named event the tray sets to bring a running history window forward.
pub const SHOW_EVENT: &str = "Local\\BlinkerHistoryShow";
/// Tab asked for by the next show (read when the event fires).
pub fn request_file() -> PathBuf {
    config::cache_dir().join("history-tab")
}

const SIZE: (f32, f32) = (980.0, 660.0);
const BAR_H: f32 = 56.0;
const CARD_W: f32 = 214.0;
const THUMB_H: f32 = 134.0;
/// How many files the window looks at (the newest).
const MAX_ITEMS: usize = 400;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Shots,
    Videos,
    Uploads,
}

impl Tab {
    pub fn arg(self) -> &'static str {
        match self {
            Tab::Shots => "shots",
            Tab::Videos => "videos",
            Tab::Uploads => "uploads",
        }
    }
    pub fn from_arg(s: &str) -> Tab {
        match s.trim() {
            "videos" => Tab::Videos,
            "uploads" => Tab::Uploads,
            _ => Tab::Shots,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Image,
    Video,
    /// A GIF or sound: no editor of ours opens it.
    Other,
}

#[derive(Clone)]
struct Item {
    path: PathBuf,
    kind: Kind,
    modified: SystemTime,
    size: u64,
}

fn kind_of(path: &Path) -> Option<Kind> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    Some(match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" => Kind::Image,
        "mp4" => Kind::Video,
        "gif" | "mp3" | "m4a" | "wav" => Kind::Other,
        _ => return None,
    })
}

/// The captures folder, newest first (dated folders included, three levels deep).
fn scan(dir: &Path) -> Vec<Item> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<Item>) {
        let Ok(list) = std::fs::read_dir(dir) else { return };
        for e in list.flatten() {
            let path = e.path();
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                if depth < 3 {
                    walk(&path, depth + 1, out);
                }
            } else if let Some(kind) = kind_of(&path) {
                out.push(Item { path, kind, modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), size: meta.len() });
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, 0, &mut out);
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out.truncate(MAX_ITEMS);
    out
}

// ---------------------------------------------------------------------------
// Thumbnails: made by two worker threads, kept in the cache folder

struct Thumbs {
    ready: Mutex<HashMap<PathBuf, Option<image::RgbaImage>>>,
    queue: Mutex<VecDeque<PathBuf>>,
    wake: Condvar,
    asked: Mutex<HashSet<PathBuf>>,
}

impl Thumbs {
    fn start(ctx: &egui::Context) -> Arc<Thumbs> {
        let t = Arc::new(Thumbs { ready: Mutex::default(), queue: Mutex::default(), wake: Condvar::new(), asked: Mutex::default() });
        for _ in 0..2 {
            let (t, ctx) = (t.clone(), ctx.clone());
            std::thread::spawn(move || {
                crate::media::startup();
                loop {
                    let path = {
                        let mut q = t.queue.lock().unwrap();
                        loop {
                            if let Some(p) = q.pop_front() {
                                break p;
                            }
                            q = t.wake.wait(q).unwrap();
                        }
                    };
                    let img = thumb(&path);
                    t.ready.lock().unwrap().insert(path, img);
                    ctx.request_repaint();
                }
            });
        }
        t
    }

    fn ask(&self, path: &Path) {
        if self.asked.lock().unwrap().insert(path.to_path_buf()) {
            // the ones on screen now go first
            self.queue.lock().unwrap().push_front(path.to_path_buf());
            self.wake.notify_one();
        }
    }
}

/// A small picture of a file, from the cache or made now.
fn thumb(path: &Path) -> Option<image::RgbaImage> {
    let meta = std::fs::metadata(path).ok()?;
    let stamp = meta.modified().ok()?.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut h);
        stamp.hash(&mut h);
        h.finish()
    };
    let dir = config::cache_dir().join("thumbs");
    let cached = dir.join(format!("{key:016x}.png"));
    if let Ok(img) = image::open(&cached) {
        return Some(img.to_rgba8());
    }
    let (w, h) = ((CARD_W * 2.0) as u32, (THUMB_H * 2.0) as u32);
    let img = match kind_of(path)? {
        Kind::Image => image::open(path).ok().map(|i| i.thumbnail(w, h).to_rgba8()),
        Kind::Video => {
            let (r, _) = crate::media::VideoReader::open(path, w, h).ok()?;
            let _ = r.seek(crate::record::encoder::HNS / 3);
            r.next().ok().flatten().map(|(_, img)| img)
        }
        Kind::Other if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gif")) => {
            image::open(path).ok().map(|i| i.thumbnail(w, h).to_rgba8())
        }
        Kind::Other => None,
    }?;
    let _ = std::fs::create_dir_all(&dir);
    let _ = img.save(&cached);
    Some(img)
}

// ---------------------------------------------------------------------------

pub fn run(shared: &Arc<Shared>, tab: Tab) {
    let event = win::create_event(SHOW_EVENT);
    let viewport = egui::ViewportBuilder::default()
        .with_title(TITLE)
        .with_icon(theme::icon_data(64))
        .with_inner_size([SIZE.0, SIZE.1])
        .with_transparent(theme::glass());
    let viewport = chrome::style(viewport);
    let shared2 = shared.clone();
    runtime::run_smooth_window(shared, viewport, true, move |cc| {
        let hwnd = win::hwnd_of(cc);
        if let Some(h) = hwnd {
            win::style_borderless(h);
            chrome::place_window_buttons(h, BAR_H);
            #[cfg(target_os = "macos")]
            if theme::glass() {
                win::window_backdrop(h, 0x161615, crate::home::GLASS_TINT);
                win::window_glass(h, (7.0, 7.0, SIZE.0 - 14.0, BAR_H - 10.0), 18.0, 0);
            }
            win::force_foreground(h);
        }
        let show = Arc::new(AtomicBool::new(false));
        if let Some(ev) = event {
            let (show, ctx) = (show.clone(), cc.egui_ctx.clone());
            std::thread::spawn(move || {
                loop {
                    if win::wait_event(ev, 2000) {
                        show.store(true, Ordering::SeqCst);
                        ctx.request_repaint();
                    }
                }
            });
        }
        let cfg = shared2.config();
        HistoryApp {
            hwnd,
            show,
            items: scan(&cfg.save_dir),
            uploads: crate::upload::uploads(),
            thumbs: Thumbs::start(&cc.egui_ctx),
            tex: HashMap::new(),
            tab,
            cfg,
            shared: shared2,
            jobs: HashMap::new(),
            deleting: HashMap::new(),
            confirm: None,
            notice: None,
            content: Rect::NOTHING,
            renaming: None,
            menu: None,
        }
    });
}

struct HistoryApp {
    hwnd: Option<win::HWND>,
    /// The tray asked to bring the window forward.
    show: Arc<AtomicBool>,
    items: Vec<Item>,
    uploads: Vec<Uploaded>,
    thumbs: Arc<Thumbs>,
    tex: HashMap<PathBuf, Option<TextureHandle>>,
    tab: Tab,
    cfg: Config,
    shared: Arc<Shared>,
    /// Uploads started from here.
    jobs: HashMap<PathBuf, Arc<crate::upload::Job>>,
    /// Deletions from the server going on: (url → result once done).
    deleting: HashMap<String, Arc<Mutex<Option<Result<(), String>>>>>,
    /// The upload whose "delete" was clicked once (a second click deletes).
    confirm: Option<String>,
    notice: Option<Notice>,
    content: Rect,
    /// A file being renamed: its path, the name typed so far, and whether the
    /// field still has to take focus.
    renaming: Option<(PathBuf, String, bool)>,
    /// The right-click menu: its file, where it opened (the pointer) and its
    /// size last frame.
    menu: Option<(Item, egui::Pos2, Vec2)>,
}

fn human_size(b: u64) -> String {
    let kb = b as f64 / 1024.0;
    if kb < 1000.0 { lf!("{kb:.0} кб", "{kb:.0} kb") } else { lf!("{:.1} мб", "{:.1} mb", kb / 1024.0).replace('.', ",") }
}

fn day_title(t: SystemTime) -> String {
    let d = chrono::DateTime::<chrono::Local>::from(t).date_naive();
    let today = chrono::Local::now().date_naive();
    if d == today {
        l("сегодня", "today").into()
    } else if Some(d) == today.pred_opt() {
        l("вчера", "yesterday").into()
    } else {
        d.format("%d.%m.%Y").to_string()
    }
}

fn clock(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t).format("%H:%M").to_string()
}

fn left(expires: Option<u64>) -> String {
    let Some(e) = expires else { return l("без срока", "no expiry").into() };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let s = e.saturating_sub(now);
    let (d, h, m) = (s / 86400, s % 86400 / 3600, (s % 3600 / 60).max(1));
    if d > 0 {
        lf!("удалится через {d} д {h} ч", "deleted in {d} d {h} h")
    } else if h > 0 {
        lf!("удалится через {h} ч", "deleted in {h} h")
    } else {
        lf!("удалится через {m} мин", "deleted in {m} min")
    }
}

/// Opens a file the way the tray would: our editors for pictures and videos.
fn open(item: &Item) {
    let flag = match item.kind {
        Kind::Image => "--edit",
        Kind::Video => "--video",
        Kind::Other => {
            win::open_path(&item.path);
            return;
        }
    };
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg(flag).arg(&item.path).spawn();
    }
}

/// Characters no file name may hold (Windows is the strictest).
fn bad_name(name: &str) -> bool {
    name.chars().any(|c| matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control())
        || name.ends_with('.')
        || name.ends_with(' ')
}

/// A small picture to drag along with a file.
fn drag_picture(path: &Path) -> Vec<u8> {
    let mut png = Vec::new();
    if let Some(img) = thumb(path) {
        let w = 160.min(img.width());
        let h = (w * img.height() / img.width().max(1)).clamp(1, 160);
        let small = image::imageops::thumbnail(&img, w, h);
        let _ = image::ImageEncoder::write_image(
            image::codecs::png::PngEncoder::new(&mut png),
            small.as_raw(),
            small.width(),
            small.height(),
            image::ExtendedColorType::Rgba8,
        );
    }
    png
}

/// Width of the right-click menu.
const MENU_W: f32 = 252.0;

/// A row of a right-click menu; true when clicked.
fn menu_row(ui: &mut egui::Ui, icon: Icon, label: &str) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(MENU_W - 10.0, 30.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(r, 6, HOVER);
    }
    let icon_r = Rect::from_center_size(pos2(r.left() + 18.0, r.center().y), Vec2::splat(16.0));
    icons::paint(p, icon_r, icon, 14.0, if resp.hovered() { TEXT } else { DIM });
    p.text(pos2(r.left() + 36.0, r.center().y), Align2::LEFT_CENTER, label, font(13.0), TEXT);
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

fn menu_sep(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(MENU_W - 10.0, 9.0), Sense::hover());
    ui.painter().hline(r.shrink2(vec2(8.0, 0.0)).x_range(), r.center().y, Stroke::new(1.0, LINE));
}

impl HistoryApp {
    fn start_rename(&mut self, path: &Path) {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        self.renaming = Some((path.to_path_buf(), stem, true));
    }

    /// Renames the file (the extension stays); a clash or a bad name is a notice.
    fn finish_rename(&mut self, ctx: &egui::Context) {
        let Some((old, name, _)) = self.renaming.take() else { return };
        let name = name.trim();
        let stem = old.file_stem().unwrap_or_default().to_string_lossy();
        if name.is_empty() || name == stem {
            return;
        }
        if bad_name(name) {
            self.notice = Some(Notice::new(ctx, l("в имени не может быть \\ / : * ? \" < > |", "a name can't hold \\ / : * ? \" < > |")));
            return;
        }
        let new = match old.extension() {
            Some(ext) => old.with_file_name(format!("{name}.{}", ext.to_string_lossy())),
            None => old.with_file_name(name),
        };
        // a different case of the same name is the same file on Windows and macOS
        let same_file = new.to_string_lossy().to_lowercase() == old.to_string_lossy().to_lowercase();
        if new.exists() && !same_file {
            self.notice = Some(Notice::new(ctx, l("файл с таким именем уже есть", "a file with that name exists")));
            return;
        }
        if let Err(e) = std::fs::rename(&old, &new) {
            self.notice = Some(Notice::new(ctx, lf!("не переименовалось: {e}", "couldn't rename: {e}")));
            return;
        }
        for it in self.items.iter_mut().filter(|it| it.path == old) {
            it.path = new.clone();
        }
        if let Some(t) = self.tex.remove(&old) {
            self.tex.insert(new.clone(), t);
        }
        if let Some(j) = self.jobs.remove(&old) {
            self.jobs.insert(new, j);
        }
    }

    fn refresh(&mut self) {
        self.cfg = config::load();
        self.items = scan(&self.cfg.save_dir);
        self.uploads = crate::upload::uploads();
    }

    fn texture(&mut self, ctx: &egui::Context, path: &Path) -> Option<TextureHandle> {
        if let Some(t) = self.tex.get(path) {
            return t.clone();
        }
        let ready = self.thumbs.ready.lock().unwrap().remove(path);
        match ready {
            Some(img) => {
                let t = img.map(|i| ctx.load_texture(path.display().to_string(), theme::color_image(&i), egui::TextureOptions::LINEAR));
                self.tex.insert(path.to_path_buf(), t.clone());
                t
            }
            None => {
                self.thumbs.ask(path);
                None
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let glass = theme::glass();
        egui::Panel::top("bar")
            .exact_size(BAR_H)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(if glass { Color32::TRANSPARENT } else { PANEL }))
            .show(ui, |ui| {
                let r = ui.max_rect();
                chrome::drag_region(ui, r);
                if !glass {
                    ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, LINE));
                }
                ui.horizontal_centered(|ui| {
                    ui.add_space(14.0);
                    chrome::leading_space(ui);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(egui::RichText::new(l("история", "history")).font(font_semibold(16.0)).color(TEXT));
                    ui.add_space(10.0);
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let count = |k: Kind| self.items.iter().filter(|i| i.kind == k || (k == Kind::Video && i.kind == Kind::Other)).count();
                    let (shots, videos, ups) = (count(Kind::Image), count(Kind::Video), self.uploads.len());
                    for (tab, label) in [
                        (Tab::Shots, lf!("снимки {shots}", "shots {shots}")),
                        (Tab::Videos, lf!("видео и gif {videos}", "videos & gifs {videos}")),
                        (Tab::Uploads, lf!("загрузки {ups}", "uploads {ups}")),
                    ] {
                        if ui::filter(ui, &label, self.tab == tab).clicked() {
                            self.tab = tab;
                            self.confirm = None;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        chrome::window_buttons(ui);
                        ui.add_space(10.0);
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if ui::icon_button(ui, Icon::Folder, l("открыть папку", "open the folder"), 32.0, false, ui::Kind::Ghost).clicked() {
                            let _ = std::fs::create_dir_all(&self.cfg.save_dir);
                            win::open_path(&self.cfg.save_dir);
                        }
                        if ui::icon_button(ui, Icon::Repeat, l("обновить", "refresh"), 32.0, false, ui::Kind::Ghost).clicked() {
                            self.refresh();
                        }
                    });
                });
            });
    }

    fn grid(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame, kinds: &[Kind]) {
        let items: Vec<Item> = self.items.iter().filter(|i| kinds.contains(&i.kind)).cloned().collect();
        if items.is_empty() {
            empty(ui, l("здесь пока пусто", "nothing here yet"), l("снимки и записи из папки blinker появятся тут", "screenshots and recordings from the blinker folder show up here"));
            return;
        }
        let w = ui.available_width();
        let cols = ((w + 14.0) / (CARD_W + 14.0)).floor().max(1.0) as usize;
        let card_w = (w - (cols - 1) as f32 * 14.0) / cols as f32;
        let mut day = String::new();
        let mut i = 0;
        while i < items.len() {
            let d = day_title(items[i].modified);
            if d != day {
                ui.add_space(if day.is_empty() { 4.0 } else { 18.0 });
                ui.label(egui::RichText::new(&d).font(font_medium(13.0)).color(DIM));
                ui.add_space(8.0);
                day = d.clone();
            }
            // a row: up to `cols` cards of the same day
            let row: Vec<Item> = items[i..].iter().take(cols).take_while(|it| day_title(it.modified) == d).cloned().collect();
            i += row.len();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for it in &row {
                    self.card(ui, frame, it, card_w);
                }
            });
            ui.add_space(14.0);
        }
    }

    /// A file: a click opens it in its usual app, a right click shows every
    /// action, a drag takes the file out (into a chat, a folder…).
    fn card(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame, it: &Item, w: f32) {
        let h = THUMB_H * w / CARD_W;
        let (rect, resp) = ui.allocate_exact_size(vec2(w, h + 44.0), Sense::click_and_drag());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let thumb = Rect::from_min_size(rect.min, vec2(w, h));
        let p = ui.painter().clone();
        p.rect_filled(thumb, 10, CANVAS);
        match self.texture(ui.ctx(), &it.path) {
            Some(tex) => {
                let uv = ui::cover_uv(thumb.size(), tex.size_vec2());
                p.add(egui::epaint::RectShape::filled(thumb, egui::CornerRadius::same(10), Color32::WHITE).with_texture(tex.id(), uv));
            }
            None => {
                let icon = match it.kind {
                    Kind::Image => Icon::Region,
                    Kind::Video => Icon::Video,
                    Kind::Other => Icon::Sound,
                };
                icons::paint(&p, thumb, icon, 22.0, FAINT);
            }
        }
        p.rect_stroke(thumb, 10, Stroke::new(1.0, Color32::from_white_alpha(14)), StrokeKind::Inside);
        if it.kind == Kind::Video {
            ui::play_badge(&p, thumb.center(), 17.0, 1.0);
        }
        let name = it.path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let ext = it.path.extension().unwrap_or_default().to_string_lossy().to_lowercase();
        let name_r = Rect::from_min_size(pos2(rect.left(), thumb.bottom() + 4.0), vec2(w, 22.0));
        let renaming = self.renaming.as_ref().is_some_and(|(p, ..)| p == &it.path);
        if renaming {
            self.rename_field(ui, name_r);
        } else {
            let ng = p.layout(name, font_medium(12.5), TEXT, w);
            let ng_h = ng.size().y.min(18.0);
            p.with_clip_rect(Rect::from_min_size(pos2(rect.left(), thumb.bottom() + 7.0), vec2(w, ng_h))).galley(pos2(rect.left(), thumb.bottom() + 7.0), ng, TEXT);
            let nr = ui.interact(name_r, ui.scope_id().with((&it.path, "name")), Sense::click());
            if nr.on_hover_cursor(CursorIcon::Text).on_hover_text(l("двойной клик: переименовать", "double click: rename")).double_clicked() {
                self.start_rename(&it.path);
            }
        }
        p.text(
            pos2(rect.left(), thumb.bottom() + 33.0),
            Align2::LEFT_CENTER,
            format!("{} · {ext} · {}", clock(it.modified), human_size(it.size)),
            mono(11.0),
            FAINT,
        );

        // an upload going on: its progress over the picture
        if let Some(job) = self.jobs.get(&it.path).cloned() {
            match job.result() {
                None => {
                    p.rect_filled(thumb, 10, Color32::from_black_alpha(140));
                    p.text(thumb.center(), Align2::CENTER_CENTER, lf!("загрузка {:.0}%", "uploading {:.0}%", job.fraction() * 100.0), mono(12.0), TEXT);
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
                }
                Some(r) => {
                    self.jobs.remove(&it.path);
                    self.notice = Some(Notice::new(
                        ui.ctx(),
                        match r {
                            Ok(_) => l("ссылка скопирована", "link copied").to_string(),
                            Err(e) => lf!("не загрузилось: {e}", "upload failed: {e}"),
                        },
                    ));
                    self.uploads = crate::upload::uploads();
                }
            }
        }

        let menu_here = self.menu.as_ref().is_some_and(|(m, ..)| m.path == it.path);
        if (resp.hovered() || menu_here) && !self.jobs.contains_key(&it.path) {
            p.rect_filled(thumb, 10, Color32::from_white_alpha(10));
            p.rect_stroke(thumb, 10, Stroke::new(1.0, Color32::from_white_alpha(40)), StrokeKind::Inside);
        }
        if resp.drag_started() {
            let png = drag_picture(&it.path);
            let _ = drag::start_drag(frame, drag::DragItem::Files(vec![it.path.clone()]), drag::Image::Raw(png), |_, _| {}, drag::Options::default());
            return;
        }
        let on_name = ui.rect_contains_pointer(name_r);
        let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
        if !renaming && !on_name && resp.clicked() {
            win::open_path(&it.path);
        }
        if resp.secondary_clicked()
            && let Some(at) = resp.interact_pointer_pos()
        {
            self.menu = Some((it.clone(), at, vec2(MENU_W, 0.0)));
        }
    }

    /// The right-click menu, to the right of where it was opened (to the
    /// left when there's no room), as narrow as its rows.
    fn menu(&mut self, ctx: &egui::Context) {
        let Some((it, at, size)) = self.menu.clone() else { return };
        let win = ctx.content_rect().shrink(6.0);
        let x = if at.x + 4.0 + size.x <= win.right() { at.x + 4.0 } else { at.x - 4.0 - size.x };
        let y = (at.y - 6.0).min(win.bottom() - size.y).max(win.top());
        let resp = egui::Area::new(egui::Id::unique("history-menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(x.max(win.left()), y))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, LINE))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::same(5))
                    .shadow(egui::epaint::Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(90) })
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.set_width(MENU_W - 10.0);
                        self.card_menu(ui, &it)
                    })
                    .inner
            });
        let rect = resp.response.rect;
        let picked = resp.inner;
        // a click anywhere else, or Esc, closes it
        let away = ctx.input(|i| (i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p))) || i.key_pressed(egui::Key::Escape));
        if picked || away {
            self.menu = None;
        } else if let Some(m) = &mut self.menu {
            m.2 = rect.size();
        }
    }

    /// Everything a file allows, in its right-click menu; true once one is picked.
    fn card_menu(&mut self, ui: &mut egui::Ui, it: &Item) -> bool {
        let ctx = ui.ctx().clone();
        let mut picked = false;
        let mut row = |ui: &mut egui::Ui, icon: Icon, label: &str| {
            let r = menu_row(ui, icon, label);
            picked |= r;
            r
        };
        let open_icon = if it.kind == Kind::Image { Icon::Region } else { Icon::Play };
        if row(ui, open_icon, l("открыть", "open")) {
            win::open_path(&it.path);
        }
        let edit = if it.kind == Kind::Image { l("править в blinker", "edit in blinker") } else { l("обрезать в blinker", "trim in blinker") };
        if it.kind != Kind::Other && row(ui, Icon::Edit, edit) {
            open(it);
        }
        menu_sep(ui);
        let copy = if it.kind == Kind::Image { l("копировать картинку", "copy the picture") } else { l("копировать файл", "copy the file") };
        if row(ui, Icon::Copy, copy) {
            let ok = match it.kind {
                Kind::Image => image::open(&it.path).map(|i| crate::output::copy_image(&i.to_rgba8())).unwrap_or(false),
                _ => crate::output::copy_file(&it.path),
            };
            self.notice = Some(Notice::new(&ctx, if ok { l("скопировано", "copied") } else { l("не скопировалось", "couldn't copy") }));
        }
        if crate::upload::configured(&self.cfg) && row(ui, Icon::Upload, l("загрузить и скопировать ссылку", "upload and copy the link")) {
            let job = crate::upload::start(&self.cfg, it.path.clone(), None);
            self.jobs.insert(it.path.clone(), job);
        }
        menu_sep(ui);
        if row(ui, Icon::Text, l("переименовать", "rename")) {
            self.start_rename(&it.path);
        }
        if row(ui, Icon::Folder, l("показать в папке", "show in the folder")) {
            win::reveal_in_explorer(&it.path);
        }
        if cfg!(windows) && row(ui, Icon::Trash, l("в корзину", "move to the bin")) {
            if win::trash(&it.path) {
                self.items.retain(|i| i.path != it.path);
                self.notice = Some(Notice::new(&ctx, l("в корзине: оттуда можно вернуть", "in the bin: it can be restored from there")));
            } else {
                self.notice = Some(Notice::new(&ctx, l("не удалилось", "couldn't delete")));
            }
        }
        picked
    }

    /// The name field of the file being renamed: Enter or a click elsewhere
    /// keeps the name, Esc drops it.
    fn rename_field(&mut self, ui: &mut egui::Ui, r: Rect) {
        let ctx = ui.ctx().clone();
        let Some((_, buf, focus)) = self.renaming.as_mut() else { return };
        let id = ui.scope_id().with("rename");
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
        let resp = child.add(
            egui::TextEdit::singleline(buf)
                .id(id)
                .font(font_medium(12.5))
                .text_color(TEXT)
                .desired_width(r.width())
                .margin(vec2(4.0, 2.0)),
        );
        if std::mem::take(focus) {
            resp.request_focus();
            // the whole name selected, ready to be typed over
            if let Some(mut st) = egui::text_edit::TextEditState::load(&ctx, id) {
                let n = buf.chars().count();
                st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
                st.store(&ctx, id);
            }
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.renaming = None;
        } else if resp.lost_focus() || !resp.has_focus() {
            self.finish_rename(&ctx);
        }
    }

    fn uploads_list(&mut self, ui: &mut egui::Ui) {
        if self.uploads.is_empty() {
            empty(
                ui,
                l("загрузок пока нет", "no uploads yet"),
                l("то, что ты загрузишь, появится тут: ссылку можно скопировать или удалить файл с сервера", "what you upload shows up here: copy the link or delete the file from the server"),
            );
            return;
        }
        let list = self.uploads.clone();
        ui::group(ui, "", |ui| {
            for u in &list {
                self.upload_row(ui, u);
            }
        });
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new(l("на post.blinker.cool файлы живут 3 дня, потом пропадают и отсюда", "files live 3 days on post.blinker.cool, then they're gone from here too"))
                .font(font(12.0))
                .color(FAINT),
        );
    }

    /// One upload: its picture (a click opens the file), the name, the link
    /// and how long it lives, then the link's buttons.
    fn upload_row(&mut self, ui: &mut egui::Ui, u: &Uploaded) {
        let name = u.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(u.at);
        let desc = format!("{} · {} · {}", day_title(at) + " " + &clock(at), human_size(u.size), left(u.expires));
        let here = u.path.exists();
        let tex = here.then(|| self.texture(ui.ctx(), &u.path)).flatten();
        let video = kind_of(&u.path) == Some(Kind::Video);
        let mut action = None;
        let state = self.deleting.get(&u.url).map(|s| s.lock().unwrap().clone());

        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 84.0), Sense::hover());
        let p = ui.painter().clone();
        // the picture
        let pic = Rect::from_min_size(row.min + vec2(12.0, 10.0), vec2(112.0, 64.0));
        p.rect_filled(pic, 8, CANVAS);
        match &tex {
            Some(t) => {
                let uv = ui::cover_uv(pic.size(), t.size_vec2());
                p.add(egui::epaint::RectShape::filled(pic, egui::CornerRadius::same(8), Color32::WHITE).with_texture(t.id(), uv));
            }
            None => icons::paint(&p, pic, if video { Icon::Video } else { Icon::Region }, 20.0, FAINT),
        }
        p.rect_stroke(pic, 8, Stroke::new(1.0, Color32::from_white_alpha(14)), StrokeKind::Inside);
        if video && tex.is_some() {
            ui::play_badge(&p, pic.center(), 13.0, 1.0);
        }
        // the name and the picture open the file (while it's on this computer)
        let text_left = pic.right() + 14.0;
        let open_r = Rect::from_min_max(pic.min, pos2(row.right() - 210.0, pic.bottom()));
        let open = ui.interact(open_r, ui.scope_id().with((&u.url, "open")), Sense::click());
        if here && open.hovered() {
            p.rect_stroke(pic, 8, Stroke::new(1.0, Color32::from_white_alpha(60)), StrokeKind::Inside);
        }
        let tip = if here { l("открыть файл", "open the file") } else { l("файла больше нет на этом компьютере", "the file is no longer on this computer") };
        if open.on_hover_cursor(if here { CursorIcon::PointingHand } else { CursorIcon::Default }).on_hover_text(tip).clicked() && here {
            win::open_path(&u.path);
        }
        let width = (row.right() - 214.0 - text_left).max(60.0);
        let line = |y: f32, text: &str, f: egui::FontId, c: Color32| {
            let g = p.layout(text.to_owned(), f, c, f32::INFINITY);
            p.with_clip_rect(Rect::from_min_size(pos2(text_left, y - 10.0), vec2(width, 20.0))).galley(pos2(text_left, y - g.size().y / 2.0), g, c);
        };
        line(pic.top() + 12.0, &name, font_medium(13.0), TEXT);
        line(pic.top() + 32.0, &u.url, mono(11.5), DIM);
        line(pic.top() + 51.0, &desc, font(11.5), FAINT);

        // the link's buttons
        let mut buttons = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(pos2(row.right() - 210.0, row.top()), pos2(row.right() - 10.0, row.bottom())))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        let bui = &mut buttons;
        bui.spacing_mut().item_spacing.x = 4.0;
        match &state {
            Some(None) => {
                bui.label(egui::RichText::new(l("удаляю…", "deleting…")).font(font(12.5)).color(DIM));
            }
            _ => {
                let confirm = self.confirm.as_deref() == Some(u.url.as_str());
                let (label, kind) = if confirm { (l("точно удалить?", "really delete?"), ui::Kind::Accent) } else { (l("удалить", "delete"), ui::Kind::Ghost) };
                let tip = if u.delete_url.is_some() { l("удалить файл с сервера", "delete the file from the server") } else { l("этот сервер не дал ссылку на удаление: уберётся только из списка", "this server gave no delete link: it only leaves the list") };
                if ui::button(bui, label, kind).on_hover_text(tip).clicked() {
                    action = Some(if confirm { 2 } else { 1 });
                }
                if ui::icon_button(bui, Icon::Link, l("открыть ссылку в браузере", "open the link in the browser"), 30.0, false, ui::Kind::Ghost).clicked() {
                    win::open_url(&u.url);
                }
                if ui::icon_button(bui, Icon::Copy, l("копировать ссылку", "copy the link"), 30.0, false, ui::Kind::Ghost).clicked() {
                    crate::output::copy_text(&u.url);
                    self.notice = Some(Notice::new(bui.ctx(), l("ссылка скопирована", "link copied")));
                }
            }
        }
        match action {
            Some(1) => self.confirm = Some(u.url.clone()),
            Some(_) => {
                self.confirm = None;
                let slot = Arc::new(Mutex::new(None));
                let (s2, u2, ctx) = (slot.clone(), u.clone(), ui.ctx().clone());
                std::thread::spawn(move || {
                    *s2.lock().unwrap() = Some(crate::upload::delete_upload(&u2));
                    ctx.request_repaint();
                });
                self.deleting.insert(u.url.clone(), slot);
            }
            None => {}
        }
        if let Some(Some(r)) = state {
            self.deleting.remove(&u.url);
            self.notice = Some(Notice::new(
                ui.ctx(),
                match r {
                    Ok(()) => l("удалено с сервера", "deleted from the server").to_string(),
                    Err(e) => lf!("не удалилось: {e}", "couldn't delete: {e}"),
                },
            ));
            self.uploads = crate::upload::uploads();
        }
    }
}

fn empty(ui: &mut egui::Ui, title: &str, sub: &str) {
    ui.add_space(90.0);
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(title).font(font_semibold(17.0)).color(TEXT));
        ui.add_space(6.0);
        ui.label(egui::RichText::new(sub).font(font(13.0)).color(DIM));
    });
}

impl eframe::App for HistoryApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if theme::glass() { [0.0; 4] } else { BG.to_normalized_gamma_f32() }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.show.swap(false, Ordering::SeqCst) {
            if let Ok(t) = std::fs::read_to_string(request_file()) {
                self.tab = Tab::from_arg(&t);
            }
            self.refresh();
            if let Some(h) = self.hwnd {
                win::force_foreground(h);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.top_bar(ui);
        // on glass the list is clear: the frosted window shows through
        let fill = if theme::glass() { Color32::TRANSPARENT } else { BG };
        egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(fill)).show(ui, |ui| {
            self.content = ui.max_rect();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                // the middle button held down drags the list
                if ui.input(|i| i.pointer.middle_down()) && ui.rect_contains_pointer(ui.clip_rect()) {
                    let d = ui.input(|i| i.pointer.delta());
                    ui.scroll_with_delta(vec2(0.0, d.y));
                    ui.ctx().set_cursor_icon(CursorIcon::AllScroll);
                }
                egui::Frame::new().inner_margin(egui::Margin { left: 22, right: 22, top: 14, bottom: 22 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    match self.tab {
                        Tab::Shots => self.grid(ui, frame, &[Kind::Image]),
                        Tab::Videos => self.grid(ui, frame, &[Kind::Video, Kind::Other]),
                        Tab::Uploads => self.uploads_list(ui),
                    }
                });
            });
        });
        self.menu(&ctx);
        chrome::frame_border(&ctx);
        if let Some(n) = &self.notice
            && !n.show(&ctx, self.content)
        {
            self.notice = None;
        }
        let _ = &self.shared;
    }
}

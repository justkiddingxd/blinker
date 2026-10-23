#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod beauty;
mod brand;
mod ocr;
mod paddle;
#[cfg_attr(target_os = "macos", path = "mac/capture.rs")]
mod capture;
mod chrome;
mod config;
mod demo;
mod discord;
mod editor;
mod games;
mod geom;
mod history;
mod home;
mod hotkey;
#[cfg_attr(target_os = "macos", path = "mac/media.rs")]
mod media;
mod record;
mod icons;
mod output;
mod overlay;
mod pin;
mod runtime;
#[cfg_attr(target_os = "macos", path = "mac/shell.rs")]
mod shell;
mod sharex;
mod sk;
mod sound;
mod theme;
mod toast;
mod tray;
#[cfg(windows)]
mod traymenu;
mod ui;
mod update;
mod upload;
mod vedit;
/// Platform layer: Win32 on Windows; on macOS the same API over AppKit,
/// CoreGraphics and ScreenCaptureKit (`src/mac`).
#[cfg_attr(target_os = "macos", path = "mac/mod.rs")]
mod win;

use editor::EditorState;
use image::RgbaImage;
use overlay::Action;
use runtime::{Command, HomeTab, Shared};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Process layout:
/// - the tray process (no flags / `--background`): tray, hotkeys, capture overlay, toast;
/// - `--settings [tab]`, `--edit <file> [--temp]`, `--pin <file> <x> <y>`: separate
///   lightweight processes, so they live independently of captures.
fn main() {
    win::init_process();
    media::mark_main_thread();
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let arg_after = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();

    if let Some(i) = args.iter().position(|a| a == "--pin") {
        if let Some(path) = args.get(i + 1) {
            let x = args.get(i + 2).and_then(|v| v.parse().ok()).unwrap_or(100);
            let y = args.get(i + 3).and_then(|v| v.parse().ok()).unwrap_or(100);
            pin::run(path.into(), x, y);
        }
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--text") {
        if let Some(path) = args.get(i + 1) {
            let x = args.get(i + 2).and_then(|v| v.parse().ok()).unwrap_or(100);
            let y = args.get(i + 3).and_then(|v| v.parse().ok()).unwrap_or(100);
            ocr::run(path.into(), x, y);
        }
        return;
    }
    if has("--quit") {
        win::signal_quit();
        return;
    }
    if let Some(dir) = arg_after("--demo") {
        let lang = arg_after(&dir).unwrap_or_default();
        if has("replay") {
            demo::run_replay(PathBuf::from(dir), &lang);
        } else {
            demo::run(PathBuf::from(dir), &lang, has("record"));
        }
        return;
    }
    // blinker://update (the "update" button in blinker's Discord): the
    // settings open and install the new version; blinker starts first if it
    // isn't running
    let url_update = args.iter().any(|a| a.trim_end_matches('/').eq_ignore_ascii_case("blinker://update"));
    if url_update && win::window_class_exists("BlinkerShell") {
        open_update();
        return;
    }
    if has("--settings") {
        run_settings(arg_after("--settings").as_deref());
        return;
    }
    if has("--history") {
        let tab = history::Tab::from_arg(arg_after("--history").as_deref().unwrap_or(""));
        let _ = std::fs::write(history::request_file(), tab.arg());
        if !win::signal_event(history::SHOW_EVENT) {
            history::run(&Shared::new(config::load()), tab);
        }
        return;
    }
    // for working on the card: `--toast picture.png` shows it for that file
    if let Some(path) = arg_after("--toast") {
        let Ok(img) = image::open(&path) else { return };
        let shared = runtime::Shared::new(config::load());
        let (x, y) = win::cursor_pos();
        let Some(mon) = win::monitor_at(&win::monitors(), x, y) else { return };
        let title = crate::i18n::t("скопировано").to_string();
        let badge = has("--clip").then_some(CLIPPED);
        toast::run(&shared, Arc::new(img.to_rgba8()), title, Some(PathBuf::from(path)), None, mon, toast::Kind::Image, None, badge);
        return;
    }
    if let Some(path) = arg_after("--video") {
        vedit::run(PathBuf::from(path));
        return;
    }
    if let Some(path) = arg_after("--edit") {
        run_editor_process(Path::new(&path), has("--temp"));
        return;
    }
    // `blinker.exe picture.png` ("Open with"): just the editor.
    if let Some(path) = args.iter().skip(1).find(|a| !a.starts_with("--")).map(PathBuf::from)
        && path.is_file()
    {
        run_editor_process(&path, false);
        return;
    }

    // after an update the old instance is still on its way out: wait for it
    let mut instance = win::acquire_instance();
    if has("--relaunch") {
        let t = std::time::Instant::now();
        while instance.is_none() && t.elapsed() < std::time::Duration::from_secs(15) {
            std::thread::sleep(std::time::Duration::from_millis(200));
            instance = win::acquire_instance();
        }
    }
    let Some(_instance) = instance else {
        win::signal_existing_instance();
        return;
    };
    update::clean_up();
    let cfg = config::load();
    win::register_url_scheme();
    if cfg.autostart {
        // Keep the Run entry pointing at the current executable.
        win::set_autostart(true);
    }
    let shared = Shared::new(cfg);
    let shell = shell::spawn(shared.clone());
    update::auto(shared.clone());
    discord::run(shared.clone());
    #[cfg(windows)]
    std::thread::spawn(win::ensure_start_menu_entry);

    // the first start next to ShareX: settings open where it is brought over
    let sharex = !shared.config().sharex_offered && sharex::detect().is_some();
    if sharex {
        shared.update_config(|c| c.sharex_offered = true);
    }
    if has("--capture") {
        shared.push(Command::CaptureRegion);
    } else if url_update {
        shared.push(Command::OpenUpdate);
    } else if sharex {
        shared.push(Command::OpenHome(HomeTab::Integrations));
    } else if !has("--background") {
        shared.push(Command::OpenHome(HomeTab::Capture));
    }

    let restart = controller(&shared);

    output::wait_pending();
    shell.shutdown();
    if !restart {
        win::signal_quit();
    }
    if restart {
        update::relaunch(shared.quiet_restart.load(std::sync::atomic::Ordering::SeqCst));
    }
}

// ---------------------------------------------------------------------------
// Child processes

fn spawn_self(args: &[&std::ffi::OsStr]) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).args(args).spawn();
    }
}

/// One history window: a running one is brought forward.
fn open_history(tab: history::Tab) {
    let _ = std::fs::write(history::request_file(), tab.arg());
    if !win::signal_event(history::SHOW_EVENT) {
        spawn_self(&["--history".as_ref(), tab.arg().as_ref()]);
    }
}

/// The settings, installing the new version.
fn open_update() {
    let _ = std::fs::write(home::request_file(), home::UPDATE_ARG);
    if !win::signal_event(home::SHOW_EVENT) {
        spawn_self(&["--settings".as_ref(), home::UPDATE_ARG.as_ref()]);
    }
}

/// One settings process: if it is running (shown or hidden), it is asked to show.
fn open_settings(tab: HomeTab) {
    let _ = std::fs::write(home::request_file(), home::tab_arg(tab));
    if !win::signal_event(home::SHOW_EVENT) {
        spawn_self(&["--settings".as_ref(), home::tab_arg(tab).as_ref()]);
    }
}

fn open_editor_file(path: &Path) {
    spawn_self(&["--edit".as_ref(), path.as_os_str()]);
}

/// Hands a fresh capture to a new editor process via a temp file.
fn open_editor_image(img: &RgbaImage) {
    let dir = config::cache_dir().join("edit");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("capture-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")));
    if output::write(img, &path, config::ImageFormat::Png, 90).is_ok() {
        spawn_self(&["--edit".as_ref(), path.as_os_str(), "--temp".as_ref()]);
    }
}

fn run_settings(tab: Option<&str>) {
    let raw = tab.unwrap_or("");
    let tab = home::tab_from_arg(raw);
    // "update" goes on as it is: the running settings start the update
    let _ = std::fs::write(home::request_file(), if raw == home::UPDATE_ARG { raw } else { home::tab_arg(tab) });
    if win::signal_event(home::SHOW_EVENT) {
        return;
    }
    let shared = Shared::new(config::load());
    home::run(&shared, tab);
}

/// `temp`: the file is a hand-off from the tray process, not a document to overwrite.
fn run_editor_process(path: &Path, temp: bool) {
    let Some(img) = output::load(path) else { return };
    if temp {
        let _ = std::fs::remove_file(path);
    }
    let shared = Shared::new(config::load());
    let cfg = shared.config();
    let doc = if temp { None } else { Some(path.to_path_buf()) };
    let st = EditorState::new(Arc::new(img), doc, cfg.editor_color, cfg.editor_width);
    editor::run(&shared, st, None);
}

// ---------------------------------------------------------------------------
// Tray process

/// Starts / stops / restarts the instant replay to match the settings.
fn sync_replay(shared: &Shared, replay: &mut Option<record::replay::Replay>) {
    let cfg = shared.config();
    let key = record::replay::config_key(&cfg);
    if let Some(r) = replay.as_ref()
        && r.key == key
    {
        r.set_secs(cfg.replay_secs);
        return;
    }
    if let Some(r) = replay.take() {
        r.stop();
    }
    if cfg.replay_enabled {
        match record::replay::Replay::start(&cfg) {
            Ok(r) => *replay = Some(r),
            Err(e) => config::log::error(format!("replay: {e}")),
        }
    }
    shared.set_replay_live(replay.is_some());
}

/// Runs until "quit"; true when the app should start again (after an update).
fn controller(shared: &Arc<Shared>) -> bool {
    let mut recording: Option<record::Recording> = None;
    let mut replay: Option<record::replay::Replay> = None;
    sync_replay(shared, &mut replay);
    loop {
        let cmd_now = shared.wait();
        // a capture never takes in the text window of the one before
        if cmd_now.is_capture() && cmd_now != Command::PickColor {
            ocr::close_open();
        }
        match cmd_now.clone() {
            Command::Quit | Command::Restart => {
                if let Some(rec) = recording.take() {
                    let _ = rec.stop();
                }
                if let Some(r) = replay.take() {
                    r.stop();
                }
                shared.set_replay_live(false);
                return cmd_now == Command::Restart;
            }
            Command::Reconfigure => sync_replay(shared, &mut replay),
            Command::RecordPause => {
                if let Some(rec) = &recording {
                    rec.toggle_pause();
                }
            }
            Command::ReplaySave => match &replay {
                Some(r) => match r.save(&shared.config()) {
                    Ok(f) => show_video_result(shared, f, crate::i18n::t("повтор"), Some(CLIPPED)),
                    Err(e) => win::message_box(crate::i18n::t("повтор не сохранился"), &e),
                },
                None => win::message_box(
                    crate::i18n::t("повтор выключен"),
                    crate::i18n::t("он включается в настройках, раздел «запись»"),
                ),
            },
            Command::Record | Command::RecordGif | Command::RecordScreen => {
                let gif = matches!(cmd_now, Command::RecordGif);
                if let Some(rec) = recording.take() {
                    shared.recording.store(false, std::sync::atomic::Ordering::SeqCst);
                    *shared.recording_note.lock().unwrap() = None;
                    finish_recording(shared, rec);
                    continue;
                }
                // the whole display under the pointer, or what's picked on the overlay
                let chosen = if cmd_now == Command::RecordScreen {
                    let (x, y) = win::cursor_pos();
                    win::monitor_at(&win::monitors(), x, y).map(|m| (record::Selection { rect: m.rect, monitor: m, window: None }, false))
                } else {
                    let Some(snap) = capture::grab_desktop() else { continue };
                    overlay::run(shared, snap, true, gif).map(|out| (record::Selection { rect: out.rect, monitor: out.monitor, window: out.window }, out.gif))
                };
                if let Some((sel, gif)) = chosen {
                    let (s2, s3) = (shared.clone(), shared.clone());
                    match record::start(
                        &shared.config(),
                        sel,
                        gif,
                        move || s2.push(Command::Record),
                        move || s3.push(Command::RecordPause),
                    ) {
                        Ok(r) => {
                            let cfg = shared.config();
                            let note = if gif {
                                format!("GIF · {} fps", cfg.gif_fps)
                            } else {
                                format!("{} × {} · {} fps", r.rect.w, r.rect.h, record::resolve_fps(&cfg, &r.monitor))
                            };
                            let start = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                            *shared.recording_note.lock().unwrap() = Some((start, note));
                            recording = Some(r);
                            shared.recording.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        Err(e) => {
                            config::log::error(format!("record: {e}"));
                            win::message_box(crate::i18n::t("запись не началась"), &e);
                        }
                    }
                }
            }
            Command::CaptureRegion => {
                let Some(snap) = capture::grab_desktop() else { continue };
                if let Some(out) = overlay::run(shared, snap, false, false) {
                    deliver(shared, out.image, out.action, out.rect, out.monitor);
                }
            }
            Command::CaptureText => {
                let Some(snap) = capture::grab_desktop() else { continue };
                if let Some(out) = overlay::text(shared, snap) {
                    deliver(shared, out.image, out.action, out.rect, out.monitor);
                }
            }
            Command::PickColor => {
                let Some(snap) = capture::grab_desktop() else { continue };
                overlay::pick(shared, snap);
            }
            Command::CaptureScreen => {
                let (x, y) = win::cursor_pos();
                let Some(mon) = win::monitor_at(&win::monitors(), x, y) else { continue };
                if let Some(img) = capture::grab_rect(mon.rect) {
                    sound::play_capture(&shared.config());
                    deliver(shared, img, Action::Copy, mon.rect, mon);
                }
            }
            Command::CaptureWindow => {
                let Some(r) = capture::foreground_window_rect() else { continue };
                if let Some(img) = capture::grab_rect(r) {
                    sound::play_capture(&shared.config());
                    let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
                    let Some(mon) = win::monitor_at(&win::monitors(), cx, cy) else { continue };
                    deliver(shared, img, Action::Copy, r, mon);
                }
            }
            Command::CaptureLast => {
                let last = *shared.last_region.lock().unwrap();
                match last {
                    Some(r) => {
                        if let Some(img) = capture::grab_rect(r) {
                            sound::play_capture(&shared.config());
                            let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
                            let Some(mon) = win::monitor_at(&win::monitors(), cx, cy) else { continue };
                            deliver(shared, img, Action::Copy, r, mon);
                        }
                    }
                    // Nothing to repeat yet: fall back to a normal region capture.
                    None => shared.push(Command::CaptureRegion),
                }
            }
            Command::OpenHome(tab) => open_settings(tab),
            Command::OpenUpdate => open_update(),
            Command::OpenHistory => open_history(history::Tab::Shots),
            Command::OpenFolder => {
                let dir = shared.config().save_dir;
                let _ = std::fs::create_dir_all(&dir);
                win::open_path(&dir);
            }
        }
    }
}

/// Stops a recording, then shows the toast (or opens the video editor).
fn finish_recording(shared: &Arc<Shared>, rec: record::Recording) {
    let Some(f) = rec.stop() else {
        win::message_box(crate::i18n::t("запись не сохранилась"), crate::i18n::t("не удалось дописать файл."));
        return;
    };
    if f.gif {
        finish_gif(shared, f);
    } else {
        show_video_result(shared, f, crate::i18n::t("запись"), None);
    }
}

/// Converts a GIF recording in the background; the card shows it as saving
/// and the finished GIF goes to the clipboard.
fn finish_gif(shared: &Arc<Shared>, f: record::Finished) {
    discord::count(true);
    let cfg = shared.config();
    let dst = record::clip_path(&cfg, "blinker", "gif", record::game_now().as_deref());
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let (src, dst, done) = (f.path.clone(), dst.clone(), done.clone());
        let width = f.thumb.as_ref().map(|t| t.width()).unwrap_or(cfg.gif_width).min(cfg.gif_width.max(160));
        let fps = cfg.gif_fps.clamp(5, 30);
        let duration = (f.duration * record::encoder::HNS as f64) as i64;
        std::thread::spawn(move || {
            media::startup();
            let opt = vedit::export::Options {
                kind: vedit::export::Kind::Gif,
                from: 0,
                to: duration,
                tracks: Vec::new(),
                mix_down: false,
                bitrate_quality: config::RecQuality::Balanced,
                gif_width: width,
                gif_fps: fps,
                quality: None,
                codec: record::encoder::Codec::H264,
                aspect: vedit::shorts::Aspect::Original,
                crop_x: 0.5,
                captions: Vec::new(),
                style: Default::default(),
                titles: Vec::new(),
                transform: Default::default(),
                keep: Vec::new(),
                stickers: Vec::new(),
                music: None,
            };
            let (p, c) = (std::sync::atomic::AtomicU32::new(0), std::sync::atomic::AtomicBool::new(false));
            match vedit::export::gif(&src, &dst, &opt, &p, &c) {
                Ok(()) => {
                    output::copy_file(&dst);
                }
                Err(e) => config::log::error(format!("gif: {e}")),
            }
            let _ = std::fs::remove_file(&src);
            done.store(true, std::sync::atomic::Ordering::Release);
        });
    }
    if !cfg.show_toast || shared.peek().is_some() {
        return;
    }
    let thumb = Arc::new(f.thumb.unwrap_or_else(|| RgbaImage::from_pixel(16, 9, image::Rgba([22, 22, 21, 255]))));
    let secs = f.duration.round() as u64;
    let title = format!("gif · {}:{:02}", secs / 60, secs % 60);
    let _ = toast::run(shared, thumb, title, Some(dst), Some(done), f.monitor, toast::Kind::Gif, None, None);
}

/// Toast (or the editor) for a finished recording / saved replay.
/// Over the card of a saved instant replay.
const CLIPPED: &str = "Clipped!";

/// `badge`: a word over the card (a saved replay is "Clipped!").
fn show_video_result(shared: &Arc<Shared>, f: record::Finished, what: &str, badge: Option<&'static str>) {
    discord::count(true);
    let cfg = shared.config();
    if cfg.rec_open_editor {
        open_video_editor(&f.path);
        return;
    }
    if !cfg.show_toast || shared.peek().is_some() {
        return;
    }
    let thumb = Arc::new(f.thumb.unwrap_or_else(|| RgbaImage::from_pixel(16, 9, image::Rgba([22, 22, 21, 255]))));
    let secs = f.duration.round() as u64;
    let title = format!("{what} · {}:{:02}", secs / 60, secs % 60);
    let saved = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let cfg = shared.config();
    let job = (cfg.upload_auto && upload::configured(&cfg)).then(|| upload::start(&cfg, f.path.clone(), None));
    if toast::run(shared, thumb, title, Some(f.path.clone()), Some(saved), f.monitor, toast::Kind::Video, job, badge)
        == toast::ToastResult::Edit
    {
        open_video_editor(&f.path);
    }
}

fn open_video_editor(path: &Path) {
    spawn_self(&["--video".as_ref(), path.as_os_str()]);
}

/// Clipboard / file / toast for a finished capture.
fn deliver(shared: &Arc<Shared>, image: RgbaImage, action: Action, rect: win::IRect, monitor: win::Monitor) {
    *shared.last_region.lock().unwrap() = Some(rect);
    discord::count(false);
    let cfg = shared.config();
    let img = Arc::new(if action == Action::Beauty { beauty::render(&image, &cfg.beauty) } else { image });
    match action {
        Action::Edit => {
            open_editor_image(&img);
            return;
        }
        Action::Pin => {
            pin::spawn(&img, rect);
            return;
        }
        Action::Text => {
            ocr::spawn(&img, rect);
            return;
        }
        Action::Record => return,
        Action::Copy | Action::Save | Action::Beauty | Action::Upload => {}
    }
    let uploading = action == Action::Upload || (cfg.upload_auto && upload::configured(&cfg));
    // An uploaded capture puts the link on the clipboard, not the picture.
    let copied = action != Action::Upload
        && !(uploading && cfg.upload_auto)
        && (action != Action::Save || cfg.copy_on_save)
        && output::copy_image(&img);
    let saved = (action == Action::Save || cfg.auto_save || uploading).then(|| {
        if action == Action::Save || cfg.auto_save {
            output::save_async(img.clone(), &cfg)
        } else {
            // not kept in the library: a temp file just for the upload
            let dir = config::cache_dir().join("upload");
            let _ = std::fs::create_dir_all(&dir);
            let p = dir.join(chrono::Local::now().format("blinker %Y-%m-%d %H-%M-%S.png").to_string());
            let ok = output::write(&img, &p, config::ImageFormat::Png, 90).is_ok();
            (p, Arc::new(std::sync::atomic::AtomicBool::new(ok)))
        }
    });
    let (path, flag) = match &saved {
        Some((p, f)) => (Some(p.clone()), Some(f.clone())),
        None => (None, None),
    };
    let job = match (&path, uploading) {
        (Some(p), true) => Some(upload::start(&cfg, p.clone(), flag.clone())),
        _ => None,
    };
    let title = match (action, copied) {
        _ if job.is_some() => crate::i18n::t("загрузка"),
        (Action::Beauty, _) => crate::i18n::t("красивый снимок"),
        (Action::Save, true) => crate::i18n::t("сохранено и скопировано"),
        (Action::Save, false) => crate::i18n::t("сохранено"),
        _ => crate::i18n::t("скопировано"),
    };
    let toast_path = if cfg.auto_save || action == Action::Save { path.clone() } else { None };
    if cfg.show_toast
        && shared.peek().is_none()
        && toast::run(
            shared,
            img.clone(),
            format!("{title} · {} × {}", img.width(), img.height()),
            toast_path.clone(),
            flag,
            monitor,
            toast::Kind::Image,
            job,
            None,
        ) == toast::ToastResult::Edit
    {
        match toast_path {
            Some(p) => {
                output::wait_pending();
                open_editor_file(&p);
            }
            None => open_editor_image(&img),
        }
    }
}

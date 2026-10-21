//! Moving over from ShareX: its settings (hotkeys, the screenshot folder and
//! format, what happens after a capture, the notification, the sound, the
//! custom uploader) into ours, in one go.
//!
//! ShareX keeps them as JSON in its personal folder (`Documents\ShareX`, or
//! the one `PersonalPath.cfg` next to ShareX.exe points to); an exported
//! backup (`.sxb`) is a zip of the same files. `.sxcu` files are single
//! custom uploaders.

use crate::config::{Config, Corner, CustomUploader, HotkeyAction, ImageFormat};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Where ShareX settings were found.
#[derive(Clone, Debug)]
pub enum Source {
    Folder(PathBuf),
    /// An exported backup (zip).
    Backup(PathBuf),
}

impl Source {
    pub fn path(&self) -> &Path {
        match self {
            Source::Folder(p) | Source::Backup(p) => p,
        }
    }
}

/// The ShareX personal folder on this computer, if it has one.
pub fn detect() -> Option<Source> {
    let mut candidates = Vec::new();
    #[cfg(windows)]
    for dir in [std::env::var_os("ProgramFiles"), std::env::var_os("ProgramFiles(x86)")].into_iter().flatten() {
        // a custom personal folder is written next to ShareX.exe
        if let Ok(p) = std::fs::read_to_string(PathBuf::from(dir).join("ShareX").join("PersonalPath.cfg")) {
            let p = p.trim().trim_matches('"');
            if !p.is_empty() {
                candidates.push(PathBuf::from(expand_env(p)));
            }
        }
    }
    if let Some(docs) = dirs::document_dir() {
        candidates.push(docs.join("ShareX"));
    }
    candidates.into_iter().find(|d| d.join("ApplicationConfig.json").is_file()).map(Source::Folder)
}

#[cfg(windows)]
fn expand_env(p: &str) -> String {
    let mut out = p.to_string();
    for (k, v) in std::env::vars() {
        out = out.replace(&format!("%{k}%"), &v);
    }
    out
}

/// What a file picked by the user holds: a backup, the personal folder (or
/// a file inside it).
pub fn source_of(path: &Path) -> Option<Source> {
    if path.is_dir() {
        return path.join("ApplicationConfig.json").is_file().then(|| Source::Folder(path.to_path_buf()));
    }
    match path.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("sxb" | "zip") => Some(Source::Backup(path.to_path_buf())),
        Some("json") => path.parent().and_then(source_of),
        _ => None,
    }
}

struct Files {
    app: Option<Value>,
    hotkeys: Option<Value>,
    uploaders: Option<Value>,
}

fn parse(text: &str) -> Option<Value> {
    // ShareX writes a byte order mark at the start
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

fn read(src: &Source) -> Result<Files, String> {
    match src {
        Source::Folder(dir) => {
            let get = |name: &str| std::fs::read_to_string(dir.join(name)).ok().and_then(|t| parse(&t));
            Ok(Files { app: get("ApplicationConfig.json"), hotkeys: get("HotkeysConfig.json"), uploaders: get("UploadersConfig.json") })
        }
        Source::Backup(file) => {
            let f = std::fs::File::open(file).map_err(|e| e.to_string())?;
            let mut zip = zip::ZipArchive::new(f).map_err(|_| crate::i18n::l("это не архив настроек sharex", "this isn't a sharex settings backup").to_string())?;
            let mut get = |name: &str| -> Option<Value> {
                let idx = (0..zip.len()).find(|&i| zip.name_for_index(i).is_some_and(|n| n.rsplit(['/', '\\']).next() == Some(name)))?;
                let mut text = String::new();
                std::io::Read::read_to_string(&mut zip.by_index(idx).ok()?, &mut text).ok()?;
                parse(&text)
            };
            Ok(Files { app: get("ApplicationConfig.json"), hotkeys: get("HotkeysConfig.json"), uploaders: get("UploadersConfig.json") })
        }
    }
}

/// What was brought over, for the message after the import.
#[derive(Default, Debug)]
pub struct Report {
    pub items: Vec<String>,
}

impl Report {
    pub fn summary(&self) -> String {
        if self.items.is_empty() {
            crate::i18n::l("в настройках sharex нечего перенести", "nothing to bring over from sharex").to_string()
        } else {
            lf!("перенесено: {}", "brought over: {}", self.items.join(", "))
        }
    }
}

/// Fills `cfg` from the ShareX settings in `src`.
pub fn import(src: &Source, cfg: &mut Config) -> Result<Report, String> {
    let files = read(src)?;
    if files.app.is_none() && files.hotkeys.is_none() && files.uploaders.is_none() {
        return Err(crate::i18n::l("настройки sharex не найдены", "no sharex settings found").into());
    }
    let mut report = Report::default();
    if let Some(app) = &files.app {
        apply_app(app, cfg, &mut report);
    }
    // ShareX is a Windows app: its keys mean something on a Windows keyboard
    if cfg!(windows)
        && let Some(hk) = &files.hotkeys
    {
        apply_hotkeys(hk, cfg, &mut report);
    }
    if let Some(up) = &files.uploaders {
        let image_dest = files.app.as_ref().and_then(|a| a.pointer("/DefaultTaskSettings/ImageDestination")).and_then(Value::as_str);
        apply_uploader(up, image_dest, cfg, &mut report);
    }
    Ok(report)
}

fn str_at<'a>(v: &'a Value, path: &str) -> Option<&'a str> {
    v.pointer(path).and_then(Value::as_str)
}

fn apply_app(app: &Value, cfg: &mut Config, report: &mut Report) {
    let task = |p: &str| app.pointer(&format!("/DefaultTaskSettings{p}"));
    // the screenshot folder, when ShareX had its own
    if app.pointer("/UseCustomScreenshotsPath").and_then(Value::as_bool) == Some(true)
        && let Some(dir) = str_at(app, "/CustomScreenshotsPath").filter(|d| !d.trim().is_empty())
    {
        let dir = PathBuf::from(dir.trim());
        if dir.is_dir() {
            cfg.save_dir = dir;
            report.items.push(crate::i18n::l("папка", "folder").into());
        }
    }
    if let Some(f) = task("/ImageSettings/ImageFormat").and_then(Value::as_str) {
        cfg.format = match f.to_ascii_uppercase().as_str() {
            "JPEG" | "JPG" => ImageFormat::Jpeg,
            "WEBP" => ImageFormat::Webp,
            _ => ImageFormat::Png,
        };
        if let Some(q) = task("/ImageSettings/ImageJPEGQuality").and_then(Value::as_u64) {
            cfg.jpeg_quality = q.clamp(50, 100) as u8;
        }
        report.items.push(lf!("формат {}", "{} format", cfg.format.extension()));
    }
    if let Some(jobs) = task("/AfterCaptureJob").and_then(Value::as_str) {
        let has = |j: &str| jobs.split(',').any(|x| x.trim() == j);
        cfg.auto_save = has("SaveImageToFile") || has("SaveImageToFileWithDialog");
        cfg.copy_on_save = has("CopyImageToClipboard");
        cfg.upload_auto = has("UploadImageToHost");
        report.items.push(crate::i18n::l("что делать после снимка", "what happens after a capture").into());
    }
    if let Some(sound) = task("/GeneralSettings/PlaySoundAfterCapture").and_then(Value::as_bool) {
        cfg.sound_enabled = sound;
        report.items.push(crate::i18n::l("звук", "sound").into());
    }
    let popup = task("/GeneralSettings/PopUpNotification").and_then(Value::as_str);
    let toast = task("/GeneralSettings/ShowToastNotificationAfterTaskCompleted").and_then(Value::as_bool);
    if popup.is_some() || toast.is_some() {
        cfg.show_toast = popup.map(|p| p != "None").or(toast).unwrap_or(true);
        if let Some(d) = task("/GeneralSettings/ToastWindowDuration").and_then(Value::as_f64) {
            cfg.toast_seconds = (d as f32).round().clamp(2.0, 15.0);
        }
        if let Some(place) = task("/GeneralSettings/ToastWindowPlacement").and_then(Value::as_str) {
            cfg.toast_corner = match place {
                "TopLeft" => Corner::TopLeft,
                "TopRight" | "TopCenter" => Corner::TopRight,
                "BottomLeft" => Corner::BottomLeft,
                _ => Corner::BottomRight,
            };
        }
        report.items.push(crate::i18n::l("уведомление", "notification").into());
    }
    if let Some(cursor) = task("/CaptureSettings/ScreenRecordShowCursor").and_then(Value::as_bool) {
        cfg.rec_cursor = cursor;
    }
    if let Some(fps) = task("/CaptureSettings/ScreenRecordFPS").and_then(Value::as_u64) {
        cfg.rec_fps = nearest(fps as u32, &[30, 60, 120]);
        report.items.push(lf!("запись {} fps", "recording {} fps", cfg.rec_fps));
    }
    if let Some(fps) = task("/CaptureSettings/GIFFPS").and_then(Value::as_u64) {
        cfg.gif_fps = nearest(fps as u32, &[10, 15, 24]);
    }
}

fn nearest(v: u32, of: &[u32]) -> u32 {
    *of.iter().min_by_key(|x| x.abs_diff(v)).unwrap_or(&v)
}

/// ShareX task names for our actions.
fn action_of(job: &str) -> Option<HotkeyAction> {
    Some(match job {
        "RectangleRegion" | "RectangleLight" | "RectangleTransparent" => HotkeyAction::Region,
        "PrintScreen" => HotkeyAction::Screen,
        "ActiveWindow" => HotkeyAction::Window,
        "LastRegion" => HotkeyAction::Last,
        "ScreenRecorder" | "StartScreenRecorder" => HotkeyAction::Record,
        "ScreenRecorderGIF" | "StartScreenRecorderGIF" => HotkeyAction::RecordGif,
        _ => return None,
    })
}

/// A .NET `Keys` value as ShareX writes it ("PrintScreen, Control", "D4,
/// Shift, Control") into ours ("Ctrl+Shift+4"). The key codes are Windows
/// virtual keys in both.
pub fn hotkey_of(keys: &str, win: bool) -> Option<String> {
    #[cfg(windows)]
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};
    #[cfg(target_os = "macos")]
    use crate::win::vk::{MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};
    let mut mods = if win { MOD_WIN } else { 0 };
    let mut vk = None;
    for part in keys.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part {
            "Control" | "ControlKey" => mods |= MOD_CONTROL,
            "Shift" | "ShiftKey" => mods |= MOD_SHIFT,
            "Alt" | "Menu" => mods |= MOD_ALT,
            "None" => {}
            key => vk = Some(dotnet_key(key)?),
        }
    }
    vk.map(|vk| crate::hotkey::format(crate::hotkey::Combo { mods, vk }))
}

fn dotnet_key(name: &str) -> Option<u32> {
    let named = match name {
        "PrintScreen" | "Snapshot" => 0x2C,
        "Pause" => 0x13,
        "Scroll" => 0x91,
        "Insert" => 0x2D,
        "Delete" => 0x2E,
        "Home" => 0x24,
        "End" => 0x23,
        "PageUp" | "Prior" => 0x21,
        "PageDown" | "Next" => 0x22,
        "Left" => 0x25,
        "Up" => 0x26,
        "Right" => 0x27,
        "Down" => 0x28,
        "Space" => 0x20,
        "Tab" => 0x09,
        "Return" | "Enter" => 0x0D,
        "Escape" => 0x1B,
        "Back" => 0x08,
        "Oemtilde" | "Oem3" => 0xC0,
        "OemMinus" => 0xBD,
        "Oemplus" => 0xBB,
        "OemOpenBrackets" | "Oem4" => 0xDB,
        "OemCloseBrackets" | "Oem6" => 0xDD,
        "OemSemicolon" | "Oem1" => 0xBA,
        "OemQuotes" | "Oem7" => 0xDE,
        "Oemcomma" => 0xBC,
        "OemPeriod" => 0xBE,
        "OemQuestion" | "Oem2" => 0xBF,
        "OemPipe" | "Oem5" => 0xDC,
        _ => 0,
    };
    if named != 0 {
        return Some(named);
    }
    let b = name.as_bytes();
    match b {
        [c] if c.is_ascii_uppercase() => Some(*c as u32),
        [b'D', d] if d.is_ascii_digit() => Some(*d as u32),
        _ if name.starts_with("NumPad") => name[6..].parse::<u32>().ok().filter(|n| *n <= 9).map(|n| 0x60 + n),
        _ if name.starts_with('F') => name[1..].parse::<u32>().ok().filter(|n| (1..=24).contains(n)).map(|n| 0x6F + n),
        _ => None,
    }
}

fn apply_hotkeys(hk: &Value, cfg: &mut Config, report: &mut Report) {
    let Some(list) = hk.get("Hotkeys").and_then(Value::as_array) else { return };
    let mut found: Vec<(HotkeyAction, String)> = Vec::new();
    for h in list {
        let Some(job) = h.pointer("/TaskSettings/Job").and_then(Value::as_str) else { continue };
        let Some(action) = action_of(job) else { continue };
        let Some(keys) = h.pointer("/HotkeyInfo/Hotkey").and_then(Value::as_str) else { continue };
        let win = h.pointer("/HotkeyInfo/Win").and_then(Value::as_bool).unwrap_or(false);
        if let Some(binding) = hotkey_of(keys, win) {
            found.push((action, binding));
        }
    }
    if found.is_empty() {
        return;
    }
    // the actions ShareX had keys for get exactly those keys
    for a in HotkeyAction::ALL {
        if found.iter().any(|(x, _)| *x == a) {
            cfg.bindings_mut(a).clear();
        }
    }
    for (a, b) in &found {
        for other in HotkeyAction::ALL {
            cfg.bindings_mut(other).retain(|x| x != b);
        }
        cfg.bindings_mut(*a).push(b.clone());
    }
    report.items.push(lf!("горячие клавиши ({})", "hotkeys ({})", found.len()));
}

fn apply_uploader(up: &Value, image_dest: Option<&str>, cfg: &mut Config, report: &mut Report) {
    let Some(list) = up.get("CustomUploadersList").and_then(Value::as_array).filter(|l| !l.is_empty()) else { return };
    // a custom uploader matters when ShareX sent images to one (or had nothing else)
    if image_dest.is_some_and(|d| d != "CustomImageUploader") {
        return;
    }
    let i = up.get("CustomImageUploaderSelected").and_then(Value::as_u64).unwrap_or(0) as usize;
    if let Some(c) = list.get(i).or(list.first()).and_then(uploader_of) {
        report.items.push(lf!("загрузчик «{}»", "the “{}” uploader", if c.name.is_empty() { &c.url } else { &c.name }));
        cfg.upload_custom = Some(c);
    }
}

/// One ShareX custom uploader (an entry of the list, or a whole .sxcu file).
pub fn uploader_of(v: &Value) -> Option<CustomUploader> {
    let url = v.get("RequestURL").and_then(Value::as_str)?.trim().to_string();
    if url.is_empty() {
        return None;
    }
    let pairs = |key: &str| -> Vec<(String, String)> {
        v.get(key)
            .and_then(Value::as_object)
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))).collect())
            .unwrap_or_default()
    };
    Some(CustomUploader {
        name: v.get("Name").and_then(Value::as_str).unwrap_or("").to_string(),
        method: v.get("RequestMethod").or(v.get("RequestType")).and_then(Value::as_str).unwrap_or("POST").to_string(),
        url,
        parameters: pairs("Parameters"),
        headers: pairs("Headers"),
        body: v.get("Body").and_then(Value::as_str).unwrap_or("MultipartFormData").to_string(),
        file_form_name: v.get("FileFormName").and_then(Value::as_str).unwrap_or("file").to_string(),
        arguments: pairs("Arguments"),
        url_template: v.get("URL").and_then(Value::as_str).unwrap_or("").to_string(),
    })
}

/// A `.sxcu` file: one custom uploader.
pub fn read_sxcu(path: &Path) -> Result<CustomUploader, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse(&text)
        .as_ref()
        .and_then(uploader_of)
        .ok_or_else(|| crate::i18n::l("в файле нет загрузчика sharex", "no sharex uploader in this file").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys() {
        let key = |k: &str| hotkey_of(k, false).and_then(|s| crate::hotkey::parse(&s));
        assert_eq!(key("PrintScreen").map(|c| c.vk), Some(0x2C));
        let c = key("D4, Shift, Control").unwrap();
        assert_eq!(c.vk, 0x34);
        assert_ne!(c.mods, 0);
        assert_eq!(key("F13").map(|c| c.vk), Some(0x7C));
        assert_eq!(key("NumPad7").map(|c| c.vk), Some(0x67));
        assert_eq!(key("None"), None);
    }

    #[test]
    fn whole_import() {
        let dir = std::env::temp_dir().join(format!("blinker-sharex-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("ApplicationConfig.json"),
            "\u{feff}{\"DefaultTaskSettings\":{\"ImageDestination\":\"CustomImageUploader\",\"AfterCaptureJob\":\"CopyImageToClipboard, UploadImageToHost\",\
             \"ImageSettings\":{\"ImageFormat\":\"JPEG\",\"ImageJPEGQuality\":85},\"GeneralSettings\":{\"PlaySoundAfterCapture\":true,\
             \"PopUpNotification\":\"ToastNotification\",\"ToastWindowDuration\":5.0,\"ToastWindowPlacement\":\"TopLeft\"},\
             \"CaptureSettings\":{\"ScreenRecordFPS\":30,\"GIFFPS\":15}}}",
        )
        .unwrap();
        std::fs::write(
            dir.join("HotkeysConfig.json"),
            r#"{"Hotkeys":[{"HotkeyInfo":{"Hotkey":"PrintScreen, Control","Win":false},"TaskSettings":{"Job":"RectangleRegion"}},
               {"HotkeyInfo":{"Hotkey":"None","Win":false},"TaskSettings":{"Job":"PrintScreen"}}]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("UploadersConfig.json"),
            r#"{"CustomUploadersList":[{"Name":"zipline","RequestMethod":"POST","RequestURL":"https://z.example/api/upload",
               "Headers":{"Authorization":"abc"},"Body":"MultipartFormData","FileFormName":"file","URL":"{json:files[0]}"}],
               "CustomImageUploaderSelected":0}"#,
        )
        .unwrap();
        let mut cfg = Config::default();
        let r = import(&source_of(&dir).unwrap(), &mut cfg).unwrap();
        eprintln!("{}", r.summary());
        assert_eq!(cfg.format, ImageFormat::Jpeg);
        assert_eq!(cfg.jpeg_quality, 85);
        assert!(cfg.sound_enabled && cfg.upload_auto && cfg.copy_on_save && !cfg.auto_save);
        assert_eq!(cfg.toast_corner, Corner::TopLeft);
        assert_eq!(cfg.rec_fps, 30);
        let up = cfg.upload_custom.clone().unwrap();
        assert_eq!(up.name, "zipline");
        assert_eq!(up.headers, vec![("Authorization".to_string(), "abc".to_string())]);
        if cfg!(windows) {
            assert_eq!(cfg.hotkey_region, vec!["Ctrl+PrintScreen".to_string()]);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

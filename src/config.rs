use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Png,
    Webp,
    Jpeg,
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Webp => "webp",
            ImageFormat::Jpeg => "jpg",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "png" => Some(ImageFormat::Png),
            "webp" => Some(ImageFormat::Webp),
            "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SoundKind {
    Shutter,
    Click,
    Blink,
    Drop,
    Custom,
}

impl SoundKind {
    pub const BUILTIN: [SoundKind; 4] = [SoundKind::Shutter, SoundKind::Click, SoundKind::Blink, SoundKind::Drop];
    pub fn label(self) -> &'static str {
        match self {
            SoundKind::Shutter => crate::i18n::t("затвор"),
            SoundKind::Click => crate::i18n::t("щелчок"),
            SoundKind::Blink => crate::i18n::t("блинк"),
            SoundKind::Drop => crate::i18n::t("капля"),
            SoundKind::Custom => crate::i18n::t("свой файл"),
        }
    }
}

/// What a left click on the tray icon does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrayAction {
    Region,
    Screen,
    Window,
    Last,
    Record,
    Settings,
    Menu,
    Nothing,
}

impl TrayAction {
    pub const ALL: [TrayAction; 8] = [
        TrayAction::Region,
        TrayAction::Screen,
        TrayAction::Window,
        TrayAction::Last,
        TrayAction::Record,
        TrayAction::Settings,
        TrayAction::Menu,
        TrayAction::Nothing,
    ];
    pub fn label(self) -> &'static str {
        match self {
            TrayAction::Region => crate::i18n::t("снимок области"),
            TrayAction::Screen => crate::i18n::t("весь экран"),
            TrayAction::Window => crate::i18n::t("активное окно"),
            TrayAction::Last => crate::i18n::t("повторить область"),
            TrayAction::Record => crate::i18n::t("запись экрана"),
            TrayAction::Settings => crate::i18n::t("открыть настройки"),
            TrayAction::Menu => crate::i18n::t("показать меню"),
            TrayAction::Nothing => crate::i18n::t("ничего"),
        }
    }
}

/// Video quality preset: mean bitrate per pixel per frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecQuality {
    Compact,
    Balanced,
    High,
}

impl RecQuality {
    pub const ALL: [RecQuality; 3] = [RecQuality::Compact, RecQuality::Balanced, RecQuality::High];
    pub fn label(self) -> &'static str {
        match self {
            RecQuality::Compact => crate::i18n::t("компактно"),
            RecQuality::Balanced => crate::i18n::t("баланс"),
            RecQuality::High => crate::i18n::t("высокое"),
        }
    }
    /// Constant-quality level for the encoder (1..100, like CRF): the file
    /// grows only with what actually moves on screen.
    pub fn quality(self) -> u32 {
        match self {
            RecQuality::Compact => 45,
            RecQuality::Balanced => 60,
            RecQuality::High => 72,
        }
    }

    /// Typical MB per minute of Full HD desktop work (measured: documents,
    /// browser, scrolling with pauses).
    pub fn typical_mb_min(self) -> f32 {
        match self {
            RecQuality::Compact => 4.0,
            RecQuality::Balanced => 5.5,
            RecQuality::High => 6.5,
        }
    }

    /// Bitrate ceiling / fallback mean bitrate, bits per second.
    pub fn bitrate(self, w: u32, h: u32, fps: u32) -> u32 {
        let bpp = match self {
            RecQuality::Compact => 0.035,
            RecQuality::Balanced => 0.06,
            RecQuality::High => 0.11,
        };
        // fps counts sub-linearly: at 60 fps consecutive frames differ less.
        let f = (fps as f64).powf(0.75) * 30f64.powf(0.25);
        ((w as f64 * h as f64 * f * bpp) as u32).clamp(800_000, 50_000_000)
    }
}

/// Where the drawing tools panel sits while selecting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolsPos {
    /// Top of the screen.
    Screen,
    Above,
    Below,
    /// Vertical, right of the selection.
    Right,
}

impl ToolsPos {
    pub const ALL: [ToolsPos; 4] = [ToolsPos::Screen, ToolsPos::Above, ToolsPos::Below, ToolsPos::Right];
    pub fn label(self) -> &'static str {
        match self {
            ToolsPos::Screen => crate::i18n::l("сверху экрана", "top of the screen"),
            ToolsPos::Above => crate::i18n::l("над областью", "above the area"),
            ToolsPos::Below => crate::i18n::l("под областью", "below the area"),
            ToolsPos::Right => crate::i18n::l("справа от области", "right of the area"),
        }
    }
}

/// Codec of recordings and replays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    H264,
    Hevc,
}

/// Re-encode preset "for Telegram" (see `reencode.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TgMode {
    Balanced,
    Min,
    Hq,
    Hevc,
}

/// Canvas proportions of a "beautiful" shot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Aspect {
    Auto,
    Wide,
    Square,
    Portrait,
}

impl Aspect {
    pub const ALL: [Aspect; 4] = [Aspect::Auto, Aspect::Wide, Aspect::Square, Aspect::Portrait];
    pub fn label(self) -> &'static str {
        match self {
            Aspect::Auto => crate::i18n::t("по снимку"),
            Aspect::Wide => "16:9",
            Aspect::Square => "1:1",
            Aspect::Portrait => "4:5",
        }
    }
}

/// Settings of the "beautiful shot" framing (see `beauty.rs`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Beauty {
    /// Index into `beauty::BACKGROUNDS`.
    pub background: usize,
    /// Padding as a share of the shot's longer side.
    pub padding: f32,
    /// Corner radius, px at 1200 px.
    pub radius: f32,
    /// 0..1
    pub shadow: f32,
    pub aspect: Aspect,
}

impl Default for Beauty {
    fn default() -> Self {
        Self { background: 0, padding: 0.07, radius: 12.0, shadow: 0.6, aspect: Aspect::Auto }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToastSize {
    Small,
    Medium,
    Large,
}

impl ToastSize {
    /// Card width in points.
    pub fn width(self) -> f32 {
        match self {
            ToastSize::Small => 220.0,
            ToastSize::Medium => 300.0,
            ToastSize::Large => 400.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub save_dir: PathBuf,
    /// Interface language: "ru", "en" or "" (system).
    pub lang: String,
    /// Lay files out as `save_dir/2026-10-20/pics` and `.../videos`.
    pub sort_by_date: bool,
    pub format: ImageFormat,
    pub jpeg_quality: u8,
    /// Save every capture to `save_dir` (feeds the library).
    pub auto_save: bool,
    /// Also put the image on the clipboard when the user picks "Save".
    pub copy_on_save: bool,
    pub show_toast: bool,
    pub magnifier: bool,
    pub tools_pos: ToolsPos,
    /// Releasing the mouse immediately copies the selection (no toolbar).
    pub instant_mode: bool,
    // Each action can have several key combinations.
    #[serde(deserialize_with = "one_or_many")]
    pub hotkey_region: Vec<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub hotkey_screen: Vec<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub hotkey_window: Vec<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub hotkey_last: Vec<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub hotkey_record: Vec<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub hotkey_record_gif: Vec<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub hotkey_pause: Vec<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub hotkey_replay: Vec<String>,
    pub autostart: bool,
    pub editor_color: [u8; 4],
    pub editor_width: f32,
    /// Loupe cell size in points; 0 hides the loupe (mouse wheel while selecting).
    pub loupe_cell: f32,
    pub toast_corner: Corner,
    pub toast_seconds: f32,
    pub toast_size: ToastSize,
    pub toast_caption: bool,
    pub sound_enabled: bool,
    pub sound_kind: SoundKind,
    pub sound_file: Option<PathBuf>,
    pub sound_volume: f32,
    pub tray_click: TrayAction,
    pub tray_right_click: TrayAction,
    /// Frames per second; 0 = the monitor's refresh rate.
    pub rec_fps: u32,
    pub rec_codec: VideoCodec,
    pub rec_quality: RecQuality,
    /// Desktop sound (loopback) into track 1.
    pub rec_desktop_audio: bool,
    /// Microphone into track 2.
    pub rec_mic: bool,
    pub rec_cursor: bool,
    /// Open the video editor as soon as a recording stops.
    pub rec_open_editor: bool,
    /// Ripples on mouse clicks in the video.
    pub rec_show_clicks: bool,
    /// Shortcut pills (Ctrl+C...) in the video; plain typing is never shown.
    pub rec_show_keys: bool,
    /// Instant replay: keep the last `replay_secs` of the screen on disk.
    pub replay_enabled: bool,
    pub replay_secs: u32,
    pub replay_desktop_audio: bool,
    pub replay_mic: bool,
    pub beauty: Beauty,
    /// Upload endpoint (see `upload.rs` for the protocol); empty = off.
    pub upload_url: String,
    pub upload_token: String,
    /// Upload every capture right away and copy the link.
    pub upload_auto: bool,
    pub tg_mode: TgMode,
    /// GIF recordings: max width and frame rate.
    pub gif_width: u32,
    pub gif_fps: u32,
    /// Re-encode every finished recording for Telegram right away.
    pub tg_auto: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            save_dir: default_save_dir(),
            lang: String::new(),
            sort_by_date: true,
            format: ImageFormat::Png,
            jpeg_quality: 90,
            auto_save: true,
            copy_on_save: true,
            show_toast: true,
            magnifier: true,
            tools_pos: ToolsPos::Screen,
            instant_mode: false,
            hotkey_region: vec!["PrintScreen".into()],
            hotkey_screen: vec!["Ctrl+PrintScreen".into()],
            hotkey_window: Vec::new(),
            hotkey_last: vec!["Shift+PrintScreen".into()],
            hotkey_record: vec!["Ctrl+Shift+PrintScreen".into()],
            hotkey_record_gif: Vec::new(),
            hotkey_pause: vec!["Ctrl+Alt+P".into()],
            hotkey_replay: vec!["Ctrl+Alt+PrintScreen".into()],
            autostart: false,
            editor_color: [0xE3, 0xB3, 0x41, 255],
            editor_width: 4.0,
            loupe_cell: 9.0,
            toast_corner: Corner::BottomRight,
            toast_seconds: 4.0,
            toast_size: ToastSize::Medium,
            toast_caption: true,
            sound_enabled: false,
            sound_kind: SoundKind::Shutter,
            sound_file: None,
            sound_volume: 0.5,
            tray_click: TrayAction::Region,
            tray_right_click: TrayAction::Menu,
            rec_fps: 60,
            rec_codec: VideoCodec::H264,
            rec_quality: RecQuality::Balanced,
            rec_desktop_audio: true,
            rec_mic: true,
            rec_cursor: true,
            rec_open_editor: false,
            rec_show_clicks: false,
            rec_show_keys: false,
            replay_enabled: false,
            replay_secs: 60,
            replay_desktop_audio: true,
            replay_mic: false,
            beauty: Beauty::default(),
            upload_url: String::new(),
            upload_token: String::new(),
            upload_auto: false,
            tg_mode: TgMode::Balanced,
            gif_width: 800,
            gif_fps: 15,
            tg_auto: false,
        }
    }
}

/// Accepts both the old `key = "Ctrl+X"` and the new `key = ["Ctrl+X", ...]`.
fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) if s.trim().is_empty() => Vec::new(),
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v.into_iter().filter(|s| !s.trim().is_empty()).collect(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HotkeyAction {
    Region,
    Screen,
    Window,
    Last,
    Record,
    RecordGif,
    Pause,
    Replay,
}

impl HotkeyAction {
    pub const ALL: [HotkeyAction; 8] = [
        HotkeyAction::Region,
        HotkeyAction::Screen,
        HotkeyAction::Window,
        HotkeyAction::Last,
        HotkeyAction::Record,
        HotkeyAction::RecordGif,
        HotkeyAction::Pause,
        HotkeyAction::Replay,
    ];

    pub fn label(self) -> &'static str {
        match self {
            HotkeyAction::Region => crate::i18n::t("область"),
            HotkeyAction::Screen => crate::i18n::t("весь экран"),
            HotkeyAction::Window => crate::i18n::t("активное окно"),
            HotkeyAction::Last => crate::i18n::t("повторить область"),
            HotkeyAction::Record => crate::i18n::t("запись экрана"),
            HotkeyAction::RecordGif => crate::i18n::t("запись gif"),
            HotkeyAction::Pause => crate::i18n::t("пауза записи"),
            HotkeyAction::Replay => crate::i18n::t("сохранить повтор"),
        }
    }

    pub fn desc(self) -> &'static str {
        match self {
            HotkeyAction::Region => crate::i18n::t("или клик по окну"),
            HotkeyAction::Screen => crate::i18n::t("монитор под курсором"),
            HotkeyAction::Window => crate::i18n::t("окно в фокусе"),
            HotkeyAction::Last => crate::i18n::t("как в прошлый раз"),
            HotkeyAction::Record => crate::i18n::t("повторное нажатие останавливает"),
            HotkeyAction::RecordGif => crate::i18n::t("без звука, повторное нажатие останавливает"),
            HotkeyAction::Pause => "",
            HotkeyAction::Replay => crate::i18n::t("работает, если повтор включён"),
        }
    }
}

impl Config {
    pub fn bindings(&self, a: HotkeyAction) -> &Vec<String> {
        match a {
            HotkeyAction::Region => &self.hotkey_region,
            HotkeyAction::Screen => &self.hotkey_screen,
            HotkeyAction::Window => &self.hotkey_window,
            HotkeyAction::Last => &self.hotkey_last,
            HotkeyAction::Record => &self.hotkey_record,
            HotkeyAction::RecordGif => &self.hotkey_record_gif,
            HotkeyAction::Pause => &self.hotkey_pause,
            HotkeyAction::Replay => &self.hotkey_replay,
        }
    }

    pub fn bindings_mut(&mut self, a: HotkeyAction) -> &mut Vec<String> {
        match a {
            HotkeyAction::Region => &mut self.hotkey_region,
            HotkeyAction::Screen => &mut self.hotkey_screen,
            HotkeyAction::Window => &mut self.hotkey_window,
            HotkeyAction::Last => &mut self.hotkey_last,
            HotkeyAction::Record => &mut self.hotkey_record,
            HotkeyAction::RecordGif => &mut self.hotkey_record_gif,
            HotkeyAction::Pause => &mut self.hotkey_pause,
            HotkeyAction::Replay => &mut self.hotkey_replay,
        }
    }

    /// Folder for today's screenshots (created on demand).
    pub fn pics_dir(&self) -> PathBuf {
        self.dated_dir("pics")
    }

    /// Folder for today's recordings and replays (created on demand).
    pub fn videos_dir(&self) -> PathBuf {
        self.dated_dir("videos")
    }

    fn dated_dir(&self, kind: &str) -> PathBuf {
        let dir = if self.sort_by_date {
            self.save_dir.join(chrono::Local::now().format("%Y-%m-%d").to_string()).join(kind)
        } else {
            self.save_dir.clone()
        };
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// First binding, for labels in menus.
    pub fn primary_binding(&self, a: HotkeyAction) -> &str {
        self.bindings(a).first().map(String::as_str).unwrap_or("")
    }
}

fn default_save_dir() -> PathBuf {
    dirs::picture_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Blinker")
}

pub fn app_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Blinker")
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Blinker")
}

fn config_path() -> PathBuf {
    app_dir().join("config.toml")
}

/// Last write time of the config file (another Blinker process may change it).
pub fn modified() -> Option<std::time::SystemTime> {
    std::fs::metadata(config_path()).and_then(|m| m.modified()).ok()
}

fn failed_hotkeys_path() -> PathBuf {
    cache_dir().join("hotkeys-failed.txt")
}

/// Written by the tray process after registering hotkeys, read by settings.
pub fn save_failed_hotkeys(failed: &std::collections::HashSet<String>) {
    let _ = std::fs::create_dir_all(cache_dir());
    let text: Vec<&str> = failed.iter().map(String::as_str).collect();
    let _ = std::fs::write(failed_hotkeys_path(), text.join("\n"));
}

pub fn load_failed_hotkeys() -> std::collections::HashSet<String> {
    std::fs::read_to_string(failed_hotkeys_path())
        .map(|s| s.lines().filter(|l| !l.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

pub fn load() -> Config {
    migrate_dirs();
    let mut cfg: Config = std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default();
    if migrate_save_dir(&mut cfg) {
        save(&cfg);
    }
    crate::i18n::apply(&cfg.lang);
    cfg
}

/// The app was called Blinkshot: move its settings and cache to the new name.
fn migrate_dirs() {
    for (base, new) in [(dirs::config_dir(), app_dir()), (dirs::cache_dir(), cache_dir())] {
        let Some(old) = base.map(|b| b.join("Blinkshot")) else { continue };
        if old.is_dir() && !new.exists() {
            let _ = std::fs::rename(&old, &new);
        }
    }
}

/// Default save folder of the old name: rename it too. When it can't be moved
/// (open in Explorer, say), keep saving there.
fn migrate_save_dir(cfg: &mut Config) -> bool {
    let Some(old) = dirs::picture_dir().or_else(dirs::home_dir).map(|d| d.join("Blinkshot")) else { return false };
    if cfg.save_dir != old {
        return false;
    }
    let new = default_save_dir();
    if old.is_dir() && (new.exists() || std::fs::rename(&old, &new).is_err()) {
        return false;
    }
    cfg.save_dir = new;
    true
}

pub fn save(cfg: &Config) {
    let path = config_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match toml::to_string_pretty(cfg) {
        Ok(s) => {
            if let Err(e) = std::fs::write(&path, s) {
                log::error(format!("failed to write config: {e}"));
            }
        }
        Err(e) => log::error(format!("failed to serialize config: {e}")),
    }
}

pub mod log {
    pub fn error(msg: impl AsRef<str>) {
        eprintln!("[blinker] {}", msg.as_ref());
    }
}

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

/// Where the recording pill sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PillPlace {
    /// Under the recorded area; recording the whole screen, by the tray.
    #[default]
    Auto,
    TopLeft,
    Top,
    TopRight,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl PillPlace {
    pub const ALL: [PillPlace; 7] =
        [PillPlace::Auto, PillPlace::TopLeft, PillPlace::Top, PillPlace::TopRight, PillPlace::BottomLeft, PillPlace::Bottom, PillPlace::BottomRight];

    pub fn label(self) -> &'static str {
        use crate::i18n::l;
        match self {
            PillPlace::Auto => l("под областью", "under the area"),
            PillPlace::TopLeft => l("слева сверху", "top left"),
            PillPlace::Top => l("сверху по центру", "top middle"),
            PillPlace::TopRight => l("справа сверху", "top right"),
            PillPlace::BottomLeft => l("слева снизу", "bottom left"),
            PillPlace::Bottom => l("снизу по центру", "bottom middle"),
            PillPlace::BottomRight => l("справа снизу", "bottom right"),
        }
    }

    /// The same for a whole-screen recording, where "auto" is by the tray.
    pub fn label_screen(self) -> &'static str {
        match self {
            // a Mac's menu bar is up top: there it is just the corner
            PillPlace::Auto if cfg!(target_os = "macos") => crate::i18n::l("в углу", "in the corner"),
            PillPlace::Auto => crate::i18n::l("у трея", "by the tray"),
            p => p.label(),
        }
    }

    /// Where on the screen, as fractions (0 left/top … 1 right/bottom).
    pub fn anchor(self) -> Option<(f32, f32)> {
        match self {
            PillPlace::Auto => None,
            PillPlace::TopLeft => Some((0.0, 0.0)),
            PillPlace::Top => Some((0.5, 0.0)),
            PillPlace::TopRight => Some((1.0, 0.0)),
            PillPlace::BottomLeft => Some((0.0, 1.0)),
            PillPlace::Bottom => Some((0.5, 1.0)),
            PillPlace::BottomRight => Some((1.0, 1.0)),
        }
    }
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

/// A ShareX-style custom uploader (imported from ShareX or a .sxcu file):
/// any server that takes a multipart form or the raw file and answers with
/// the link somewhere in its response.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomUploader {
    pub name: String,
    /// "POST" / "PUT"
    pub method: String,
    pub url: String,
    /// Query string parameters.
    pub parameters: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    /// "MultipartFormData" or "Binary".
    pub body: String,
    /// Multipart: the form field of the file.
    pub file_form_name: String,
    /// Multipart: the other form fields.
    pub arguments: Vec<(String, String)>,
    /// Where the link is in the response: "{json:data.url}", "{response}"...
    pub url_template: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub save_dir: PathBuf,
    /// Interface language: "ru", "en" or "" (system).
    pub lang: String,
    /// Lay files out as `save_dir/2026-10-20/pics` and `.../videos`.
    pub sort_by_date: bool,
    /// Clips made while a game runs: `save_dir/games/<game>`, named after it.
    pub sort_by_game: bool,
    /// While selecting, QR codes on the screen show where they lead.
    pub read_qr: bool,
    /// The whisper model for subtitles from speech (the video editor).
    pub speech_model: crate::vedit::speech::Model,
    /// How subtitles look in the video editor.
    pub caption_style: crate::vedit::shorts::CaptionStyle,
    pub format: ImageFormat,
    pub jpeg_quality: u8,
    /// Save every capture to `save_dir` (feeds the library).
    pub auto_save: bool,
    /// Also put the image on the clipboard when the user picks "Save".
    pub copy_on_save: bool,
    pub show_toast: bool,
    pub magnifier: bool,
    pub tools_pos: ToolsPos,
    /// The first capture's walkthrough was shown (or skipped).
    pub tour_done: bool,
    /// The note about the NVIDIA overlay's "Press Alt+Z" was read (or put away).
    pub nvidia_tip_seen: bool,
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
    #[serde(deserialize_with = "one_or_many")]
    pub hotkey_pick: Vec<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub hotkey_text: Vec<String>,
    // Keys of the actions on a selected area: read inside a capture only.
    #[serde(deserialize_with = "one_or_many")]
    pub key_copy: Vec<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub key_save: Vec<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub key_edit: Vec<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub key_pin: Vec<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub key_beauty: Vec<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub key_upload: Vec<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub key_text: Vec<String>,
    pub autostart: bool,
    pub editor_color: [u8; 4],
    pub editor_width: f32,
    /// Each drawing tool's own size in a capture (its name: thickness or text
    /// size; missing: `editor_width`).
    pub tool_widths: std::collections::BTreeMap<String, f32>,
    /// 1 once the drawing sizes were set back to 4 for everyone (0.4.2).
    pub widths_reset: u32,
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
    /// The tray menu as laid out in the settings.
    pub tray_menu: crate::tray::TrayMenu,
    /// The built-in theme ("graphite", "midnight"…; theme.css goes on top).
    pub theme: String,
    /// An accent color of one's own over the theme's ("#7aa2ff"; empty: the theme's).
    pub accent: String,
    /// The user's theme.css goes over the theme (off: put aside, kept).
    pub theme_css: bool,
    /// Files come out without metadata: no time, no Windows build, no
    /// program names (`strip.rs`).
    pub strip_metadata: bool,
    /// The monitor the card after a capture shows on: its rectangle
    /// (x, y, width, height); none: where the capture or recording was.
    pub toast_monitor: Option<[i32; 4]>,
    /// Set: a single left click waits a moment to see whether a second comes.
    pub tray_double_click: TrayAction,
    /// Install new versions by itself (in the background, then restart).
    pub auto_update: bool,
    /// "Clipping with blinker" on the Discord profile.
    pub discord_presence: bool,
    /// The status names the game being played ("Clipping Dota 2").
    pub discord_game: bool,
    /// The status shows today's numbers ("3 screenshots · 1 clip today").
    pub discord_stats: bool,
    /// The version whose "what's new" was seen; another one shows it again.
    pub seen_version: String,
    /// Frames per second; 0 = the monitor's refresh rate.
    pub rec_fps: u32,
    pub rec_codec: VideoCodec,
    pub rec_quality: RecQuality,
    /// Desktop sound (loopback) into track 1.
    pub rec_desktop_audio: bool,
    /// Microphone into track 2.
    pub rec_mic: bool,
    /// Which microphone (device id); empty = the system default.
    pub mic_device: String,
    pub rec_cursor: bool,
    /// Open the video editor as soon as a recording stops.
    pub rec_open_editor: bool,
    /// Ripples on mouse clicks in the video.
    pub rec_show_clicks: bool,
    /// Shortcut pills (Ctrl+C...) in the video; plain typing is never shown.
    pub rec_show_keys: bool,
    /// Where the recording pill sits recording an area or a window...
    pub rec_pill: PillPlace,
    /// ...and recording a whole display (auto: by the tray).
    pub rec_pill_screen: PillPlace,
    /// Instant replay: keep the last `replay_secs` of the screen on disk.
    pub replay_enabled: bool,
    pub replay_secs: u32,
    /// Saving the replay empties it: the next save starts from that moment.
    pub replay_reset_on_save: bool,
    pub replay_desktop_audio: bool,
    pub replay_mic: bool,
    pub beauty: Beauty,
    /// Uploading at all: off hides every upload button and the U key.
    pub upload_enabled: bool,
    /// Upload endpoint (see `upload.rs` for the protocol); empty = the
    /// default server, post.blinker.cool.
    pub upload_url: String,
    pub upload_token: String,
    /// A ShareX custom uploader instead of the blinker protocol.
    pub upload_custom: Option<CustomUploader>,
    /// The note about post.blinker.cool (3 days, 100 MB) was shown.
    pub post_notice_seen: bool,
    /// The offer to bring over ShareX settings was made (once).
    pub sharex_offered: bool,
    /// Upload every capture right away and copy the link.
    pub upload_auto: bool,
    /// GIF recordings: max width and frame rate.
    pub gif_width: u32,
    pub gif_fps: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            save_dir: default_save_dir(),
            lang: String::new(),
            sort_by_date: true,
            sort_by_game: true,
            read_qr: true,
            speech_model: Default::default(),
            caption_style: Default::default(),
            format: ImageFormat::Png,
            jpeg_quality: 90,
            auto_save: true,
            copy_on_save: true,
            show_toast: true,
            magnifier: true,
            tour_done: false,
            nvidia_tip_seen: false,
            tools_pos: ToolsPos::Screen,
            instant_mode: false,
            hotkey_region: default_keys(&["PrintScreen"], &["Shift+Cmd+2"]),
            hotkey_screen: default_keys(&["Ctrl+PrintScreen"], &["Shift+Cmd+1"]),
            hotkey_window: Vec::new(),
            hotkey_last: default_keys(&["Shift+PrintScreen"], &["Opt+Shift+Cmd+2"]),
            hotkey_record: default_keys(&["Ctrl+Shift+PrintScreen"], &["Shift+Cmd+7"]),
            hotkey_record_gif: Vec::new(),
            hotkey_pause: default_keys(&["Ctrl+Alt+P"], &["Ctrl+Opt+P"]),
            hotkey_replay: default_keys(&["Ctrl+Alt+PrintScreen"], &["Opt+Shift+Cmd+7"]),
            hotkey_pick: default_keys(&["Ctrl+Alt+C"], &["Ctrl+Opt+Cmd+C"]),
            hotkey_text: default_keys(&["Ctrl+Alt+T"], &["Ctrl+Opt+Cmd+T"]),
            key_copy: default_keys(&["Ctrl+C"], &["Cmd+C"]),
            key_save: default_keys(&["Ctrl+S"], &["Cmd+S"]),
            key_edit: vec!["E".into()],
            key_pin: vec!["P".into()],
            key_beauty: vec!["B".into()],
            key_upload: vec!["U".into()],
            key_text: vec!["T".into()],
            autostart: false,
            editor_color: [0xE3, 0xB3, 0x41, 255],
            editor_width: 4.0,
            tool_widths: Default::default(),
            widths_reset: 0,
            loupe_cell: 9.0,
            toast_corner: Corner::BottomRight,
            toast_seconds: 4.0,
            toast_size: ToastSize::Medium,
            toast_caption: true,
            sound_enabled: false,
            sound_kind: SoundKind::Shutter,
            sound_file: None,
            // a Mac's speakers are loud: a quieter shutter there
            sound_volume: if cfg!(target_os = "macos") { 0.2 } else { 0.5 },
            // a Mac opens a menu bar icon's menu on a click
            tray_click: if cfg!(target_os = "macos") { TrayAction::Menu } else { TrayAction::Region },
            tray_right_click: if cfg!(target_os = "macos") { TrayAction::Region } else { TrayAction::Menu },
            tray_menu: Default::default(),
            theme: "graphite".into(),
            accent: String::new(),
            theme_css: true,
            strip_metadata: true,
            toast_monitor: None,
            tray_double_click: TrayAction::Nothing,
            auto_update: false,
            discord_presence: false,
            discord_game: true,
            discord_stats: true,
            seen_version: String::new(),
            rec_fps: 60,
            rec_codec: VideoCodec::H264,
            rec_quality: RecQuality::Balanced,
            rec_desktop_audio: true,
            rec_mic: true,
            mic_device: String::new(),
            rec_cursor: true,
            rec_open_editor: false,
            rec_show_clicks: false,
            rec_show_keys: false,
            rec_pill: PillPlace::Auto,
            rec_pill_screen: PillPlace::Auto,
            replay_enabled: false,
            replay_secs: 60,
            replay_reset_on_save: false,
            replay_desktop_audio: true,
            replay_mic: false,
            beauty: Beauty::default(),
            upload_enabled: true,
            upload_url: String::new(),
            upload_token: String::new(),
            upload_custom: None,
            post_notice_seen: false,
            sharex_offered: false,
            upload_auto: false,
            gif_width: 800,
            gif_fps: 15,
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
    /// The color under the pointer, as hex, to the clipboard.
    Pick,
    /// Select an area and read the text in it.
    Text,
}

impl HotkeyAction {
    pub const ALL: [HotkeyAction; 10] = [
        HotkeyAction::Region,
        HotkeyAction::Screen,
        HotkeyAction::Window,
        HotkeyAction::Last,
        HotkeyAction::Record,
        HotkeyAction::RecordGif,
        HotkeyAction::Pause,
        HotkeyAction::Replay,
        HotkeyAction::Pick,
        HotkeyAction::Text,
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
            HotkeyAction::Pick => crate::i18n::l("пипетка", "color picker"),
            HotkeyAction::Text => crate::i18n::l("текст с экрана", "text from the screen"),
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
            HotkeyAction::Pick => crate::i18n::l("клик копирует цвет в hex", "a click copies the color as hex"),
            HotkeyAction::Text => crate::i18n::l("выдели область: текст в ней распознается и скопируется", "select an area: its text is read and copied"),
        }
    }
}

/// An action on a selected area, with keys pressed inside a capture (not
/// system-wide: they don't take the keys away from other apps).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CaptureKey {
    Copy,
    Save,
    Edit,
    Pin,
    Beauty,
    Upload,
    Text,
}

impl CaptureKey {
    pub const ALL: [CaptureKey; 7] =
        [CaptureKey::Copy, CaptureKey::Save, CaptureKey::Edit, CaptureKey::Pin, CaptureKey::Beauty, CaptureKey::Upload, CaptureKey::Text];

    pub fn label(self) -> &'static str {
        match self {
            CaptureKey::Copy => crate::i18n::t("копировать"),
            CaptureKey::Save => crate::i18n::t("сохранить"),
            CaptureKey::Edit => crate::i18n::t("править"),
            CaptureKey::Pin => crate::i18n::t("закрепить поверх окон"),
            CaptureKey::Beauty => crate::i18n::t("красивый снимок"),
            CaptureKey::Upload => crate::i18n::t("загрузить и скопировать ссылку"),
            CaptureKey::Text => crate::i18n::l("распознать текст", "recognize text"),
        }
    }

    pub fn desc(self) -> &'static str {
        match self {
            CaptureKey::Copy => crate::i18n::l("и enter", "and enter"),
            _ => "",
        }
    }
}

impl Config {
    pub fn capture_keys(&self, k: CaptureKey) -> &Vec<String> {
        match k {
            CaptureKey::Copy => &self.key_copy,
            CaptureKey::Save => &self.key_save,
            CaptureKey::Edit => &self.key_edit,
            CaptureKey::Pin => &self.key_pin,
            CaptureKey::Beauty => &self.key_beauty,
            CaptureKey::Upload => &self.key_upload,
            CaptureKey::Text => &self.key_text,
        }
    }

    pub fn capture_keys_mut(&mut self, k: CaptureKey) -> &mut Vec<String> {
        match k {
            CaptureKey::Copy => &mut self.key_copy,
            CaptureKey::Save => &mut self.key_save,
            CaptureKey::Edit => &mut self.key_edit,
            CaptureKey::Pin => &mut self.key_pin,
            CaptureKey::Beauty => &mut self.key_beauty,
            CaptureKey::Upload => &mut self.key_upload,
            CaptureKey::Text => &mut self.key_text,
        }
    }

    /// The first key of a capture action as shown in tips ("cmd s"), or "".
    pub fn capture_hint(&self, k: CaptureKey) -> String {
        self.capture_keys(k).first().map(|b| crate::hotkey::display_parts(b).join(" ")).unwrap_or_default()
    }

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
            HotkeyAction::Pick => &self.hotkey_pick,
            HotkeyAction::Text => &self.hotkey_text,
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
            HotkeyAction::Pick => &mut self.hotkey_pick,
            HotkeyAction::Text => &mut self.hotkey_text,
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

    /// Folder for a clip: the game's own when one runs (and that's on),
    /// else today's videos.
    pub fn clips_dir(&self, game: Option<&str>) -> PathBuf {
        match game.map(safe_name).filter(|g| self.sort_by_game && !g.is_empty()) {
            Some(g) => {
                let dir = self.save_dir.join(GAMES_DIR).join(g);
                let _ = std::fs::create_dir_all(&dir);
                dir
            }
            None => self.videos_dir(),
        }
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

/// Default bindings: Windows keyboards have PrtSc; on a Mac ⇧⌘3…⌘5 belong to
/// the system, so blinker takes the free digits next to them.
/// The folder of games' clips in `save_dir`.
pub const GAMES_DIR: &str = "games";

/// A game's name made fit for a file or folder name.
pub fn safe_name(s: &str) -> String {
    let s: String = s.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { ' ' } else { c }).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches('.').trim().to_string()
}

fn default_keys(windows: &[&str], mac: &[&str]) -> Vec<String> {
    let keys = if cfg!(target_os = "macos") { mac } else { windows };
    keys.iter().map(|k| k.to_string()).collect()
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

/// The config as saved, without the rest of `load` (nothing migrated or
/// written back): for a look at a setting from anywhere.
pub fn peek() -> Config {
    std::fs::read_to_string(config_path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
}

/// The theme and accent as saved (the theme reads them again whenever the
/// file changes).
pub fn theme_choice() -> (String, String, bool) {
    let cfg = peek();
    (cfg.theme, cfg.accent, cfg.theme_css)
}

pub fn load() -> Config {
    migrate_dirs();
    let text = std::fs::read_to_string(config_path()).ok();
    let fresh = text.is_none();
    // people who used blinker before the walkthrough existed know it already
    let knows = text.as_deref().is_some_and(|t| !t.contains("tour_done"));
    let mut cfg: Config = text.and_then(|s| toml::from_str(&s).ok()).unwrap_or_default();
    if knows {
        cfg.tour_done = true;
    }
    cfg.tray_menu = std::mem::take(&mut cfg.tray_menu).fixed();
    // 0.4.2: every drawing tool starts at size 4, whatever was set before
    let reset = cfg.widths_reset < 1;
    if reset {
        cfg.editor_width = 4.0;
        cfg.tool_widths.clear();
        cfg.widths_reset = 1;
    }
    // a new install has nothing new to tell; an update from a version
    // without this field does
    if fresh {
        cfg.seen_version = env!("CARGO_PKG_VERSION").to_string();
    }
    if migrate_save_dir(&mut cfg) || fresh || reset {
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
    //! blinker's log: a line per start and exit of each process, its errors,
    //! and what took it down (a panic with its place in the code, a crash in
    //! native code). Windows: %LOCALAPPDATA%\Blinker\logs\blinker.log; a Mac:
    //! ~/Library/Logs/Blinker/blinker.log (where its native layer writes too).
    //! Past 2 MB the file becomes blinker.old.log and a new one starts.

    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::OnceLock;

    /// What this process is ("tray", "settings", "text"…), for every line.
    static ROLE: OnceLock<&'static str> = OnceLock::new();

    pub fn path() -> PathBuf {
        #[cfg(target_os = "macos")]
        let dir = dirs::home_dir().unwrap_or_else(std::env::temp_dir).join("Library/Logs/Blinker");
        #[cfg(not(target_os = "macos"))]
        let dir = super::cache_dir().join("logs");
        dir.join("blinker.log")
    }

    pub fn error(msg: impl AsRef<str>) {
        write("error", msg.as_ref());
    }

    pub fn info(msg: impl AsRef<str>) {
        write("info", msg.as_ref());
    }

    fn write(level: &str, msg: &str) {
        eprintln!("[blinker] {msg}");
        let path = path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
            let _ = std::fs::rename(&path, path.with_file_name("blinker.old.log"));
        }
        let line = format!(
            "{} [{} {}] {level}: {msg}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            std::process::id(),
            ROLE.get().copied().unwrap_or("?")
        );
        // one write per line: the processes share the file
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = f.write_all(line.as_bytes());
        }
    }

    /// The process starts as `role`: logged, and from now on a panic is
    /// written down before it takes the process with it (the release build
    /// aborts on a panic, which leaves no message anywhere else).
    pub fn start(role: &'static str) {
        let _ = ROLE.set(role);
        info(format!("start {} ({})", env!("CARGO_PKG_VERSION"), std::env::consts::OS));
        std::panic::set_hook(Box::new(|p| {
            let what = p
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "?".into());
            let at = p.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
            let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
            error(format!("crash: panic in thread {thread} at {at}: {what}"));
        }));
    }
}

#[cfg(test)]
mod log_tests {
    /// A panic leaves its message and place in the log.
    #[test]
    fn panic_is_logged() {
        super::log::start("test");
        let mark = format!("log test {}", std::process::id());
        let m = mark.clone();
        let _ = std::thread::Builder::new().name("log-test".into()).spawn(move || panic!("{m}")).unwrap().join();
        let _ = std::panic::take_hook();
        let text = std::fs::read_to_string(super::log::path()).unwrap();
        let line = text.lines().rev().find(|l| l.contains(&mark)).expect("the panic's line");
        assert!(line.contains("[") && line.contains("test]") && line.contains("crash: panic in thread log-test at src"), "{line}");
    }
}

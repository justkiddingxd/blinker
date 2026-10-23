//! The tray menu as the user lays it out (settings → tray menu): rows of
//! big tiles on top, each under its heading, then a list with separators.
//! Both shells (Windows' own menu, the Mac's native one) build theirs from it.

use crate::config::HotkeyAction;
use crate::icons::Icon;
use crate::runtime::{Command, HomeTab};
use serde::{Deserialize, Serialize};

/// What a menu entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrayItem {
    Region,
    Screen,
    Window,
    Last,
    Text,
    Pick,
    Record,
    RecordScreen,
    Gif,
    Replay,
    History,
    Folder,
    Settings,
    Quit,
    /// While recording, in place of the tiles.
    Stop,
    Pause,
}

impl TrayItem {
    /// What can be put in the menu, in the order the settings offer it.
    pub const ALL: [TrayItem; 14] = [
        TrayItem::Region,
        TrayItem::Screen,
        TrayItem::Window,
        TrayItem::Last,
        TrayItem::Text,
        TrayItem::Pick,
        TrayItem::Record,
        TrayItem::RecordScreen,
        TrayItem::Gif,
        TrayItem::Replay,
        TrayItem::History,
        TrayItem::Folder,
        TrayItem::Settings,
        TrayItem::Quit,
    ];

    /// Every item, the recording ones too: an item's menu id is its place
    /// here plus one.
    const EVERY: [TrayItem; 16] = [
        TrayItem::Region,
        TrayItem::Screen,
        TrayItem::Window,
        TrayItem::Last,
        TrayItem::Text,
        TrayItem::Pick,
        TrayItem::Record,
        TrayItem::RecordScreen,
        TrayItem::Gif,
        TrayItem::Replay,
        TrayItem::History,
        TrayItem::Folder,
        TrayItem::Settings,
        TrayItem::Quit,
        TrayItem::Stop,
        TrayItem::Pause,
    ];

    pub fn id(self) -> u32 {
        Self::EVERY.iter().position(|i| *i == self).unwrap_or(0) as u32 + 1
    }

    pub fn from_id(id: u32) -> Option<TrayItem> {
        Self::EVERY.get((id as usize).checked_sub(1)?).copied()
    }

    /// Can't be taken out of the menu: without them blinker can't be set
    /// up or closed from the tray.
    pub fn locked(self) -> bool {
        matches!(self, TrayItem::Settings | TrayItem::Quit)
    }

    pub fn icon(self) -> Icon {
        match self {
            TrayItem::Region => Icon::Region,
            TrayItem::Screen | TrayItem::RecordScreen => Icon::Screen,
            TrayItem::Window => Icon::Window,
            TrayItem::Last | TrayItem::Replay => Icon::Repeat,
            TrayItem::Text => Icon::Scan,
            TrayItem::Pick => Icon::Pipette,
            TrayItem::Record => Icon::Record,
            TrayItem::Gif => Icon::Video,
            TrayItem::History => Icon::Frame,
            TrayItem::Folder => Icon::Folder,
            TrayItem::Settings => Icon::Settings,
            TrayItem::Quit | TrayItem::Stop => Icon::Close,
            TrayItem::Pause => Icon::Pause,
        }
    }

    /// The SF Symbol of its native Mac menu row.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn symbol(self) -> &'static str {
        match self {
            TrayItem::Region => "rectangle.dashed",
            TrayItem::Screen | TrayItem::RecordScreen => "display",
            TrayItem::Window => "macwindow",
            TrayItem::Last => "arrow.counterclockwise",
            TrayItem::Text => "text.viewfinder",
            TrayItem::Pick => "eyedropper",
            TrayItem::Record => "record.circle",
            TrayItem::Gif => "photo.stack",
            TrayItem::Replay => "clock.arrow.circlepath",
            TrayItem::History => "clock",
            TrayItem::Folder => "folder",
            TrayItem::Settings => "gearshape",
            TrayItem::Quit => "power",
            TrayItem::Stop => "stop.circle",
            TrayItem::Pause => "pause.circle",
        }
    }

    /// Its name as a row of the list (and in the settings).
    pub fn label(self) -> &'static str {
        let l = crate::i18n::l;
        match self {
            TrayItem::Region => l("снимок области", "capture an area"),
            TrayItem::Screen => l("снимок экрана", "capture the screen"),
            TrayItem::Window => l("снимок окна", "capture a window"),
            TrayItem::Last => l("повторить область", "repeat the last area"),
            TrayItem::Text => l("распознать текст", "read text"),
            TrayItem::Pick => l("пипетка", "color picker"),
            TrayItem::Record => l("запись области", "record an area"),
            TrayItem::RecordScreen => l("запись экрана", "record the screen"),
            TrayItem::Gif => l("запись gif", "record a gif"),
            TrayItem::Replay => l("сохранить повтор", "save the replay"),
            TrayItem::History => l("история", "history"),
            TrayItem::Folder => l("папка со снимками", "captures folder"),
            TrayItem::Settings => l("настройки", "settings"),
            TrayItem::Quit => l("выход", "quit"),
            TrayItem::Stop => l("стоп", "stop"),
            TrayItem::Pause => l("пауза", "pause"),
        }
    }

    /// Its short name on a tile (under a heading that says what kind).
    pub fn tile_label(self) -> &'static str {
        let l = crate::i18n::l;
        match self {
            TrayItem::Region | TrayItem::Record => l("область", "area"),
            TrayItem::Screen | TrayItem::RecordScreen => l("экран", "screen"),
            TrayItem::Window => l("окно", "window"),
            TrayItem::Last => l("повторить", "repeat"),
            TrayItem::Text => l("текст", "text"),
            TrayItem::Replay => l("повтор", "replay"),
            TrayItem::Folder => l("папка", "folder"),
            TrayItem::Gif => "gif",
            other => other.label(),
        }
    }

    /// The key it has among the hotkeys, if any.
    pub fn hotkey(self) -> Option<HotkeyAction> {
        match self {
            TrayItem::Region => Some(HotkeyAction::Region),
            TrayItem::Screen => Some(HotkeyAction::Screen),
            TrayItem::Window => Some(HotkeyAction::Window),
            TrayItem::Last => Some(HotkeyAction::Last),
            TrayItem::Text => Some(HotkeyAction::Text),
            TrayItem::Pick => Some(HotkeyAction::Pick),
            TrayItem::Record | TrayItem::Stop => Some(HotkeyAction::Record),
            TrayItem::Gif => Some(HotkeyAction::RecordGif),
            TrayItem::Replay => Some(HotkeyAction::Replay),
            TrayItem::Pause => Some(HotkeyAction::Pause),
            _ => None,
        }
    }

    pub fn command(self) -> Command {
        match self {
            TrayItem::Region => Command::CaptureRegion,
            TrayItem::Screen => Command::CaptureScreen,
            TrayItem::Window => Command::CaptureWindow,
            TrayItem::Last => Command::CaptureLast,
            TrayItem::Text => Command::CaptureText,
            TrayItem::Pick => Command::PickColor,
            TrayItem::Record | TrayItem::Stop => Command::Record,
            TrayItem::RecordScreen => Command::RecordScreen,
            TrayItem::Gif => Command::RecordGif,
            TrayItem::Replay => Command::ReplaySave,
            TrayItem::History => Command::OpenHistory,
            TrayItem::Folder => Command::OpenFolder,
            TrayItem::Settings => Command::OpenHome(HomeTab::Capture),
            TrayItem::Quit => Command::Quit,
            TrayItem::Pause => Command::RecordPause,
        }
    }
}

/// A row of tiles and the heading over it ("" for none; "@screenshot" and
/// "@video" are the built-in ones, in the interface's language).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TileRow {
    #[serde(default)]
    pub title: String,
    pub items: Vec<TrayItem>,
}

impl TileRow {
    /// How many tiles fit in a row.
    pub const MAX: usize = 5;

    /// The heading as shown.
    pub fn heading(&self) -> &str {
        match self.title.as_str() {
            "@screenshot" => crate::i18n::l("скриншот", "screenshot"),
            "@video" => crate::i18n::l("видео", "video"),
            t => t,
        }
    }
}

/// A row of the list.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Entry {
    Item(TrayItem),
    Separator,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrayMenu {
    pub tiles: Vec<TileRow>,
    pub list: Vec<Entry>,
}

impl Default for TrayMenu {
    fn default() -> Self {
        use TrayItem::*;
        Self {
            tiles: vec![
                TileRow { title: "@screenshot".into(), items: vec![Region, Screen, Window] },
                TileRow { title: "@video".into(), items: vec![Record, RecordScreen, Gif] },
            ],
            list: vec![
                Entry::Item(Last),
                Entry::Item(Replay),
                Entry::Separator,
                Entry::Item(History),
                Entry::Item(Folder),
                Entry::Item(Settings),
                Entry::Separator,
                Entry::Item(Quit),
            ],
        }
    }
}

impl TrayMenu {
    /// Holds `item` somewhere.
    pub fn has(&self, item: TrayItem) -> bool {
        self.tiles.iter().any(|r| r.items.contains(&item)) || self.list.contains(&Entry::Item(item))
    }

    /// Settings and quit are always there (an old or hand-edited config
    /// may have lost them): what's missing goes to the end of the list.
    pub fn fixed(mut self) -> Self {
        for item in [TrayItem::Settings, TrayItem::Quit] {
            if !self.has(item) {
                self.list.push(Entry::Item(item));
            }
        }
        self.tiles.retain(|r| !r.items.is_empty());
        self
    }
}

/// A row of the menu as it opens now.
pub enum Row {
    Heading(String),
    Tiles(Vec<TrayItem>),
    Item(TrayItem),
    Separator,
}

/// The menu to show: while recording, "stop" and "pause" stand in for the
/// tiles; the replay's row only while the replay is on; no separator at an
/// end or next to another.
pub fn rows(menu: &TrayMenu, recording: bool, replay_on: bool) -> Vec<Row> {
    let shown = |i: &TrayItem| replay_on || *i != TrayItem::Replay;
    let mut out = Vec::new();
    if recording {
        out.push(Row::Tiles(vec![TrayItem::Stop, TrayItem::Pause]));
    } else {
        for row in &menu.tiles {
            let items: Vec<TrayItem> = row.items.iter().copied().filter(shown).collect();
            if items.is_empty() {
                continue;
            }
            if !row.heading().trim().is_empty() {
                out.push(Row::Heading(row.heading().trim().to_string()));
            }
            out.push(Row::Tiles(items));
        }
    }
    let mut list: Vec<Row> = Vec::new();
    for e in &menu.list {
        match e {
            Entry::Item(i) if shown(i) => list.push(Row::Item(*i)),
            Entry::Item(_) => {}
            Entry::Separator => {
                if !list.is_empty() && !matches!(list.last(), Some(Row::Separator)) {
                    list.push(Row::Separator);
                }
            }
        }
    }
    while matches!(list.last(), Some(Row::Separator)) {
        list.pop();
    }
    out.extend(list);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default menu reads back from the config file as written, and the
    /// rows skip the replay when it's off and close up the separators.
    #[test]
    fn menu_round_trip_and_rows() {
        #[derive(Serialize, Deserialize)]
        struct Wrap {
            tray_menu: TrayMenu,
        }
        let text = toml::to_string(&Wrap { tray_menu: TrayMenu::default() }).unwrap();
        let back: Wrap = toml::from_str(&text).unwrap();
        assert_eq!(back.tray_menu, TrayMenu::default());
        let menu = TrayMenu { tiles: Vec::new(), list: vec![Entry::Separator, Entry::Item(TrayItem::Replay), Entry::Separator, Entry::Item(TrayItem::Quit), Entry::Separator] };
        let rows = rows(&menu, false, false);
        assert_eq!(rows.len(), 1);
        assert!(matches!(rows[0], Row::Item(TrayItem::Quit)));
        for i in TrayItem::EVERY {
            assert_eq!(TrayItem::from_id(i.id()), Some(i));
        }
    }
}

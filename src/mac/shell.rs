//! The always-on part of Blinker on macOS: the menu bar icon and the global
//! hotkeys. AppKit keeps both on the main thread, so there is no shell
//! thread: between windows the controller keeps the event queue moving
//! (`Shared::wait`), and while a window is open winit does.

use crate::config::{HotkeyAction, TrayAction};
use crate::hotkey;
use crate::runtime::{Command, HomeTab, Shared, WM_RELOAD_HOTKEYS, WM_RESTART, WM_SUSPEND_HOTKEYS};
use crate::win::{self, ffi, vk};
use eframe::egui;
use std::ffi::CString;
use std::sync::{Arc, Mutex, OnceLock};

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
static HOTKEYS: Mutex<Vec<(u32, HotkeyAction)>> = Mutex::new(Vec::new());

pub struct ShellHandle;

impl ShellHandle {
    /// The app quits: the other blinker processes (settings, editors, pins)
    /// end too, like any Mac app's windows.
    pub fn shutdown(self) {
        unregister();
        unsafe { ffi::bk_tray_remove() };
        win::post(win::QUIT);
    }
}

pub fn spawn(shared: Arc<Shared>) -> ShellHandle {
    let _ = SHARED.set(shared.clone());
    launch(&shared);
    // other threads (capture, effects) read the monitor list the main thread saw
    win::displays();
    unsafe { ffi::bk_on_reopen(win::on_reopen) };
    unsafe { ffi::bk_on_url(on_url) };
    // asks once; the system prompt points to the right switch in Settings,
    // and a note next to it tells why
    if !win::screen_access(false) {
        let before = win::hint::snapshot();
        win::screen_access(true);
        win::hint::watch(before);
    }
    let icon = crate::theme::tray_icon(36, false, 2.0);
    unsafe { ffi::bk_tray_create(icon.as_raw().as_ptr(), icon.width() as i32, icon.height() as i32, on_tray) };
    // the tile keeps its own colors: not a template image
    tray_state_changed(&shared);
    unsafe { ffi::bk_hotkeys_init(on_hotkey) };
    register(&shared);
    win::listen(win::ACTIVATE, 1, on_note);
    win::listen(&format!("BlinkerShell.{WM_RELOAD_HOTKEYS}"), 2, on_note);
    win::listen(&format!("BlinkerShell.{WM_SUSPEND_HOTKEYS}"), 3, on_note);
    win::listen(&format!("BlinkerShell.{WM_RESTART}"), 4, on_note);
    win::listen(win::QUIT_TRAY, 5, on_note);
    ShellHandle
}

/// winit sets up the application (and its event loop) with the first window
/// of the process: a one-frame invisible one, so the menu bar icon and the
/// hotkeys work before any real window opens.
fn launch(shared: &Arc<Shared>) {
    struct Boot;
    impl eframe::App for Boot {
        fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
            [0.0; 4]
        }
        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    let viewport = egui::ViewportBuilder::default()
        .with_title("blinker")
        .with_decorations(false)
        .with_transparent(true)
        .with_taskbar(false)
        .with_active(false)
        .with_position([-4000.0, -4000.0])
        .with_inner_size([1.0, 1.0]);
    crate::runtime::run_window(shared, viewport, false, |cc| {
        if let Some(h) = win::hwnd_of(cc) {
            win::cloak(h, true);
        }
        Boot
    });
}

fn shared() -> Option<&'static Arc<Shared>> {
    SHARED.get()
}

// ---------------------------------------------------------------------------
// Hotkeys

fn command_of(a: HotkeyAction) -> Command {
    match a {
        HotkeyAction::Region => Command::CaptureRegion,
        HotkeyAction::Screen => Command::CaptureScreen,
        HotkeyAction::Window => Command::CaptureWindow,
        HotkeyAction::Last => Command::CaptureLast,
        HotkeyAction::Record => Command::Record,
        HotkeyAction::RecordGif => Command::RecordGif,
        HotkeyAction::Pause => Command::RecordPause,
        HotkeyAction::Replay => Command::ReplaySave,
        HotkeyAction::Pick => Command::PickColor,
        HotkeyAction::Text => Command::CaptureText,
    }
}

fn register(shared: &Shared) {
    unregister();
    let cfg = shared.config();
    // no double click on a Mac (see the settings): a click acts at once
    unsafe { ffi::bk_tray_set_double(0) };
    let mut failed = std::collections::HashSet::new();
    let mut ids = HOTKEYS.lock().unwrap();
    let mut next = 1u32;
    for action in HotkeyAction::ALL {
        for binding in cfg.bindings(action) {
            let ok = hotkey::parse(binding)
                .and_then(|c| Some((vk::to_keycode(c.vk)?, vk::carbon_mods(c.mods))))
                .is_some_and(|(code, mods)| unsafe { ffi::bk_hotkey_register(next, code as u32, mods) } != 0);
            if ok {
                ids.push((next, action));
                next += 1;
            } else {
                failed.insert(binding.clone());
            }
        }
    }
    crate::config::save_failed_hotkeys(&failed);
    shared.hotkeys.lock().unwrap().failed = failed;
}

fn unregister() {
    for (id, _) in HOTKEYS.lock().unwrap().drain(..) {
        unsafe { ffi::bk_hotkey_unregister(id) };
    }
}

extern "C" fn on_hotkey(id: u32) {
    let action = HOTKEYS.lock().unwrap().iter().find(|(i, _)| *i == id).map(|(_, a)| *a);
    if let (Some(a), Some(s)) = (action, shared()) {
        // the menu bar menu holds the main thread while it's open: close it
        // so the capture can start
        unsafe { ffi::bk_tray_menu_close() };
        s.push(command_of(a));
    }
}

/// Signals from other processes (see `win::listen`).
extern "C" fn on_note(which: i32) {
    let Some(s) = shared() else { return };
    match which {
        1 => s.push(Command::OpenHome(HomeTab::Capture)),
        2 => {
            register(s);
            s.push(Command::Reconfigure);
        }
        3 => unregister(),
        4 => s.push(Command::Restart),
        5 => s.push(Command::Quit),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Menu bar icon

/// The replay started or stopped: the star tile with a red dot while it
/// records, without otherwise. Its own colors on any menu bar (not a template).
pub fn tray_state_changed(shared: &Shared) {
    let live = shared.replay_live.load(std::sync::atomic::Ordering::SeqCst);
    let icon = crate::theme::tray_icon(36, live, 2.0);
    let tip = ffi::cstr(if live { crate::i18n::l("blinker · идёт повтор", "blinker · replay is on") } else { "blinker" });
    unsafe { ffi::bk_tray_set_image(icon.as_raw().as_ptr(), icon.width() as i32, icon.height() as i32, 0, tip.as_ptr()) };
}

/// A `blinker://` link: 1 = update (the settings open and install it).
extern "C" fn on_url(which: i32) {
    if which == 1
        && let Some(s) = shared()
    {
        s.push(Command::OpenUpdate);
    }
}

extern "C" fn on_tray(right: i32, clicks: i32) {
    let Some(s) = shared() else { return };
    let cfg = s.config();
    let _ = clicks;
    let action = if right != 0 { cfg.tray_right_click } else { cfg.tray_click };
    tray_action(s, action);
}

/// What a click on the icon does (each button has its own setting).
fn tray_action(shared: &Shared, action: TrayAction) {
    match action {
        TrayAction::Region => shared.push(Command::CaptureRegion),
        TrayAction::Screen => shared.push(Command::CaptureScreen),
        TrayAction::Window => shared.push(Command::CaptureWindow),
        TrayAction::Last => shared.push(Command::CaptureLast),
        TrayAction::Record => shared.push(Command::Record),
        TrayAction::Settings => shared.push(Command::OpenHome(HomeTab::Capture)),
        TrayAction::Menu => show_menu(shared),
        TrayAction::Nothing => {}
    }
}

struct Row {
    id: i32,
    depth: i32,
    submenu: bool,
    label: CString,
    symbol: Option<CString>,
    key: Option<(CString, u64)>,
}

fn show_menu(shared: &Shared) {
    use crate::tray::{Row as Shown, TrayItem};
    let cfg = shared.config();
    // the native menu shows the shortcut itself, with the usual ⌃⌥⇧⌘ glyphs
    let key = |a: HotkeyAction| {
        let c = hotkey::parse(cfg.primary_binding(a))?;
        Some((ffi::cstr(vk::key_equivalent(c.vk)?), vk::flags_from_mods(c.mods)))
    };
    let item = |id: i32, depth: i32, label: &str, symbol: &str, k: Option<(CString, u64)>| Row {
        id,
        depth,
        submenu: false,
        label: ffi::cstr(label),
        symbol: (!symbol.is_empty()).then(|| ffi::cstr(symbol)),
        key: k,
    };
    let sep = || Row { id: 0, depth: 0, submenu: false, label: ffi::cstr(""), symbol: None, key: None };
    let recording = shared.recording.load(std::sync::atomic::Ordering::SeqCst);
    let live = shared.replay_live.load(std::sync::atomic::Ordering::SeqCst);
    // the user's layout (settings → tray menu): tiles in rows (depth -1),
    // each under its heading (-2), then the list
    let mut rows = Vec::new();
    for row in crate::tray::rows(&cfg.tray_menu, recording, cfg.replay_enabled) {
        match row {
            Shown::Heading(h) => rows.push(item(0, -2, &h, "", None)),
            Shown::Tiles(t) => {
                for i in t {
                    rows.push(item(i.id() as i32, -1, i.tile_label(), i.symbol(), i.hotkey().and_then(key)));
                }
            }
            Shown::Item(i) => {
                let label = if i == TrayItem::Replay && live { crate::i18n::l("сохранить повтор · идёт запись", "save the replay · recording") } else { i.label() };
                rows.push(item(i.id() as i32, 0, label, i.symbol(), i.hotkey().and_then(key)));
            }
            Shown::Separator => rows.push(sep()),
        }
    }
    let items: Vec<ffi::MenuItem> = rows
        .iter()
        .map(|r| ffi::MenuItem {
            id: r.id,
            depth: r.depth,
            submenu: r.submenu as i32,
            label: r.label.as_ptr(),
            symbol: r.symbol.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            key: r.key.as_ref().map_or(std::ptr::null(), |(k, _)| k.as_ptr()),
            mods: r.key.as_ref().map_or(0, |(_, m)| *m),
        })
        .collect();
    unsafe { ffi::bk_tray_menu(items.as_ptr(), items.len() as i32, on_menu) };
}

extern "C" fn on_menu(id: i32) {
    let Some(s) = shared() else { return };
    if let Some(item) = u32::try_from(id).ok().and_then(crate::tray::TrayItem::from_id) {
        s.push(item.command());
    }
}

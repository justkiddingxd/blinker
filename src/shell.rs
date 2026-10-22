//! The always-on part of Blinker: a background thread owning the tray icon
//! and the global hotkeys, pumping its own Win32 message loop.

use crate::config::{HotkeyAction, TrayAction};
use crate::hotkey;
use crate::icons::Icon;
use crate::runtime::{Command, HomeTab, Shared, WM_RELOAD_HOTKEYS, WM_RESTART, WM_SUSPEND_HOTKEYS, WM_TRAY_STATE};
use crate::theme;
use crate::traymenu::{self, Item};
use crate::win;
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::INFINITE;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_TRAY: u32 = WM_APP + 20;

pub struct ShellHandle {
    thread: std::thread::JoinHandle<()>,
    thread_id: u32,
}

impl ShellHandle {
    pub fn shutdown(self) {
        win::post_thread_message(self.thread_id, WM_QUIT);
        let _ = self.thread.join();
    }
}

pub fn spawn(shared: Arc<Shared>) -> ShellHandle {
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("blinker-shell".into())
        .spawn(move || {
            let tid = win::current_thread_id();
            // Make sure this thread has a message queue before anyone posts to it.
            let mut msg: MSG = unsafe { std::mem::zeroed() };
            unsafe { PeekMessageW(&mut msg, null_mut(), WM_USER, WM_USER, PM_NOREMOVE) };
            shared.shell_thread.store(tid, Ordering::Relaxed);
            let _ = tx.send(tid);
            run(shared);
        })
        .expect("spawn shell thread");
    let thread_id = rx.recv().unwrap_or(0);
    ShellHandle { thread, thread_id }
}

// ---------------------------------------------------------------------------
// Hotkeys: every binding of every action gets its own id.

struct Hotkeys {
    ids: Vec<(i32, HotkeyAction)>,
}

impl Hotkeys {
    fn register(&mut self, shared: &Shared) {
        self.unregister();
        let cfg = shared.config();
        let mut failed = std::collections::HashSet::new();
        let mut next = 1;
        for action in HotkeyAction::ALL {
            for binding in cfg.bindings(action) {
                let ok = match hotkey::parse(binding) {
                    Some(c) => unsafe { RegisterHotKey(null_mut(), next, c.mods | MOD_NOREPEAT, c.vk) != 0 },
                    None => false,
                };
                if ok {
                    self.ids.push((next, action));
                    next += 1;
                } else {
                    failed.insert(binding.clone());
                }
            }
        }
        crate::config::save_failed_hotkeys(&failed);
        shared.hotkeys.lock().unwrap().failed = failed;
    }

    fn unregister(&mut self) {
        for (id, _) in self.ids.drain(..) {
            unsafe { UnregisterHotKey(null_mut(), id) };
        }
    }

    fn command(&self, id: i32) -> Option<Command> {
        self.ids.iter().find(|(i, _)| *i == id).map(|(_, a)| match a {
            HotkeyAction::Region => Command::CaptureRegion,
            HotkeyAction::Screen => Command::CaptureScreen,
            HotkeyAction::Window => Command::CaptureWindow,
            HotkeyAction::Last => Command::CaptureLast,
            HotkeyAction::Record => Command::Record,
            HotkeyAction::RecordGif => Command::RecordGif,
            HotkeyAction::Pause => Command::RecordPause,
            HotkeyAction::Replay => Command::ReplaySave,
        })
    }
}

// ---------------------------------------------------------------------------
// Tray icon (plain Shell_NotifyIcon, so the menu can be our own).

#[derive(Clone, Copy)]
enum TrayEvent {
    Left,
    Double,
    Right,
    Recreate,
    /// Sent by the settings process.
    ReloadHotkeys,
    SuspendHotkeys,
    /// Sent by the settings after installing an update.
    Restart,
}

const CLICK_TIMER: usize = 1;

thread_local! {
    /// A double click has an action of its own.
    static DOUBLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static AFTER_DOUBLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static EVENTS: RefCell<Vec<TrayEvent>> = const { RefCell::new(Vec::new()) };
    static TASKBAR_CREATED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

unsafe extern "system" fn tray_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_TRAY {
        match lp as u32 {
            // with a double-click action, a single click waits to see
            // whether a second one comes (the timer below)
            WM_LBUTTONUP if DOUBLE.with(|d| d.get()) => {
                if !AFTER_DOUBLE.with(|a| a.replace(false)) {
                    unsafe { SetTimer(hwnd, CLICK_TIMER, GetDoubleClickTime(), None) };
                }
            }
            WM_LBUTTONUP => EVENTS.with(|e| e.borrow_mut().push(TrayEvent::Left)),
            WM_LBUTTONDBLCLK if DOUBLE.with(|d| d.get()) => {
                unsafe { KillTimer(hwnd, CLICK_TIMER) };
                // the button-up of the second click follows: not a click of its own
                AFTER_DOUBLE.with(|a| a.set(true));
                EVENTS.with(|e| e.borrow_mut().push(TrayEvent::Double));
            }
            WM_RBUTTONUP | WM_CONTEXTMENU => EVENTS.with(|e| e.borrow_mut().push(TrayEvent::Right)),
            _ => {}
        }
        return 0;
    }
    if msg == WM_TIMER && wp == CLICK_TIMER {
        unsafe { KillTimer(hwnd, CLICK_TIMER) };
        EVENTS.with(|e| e.borrow_mut().push(TrayEvent::Left));
        return 0;
    }
    if msg == WM_RESTART {
        EVENTS.with(|e| e.borrow_mut().push(TrayEvent::Restart));
        return 0;
    }
    if msg == WM_RELOAD_HOTKEYS || msg == WM_SUSPEND_HOTKEYS {
        let ev = if msg == WM_RELOAD_HOTKEYS { TrayEvent::ReloadHotkeys } else { TrayEvent::SuspendHotkeys };
        EVENTS.with(|e| e.borrow_mut().push(ev));
        return 0;
    }
    if msg != 0 && msg == TASKBAR_CREATED.with(|c| c.get()) {
        EVENTS.with(|e| e.borrow_mut().push(TrayEvent::Recreate));
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

struct Tray {
    hwnd: HWND,
    icon: HICON,
    /// Showing the "replay is on" icon.
    live: bool,
}

impl Tray {
    fn create() -> Option<Tray> {
        unsafe {
            let class = win::wide("BlinkerShell");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(tray_wndproc),
                hInstance: GetModuleHandleW(null()),
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassExW(&wc);
            let msg_name = win::wide("TaskbarCreated");
            TASKBAR_CREATED.with(|c| c.set(RegisterWindowMessageW(msg_name.as_ptr())));
            let title = win::wide("Blinker");
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            if hwnd.is_null() {
                return None;
            }
            let icon = make_icon(&theme::logo(32));
            let tray = Tray { hwnd, icon, live: false };
            tray.add();
            Some(tray)
        }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        d.hWnd = self.hwnd;
        d.uID = 1;
        d
    }

    fn tip(&self) -> String {
        if self.live { lf!("blinker · идёт повтор", "blinker · replay is on") } else { "blinker".into() }
    }

    fn add(&self) {
        let mut d = self.data();
        d.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        d.uCallbackMessage = WM_TRAY;
        d.hIcon = self.icon;
        let tip: Vec<u16> = self.tip().encode_utf16().take(127).collect();
        d.szTip[..tip.len()].copy_from_slice(&tip);
        unsafe { Shell_NotifyIconW(NIM_ADD, &d) };
    }

    /// Red sparkle while the instant replay records.
    fn set_live(&mut self, live: bool) {
        if live == self.live {
            return;
        }
        self.live = live;
        let color = if live { theme::DANGER } else { theme::ACCENT };
        let icon = unsafe { make_icon(&theme::logo_in(32, color)) };
        if icon.is_null() {
            return;
        }
        let old = std::mem::replace(&mut self.icon, icon);
        let mut d = self.data();
        d.uFlags = NIF_ICON | NIF_TIP;
        d.hIcon = self.icon;
        let tip: Vec<u16> = self.tip().encode_utf16().take(127).collect();
        d.szTip[..tip.len()].copy_from_slice(&tip);
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &d);
            DestroyIcon(old);
        }
    }

    fn anchor() -> (i32, i32) {
        win::cursor_pos()
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let d = self.data();
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &d);
            DestroyIcon(self.icon);
            DestroyWindow(self.hwnd);
        }
    }
}

/// HICON from a straight-alpha RGBA image.
unsafe fn make_icon(img: &image::RgbaImage) -> HICON {
    unsafe {
        let (w, h) = (img.width() as i32, img.height() as i32);
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let screen = GetDC(null_mut());
        let mut bits: *mut c_void = null_mut();
        let color = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        ReleaseDC(null_mut(), screen);
        if color.is_null() || bits.is_null() {
            return null_mut();
        }
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
        for (d, s) in dst.chunks_exact_mut(4).zip(img.as_raw().chunks_exact(4)) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = s[3];
        }
        let mask = CreateBitmap(w, h, 1, 1, null());
        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        DeleteObject(mask);
        DeleteObject(color);
        icon
    }
}

// ---------------------------------------------------------------------------

/// What a click on the tray icon does (each button has its own setting).
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

fn show_menu(shared: &Shared) {
    let cfg = shared.config();
    let hk = |a: HotkeyAction| hotkey::display_parts(cfg.primary_binding(a)).join(" ");
    let l = crate::i18n::l;
    let recording = shared.recording.load(std::sync::atomic::Ordering::SeqCst);
    // the main actions as big tiles, the rest as a list
    let tiles = if recording {
        vec![Item::new(11, Icon::Close, l("стоп", "stop"), ""), Item::new(12, Icon::Pause, l("пауза", "pause"), "")]
    } else {
        vec![
            Item::new(1, Icon::Region, crate::i18n::t("область"), ""),
            Item::new(2, Icon::Screen, l("экран", "screen"), ""),
            Item::new(3, Icon::Window, l("окно", "window"), ""),
            Item::new(9, Icon::Record, crate::i18n::t("видео"), ""),
            Item::new(13, Icon::Video, "gif", ""),
        ]
    };
    let mut items = vec![
        Item::tiles(tiles),
        Item::separator(),
        Item::new(4, Icon::Repeat, crate::i18n::t("повторить область"), &hk(HotkeyAction::Last)),
    ];
    if cfg.replay_enabled {
        let live = shared.replay_live.load(std::sync::atomic::Ordering::SeqCst);
        let label = if live { l("сохранить повтор · идёт", "save the replay · on") } else { crate::i18n::t("сохранить повтор") };
        items.push(Item::new(10, Icon::Repeat, label, &hk(HotkeyAction::Replay)));
    }
    items.extend([
        Item::separator(),
        Item::new(14, Icon::Frame, crate::i18n::l("история", "history"), ""),
        Item::new(6, Icon::Folder, crate::i18n::t("папка с снятым"), ""),
        Item::new(7, Icon::Settings, crate::i18n::t("настройки"), ""),
        Item::separator(),
        Item::new(8, Icon::Close, crate::i18n::t("выход"), ""),
    ]);
    let cmd = match traymenu::popup(items, Tray::anchor()) {
        Some(1) => Command::CaptureRegion,
        Some(2) => Command::CaptureScreen,
        Some(3) => Command::CaptureWindow,
        Some(4) => Command::CaptureLast,
        Some(6) => Command::OpenFolder,
        Some(7) => Command::OpenHome(HomeTab::Capture),
        Some(8) => Command::Quit,
        Some(9) | Some(11) => Command::Record,
        Some(10) => Command::ReplaySave,
        Some(12) => Command::RecordPause,
        Some(13) => Command::RecordGif,
        Some(14) => Command::OpenHistory,
        _ => return,
    };
    shared.push(cmd);
}

/// The replay started or stopped (any thread): the tray thread recolors the icon.
pub fn tray_state_changed(shared: &Shared) {
    let tid = shared.shell_thread.load(Ordering::Relaxed);
    if tid != 0 {
        win::post_thread_message(tid, WM_TRAY_STATE);
    }
}

fn run(shared: Arc<Shared>) {
    let mut tray = Tray::create();
    let mut hotkeys = Hotkeys { ids: Vec::new() };
    hotkeys.register(&shared);
    let activate = win::activation_event();
    // `blinker --quit` (or a quit left over from the previous run: cleared)
    let quit = win::quit_event();
    if !quit.is_null() {
        unsafe { windows_sys::Win32::System::Threading::ResetEvent(quit) };
    }
    let mut quitting = false;

    loop {
        let handles: Vec<HANDLE> = [activate, quit].into_iter().filter(|h| !h.is_null() && !(quitting && *h == quit)).collect();
        let r = unsafe { MsgWaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, INFINITE, QS_ALLINPUT) };
        if let Some(&h) = handles.get(r.wrapping_sub(WAIT_OBJECT_0) as usize) {
            if h == activate {
                shared.push(Command::OpenHome(HomeTab::Capture));
            } else {
                quitting = true;
                shared.push(Command::Quit);
            }
        }

        let mut msg: MSG = unsafe { std::mem::zeroed() };
        while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
            match msg.message {
                WM_QUIT => {
                    hotkeys.unregister();
                    drop(tray);
                    return;
                }
                WM_HOTKEY => {
                    if let Some(cmd) = hotkeys.command(msg.wParam as i32) {
                        shared.push(cmd);
                    }
                }
                WM_RELOAD_HOTKEYS => {
                    hotkeys.register(&shared);
                    shared.push(Command::Reconfigure);
                }
                WM_SUSPEND_HOTKEYS => hotkeys.unregister(),
                WM_RESTART => shared.push(Command::Restart),
                WM_TRAY_STATE => {
                    if let Some(t) = &mut tray {
                        t.set_live(shared.replay_live.load(Ordering::SeqCst));
                    }
                }
                _ => unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                },
            }
        }

        DOUBLE.with(|d| d.set(shared.config().tray_double_click != TrayAction::Nothing));
        let events: Vec<TrayEvent> = EVENTS.with(|e| std::mem::take(&mut *e.borrow_mut()));
        for ev in events {
            match ev {
                TrayEvent::Left => tray_action(&shared, shared.config().tray_click),
                TrayEvent::Double => tray_action(&shared, shared.config().tray_double_click),
                TrayEvent::ReloadHotkeys => {
                    hotkeys.register(&shared);
                    shared.push(Command::Reconfigure);
                }
                TrayEvent::SuspendHotkeys => hotkeys.unregister(),
                TrayEvent::Restart => shared.push(Command::Restart),
                TrayEvent::Right => tray_action(&shared, shared.config().tray_right_click),
                TrayEvent::Recreate => {
                    if let Some(t) = &tray {
                        t.add();
                    }
                }
            }
        }
    }
}

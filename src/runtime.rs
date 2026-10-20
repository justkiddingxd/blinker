//! Process-wide state shared between the shell thread (tray + hotkeys) and the
//! UI thread, plus the helper that opens one eframe window on demand.
//!
//! Blinker keeps no window (and no GPU context) alive while idle: each
//! overlay/editor/home window is a separate `run_native` call that returns when
//! the window closes. That keeps the idle footprint to a few megabytes.

use crate::config::{self, Config, log};
use crate::theme;
use eframe::egui;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use windows_sys::Win32::UI::WindowsAndMessaging::WM_APP;

pub const WM_RELOAD_HOTKEYS: u32 = WM_APP + 1;
pub const WM_SUSPEND_HOTKEYS: u32 = WM_APP + 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HomeTab {
    Record,
    Upload,
    #[default]
    Capture,
    Notify,
    Sound,
    Storage,
    Hotkeys,
    System,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    CaptureRegion,
    CaptureScreen,
    CaptureWindow,
    CaptureLast,
    /// Start a screen recording, or stop the running one.
    Record,
    /// Same, recorded straight into a GIF.
    RecordGif,
    RecordPause,
    /// Save the instant replay buffer.
    ReplaySave,
    /// Settings changed in another process (replay on/off...).
    Reconfigure,
    OpenHome(HomeTab),
    OpenFolder,
    Quit,
}

impl Command {
    pub fn is_capture(&self) -> bool {
        matches!(self, Command::CaptureRegion | Command::CaptureScreen | Command::CaptureWindow | Command::CaptureLast)
    }
}

/// Key combinations that failed to register (taken by another program).
#[derive(Default, Clone)]
pub struct HotkeyStatus {
    pub failed: std::collections::HashSet<String>,
}

pub struct Shared {
    queue: Mutex<VecDeque<Command>>,
    cv: Condvar,
    ctx: Mutex<Option<egui::Context>>,
    /// Cached config + the file time it was read at (reloaded when another process saves).
    config: Mutex<(Config, Option<std::time::SystemTime>)>,
    pub hotkeys: Mutex<HotkeyStatus>,
    pub shell_thread: AtomicU32,
    /// Last captured region (absolute physical px) for "repeat region".
    pub last_region: Mutex<Option<crate::win::IRect>>,
    /// A recording is running (the tray menu offers stop / pause then).
    pub recording: std::sync::atomic::AtomicBool,
}

impl Shared {
    pub fn new(config: Config) -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(VecDeque::new()),
            cv: Condvar::new(),
            ctx: Mutex::new(None),
            config: Mutex::new((config, config::modified())),
            hotkeys: Mutex::new(HotkeyStatus::default()),
            shell_thread: AtomicU32::new(0),
            last_region: Mutex::new(None),
            recording: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub fn push(&self, cmd: Command) {
        let mut q = self.queue.lock().unwrap();
        // Collapse repeated presses of the same key while we're busy.
        if q.back() != Some(&cmd) {
            q.push_back(cmd);
        }
        drop(q);
        self.cv.notify_all();
        if let Some(ctx) = self.ctx.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }

    pub fn wait(&self) -> Command {
        let mut q = self.queue.lock().unwrap();
        loop {
            if let Some(c) = q.pop_front() {
                return c;
            }
            q = self.cv.wait(q).unwrap();
        }
    }

    pub fn peek(&self) -> Option<Command> {
        self.queue.lock().unwrap().front().cloned()
    }

    /// Removes queued commands matching `pred`, returning whether any matched.
    pub fn take_where(&self, pred: impl Fn(&Command) -> bool) -> bool {
        let mut q = self.queue.lock().unwrap();
        let before = q.len();
        q.retain(|c| !pred(c));
        before != q.len()
    }

    pub fn config(&self) -> Config {
        let mut guard = self.config.lock().unwrap();
        let disk = config::modified();
        if disk.is_some() && disk != guard.1 {
            *guard = (config::load(), disk);
        }
        guard.0.clone()
    }

    /// Read-modify-write against the file, so changes made by other Blinker
    /// processes (settings, editor) are never overwritten with a stale copy.
    pub fn update_config(&self, f: impl FnOnce(&mut Config)) {
        let mut guard = self.config.lock().unwrap();
        let mut c = config::load();
        f(&mut c);
        // `load` applied the language from disk; the new value wins
        crate::i18n::apply(&c.lang);
        config::save(&c);
        *guard = (c, config::modified());
    }

    pub fn reload_hotkeys(&self) {
        self.post_to_shell(WM_RELOAD_HOTKEYS);
    }

    pub fn suspend_hotkeys(&self) {
        self.post_to_shell(WM_SUSPEND_HOTKEYS);
    }

    /// In the tray process: its own shell thread. In settings/editor
    /// processes: the tray process's hidden window.
    fn post_to_shell(&self, msg: u32) {
        let tid = self.shell_thread.load(Ordering::Relaxed);
        if tid != 0 {
            crate::win::post_thread_message(tid, msg);
        } else {
            crate::win::post_to_window_class("BlinkerShell", msg);
        }
    }
}

/// Opens one native window and blocks until it closes.
pub fn run_window<A: eframe::App + 'static>(
    shared: &Arc<Shared>,
    viewport: egui::ViewportBuilder,
    centered: bool,
    make: impl FnOnce(&eframe::CreationContext<'_>) -> A + 'static,
) {
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        run_and_return: true,
        centered,
        persist_window: false,
        ..Default::default()
    };
    let shared2 = shared.clone();
    let result = eframe::run_native(
        "Blinker",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            *shared2.ctx.lock().unwrap() = Some(cc.egui_ctx.clone());
            Ok(Box::new(make(cc)))
        }),
    );
    *shared.ctx.lock().unwrap() = None;
    crate::win::trim_memory();
    if let Err(e) = result {
        log::error(format!("window failed: {e}"));
    }
}

//! macOS platform layer with the API of the Win32 one (`src/win.rs`), so the
//! rest of blinker is shared. Everything here works in physical pixels too:
//!
//! macOS lays monitors out in points. Each monitor gets a pixel rectangle
//! whose origin is its point origin × the largest backing scale (so monitors
//! never overlap) and whose size is its real pixel size. A pixel on a monitor
//! is a real pixel of that monitor, which is what screenshots and recordings
//! need.

#![allow(dead_code)]

pub mod ffi;
pub mod hint;
pub mod vk;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::ffi::c_void;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

pub use crate::geom::{IRect, Monitor};

/// An `NSWindow *` of ours (foreign windows are `CGWindowID`s in `isize`s).
#[allow(clippy::upper_case_acronyms)]
pub type HWND = *mut c_void;

// ---------------------------------------------------------------------------
// Monitors and coordinates

#[derive(Clone, Copy, Debug)]
pub struct Display {
    pub id: u32,
    /// Frame in global points (top-left origin).
    pub pt: (f64, f64, f64, f64),
    pub scale: f64,
    pub refresh: f64,
    /// Pixels the notch takes at the top (0 without one).
    pub safe_top: i32,
    pub px: IRect,
    pub work: IRect,
}

impl Display {
    pub fn monitor(&self) -> Monitor {
        Monitor { rect: self.px, work: self.work, scale: self.scale as f32 }
    }
    fn contains_pt(&self, x: f64, y: f64) -> bool {
        let (l, t, w, h) = self.pt;
        x >= l && y >= t && x < l + w && y < t + h
    }
    fn dist_pt(&self, x: f64, y: f64) -> f64 {
        let (l, t, w, h) = self.pt;
        let dx = (l - x).max(0.0).max(x - (l + w));
        let dy = (t - y).max(0.0).max(y - (t + h));
        dx * dx + dy * dy
    }
    fn dist_px(&self, x: i32, y: i32) -> i64 {
        let r = self.px;
        let dx = (r.x - x).max(0).max(x - r.right()) as i64;
        let dy = (r.y - y).max(0).max(y - r.bottom()) as i64;
        dx * dx + dy * dy
    }
}

static DISPLAYS: Mutex<Vec<Display>> = Mutex::new(Vec::new());

/// Current monitors (primary first). AppKit answers on the main thread only:
/// other threads get the list the main thread saw last.
pub fn displays() -> Vec<Display> {
    let main = unsafe { ffi::bk_is_main_thread() } != 0;
    if !main {
        let cached = DISPLAYS.lock().unwrap().clone();
        if !cached.is_empty() {
            return cached;
        }
    }
    let mut raw = [ffi::Display::default(); 16];
    let n = unsafe { ffi::bk_displays(raw.as_mut_ptr(), raw.len() as i32) }.max(0) as usize;
    let raw = &raw[..n];
    let k = raw.iter().map(|d| d.scale).fold(1.0f64, f64::max);
    let list: Vec<Display> = raw
        .iter()
        .map(|d| {
            let s = d.scale.max(1.0);
            let px = IRect::new((d.x * k).round() as i32, (d.y * k).round() as i32, (d.w * s).round() as i32, (d.h * s).round() as i32);
            let wx = px.x + ((d.vx - d.x) * s).round() as i32;
            let wy = px.y + ((d.vy - d.y) * s).round() as i32;
            let work = IRect::new(wx, wy, (d.vw * s).round() as i32, (d.vh * s).round() as i32);
            let safe_top = (d.safe_top * s).round() as i32;
            Display { id: d.id, pt: (d.x, d.y, d.w, d.h), scale: s, refresh: d.refresh, safe_top, px, work }
        })
        .collect();
    if !list.is_empty() {
        *DISPLAYS.lock().unwrap() = list.clone();
    }
    list
}

fn display_at_pt(list: &[Display], x: f64, y: f64) -> Option<Display> {
    list.iter()
        .find(|d| d.contains_pt(x, y))
        .or_else(|| list.iter().min_by(|a, b| a.dist_pt(x, y).total_cmp(&b.dist_pt(x, y))))
        .copied()
}

fn display_at_px(list: &[Display], x: i32, y: i32) -> Option<Display> {
    list.iter().find(|d| d.px.contains(x, y)).or_else(|| list.iter().min_by_key(|d| d.dist_px(x, y))).copied()
}

/// The monitor holding the pixel (x, y), or the nearest one.
pub fn display_at(x: i32, y: i32) -> Option<Display> {
    display_at_px(&displays(), x, y)
}

pub fn pt_to_px(x: f64, y: f64) -> (i32, i32) {
    match display_at_pt(&displays(), x, y) {
        Some(d) => (d.px.x + ((x - d.pt.0) * d.scale).round() as i32, d.px.y + ((y - d.pt.1) * d.scale).round() as i32),
        None => (x.round() as i32, y.round() as i32),
    }
}

pub fn px_to_pt(x: i32, y: i32) -> (f64, f64) {
    match display_at(x, y) {
        Some(d) => (d.pt.0 + (x - d.px.x) as f64 / d.scale, d.pt.1 + (y - d.px.y) as f64 / d.scale),
        None => (x as f64, y as f64),
    }
}

/// A pixel rectangle in points, scaled by the monitor under its center.
pub fn rect_to_pt(r: IRect) -> (f64, f64, f64, f64, f64) {
    match display_at(r.x + r.w / 2, r.y + r.h / 2) {
        Some(d) => (
            d.pt.0 + (r.x - d.px.x) as f64 / d.scale,
            d.pt.1 + (r.y - d.px.y) as f64 / d.scale,
            r.w as f64 / d.scale,
            r.h as f64 / d.scale,
            d.scale,
        ),
        None => (r.x as f64, r.y as f64, r.w as f64, r.h as f64, 1.0),
    }
}

/// A point rectangle in pixels, scaled by the monitor under its center.
pub fn rect_to_px(x: f64, y: f64, w: f64, h: f64) -> IRect {
    match display_at_pt(&displays(), x + w / 2.0, y + h / 2.0) {
        Some(d) => IRect::new(
            d.px.x + ((x - d.pt.0) * d.scale).round() as i32,
            d.px.y + ((y - d.pt.1) * d.scale).round() as i32,
            (w * d.scale).round() as i32,
            (h * d.scale).round() as i32,
        ),
        None => IRect::new(x as i32, y as i32, w as i32, h as i32),
    }
}

pub fn monitors() -> Vec<Monitor> {
    displays().iter().map(Display::monitor).collect()
}

pub fn virtual_screen() -> IRect {
    let list = displays();
    let Some(first) = list.first() else { return IRect::new(0, 0, 1920, 1080) };
    let (mut l, mut t, mut r, mut b) = (first.px.x, first.px.y, first.px.right(), first.px.bottom());
    for d in &list {
        l = l.min(d.px.x);
        t = t.min(d.px.y);
        r = r.max(d.px.right());
        b = b.max(d.px.bottom());
    }
    IRect::from_ltrb(l, t, r, b)
}

pub fn cursor_pos() -> (i32, i32) {
    let (mut x, mut y) = (0.0, 0.0);
    unsafe { ffi::bk_cursor_pos(&mut x, &mut y) };
    pt_to_px(x, y)
}

pub fn monitor_at(monitors: &[Monitor], x: i32, y: i32) -> Option<Monitor> {
    monitors.iter().find(|m| m.rect.contains(x, y)).or_else(|| monitors.first()).copied()
}

/// Pixels at the top of the monitor at (x, y) hidden behind the camera
/// housing of MacBooks with a notch (0 elsewhere).
pub fn safe_top_at(x: i32, y: i32) -> i32 {
    displays().iter().find(|d| d.px.contains(x, y)).map_or(0, |d| d.safe_top)
}

/// Refresh rate (Hz) of the monitor at a screen point; 60 if unknown.
pub fn refresh_rate(x: i32, y: i32) -> u32 {
    display_at(x, y).map(|d| d.refresh.round() as u32).filter(|&r| r > 1).unwrap_or(60)
}

// ---------------------------------------------------------------------------
// Process

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Tray, hotkeys, overlay, toasts: no Dock icon, no menu bar.
    Tray = 1,
    /// Settings, editors: a normal app with a Dock icon.
    Regular = 2,
    /// A pinned screenshot: floats, no Dock icon.
    Pin = 3,
}

static KIND: AtomicU8 = AtomicU8::new(1);

fn kind() -> Kind {
    match KIND.load(Ordering::Relaxed) {
        2 => Kind::Regular,
        3 => Kind::Pin,
        _ => Kind::Tray,
    }
}

pub fn init_process() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let kind = if has("--pin") || has("--demo") {
        Kind::Pin
    } else if has("--settings") || has("--edit") || has("--video") || args.iter().any(|a| !a.starts_with("--") && Path::new(a).is_file()) {
        Kind::Regular
    } else {
        Kind::Tray
    };
    KIND.store(kind as u8, Ordering::Relaxed);
}

/// Options for the (one, reused) winit event loop of this process.
pub fn configure_event_loop(b: &mut eframe::EventLoopBuilder<eframe::UserEvent>) {
    use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
    let regular = kind() == Kind::Regular;
    b.with_activation_policy(if regular { ActivationPolicy::Regular } else { ActivationPolicy::Accessory });
    // the tray process has no menu bar: no ⌘Q quitting it from the overlay
    b.with_default_menu(regular);
    b.with_activate_ignoring_other_apps(regular);
}

/// Opened again from Finder or the Dock: the tray shows the settings.
pub extern "C" fn on_reopen(_: i32) {
    post(ACTIVATE);
}

fn apply_activation_policy() {
    static DONE: std::sync::Once = std::sync::Once::new();
    DONE.call_once(|| unsafe {
        ffi::bk_set_activation_policy((kind() == Kind::Regular) as i32);
        ffi::bk_on_reopen(on_reopen);
        if kind() == Kind::Regular {
            ffi::bk_install_key_monitor();
        }
        // settings, editors and pins go away with the tray
        if kind() != Kind::Tray {
            listen(QUIT, 0, on_quit);
        }
    });
}

extern "C" fn on_quit(_: i32) {
    std::process::exit(0);
}

/// Runs the application until `wake_main_loop` (between our windows; main thread).
pub fn run_idle() {
    unsafe { ffi::bk_run_idle() };
}

/// Makes a waiting `run_idle` return (any thread).
pub fn wake_main_loop() {
    unsafe { ffi::bk_wake_main() };
}

/// Remembers the app in front while one of our windows is open, and gives it
/// the focus back afterwards (an accessory app has nobody to hand it to).
pub struct FocusGuard(*mut c_void);

impl FocusGuard {
    pub fn take() -> Self {
        unsafe { ffi::bk_tray_protect() };
        FocusGuard(unsafe { ffi::bk_focus_take() })
    }
}

impl Drop for FocusGuard {
    fn drop(&mut self) {
        unsafe {
            ffi::bk_focus_restore(self.0);
            // the icon's window is kept out of winit's closing; should that
            // ever fail, the icon is made again
            ffi::bk_tray_revive();
        }
    }
}

pub fn trim_memory() {
    unsafe { ffi::bk_trim_memory() };
}

pub fn current_thread_id() -> u32 {
    0
}

pub fn post_thread_message(_thread_id: u32, _msg: u32) {}

// ---------------------------------------------------------------------------
// Single instance and signals between processes: a lock file held while a
// process lives, plus a Darwin notification to poke it.

fn note_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '.' }).collect();
    format!("app.blinker.{}", clean.trim_matches('.'))
}

fn lock_path(name: &str) -> std::path::PathBuf {
    let dir = crate::config::cache_dir().join("locks");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{}.lock", note_name(name)))
}

/// Holds `name`'s lock file; None if another process holds it.
fn hold(name: &str) -> Option<std::fs::File> {
    let f = std::fs::File::options().create(true).truncate(false).write(true).open(lock_path(name)).ok()?;
    f.try_lock().ok()?;
    Some(f)
}

/// Some process holds `name`'s lock.
fn held(name: &str) -> bool {
    match std::fs::File::options().create(true).truncate(false).write(true).open(lock_path(name)) {
        Ok(f) => {
            if f.try_lock().is_ok() {
                let _ = f.unlock();
                false
            } else {
                true
            }
        }
        Err(_) => false,
    }
}

const SHELL: &str = "BlinkerShell";
pub const ACTIVATE: &str = "Blinker.Activate";
/// "Quit" in the menu bar icon's menu: every blinker process ends.
pub const QUIT: &str = "Blinker.Quit";

pub struct InstanceGuard(#[allow(dead_code)] std::fs::File);

pub fn acquire_instance() -> Option<InstanceGuard> {
    hold(SHELL).map(InstanceGuard)
}

pub fn signal_existing_instance() {
    post(ACTIVATE);
}

/// Posts the Darwin notification for `name`.
pub fn post(name: &str) {
    let n = ffi::cstr(note_name(name));
    unsafe { ffi::bk_notify_post(n.as_ptr()) };
}

/// Calls `cb(which)` on the main thread whenever `name` is posted.
pub fn listen(name: &str, which: i32, cb: ffi::NotifyCb) {
    let n = ffi::cstr(note_name(name));
    unsafe { ffi::bk_notify_listen(n.as_ptr(), which, cb) };
}

static HELD: Mutex<Vec<std::fs::File>> = Mutex::new(Vec::new());

/// A named event this process waits on (see `wait_event`).
pub fn create_event(name: &str) -> Option<isize> {
    if let Some(f) = hold(name) {
        HELD.lock().unwrap().push(f);
    }
    let n = ffi::cstr(note_name(name));
    let token = unsafe { ffi::bk_notify_token(n.as_ptr()) };
    (token >= 0).then_some(token as isize)
}

pub fn wait_event(event: isize, ms: u32) -> bool {
    let until = std::time::Instant::now() + std::time::Duration::from_millis(ms as u64);
    loop {
        if unsafe { ffi::bk_notify_check(event as i32) } != 0 {
            return true;
        }
        if std::time::Instant::now() >= until {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// Signals a named event; false if no process created it.
pub fn signal_event(name: &str) -> bool {
    if !held(name) {
        return false;
    }
    post(name);
    true
}

/// "The tray process is running" (its lock is held).
pub fn window_class_exists(class: &str) -> bool {
    held(class)
}

/// Message to the tray process (`runtime::WM_*` codes).
pub fn post_to_window_class(class: &str, msg: u32) {
    post(&format!("{class}.{msg}"))
}

// ---------------------------------------------------------------------------
// Our windows

pub fn hwnd_of(h: &impl HasWindowHandle) -> Option<HWND> {
    let RawWindowHandle::AppKit(a) = h.window_handle().ok()?.as_raw() else { return None };
    let w = unsafe { ffi::bk_window_of_view(a.ns_view.as_ptr()) };
    if w.is_null() {
        return None;
    }
    apply_activation_policy();
    unsafe { ffi::bk_window_init(w) };
    Some(w)
}

/// One of our windows by its title (for egui viewports made without a handle).
pub fn find_window(title: &str) -> Option<HWND> {
    let t = ffi::cstr(title);
    let w = unsafe { ffi::bk_window_find(t.as_ptr()) };
    if w.is_null() {
        return None;
    }
    unsafe { ffi::bk_window_init(w) };
    Some(w)
}

pub fn set_window_rect(hwnd: HWND, r: IRect, topmost: bool) {
    let (x, y, w, h, _) = rect_to_pt(r);
    unsafe { ffi::bk_window_set_frame(hwnd, x, y, w, h, topmost as i32) };
}

pub fn window_rect(hwnd: HWND) -> IRect {
    let (mut x, mut y, mut w, mut h) = (0.0, 0.0, 0.0, 0.0);
    unsafe { ffi::bk_window_frame(hwnd, &mut x, &mut y, &mut w, &mut h) };
    rect_to_px(x, y, w, h)
}

/// Bounds of a foreign window (`CGWindowID`).
pub fn window_bounds(id: u32) -> Option<IRect> {
    let mut w = ffi::Win::default();
    (unsafe { ffi::bk_window_info(id, &mut w) } != 0).then(|| rect_to_px(w.x, w.y, w.w, w.h))
}

/// Bounds of a foreign window passed as a handle (its `CGWindowID`).
pub fn frame_bounds(hwnd: HWND) -> IRect {
    window_bounds(hwnd as usize as u32).unwrap_or_default()
}

pub fn window_scale(hwnd: HWND) -> f32 {
    unsafe { ffi::bk_window_scale(hwnd) as f32 }
}

pub fn disable_transitions(hwnd: HWND) {
    unsafe { ffi::bk_window_plain(hwnd, 1) };
}

pub fn no_dwm_border(hwnd: HWND) {
    unsafe { ffi::bk_window_plain(hwnd, 0) };
}

pub fn make_overlay(hwnd: HWND) {
    unsafe { ffi::bk_window_make_overlay(hwnd) };
}

/// Invisible but rendering (revealed after its first real frames).
pub fn cloak(hwnd: HWND, on: bool) {
    unsafe { ffi::bk_window_set_alpha(hwnd, if on { 0.0 } else { 1.0 }) };
}

pub fn force_foreground(hwnd: HWND) {
    unsafe { ffi::bk_window_activate(hwnd) };
}

pub fn is_foreground(hwnd: HWND) -> bool {
    unsafe { ffi::bk_window_is_key(hwnd) != 0 }
}

/// The system window buttons from `x` on, centered on `cy` points from the top.
pub fn window_buttons_at(hwnd: HWND, x: f32, cy: f32) {
    unsafe { ffi::bk_window_buttons_at(hwnd, x as f64, cy as f64) };
}

pub fn style_borderless(hwnd: HWND) {
    unsafe { ffi::bk_window_style_borderless(hwnd) };
}

/// Shows / hides without activating. A hidden settings process also leaves
/// the Dock until it is shown again.
pub fn show_window(hwnd: HWND, show: bool) {
    if kind() == Kind::Regular {
        unsafe { ffi::bk_set_activation_policy(show as i32) };
    }
    unsafe { ffi::bk_window_show(hwnd, show as i32) };
}

pub fn key_down(vk: u32) -> bool {
    vk::to_keycode(vk).is_some_and(|k| unsafe { ffi::bk_key_down(k) } != 0)
}

/// The last key pressed in our windows, as (MOD_* modifiers, virtual key).
pub fn take_key_press() -> Option<(u32, u32)> {
    let (mut code, mut flags) = (0u16, 0u64);
    if unsafe { ffi::bk_take_key_press(&mut code, &mut flags) } == 0 {
        return None;
    }
    Some((vk::mods_from_flags(flags), vk::from_keycode(code)?))
}

// ---------------------------------------------------------------------------
// Shell helpers

pub fn message_box(title: &str, text: &str) {
    let (t, m) = (ffi::cstr(title), ffi::cstr(text));
    unsafe { ffi::bk_message_box(t.as_ptr(), m.as_ptr()) };
}

pub fn reveal_in_explorer(path: &Path) {
    let p = ffi::cpath(path);
    unsafe { ffi::bk_reveal(p.as_ptr()) };
}

pub fn open_path(path: &Path) {
    let p = ffi::cpath(path);
    unsafe { ffi::bk_open(p.as_ptr()) };
}

/// Opens a web address in the default browser.
pub fn open_url(url: &str) {
    let u = ffi::cstr(url);
    unsafe { ffi::bk_open_url(u.as_ptr()) };
}

pub fn system_is_russian() -> bool {
    unsafe { ffi::bk_system_is_russian() != 0 }
}

const AGENT: &str = "app.blinker";

fn agent_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join("Library/LaunchAgents").join(format!("{AGENT}.plist")))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Open at login: a per-user launch agent that starts the tray (`--background`).
pub fn set_autostart(enable: bool) -> bool {
    let Some(path) = agent_path() else { return false };
    if !enable {
        return match std::fs::remove_file(&path) {
            Ok(()) => true,
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
        };
    }
    let exe = std::env::current_exe().unwrap_or_default();
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{AGENT}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>--background</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>LimitLoadToSessionType</key>
    <string>Aqua</string>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#,
        xml_escape(&exe.to_string_lossy())
    );
    if std::fs::read_to_string(&path).ok().as_deref() == Some(plist.as_str()) {
        return true;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&path, plist).is_ok()
}

/// Screen recording permission (asks once when `request`).
pub fn screen_access(request: bool) -> bool {
    unsafe { ffi::bk_screen_access(request as i32) != 0 }
}

/// An HTTP request for `upload`: (status, response text, NSURLError code on failure).
pub fn http_request(
    method: &str,
    url: &str,
    headers: &[(&str, String)],
    body: &[u8],
    progress: &std::sync::atomic::AtomicU32,
) -> (u32, String, Option<i32>) {
    extern "C" fn on_progress(ctx: *mut c_void, f: f64) {
        let p = unsafe { &*(ctx as *const std::sync::atomic::AtomicU32) };
        p.store((f.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
    }
    let url = ffi::cstr(url);
    let method = ffi::cstr(method);
    let keys: Vec<_> = headers.iter().map(|(k, _)| ffi::cstr(k)).collect();
    let values: Vec<_> = headers.iter().map(|(_, v)| ffi::cstr(v)).collect();
    let kp: Vec<_> = keys.iter().map(|k| k.as_ptr()).collect();
    let vp: Vec<_> = values.iter().map(|v| v.as_ptr()).collect();
    let (mut status, mut resp, mut code) = (0, std::ptr::null_mut(), 0);
    let ok = unsafe {
        ffi::bk_http_request(
            method.as_ptr(),
            url.as_ptr(),
            kp.as_ptr(),
            vp.as_ptr(),
            kp.len() as i32,
            body.as_ptr(),
            body.len(),
            on_progress,
            progress as *const _ as *mut c_void,
            &mut status,
            &mut resp,
            &mut code,
        )
    };
    let text = if resp.is_null() {
        String::new()
    } else {
        let s = unsafe { std::ffi::CStr::from_ptr(resp) }.to_string_lossy().into_owned();
        unsafe { ffi::bk_free(resp as *mut c_void) };
        s
    };
    if ok != 0 { (status as u32, text, None) } else { (0, text, Some(code)) }
}

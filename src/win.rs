//! Thin wrappers over the Win32 APIs Blinker needs. Everything here works in
//! physical pixels; DPI awareness is set to per-monitor v2 at startup.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::ffi::c_void;
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub use crate::geom::{IRect, Monitor};
pub use windows_sys::Win32::Foundation::HWND;

impl From<RECT> for IRect {
    fn from(r: RECT) -> Self {
        IRect::from_ltrb(r.left, r.top, r.right, r.bottom)
    }
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn init_process() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let id = wide("Blinker.App");
        SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
    }
}

// ---------------------------------------------------------------------------
// Single instance

pub struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

const INSTANCE_MUTEX: &str = "Local\\Blinker.Instance";
const ACTIVATE_EVENT: &str = "Local\\Blinker.Activate";

pub fn acquire_instance() -> Option<InstanceGuard> {
    unsafe {
        let name = wide(INSTANCE_MUTEX);
        let h = CreateMutexW(null(), 1, name.as_ptr());
        if !h.is_null() && GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(h);
            return None;
        }
        Some(InstanceGuard(h))
    }
}

/// Auto-reset event a second launch signals to bring the running instance forward.
pub fn activation_event() -> HANDLE {
    let name = wide(ACTIVATE_EVENT);
    unsafe { CreateEventW(null(), 0, 0, name.as_ptr()) }
}

pub fn signal_existing_instance() {
    let name = wide(ACTIVATE_EVENT);
    unsafe {
        let h = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
        if !h.is_null() {
            SetEvent(h);
            CloseHandle(h);
        }
    }
}

// ---------------------------------------------------------------------------
// Windows / monitors

pub fn hwnd_of(h: &impl HasWindowHandle) -> Option<HWND> {
    match h.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(w) => Some(w.hwnd.get() as HWND),
        _ => None,
    }
}

pub fn virtual_screen() -> IRect {
    unsafe {
        IRect::new(
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

pub fn cursor_pos() -> (i32, i32) {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn cb(hmon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> windows_sys::core::BOOL {
        let out = unsafe { &mut *(data as *mut Vec<Monitor>) };
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if unsafe { GetMonitorInfoW(hmon, &mut info) } != 0 {
            let (mut dx, mut dy) = (96u32, 96u32);
            unsafe { GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
            out.push(Monitor {
                rect: info.rcMonitor.into(),
                work: info.rcWork.into(),
                scale: dx as f32 / 96.0,
            });
        }
        1
    }
    let mut out: Vec<Monitor> = Vec::new();
    unsafe {
        EnumDisplayMonitors(null_mut(), null(), Some(cb), &mut out as *mut _ as LPARAM);
    }
    out
}

/// Refresh rate (Hz) of the monitor at a screen point; 60 if unknown.
pub fn refresh_rate(x: i32, y: i32) -> u32 {
    unsafe {
        let hmon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFOEXW = std::mem::zeroed();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(hmon, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) == 0 {
            return 60;
        }
        let mut dm: DEVMODEW = std::mem::zeroed();
        dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        if EnumDisplaySettingsW(info.szDevice.as_ptr(), ENUM_CURRENT_SETTINGS, &mut dm) == 0 || dm.dmDisplayFrequency <= 1 {
            return 60;
        }
        dm.dmDisplayFrequency
    }
}

/// Pixels at the top of a monitor hidden by a camera notch: none on Windows.
pub fn safe_top_at(_x: i32, _y: i32) -> i32 {
    0
}

pub fn monitor_at(monitors: &[Monitor], x: i32, y: i32) -> Option<Monitor> {
    monitors
        .iter()
        .find(|m| m.rect.contains(x, y))
        .or_else(|| monitors.first())
        .copied()
}

pub fn set_window_rect(hwnd: HWND, r: IRect, topmost: bool) {
    unsafe {
        let insert_after = if topmost { HWND_TOPMOST } else { null_mut() };
        let mut flags = SWP_NOACTIVATE;
        if !topmost {
            flags |= SWP_NOZORDER;
        }
        SetWindowPos(hwnd, insert_after, r.x, r.y, r.w, r.h, flags);
    }
}

/// Visible window bounds (without the invisible resize border / shadow).
pub fn frame_bounds(hwnd: HWND) -> IRect {
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut r as *mut RECT as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
    } == 0;
    if ok { r.into() } else { window_rect(hwnd) }
}

pub fn window_rect(hwnd: HWND) -> IRect {
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetWindowRect(hwnd, &mut r) };
    r.into()
}

pub fn window_scale(hwnd: HWND) -> f32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }
}

fn dwm_set<T>(hwnd: HWND, attr: DWMWINDOWATTRIBUTE, value: T) {
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            attr as u32,
            &value as *const T as *const c_void,
            std::mem::size_of::<T>() as u32,
        );
    }
}

/// Our popups must appear/disappear instantly, without the fade animation.
pub fn disable_transitions(hwnd: HWND) {
    dwm_set(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, 1i32);
}

pub fn force_foreground(hwnd: HWND) {
    unsafe {
        let fg = GetForegroundWindow();
        if fg == hwnd {
            return;
        }
        let cur = GetCurrentThreadId();
        let fg_thread = if fg.is_null() { 0 } else { GetWindowThreadProcessId(fg, null_mut()) };
        let attached = fg_thread != 0 && fg_thread != cur && AttachThreadInput(cur, fg_thread, 1) != 0;
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
        if attached {
            AttachThreadInput(cur, fg_thread, 0);
        }
        if GetForegroundWindow() != hwnd {
            // Windows refuses to hand over focus unless we "own" the last input
            // event: a synthetic Alt tap satisfies the foreground lock.
            keybd_event(VK_MENU as u8, 0, KEYEVENTF_EXTENDEDKEY, 0);
            keybd_event(VK_MENU as u8, 0, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP, 0);
            SetForegroundWindow(hwnd);
            SetFocus(hwnd);
        }
    }
}

/// No DWM outline or rounding: for windows that draw their own frame.
pub fn no_dwm_border(hwnd: HWND) {
    dwm_set(hwnd, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE);
    dwm_set(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND);
}

/// DWM cloaking: the window exists and renders, but isn't shown yet.
/// Used to reveal windows only after their first real frame (no white flash).
pub fn cloak(hwnd: HWND, on: bool) {
    dwm_set(hwnd, DWMWA_CLOAK, if on { 1i32 } else { 0i32 });
}

pub fn is_foreground(hwnd: HWND) -> bool {
    unsafe { GetForegroundWindow() == hwnd }
}

/// Look of our undecorated windows: rounded corners, hairline border, DWM shadow.
pub fn style_borderless(hwnd: HWND) {
    let border = 0x2B | 0x2B << 8 | 0x28 << 16;
    dwm_set(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, 1i32);
    dwm_set(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUNDSMALL);
    dwm_set(hwnd, DWMWA_BORDER_COLOR, border as u32);
    let margins = MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 0, cyBottomHeight: 1 };
    unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) };
}

pub fn key_down(vk: u32) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

/// Posts `msg` to the first top-level window of the given class (another process).
pub fn post_to_window_class(class: &str, msg: u32) {
    let class = wide(class);
    unsafe {
        let hwnd = FindWindowW(class.as_ptr(), null());
        if !hwnd.is_null() {
            PostMessageW(hwnd, msg, 0, 0);
        }
    }
}

/// Plain topmost warning box (only for failures the user must know about).
pub fn message_box(title: &str, text: &str) {
    let (t, m) = (wide(title), wide(text));
    unsafe { MessageBoxW(null_mut(), m.as_ptr(), t.as_ptr(), MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND) };
}

pub fn window_class_exists(class: &str) -> bool {
    let class = wide(class);
    unsafe { !FindWindowW(class.as_ptr(), null()).is_null() }
}

pub fn show_window(hwnd: HWND, show: bool) {
    unsafe { ShowWindow(hwnd, if show { SW_SHOWNA } else { SW_HIDE }) };
}

/// Auto-reset named event (handle as isize so it can cross threads).
pub fn create_event(name: &str) -> Option<isize> {
    let n = wide(name);
    let h = unsafe { CreateEventW(null(), 0, 0, n.as_ptr()) };
    (!h.is_null()).then_some(h as isize)
}

pub fn wait_event(event: isize, ms: u32) -> bool {
    unsafe { WaitForSingleObject(event as HANDLE, ms) == WAIT_OBJECT_0 }
}

/// Sets an existing named event; false if nobody created it.
pub fn signal_event(name: &str) -> bool {
    let n = wide(name);
    unsafe {
        let h = OpenEventW(EVENT_MODIFY_STATE, 0, n.as_ptr());
        if h.is_null() {
            return false;
        }
        SetEvent(h);
        CloseHandle(h);
        true
    }
}

pub fn post_thread_message(thread_id: u32, msg: u32) {
    if thread_id != 0 {
        unsafe { PostThreadMessageW(thread_id, msg, 0, 0) };
    }
}

// ---------------------------------------------------------------------------
// Shell helpers

pub fn reveal_in_explorer(path: &Path) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn();
}

pub fn open_path(path: &Path) {
    let p = wide(&path.to_string_lossy());
    let verb = wide("open");
    unsafe {
        ShellExecuteW(null_mut(), verb.as_ptr(), p.as_ptr(), null(), null(), SW_SHOWNORMAL);
    }
}

/// Opens a web address in the default browser.
pub fn open_url(url: &str) {
    let u = wide(url);
    let verb = wide("open");
    unsafe {
        ShellExecuteW(null_mut(), verb.as_ptr(), u.as_ptr(), null(), null(), SW_SHOWNORMAL);
    }
}

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "Blinker";

pub fn set_autostart(enable: bool) -> bool {
    unsafe {
        let mut key: HKEY = null_mut();
        let sub = wide(RUN_KEY);
        if RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_SET_VALUE, &mut key) != 0 {
            return false;
        }
        RegDeleteValueW(key, wide("Blinkshot").as_ptr()); // old name
        let name = wide(RUN_VALUE);
        let ok = if enable {
            let exe = std::env::current_exe().unwrap_or_default();
            let val = wide(&format!("\"{}\" --background", exe.display()));
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                val.as_ptr() as *const u8,
                (val.len() * 2) as u32,
            ) == 0
        } else {
            let r = RegDeleteValueW(key, name.as_ptr());
            r == 0 || r == ERROR_FILE_NOT_FOUND
        };
        RegCloseKey(key);
        ok
    }
}

/// A shortcut in the Start menu, which is how Windows search finds apps:
/// made on the first start, pointed at this exe again after it moved.
pub fn ensure_start_menu_entry() {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile, STGM_READ};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{Interface, PCWSTR};
    let (Some(appdata), Ok(exe)) = (std::env::var_os("APPDATA"), std::env::current_exe()) else { return };
    let lnk = std::path::PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs\blinker.lnk");
    let w = |p: &std::path::Path| -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    };
    let r = (|| -> windows::core::Result<()> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let file: IPersistFile = link.cast()?;
            let lnk_w = w(&lnk);
            if lnk.exists() && file.Load(PCWSTR(lnk_w.as_ptr()), STGM_READ).is_ok() {
                let mut buf = [0u16; 1024];
                if link.GetPath(&mut buf, null_mut(), 0).is_ok() {
                    let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
                    if std::path::PathBuf::from(String::from_utf16_lossy(&buf[..len])) == exe {
                        return Ok(());
                    }
                }
            }
            let exe_w = w(&exe);
            link.SetPath(PCWSTR(exe_w.as_ptr()))?;
            link.SetIconLocation(PCWSTR(exe_w.as_ptr()), 0)?;
            if let Some(dir) = exe.parent() {
                link.SetWorkingDirectory(PCWSTR(w(dir).as_ptr()))?;
            }
            let desc = wide(crate::i18n::l("blinker — скриншоты и запись экрана", "blinker — screenshots and screen recording"));
            link.SetDescription(PCWSTR(desc.as_ptr()))?;
            file.Save(PCWSTR(lnk_w.as_ptr()), true)?;
        }
        Ok(())
    })();
    if let Err(e) = r {
        crate::config::log::error(format!("start menu shortcut: {e}"));
    }
}

pub fn current_thread_id() -> u32 {
    unsafe { GetCurrentThreadId() }
}

/// Hands unused pages back to the OS after a window closes, so the tray
/// process doesn't keep the GPU driver's transient allocations resident.
pub fn trim_memory() {
    unsafe {
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}


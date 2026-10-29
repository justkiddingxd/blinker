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

/// A crash in native code (an access violation and the like) writes its
/// code and where in blinker.exe it happened into the log, then goes on as
/// it would (Windows' report, the process ends).
pub fn log_native_crashes() {
    use windows_sys::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, SetUnhandledExceptionFilter};
    unsafe extern "system" fn on_crash(info: *const EXCEPTION_POINTERS) -> i32 {
        unsafe {
            let rec = if info.is_null() { null_mut() } else { (*info).ExceptionRecord };
            let (code, addr) = if rec.is_null() { (0, 0) } else { ((*rec).ExceptionCode as u32, (*rec).ExceptionAddress as usize) };
            let base = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(null()) as usize;
            crate::config::log::error(format!("crash: exception 0x{code:08x} at blinker.exe+0x{:x}", addr.wrapping_sub(base)));
        }
        0 // EXCEPTION_CONTINUE_SEARCH
    }
    unsafe { SetUnhandledExceptionFilter(Some(on_crash)) };
}

pub fn init_process() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let id = wide("Blinker.App");
        SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
    }
    // settings, history, editors and pins go away with the tray
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let child = ["--pin", "--text", "--settings", "--history", "--edit", "--video"].iter().any(|f| has(f))
        || args.iter().any(|a| !a.starts_with("--") && Path::new(a).is_file());
    if child {
        let ev = quit_event();
        if !ev.is_null() {
            let ev = ev as isize;
            std::thread::spawn(move || {
                if unsafe { WaitForSingleObject(ev as HANDLE, INFINITE) } == WAIT_OBJECT_0 {
                    std::process::exit(0);
                }
            });
        }
    }
}

/// Set when the tray quits: every blinker process waits on it. Manual reset,
/// so one signal reaches them all.
const QUIT_EVENT: &str = "Local\\Blinker.Quit";

pub fn quit_event() -> HANDLE {
    let name = wide(QUIT_EVENT);
    unsafe { CreateEventW(null(), 1, 0, name.as_ptr()) }
}

/// Tells every blinker process to quit (the tray included).
pub fn signal_quit() {
    let ev = quit_event();
    if !ev.is_null() {
        unsafe {
            SetEvent(ev);
            CloseHandle(ev);
        }
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

/// Each monitor's number as Windows' display settings show it (from its
/// device name, "\\.\DISPLAY2" → 2), by its rectangle.
pub fn monitor_numbers() -> Vec<(IRect, u32)> {
    unsafe extern "system" fn cb(hmon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> windows_sys::core::BOOL {
        let out = unsafe { &mut *(data as *mut Vec<(IRect, u32)>) };
        let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(hmon, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) } != 0 {
            let name = String::from_utf16_lossy(&info.szDevice);
            let digits: String = name.trim_end_matches('\0').chars().rev().take_while(char::is_ascii_digit).collect();
            let n = digits.chars().rev().collect::<String>().parse().unwrap_or(0);
            out.push((info.monitorInfo.rcMonitor.into(), n));
        }
        1
    }
    let mut out: Vec<(IRect, u32)> = Vec::new();
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

/// The capture overlay is a bare popup over the whole desktop. winit's
/// undecorated window keeps WS_CAPTION and the system buttons and only hides
/// them by calling the whole window client area; where that fails, the title
/// bar shows (it can be dragged, the picture is pushed down past the taskbar).
/// The frame styles come off here; true if there were any. Checked every
/// frame: winit puts its styles back (when it shows the window, for one).
///
/// And the client area is the whole window: for the drop shadow of a
/// borderless window winit leaves its top row out (`top += 1` in its
/// WM_NCCALCSIZE), which over the screens is a 1 px strip of nothing.
pub fn make_overlay(hwnd: HWND) -> bool {
    unsafe extern "system" fn whole_client(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
        if msg == WM_NCCALCSIZE && wp != 0 {
            // the proposed rectangle stays as it is: client = window
            return 0;
        }
        unsafe { windows_sys::Win32::UI::Shell::DefSubclassProc(h, msg, wp, lp) }
    }
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let frame = WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
        let ex_frame = WS_EX_DLGMODALFRAME | WS_EX_CLIENTEDGE | WS_EX_WINDOWEDGE | WS_EX_STATICEDGE;
        if style & frame == 0 && ex & ex_frame == 0 {
            return false;
        }
        // before winit's own procedure (the same id again only updates it)
        windows_sys::Win32::UI::Shell::SetWindowSubclass(hwnd, Some(whole_client), 0x626C6B, 0);
        SetWindowLongPtrW(hwnd, GWL_STYLE, ((style & !frame) | WS_POPUP) as isize);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex & !ex_frame) as isize);
        SetWindowPos(hwnd, null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        true
    }
}

/// Our popups must appear/disappear instantly, without the fade animation.
pub fn disable_transitions(hwnd: HWND) {
    dwm_set(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, 1i32);
}

/// A window already open, asked for again: out of the taskbar if it was
/// minimized, and in front.
pub fn raise(hwnd: HWND) {
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
    }
    force_foreground(hwnd);
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

/// No system shadow around the window: it would outline the whole window,
/// transparent parts included. winit keeps a 1 px frame at the top of a
/// borderless window to get that shadow; a window region without that row
/// drops both (DWM draws no shadow for a window with a region).
pub fn no_shadow(hwnd: HWND) {
    unsafe {
        let rgn = windows_sys::Win32::Graphics::Gdi::CreateRectRgn(0, 1, 32767, 32767);
        // the window owns the region from here on
        SetWindowRgn(hwnd, rgn, 1);
    }
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
        // the asking process holds the foreground (a hotkey, a tray click):
        // it passes it on, so the window it wakes can come forward
        AllowSetForegroundWindow(ASFW_ANY);
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

/// The NVIDIA overlay is installed (the NVIDIA App or GeForce Experience):
/// its capture hook is in System32.
pub fn nvidia_overlay() -> bool {
    let sys = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
    sys.join("System32").join("nvspcap64.dll").is_file()
}

/// Moves a file to the Recycle Bin (it can be brought back from there).
pub fn trash(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW, SHFileOperationW};
    // a list of paths, each ended by a zero, the list by another
    let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
    from.extend([0, 0]);
    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.wFunc = FO_DELETE;
    op.pFrom = from.as_ptr();
    op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as u16;
    unsafe { SHFileOperationW(&mut op) == 0 && op.fAnyOperationsAborted == 0 }
}

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

/// Some program opens `scheme://` links (the Discord app: `discord`).
pub fn url_scheme_registered(scheme: &str) -> bool {
    let sub = wide(scheme);
    let mut key: HKEY = null_mut();
    unsafe {
        let ok = RegOpenKeyExW(HKEY_CLASSES_ROOT, sub.as_ptr(), 0, KEY_READ, &mut key) == 0;
        if ok {
            RegCloseKey(key);
        }
        ok
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

/// `blinker://` links open this exe (`blinker://update`: the "update" button
/// in blinker's Discord, through blinker.cool/update). Written for the
/// current user on every start, so it follows the exe when it moves.
pub fn register_url_scheme() {
    let Ok(exe) = std::env::current_exe() else { return };
    let set = |sub: &str, name: Option<&str>, value: &str| unsafe {
        let mut key: HKEY = null_mut();
        let path = wide(&format!("Software\\Classes\\blinker{sub}"));
        if RegCreateKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, null(), 0, KEY_SET_VALUE, null(), &mut key, null_mut()) != 0 {
            return;
        }
        let name = name.map(wide);
        let val = wide(value);
        RegSetValueExW(key, name.as_ref().map_or(null(), |n| n.as_ptr()), 0, REG_SZ, val.as_ptr() as *const u8, (val.len() * 2) as u32);
        RegCloseKey(key);
    };
    set("", None, "URL:blinker");
    set("", Some("URL Protocol"), "");
    set("\\DefaultIcon", None, &format!("\"{}\",0", exe.display()));
    set("\\shell\\open\\command", None, &format!("\"{}\" \"%1\"", exe.display()));
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


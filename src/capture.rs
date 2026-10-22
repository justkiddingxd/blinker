//! Screen grabbing (GDI BitBlt of the composed desktop) and window discovery.

use crate::win::{self, IRect, Monitor};
use image::RgbaImage;
use std::ffi::c_void;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Debug)]
pub struct WindowInfo {
    /// In snapshot image coordinates.
    pub rect: IRect,
    pub hwnd: isize,
    #[allow(dead_code)]
    pub title: String,
    /// Parts of the window worth taking alone (a browser's page, a panel),
    /// in snapshot image coordinates, smallest first.
    pub parts: Vec<IRect>,
}

/// A frozen copy of the whole virtual desktop plus the geometry the overlay
/// needs for snapping. All rects are relative to the image (0,0 = `origin`).
pub struct Snapshot {
    pub image: RgbaImage,
    pub origin: (i32, i32),
    pub windows: Vec<WindowInfo>,
    pub monitors: Vec<Monitor>,
}

impl Snapshot {
    pub fn monitor_at(&self, x: i32, y: i32) -> Monitor {
        win::monitor_at(&self.monitors, x, y).unwrap_or(Monitor {
            rect: IRect::new(0, 0, self.image.width() as i32, self.image.height() as i32),
            work: IRect::new(0, 0, self.image.width() as i32, self.image.height() as i32),
            scale: 1.0,
        })
    }
}

pub fn grab_desktop() -> Option<Snapshot> {
    let vs = win::virtual_screen();
    // Enumerate before grabbing: cheap, and the z-order matches what we freeze.
    let windows = enum_windows()
        .into_iter()
        .filter_map(|w| {
            let r = w.rect.intersect(&vs);
            (!r.is_empty()).then(|| WindowInfo {
                rect: r.offset(-vs.x, -vs.y),
                hwnd: w.hwnd,
                title: w.title,
                parts: parts(w.hwnd as HWND, w.rect).into_iter().filter_map(|p| Some(p.intersect(&vs)).filter(|p| !p.is_empty())).map(|p| p.offset(-vs.x, -vs.y)).collect(),
            })
        })
        .collect();
    let monitors = win::monitors()
        .into_iter()
        .map(|m| Monitor {
            rect: m.rect.offset(-vs.x, -vs.y),
            work: m.work.offset(-vs.x, -vs.y),
            scale: m.scale,
        })
        .collect();
    let image = grab_rect(vs)?;
    Some(Snapshot { image, origin: (vs.x, vs.y), windows, monitors })
}

/// Grabs an absolute screen rectangle (physical pixels).
pub fn grab_rect(r: IRect) -> Option<RgbaImage> {
    if r.is_empty() {
        return None;
    }
    unsafe {
        let screen = GetDC(null_mut());
        if screen.is_null() {
            return None;
        }
        let mem = CreateCompatibleDC(screen);
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = r.w;
        bmi.bmiHeader.biHeight = -r.h; // top-down
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut c_void = null_mut();
        let bmp = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let mut result = None;
        if !bmp.is_null() && !bits.is_null() {
            let old = SelectObject(mem, bmp);
            if BitBlt(mem, 0, 0, r.w, r.h, screen, r.x, r.y, SRCCOPY) != 0 {
                GdiFlush();
                let len = r.w as usize * r.h as usize * 4;
                let src = std::slice::from_raw_parts(bits as *const u8, len);
                let mut buf = vec![0u8; len];
                for (d, s) in buf.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = 255;
                }
                result = RgbaImage::from_raw(r.w as u32, r.h as u32, buf);
            }
            SelectObject(mem, old);
            DeleteObject(bmp);
        }
        DeleteDC(mem);
        ReleaseDC(null_mut(), screen);
        result
    }
}

struct RawWindow {
    rect: IRect,
    hwnd: isize,
    title: String,
}

fn enum_windows() -> Vec<RawWindow> {
    unsafe extern "system" fn cb(hwnd: HWND, data: LPARAM) -> windows_sys::core::BOOL {
        let out = unsafe { &mut *(data as *mut Vec<RawWindow>) };
        if let Some(w) = unsafe { inspect(hwnd) } {
            out.push(w);
        }
        1
    }
    let mut out = Vec::new();
    unsafe { EnumWindows(Some(cb), &mut out as *mut _ as LPARAM) };
    out
}

/// The child windows of `hwnd` that make a part of it worth a screenshot of
/// its own: a browser's page (Chromium and Electron draw it in a child
/// window), an editor, a panel. Smallest first.
fn parts(hwnd: HWND, window: IRect) -> Vec<IRect> {
    unsafe extern "system" fn cb(child: HWND, data: LPARAM) -> windows_sys::core::BOOL {
        let out = unsafe { &mut *(data as *mut Vec<IRect>) };
        if out.len() < 64 && unsafe { IsWindowVisible(child) } != 0 {
            let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            if unsafe { GetWindowRect(child, &mut r) } != 0 {
                out.push(r.into());
            }
        }
        1
    }
    let mut all: Vec<IRect> = Vec::new();
    unsafe { EnumChildWindows(hwnd, Some(cb), &mut all as *mut _ as LPARAM) };
    let area = |r: &IRect| r.w as i64 * r.h as i64;
    let mut out: Vec<IRect> = Vec::new();
    for r in all {
        let r = r.intersect(&window);
        // big enough to mean something, clearly smaller than the window
        if r.w < 80 || r.h < 48 || area(&r) * 100 > area(&window) * 96 {
            continue;
        }
        if !out.contains(&r) {
            out.push(r);
        }
    }
    out.sort_by_key(area);
    out
}

unsafe fn inspect(hwnd: HWND) -> Option<RawWindow> {
    unsafe {
        if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == GetCurrentProcessId() {
            return None;
        }
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            &mut cloaked as *mut u32 as *mut c_void,
            4,
        ) == 0
            && cloaked != 0
        {
            return None;
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        // Click-through overlays (GPU overlays, etc.) would swallow hover.
        if ex & WS_EX_TRANSPARENT != 0 {
            return None;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
        let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
        if class == "Progman" || class == "WorkerW" {
            return None;
        }
        let mut title = [0u16; 256];
        let n = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
        let title = String::from_utf16_lossy(&title[..n.max(0) as usize]);
        if title.is_empty() && ex & WS_EX_TOOLWINDOW != 0 {
            return None;
        }
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut r as *mut RECT as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        ) != 0
        {
            GetWindowRect(hwnd, &mut r);
        }
        let rect: IRect = r.into();
        if rect.w <= 4 || rect.h <= 4 {
            return None;
        }
        Some(RawWindow { rect, hwnd: hwnd as isize, title })
    }
}

/// Bounds of the foreground window (without the invisible resize border).
pub fn foreground_window_rect() -> Option<IRect> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() || IsIconic(hwnd) != 0 {
            return None;
        }
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut r as *mut RECT as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        ) != 0
        {
            GetWindowRect(hwnd, &mut r);
        }
        let rect = IRect::from(r).intersect(&win::virtual_screen());
        (!rect.is_empty()).then_some(rect)
    }
}

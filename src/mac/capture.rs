//! Screen grabbing (ScreenCaptureKit screenshots) and window discovery on
//! macOS. Every display has its own space there and a window can't span
//! them, so the capture overlay is one window per display: a snapshot each,
//! the one under the pointer first.

use crate::win::{self, IRect, Monitor, ffi};
use image::RgbaImage;

#[derive(Clone, Debug)]
pub struct WindowInfo {
    /// In snapshot image coordinates.
    pub rect: IRect,
    /// `CGWindowID`
    pub hwnd: isize,
    #[allow(dead_code)]
    pub title: String,
    /// Parts of the window taken alone: Windows only for now.
    pub parts: Vec<IRect>,
}

/// A frozen copy of the monitor under the pointer plus the geometry the
/// overlay needs for snapping. All rects are relative to the image (0,0 = `origin`).
pub struct Snapshot {
    pub image: RgbaImage,
    pub origin: (i32, i32),
    pub windows: Vec<WindowInfo>,
    pub monitors: Vec<Monitor>,
    /// The other displays, frozen at the same moment (an overlay window each).
    pub others: Vec<Snapshot>,
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
    let (cx, cy) = win::cursor_pos();
    let under = win::display_at(cx, cy)?;
    // Enumerate before grabbing: cheap, and the z-order matches what we freeze.
    let windows = enum_windows();
    let displays = win::displays();
    // the other displays at the same moment, in parallel
    let others: Vec<_> = displays
        .iter()
        .filter(|d| d.id != under.id)
        .map(|d| {
            let (d, windows) = (*d, windows.clone());
            std::thread::spawn(move || snapshot_of(&d, &windows, false))
        })
        .collect();
    let mut main = snapshot_of(&under, &windows, true)?;
    main.others = others.into_iter().filter_map(|t| t.join().ok().flatten()).collect();
    Some(main)
}

/// One display, frozen, with the windows on it. `notice`: say so when the
/// screen can't be recorded (only on the main thread: it shows a dialog).
fn snapshot_of(d: &win::Display, all: &[(IRect, u32)], notice: bool) -> Option<Snapshot> {
    let vs = d.px;
    let windows = all
        .iter()
        .filter_map(|(rect, id)| {
            let r = rect.intersect(&vs);
            (!r.is_empty()).then(|| WindowInfo { rect: r.offset(-vs.x, -vs.y), hwnd: *id as isize, title: String::new(), parts: Vec::new() })
        })
        .collect();
    let m = d.monitor();
    let monitors = vec![Monitor { rect: m.rect.offset(-vs.x, -vs.y), work: m.work.offset(-vs.x, -vs.y), scale: m.scale }];
    let image = if notice { grab_rect(vs) } else { grab(vs).ok() }?;
    Some(Snapshot { image, origin: (vs.x, vs.y), windows, monitors, others: Vec::new() })
}

/// Grabs an absolute screen rectangle (physical pixels). A rectangle across
/// monitors is cut to the one holding its center.
pub fn grab_rect(r: IRect) -> Option<RgbaImage> {
    match grab(r) {
        Ok(img) => Some(img),
        Err(e) if e.is_empty() => None,
        Err(e) => {
            crate::config::log::error(format!("screenshot: {e}"));
            if win::screen_access(false) {
                // allowed, yet refused: usually a permission given while blinker ran
                win::message_box(
                    crate::i18n::l("снимок не получился", "the screenshot failed"),
                    &lf!(
                        "{e}\n\nесли разрешение на запись экрана дано только что, выйди из blinker через значок в строке меню и запусти снова.",
                        "{e}\n\nif screen recording was allowed just now, quit blinker from its menu bar icon and open it again.",
                    ),
                );
            } else {
                no_permission_notice();
            }
            None
        }
    }
}

/// The pixels of `r` (cut to the monitor holding its center); an empty
/// error for an empty rectangle.
fn grab(r: IRect) -> Result<RgbaImage, String> {
    if r.is_empty() {
        return Err(String::new());
    }
    let d = win::display_at(r.x + r.w / 2, r.y + r.h / 2).ok_or_else(String::new)?;
    let r = r.intersect(&d.px);
    if r.is_empty() {
        return Err(String::new());
    }
    let s = d.scale;
    let (sx, sy) = ((r.x - d.px.x) as f64 / s, (r.y - d.px.y) as f64 / s);
    let mut err = ffi::ErrBuf::new();
    let buf = unsafe { ffi::bk_capture_display(d.id, sx, sy, r.w as f64 / s, r.h as f64 / s, r.w, r.h, 0, err.ptr(), ffi::ErrBuf::LEN) };
    if buf.is_null() {
        let e = err.text();
        return Err(if e.is_empty() { "capture failed".into() } else { e });
    }
    let len = r.w as usize * r.h as usize * 4;
    let data = unsafe { std::slice::from_raw_parts(buf, len) }.to_vec();
    unsafe { ffi::bk_free(buf as *mut _) };
    RgbaImage::from_raw(r.w as u32, r.h as u32, data).ok_or_else(|| "bad image size".into())
}

/// Tells how to allow screen recording when that is what failed.
pub fn no_permission_notice() {
    if win::screen_access(false) {
        return;
    }
    win::message_box(
        crate::i18n::l("нет доступа к экрану", "no access to the screen"),
        crate::i18n::l(
            "разреши blinker запись экрана: системные настройки → конфиденциальность и безопасность → запись экрана и системного звука. потом перезапусти blinker.",
            "allow blinker to record the screen: System Settings → Privacy & Security → Screen & System Audio Recording. then restart blinker.",
        ),
    );
}

/// Ordinary windows on screen, front to back: (bounds, CGWindowID).
fn enum_windows() -> Vec<(IRect, u32)> {
    let mut list = vec![ffi::Win::default(); 512];
    let n = unsafe { ffi::bk_window_list(list.as_mut_ptr(), list.len() as i32) }.max(0) as usize;
    let me = std::process::id() as i32;
    list[..n]
        .iter()
        // normal and floating windows; not the menu bar, the Dock or overlays
        .filter(|w| (0..=8).contains(&w.layer) && w.pid != me && w.alpha > 0.0 && w.w > 4.0 && w.h > 4.0)
        .map(|w| (win::rect_to_px(w.x, w.y, w.w, w.h), w.id))
        .collect()
}

/// Bounds of the front window of the active app.
pub fn foreground_window_rect() -> Option<IRect> {
    let mut w = ffi::Win::default();
    if unsafe { ffi::bk_frontmost_window(&mut w) } == 0 {
        return None;
    }
    let rect = win::rect_to_px(w.x, w.y, w.w, w.h).intersect(&win::virtual_screen());
    (!rect.is_empty()).then_some(rect)
}

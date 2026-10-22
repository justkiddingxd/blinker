//! Custom tray context menu: small layered popups drawn with tiny-skia
//! (Fira text, our line icons), with a row of big tiles for the main actions
//! and submenus. No GPU context, so it opens instantly. Runs a nested modal
//! loop on the shell thread.

use crate::icons::Icon;
use crate::theme;
use crate::win;
use ab_glyph::FontRef;
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use tiny_skia as sk;
pub use crate::sk::{draw_icon, draw_text, paint_of, text_width};
#[allow(unused_imports)]
use crate::sk::{blend, rgba};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub struct Item {
    pub id: u32,
    pub icon: Option<Icon>,
    pub label: String,
    pub hint: String,
    /// Non-empty: this row opens a submenu.
    pub children: Vec<Item>,
    /// Non-empty: this row is a strip of big tiles (icon over a short name).
    pub tiles: Vec<Item>,
}

impl Item {
    pub fn new(id: u32, icon: Icon, label: &str, hint: &str) -> Self {
        Self { id, icon: Some(icon), label: label.into(), hint: hint.into(), children: Vec::new(), tiles: Vec::new() }
    }
    #[allow(dead_code)]
    pub fn sub(icon: Icon, label: &str, children: Vec<Item>) -> Self {
        Self { id: u32::MAX, icon: Some(icon), label: label.into(), hint: String::new(), children, tiles: Vec::new() }
    }
    pub fn tiles(tiles: Vec<Item>) -> Self {
        Self { id: u32::MAX - 1, icon: None, label: String::new(), hint: String::new(), children: Vec::new(), tiles }
    }
    pub fn separator() -> Self {
        Self { id: 0, icon: None, label: String::new(), hint: String::new(), children: Vec::new(), tiles: Vec::new() }
    }
    fn is_sep(&self) -> bool {
        self.id == 0
    }
    fn is_sub(&self) -> bool {
        !self.children.is_empty()
    }
    fn is_tiles(&self) -> bool {
        !self.tiles.is_empty()
    }
}

/// Size of one tile, points.
const TILE_W: f32 = 62.0;
const TILE_H: f32 = 62.0;

/// One popup window (the root menu or an open submenu).
struct Level {
    items: Vec<Item>,
    /// (top, bottom) of each item, in window pixels.
    rows: Vec<(i32, i32)>,
    hover: Option<usize>,
    /// The tile under the pointer, when `hover` is a row of tiles.
    hover_tile: Option<usize>,
    scale: f32,
    pos: (i32, i32),
    size: (i32, i32),
    hwnd: HWND,
    tracking: bool,
}

struct Menu {
    levels: Vec<Level>,
    result: Option<u32>,
    done: bool,
}

thread_local! {
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
}

const CLASS: &str = "BlinkerTrayMenu";

fn register_class() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let name = win::wide(CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(wndproc),
            hInstance: GetModuleHandleW(null()),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: name.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
        std::mem::forget(name);
    });
}

/// The menu is drawn a little larger than the rest of the interface: it is
/// read at a glance, often on a big screen far away.
const ZOOM: f32 = 1.15;

/// Builds a level window. `anchor`: for the root, the tray click point (the
/// menu opens above it); for a submenu, the parent row's (right, top) and
/// (left) edges so it can flip to the other side when there is no room.
fn make_level(items: Vec<Item>, anchor: (i32, i32), sub_of: Option<(i32, i32)>, root: bool) -> Option<Level> {
    let mon = win::monitor_at(&win::monitors(), anchor.0, anchor.1);
    let scale = mon.map(|m| m.scale).unwrap_or(1.0) * ZOOM;
    let work = mon.map(|m| m.work).unwrap_or(win::IRect::new(0, 0, 1920, 1040));
    let px = |v: f32| (v * scale).round() as i32;

    let pad = px(5.0);
    let mut y = pad;
    let mut rows = Vec::new();
    for it in &items {
        let h = if it.is_sep() {
            px(9.0)
        } else if it.is_tiles() {
            px(TILE_H + 4.0)
        } else {
            px(32.0)
        };
        rows.push((y, y + h));
        y += h;
    }
    let tiles_w = items.iter().map(|it| it.tiles.len()).max().unwrap_or(0) as f32 * TILE_W + 10.0;
    let size = (px(if root { 268f32.max(tiles_w) } else { 236.0 }), y + pad);
    let (mut x, mut top) = match sub_of {
        // to the right of the parent row, first row aligned with it
        Some((_parent_left, _)) => (anchor.0 - px(2.0), anchor.1 - pad),
        None => (anchor.0, anchor.1 - size.1),
    };
    if x + size.0 > work.right() {
        x = match sub_of {
            Some((parent_left, _)) => parent_left - size.0 + px(2.0),
            None => anchor.0 - size.0,
        };
    }
    if sub_of.is_none() && top < work.y {
        top = anchor.1;
    }
    x = x.clamp(work.x, work.right() - size.0);
    top = top.clamp(work.y, work.bottom() - size.1);

    let hwnd = unsafe {
        let class = win::wide(CLASS);
        let title = win::wide("blinker");
        // Submenus never take activation: the root keeps focus and closes the
        // whole menu when the user clicks elsewhere.
        let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | if root { 0 } else { WS_EX_NOACTIVATE };
        CreateWindowExW(
            ex,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            x,
            top,
            size.0,
            size.1,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        )
    };
    if hwnd.is_null() {
        return None;
    }
    Some(Level { items, rows, hover: None, hover_tile: None, scale, pos: (x, top), size, hwnd, tracking: false })
}

/// Shows the menu at `anchor` (screen px) and blocks until it closes.
/// Returns the id of the chosen item.
pub fn popup(items: Vec<Item>, anchor: (i32, i32)) -> Option<u32> {
    register_class();
    let root = make_level(items, anchor, None, true)?;
    let hwnd = root.hwnd;
    MENU.with(|m| *m.borrow_mut() = Some(Menu { levels: vec![root], result: None, done: false }));
    render(0);
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
    }

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        let r = unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) };
        if r <= 0 {
            unsafe { PostQuitMessage(msg.wParam as i32) };
            break;
        }
        // Thread messages (hotkeys, app commands) belong to the outer loop.
        if msg.hwnd.is_null() && (msg.message == WM_HOTKEY || msg.message >= WM_APP) {
            unsafe { PostThreadMessageW(GetCurrentThreadId(), msg.message, msg.wParam, msg.lParam) };
            break;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        open_pending_submenu();
        if MENU.with(|m| m.try_borrow().map(|m| m.as_ref().is_none_or(|m| m.done)).unwrap_or(false)) {
            break;
        }
    }
    let menu = MENU.with(|m| m.borrow_mut().take());
    let mut result = None;
    if let Some(m) = menu {
        result = m.result;
        for l in m.levels.iter().rev() {
            unsafe { DestroyWindow(l.hwnd) };
        }
    }
    result
}

fn item_at(l: &Level, y: i32) -> Option<usize> {
    l.rows.iter().position(|(t, b)| y >= *t && y < *b).filter(|&i| !l.items[i].is_sep())
}

/// Left edge of the first tile of a row, window pixels.
fn tiles_left(l: &Level, n: usize) -> f32 {
    (l.size.0 as f32 - n as f32 * TILE_W * l.scale) / 2.0
}

fn tile_at(l: &Level, row: usize, x: i32) -> Option<usize> {
    let n = l.items[row].tiles.len();
    if n == 0 {
        return None;
    }
    let i = ((x as f32 - tiles_left(l, n)) / (TILE_W * l.scale)).floor();
    (i >= 0.0 && (i as usize) < n).then_some(i as usize)
}

/// Drops every submenu deeper than `level` and returns their windows. They
/// must be destroyed only after the menu state is released: DestroyWindow
/// sends messages to the window procedure right away, which re-enters it.
fn close_below(m: &mut Menu, level: usize) -> Vec<HWND> {
    let mut gone = Vec::new();
    while m.levels.len() > level + 1 {
        if let Some(l) = m.levels.pop() {
            gone.push(l.hwnd);
        }
    }
    gone
}

/// Opens the submenu for the hovered row of the deepest level, if it has one
/// and isn't open yet (done outside the window procedure: creating windows
/// there would re-enter it).
fn open_pending_submenu() {
    let request = MENU.with(|cell| {
        let mut guard = cell.try_borrow_mut().ok()?;
        let m = guard.as_mut()?;
        // the deepest level whose hovered row is a submenu that isn't shown
        for li in (0..m.levels.len()).rev() {
            let l = &m.levels[li];
            let Some(h) = l.hover else { continue };
            if !l.items[h].is_sub() {
                continue;
            }
            if m.levels.len() > li + 1 {
                return None; // already open
            }
            let (t, _) = l.rows[h];
            let anchor = (l.pos.0 + l.size.0, l.pos.1 + t);
            let parent_left = l.pos.0;
            let children: Vec<Item> = l.items[h]
                .children
                .iter()
                .map(|c| Item { id: c.id, icon: c.icon, label: c.label.clone(), hint: c.hint.clone(), children: Vec::new(), tiles: Vec::new() })
                .collect();
            return Some((children, anchor, parent_left));
        }
        None
    });
    if let Some((children, anchor, parent_left)) = request
        && let Some(level) = make_level(children, anchor, Some((parent_left, anchor.1)), false)
    {
        let hwnd = level.hwnd;
        let idx = MENU.with(|cell| {
            let mut g = cell.borrow_mut();
            let m = g.as_mut().unwrap();
            m.levels.push(level);
            m.levels.len() - 1
        });
        render(idx);
        unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let x = (lp & 0xFFFF) as i16 as i32;
    let y = ((lp >> 16) & 0xFFFF) as i16 as i32;
    let mut redraw: Vec<usize> = Vec::new();
    let mut destroy: Vec<HWND> = Vec::new();
    let handled = MENU.with(|cell| {
        // Re-entered (a message sent while we hold the state): default handling.
        let Ok(mut guard) = cell.try_borrow_mut() else { return false };
        let Some(m) = guard.as_mut() else { return false };
        let Some(li) = m.levels.iter().position(|l| l.hwnd == hwnd) else { return false };
        match msg {
            WM_MOUSEACTIVATE => true,
            WM_MOUSEMOVE => {
                let l = &mut m.levels[li];
                if !l.tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe { TrackMouseEvent(&mut tme) };
                    l.tracking = true;
                }
                let h = item_at(l, y);
                let tile = h.and_then(|r| tile_at(l, r, x));
                if h != l.hover {
                    l.hover = h;
                    l.hover_tile = tile;
                    redraw.push(li);
                    // moving to another row of a parent closes deeper submenus
                    destroy.extend(close_below(m, li));
                } else if tile != l.hover_tile {
                    l.hover_tile = tile;
                    redraw.push(li);
                }
                true
            }
            WM_MOUSELEAVE => {
                let open_child = m.levels.len() > li + 1;
                let l = &mut m.levels[li];
                l.tracking = false;
                // keep the row of an open submenu highlighted
                if l.hover.is_some() && !open_child {
                    l.hover = None;
                    redraw.push(li);
                }
                true
            }
            WM_LBUTTONUP => {
                let l = &m.levels[li];
                if let Some(i) = item_at(l, y) {
                    if l.items[i].is_tiles() {
                        if let Some(t) = tile_at(l, i, x) {
                            m.result = Some(l.items[i].tiles[t].id);
                            m.done = true;
                        }
                    } else if !l.items[i].is_sub() {
                        m.result = Some(l.items[i].id);
                        m.done = true;
                    }
                }
                true
            }
            WM_RBUTTONUP => true,
            WM_KEYDOWN => {
                // keys act on the deepest open level
                let d = m.levels.len() - 1;
                let n = m.levels[d].items.len();
                let step = |l: &Level, from: Option<usize>, dir: i32| -> Option<usize> {
                    let mut i = from.map(|v| v as i32).unwrap_or(if dir > 0 { -1 } else { n as i32 });
                    for _ in 0..n {
                        i = (i + dir).rem_euclid(n as i32);
                        if !l.items[i as usize].is_sep() && !l.items[i as usize].is_tiles() {
                            return Some(i as usize);
                        }
                    }
                    None
                };
                match wp as u16 {
                    VK_ESCAPE | VK_LEFT if d > 0 => {
                        destroy.extend(close_below(m, d - 1));
                    }
                    VK_ESCAPE => m.done = true,
                    VK_DOWN => {
                        let h = step(&m.levels[d], m.levels[d].hover, 1);
                        m.levels[d].hover = h;
                        redraw.push(d);
                    }
                    VK_UP => {
                        let h = step(&m.levels[d], m.levels[d].hover, -1);
                        m.levels[d].hover = h;
                        redraw.push(d);
                    }
                    VK_RIGHT => {} // opens via open_pending_submenu when the row is a submenu
                    VK_RETURN | VK_SPACE => {
                        if let Some(i) = m.levels[d].hover
                            && !m.levels[d].items[i].is_sub()
                        {
                            m.result = Some(m.levels[d].items[i].id);
                            m.done = true;
                        }
                    }
                    _ => {}
                }
                true
            }
            WM_ACTIVATE => {
                if li == 0 && (wp & 0xFFFF) as u32 == WA_INACTIVE && !cfg!(test) {
                    m.done = true;
                }
                false
            }
            _ => false,
        }
    });
    for h in destroy {
        unsafe { DestroyWindow(h) };
    }
    for li in redraw {
        render(li);
    }
    if msg == WM_MOUSEACTIVATE {
        return MA_NOACTIVATE as LRESULT;
    }
    if handled { 0 } else { unsafe { DefWindowProcW(hwnd, msg, wp, lp) } }
}

// ---------------------------------------------------------------------------
// Drawing

fn render(li: usize) {
    MENU.with(|cell| {
        let Ok(guard) = cell.try_borrow() else { return };
        let Some(m) = guard.as_ref().and_then(|m| m.levels.get(li)) else { return };
        let (w, h) = m.size;
        let Some(mut pm) = sk::Pixmap::new(w as u32, h as u32) else { return };
        let s = m.scale;
        let regular = FontRef::try_from_slice(theme::FONT_REGULAR).ok();
        let mono = FontRef::try_from_slice(theme::FONT_MONO).ok();

        if let Some(bg) = theme::rounded_rect_path(0.5, 0.5, w as f32 - 1.0, h as f32 - 1.0, 9.0 * s) {
            pm.fill_path(&bg, &paint_of(theme::PANEL), sk::FillRule::Winding, sk::Transform::identity(), None);
            let st = sk::Stroke { width: 1.0, ..Default::default() };
            pm.stroke_path(&bg, &paint_of(theme::LINE), &st, sk::Transform::identity(), None);
        }
        let pad = 5.0 * s;
        for (i, it) in m.items.iter().enumerate() {
            let (t, b) = m.rows[i];
            let (t, b) = (t as f32, b as f32);
            let cy = (t + b) / 2.0;
            if it.is_tiles() {
                let n = it.tiles.len();
                let left = tiles_left(m, n);
                for (k, tile) in it.tiles.iter().enumerate() {
                    let x0 = left + k as f32 * TILE_W * s;
                    let hot = m.hover == Some(i) && m.hover_tile == Some(k);
                    if hot && let Some(p) = theme::rounded_rect_path(x0 + 2.0 * s, t + 2.0 * s, TILE_W * s - 4.0 * s, b - t - 4.0 * s, 8.0 * s) {
                        pm.fill_path(&p, &paint_of(theme::HOVER), sk::FillRule::Winding, sk::Transform::identity(), None);
                    }
                    let cx = x0 + TILE_W * s / 2.0;
                    if let Some(icon) = tile.icon {
                        draw_icon(&mut pm, icon, cx, t + 24.0 * s, 19.0 * s, if hot { theme::ACCENT } else { theme::TEXT2 });
                    }
                    if let Some(f) = &regular {
                        let px = 11.5 * s;
                        let tw = text_width(f, px, &tile.label);
                        draw_text(&mut pm, f, px, cx - tw / 2.0, t + 49.0 * s, &tile.label, if hot { theme::TEXT } else { theme::DIM });
                    }
                }
                continue;
            }
            if it.is_sep() {
                if let Some(r) = sk::Rect::from_xywh(pad + 8.0 * s, cy.floor(), w as f32 - 2.0 * pad - 16.0 * s, 1.0) {
                    pm.fill_rect(r, &paint_of(theme::LINE), sk::Transform::identity(), None);
                }
                continue;
            }
            let hovered = m.hover == Some(i);
            if hovered
                && let Some(p) = theme::rounded_rect_path(pad, t, w as f32 - 2.0 * pad, b - t, 6.0 * s)
            {
                pm.fill_path(&p, &paint_of(theme::HOVER), sk::FillRule::Winding, sk::Transform::identity(), None);
            }
            if let Some(icon) = it.icon {
                let col = if hovered { theme::ACCENT } else { theme::DIM };
                draw_icon(&mut pm, icon, pad + 18.0 * s, cy, 15.0 * s, col);
            }
            if let Some(f) = &regular {
                let col = if hovered { theme::TEXT } else { theme::TEXT2 };
                draw_text(&mut pm, f, 13.0 * s, pad + 36.0 * s, cy, &it.label, col);
            }
            if it.is_sub() {
                // chevron ">"
                let x = w as f32 - pad - 14.0 * s;
                let mut pb = sk::PathBuilder::new();
                pb.move_to(x, cy - 4.0 * s);
                pb.line_to(x + 4.0 * s, cy);
                pb.line_to(x, cy + 4.0 * s);
                if let Some(path) = pb.finish() {
                    let st = sk::Stroke { width: 1.4 * s, line_cap: sk::LineCap::Round, ..Default::default() };
                    let col = if hovered { theme::TEXT } else { theme::FAINT };
                    pm.stroke_path(&path, &paint_of(col), &st, sk::Transform::identity(), None);
                }
            } else if !it.hint.is_empty()
                && let Some(f) = &mono
            {
                let px = 11.0 * s;
                let tw = text_width(f, px, &it.hint);
                draw_text(&mut pm, f, px, w as f32 - pad - 10.0 * s - tw, cy, &it.hint, theme::FAINT);
            }
        }
        unsafe { present(m.hwnd, &pm, m.pos) };
    });
}

/// Pushes the premultiplied pixmap into the layered window.
pub unsafe fn present(hwnd: HWND, pm: &sk::Pixmap, pos: (i32, i32)) {
    unsafe {
        let (w, h) = (pm.width() as i32, pm.height() as i32);
        let screen = GetDC(null_mut());
        let mem = CreateCompatibleDC(screen);
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut c_void = null_mut();
        let bmp = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if !bmp.is_null() && !bits.is_null() {
            let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
            for (d, s) in dst.chunks_exact_mut(4).zip(pm.data().chunks_exact(4)) {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
                d[3] = s[3];
            }
            let old = SelectObject(mem, bmp);
            let pt_dst = POINT { x: pos.0, y: pos.1 };
            let size = SIZE { cx: w, cy: h };
            let pt_src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(hwnd, screen, &pt_dst, &size, mem, &pt_src, 0, &blend, ULW_ALPHA);
            SelectObject(mem, old);
            DeleteObject(bmp);
        }
        DeleteDC(mem);
        ReleaseDC(null_mut(), screen);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opens the menu, hovers the submenu row, then another row (closes the
    /// submenu inside the window procedure), then Esc. Messages are posted to
    /// our own popup windows only. `BLINKER_TEST_MENU=1` to run.
    #[test]
    fn submenu_open_close() {
        if std::env::var("BLINKER_TEST_MENU").is_err() {
            return;
        }
        std::thread::spawn(|| unsafe {
            let class = win::wide(CLASS);
            let find = || FindWindowW(class.as_ptr(), null());
            let post = |h: HWND, msg: u32, wp: usize, y: i32| {
                PostMessageW(h, msg, wp, ((y as u32) << 16 | 20) as isize);
            };
            std::thread::sleep(std::time::Duration::from_millis(400));
            let root = find();
            assert!(!root.is_null());
            let row = |i: i32| ((5.0 + 32.0 * i as f32 + 16.0) * ZOOM) as i32; // at 100% scale
            for _ in 0..3 {
                post(root, WM_MOUSEMOVE, 0, row(0)); // "screenshot ›": opens the submenu
                std::thread::sleep(std::time::Duration::from_millis(150));
                post(root, WM_MOUSEMOVE, 0, row(1)); // "recording ›": closes it, opens the other
                std::thread::sleep(std::time::Duration::from_millis(150));
                post(root, WM_MOUSEMOVE, 0, row(3)); // plain row: closes the submenu
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
            post(root, WM_KEYDOWN, VK_ESCAPE as usize, 0);
        });
        let items = vec![
            Item::sub(Icon::Region, "a", vec![Item::new(1, Icon::Region, "a1", ""), Item::new(2, Icon::Screen, "a2", "")]),
            Item::sub(Icon::Record, "b", vec![Item::new(3, Icon::Record, "b1", "")]),
            Item::separator(),
            Item::new(4, Icon::Folder, "c", ""),
        ];
        let r = popup(items, (600, 600));
        assert_eq!(r, None);
    }
}

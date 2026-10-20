//! Click and shortcut effects drawn into the recording (for tutorials).
//! Low-level mouse / keyboard hooks run only while such a recording is on.
//! Only combinations with Ctrl / Alt / Win and special keys are shown, never
//! plain typing, so passwords and messages don't end up in the video.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::IDXGISurface;
use windows::core::{Interface, w};
use windows_sys::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLICK_LIFE: f32 = 0.55;
const KEY_LIFE: f32 = 1.7;

struct Click {
    x: i32,
    y: i32,
    at: Instant,
    right: bool,
}

struct Key {
    label: String,
    at: Instant,
}

#[derive(Default)]
struct Mods {
    ctrl: bool,
    alt: bool,
    shift: bool,
    win: bool,
}

struct State {
    clicks: Vec<Click>,
    keys: Vec<Key>,
    mods: Mods,
}

pub struct Fx {
    state: Mutex<State>,
    clicks: bool,
    keys: bool,
    scale: f32,
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    hook_thread: Mutex<u32>,
}

unsafe impl Send for Fx {}
unsafe impl Sync for Fx {}

static ACTIVE: Mutex<Option<Arc<Fx>>> = Mutex::new(None);

impl Fx {
    /// Starts the hooks. `scale`: monitor DPI scale, for effect sizes.
    pub fn start(clicks: bool, keys: bool, scale: f32) -> Option<Arc<Fx>> {
        if !clicks && !keys {
            return None;
        }
        let d2d: ID2D1Factory = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_MULTI_THREADED, None) }.ok()?;
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }.ok()?;
        let fx = Arc::new(Fx {
            state: Mutex::new(State { clicks: Vec::new(), keys: Vec::new(), mods: Mods::default() }),
            clicks,
            keys,
            scale: scale.max(1.0),
            d2d,
            dwrite,
            hook_thread: Mutex::new(0),
        });
        *ACTIVE.lock().unwrap() = Some(fx.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || unsafe {
            let inst: HINSTANCE = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
            let m = if clicks { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), inst, 0) } else { std::ptr::null_mut() };
            let k = if keys { SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), inst, 0) } else { std::ptr::null_mut() };
            let _ = tx.send(GetCurrentThreadId());
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                DispatchMessageW(&msg);
            }
            if !m.is_null() {
                UnhookWindowsHookEx(m);
            }
            if !k.is_null() {
                UnhookWindowsHookEx(k);
            }
        });
        *fx.hook_thread.lock().unwrap() = rx.recv().unwrap_or(0);
        Some(fx)
    }

    pub fn stop(&self) {
        crate::win::post_thread_message(*self.hook_thread.lock().unwrap(), WM_QUIT);
        let mut a = ACTIVE.lock().unwrap();
        if a.as_ref().is_some_and(|f| std::ptr::eq(f.as_ref(), self)) {
            *a = None;
        }
    }

    /// Some effect is still animating (the pacer then keeps producing frames).
    pub fn active(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        let now = Instant::now();
        s.clicks.retain(|c| now - c.at < Duration::from_secs_f32(CLICK_LIFE));
        s.keys.retain(|k| now - k.at < Duration::from_secs_f32(KEY_LIFE));
        !s.clicks.is_empty() || !s.keys.is_empty()
    }

    /// Draws the live effects onto `tex` (`size` px) whose top-left is at
    /// screen position `origin`.
    pub fn draw(&self, tex: &ID3D11Texture2D, size: (u32, u32), origin: (i32, i32)) {
        let s = self.state.lock().unwrap();
        let now = Instant::now();
        unsafe {
            let Ok(surface) = tex.cast::<IDXGISurface>() else { return };
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let Ok(rt) = self.d2d.CreateDxgiSurfaceRenderTarget(&surface, &props) else { return };
            rt.BeginDraw();
            let brush = |r: f32, g: f32, b: f32, a: f32| rt.CreateSolidColorBrush(&D2D1_COLOR_F { r, g, b, a }, None).ok();
            let k = self.scale;
            for c in &s.clicks {
                let p = ((now - c.at).as_secs_f32() / CLICK_LIFE).clamp(0.0, 1.0);
                let ease = 1.0 - (1.0 - p).powi(3);
                let a = 1.0 - p;
                let center = windows_numerics::Vector2 { X: (c.x - origin.0) as f32, Y: (c.y - origin.1) as f32 };
                let r = (9.0 + 17.0 * ease) * k;
                let e = D2D1_ELLIPSE { point: center, radiusX: r, radiusY: r };
                let (cr, cg, cb) = if c.right { (0.90, 0.28, 0.30) } else { (0.96, 0.95, 0.92) };
                if let Some(fill) = brush(cr, cg, cb, 0.22 * a) {
                    rt.FillEllipse(&e, &fill);
                }
                if let Some(dark) = brush(0.0, 0.0, 0.0, 0.35 * a) {
                    rt.DrawEllipse(&e, &dark, 5.0 * k, None);
                }
                if let Some(line) = brush(cr, cg, cb, 0.95 * a) {
                    rt.DrawEllipse(&e, &line, 2.6 * k, None);
                }
            }
            // shortcut pills, newest on the right, bottom center
            if !s.keys.is_empty()
                && let Ok(fmt) = self.dwrite.CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    17.0 * k,
                    w!("ru-ru"),
                )
            {
                let shown: Vec<&Key> = s.keys.iter().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect();
                let mut items = Vec::new();
                let mut total = 0.0;
                let gap = 8.0 * k;
                for key in &shown {
                    let text: Vec<u16> = key.label.encode_utf16().collect();
                    let Ok(layout) = self.dwrite.CreateTextLayout(&text, &fmt, 2000.0, 200.0) else { continue };
                    let mut m = DWRITE_TEXT_METRICS::default();
                    let _ = layout.GetMetrics(&mut m);
                    let wpx = m.width + 28.0 * k;
                    total += wpx + gap;
                    items.push((key, layout, wpx, m.height));
                }
                total -= gap;
                let h = 40.0 * k;
                let mut x = (size.0 as f32 - total) / 2.0;
                let y = size.1 as f32 - h - 26.0 * k;
                for (key, layout, wpx, th) in items {
                    let age = (now - key.at).as_secs_f32();
                    let a = (((KEY_LIFE - age) / 0.3).clamp(0.0, 1.0)).min((age / 0.08).clamp(0.0, 1.0));
                    let rr = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F { left: x, top: y, right: x + wpx, bottom: y + h },
                        radiusX: 8.0 * k,
                        radiusY: 8.0 * k,
                    };
                    if let Some(bg) = brush(0.07, 0.07, 0.067, 0.88 * a) {
                        rt.FillRoundedRectangle(&rr, &bg);
                    }
                    if let Some(border) = brush(1.0, 1.0, 1.0, 0.14 * a) {
                        rt.DrawRoundedRectangle(&rr, &border, 1.0 * k, None);
                    }
                    if let Some(fg) = brush(0.92, 0.91, 0.88, a) {
                        let o = windows_numerics::Vector2 { X: x + 14.0 * k, Y: y + (h - th) / 2.0 };
                        rt.DrawTextLayout(o, &layout, &fg, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    }
                    x += wpx + gap;
                }
            }
            let _ = rt.EndDraw(None, None);
        }
    }

    fn click(&self, x: i32, y: i32, right: bool) {
        if self.clicks {
            self.state.lock().unwrap().clicks.push(Click { x, y, at: Instant::now(), right });
        }
    }

    fn key(&self, vk: u32, down: bool) {
        let mut s = self.state.lock().unwrap();
        let m = &mut s.mods;
        match vk as u16 {
            VK_CONTROL | VK_LCONTROL | VK_RCONTROL => m.ctrl = down,
            VK_MENU | VK_LMENU | VK_RMENU => m.alt = down,
            VK_SHIFT | VK_LSHIFT | VK_RSHIFT => m.shift = down,
            VK_LWIN | VK_RWIN => m.win = down,
            _ if down && self.keys => {
                let special = key_name(vk).1;
                let combo = m.ctrl || m.alt || m.win;
                if !(combo || special) {
                    return;
                }
                let mut parts: Vec<&str> = Vec::new();
                if m.ctrl {
                    parts.push("ctrl");
                }
                if m.alt {
                    parts.push("alt");
                }
                if m.shift {
                    parts.push("shift");
                }
                if m.win {
                    parts.push("win");
                }
                let name = key_name(vk).0;
                parts.push(&name);
                let label = parts.join(" + ");
                let now = Instant::now();
                // Auto-repeat while held: just keep the pill alive.
                if let Some(last) = s.keys.last_mut()
                    && last.label == label
                    && now - last.at < Duration::from_millis(600)
                {
                    last.at = now;
                    return;
                }
                s.keys.push(Key { label, at: now });
            }
            _ => {}
        }
    }
}

/// (display name, shown even without modifiers)
fn key_name(vk: u32) -> (String, bool) {
    let v = vk as u16;
    let s = |t: &str, special: bool| (t.to_string(), special);
    match v {
        0x30..=0x39 | 0x41..=0x5A => ((v as u8 as char).to_ascii_lowercase().to_string(), false),
        VK_RETURN => s("enter", true),
        VK_ESCAPE => s("esc", true),
        VK_TAB => s("tab", true),
        VK_BACK => s("backspace", true),
        VK_DELETE => s("delete", true),
        VK_SPACE => s("space", false),
        VK_LEFT => s("←", true),
        VK_RIGHT => s("→", true),
        VK_UP => s("↑", true),
        VK_DOWN => s("↓", true),
        VK_HOME => s("home", true),
        VK_END => s("end", true),
        VK_PRIOR => s("pgup", true),
        VK_NEXT => s("pgdn", true),
        VK_INSERT => s("insert", true),
        VK_SNAPSHOT => s("prtsc", true),
        VK_F1..=VK_F24 => (format!("f{}", v - VK_F1 + 1), true),
        _ => {
            // Punctuation etc. from the current layout.
            let ch = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_CHAR) } & 0xFFFF;
            match char::from_u32(ch) {
                Some(c) if !c.is_control() => (c.to_lowercase().to_string(), false),
                _ => (format!("#{vk}"), false),
            }
        }
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 {
        let right = match wp as u32 {
            WM_LBUTTONDOWN | WM_MBUTTONDOWN => Some(false),
            WM_RBUTTONDOWN => Some(true),
            _ => None,
        };
        if let Some(right) = right {
            let info = unsafe { &*(lp as *const MSLLHOOKSTRUCT) };
            if let Some(fx) = ACTIVE.lock().unwrap().clone() {
                fx.click(info.pt.x, info.pt.y, right);
            }
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wp, lp) }
}

unsafe extern "system" fn key_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 {
        let down = matches!(wp as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        let up = matches!(wp as u32, WM_KEYUP | WM_SYSKEYUP);
        if down || up {
            let info = unsafe { &*(lp as *const KBDLLHOOKSTRUCT) };
            if let Some(fx) = ACTIVE.lock().unwrap().clone() {
                fx.key(info.vkCode, down);
            }
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wp, lp) }
}

#[cfg(test)]
mod tests {
    /// Renders sample effects onto a grey frame: `BLINKER_TEST_FX` = png path.
    #[test]
    fn fx_render() {
        let Ok(out) = std::env::var("BLINKER_TEST_FX") else { return };
        let gpu = crate::record::video::Gpu::new().unwrap();
        let fx = super::Fx::start(true, true, 1.0).unwrap();
        fx.click(200, 150, false);
        fx.click(420, 200, true);
        // pretend it happened a bit ago so the ripple has grown
        {
            let mut s = fx.state.lock().unwrap();
            s.clicks[0].at -= std::time::Duration::from_millis(150);
        }
        fx.key(0xA2, true); // LCtrl down
        fx.key(0x43, true); // C
        fx.key(0xA2, false);
        fx.key(0x0D, true); // Enter
        for k in fx.state.lock().unwrap().keys.iter_mut() {
            k.at -= std::time::Duration::from_millis(300);
        }
        let (w, h) = (640u32, 360u32);
        let tex = gpu.texture(w, h, false).unwrap();
        let grey = vec![0x80u8; (w * h * 4) as usize];
        unsafe {
            gpu.context.UpdateSubresource(&tex, 0, None, grey.as_ptr() as _, w * 4, 0);
        }
        fx.draw(&tex, (w, h), (0, 0));
        fx.stop();
        gpu.read_rgba(&tex, w, h).unwrap().save(out).unwrap();
    }
}

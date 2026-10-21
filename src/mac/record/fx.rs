//! Click and shortcut effects drawn into the recording (for tutorials).
//! A listen-only event tap runs only while such a recording is on. Only
//! combinations with ⌃ / ⌥ / ⌘ and special keys are shown, never plain
//! typing, so passwords and messages don't end up in the video. Key presses
//! need the Input Monitoring permission; clicks are shown without it.

use super::video::Frame;
use crate::sk::{draw_text, text_width};
use crate::win::{self, ffi, vk};
use ab_glyph::FontRef;
use eframe::egui::Color32;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tiny_skia as sk;

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

struct State {
    clicks: Vec<Click>,
    keys: Vec<Key>,
}

pub struct Fx {
    state: Mutex<State>,
    clicks: bool,
    keys: bool,
    scale: f32,
    tap: Mutex<*mut c_void>,
}

unsafe impl Send for Fx {}
unsafe impl Sync for Fx {}

static ACTIVE: Mutex<Option<Arc<Fx>>> = Mutex::new(None);

extern "C" fn on_click(x: f64, y: f64, right: i32) {
    if let Some(fx) = ACTIVE.lock().unwrap().clone() {
        let (px, py) = win::pt_to_px(x, y);
        fx.click(px, py, right != 0);
    }
}

extern "C" fn on_key(code: u16, down: i32, flags: u64) {
    if down == 0 {
        return;
    }
    if let Some(fx) = ACTIVE.lock().unwrap().clone() {
        fx.key(code, flags);
    }
}

impl Fx {
    /// Starts listening. `scale`: monitor scale, for effect sizes.
    pub fn start(clicks: bool, keys: bool, scale: f32) -> Option<Arc<Fx>> {
        if !clicks && !keys {
            return None;
        }
        let fx = Arc::new(Fx {
            state: Mutex::new(State { clicks: Vec::new(), keys: Vec::new() }),
            clicks,
            keys,
            scale: scale.max(1.0),
            tap: Mutex::new(std::ptr::null_mut()),
        });
        *ACTIVE.lock().unwrap() = Some(fx.clone());
        let tap = unsafe { ffi::bk_input_start(clicks as i32, keys as i32, on_click, on_key) };
        if tap.is_null() {
            crate::config::log::error("effects: no event tap (input monitoring permission?)");
        }
        *fx.tap.lock().unwrap() = tap;
        Some(fx)
    }

    pub fn stop(&self) {
        let tap = std::mem::replace(&mut *self.tap.lock().unwrap(), std::ptr::null_mut());
        unsafe { ffi::bk_input_stop(tap) };
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

    /// Draws the live effects onto `frame` (`size` px) whose top-left is at
    /// screen position `origin`.
    pub fn draw(&self, frame: &Frame, size: (u32, u32), origin: (i32, i32)) {
        let s = self.state.lock().unwrap();
        let now = Instant::now();
        let k = self.scale;
        let mut layers: Vec<(sk::Pixmap, i32, i32)> = Vec::new();
        for c in &s.clicks {
            let p = ((now - c.at).as_secs_f32() / CLICK_LIFE).clamp(0.0, 1.0);
            let ease = 1.0 - (1.0 - p).powi(3);
            let a = 1.0 - p;
            let r = (9.0 + 17.0 * ease) * k;
            let half = (r + 4.0 * k).ceil() as i32 + 1;
            let Some(mut pm) = sk::Pixmap::new((half * 2) as u32, (half * 2) as u32) else { continue };
            let (cr, cg, cb) = if c.right { (0.90, 0.28, 0.30) } else { (0.96, 0.95, 0.92) };
            let col = |r: f32, g: f32, b: f32, al: f32| Color32::from_rgba_unmultiplied((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, (al.clamp(0.0, 1.0) * 255.0) as u8);
            if let Some(circle) = sk::PathBuilder::from_circle(half as f32, half as f32, r) {
                let id = sk::Transform::identity();
                pm.fill_path(&circle, &crate::sk::paint_of(col(cr, cg, cb, 0.22 * a)), sk::FillRule::Winding, id, None);
                let dark = sk::Stroke { width: 5.0 * k, ..Default::default() };
                pm.stroke_path(&circle, &crate::sk::paint_of(col(0.0, 0.0, 0.0, 0.35 * a)), &dark, id, None);
                let line = sk::Stroke { width: 2.6 * k, ..Default::default() };
                pm.stroke_path(&circle, &crate::sk::paint_of(col(cr, cg, cb, 0.95 * a)), &line, id, None);
            }
            layers.push((pm, c.x - origin.0 - half, c.y - origin.1 - half));
        }
        // shortcut pills, newest on the right, bottom center
        if !s.keys.is_empty()
            && let Ok(font) = FontRef::try_from_slice(crate::theme::FONT_SEMIBOLD)
        {
            let shown: Vec<&Key> = s.keys.iter().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect();
            let px = 17.0 * k;
            let gap = 8.0 * k;
            let h = 40.0 * k;
            let widths: Vec<f32> = shown.iter().map(|key| text_width(&font, px, &key.label) + 28.0 * k).collect();
            let total = widths.iter().sum::<f32>() + gap * (widths.len().saturating_sub(1)) as f32;
            if let Some(mut pm) = sk::Pixmap::new(total.ceil() as u32 + 2, h.ceil() as u32 + 2) {
                let mut x = 1.0;
                for (key, w) in shown.iter().zip(&widths) {
                    let age = (now - key.at).as_secs_f32();
                    let a = (((KEY_LIFE - age) / 0.3).clamp(0.0, 1.0)).min((age / 0.08).clamp(0.0, 1.0));
                    if let Some(rr) = crate::theme::rounded_rect_path(x, 1.0, *w, h, 8.0 * k) {
                        let id = sk::Transform::identity();
                        let bg = Color32::from_rgba_unmultiplied(18, 18, 17, (0.88 * a * 255.0) as u8);
                        pm.fill_path(&rr, &crate::sk::paint_of(bg), sk::FillRule::Winding, id, None);
                        let border = sk::Stroke { width: 1.0 * k, ..Default::default() };
                        pm.stroke_path(&rr, &crate::sk::paint_of(Color32::from_white_alpha((0.14 * a * 255.0) as u8)), &border, id, None);
                    }
                    let fg = Color32::from_rgba_unmultiplied(235, 232, 224, (a * 255.0) as u8);
                    draw_text(&mut pm, &font, px, x + 14.0 * k, 1.0 + h / 2.0, &key.label, fg);
                    x += w + gap;
                }
                let x0 = ((size.0 as f32 - total) / 2.0).round() as i32;
                let y0 = (size.1 as f32 - h - 26.0 * k).round() as i32;
                layers.push((pm, x0, y0));
            }
        }
        drop(s);
        if layers.is_empty() {
            return;
        }
        frame.with_pixels(true, |data, stride, fw, fh| {
            for (pm, x0, y0) in &layers {
                blend_bgra(data, stride, fw, fh, pm, *x0, *y0);
            }
        });
    }

    fn click(&self, x: i32, y: i32, right: bool) {
        if self.clicks {
            self.state.lock().unwrap().clicks.push(Click { x, y, at: Instant::now(), right });
        }
    }

    fn key(&self, code: u16, flags: u64) {
        if !self.keys {
            return;
        }
        let Some(vkey) = vk::from_keycode(code) else { return };
        // modifiers alone are not a shortcut
        let modifier = [vk::VK_SHIFT, vk::VK_CONTROL, vk::VK_MENU, vk::VK_LWIN, vk::VK_RWIN, vk::VK_RSHIFT, vk::VK_RCONTROL, vk::VK_RMENU];
        if modifier.contains(&(vkey as u16)) {
            return;
        }
        let mods = vk::mods_from_flags(flags);
        let (name, special) = key_name(vkey);
        let combo = mods & (vk::MOD_CONTROL | vk::MOD_ALT | vk::MOD_WIN) != 0;
        if !(combo || special) {
            return;
        }
        let mut parts: Vec<&str> = Vec::new();
        if mods & vk::MOD_CONTROL != 0 {
            parts.push("ctrl");
        }
        if mods & vk::MOD_ALT != 0 {
            parts.push("opt");
        }
        if mods & vk::MOD_SHIFT != 0 {
            parts.push("shift");
        }
        if mods & vk::MOD_WIN != 0 {
            parts.push("cmd");
        }
        parts.push(&name);
        let label = parts.join(" + ");
        let now = Instant::now();
        let mut s = self.state.lock().unwrap();
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
}

/// Premultiplied RGBA `pm` over BGRA `dst` at (x0, y0).
fn blend_bgra(dst: &mut [u8], stride: usize, fw: u32, fh: u32, pm: &sk::Pixmap, x0: i32, y0: i32) {
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let src = pm.data();
    for y in 0..ph {
        let dy = y0 + y;
        if dy < 0 || dy >= fh as i32 {
            continue;
        }
        for x in 0..pw {
            let dx = x0 + x;
            if dx < 0 || dx >= fw as i32 {
                continue;
            }
            let s = &src[((y * pw + x) * 4) as usize..][..4];
            let a = s[3] as u32;
            if a == 0 {
                continue;
            }
            let d = &mut dst[dy as usize * stride + dx as usize * 4..][..4];
            let inv = 255 - a;
            d[0] = (s[2] as u32 + d[0] as u32 * inv / 255).min(255) as u8;
            d[1] = (s[1] as u32 + d[1] as u32 * inv / 255).min(255) as u8;
            d[2] = (s[0] as u32 + d[2] as u32 * inv / 255).min(255) as u8;
            d[3] = 255;
        }
    }
}

/// (display name, shown even without modifiers)
fn key_name(vkey: u32) -> (String, bool) {
    use vk::*;
    let v = vkey as u16;
    let s = |t: &str, special: bool| (t.to_string(), special);
    match v {
        0x30..=0x39 | 0x41..=0x5A => ((v as u8 as char).to_ascii_lowercase().to_string(), false),
        VK_RETURN => s("return", true),
        VK_ESCAPE => s("esc", true),
        VK_TAB => s("tab", true),
        VK_BACK => s("delete", true),
        VK_DELETE => s("fn delete", true),
        VK_SPACE => s("space", false),
        VK_LEFT => s("←", true),
        VK_RIGHT => s("→", true),
        VK_UP => s("↑", true),
        VK_DOWN => s("↓", true),
        VK_HOME => s("home", true),
        VK_END => s("end", true),
        VK_PRIOR => s("pgup", true),
        VK_NEXT => s("pgdn", true),
        VK_SNAPSHOT => s("f13", true),
        VK_F1..=VK_F24 => (format!("f{}", v - VK_F1 + 1), true),
        VK_OEM_1 => s(";", false),
        VK_OEM_PLUS => s("=", false),
        VK_OEM_COMMA => s(",", false),
        VK_OEM_MINUS => s("-", false),
        VK_OEM_PERIOD => s(".", false),
        VK_OEM_2 => s("/", false),
        VK_OEM_3 => s("`", false),
        VK_OEM_4 => s("[", false),
        VK_OEM_5 => s("\\", false),
        VK_OEM_6 => s("]", false),
        VK_OEM_7 => s("'", false),
        _ => (format!("#{vkey}"), false),
    }
}

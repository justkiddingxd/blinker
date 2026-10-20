//! On-screen recording chrome: a dashed viewfinder around the recorded area
//! and a small pill with a pulsing dot, the elapsed time, pause and stop.
//! Both are layered windows excluded from capture, drawn with tiny-skia on
//! their own thread (no GPU context while recording).

use super::Clock;
use super::encoder::HNS;
use crate::theme;
use crate::traymenu::{draw_text, paint_of, present};
use crate::win::{self, IRect, Monitor};
use ab_glyph::FontRef;
use eframe::egui::Color32;
use std::cell::RefCell;
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::time::Instant;
use tiny_skia as sk;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "BlinkerRecHud";
const REC: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);

#[derive(Clone, Copy, PartialEq)]
enum Btn {
    Pause,
    Stop,
}

struct State {
    pill: HWND,
    pill_pos: (i32, i32),
    pill_size: (i32, i32),
    /// Button ranges in pill pixels (x0, x1).
    pause_x: (i32, i32),
    stop_x: (i32, i32),
    hover: Option<Btn>,
    tracking: bool,
    shown: Instant,
    scale: f32,
    clock: Arc<Clock>,
    on_stop: Box<dyn Fn()>,
    on_pause: Box<dyn Fn()>,
    /// Window being recorded: the pill rides under it.
    follow: Option<isize>,
    last_paint: Instant,
}

/// Pill position under `area` (centered), above it when the bottom has no
/// room, inside along the bottom edge when neither fits.
fn pill_position(area: IRect, monitor: &Monitor, size: (i32, i32), scale: f32) -> (i32, i32) {
    let px = |v: f32| (v * scale).round() as i32;
    let gap = px(8.0);
    let margin = px(5.0);
    let (mon, work) = (monitor.rect, monitor.work);
    let mut pos = (area.x + area.w / 2 - size.0 / 2, area.bottom() + margin + gap);
    if pos.1 + size.1 > work.bottom() {
        pos.1 = area.y - margin - gap - size.1;
        if pos.1 < mon.y {
            pos.1 = area.bottom().min(work.bottom()) - px(14.0) - size.1;
        }
    }
    pos.0 = pos.0.clamp(mon.x, (mon.right() - size.0).max(mon.x));
    pos
}

impl State {
    fn button_at(&self, x: i32) -> Option<Btn> {
        if x >= self.stop_x.0 {
            Some(Btn::Stop)
        } else if x >= self.pause_x.0 {
            Some(Btn::Pause)
        } else {
            None
        }
    }
}

thread_local! {
    static HUD: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub struct Hud {
    thread: u32,
    join: Option<std::thread::JoinHandle<()>>,
}

fn register_class() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let name = win::wide(CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
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

unsafe fn layered(r: IRect, click_through: bool) -> HWND {
    unsafe {
        let class = win::wide(CLASS);
        let title = win::wide("Blinker");
        let mut ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE;
        if click_through {
            ex |= WS_EX_TRANSPARENT;
        }
        let h = CreateWindowExW(
            ex,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            r.x,
            r.y,
            r.w,
            r.h,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if !h.is_null() {
            // Never part of any screen capture, including our own recording.
            SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE);
        }
        h
    }
}

impl Hud {
    /// `frame`: draw the viewfinder around `area` (not for window recording,
    /// where the window may move).
    pub fn show(
        area: IRect,
        monitor: &Monitor,
        frame: bool,
        follow: Option<isize>,
        clock: Arc<Clock>,
        on_stop: impl Fn() + Send + 'static,
        on_pause: impl Fn() + Send + 'static,
    ) -> Hud {
        let monitor = *monitor;
        let (tx, rx) = std::sync::mpsc::channel();
        let join = std::thread::spawn(move || {
            register_class();
            let s = monitor.scale.max(1.0);
            let px = |v: f32| (v * s).round() as i32;
            let m = px(5.0);
            let frame_r = IRect::new(area.x - m, area.y - m, area.w + 2 * m, area.h + 2 * m);
            let pill_size = (px(150.0), px(30.0));
            let pill_pos = pill_position(area, &monitor, pill_size, s);
            let frame_hwnd = if frame { unsafe { layered(frame_r, true) } } else { null_mut() };
            let pill = unsafe { layered(IRect::new(pill_pos.0, pill_pos.1, pill_size.0, pill_size.1), false) };
            HUD.with(|h| {
                *h.borrow_mut() = Some(State {
                    pill,
                    pill_pos,
                    pill_size,
                    pause_x: (pill_size.0 - px(64.0), pill_size.0 - px(34.0)),
                    stop_x: (pill_size.0 - px(34.0), pill_size.0),
                    hover: None,
                    tracking: false,
                    shown: Instant::now(),
                    scale: s,
                    clock,
                    on_stop: Box::new(on_stop),
                    on_pause: Box::new(on_pause),
                    follow,
                    last_paint: Instant::now(),
                })
            });
            if !frame_hwnd.is_null() {
                paint_frame(frame_hwnd, frame_r, area, s);
            }
            paint_pill();
            unsafe {
                if !frame_hwnd.is_null() {
                    ShowWindow(frame_hwnd, SW_SHOWNOACTIVATE);
                }
                ShowWindow(pill, SW_SHOWNOACTIVATE);
                // following a window: check its position ~30 times a second
                SetTimer(pill, 1, if follow.is_some() { 33 } else { 250 }, None);
            }
            let _ = tx.send(unsafe { GetCurrentThreadId() });
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                DestroyWindow(pill);
                if !frame_hwnd.is_null() {
                    DestroyWindow(frame_hwnd);
                }
            }
            HUD.with(|h| h.borrow_mut().take());
        });
        let thread = rx.recv().unwrap_or(0);
        Hud { thread, join: Some(join) }
    }

    pub fn close(mut self) {
        win::post_thread_message(self.thread, WM_QUIT);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// Dashed hairline + solid corner brackets just outside the recorded area.
fn paint_frame(hwnd: HWND, frame_r: IRect, area: IRect, s: f32) {
    let Some(mut pm) = sk::Pixmap::new(frame_r.w as u32, frame_r.h as u32) else { return };
    let o = (area.x - frame_r.x) as f32;
    let (w, h) = (area.w as f32, area.h as f32);
    let inset = 2.5 * s;
    let (l, t, r, b) = (o - inset, o - inset, o + w + inset, o + h + inset);
    let white = Color32::from_rgba_unmultiplied(240, 238, 232, 235);
    let shadow = Color32::from_black_alpha(110);
    let mut pb = sk::PathBuilder::new();
    pb.move_to(l, t);
    pb.line_to(r, t);
    pb.line_to(r, b);
    pb.line_to(l, b);
    pb.close();
    if let Some(path) = pb.finish() {
        let dash = sk::StrokeDash::new(vec![7.0 * s, 5.0 * s], 0.0);
        for (col, width) in [(shadow, 3.0 * s), (white, 1.4 * s)] {
            let st = sk::Stroke { width, dash: dash.clone(), ..Default::default() };
            pm.stroke_path(&path, &paint_of(col), &st, sk::Transform::identity(), None);
        }
    }
    let len = (20.0 * s).min(w / 3.0).min(h / 3.0);
    let ob = 1.5 * s;
    for (cx, cy, dx, dy) in [(l - ob, t - ob, 1.0, 1.0), (r + ob, t - ob, -1.0, 1.0), (l - ob, b + ob, 1.0, -1.0), (r + ob, b + ob, -1.0, -1.0)] {
        let mut pb = sk::PathBuilder::new();
        pb.move_to(cx, cy + dy * len);
        pb.line_to(cx, cy);
        pb.line_to(cx + dx * len, cy);
        if let Some(path) = pb.finish() {
            for (col, width) in [(shadow, 5.0 * s), (white, 3.0 * s)] {
                let st = sk::Stroke { width, line_cap: sk::LineCap::Square, ..Default::default() };
                pm.stroke_path(&path, &paint_of(col), &st, sk::Transform::identity(), None);
            }
        }
    }
    unsafe { present(hwnd, &pm, (frame_r.x, frame_r.y)) };
}

fn fmt_time(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

fn fill(pm: &mut sk::Pixmap, path: Option<sk::Path>, c: Color32) {
    if let Some(p) = path {
        pm.fill_path(&p, &paint_of(c), sk::FillRule::Winding, sk::Transform::identity(), None);
    }
}

fn paint_pill() {
    HUD.with(|cell| {
        let guard = cell.borrow();
        let Some(st) = guard.as_ref() else { return };
        let (w, h) = st.pill_size;
        let Some(mut pm) = sk::Pixmap::new(w as u32, h as u32) else { return };
        let s = st.scale;
        if let Some(bg) = theme::rounded_rect_path(0.5, 0.5, w as f32 - 1.0, h as f32 - 1.0, h as f32 / 2.0 - 0.5) {
            pm.fill_path(&bg, &paint_of(Color32::from_rgba_unmultiplied(22, 22, 21, 240)), sk::FillRule::Winding, sk::Transform::identity(), None);
            let line = sk::Stroke { width: 1.0, ..Default::default() };
            pm.stroke_path(&bg, &paint_of(theme::LINE), &line, sk::Transform::identity(), None);
        }
        let paused = st.clock.is_paused();
        let cy = h as f32 / 2.0;
        let blink = (st.shown.elapsed().as_millis() % 1000) as f32 / 1000.0;
        // dot: pulsing red while recording, steady grey while paused
        let dot = if paused {
            theme::FAINT
        } else {
            let a = 0.55 + 0.45 * (blink * std::f32::consts::TAU).cos().abs();
            Color32::from_rgba_unmultiplied(REC.r(), REC.g(), REC.b(), (255.0 * a) as u8)
        };
        fill(&mut pm, sk::PathBuilder::from_circle(14.0 * s, cy, 4.5 * s), dot);
        if let Ok(f) = FontRef::try_from_slice(theme::FONT_MONO_MEDIUM) {
            let secs = (st.clock.now().max(0) / HNS) as u64;
            // the time blinks while paused
            let col = if paused && blink > 0.5 { theme::FAINT } else { theme::TEXT };
            draw_text(&mut pm, &f, 12.5 * s, 26.0 * s, cy, &fmt_time(secs), col);
        }
        let x0 = st.pause_x.0 as f32;
        if let Some(sep) = sk::Rect::from_xywh(x0 - 2.0 * s, 7.0 * s, 1.0, h as f32 - 14.0 * s) {
            pm.fill_rect(sep, &paint_of(theme::LINE), sk::Transform::identity(), None);
        }
        for (btn, (a, b)) in [(Btn::Pause, st.pause_x), (Btn::Stop, st.stop_x)] {
            let c = ((a + b) as f32 / 2.0, cy);
            let hovered = st.hover == Some(btn);
            if hovered {
                fill(&mut pm, sk::PathBuilder::from_circle(c.0, c.1, 11.0 * s), theme::HOVER);
            }
            let fg = if hovered && btn == Btn::Stop {
                REC
            } else if hovered {
                theme::ACCENT
            } else {
                theme::TEXT
            };
            match btn {
                Btn::Stop => {
                    let sq = 9.0 * s;
                    fill(&mut pm, theme::rounded_rect_path(c.0 - sq / 2.0, c.1 - sq / 2.0, sq, sq, 1.5 * s), fg);
                }
                Btn::Pause if paused => {
                    // resume: play triangle
                    let mut pb = sk::PathBuilder::new();
                    pb.move_to(c.0 - 3.5 * s, c.1 - 5.5 * s);
                    pb.line_to(c.0 + 5.5 * s, c.1);
                    pb.line_to(c.0 - 3.5 * s, c.1 + 5.5 * s);
                    pb.close();
                    fill(&mut pm, pb.finish(), fg);
                }
                Btn::Pause => {
                    for dx in [-3.0, 3.0] {
                        let x = c.0 + dx * s - 1.5 * s;
                        fill(&mut pm, theme::rounded_rect_path(x, c.1 - 5.0 * s, 3.0 * s, 10.0 * s, 1.0 * s), fg);
                    }
                }
            }
        }
        unsafe { present(st.pill, &pm, st.pill_pos) };
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let is_pill = HUD.with(|c| c.borrow().as_ref().is_some_and(|s| s.pill == hwnd));
    if is_pill {
        match msg {
            WM_TIMER => {
                let (moved, due) = HUD.with(|c| {
                    let mut g = c.borrow_mut();
                    let Some(st) = g.as_mut() else { return (false, false) };
                    let mut moved = false;
                    if let Some(w) = st.follow {
                        let w = w as HWND;
                        // minimized windows sit at -32000: keep the pill where it was
                        if unsafe { IsIconic(w) } == 0 {
                            let b = win::frame_bounds(w);
                            let mons = win::monitors();
                            if let Some(m) = win::monitor_at(&mons, b.x + b.w / 2, b.y + b.h / 2) {
                                let pos = pill_position(b, &m, st.pill_size, st.scale);
                                if pos != st.pill_pos {
                                    st.pill_pos = pos;
                                    moved = true;
                                }
                            }
                        }
                    }
                    let due = st.last_paint.elapsed().as_millis() >= 240;
                    if moved || due {
                        st.last_paint = Instant::now();
                    }
                    (moved, due)
                });
                if due {
                    // Stay above other topmost windows that appeared meanwhile.
                    unsafe { SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
                }
                if moved || due {
                    paint_pill();
                }
                return 0;
            }
            WM_MOUSEMOVE => {
                let x = (lp & 0xFFFF) as i16 as i32;
                let changed = HUD.with(|c| {
                    let mut g = c.borrow_mut();
                    let Some(st) = g.as_mut() else { return false };
                    if !st.tracking {
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        unsafe { TrackMouseEvent(&mut tme) };
                        st.tracking = true;
                    }
                    let h = st.button_at(x);
                    let ch = h != st.hover;
                    st.hover = h;
                    ch
                });
                if changed {
                    paint_pill();
                }
                return 0;
            }
            WM_MOUSELEAVE => {
                HUD.with(|c| {
                    if let Some(st) = c.borrow_mut().as_mut() {
                        st.hover = None;
                        st.tracking = false;
                    }
                });
                paint_pill();
                return 0;
            }
            WM_SETCURSOR => {
                let hover = HUD.with(|c| c.borrow().as_ref().is_some_and(|s| s.hover.is_some()));
                unsafe { SetCursor(LoadCursorW(null_mut(), if hover { IDC_HAND } else { IDC_ARROW })) };
                return 1;
            }
            WM_LBUTTONUP => {
                let x = (lp & 0xFFFF) as i16 as i32;
                HUD.with(|c| {
                    if let Some(st) = c.borrow().as_ref() {
                        match st.button_at(x) {
                            Some(Btn::Stop) => (st.on_stop)(),
                            Some(Btn::Pause) => (st.on_pause)(),
                            None => {}
                        }
                    }
                });
                paint_pill();
                return 0;
            }
            WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
            _ => {}
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

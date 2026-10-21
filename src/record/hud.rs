//! On-screen recording chrome: a dashed viewfinder around the recorded area
//! and a small pill with a pulsing dot, the elapsed time, pause and stop.
//! Both are layered windows excluded from capture, drawn with tiny-skia on
//! their own thread (no GPU context while recording).

pub(crate) use super::pill::*;
use super::Clock;
use super::encoder::HNS;
use crate::traymenu::present;
use crate::win::{self, IRect, Monitor};
use std::cell::RefCell;
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::time::Instant;
use tiny_skia as sk;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "BlinkerRecHud";

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
    /// Last time the pill re-asserted itself above other topmost windows.
    last_top: Instant,
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
            // room for the frame's corner brackets (see `draw_frame`)
        let m = px(7.0);
            let frame_r = IRect::new(area.x - m, area.y - m, area.w + 2 * m, area.h + 2 * m);
            let (pill_size, pause_x, stop_x) = pill_layout(s);
            let pill_pos = pill_position(area, &monitor, pill_size, s);
            let frame_hwnd = if frame { unsafe { layered(frame_r, true) } } else { null_mut() };
            let pill = unsafe { layered(IRect::new(pill_pos.0, pill_pos.1, pill_size.0, pill_size.1), false) };
            HUD.with(|h| {
                *h.borrow_mut() = Some(State {
                    pill,
                    pill_pos,
                    pill_size,
                    pause_x,
                    stop_x,
                    hover: None,
                    tracking: false,
                    shown: Instant::now(),
                    scale: s,
                    clock,
                    on_stop: Box::new(on_stop),
                    on_pause: Box::new(on_pause),
                    follow,
                    last_top: Instant::now(),
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
                // ~30 frames a second for the breathing dot (and a fallback for following)
                SetTimer(pill, 1, 33, None);
            }
            // following a window: Windows tells us the moment it moves
            let hook = match follow {
                Some(h) => unsafe {
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(h as HWND, &mut pid);
                    SetWinEventHook(
                        EVENT_OBJECT_LOCATIONCHANGE,
                        EVENT_OBJECT_LOCATIONCHANGE,
                        null_mut(),
                        Some(on_win_event),
                        pid,
                        0,
                        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                    )
                },
                None => null_mut(),
            };
            let _ = tx.send(unsafe { GetCurrentThreadId() });
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                if !hook.is_null() {
                    UnhookWinEvent(hook);
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

fn paint_frame(hwnd: HWND, frame_r: IRect, area: IRect, s: f32) {
    let Some(mut pm) = sk::Pixmap::new(frame_r.w as u32, frame_r.h as u32) else { return };
    let o = (area.x - frame_r.x) as f32;
    draw_frame(&mut pm, (o, o), (area.w as f32, area.h as f32), s);
    unsafe { present(hwnd, &pm, (frame_r.x, frame_r.y)) };
}

fn paint_pill() {
    HUD.with(|cell| {
        let guard = cell.borrow();
        let Some(st) = guard.as_ref() else { return };
        let look = PillLook {
            secs: (st.clock.now().max(0) / HNS) as u64,
            paused: st.clock.is_paused(),
            clock: st.shown.elapsed().as_secs_f32(),
            hover: st.hover,
        };
        if let Some(pm) = draw_pill(&look, st.scale) {
            unsafe { present(st.pill, &pm, st.pill_pos) };
        }
    });
}

/// Puts the pill under the followed window, moving it without a redraw.
fn follow_now() {
    let target = HUD.with(|c| {
        let mut g = c.borrow_mut();
        let st = g.as_mut()?;
        let w = st.follow? as HWND;
        // minimized windows sit at -32000: keep the pill where it was
        if unsafe { IsIconic(w) } != 0 {
            return None;
        }
        let b = win::frame_bounds(w);
        let m = win::monitor_at(&win::monitors(), b.x + b.w / 2, b.y + b.h / 2)?;
        let pos = pill_position(b, &m, st.pill_size, st.scale);
        if pos == st.pill_pos {
            return None;
        }
        st.pill_pos = pos;
        Some((st.pill, pos))
    });
    // outside the borrow: moving sends messages to `wndproc`
    if let Some((pill, pos)) = target {
        unsafe { SetWindowPos(pill, null_mut(), pos.0, pos.1, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) };
    }
}

/// The followed window moved or resized (WinEvent hook, out of context:
/// delivered through this thread's message loop).
unsafe extern "system" fn on_win_event(_: HWINEVENTHOOK, _event: u32, hwnd: HWND, object: i32, child: i32, _: u32, _: u32) {
    if object != OBJID_WINDOW || child != CHILDID_SELF as i32 {
        return;
    }
    let ours = HUD.with(|c| c.borrow().as_ref().and_then(|s| s.follow).is_some_and(|f| f as HWND == hwnd));
    if ours {
        follow_now();
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let is_pill = HUD.with(|c| c.borrow().as_ref().is_some_and(|s| s.pill == hwnd));
    if is_pill {
        match msg {
            WM_TIMER => {
                follow_now();
                let top = HUD.with(|c| {
                    let mut g = c.borrow_mut();
                    let Some(st) = g.as_mut() else { return false };
                    let due = st.last_top.elapsed().as_millis() >= 240;
                    if due {
                        st.last_top = Instant::now();
                    }
                    due
                });
                if top {
                    // Stay above other topmost windows that appeared meanwhile.
                    unsafe { SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
                }
                paint_pill();
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

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
    /// Where the pill is now.
    pill_pos: (i32, i32),
    /// Where it belongs (`pill_position`); it moves off that place only to
    /// clear a taskbar that slid up over it.
    home: (i32, i32),
    /// Its glide away from `home` (pixels), towards the taskbar's edge.
    offset: (f32, f32),
    monitor: Monitor,
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
                    home: pill_pos,
                    offset: (0.0, 0.0),
                    monitor,
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
            glass: false,
        };
        if let Some(pm) = draw_pill(&look, st.scale) {
            unsafe { present(st.pill, &pm, st.pill_pos) };
        }
    });
}

/// Puts the pill under the followed window, moving it without a redraw.
fn follow_now() {
    HUD.with(|c| {
        let mut g = c.borrow_mut();
        let Some(st) = g.as_mut() else { return };
        let Some(w) = st.follow else { return };
        let w = w as HWND;
        // minimized windows sit at -32000: keep the pill where it was
        if unsafe { IsIconic(w) } != 0 {
            return;
        }
        let b = win::frame_bounds(w);
        if let Some(m) = win::monitor_at(&win::monitors(), b.x + b.w / 2, b.y + b.h / 2) {
            st.home = pill_position(b, &m, st.pill_size, st.scale);
            st.monitor = m;
        }
    });
}

/// How far the pill at `pos` has to move to clear a taskbar over it (one
/// that pops up when it hides itself, say): above a taskbar at the bottom,
/// below one on top, beside one on a side.
fn taskbar_push(pos: (i32, i32), size: (i32, i32), monitor: &Monitor, scale: f32) -> (f32, f32) {
    let gap = (8.0 * scale).round() as i32;
    let pill = IRect::new(pos.0, pos.1, size.0, size.1);
    let mon = monitor.rect;
    for class in ["Shell_TrayWnd", "Shell_SecondaryTrayWnd"] {
        let class = win::wide(class);
        let mut h: HWND = null_mut();
        loop {
            h = unsafe { FindWindowExW(null_mut(), h, class.as_ptr(), null()) };
            if h.is_null() {
                break;
            }
            let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            if unsafe { IsWindowVisible(h) } == 0 || unsafe { GetWindowRect(h, &mut r) } == 0 {
                continue;
            }
            // what is really drawn: tools that make the taskbar float
            // (RoundedTB and the like) cut its window down to a shape
            let mut bar = IRect::from(r);
            let mut shape = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            let kind = unsafe { windows_sys::Win32::Graphics::Gdi::GetWindowRgnBox(h, &mut shape) };
            if kind == windows_sys::Win32::Graphics::Gdi::SIMPLEREGION || kind == windows_sys::Win32::Graphics::Gdi::COMPLEXREGION {
                bar = IRect::new(r.left + shape.left, r.top + shape.top, shape.right - shape.left, shape.bottom - shape.top);
            }
            let bar = bar.intersect(&mon);
            if bar.is_empty() || pill.intersect(&bar).is_empty() {
                continue;
            }
            // a taskbar drawn short (floating, Windows 11 with a mod) still
            // has a window as wide as the screen: what counts is where its
            // buttons, tray and clock are
            let along = bar.w >= bar.h;
            if let Some(c) = taskbar_content(h) {
                let clear = if along { pill.right() + gap <= c.x || pill.x >= c.right() + gap } else { pill.bottom() + gap <= c.y || pill.y >= c.bottom() + gap };
                if clear {
                    continue;
                }
            }
            return if bar.w >= bar.h {
                if bar.y + bar.h / 2 > mon.y + mon.h / 2 {
                    (0.0, (bar.y - gap - pill.bottom()) as f32)
                } else {
                    (0.0, (bar.bottom() + gap - pill.y) as f32)
                }
            } else if bar.x + bar.w / 2 > mon.x + mon.w / 2 {
                ((bar.x - gap - pill.right()) as f32, 0.0)
            } else {
                ((bar.right() + gap - pill.x) as f32, 0.0)
            };
        }
    }
    (0.0, 0.0)
}

thread_local! {
    /// The last look at a taskbar's contents: (its window, when, the span).
    static CONTENT: std::cell::RefCell<Option<(HWND, Instant, Option<IRect>)>> = const { std::cell::RefCell::new(None) };
}

/// Where a taskbar's buttons, tray and clock are (UI Automation, the way
/// screen readers see them), looked at again every couple of seconds. A
/// hidden Windows 11 taskbar shows none (they come with it as it slides
/// up): an empty look is tried again soon.
fn taskbar_content(h: HWND) -> Option<IRect> {
    if let Some((w, at, c)) = CONTENT.with(|c| *c.borrow())
        && w == h
        && at.elapsed().as_millis() < if c.is_some() { 2000 } else { 300 }
    {
        return c;
    }
    let c = read_taskbar(h);
    CONTENT.with(|cell| *cell.borrow_mut() = Some((h, Instant::now(), c)));
    c
}

fn read_taskbar(h: HWND) -> Option<IRect> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation, TreeScope_Descendants};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        let root = uia.ElementFromHandle(windows::Win32::Foundation::HWND(h as _)).ok()?;
        let whole = root.CurrentBoundingRectangle().ok()?;
        let all = root.FindAll(TreeScope_Descendants, &uia.CreateTrueCondition().ok()?).ok()?;
        let (ww, wh) = (whole.right - whole.left, whole.bottom - whole.top);
        let mut span: Option<IRect> = None;
        for i in 0..all.Length().ok()? {
            let Ok(e) = all.GetElement(i) else { continue };
            // containers (panes, groups, toolbars, lists…) aren't content:
            // Windows 11 keeps an empty old tray pane at the right edge
            let kind = e.CurrentControlType().map(|c| c.0).unwrap_or(0);
            if matches!(kind, 50008 | 50021 | 50025 | 50026 | 50032 | 50033 | 50037 | 50040) {
                continue;
            }
            let Ok(r) = e.CurrentBoundingRectangle() else { continue };
            let (w, hh) = (r.right - r.left, r.bottom - r.top);
            // and nothing that spans (nearly) the whole bar
            if w <= 0 || hh <= 0 || (ww >= wh && w * 10 >= ww * 9) || (wh > ww && hh * 10 >= wh * 9) {
                continue;
            }
            let r = IRect::new(r.left, r.top, w, hh);
            span = Some(match span {
                Some(s) => {
                    let (x0, y0) = (s.x.min(r.x), s.y.min(r.y));
                    IRect::new(x0, y0, s.right().max(r.right()) - x0, s.bottom().max(r.bottom()) - y0)
                }
                None => r,
            });
        }
        span
    }
}

/// Puts the pill at its place, gliding off a taskbar that came up over it
/// and back once it's gone.
fn step() {
    let target = HUD.with(|c| {
        let mut g = c.borrow_mut();
        let st = g.as_mut()?;
        let want = taskbar_push(st.home, st.pill_size, &st.monitor, st.scale);
        let ease = |cur: f32, to: f32| if (to - cur).abs() < 0.5 { to } else { cur + (to - cur) * 0.3 };
        st.offset = (ease(st.offset.0, want.0), ease(st.offset.1, want.1));
        let pos = (st.home.0 + st.offset.0.round() as i32, st.home.1 + st.offset.1.round() as i32);
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
        step();
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let is_pill = HUD.with(|c| c.borrow().as_ref().is_some_and(|s| s.pill == hwnd));
    if is_pill {
        match msg {
            WM_TIMER => {
                follow_now();
                step();
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

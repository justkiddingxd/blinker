//! On-screen recording chrome on macOS: a dashed viewfinder around the
//! recorded area and a small pill with a pulsing dot, the elapsed time, pause
//! and stop. Both are non-activating panels left out of the capture; the
//! pictures are the same tiny-skia drawings as on Windows. AppKit runs them
//! on the main thread, driven by a timer.

pub(crate) use super::pill::*;
use super::Clock;
use super::encoder::HNS;
use crate::win::{self, IRect, Monitor, ffi};
use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tiny_skia as sk;

struct State {
    pill: *mut c_void,
    frame: *mut c_void,
    pill_pos: (i32, i32),
    pill_size: (i32, i32),
    pause_x: (i32, i32),
    stop_x: (i32, i32),
    hover: Option<Btn>,
    shown: Instant,
    scale: f32,
    clock: Arc<Clock>,
    on_stop: Box<dyn Fn() + Send>,
    on_pause: Box<dyn Fn() + Send>,
    /// Window being recorded (`CGWindowID`): the pill rides under it.
    follow: Option<u32>,
    last_paint: Instant,
    timer: *mut c_void,
    /// Monitors for placing the pill under a followed window (refreshed now and then).
    monitors: Vec<Monitor>,
    monitors_at: Instant,
}

unsafe impl Send for State {}

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

static HUD: Mutex<Option<State>> = Mutex::new(None);

pub struct Hud;

/// Shows a premultiplied pixmap in a panel at `pos` (px).
fn present(panel: *mut c_void, pm: &sk::Pixmap, pos: (i32, i32), scale: f32) {
    let (x, y) = win::px_to_pt(pos.0, pos.1);
    unsafe { ffi::bk_panel_set_image(panel, pm.data().as_ptr(), pm.width() as i32, pm.height() as i32, x, y, scale as f64) };
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
        let s = monitor.scale.max(1.0);
        let px = |v: f32| (v * s).round() as i32;
        // room for the frame's corner brackets (see `draw_frame`)
        let m = px(7.0);
        let frame_r = IRect::new(area.x - m, area.y - m, area.w + 2 * m, area.h + 2 * m);
        let (pill_size, pause_x, stop_x) = pill_layout(s);
        let pill_pos = pill_position(area, monitor, pill_size, s);
        let frame_panel = if frame { unsafe { ffi::bk_panel_new(1, on_panel, std::ptr::null_mut()) } } else { std::ptr::null_mut() };
        let pill = unsafe { ffi::bk_panel_new(0, on_panel, std::ptr::null_mut()) };
        if crate::theme::glass() {
            // the card's radius (see `draw_pill`), in points
            unsafe { ffi::bk_panel_glass(pill, 12.0) };
        }
        if !frame_panel.is_null()
            && let Some(mut pm) = sk::Pixmap::new(frame_r.w.max(1) as u32, frame_r.h.max(1) as u32)
        {
            let o = (area.x - frame_r.x) as f32;
            draw_frame(&mut pm, (o, o), (area.w as f32, area.h as f32), s);
            present(frame_panel, &pm, (frame_r.x, frame_r.y), s);
            unsafe { ffi::bk_panel_show(frame_panel) };
        }
        *HUD.lock().unwrap() = Some(State {
            pill,
            frame: frame_panel,
            pill_pos,
            pill_size,
            pause_x,
            stop_x,
            hover: None,
            shown: Instant::now(),
            scale: s,
            clock,
            on_stop: Box::new(on_stop),
            on_pause: Box::new(on_pause),
            follow: follow.map(|h| h as u32),
            last_paint: Instant::now(),
            timer: std::ptr::null_mut(),
            monitors: win::monitors(),
            monitors_at: Instant::now(),
        });
        paint_pill();
        unsafe { ffi::bk_panel_show(pill) };
        // following a window: every frame of the display; else ~30 times a second
        let timer = unsafe {
            if follow.is_some() {
                ffi::bk_frame_timer_start(pill, on_timer, std::ptr::null_mut())
            } else {
                ffi::bk_timer_start(1.0 / 30.0, on_timer, std::ptr::null_mut())
            }
        };
        if let Some(st) = HUD.lock().unwrap().as_mut() {
            st.timer = timer;
        }
        Hud
    }

    pub fn close(self) {
        if let Some(st) = HUD.lock().unwrap().take() {
            unsafe {
                ffi::bk_timer_stop(st.timer);
                ffi::bk_panel_close(st.pill);
                ffi::bk_panel_close(st.frame);
            }
        }
    }
}

fn paint_pill() {
    let guard = HUD.lock().unwrap();
    let Some(st) = guard.as_ref() else { return };
    let look = PillLook {
        secs: (st.clock.now().max(0) / HNS) as u64,
        paused: st.clock.is_paused(),
        clock: st.shown.elapsed().as_secs_f32(),
        hover: st.hover,
        glass: crate::theme::glass(),
    };
    if let Some(pm) = draw_pill(&look, st.scale) {
        present(st.pill, &pm, st.pill_pos, st.scale);
    }
}

/// Every frame of the display while following a window (else ~30 times a
/// second): the pill keeps up with the window, and breathes.
extern "C" fn on_timer(_: *mut c_void) {
    let repaint = {
        let mut g = HUD.lock().unwrap();
        let Some(st) = g.as_mut() else { return };
        if let Some(id) = st.follow
            && let Some(b) = win::window_bounds(id)
        {
            if st.monitors_at.elapsed().as_millis() > 500 {
                st.monitors = win::monitors();
                st.monitors_at = Instant::now();
            }
            if let Some(m) = win::monitor_at(&st.monitors, b.x + b.w / 2, b.y + b.h / 2) {
                let pos = pill_position(b, &m, st.pill_size, st.scale);
                if pos != st.pill_pos {
                    // just moved, no redraw: right on this frame
                    st.pill_pos = pos;
                    let (x, y) = win::px_to_pt(pos.0, pos.1);
                    unsafe { ffi::bk_panel_move(st.pill, x, y) };
                }
            }
        }
        let due = st.last_paint.elapsed().as_millis() >= 33;
        if due {
            st.last_paint = Instant::now();
        }
        due
    };
    if repaint {
        paint_pill();
    }
}

/// Mouse over the pill: `x` in points from its left edge.
extern "C" fn on_panel(_: *mut c_void, event: i32, x: f64, _y: f64) {
    let mut action: Option<Btn> = None;
    let changed = {
        let mut g = HUD.lock().unwrap();
        let Some(st) = g.as_mut() else { return };
        let xp = (x * st.scale as f64).round() as i32;
        let before = st.hover;
        match event {
            1 => st.hover = st.button_at(xp),
            2 => st.hover = None,
            3 => action = st.button_at(xp),
            _ => {}
        }
        unsafe { ffi::bk_panel_set_hand(st.pill, st.hover.is_some() as i32) };
        before != st.hover
    };
    if let Some(btn) = action {
        let g = HUD.lock().unwrap();
        if let Some(st) = g.as_ref() {
            match btn {
                Btn::Stop => (st.on_stop)(),
                Btn::Pause => (st.on_pause)(),
            }
        }
    }
    if changed || action.is_some() {
        paint_pill();
    }
}

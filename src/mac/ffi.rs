//! Declarations of the native layer (src/mac/native/*.m). Geometry is in
//! global points with a top-left origin; time in 100 ns host-clock units.

#![allow(dead_code)]

use std::ffi::{c_char, c_int, c_void};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Display {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub vx: f64,
    pub vy: f64,
    pub vw: f64,
    pub vh: f64,
    pub scale: f64,
    pub refresh: f64,
    pub safe_top: f64,
    pub id: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Win {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub id: u32,
    pub pid: i32,
    pub layer: i32,
    pub alpha: f32,
}

#[repr(C)]
pub struct MenuItem {
    pub id: c_int,
    pub depth: c_int,
    pub submenu: c_int,
    pub label: *const c_char,
    pub symbol: *const c_char,
    pub key: *const c_char,
    pub mods: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct StreamCfg {
    pub display: u32,
    pub window: u32,
    pub sx: f64,
    pub sy: f64,
    pub sw: f64,
    pub sh: f64,
    pub width: c_int,
    pub height: c_int,
    pub fps: c_int,
    pub cursor: c_int,
    /// 0 screen, 1 desktop sound, 2 microphone
    pub kind: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VideoSpec {
    pub width: c_int,
    pub height: c_int,
    pub fps: c_int,
    pub bitrate: c_int,
    pub codec: c_int,
    pub quality: c_int,
    pub keyframe_secs: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MediaInfo {
    pub width: c_int,
    pub height: c_int,
    pub fps: f64,
    pub duration: i64,
    pub video_stream: c_int,
    pub audio_streams: [c_int; 16],
    pub audio_count: c_int,
}

pub type TrayCb = extern "C" fn(right: c_int);
pub type MenuCb = extern "C" fn(id: c_int);
pub type HotkeyCb = extern "C" fn(id: u32);
pub type NotifyCb = extern "C" fn(which: c_int);
pub type VideoCb = extern "C" fn(ctx: *mut c_void, pixbuf: *mut c_void, t: i64);
pub type AudioCb = extern "C" fn(ctx: *mut c_void, pcm: *const i16, frames: c_int, t: i64);
pub type PanelCb = extern "C" fn(ctx: *mut c_void, event: c_int, x: f64, y: f64);
pub type TimerCb = extern "C" fn(ctx: *mut c_void);
pub type ClickCb = extern "C" fn(x: f64, y: f64, right: c_int);
pub type KeyCb = extern "C" fn(keycode: u16, down: c_int, flags: u64);
pub type ProgressCb = extern "C" fn(ctx: *mut c_void, fraction: f64);

unsafe extern "C" {
    // app.m
    pub fn bk_is_main_thread() -> c_int;
    pub fn bk_log(line: *const c_char);
    pub fn bk_free(p: *mut c_void);
    pub fn bk_set_activation_policy(regular: c_int);
    pub fn bk_run_idle();
    pub fn bk_on_reopen(cb: NotifyCb);
    pub fn bk_wake_main();
    pub fn bk_trim_memory();
    pub fn bk_displays(out: *mut Display, max: c_int) -> c_int;
    pub fn bk_cursor_pos(x: *mut f64, y: *mut f64);
    pub fn bk_key_down(keycode: u16) -> c_int;
    pub fn bk_install_key_monitor();
    pub fn bk_take_key_press(keycode: *mut u16, flags: *mut u64) -> c_int;
    pub fn bk_window_of_view(view: *mut c_void) -> *mut c_void;
    pub fn bk_window_init(w: *mut c_void);
    pub fn bk_window_find(title: *const c_char) -> *mut c_void;
    pub fn bk_window_frame(w: *mut c_void, x: *mut f64, y: *mut f64, ww: *mut f64, hh: *mut f64);
    pub fn bk_window_set_frame(w: *mut c_void, x: f64, y: f64, ww: f64, hh: f64, topmost: c_int);
    pub fn bk_window_scale(w: *mut c_void) -> f64;
    pub fn bk_window_set_alpha(w: *mut c_void, a: f64);
    pub fn bk_window_activate(w: *mut c_void);
    pub fn bk_window_is_key(w: *mut c_void) -> c_int;
    pub fn bk_window_show(w: *mut c_void, show: c_int);
    pub fn bk_window_style_borderless(w: *mut c_void);
    pub fn bk_window_plain(w: *mut c_void, shadow: c_int);
    pub fn bk_window_make_overlay(w: *mut c_void);
    pub fn bk_focus_take() -> *mut c_void;
    pub fn bk_focus_restore(app: *mut c_void);
    pub fn bk_message_box(title: *const c_char, text: *const c_char);
    pub fn bk_reveal(path: *const c_char);
    pub fn bk_open(path: *const c_char);
    pub fn bk_open_url(url: *const c_char);
    pub fn bk_system_is_russian() -> c_int;
    pub fn bk_notify_listen(name: *const c_char, which: c_int, cb: NotifyCb) -> c_int;
    pub fn bk_notify_post(name: *const c_char);
    pub fn bk_notify_token(name: *const c_char) -> c_int;
    pub fn bk_notify_check(token: c_int) -> c_int;
    pub fn bk_window_buttons_at(w: *mut c_void, x: f64, cy: f64);
    pub fn bk_tray_create(rgba: *const u8, w: c_int, h: c_int, cb: TrayCb);
    pub fn bk_tray_remove();
    pub fn bk_tray_set_image(rgba: *const u8, w: c_int, h: c_int, template: c_int, tip: *const c_char);
    pub fn bk_tray_protect();
    pub fn bk_tray_revive();
    pub fn bk_tray_menu(items: *const MenuItem, n: c_int, cb: MenuCb);
    pub fn bk_hotkeys_init(cb: HotkeyCb);
    pub fn bk_hotkey_register(id: u32, keycode: u32, mods: u32) -> c_int;
    pub fn bk_hotkey_unregister(id: u32);

    // capture.m
    pub fn bk_window_list(out: *mut Win, max: c_int) -> c_int;
    pub fn bk_window_info(id: u32, out: *mut Win) -> c_int;
    pub fn bk_frontmost_window(out: *mut Win) -> c_int;
    pub fn bk_screen_access(request: c_int) -> c_int;
    pub fn bk_pid_is_ours(pid: c_int) -> c_int;
    pub fn bk_capture_display(
        display: u32,
        sx: f64,
        sy: f64,
        sw: f64,
        sh: f64,
        pw: c_int,
        ph: c_int,
        cursor: c_int,
        err: *mut c_char,
        errlen: c_int,
    ) -> *mut u8;

    // record.m
    pub fn bk_host_time_hns() -> i64;
    pub fn bk_cf_retain(p: *const c_void);
    pub fn bk_cf_release(p: *const c_void);
    pub fn bk_pixbuf_new(w: c_int, h: c_int) -> *mut c_void;
    pub fn bk_pixbuf_lock(p: *mut c_void, readonly: c_int, stride: *mut usize, w: *mut c_int, h: *mut c_int) -> *mut u8;
    pub fn bk_pixbuf_unlock(p: *mut c_void, readonly: c_int);
    pub fn bk_pixbuf_copy(p: *mut c_void) -> *mut c_void;
    pub fn bk_mic_available() -> c_int;
    pub fn bk_stream_start(cfg: *const StreamCfg, v: VideoCb, a: AudioCb, ctx: *mut c_void, err: *mut c_char, errlen: c_int) -> *mut c_void;
    pub fn bk_stream_stop(s: *mut c_void);
    pub fn bk_hevc_available() -> c_int;
    pub fn bk_writer_new(path: *const c_char, v: *const VideoSpec, audio_tracks: c_int, realtime: c_int, err: *mut c_char, errlen: c_int)
    -> *mut c_void;
    pub fn bk_writer_video_pixbuf(w: *mut c_void, pb: *mut c_void, t: i64, dur: i64) -> c_int;
    pub fn bk_writer_video_bgra(w: *mut c_void, data: *const u8, t: i64, dur: i64) -> c_int;
    pub fn bk_writer_audio(w: *mut c_void, track: c_int, pcm: *const i16, frames: c_int, t: i64) -> c_int;
    pub fn bk_writer_end_audio(w: *mut c_void);
    pub fn bk_writer_finish(w: *mut c_void, err: *mut c_char, errlen: c_int) -> c_int;
    pub fn bk_writer_free(w: *mut c_void);
    pub fn bk_scan_segments(
        paths: *const *const c_char,
        starts: *const i64,
        n: c_int,
        from: i64,
        to: i64,
        base: *mut i64,
        len: *mut i64,
        err: *mut c_char,
        errlen: c_int,
    ) -> c_int;
    pub fn bk_remux(
        paths: *const *const c_char,
        starts: *const i64,
        n: c_int,
        base: i64,
        end: i64,
        dst: *const c_char,
        pcm: *const *const i16,
        pcm_frames: *const usize,
        tracks: c_int,
        err: *mut c_char,
        errlen: c_int,
    ) -> c_int;

    // reader.m
    pub fn bk_probe(path: *const c_char, info: *mut MediaInfo, err: *mut c_char, errlen: c_int) -> c_int;
    pub fn bk_vreader_open(path: *const c_char, w: c_int, h: c_int, from: i64, err: *mut c_char, errlen: c_int) -> *mut c_void;
    pub fn bk_vreader_next(r: *mut c_void, out: *mut u8, ts: *mut i64) -> c_int;
    pub fn bk_vreader_free(r: *mut c_void);
    pub fn bk_areader_open(path: *const c_char, stream: c_int, from: i64, err: *mut c_char, errlen: c_int) -> *mut c_void;
    pub fn bk_areader_next(r: *mut c_void, pcm: *mut *const i16, frames: *mut c_int, ts: *mut i64) -> c_int;
    pub fn bk_areader_free(r: *mut c_void);

    // hud.m
    pub fn bk_panel_new(click_through: c_int, cb: PanelCb, ctx: *mut c_void) -> *mut c_void;
    pub fn bk_panel_set_image(p: *mut c_void, rgba: *const u8, w: c_int, h: c_int, x: f64, y: f64, scale: f64);
    pub fn bk_panel_set_hand(p: *mut c_void, hand: c_int);
    pub fn bk_panel_move(p: *mut c_void, x: f64, y: f64);
    pub fn bk_frame_timer_start(panel: *mut c_void, cb: TimerCb, ctx: *mut c_void) -> *mut c_void;
    pub fn bk_panel_show(p: *mut c_void);
    pub fn bk_panel_close(p: *mut c_void);
    pub fn bk_timer_start(interval: f64, cb: TimerCb, ctx: *mut c_void) -> *mut c_void;
    pub fn bk_timer_stop(t: *mut c_void);

    // input.m
    pub fn bk_input_start(clicks: c_int, keys: c_int, click: ClickCb, key: KeyCb) -> *mut c_void;
    pub fn bk_input_stop(t: *mut c_void);

    // net.m
    pub fn bk_http_request(
        method: *const c_char,
        url: *const c_char,
        keys: *const *const c_char,
        values: *const *const c_char,
        nheaders: c_int,
        body: *const u8,
        len: usize,
        progress: ProgressCb,
        ctx: *mut c_void,
        status: *mut c_int,
        resp: *mut *mut c_char,
        err_code: *mut c_int,
    ) -> c_int;
}

/// `CString` for a path or text (interior NULs dropped).
pub fn cstr(s: impl AsRef<str>) -> std::ffi::CString {
    std::ffi::CString::new(s.as_ref().replace('\0', "")).unwrap_or_default()
}

pub fn cpath(p: &std::path::Path) -> std::ffi::CString {
    cstr(p.to_string_lossy())
}

/// Error buffer for the native calls.
pub struct ErrBuf(pub [c_char; 512]);

impl ErrBuf {
    pub fn new() -> Self {
        Self([0; 512])
    }
    pub fn ptr(&mut self) -> *mut c_char {
        self.0.as_mut_ptr()
    }
    pub const LEN: c_int = 512;
    pub fn text(&self) -> String {
        unsafe { std::ffi::CStr::from_ptr(self.0.as_ptr()) }.to_string_lossy().into_owned()
    }
}

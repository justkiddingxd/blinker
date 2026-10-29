//! A note next to macOS's "allow screen recording" prompt saying why blinker
//! asks. The prompt belongs to the system, so it is found in the window list:
//! the first new window of another app, about the size of an alert, that
//! appears right after the request. The note follows it and closes with it
//! (or when clicked).

use super::{displays, ffi};
use crate::i18n::l;
use crate::sk::{blend, draw_text, paint_of, text_width};
use crate::theme;
use ab_glyph::FontRef;
use std::collections::HashSet;
use std::ffi::c_void;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tiny_skia as sk;

/// Card width, points.
const WIDTH: f32 = 300.0;
/// The arrow pointing at the prompt.
const ARROW: f32 = 9.0;
/// Between the prompt's edge and the arrow tip.
const GAP: f64 = 6.0;
/// How long to look for the prompt after the request.
const LOOK_FOR: Duration = Duration::from_secs(8);

struct Hint {
    before: HashSet<u32>,
    started: Instant,
    prompt: Option<u32>,
    /// Prompt bounds the note was last placed at (points).
    placed: Option<(f64, f64, f64, f64)>,
    /// (picture, its scale, arrow on the right)
    pic: Option<(sk::Pixmap, f32, bool)>,
    panel: *mut c_void,
    timer: *mut c_void,
}

unsafe impl Send for Hint {}

static HINT: Mutex<Option<Hint>> = Mutex::new(None);

fn on_screen() -> Vec<ffi::Win> {
    let mut list = vec![ffi::Win::default(); 512];
    let n = unsafe { ffi::bk_window_list(list.as_mut_ptr(), list.len() as i32) }.max(0) as usize;
    list.truncate(n);
    list
}

/// Windows on screen now: call right before asking for the permission.
pub fn snapshot() -> HashSet<u32> {
    on_screen().iter().map(|w| w.id).collect()
}

/// Looks for the system prompt that appeared since `before` and puts the
/// note next to it (main thread).
pub fn watch(before: HashSet<u32>) {
    let panel = unsafe { ffi::bk_panel_new(0, on_panel, std::ptr::null_mut()) };
    let timer = unsafe { ffi::bk_timer_start(0.1, on_tick, std::ptr::null_mut()) };
    let old = HINT.lock().unwrap().replace(Hint { before, started: Instant::now(), prompt: None, placed: None, pic: None, panel, timer });
    if let Some(h) = old {
        close(h);
    }
}

fn close(h: Hint) {
    unsafe {
        ffi::bk_timer_stop(h.timer);
        ffi::bk_panel_close(h.panel);
    }
}

fn is_prompt(w: &ffi::Win, before: &HashSet<u32>) -> bool {
    !before.contains(&w.id)
        && w.alpha > 0.0
        && (180.0..720.0).contains(&w.w)
        && (90.0..720.0).contains(&w.h)
        && unsafe { ffi::bk_pid_is_ours(w.pid) } == 0
}

extern "C" fn on_tick(_: *mut c_void) {
    let mut g = HINT.lock().unwrap();
    let Some(h) = g.as_mut() else { return };
    let wins = on_screen();
    let prompt = match h.prompt {
        Some(id) => wins.iter().find(|w| w.id == id).copied(),
        None => {
            let found = wins.iter().find(|w| is_prompt(w, &h.before)).copied();
            if found.is_none() && h.started.elapsed() > LOOK_FOR {
                // no prompt: asked before, or the permission is already there
                if let Some(h) = g.take() {
                    close(h);
                }
                return;
            }
            found
        }
    };
    match prompt {
        Some(w) => {
            h.prompt = Some(w.id);
            place(h, &w);
        }
        // the prompt was answered
        None if h.prompt.is_some() => {
            if let Some(h) = g.take() {
                close(h);
            }
        }
        None => {}
    }
}

/// A click on the note dismisses it.
extern "C" fn on_panel(_: *mut c_void, event: i32, _x: f64, _y: f64) {
    if event == 3
        && let Some(h) = HINT.lock().unwrap().take()
    {
        close(h);
    }
}

/// Next to the prompt: on its right, or on its left when the screen ends.
fn place(h: &mut Hint, w: &ffi::Win) {
    let bounds = (w.x, w.y, w.w, w.h);
    if h.placed == Some(bounds) {
        return;
    }
    let (cx, cy) = (w.x + w.w / 2.0, w.y + w.h / 2.0);
    let list = displays();
    let d = list
        .iter()
        .find(|d| cx >= d.pt.0 && cy >= d.pt.1 && cx < d.pt.0 + d.pt.2 && cy < d.pt.1 + d.pt.3)
        .or(list.first())
        .copied();
    let (scale, right_edge) = d.map(|d| (d.scale as f32, d.pt.0 + d.pt.2)).unwrap_or((2.0, f64::MAX));
    let total = (WIDTH + ARROW) as f64;
    let on_left = w.x + w.w + GAP + total > right_edge - 8.0;
    if h.pic.as_ref().is_none_or(|(_, s, left)| *s != scale || *left != on_left) {
        h.pic = render(scale, on_left).map(|pm| (pm, scale, on_left));
    }
    let Some((pm, _, _)) = &h.pic else { return };
    let x = if on_left { w.x - GAP - total } else { w.x + w.w + GAP };
    unsafe {
        ffi::bk_panel_set_image(h.panel, pm.data().as_ptr(), pm.width() as i32, pm.height() as i32, x, w.y, scale as f64);
        if h.placed.is_none() {
            ffi::bk_panel_show(h.panel);
        }
    }
    h.placed = Some(bounds);
}

/// Splits `text` into lines no wider than `max` px.
fn wrap(font: &FontRef, px: f32, text: &str, max: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if !line.is_empty() && text_width(font, px, &candidate) > max {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The note at scale `s`; the arrow points left (at a prompt on the left), or
/// right when `on_left`.
fn render(s: f32, on_left: bool) -> Option<sk::Pixmap> {
    let title_font = FontRef::try_from_slice(theme::FONT_SEMIBOLD).ok()?;
    let body_font = FontRef::try_from_slice(theme::FONT_REGULAR).ok()?;
    let px = |v: f32| v * s;
    let pad = px(16.0);
    let card_w = px(WIDTH);
    let mark = px(15.0);
    let (title_px, body_px) = (px(14.0), px(12.5));
    let (title_lh, body_lh, para_gap) = (px(19.0), px(17.5), px(8.0));
    let title = l("зачем blinker просит доступ к экрану", "why blinker asks for the screen");
    let paras = [
        l(
            "в macOS одно разрешение и на скриншоты, и на запись видео. без него blinker видит только обои, без окон.",
            "macOS has one permission for both screenshots and screen recording. without it blinker sees only the wallpaper, no windows.",
        ),
        l(
            "снимки остаются на этом маке: blinker никуда их не отправляет, пока ты сам не нажмёшь «загрузить».",
            "captures stay on this Mac: blinker sends nothing anywhere unless you press upload.",
        ),
        l(
            "открой системные настройки из этого окна и включи blinker. потом выйди из него через значок в строке меню и запусти снова.",
            "open System Settings from this window and turn blinker on. then quit it from its menu bar icon and open it again.",
        ),
    ];
    let title_lines = wrap(&title_font, title_px, title, card_w - 2.0 * pad - mark - px(8.0));
    let body: Vec<Vec<String>> = paras.iter().map(|p| wrap(&body_font, body_px, p, card_w - 2.0 * pad)).collect();
    let body_h: f32 = body.iter().map(|p| p.len() as f32 * body_lh).sum::<f32>() + para_gap * (body.len() - 1) as f32;
    let card_h = pad + title_lines.len() as f32 * title_lh + px(10.0) + body_h + pad - px(4.0);
    let arrow = px(ARROW);
    let mut pm = sk::Pixmap::new((card_w + arrow).ceil() as u32, card_h.ceil() as u32)?;
    let x0 = if on_left { 0.0 } else { arrow };
    let id = sk::Transform::identity();

    // card with an arrow on the prompt's side, one outline around both
    let r = px(10.0);
    let tip_y = pad + title_lh / 2.0;
    let card = theme::rounded_rect_path(x0 + 0.5, 0.5, card_w - 1.0, card_h - 1.0, r)?;
    let mut pb = sk::PathBuilder::new();
    if on_left {
        let e = x0 + card_w - 0.5;
        pb.move_to(e - 1.0, tip_y - arrow);
        pb.line_to(e + arrow - 0.5, tip_y);
        pb.line_to(e - 1.0, tip_y + arrow);
    } else {
        let e = x0 + 0.5;
        pb.move_to(e + 1.0, tip_y - arrow);
        pb.line_to(0.5, tip_y);
        pb.line_to(e + 1.0, tip_y + arrow);
    }
    pb.close();
    let tip = pb.finish()?;
    let line = sk::Stroke { width: px(1.0), ..Default::default() };
    pm.stroke_path(&card, &paint_of(theme::LINE()), &line, id, None);
    pm.stroke_path(&tip, &paint_of(theme::LINE()), &line, id, None);
    pm.fill_path(&card, &paint_of(theme::PANEL()), sk::FillRule::Winding, id, None);
    pm.fill_path(&tip, &paint_of(theme::PANEL()), sk::FillRule::Winding, id, None);

    // the sparkle, then the title next to it
    let mark_img = theme::mark_image(mark.round() as u32);
    let (mx, my) = ((x0 + pad).round() as i32, (pad + title_lh / 2.0 - mark / 2.0).round() as i32);
    for (x, y, p) in mark_img.enumerate_pixels() {
        if p.0[3] > 0 {
            blend(&mut pm, mx + x as i32, my + y as i32, theme::ACCENT(), p.0[3] as f32 / 255.0);
        }
    }
    let tx = x0 + pad + mark + px(8.0);
    let mut y = pad;
    for t in &title_lines {
        draw_text(&mut pm, &title_font, title_px, tx, y + title_lh / 2.0, t, theme::TEXT());
        y += title_lh;
    }
    y += px(10.0);
    for (i, p) in body.iter().enumerate() {
        if i > 0 {
            y += para_gap;
        }
        for t in p {
            draw_text(&mut pm, &body_font, body_px, x0 + pad, y + body_lh / 2.0, t, theme::DIM());
            y += body_lh;
        }
    }
    Some(pm)
}

#[cfg(test)]
mod tests {
    /// Renders the note (both sides, Russian and English): `BLINKER_TEST_HINT` = folder.
    #[test]
    fn hint_render() {
        let Ok(dir) = std::env::var("BLINKER_TEST_HINT") else { return };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for lang in ["ru", "en"] {
            crate::i18n::apply(lang);
            for left in [false, true] {
                let pm = super::render(2.0, left).unwrap();
                crate::theme::pixmap_to_image(&pm).save(dir.join(format!("hint-{lang}-{}.png", if left { "left" } else { "right" }))).unwrap();
            }
        }
    }
}

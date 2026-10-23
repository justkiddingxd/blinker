//! The editor's working parts around the preview: the tool strip, the
//! inspector (the frame, texts, sound), the text and subtitle lanes of the
//! timeline with its zoom, and the texts drawn over the preview as they will
//! be burnt in (with their animations).

use super::shorts::{self, Anim, Aspect, CapBack, CapFont, CaptionStyle, Music, Sticker, Title};
use super::*;

/// The inspector's pages.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Tab {
    Video,
    Text,
    Sound,
}

/// What to bring in.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Import {
    Music,
    Picture,
}

/// What's picked on the timeline (its inspector page opens).
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Sel {
    None,
    Caption(usize),
    Title(usize),
    Sticker(usize),
}

/// The two lanes of texts.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Lane {
    Caption,
    Title,
    Image,
}

pub(super) const INSPECTOR_W: f32 = 300.0;
/// A new text's length.
const TITLE_LEN: i64 = 3 * HNS;

impl VideoApp {
    // ------------------------------------------------------------ the view

    /// The visible part of the timeline (source time).
    pub(super) fn view_span(&self) -> i64 {
        (self.view.1 - self.view.0).max(1)
    }

    /// Zoom the timeline by `k` (<1 closer) about time `at`.
    pub(super) fn zoom_view(&mut self, k: f32, at: i64) {
        let dur = self.dur();
        let span = ((self.view_span() as f64 * k as f64) as i64).clamp((HNS / 2).min(dur), dur);
        let f = (at - self.view.0) as f64 / self.view_span() as f64;
        let mut a = at - (span as f64 * f) as i64;
        a = a.clamp(0, dur - span);
        self.view = (a, a + span);
    }

    /// Keeps the playhead in view while it plays.
    pub(super) fn follow(&mut self) {
        let span = self.view_span();
        if span >= self.dur() {
            return;
        }
        if self.pos > self.view.1 - span / 20 || self.pos < self.view.0 {
            let a = (self.pos - span / 10).clamp(0, self.dur() - span);
            self.view = (a, a + span);
        }
    }

    /// Wheel over the timeline: Ctrl (pinch) zooms about the pointer, a plain
    /// wheel scrolls along when zoomed in.
    pub(super) fn view_input(&mut self, ui: &egui::Ui, timeline: Rect, area: Rect) {
        let Some(pt) = ui.input(|i| i.pointer.hover_pos()).filter(|p| timeline.contains(*p)) else { return };
        let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
        if (zoom - 1.0).abs() > 0.001 {
            let at = self.t_of(pt.x.clamp(area.left(), area.right()), area);
            self.zoom_view(1.0 / zoom, at);
        } else if self.view_span() < self.dur() {
            let d = if scroll.x.abs() > scroll.y.abs() { scroll.x } else { scroll.y };
            if d != 0.0 {
                let dt = (-(d as f64) / area.width().max(1.0) as f64 * self.view_span() as f64) as i64;
                let a = (self.view.0 + dt).clamp(0, self.dur() - self.view_span());
                self.view = (a, a + self.view_span());
            }
        }
    }

    /// Times along the top of the timeline.
    pub(super) fn ruler(&mut self, ui: &mut egui::Ui, row: Rect, area: Rect) {
        let p = ui.painter().clone();
        // a step between ticks that leaves room for the labels
        let per_px = self.view_span() as f64 / area.width().max(1.0) as f64;
        let steps = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0];
        let step = steps.iter().map(|s| (s * HNS as f64) as i64).find(|s| *s as f64 / per_px >= 74.0).unwrap_or(600 * HNS);
        let mut t = (self.view.0 / step) * step;
        while t <= self.view.1 {
            if t >= self.view.0 {
                let x = self.x_of(t, area);
                p.vline(x, (row.bottom() - 4.0)..=row.bottom(), Stroke::new(1.0, FAINT));
                p.text(pos2(x + 3.0, row.center().y - 1.0), Align2::LEFT_CENTER, fmt_time(t, step < HNS), mono(10.0), FAINT);
            }
            t += step;
        }
    }

    /// The timeline's zoom: closer, farther, all of it (right to left).
    pub(super) fn zoom_controls(&mut self, ui: &mut egui::Ui) {
        let dur = self.dur();
        if self.view_span() < dur && ui::button_h(ui, crate::i18n::l("весь клип", "whole clip"), ui::Kind::Ghost, 28.0).clicked() {
            self.view = (0, dur);
        }
        if ui::icon_button(ui, Icon::Plus, crate::i18n::l("приблизить  ctrl+колесо", "zoom in  ctrl+wheel"), 28.0, false, ui::Kind::Ghost).clicked() {
            self.zoom_view(0.5, self.pos);
        }
        if ui::button_h(ui, "−", ui::Kind::Ghost, 28.0).on_hover_text(crate::i18n::l("отдалить", "zoom out")).clicked() {
            self.zoom_view(2.0, self.pos);
        }
        sep(ui);
    }

    // ------------------------------------------------------------ texts

    pub(super) fn spans(&self, lane: Lane) -> Vec<(i64, i64, String)> {
        match lane {
            Lane::Caption => self.captions.iter().map(|c| (c.from, c.to, c.text.clone())).collect(),
            Lane::Title => self.titles.iter().map(|t| (t.from, t.to, t.text.clone())).collect(),
            Lane::Image => self.stickers.iter().map(|s| (s.from, s.to, s.path.file_name().unwrap_or_default().to_string_lossy().into_owned())).collect(),
        }
    }

    fn set_span(&mut self, lane: Lane, i: usize, from: i64, to: i64) {
        match lane {
            Lane::Caption => {
                self.captions[i].from = from;
                self.captions[i].to = to;
            }
            Lane::Title => {
                self.titles[i].from = from;
                self.titles[i].to = to;
            }
            Lane::Image => {
                self.stickers[i].from = from;
                self.stickers[i].to = to;
            }
        }
    }

    fn sel_of(lane: Lane, i: usize) -> Sel {
        match lane {
            Lane::Caption => Sel::Caption(i),
            Lane::Title => Sel::Title(i),
            Lane::Image => Sel::Sticker(i),
        }
    }

    pub(super) fn select(&mut self, sel: Sel) {
        self.sel = sel;
        self.text_focus = !matches!(sel, Sel::None);
        match sel {
            Sel::None => {}
            Sel::Sticker(_) => self.tab = Tab::Video,
            _ => self.tab = Tab::Text,
        }
    }

    /// A text over the video at the playhead.
    pub(super) fn add_title(&mut self) {
        let from = self.pos.clamp(0, (self.dur() - HNS / 5).max(0));
        let to = (from + TITLE_LEN).min(self.dur()).max(from + HNS / 5);
        self.snapshot();
        self.titles.push(Title::new(from, to));
        self.select(Sel::Title(self.titles.len() - 1));
    }

    /// Deletes what's picked (a text or a subtitle).
    pub(super) fn delete_selected(&mut self) -> bool {
        match self.sel {
            Sel::Caption(i) if i < self.captions.len() => {
                self.snapshot();
                self.captions.remove(i);
            }
            Sel::Title(i) if i < self.titles.len() => {
                self.snapshot();
                self.titles.remove(i);
            }
            Sel::Sticker(i) if i < self.stickers.len() => {
                self.snapshot();
                self.stickers.remove(i);
            }
            _ => return false,
        }
        self.sel = Sel::None;
        true
    }

    /// A lane of text blocks: moved, stretched by their edges, picked by a
    /// click (the inspector edits them).
    pub(super) fn blocks_lane(&mut self, ui: &mut egui::Ui, lane: Rect, label: Rect, area: Rect, which: Lane) {
        let p = ui.painter().clone();
        let add = ui.interact(label, ui.scope_id().with(("lane-add", which as u8)), Sense::click());
        let h = add.hovered();
        let (icon, name, tip) = match which {
            Lane::Image => (Icon::Frame, crate::i18n::l("картинка", "picture"), crate::i18n::l("картинка поверх видео: стикер, логотип, мем", "a picture over the video: a sticker, a logo, a meme")),
            Lane::Title => (Icon::Text, crate::i18n::l("текст", "text"), crate::i18n::l("текст поверх видео на месте курсора", "a text over the video at the playhead")),
            Lane::Caption => (Icon::TextBox, crate::i18n::l("субтитры · t", "subtitles · t"), crate::i18n::l("субтитр на месте курсора", "a subtitle at the playhead")),
        };
        icons::paint(&p, Rect::from_center_size(pos2(label.left() + 15.0, label.center().y), Vec2::splat(14.0)), icon, 13.0, if h { ACCENT } else { FAINT });
        p.text(pos2(label.left() + 38.0, label.center().y), Align2::LEFT_CENTER, format!("+ {name}"), font(12.0), if h { TEXT } else { FAINT });
        if add.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip).clicked() {
            match which {
                Lane::Title => self.add_title(),
                Lane::Caption => self.add_caption(),
                Lane::Image => self.import(ui.ctx(), Some(Import::Picture)),
            }
        }
        p.rect_filled(lane, 3, CANVAS);
        let clip = p.with_clip_rect(lane);
        let dur = self.dur();
        let spans = self.spans(which);
        let mut clicked = None;
        for (i, (from, to, text)) in spans.iter().enumerate() {
            let (xa, xb) = (self.x_of(*from, area), self.x_of(*to, area).max(self.x_of(*from, area) + 6.0));
            if xb < lane.left() || xa > lane.right() {
                continue;
            }
            let block = Rect::from_min_max(pos2(xa, lane.top() + 2.0), pos2(xb, lane.bottom() - 2.0));
            let resp = ui.interact(block.expand2(vec2(3.0, 0.0)).intersect(lane), ui.scope_id().with(("block", which as u8, i)), Sense::click_and_drag());
            let edge = |x: f32| {
                if x <= block.left() + 5.0 {
                    CapDrag::Start
                } else if x >= block.right() - 5.0 {
                    CapDrag::End
                } else {
                    CapDrag::Move
                }
            };
            if resp.drag_started()
                && let Some(o) = ui.input(|i| i.pointer.press_origin())
            {
                self.snapshot();
                self.block_drag = Some((which, i, edge(o.x), self.t_of(o.x, area) - from));
            }
            if let Some(pt) = resp.hover_pos() {
                ui.ctx().set_cursor_icon(if edge(pt.x) == CapDrag::Move { CursorIcon::Grab } else { CursorIcon::ResizeHorizontal });
            }
            if let Some((l, j, what, off)) = self.block_drag
                && l == which
                && j == i
                && let Some(pt) = resp.interact_pointer_pos()
            {
                let t = self.t_of(pt.x, area);
                let min = HNS / 5;
                let (a, b) = match what {
                    CapDrag::Move => {
                        let len = to - from;
                        let a = (t - off).clamp(0, (dur - len).max(0));
                        (a, a + len)
                    }
                    CapDrag::Start => (t.min(to - min).max(0), *to),
                    CapDrag::End => (*from, t.max(from + min).min(dur)),
                };
                self.set_span(which, i, a, b);
            }
            if resp.drag_stopped() {
                self.block_drag = None;
            }
            let picked = self.sel == Self::sel_of(which, i);
            let hot = picked || resp.hovered();
            let base = match which {
                Lane::Title => Color32::from_rgb(0x8F, 0xB4, 0xE8),
                Lane::Image => Color32::from_rgb(0xC9, 0x9B, 0xE6),
                Lane::Caption => ACCENT,
            };
            let fill = if hot { base } else { theme::lerp_color(DIM, base, 0.5) };
            clip.rect_filled(block, 3, fill);
            if picked {
                clip.rect_stroke(block, 3, Stroke::new(1.5, TEXT), StrokeKind::Inside);
            }
            let words = if text.trim().is_empty() { crate::i18n::l("без текста", "no text") } else { text.lines().next().unwrap_or("") };
            p.with_clip_rect(block.shrink2(vec2(5.0, 0.0)).intersect(lane)).text(pos2(block.left() + 6.0, block.center().y), Align2::LEFT_CENTER, words, font_medium(11.0), ON_ACCENT);
            if resp.on_hover_text(format!("{} – {} · {}", fmt_time(*from, true), fmt_time(*to, true), words)).clicked() {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked {
            self.select(Self::sel_of(which, i));
            self.seek(spans[i].0);
        }
        let x = self.x_of(self.pos, area).round() + 0.5;
        if lane.x_range().contains(x) {
            p.vline(x, lane.y_range(), Stroke::new(2.0, TEXT));
        }
    }

    // ------------------------------------------------------------ speed

    /// How fast the clip plays at `t`.
    pub(super) fn speed_at(&self, t: i64) -> f32 {
        self.speeds.iter().find(|(a, b, _)| t >= *a && t < *b).map_or(1.0, |s| s.2)
    }

    /// `a..b` played at `speed` (1: as recorded).
    pub(super) fn set_speed(&mut self, a: i64, b: i64, speed: f32) {
        self.snapshot();
        // what overlapped it keeps only its parts outside
        let mut rest = Vec::new();
        for &(x, y, s) in &self.speeds {
            if y <= a || x >= b {
                rest.push((x, y, s));
            } else {
                if x < a {
                    rest.push((x, a, s));
                }
                if y > b {
                    rest.push((b, y, s));
                }
            }
        }
        if (speed - 1.0).abs() > 0.01 {
            rest.push((a, b, speed));
        }
        rest.sort_by_key(|r| r.0);
        self.speeds = rest;
    }

    /// The kept pieces cut where the speed changes: (from, to, speed).
    pub(super) fn kept_speed(&self) -> Vec<(i64, i64, f32)> {
        let mut out = Vec::new();
        for (a, b) in self.kept() {
            let mut cuts: Vec<i64> = self.speeds.iter().flat_map(|(x, y, _)| [*x, *y]).filter(|t| *t > a && *t < b).collect();
            cuts.push(b);
            cuts.sort();
            cuts.dedup();
            let mut from = a;
            for to in cuts {
                if to - from > HNS / 100 {
                    out.push((from, to, self.speed_at(from)));
                }
                from = to;
            }
        }
        out
    }

    /// Where source time `t` falls in the saved clip (cuts and speeds taken in).
    pub(super) fn out_time(&self, t: i64) -> i64 {
        let mut base = 0i64;
        for (a, b, s) in self.kept_speed() {
            if t < a {
                break;
            }
            let end = t.min(b);
            base += ((end - a) as f64 / s as f64) as i64;
            if t < b {
                break;
            }
        }
        base
    }

    // ------------------------------------------------------------ import

    /// Music or a picture from a file (`what`: only that kind offered).
    pub(super) fn import(&mut self, ctx: &egui::Context, what: Option<Import>) {
        const AUDIO: [&str; 6] = ["mp3", "wav", "m4a", "aac", "flac", "wma"];
        const PICTURES: [&str; 5] = ["png", "jpg", "jpeg", "webp", "bmp"];
        let mut dialog = rfd::FileDialog::new();
        if what != Some(Import::Picture) {
            dialog = dialog.add_filter(crate::i18n::l("музыка", "music"), &AUDIO);
        }
        if what != Some(Import::Music) {
            dialog = dialog.add_filter(crate::i18n::l("картинки", "pictures"), &PICTURES);
        }
        let Some(path) = dialog.pick_file() else { return };
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if AUDIO.contains(&ext.as_str()) {
            match crate::media::probe(&path) {
                Ok(info) if !info.audio_streams.is_empty() => {
                    self.snapshot();
                    self.music = Some(Music { path, name, start: self.trim.0, from: 0, len: info.duration, gain: 0.6, fade: true });
                    self.tab = Tab::Sound;
                }
                _ => self.notice = Some(Notice::new(ctx, crate::i18n::l("этот файл не открылся как музыка", "this file didn't open as music"))),
            }
        } else {
            match image::open(&path) {
                Ok(img) => {
                    // big photos are kept no bigger than the frame needs
                    let img = img.thumbnail(1600, 1600).to_rgba8();
                    let from = self.pos.clamp(0, (self.dur() - HNS / 5).max(0));
                    let to = (from + 3 * HNS).min(self.dur()).max(from + HNS / 5);
                    self.snapshot();
                    self.stickers.push(Sticker { path, img: Arc::new(img), from, to, x: 0.5, y: 0.45, scale: 0.3, anim: Anim::Pop });
                    self.select(Sel::Sticker(self.stickers.len() - 1));
                }
                Err(e) => self.notice = Some(Notice::new(ctx, lf!("картинка не открылась: {e}", "the picture didn't open: {e}"))),
            }
        }
    }

    /// The music's lane: its block moves to start it later.
    pub(super) fn music_lane(&mut self, ui: &mut egui::Ui, lane: Rect, label: Rect, area: Rect) {
        let p = ui.painter().clone();
        let add = ui.interact(label, ui.scope_id().with("music-add"), Sense::click());
        let h = add.hovered();
        icons::paint(&p, Rect::from_center_size(pos2(label.left() + 15.0, label.center().y), Vec2::splat(14.0)), Icon::Sound, 13.0, if h { ACCENT } else { FAINT });
        let text = if self.music.is_some() { crate::i18n::l("музыка", "music") } else { crate::i18n::l("+ музыка", "+ music") };
        p.text(pos2(label.left() + 38.0, label.center().y), Align2::LEFT_CENTER, text, font(12.0), if h { TEXT } else { FAINT });
        if add.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(crate::i18n::l("музыка под видео: mp3, wav, m4a…", "music under the video: mp3, wav, m4a…")).clicked() {
            if self.music.is_some() {
                self.tab = Tab::Sound;
            } else {
                self.import(ui.ctx(), Some(Import::Music));
            }
        }
        p.rect_filled(lane, 3, CANVAS);
        let Some(m) = self.music.clone() else { return };
        let (xa, xb) = (self.x_of(m.start, area), self.x_of(m.start + (m.len - m.from).max(HNS), area));
        let block = Rect::from_min_max(pos2(xa.max(lane.left()), lane.top() + 2.0), pos2(xb.min(lane.right()), lane.bottom() - 2.0));
        if block.width() <= 0.0 {
            return;
        }
        let resp = ui.interact(block, ui.scope_id().with("music-block"), Sense::click_and_drag());
        let fill = if resp.hovered() || resp.dragged() { Color32::from_rgb(0x7F, 0xCF, 0x9A) } else { Color32::from_rgb(0x5E, 0xA0, 0x76) };
        p.rect_filled(block, 3, fill);
        p.with_clip_rect(block.shrink2(vec2(5.0, 0.0))).text(pos2(block.left() + 6.0, block.center().y), Align2::LEFT_CENTER, &m.name, font_medium(11.0), ON_ACCENT);
        if resp.drag_started() {
            self.snapshot();
        }
        if resp.dragged() {
            let dt = (resp.drag_delta().x / area.width() * self.view_span() as f32) as i64;
            let dur = self.dur();
            if let Some(m) = &mut self.music {
                m.start = (m.start + dt).clamp(0, dur);
            }
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        } else if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
        if resp.on_hover_text(lf!("начало: {}", "starts at {}", fmt_time(m.start, true))).clicked() {
            self.tab = Tab::Sound;
        }
    }

    /// The pictures on screen now over the preview's `frame`: drawn, dragged.
    pub(super) fn stickers_overlay(&mut self, ui: &mut egui::Ui, frame: Rect) {
        for i in 0..self.stickers.len() {
            let st = self.stickers[i].clone();
            if self.pos < st.from || self.pos >= st.to {
                continue;
            }
            let tex = self
                .sticker_tex
                .entry(st.path.clone())
                .or_insert_with(|| ui.ctx().load_texture("sticker", theme::color_image(&st.img), egui::TextureOptions::LINEAR))
                .clone();
            let look = shorts::look(st.anim, st.from, st.to, self.pos, 0);
            let w = frame.width() * st.scale * look.scale;
            let h = w * st.img.height() as f32 / st.img.width().max(1) as f32;
            let center = pos2(frame.left() + frame.width() * st.x, frame.top() + frame.height() * st.y + look.dy * frame.height() * 0.05);
            let r = Rect::from_center_size(center, vec2(w, h));
            ui.painter().with_clip_rect(frame).image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(look.alpha));
            let resp = ui.interact(r.intersect(frame), ui.scope_id().with(("sticker", i)), Sense::click_and_drag());
            let picked = self.sel == Sel::Sticker(i);
            if resp.hovered() || picked {
                ui.painter().rect_stroke(r.expand(2.0), 4, Stroke::new(1.0, if picked { ACCENT } else { ACCENT.gamma_multiply(0.5) }), StrokeKind::Outside);
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::Grab);
            }
            if resp.drag_started() {
                self.snapshot();
                self.select(Sel::Sticker(i));
            }
            if resp.dragged() {
                let d = resp.drag_delta();
                self.stickers[i].x = (self.stickers[i].x + d.x / frame.width()).clamp(0.0, 1.0);
                self.stickers[i].y = (self.stickers[i].y + d.y / frame.height()).clamp(0.0, 1.0);
            }
            if resp.clicked() {
                self.select(Sel::Sticker(i));
            }
        }
    }

    /// The speeds over the filmstrip: a badge where a piece plays faster or slower.
    pub(super) fn paint_speeds(&self, p: &egui::Painter, strip: Rect, area: Rect) {
        for (a, b, s) in &self.speeds {
            let (xa, xb) = (self.x_of(*a, area), self.x_of(*b, area));
            if xb < strip.left() || xa > strip.right() {
                continue;
            }
            let tint = if *s > 1.0 { Color32::from_rgba_unmultiplied(255, 190, 80, 40) } else { Color32::from_rgba_unmultiplied(110, 180, 255, 40) };
            p.rect_filled(Rect::from_min_max(pos2(xa, strip.top()), pos2(xb, strip.bottom())), 0, tint);
            let text = format!("{s}×").replace('.', ",");
            let g = p.layout_no_wrap(text, font_medium(11.0), ON_ACCENT);
            let badge = Rect::from_min_size(pos2(xa.max(strip.left()) + 4.0, strip.bottom() - g.size().y - 8.0), g.size() + vec2(10.0, 4.0));
            p.rect_filled(badge, 4, if *s > 1.0 { Color32::from_rgb(255, 190, 80) } else { Color32::from_rgb(110, 180, 255) });
            p.galley(badge.min + vec2(5.0, 2.0), g, ON_ACCENT);
        }
    }

    // ------------------------------------------------------------ the strip

    /// Tools over the preview: cut, take out, add texts, undo.
    pub(super) fn tool_strip(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("tools")
            .exact_size(42.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(ui::pane_fill()).inner_margin(egui::Margin { left: 12, right: 12, top: 0, bottom: 0 }))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().hline(r.x_range().expand(12.0), r.bottom() - 0.5, Stroke::new(1.0, ui::hairline()));
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    // icon and name in one button; the hover plate goes under both
                    let icon_label = |ui: &mut egui::Ui, icon: Icon, label: &str, tip: &str| -> bool {
                        let galley = ui.painter().layout_no_wrap(label.to_string(), font(13.0), TEXT2);
                        let (r, resp) = ui.allocate_exact_size(vec2(10.0 + 16.0 + 7.0 + galley.size().x + 10.0, 30.0), Sense::click());
                        let hot = resp.hovered();
                        if hot {
                            ui.painter().rect_filled(r, 7, ui::hover_fill());
                        }
                        let col = if hot { TEXT } else { TEXT2 };
                        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 18.0, r.center().y), Vec2::splat(16.0)), icon, 14.0, if hot { ACCENT } else { DIM });
                        ui.painter().galley(pos2(r.left() + 33.0, r.center().y - galley.size().y / 2.0), galley, col);
                        resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip).clicked()
                    };
                    if icon_label(ui, Icon::Cut, crate::i18n::l("разрезать", "cut"), crate::i18n::l("разрез на месте курсора  s", "a cut at the playhead  s")) {
                        self.split();
                    }
                    let out = self.dropped.iter().any(|(a, b)| *a <= self.pos && self.pos < *b);
                    let label = if out { crate::i18n::l("вернуть кусок", "bring back") } else { crate::i18n::l("убрать кусок", "take out") };
                    if icon_label(ui, Icon::Trash, label, crate::i18n::l("кусок под курсором, между разрезами  del", "the piece under the playhead, between cuts  del")) {
                        self.toggle_piece(&ctx);
                    }
                    sep(ui);
                    ui.add_space(8.0);
                    if icon_label(ui, Icon::Text, crate::i18n::l("текст", "text"), crate::i18n::l("текст поверх видео: заголовок, подпись, реакция", "a text over the video: a title, a caption, a reaction")) {
                        self.add_title();
                    }
                    if icon_label(ui, Icon::TextBox, crate::i18n::l("субтитр", "subtitle"), crate::i18n::l("субтитр на месте курсора  t", "a subtitle at the playhead  t")) {
                        self.add_caption();
                    }
                    if icon_label(ui, Icon::Mic, crate::i18n::l("из речи", "from speech"), crate::i18n::l("субтитры из речи: распознать на этом компьютере", "subtitles from speech, recognized on this computer")) {
                        self.tab = Tab::Text;
                        self.sel = Sel::None;
                        self.speech_hint = true;
                    }
                    if icon_label(ui, Icon::Flag, crate::i18n::l("метка", "marker"), crate::i18n::l("метка на месте курсора  m", "a marker at the playhead  m")) {
                        self.add_marker();
                    }
                    sep(ui);
                    ui.add_space(8.0);
                    if icon_label(ui, Icon::Folder, crate::i18n::l("импорт", "import"), crate::i18n::l("музыка или картинка: mp3, wav, m4a, png, jpg…", "music or a picture: mp3, wav, m4a, png, jpg…")) {
                        self.import(&ctx, None);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.add_enabled_ui(!self.redo.is_empty(), |ui| {
                            if ui::icon_button(ui, Icon::Redo, crate::i18n::l("вернуть  ctrl+shift+z", "redo  ctrl+shift+z"), 30.0, false, ui::Kind::Ghost).clicked() {
                                self.redo();
                            }
                        });
                        ui.add_enabled_ui(!self.undo.is_empty(), |ui| {
                            if ui::icon_button(ui, Icon::Undo, crate::i18n::l("отменить  ctrl+z", "undo  ctrl+z"), 30.0, false, ui::Kind::Ghost).clicked() {
                                self.undo();
                            }
                        });
                    });
                });
            });
    }

    // ------------------------------------------------------------ the inspector

    pub(super) fn inspector(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("inspector")
            .exact_size(INSPECTOR_W)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(ui::pane_fill()).inner_margin(egui::Margin { left: 14, right: 14, top: 12, bottom: 12 }))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().vline(r.left() - 14.5, r.y_range().expand(12.0), Stroke::new(1.0, ui::hairline()));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (tab, label) in [(Tab::Video, crate::i18n::l("видео", "video")), (Tab::Text, crate::i18n::l("текст", "text")), (Tab::Sound, crate::i18n::l("звук", "sound"))] {
                        if ui::filter(ui, label, self.tab == tab).clicked() {
                            self.tab = tab;
                        }
                    }
                });
                ui.add_space(10.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    ui.set_width(ui.available_width());
                    match self.tab {
                        Tab::Video => self.video_page(ui),
                        Tab::Text => self.text_page(ui),
                        Tab::Sound => self.sound_page(ui),
                    }
                });
            });
    }

    fn video_page(&mut self, ui: &mut egui::Ui) {
        if let Sel::Sticker(i) = self.sel
            && i < self.stickers.len()
        {
            let st = &self.stickers[i];
            heading(ui, &lf!("картинка · {} – {}", "picture · {} – {}", fmt_time(st.from, true), fmt_time(st.to, true)));
            let mut k = self.stickers[i].scale;
            if labeled_slider(ui, crate::i18n::l("размер", "size"), &mut k, 0.05..=1.0) {
                self.stickers[i].scale = k;
            }
            note(ui, crate::i18n::l("двигай картинку прямо на превью", "drag the picture on the preview"));
            heading(ui, crate::i18n::l("появление", "comes on"));
            anim_filters(ui, &mut self.stickers[i].anim);
            if ui::button(ui, crate::i18n::l("удалить картинку", "delete the picture"), ui::Kind::Ghost).clicked() {
                self.delete_selected();
            }
            ui.add_space(10.0);
        }
        heading(ui, crate::i18n::l("скорость куска", "piece speed"));
        let (a, b) = self.piece_at(self.pos);
        let now = self.speed_at(self.pos);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
            for sp in [0.5f32, 1.0, 1.5, 2.0, 3.0] {
                let name = if sp == 1.0 { "1×".to_string() } else { format!("{sp}×").replace('.', ",") };
                if ui::filter(ui, &name, (now - sp).abs() < 0.01).clicked() {
                    self.set_speed(a, b, sp);
                }
            }
        });
        note(ui, if self.splits.is_empty() {
            crate::i18n::l("весь клип; разрежь (s), чтобы ускорить или замедлить только кусок. звук ускоряется вместе с ним, голос не пищит", "the whole clip; cut it (s) to change just a piece. the sound speeds up with it, voices keep their pitch")
        } else {
            crate::i18n::l("кусок под курсором между разрезами. звук ускоряется вместе с ним, голос не пищит", "the piece under the playhead, between cuts. the sound speeds up with it, voices keep their pitch")
        });
        ui.add_space(8.0);
        heading(ui, crate::i18n::l("кадр", "frame"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (a, name) in [(Aspect::Original, crate::i18n::l("как есть", "as recorded")), (Aspect::Crop, crate::i18n::l("9:16 часть", "9:16 slice")), (Aspect::Fit, crate::i18n::l("9:16 целиком", "9:16 whole"))] {
                if ui::filter(ui, name, self.aspect == a).clicked() {
                    self.aspect = a;
                }
            }
        });
        let hint = match self.aspect {
            Aspect::Original => crate::i18n::l("как записано", "as it was recorded"),
            Aspect::Crop => crate::i18n::l("1080 × 1920 для shorts, reels и tiktok: вертикальная полоса из кадра, рамку двигай на превью", "1080 × 1920 for shorts, reels and tiktok: an upright slice, drag the frame on the preview"),
            Aspect::Fit => crate::i18n::l("1080 × 1920 для shorts, reels и tiktok: весь кадр по центру, сверху и снизу он же размытый", "1080 × 1920 for shorts, reels and tiktok: the whole picture over a blurred copy"),
        };
        note(ui, hint);
        ui.add_space(8.0);
        heading(ui, crate::i18n::l("приближение", "zoom"));
        let mut z = self.transform.zoom;
        if labeled_slider(ui, crate::i18n::l("масштаб", "scale"), &mut z, 1.0..=3.0) {
            self.transform.zoom = z;
        }
        let zoomed = self.transform.zoom > 1.001 || self.transform.push;
        ui.add_enabled_ui(zoomed, |ui| {
            let (mut x, mut y) = (self.transform.x, self.transform.y);
            if labeled_slider(ui, crate::i18n::l("по горизонтали", "across"), &mut x, 0.0..=1.0) {
                self.transform.x = x;
            }
            if labeled_slider(ui, crate::i18n::l("по вертикали", "up-down"), &mut y, 0.0..=1.0) {
                self.transform.y = y;
            }
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(crate::i18n::l("плавный наезд", "slow push in")).font(font(12.5)).color(TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui::toggle(ui, &mut self.transform.push);
            });
        });
        note(ui, crate::i18n::l("камера медленно приближается за весь клип: живее для коротких видео", "the camera slowly moves in over the clip: livelier for short videos"));
        if !self.transform.is_identity() && ui::button(ui, crate::i18n::l("сбросить", "reset"), ui::Kind::Ghost).clicked() {
            self.transform = shorts::Transform::default();
        }
        ui.add_space(8.0);
        heading(ui, crate::i18n::l("видеодорожка", "video track"));
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(crate::i18n::l("сохранять видео", "keep the video")).font(font(12.5)).color(TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui::toggle(ui, &mut self.video_on);
            });
        });
        if !self.video_on {
            note(ui, crate::i18n::l("сохранится только звук: формат во вкладке «звук»", "only the sound is saved: the format is on the \"sound\" page"));
        }
    }

    fn text_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui::button(ui, crate::i18n::l("+ текст", "+ text"), ui::Kind::Ghost).clicked() {
                self.add_title();
            }
            if ui::button(ui, crate::i18n::l("+ субтитр", "+ subtitle"), ui::Kind::Ghost).clicked() {
                self.add_caption();
            }
        });
        ui.add_space(6.0);
        let mut delete = false;
        match self.sel {
            Sel::Title(i) if i < self.titles.len() => {
                let t = &self.titles[i];
                heading(ui, &lf!("текст · {} – {}", "text · {} – {}", fmt_time(t.from, true), fmt_time(t.to, true)));
                let edit = ui.add(
                    egui::TextEdit::multiline(&mut self.titles[i].text)
                        .hint_text(crate::i18n::l("что написать", "what to say"))
                        .char_limit(200)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .font(font(13.0)),
                );
                if std::mem::take(&mut self.text_focus) {
                    edit.request_focus();
                }
                note(ui, crate::i18n::l("двигай текст прямо на превью; края блока на дорожке «текст» меняют время", "drag the text on the preview; the block's edges on the \"text\" lane set its time"));
                heading(ui, crate::i18n::l("появление", "comes on"));
                anim_filters(ui, &mut self.titles[i].style.anim);
                style_controls(ui, &mut self.titles[i].style, false);
                delete = ui::button(ui, crate::i18n::l("удалить текст", "delete the text"), ui::Kind::Ghost).clicked();
            }
            Sel::Caption(i) if i < self.captions.len() => {
                let c = &self.captions[i];
                heading(ui, &lf!("субтитр · {} – {}", "subtitle · {} – {}", fmt_time(c.from, true), fmt_time(c.to, true)));
                let edit = ui.add(
                    egui::TextEdit::multiline(&mut self.captions[i].text)
                        .hint_text(crate::i18n::l("что говорят", "what is said"))
                        .char_limit(160)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .font(font(13.0)),
                );
                if std::mem::take(&mut self.text_focus) {
                    edit.request_focus();
                }
                delete = ui::button(ui, crate::i18n::l("удалить субтитр", "delete the subtitle"), ui::Kind::Ghost).clicked();
                ui.add_space(8.0);
            }
            _ => {
                note(ui, crate::i18n::l(
                    "текст — свои надписи поверх видео, субтитры — то, что говорят. клик по блоку на дорожке открывает его здесь",
                    "texts are your own words over the video, subtitles are what's said. click a block on its lane to edit it here",
                ));
            }
        }
        if delete {
            self.delete_selected();
        }
        if !matches!(self.sel, Sel::Title(_)) {
            ui.add_space(6.0);
            heading(ui, crate::i18n::l("все субтитры", "all subtitles"));
            let before = self.style;
            anim_filters(ui, &mut self.style.anim);
            style_controls(ui, &mut self.style, true);
            if self.style != before {
                let s = self.style;
                self.shared.update_config(|c| c.caption_style = s);
            }
            ui.add_space(8.0);
            self.speech_section(ui);
        }
    }

    /// Subtitles from speech: the model and the button.
    fn speech_section(&mut self, ui: &mut egui::Ui) {
        heading(ui, crate::i18n::l("субтитры из речи", "subtitles from speech"));
        if std::mem::take(&mut self.speech_hint) {
            ui.scroll_to_cursor(Some(egui::Align::Center));
        }
        let mut model = self.shared.config().speech_model;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for m in speech::Model::ALL {
                if ui::filter(ui, m.label(), model == m).clicked() && model != m {
                    model = m;
                    self.shared.update_config(|c| c.speech_model = m);
                }
            }
        });
        let state = if model.ready() {
            crate::i18n::l("установлена", "installed")
        } else {
            crate::i18n::l("не установлена: поставится по кнопке, или в настройках → система", "not installed: the button gets it, or settings → system")
        };
        note(ui, &format!("{} · {state}", model.desc()));
        note(ui, crate::i18n::l("слушает включённые дорожки: выключи звук игры во вкладке «звук», чтобы остался один голос", "listens to the tracks that are on: mute the game on the \"sound\" page to leave just the voice"));
        let label = if model.ready() { crate::i18n::l("распознать речь", "recognize speech") } else { crate::i18n::l("установить и распознать", "install and recognize") };
        ui.add_enabled_ui(self.speech.is_none() && self.job.is_none(), |ui| {
            if ui::button(ui, label, ui::Kind::Accent).clicked() {
                self.recognize(ui.ctx(), model);
            }
        });
    }

    fn sound_page(&mut self, ui: &mut egui::Ui) {
        heading(ui, crate::i18n::l("музыка", "music"));
        let mut remove = false;
        match &mut self.music {
            Some(m) => {
                note(ui, &m.name);
                let mut g = m.gain;
                if labeled_slider(ui, crate::i18n::l("громкость", "volume"), &mut g, 0.0..=1.5) {
                    m.gain = g;
                }
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(crate::i18n::l("затухание в конце", "fade out at the end")).font(font(12.5)).color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui::toggle(ui, &mut m.fade);
                    });
                });
                note(ui, crate::i18n::l("блок на дорожке «музыка» двигается: так она начнётся позже", "drag its block on the \"music\" lane to start it later"));
                remove = ui::button(ui, crate::i18n::l("убрать музыку", "remove the music"), ui::Kind::Ghost).clicked();
            }
            None => {
                if ui::button(ui, crate::i18n::l("добавить музыку", "add music"), ui::Kind::Ghost).clicked() {
                    self.import(ui.ctx(), Some(Import::Music));
                }
            }
        }
        if remove {
            self.snapshot();
            self.music = None;
        }
        ui.add_space(8.0);
        if self.tracks.is_empty() {
            note(ui, crate::i18n::l("в этом клипе нет звука", "this clip has no sound"));
        }
        for i in 0..self.tracks.len() {
            let (label, muted) = (self.tracks[i].label, self.tracks[i].gain.muted);
            heading(ui, label);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(crate::i18n::l("включена", "on")).font(font(12.5)).color(TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut on = !muted;
                    if ui::toggle(ui, &mut on).changed() {
                        self.tracks[i].gain.muted = !on;
                        self.sync_gains();
                    }
                });
            });
            ui.add_enabled_ui(!muted, |ui| {
                let mut v = self.tracks[i].gain.volume;
                if labeled_slider(ui, crate::i18n::l("громкость", "volume"), &mut v, 0.0..=2.0) {
                    self.tracks[i].gain.volume = v;
                    self.sync_gains();
                }
            });
            ui.add_space(6.0);
        }
        if self.tracks.len() > 1 {
            heading(ui, crate::i18n::l("в сохранённом mp4", "in the saved mp4"));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if ui::filter(ui, crate::i18n::t("одна дорожка"), self.mix_down).clicked() {
                    self.mix_down = true;
                }
                if ui::filter(ui, crate::i18n::t("раздельно"), !self.mix_down).clicked() {
                    self.mix_down = false;
                }
            });
            note(ui, if self.mix_down { crate::i18n::t("играет везде") } else { crate::i18n::t("дорожки отдельно, для монтажа") });
        }
        if !self.video_on {
            ui.add_space(6.0);
            heading(ui, crate::i18n::l("только звук", "sound only"));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for f in AudioFormat::ALL {
                    if ui::filter(ui, f.ext(), self.audio_format == f).clicked() {
                        self.audio_format = f;
                    }
                }
            });
            note(ui, self.audio_format.desc());
        }
    }

    // ------------------------------------------------------------ texts on the preview

    /// The subtitle and the texts on screen now, drawn over `frame` (the saved
    /// picture's place in the preview) as they'll be burnt in, animated;
    /// dragged, a text moves (subtitles: all of them, up and down).
    pub(super) fn texts_overlay(&mut self, ui: &mut egui::Ui, frame: Rect) {
        self.stickers_overlay(ui, frame);
        let vertical = self.aspect.vertical();
        let (out_w, out_h) = if vertical { shorts::VERTICAL } else { (self.info.width, self.info.height) };
        let k = frame.width() / out_w.max(1) as f32;
        if let Some(i) = self.captions.iter().position(|c| self.pos >= c.from && self.pos < c.to && !c.text.trim().is_empty()) {
            let c = self.captions[i].clone();
            let st = self.style;
            let center = pos2(frame.center().x, frame.top() + frame.height() * st.center_y(vertical));
            let plate = draw_text(ui.painter(), &c.text, &st, center, frame, k, (out_w, out_h), shorts::look(st.anim, c.from, c.to, self.pos, c.text.trim().chars().count()));
            let resp = ui.interact(plate, ui.scope_id().with("caption-drag"), Sense::click_and_drag());
            if resp.hovered() || self.cap_drag {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
                ui.painter().rect_stroke(plate.expand(2.0), 4, Stroke::new(1.0, ACCENT.gamma_multiply(0.6)), StrokeKind::Outside);
            }
            if resp.clicked() {
                self.select(Sel::Caption(i));
            }
            if resp.dragged()
                && let Some(pt) = resp.interact_pointer_pos()
            {
                self.cap_drag = true;
                self.style.y = Some(((pt.y - frame.top()) / frame.height()).clamp(0.06, 0.94));
            }
            if resp.drag_stopped() {
                self.cap_drag = false;
                let s = self.style;
                self.shared.update_config(|c| c.caption_style = s);
            }
        }
        for i in 0..self.titles.len() {
            let t = self.titles[i].clone();
            if self.pos < t.from || self.pos >= t.to {
                continue;
            }
            let shown = if t.text.trim().is_empty() && self.sel == Sel::Title(i) { crate::i18n::l("текст", "text").to_string() } else { t.text.clone() };
            if shown.trim().is_empty() {
                continue;
            }
            let center = pos2(frame.left() + frame.width() * t.x, frame.top() + frame.height() * t.y);
            let plate = draw_text(ui.painter(), &shown, &t.style, center, frame, k, (out_w, out_h), shorts::look(t.style.anim, t.from, t.to, self.pos, shown.trim().chars().count()));
            let resp = ui.interact(plate, ui.scope_id().with(("title-drag", i)), Sense::click_and_drag());
            let picked = self.sel == Sel::Title(i);
            if resp.hovered() || picked {
                ui.painter().rect_stroke(plate.expand(3.0), 4, Stroke::new(1.0, if picked { ACCENT } else { ACCENT.gamma_multiply(0.5) }), StrokeKind::Outside);
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::Grab);
            }
            if resp.drag_started() {
                self.snapshot();
                self.select(Sel::Title(i));
            }
            if resp.dragged() {
                let d = resp.drag_delta();
                self.titles[i].x = (self.titles[i].x + d.x / frame.width()).clamp(0.02, 0.98);
                self.titles[i].y = (self.titles[i].y + d.y / frame.height()).clamp(0.03, 0.97);
            }
            if resp.clicked() {
                self.select(Sel::Title(i));
            }
        }
    }
}

/// A text as it's burnt in, at `center` of the preview's `frame` (`k`: preview
/// points per frame pixel), as `look` has it; returns where it is.
#[allow(clippy::too_many_arguments)]
fn draw_text(p: &egui::Painter, text: &str, st: &CaptionStyle, center: egui::Pos2, frame: Rect, k: f32, out: (u32, u32), look: shorts::Look) -> Rect {
    let size0 = (st.text_size(out.0, out.1) * k).max(6.0);
    let size = size0 * look.scale;
    let family = match st.font {
        CapFont::Regular => theme::medium(),
        CapFont::Bold => theme::semibold(),
        CapFont::Mono => theme::mono_medium(),
    };
    let a = look.alpha.clamp(0.0, 1.0);
    let color = Color32::from_rgb(st.color[0], st.color[1], st.color[2]).gamma_multiply(a);
    let shown = shorts::typed_part(&st.text(text), look.typed);
    let mut job = egui::text::LayoutJob::simple(shown, egui::FontId::new(size, family), color, frame.width() * 0.84);
    job.halign = egui::Align::Center;
    let galley = p.layout_job(job);
    let center = center + vec2(0.0, look.dy * size0);
    let pad = if st.back == CapBack::Plate { vec2(size * 1.1, size * 0.6) } else { vec2(size * 0.5, size * 0.4) };
    let plate = Rect::from_center_size(center, galley.size() + pad);
    let at = pos2(center.x, plate.top() + pad.y / 2.0);
    match st.back {
        CapBack::Plate => {
            p.rect_filled(plate, size * 0.35, Color32::from_rgba_unmultiplied(12, 12, 11, (184.0 * a) as u8));
        }
        CapBack::Outline => {
            let r = (size * 0.09).max(1.0);
            for i in 0..12 {
                let ang = i as f32 / 12.0 * std::f32::consts::TAU;
                p.galley_with_override_text_color(at + vec2(ang.cos(), ang.sin()) * r, galley.clone(), Color32::from_rgb(8, 8, 8).gamma_multiply(a));
            }
        }
        CapBack::Shadow => {
            p.galley_with_override_text_color(at + vec2(0.0, size * 0.06), galley.clone(), Color32::from_black_alpha((150.0 * a) as u8));
        }
    }
    p.galley(at, galley, color);
    plate
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(font_medium(12.5)).color(TEXT2));
}

fn note(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(font(12.0)).color(DIM));
}

/// A slider with its name and its value in percent: true when it moved.
fn labeled_slider(ui: &mut egui::Ui, name: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_sized(vec2(92.0, 20.0), egui::Label::new(egui::RichText::new(name).font(font(12.5)).color(TEXT)));
        ui.spacing_mut().slider_width = (ui.available_width() - 48.0).max(60.0);
        changed = ui::slider(ui, v, range, 0.0).changed();
        ui.label(egui::RichText::new(format!("{:.0}%", *v * 100.0)).font(mono(10.5)).color(FAINT));
    });
    changed
}

fn anim_filters(ui: &mut egui::Ui, anim: &mut Anim) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
        for a in Anim::ALL {
            if ui::filter(ui, a.label(), *anim == a).clicked() {
                *anim = a;
            }
        }
    });
}

/// Size, font, caps, background, color (and, for subtitles, their place).
fn style_controls(ui: &mut egui::Ui, st: &mut CaptionStyle, place: bool) {
    let row = |ui: &mut egui::Ui, f: &mut dyn FnMut(&mut egui::Ui)| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
            f(ui);
        });
    };
    heading(ui, crate::i18n::l("размер", "size"));
    row(ui, &mut |ui| {
        for (k, name) in [(0.8, "s"), (1.0, "m"), (1.3, "l"), (1.7, "xl"), (2.2, "xxl")] {
            if ui::filter(ui, name, (st.size - k).abs() < 0.01).clicked() {
                st.size = k;
            }
        }
    });
    heading(ui, crate::i18n::l("шрифт", "font"));
    row(ui, &mut |ui| {
        for f in CapFont::ALL {
            if ui::filter(ui, f.label(), st.font == f).clicked() {
                st.font = f;
            }
        }
        if ui::filter(ui, crate::i18n::l("ЗАГЛАВНЫЕ", "CAPS"), st.upper).clicked() {
            st.upper = !st.upper;
        }
    });
    heading(ui, crate::i18n::l("фон", "background"));
    row(ui, &mut |ui| {
        for b in CapBack::ALL {
            if ui::filter(ui, b.label(), st.back == b).clicked() {
                st.back = b;
            }
        }
    });
    heading(ui, crate::i18n::l("цвет", "color"));
    row(ui, &mut |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for c in shorts::CAPTION_COLORS {
            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
            ui.painter().circle_filled(r.center(), 8.0, Color32::from_rgb(c[0], c[1], c[2]));
            if st.color == c {
                ui.painter().circle_stroke(r.center(), 10.5, Stroke::new(1.5, TEXT));
            }
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                st.color = c;
            }
        }
    });
    if place {
        heading(ui, crate::i18n::l("место", "place"));
        row(ui, &mut |ui| {
            for (y, name) in [(Some(0.14), crate::i18n::l("сверху", "top")), (Some(0.5), crate::i18n::l("по центру", "middle")), (None, crate::i18n::l("снизу", "bottom"))] {
                let on = match (st.y, y) {
                    (None, None) => true,
                    (Some(a), Some(b)) => (a - b).abs() < 0.01,
                    _ => false,
                };
                if ui::filter(ui, name, on).clicked() {
                    st.y = y;
                }
            }
        });
        note(ui, crate::i18n::l("или перетащи субтитр на превью вверх-вниз", "or drag the subtitle up and down on the preview"));
    }
}

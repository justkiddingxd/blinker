//! Other brands' marks (GitHub, Discord) for links to them: their SVG paths
//! from Simple Icons (CC0), filled with tiny-skia.

use tiny_skia as sk;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Mark {
    GitHub,
    Discord,
}

impl Mark {
    /// The path in a 24 × 24 box.
    fn path(self) -> &'static str {
        match self {
            Mark::GitHub => "M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12",
            Mark::Discord => "M20.317 4.3698a19.7913 19.7913 0 00-4.8851-1.5152.0741.0741 0 00-.0785.0371c-.211.3753-.4447.8648-.6083 1.2495-1.8447-.2762-3.68-.2762-5.4868 0-.1636-.3933-.4058-.8742-.6177-1.2495a.077.077 0 00-.0785-.037 19.7363 19.7363 0 00-4.8852 1.515.0699.0699 0 00-.0321.0277C.5334 9.0458-.319 13.5799.0992 18.0578a.0824.0824 0 00.0312.0561c2.0528 1.5076 4.0413 2.4228 5.9929 3.0294a.0777.0777 0 00.0842-.0276c.4616-.6304.8731-1.2952 1.226-1.9942a.076.076 0 00-.0416-.1057c-.6528-.2476-1.2743-.5495-1.8722-.8923a.077.077 0 01-.0076-.1277c.1258-.0943.2517-.1923.3718-.2914a.0743.0743 0 01.0776-.0105c3.9278 1.7933 8.18 1.7933 12.0614 0a.0739.0739 0 01.0785.0095c.1202.099.246.1981.3728.2924a.077.077 0 01-.0066.1276 12.2986 12.2986 0 01-1.873.8914.0766.0766 0 00-.0407.1067c.3604.698.7719 1.3628 1.225 1.9932a.076.076 0 00.0842.0286c1.961-.6067 3.9495-1.5219 6.0023-3.0294a.077.077 0 00.0313-.0552c.5004-5.177-.8382-9.6739-3.5485-13.6604a.061.061 0 00-.0312-.0286zM8.02 15.3312c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9555-2.4189 2.157-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.9555 2.4189-2.1569 2.4189zm7.9748 0c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9554-2.4189 2.1569-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.946 2.4189-2.1568 2.4189Z",
        }
    }

    /// The mark in `color` (white: tinted later), `px` square.
    pub fn image(self, px: u32) -> image::RgbaImage {
        let mut pm = sk::Pixmap::new(px.max(1), px.max(1)).expect("mark size");
        let k = px as f32 / 24.0;
        if let Some(path) = parse(self.path()) {
            let mut paint = sk::Paint::default();
            paint.anti_alias = true;
            paint.set_color_rgba8(255, 255, 255, 255);
            pm.fill_path(&path, &paint, sk::FillRule::EvenOdd, sk::Transform::from_scale(k, k), None);
        }
        crate::theme::pixmap_to_image(&pm)
    }
}

/// SVG path data (M L H V C S Q T A Z, absolute and relative) to a path.
/// Arcs become lines: the marks here only use tiny ones on corners.
fn parse(d: &str) -> Option<sk::Path> {
    let mut nums = Numbers { s: d.as_bytes(), i: 0 };
    let mut pb = sk::PathBuilder::new();
    let (mut cx, mut cy, mut sx, mut sy) = (0f32, 0f32, 0f32, 0f32);
    // the last control point, for the smooth curves
    let mut last_ctrl: Option<(f32, f32)> = None;
    let mut cmd = b'M';
    loop {
        nums.skip_space();
        let Some(&c) = nums.s.get(nums.i) else { break };
        if c.is_ascii_alphabetic() {
            cmd = c;
            nums.i += 1;
            if cmd == b'Z' || cmd == b'z' {
                pb.close();
                (cx, cy) = (sx, sy);
                last_ctrl = None;
                continue;
            }
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let (x, y) = (nums.num()? + ox, nums.num()? + oy);
                pb.move_to(x, y);
                (cx, cy, sx, sy) = (x, y, x, y);
                // more pairs after a move are lines
                cmd = if rel { b'l' } else { b'L' };
                last_ctrl = None;
            }
            b'L' => {
                let (x, y) = (nums.num()? + ox, nums.num()? + oy);
                pb.line_to(x, y);
                (cx, cy) = (x, y);
                last_ctrl = None;
            }
            b'H' => {
                cx = nums.num()? + ox;
                pb.line_to(cx, cy);
                last_ctrl = None;
            }
            b'V' => {
                cy = nums.num()? + oy;
                pb.line_to(cx, cy);
                last_ctrl = None;
            }
            b'C' => {
                let (x1, y1, x2, y2) = (nums.num()? + ox, nums.num()? + oy, nums.num()? + ox, nums.num()? + oy);
                let (x, y) = (nums.num()? + ox, nums.num()? + oy);
                pb.cubic_to(x1, y1, x2, y2, x, y);
                (cx, cy) = (x, y);
                last_ctrl = Some((x2, y2));
            }
            b'S' => {
                let (x1, y1) = last_ctrl.map_or((cx, cy), |(lx, ly)| (2.0 * cx - lx, 2.0 * cy - ly));
                let (x2, y2) = (nums.num()? + ox, nums.num()? + oy);
                let (x, y) = (nums.num()? + ox, nums.num()? + oy);
                pb.cubic_to(x1, y1, x2, y2, x, y);
                (cx, cy) = (x, y);
                last_ctrl = Some((x2, y2));
            }
            b'Q' => {
                let (x1, y1) = (nums.num()? + ox, nums.num()? + oy);
                let (x, y) = (nums.num()? + ox, nums.num()? + oy);
                pb.quad_to(x1, y1, x, y);
                (cx, cy) = (x, y);
                last_ctrl = Some((x1, y1));
            }
            b'T' => {
                let (x1, y1) = last_ctrl.map_or((cx, cy), |(lx, ly)| (2.0 * cx - lx, 2.0 * cy - ly));
                let (x, y) = (nums.num()? + ox, nums.num()? + oy);
                pb.quad_to(x1, y1, x, y);
                (cx, cy) = (x, y);
                last_ctrl = Some((x1, y1));
            }
            b'A' => {
                // rx ry rotation large-arc sweep x y; the flags may be run together
                let _ = (nums.num()?, nums.num()?, nums.num()?, nums.flag()?, nums.flag()?);
                let (x, y) = (nums.num()? + ox, nums.num()? + oy);
                pb.line_to(x, y);
                (cx, cy) = (x, y);
                last_ctrl = None;
            }
            _ => return None,
        }
    }
    pb.finish()
}

struct Numbers<'a> {
    s: &'a [u8],
    i: usize,
}

impl Numbers<'_> {
    fn skip_space(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_whitespace() || *c == b',') {
            self.i += 1;
        }
    }

    /// One number: "-1.5", ".5", "1e-3"; a second dot starts the next one.
    fn num(&mut self) -> Option<f32> {
        self.skip_space();
        let start = self.i;
        let at = |i: usize| self.s.get(i).copied();
        let mut i = self.i;
        if matches!(at(i), Some(b'-' | b'+')) {
            i += 1;
        }
        let mut dot = false;
        while let Some(c) = at(i) {
            if c.is_ascii_digit() {
                i += 1;
            } else if c == b'.' && !dot {
                dot = true;
                i += 1;
            } else if (c == b'e' || c == b'E') && i > start {
                i += 1;
                if matches!(at(i), Some(b'-' | b'+')) {
                    i += 1;
                }
            } else {
                break;
            }
        }
        self.i = i;
        std::str::from_utf8(&self.s[start..i]).ok()?.parse().ok()
    }

    /// An arc flag: a single 0 or 1.
    fn flag(&mut self) -> Option<f32> {
        self.skip_space();
        let c = *self.s.get(self.i)?;
        self.i += 1;
        match c {
            b'0' => Some(0.0),
            b'1' => Some(1.0),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_parse() {
        for m in [Mark::GitHub, Mark::Discord] {
            let p = parse(m.path()).expect("parses");
            let b = p.bounds();
            assert!(b.left() >= -0.5 && b.right() <= 24.5 && b.top() >= -0.5 && b.bottom() <= 24.5, "{m:?} {b:?}");
            assert!(b.width() > 20.0, "{m:?} {b:?}");
        }
    }

    /// `BLINKER_TEST_BRAND` = a PNG of both marks.
    #[test]
    fn sheet() {
        let Ok(out) = std::env::var("BLINKER_TEST_BRAND") else { return };
        let mut sheet = image::RgbaImage::from_pixel(200, 100, image::Rgba([30, 30, 30, 255]));
        image::imageops::overlay(&mut sheet, &Mark::GitHub.image(80), 10, 10);
        image::imageops::overlay(&mut sheet, &Mark::Discord.image(80), 110, 10);
        sheet.save(out).unwrap();
    }
}

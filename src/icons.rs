//! Line icons drawn as vectors in a 16x16 box (same geometry as the mockups).

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// some icons are only drawn by the Windows tray menu
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Icon {
    Redact,
    TextBox,
    Undo,
    Sound,
    Bell,
    Copy,
    Save,
    Edit,
    Pin,
    Close,
    Region,
    Screen,
    Window,
    Repeat,
    Folder,
    Settings,
    Keys,
    Select,
    Arrow,
    Line,
    Rect,
    Ellipse,
    Pen,
    Marker,
    Blur,
    Counter,
    Text,
    Crop,
    Record,
    Mic,
    MicOff,
    Play,
    Pause,
    Mute,
    Frame,
    Upload,
    Video,
    VideoOff,
    /// A marker on the video timeline.
    Flag,
    Trash,
    /// Opens a link (an arrow out of a box).
    Link,
    /// Text read off a picture: lines inside a frame's corners.
    Scan,
    /// Scissors: a cut on the timeline.
    Cut,
    /// The undo arrow turned the other way.
    Redo,
    /// A plus: add something.
    Plus,
    Pipette,
    /// Tiles over a list: the tray menu.
    Menu,
}

/// (points, closed)
pub type Path = (Vec<Pos2>, bool);

fn p(pts: &[(f32, f32)], closed: bool) -> Path {
    (pts.iter().map(|&(x, y)| pos2(x, y)).collect(), closed)
}

fn arc(cx: f32, cy: f32, rx: f32, ry: f32, from_deg: f32, to_deg: f32) -> Vec<Pos2> {
    let n = 28;
    (0..=n)
        .map(|i| {
            let a = (from_deg + (to_deg - from_deg) * i as f32 / n as f32).to_radians();
            pos2(cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

fn rect(l: f32, t: f32, r: f32, b: f32) -> Path {
    p(&[(l, t), (r, t), (r, b), (l, b)], true)
}

pub fn paths(icon: Icon) -> Vec<Path> {
    use Icon::*;
    match icon {
        Redact => vec![
            rect(2.5, 4.5, 13.5, 11.5),
            p(&[(4.5, 6.5), (11.5, 6.5)], false),
            p(&[(4.5, 8.0), (11.5, 8.0)], false),
            p(&[(4.5, 9.5), (11.5, 9.5)], false),
        ],
        TextBox => vec![
            rect(1.5, 2.5, 14.5, 13.5),
            p(&[(5.0, 5.5), (11.0, 5.5)], false),
            p(&[(8.0, 5.5), (8.0, 11.0)], false),
        ],
        Undo => {
            let mut a = vec![pos2(2.5, 6.0), pos2(10.0, 6.0)];
            a.extend(arc(10.0, 9.75, 3.75, 3.75, -90.0, 90.0));
            a.push(pos2(6.5, 13.5));
            vec![p(&[(5.5, 3.0), (2.5, 6.0), (5.5, 9.0)], false), (a, false)]
        }
        Redo => {
            let mut a = vec![pos2(13.5, 6.0), pos2(6.0, 6.0)];
            a.extend(arc(6.0, 9.75, 3.75, 3.75, 270.0, 90.0));
            a.push(pos2(9.5, 13.5));
            vec![p(&[(10.5, 3.0), (13.5, 6.0), (10.5, 9.0)], false), (a, false)]
        }
        Cut => vec![
            (arc(4.5, 12.0, 2.3, 2.3, 0.0, 360.0), true),
            (arc(11.5, 12.0, 2.3, 2.3, 0.0, 360.0), true),
            p(&[(6.2, 10.4), (12.0, 2.0)], false),
            p(&[(9.8, 10.4), (4.0, 2.0)], false),
        ],
        Plus => vec![p(&[(8.0, 3.0), (8.0, 13.0)], false), p(&[(3.0, 8.0), (13.0, 8.0)], false)],
        Sound => vec![
            p(&[(2.5, 6.0), (5.0, 6.0), (8.5, 3.0), (8.5, 13.0), (5.0, 10.0), (2.5, 10.0)], true),
            (arc(9.0, 8.0, 2.5, 2.5, -50.0, 50.0), false),
            (arc(9.0, 8.0, 5.0, 5.0, -55.0, 55.0), false),
        ],
        Bell => vec![
            rect(2.5, 2.5, 13.5, 13.5),
            rect(7.5, 8.5, 13.5, 13.5),
        ],
        Copy => vec![rect(5.5, 5.5, 13.5, 13.5), p(&[(2.5, 10.5), (2.5, 2.5), (10.5, 2.5)], false)],
        Save => vec![
            p(&[(8.0, 2.0), (8.0, 10.0)], false),
            p(&[(4.5, 6.5), (8.0, 10.0), (11.5, 6.5)], false),
            p(&[(2.5, 11.5), (2.5, 13.5), (13.5, 13.5), (13.5, 11.5)], false),
        ],
        Edit => vec![
            p(&[(2.5, 13.5), (3.0, 10.5), (10.5, 3.0), (13.0, 5.5), (5.5, 13.0)], true),
            p(&[(9.0, 4.5), (11.5, 7.0)], false),
        ],
        Pin => vec![
            p(&[(5.5, 2.5), (10.5, 2.5)], false),
            p(&[(6.5, 2.5), (6.5, 7.0), (4.5, 9.5), (11.5, 9.5), (9.5, 7.0), (9.5, 2.5)], false),
            p(&[(8.0, 9.5), (8.0, 14.0)], false),
        ],
        Close => vec![p(&[(4.0, 4.0), (12.0, 12.0)], false), p(&[(12.0, 4.0), (4.0, 12.0)], false)],
        Region => vec![
            p(&[(2.5, 5.0), (2.5, 2.5), (5.0, 2.5)], false),
            p(&[(11.0, 2.5), (13.5, 2.5), (13.5, 5.0)], false),
            p(&[(13.5, 11.0), (13.5, 13.5), (11.0, 13.5)], false),
            p(&[(5.0, 13.5), (2.5, 13.5), (2.5, 11.0)], false),
        ],
        Screen => vec![rect(2.5, 3.5, 13.5, 11.5), p(&[(6.0, 14.0), (10.0, 14.0)], false)],
        Window => vec![rect(2.5, 3.5, 13.5, 12.5), p(&[(2.5, 6.0), (13.5, 6.0)], false)],
        Repeat => vec![
            (arc(8.0, 8.0, 5.0, 5.0, 180.0, -127.0), false),
            p(&[(3.0, 2.5), (3.0, 5.5), (6.0, 5.5)], false),
        ],
        Folder => vec![p(&[(2.5, 4.5), (2.5, 12.5), (13.5, 12.5), (13.5, 6.0), (7.5, 6.0), (6.0, 4.5)], true)],
        Settings => vec![
            p(&[(2.0, 4.5), (9.0, 4.5)], false),
            p(&[(12.0, 4.5), (14.0, 4.5)], false),
            p(&[(2.0, 11.5), (4.0, 11.5)], false),
            p(&[(7.0, 11.5), (14.0, 11.5)], false),
            p(&[(10.5, 3.0), (10.5, 6.0)], false),
            p(&[(5.5, 10.0), (5.5, 13.0)], false),
        ],
        Keys => vec![
            p(&[(2.5, 5.5), (13.5, 5.5)], false),
            p(&[(2.5, 10.5), (13.5, 10.5)], false),
            p(&[(5.5, 2.5), (5.5, 13.5)], false),
            p(&[(10.5, 2.5), (10.5, 13.5)], false),
        ],
        Select => vec![p(
            &[(4.0, 2.5), (4.0, 13.0), (6.8, 10.3), (8.8, 14.2), (10.6, 13.3), (8.6, 9.5), (12.3, 9.3)],
            true,
        )],
        Arrow => vec![p(&[(3.0, 13.0), (13.0, 3.0)], false), p(&[(7.0, 3.0), (13.0, 3.0), (13.0, 9.0)], false)],
        Line => vec![p(&[(3.0, 13.0), (13.0, 3.0)], false)],
        Rect => vec![rect(2.5, 4.5, 13.5, 11.5)],
        Ellipse => vec![(arc(8.0, 8.0, 5.5, 4.5, 0.0, 360.0), true)],
        Pen => vec![p(&[(2.5, 12.0), (4.5, 9.0), (6.5, 11.0), (9.0, 6.0), (11.0, 8.0), (13.5, 4.0)], false)],
        Marker => vec![
            p(&[(3.0, 12.0), (10.0, 5.0), (12.0, 7.0), (5.0, 14.0)], true),
            p(&[(2.0, 15.0), (8.0, 15.0)], false),
        ],
        Blur => vec![rect(3.0, 3.0, 7.0, 7.0), rect(9.0, 3.0, 13.0, 7.0), rect(3.0, 9.0, 7.0, 13.0), rect(9.0, 9.0, 13.0, 13.0)],
        Counter => vec![
            (arc(8.0, 8.0, 5.5, 5.5, 0.0, 360.0), true),
            p(&[(7.0, 6.5), (8.5, 5.5), (8.5, 11.0)], false),
        ],
        Text => vec![p(&[(3.0, 3.5), (13.0, 3.5)], false), p(&[(8.0, 3.5), (8.0, 13.5)], false)],
        Pipette => vec![
            p(&[(2.5, 13.5), (3.0, 11.5), (9.0, 5.5)], false),
            p(&[(7.0, 4.5), (11.5, 9.0)], false),
            (arc(11.6, 4.4, 2.3, 2.3, 120.0, 330.0), false),
        ],
        Menu => vec![rect(2.5, 2.5, 6.5, 6.5), rect(9.5, 2.5, 13.5, 6.5), p(&[(2.5, 10.0), (13.5, 10.0)], false), p(&[(2.5, 13.0), (10.5, 13.0)], false)],
        Scan => vec![
            p(&[(1.5, 5.0), (1.5, 1.5), (5.0, 1.5)], false),
            p(&[(11.0, 1.5), (14.5, 1.5), (14.5, 5.0)], false),
            p(&[(14.5, 11.0), (14.5, 14.5), (11.0, 14.5)], false),
            p(&[(5.0, 14.5), (1.5, 14.5), (1.5, 11.0)], false),
            p(&[(4.5, 6.0), (11.5, 6.0)], false),
            p(&[(4.5, 8.5), (11.5, 8.5)], false),
            p(&[(4.5, 11.0), (9.0, 11.0)], false),
        ],
        Crop => vec![
            p(&[(4.5, 1.5), (4.5, 11.5), (14.5, 11.5)], false),
            p(&[(1.5, 4.5), (11.5, 4.5), (11.5, 14.5)], false),
        ],
        Record => vec![(arc(8.0, 8.0, 6.0, 6.0, 0.0, 360.0), true), (arc(8.0, 8.0, 2.5, 2.5, 0.0, 360.0), true)],
        Mic => mic(),
        MicOff => {
            let mut v = mic();
            v.push(p(&[(2.5, 2.5), (13.5, 13.5)], false));
            v
        }
        Play => vec![p(&[(4.5, 2.5), (13.0, 8.0), (4.5, 13.5)], true)],
        Pause => vec![rect(4.0, 3.0, 6.5, 13.0), rect(9.5, 3.0, 12.0, 13.0)],
        Video => vec![rect(1.5, 4.0, 10.5, 12.0), p(&[(10.5, 7.0), (14.5, 4.5), (14.5, 11.5), (10.5, 9.0)], true)],
        VideoOff => vec![
            rect(1.5, 4.0, 10.5, 12.0),
            p(&[(10.5, 7.0), (14.5, 4.5), (14.5, 11.5), (10.5, 9.0)], true),
            p(&[(1.5, 1.5), (14.5, 14.5)], false),
        ],
        Flag => vec![p(&[(3.5, 14.5), (3.5, 2.0)], false), p(&[(3.5, 2.5), (12.5, 2.5), (10.0, 5.5), (12.5, 8.5), (3.5, 8.5)], false)],
        Trash => vec![
            p(&[(2.5, 4.5), (13.5, 4.5)], false),
            p(&[(6.0, 4.5), (6.0, 2.5), (10.0, 2.5), (10.0, 4.5)], false),
            p(&[(4.0, 4.5), (4.8, 13.5), (11.2, 13.5), (12.0, 4.5)], false),
            p(&[(6.8, 7.0), (7.0, 11.0)], false),
            p(&[(9.2, 7.0), (9.0, 11.0)], false),
        ],
        Link => vec![
            p(&[(7.0, 3.5), (2.5, 3.5), (2.5, 13.5), (12.5, 13.5), (12.5, 9.0)], false),
            p(&[(7.5, 8.5), (13.5, 2.5)], false),
            p(&[(9.0, 2.5), (13.5, 2.5), (13.5, 7.0)], false),
        ],
        Frame => vec![rect(1.5, 2.5, 14.5, 13.5), rect(4.5, 5.5, 11.5, 10.5)],
        Upload => vec![
            p(&[(8.0, 11.0), (8.0, 2.5)], false),
            p(&[(4.5, 6.0), (8.0, 2.5), (11.5, 6.0)], false),
            p(&[(2.5, 10.0), (2.5, 13.5), (13.5, 13.5), (13.5, 10.0)], false),
        ],
        Mute => vec![
            p(&[(1.5, 6.0), (4.0, 6.0), (7.5, 3.0), (7.5, 13.0), (4.0, 10.0), (1.5, 10.0)], true),
            p(&[(10.0, 6.0), (14.0, 10.0)], false),
            p(&[(14.0, 6.0), (10.0, 10.0)], false),
        ],
    }
}

fn mic() -> Vec<Path> {
    let mut capsule = arc(8.0, 5.0, 2.5, 2.5, 180.0, 360.0);
    capsule.extend(arc(8.0, 8.0, 2.5, 2.5, 0.0, 180.0));
    vec![
        (capsule, true),
        (arc(8.0, 8.0, 4.5, 4.5, 0.0, 180.0), false),
        p(&[(8.0, 12.5), (8.0, 14.5)], false),
        p(&[(5.5, 14.5), (10.5, 14.5)], false),
    ]
}

/// Paints `icon` centered in `rect` at `size` points.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, size: f32, color: Color32) {
    let k = size / 16.0;
    let origin = rect.center() - eframe::egui::vec2(size, size) / 2.0;
    let stroke = Stroke::new((1.4 * k).max(1.1), color);
    for (pts, closed) in paths(icon) {
        let pts: Vec<Pos2> = pts.into_iter().map(|q| origin + q.to_vec2() * k).collect();
        if closed {
            painter.add(Shape::closed_line(pts, stroke));
        } else {
            painter.add(Shape::line(pts, stroke));
        }
    }
}

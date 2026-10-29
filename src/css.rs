//! blinker's own small CSS: the syntax of CSS, our own elements. A sheet is
//! rules of selectors (`toolbar button.ghost:hover`) and declarations
//! (`background: #222; border-radius: 8px`), with `--variables` and `var()`.
//!
//! Elements are the parts of blinker's windows (`overlay`, `toolbar`,
//! `button`…), each with classes; an element is asked for as a path from the
//! window down (`overlay toolbar button.icon.ghost`) in some states (hover,
//! pressed…). As in CSS: a more specific selector wins, then the later rule;
//! `--variables` are inherited down the path, other properties are not.

use eframe::egui::{Color32, Stroke};
use std::collections::HashMap;

/// The states an element can be in, as flags (`:hover`, `:active`…).
pub mod state {
    pub const HOVER: u8 = 1;
    /// Held down.
    pub const ACTIVE: u8 = 2;
    /// Turned on, chosen: the current tool, the open section.
    pub const CHECKED: u8 = 4;
    pub const DISABLED: u8 = 8;
    pub const FOCUS: u8 = 16;
}

/// A message in the interface's language.
fn tr(ru: String, en: String) -> String {
    if crate::i18n::en() { en } else { ru }
}

fn state_of(name: &str) -> Option<u8> {
    Some(match name {
        "hover" => state::HOVER,
        "active" | "pressed" => state::ACTIVE,
        "checked" | "selected" | "on" => state::CHECKED,
        "disabled" => state::DISABLED,
        "focus" => state::FOCUS,
        _ => return None,
    })
}

/// One part of a selector or a path: `button.icon.ghost:hover`.
#[derive(Clone, Debug, Default, PartialEq)]
struct Compound {
    /// None: `*` or no name (`.ghost`).
    name: Option<String>,
    classes: Vec<String>,
    states: u8,
    /// `:root`: the top of every path.
    root: bool,
}

impl Compound {
    fn parse(s: &str) -> Result<Compound, String> {
        let mut c = Compound::default();
        let mut rest = s;
        let name_end = rest.find(['.', ':']).unwrap_or(rest.len());
        let name = &rest[..name_end];
        if !name.is_empty() && name != "*" {
            if !name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
                return Err(tr(format!("непонятный селектор «{s}»"), format!("can't read the selector \"{s}\"")));
            }
            c.name = Some(name.to_ascii_lowercase());
        }
        rest = &rest[name_end..];
        while !rest.is_empty() {
            let kind = rest.as_bytes()[0];
            let body = &rest[1..];
            let end = body.find(['.', ':']).unwrap_or(body.len());
            let word = body[..end].to_ascii_lowercase();
            if word.is_empty() {
                return Err(tr(format!("непонятный селектор «{s}»"), format!("can't read the selector \"{s}\"")));
            }
            if kind == b'.' {
                c.classes.push(word);
            } else if word == "root" {
                c.root = true;
            } else {
                c.states |= state_of(&word).ok_or_else(|| tr(format!("нет такого состояния «:{word}»"), format!("no such state \":{word}\"")))?;
            }
            rest = &body[end..];
        }
        Ok(c)
    }

    /// This selector part fits the path's node `node` (in `states` if it's
    /// the last one).
    fn fits(&self, node: &Node, states: u8) -> bool {
        self.name.as_ref().is_none_or(|n| *n == node.name)
            && self.classes.iter().all(|c| node.classes.contains(c))
            && self.states & !states == 0
    }

    fn specificity(&self) -> u32 {
        let names = self.name.is_some() as u32;
        let classes = (self.classes.len() + self.states.count_ones() as usize + self.root as usize) as u32;
        classes * 256 + names
    }
}

#[derive(Clone, Debug)]
struct Selector {
    /// Ancestors first; the last one is the element itself.
    parts: Vec<Compound>,
    specificity: u32,
}

impl Selector {
    fn parse(s: &str) -> Result<Selector, String> {
        let parts = s.split_whitespace().map(Compound::parse).collect::<Result<Vec<_>, _>>()?;
        if parts.is_empty() {
            return Err(tr("пустой селектор".into(), "an empty selector".into()));
        }
        if parts.iter().any(|p| p.root) && (parts.len() > 1 || parts[0].name.is_some()) {
            return Err(tr(format!("«:root» бывает только сам по себе: «{s}»"), format!("\":root\" goes alone: \"{s}\"")));
        }
        let specificity = parts.iter().map(Compound::specificity).sum();
        Ok(Selector { parts, specificity })
    }

    /// Matches `path`, whose last node is in `states` (ancestors' states
    /// aren't known: a part with a state only fits the element itself).
    fn matches(&self, path: &[Node], states: u8) -> bool {
        let Some((last, ancestors)) = self.parts.split_last() else { return false };
        if last.root {
            return path.len() == 1;
        }
        let Some((node, mut above)) = path.split_last() else { return false };
        if !last.fits(node, states) {
            return false;
        }
        // the rest, right to left, each somewhere further up
        'parts: for part in ancestors.iter().rev() {
            while let Some((n, up)) = above.split_last() {
                above = up;
                if part.fits(n, 0) {
                    continue 'parts;
                }
            }
            return false;
        }
        true
    }
}

#[derive(Clone, Debug)]
struct Rule {
    selectors: Vec<Selector>,
    decls: Vec<(String, String)>,
}

/// A parsed sheet, and what was wrong in it.
#[derive(Clone, Debug, Default)]
pub struct Sheet {
    rules: Vec<Rule>,
    pub errors: Vec<String>,
}

impl Sheet {
    /// Reads `src`; `name` goes in front of each error ("theme.css: line 4: …").
    pub fn parse(src: &str, name: &str) -> Sheet {
        let mut sheet = Sheet::default();
        let src = strip_comments(src);
        let line_at = |i: usize| src[..i].matches('\n').count() + 1;
        let mut err = |at: usize, msg: String| sheet.errors.push(tr(format!("{name}: строка {}: {msg}", line_at(at)), format!("{name}: line {}: {msg}", line_at(at))));
        let mut i = 0;
        let bytes = src.as_bytes();
        while i < bytes.len() {
            let Some(open) = src[i..].find('{').map(|o| i + o) else {
                if !src[i..].trim().is_empty() {
                    err(i, tr("правило без «{ }»".into(), "a rule without \"{ }\"".into()));
                }
                break;
            };
            // the block, nested braces and all (an @media is skipped whole)
            let (mut depth, mut close) = (0, None);
            for (k, b) in bytes.iter().enumerate().skip(open) {
                match b {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(k);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(close) = close else {
                err(open, tr("не закрыта «{»".into(), "\"{\" isn't closed".into()));
                break;
            };
            let head = src[i..open].trim();
            let body = &src[open + 1..close];
            let head_at = i + src[i..open].find(|c: char| !c.is_whitespace()).unwrap_or(0);
            i = close + 1;
            if head.starts_with('@') {
                let at_rule = head.split_whitespace().next().unwrap_or(head);
                err(head_at, tr(format!("«{at_rule}» не поддерживается"), format!("\"{at_rule}\" isn't supported")));
                continue;
            }
            let mut selectors = Vec::new();
            for s in head.split(',') {
                match Selector::parse(s.trim()) {
                    Ok(sel) => selectors.push(sel),
                    Err(e) => err(head_at, e),
                }
            }
            let mut decls = Vec::new();
            for d in body.split(';') {
                let d = d.trim();
                if d.is_empty() {
                    continue;
                }
                let Some((prop, value)) = d.split_once(':') else {
                    err(open, tr(format!("нет «:» в «{d}»"), format!("no \":\" in \"{d}\"")));
                    continue;
                };
                let prop = prop.trim().to_ascii_lowercase();
                let value = value.trim().trim_end_matches("!important").trim().to_string();
                if !prop.starts_with("--") && !PROPERTIES.contains(&prop.as_str()) {
                    err(open, tr(format!("неизвестное свойство «{prop}»"), format!("unknown property \"{prop}\"")));
                    continue;
                }
                if value.is_empty() {
                    err(open, tr(format!("пустое значение у «{prop}»"), format!("\"{prop}\" has no value")));
                    continue;
                }
                decls.push((prop, value));
            }
            if !selectors.is_empty() {
                sheet.rules.push(Rule { selectors, decls });
            }
        }
        sheet
    }

    /// This sheet, then `other` on top (its rules come later).
    pub fn then(mut self, other: Sheet) -> Sheet {
        self.rules.extend(other.rules);
        self.errors.extend(other.errors);
        self
    }

    /// What `path` (a window's element down to the one asked for) looks
    /// like in `states`: its declarations, `var()`s resolved.
    pub fn compute(&self, path: &[Node], states: u8) -> Computed {
        let mut vars: HashMap<String, String> = HashMap::new();
        let mut props: HashMap<String, String> = HashMap::new();
        // `:root` first, then each node down the path; variables carry
        // over from the nodes above, other properties belong to the last
        for depth in 0..=path.len() {
            let (sub, st, last) = if depth == 0 { (&path[..0], 0, false) } else { (&path[..depth], if depth == path.len() { states } else { 0 }, depth == path.len()) };
            let mut hits: Vec<(u32, usize, &(String, String))> = Vec::new();
            for (order, rule) in self.rules.iter().enumerate() {
                let best = rule
                    .selectors
                    .iter()
                    .filter(|s| if depth == 0 { s.parts[0].root } else { !s.parts[0].root && s.matches(sub, st) })
                    .map(|s| s.specificity)
                    .max();
                if let Some(spec) = best {
                    hits.extend(rule.decls.iter().map(|d| (spec, order, d)));
                }
            }
            hits.sort_by_key(|h| (h.0, h.1));
            for (_, _, (prop, value)) in hits {
                if prop.starts_with("--") {
                    vars.insert(prop.clone(), value.clone());
                } else if last {
                    props.insert(prop.clone(), value.clone());
                }
            }
        }
        // a root path ("overlay" alone) still has its own properties
        let resolved = |v: &str| resolve(v, &vars, 0);
        Computed {
            props: props.iter().map(|(k, v)| (k.clone(), resolved(v))).collect(),
            vars: vars.iter().map(|(k, v)| (k.clone(), resolved(v))).collect(),
        }
    }
}

/// The properties an element can have; anything else is reported.
pub const PROPERTIES: &[&str] = &[
    "background",
    "background-color",
    "color",
    "border",
    "border-color",
    "border-width",
    "border-radius",
    "font-size",
    "opacity",
    "box-shadow",
];

/// A node of a path: `button.icon.ghost` → name "button", classes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Node {
    pub name: String,
    pub classes: Vec<String>,
}

impl Node {
    pub fn parse(s: &str) -> Node {
        let mut it = s.split('.');
        Node { name: it.next().unwrap_or("").to_string(), classes: it.filter(|c| !c.is_empty()).map(str::to_string).collect() }
    }
}

/// Space-separated nodes: "toolbar button.icon".
pub fn path(s: &str) -> Vec<Node> {
    s.split_whitespace().map(Node::parse).collect()
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(a) = rest.find("/*") {
        out.push_str(&rest[..a]);
        match rest[a + 2..].find("*/") {
            Some(b) => {
                // keep the lines, for the errors' line numbers
                out.extend(rest[a..a + 2 + b + 2].chars().filter(|&c| c == '\n'));
                rest = &rest[a + 2 + b + 2..];
            }
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// `var(--x)` and `var(--x, fallback)` replaced, nested ones too.
fn resolve(v: &str, vars: &HashMap<String, String>, depth: u32) -> String {
    if depth > 8 || !v.contains("var(") {
        return v.to_string();
    }
    let mut out = String::new();
    let mut rest = v;
    while let Some(a) = rest.find("var(") {
        out.push_str(&rest[..a]);
        let body_at = a + 4;
        let mut level = 1;
        let mut end = None;
        for (k, ch) in rest[body_at..].char_indices() {
            match ch {
                '(' => level += 1,
                ')' => {
                    level -= 1;
                    if level == 0 {
                        end = Some(body_at + k);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            out.push_str(&rest[a..]);
            return out;
        };
        let body = &rest[body_at..end];
        let (name, fallback) = match body.split_once(',') {
            Some((n, f)) => (n.trim(), Some(f.trim())),
            None => (body.trim(), None),
        };
        let value = vars.get(name).map(String::as_str).or(fallback).unwrap_or("");
        out.push_str(&resolve(value, vars, depth + 1));
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// An element's declarations, resolved.
#[derive(Clone, Debug, Default)]
pub struct Computed {
    pub props: HashMap<String, String>,
    /// `--variables` as seen at the element.
    pub vars: HashMap<String, String>,
}

/// What the background is: a color, the frosted glass, nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    Color(Color32),
    Glass,
}

/// A shadow under a panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub color: Color32,
}

/// How an element looks, as far as the sheet says: what it doesn't say is
/// None and the element keeps its own look.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub background: Option<Fill>,
    pub color: Option<Color32>,
    pub border: Option<Stroke>,
    pub radius: Option<f32>,
    pub font_size: Option<f32>,
    pub opacity: Option<f32>,
    /// `Some(None)`: `box-shadow: none`.
    pub shadow: Option<Option<Shadow>>,
}

impl Look {
    pub fn of(c: &Computed) -> Look {
        let get = |k: &str| c.props.get(k).map(String::as_str);
        let mut look = Look::default();
        if let Some(v) = get("background").or(get("background-color")) {
            look.background = match v.trim() {
                "glass" => Some(Fill::Glass),
                "none" => Some(Fill::Color(Color32::TRANSPARENT)),
                v => color(v).map(Fill::Color),
            };
        }
        look.color = get("color").and_then(color);
        if let Some(v) = get("border") {
            look.border = border(v);
        }
        if let Some(w) = get("border-width").and_then(length) {
            let b = look.border.get_or_insert(Stroke::new(w, Color32::TRANSPARENT));
            b.width = w;
        }
        if let Some(col) = get("border-color").and_then(color) {
            let b = look.border.get_or_insert(Stroke::new(1.0, col));
            b.color = col;
        }
        look.radius = get("border-radius").and_then(length);
        look.font_size = get("font-size").and_then(length);
        look.opacity = get("opacity").and_then(|v| {
            let v = v.trim();
            match v.strip_suffix('%') {
                Some(p) => p.trim().parse::<f32>().ok().map(|p| p / 100.0),
                None => v.parse::<f32>().ok(),
            }
            .map(|o| o.clamp(0.0, 1.0))
        });
        look.shadow = get("box-shadow").map(shadow);
        look
    }

    pub fn bg_or(&self, default: Color32) -> Color32 {
        match self.background {
            Some(Fill::Color(c)) => c,
            _ => default,
        }
    }

    pub fn fg_or(&self, default: Color32) -> Color32 {
        self.color.unwrap_or(default)
    }

    pub fn border_or(&self, default: Stroke) -> Stroke {
        self.border.unwrap_or(default)
    }

    pub fn radius_or(&self, default: f32) -> f32 {
        self.radius.unwrap_or(default)
    }

    pub fn glass(&self) -> bool {
        self.background == Some(Fill::Glass)
    }
}

/// `12px`, `12`, `0.5em` (×13 px): points.
pub fn length(v: &str) -> Option<f32> {
    let v = v.trim();
    let (num, k) = if let Some(n) = v.strip_suffix("px") {
        (n, 1.0)
    } else if let Some(n) = v.strip_suffix("rem").or(v.strip_suffix("em")) {
        (n, 13.0)
    } else {
        (v, 1.0)
    };
    num.trim().parse::<f32>().ok().filter(|x| x.is_finite() && *x >= 0.0).map(|x| (x * k).min(400.0))
}

/// `1px solid #333`, `none`, `2px var(--accent)`.
fn border(v: &str) -> Option<Stroke> {
    if v.trim() == "none" || v.trim() == "0" {
        return Some(Stroke::NONE);
    }
    let mut width = 1.0;
    let mut col = None;
    for word in split_words(v) {
        if let Some(w) = length(&word) {
            width = w;
        } else if let Some(c) = color(&word) {
            col = Some(c);
        }
    }
    col.map(|c| Stroke::new(width, c))
}

/// `0 8px 24px rgba(0,0,0,.4)`, `none`.
fn shadow(v: &str) -> Option<Shadow> {
    if v.trim() == "none" {
        return None;
    }
    let mut nums = Vec::new();
    let mut col = Color32::from_black_alpha(90);
    for word in split_words(v) {
        if let Some(c) = color(&word) {
            col = c;
        } else if let Ok(n) = word.trim_end_matches("px").parse::<f32>() {
            nums.push(n);
        }
    }
    Some(Shadow { x: *nums.first().unwrap_or(&0.0), y: *nums.get(1).unwrap_or(&8.0), blur: *nums.get(2).unwrap_or(&24.0), color: col })
}

/// Words split at spaces outside parentheses: `1px solid rgb(1, 2, 3)`.
fn split_words(v: &str) -> Vec<String> {
    let (mut out, mut cur, mut level) = (Vec::new(), String::new(), 0);
    for ch in v.chars() {
        match ch {
            '(' => level += 1,
            ')' => level -= 1,
            _ => {}
        }
        if ch.is_whitespace() && level == 0 {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb(…)`, `rgba(…)`, `hsl(…)`,
/// a few names. Straight (unpremultiplied) alpha, as in CSS.
pub fn color(v: &str) -> Option<Color32> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(hex) = v.strip_prefix('#') {
        let d: Vec<u8> = hex.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
        let (r, g, b, a) = match d.len() {
            3 => (d[0] * 17, d[1] * 17, d[2] * 17, 255),
            4 => (d[0] * 17, d[1] * 17, d[2] * 17, d[3] * 17),
            6 => (d[0] * 16 + d[1], d[2] * 16 + d[3], d[4] * 16 + d[5], 255),
            8 => (d[0] * 16 + d[1], d[2] * 16 + d[3], d[4] * 16 + d[5], d[6] * 16 + d[7]),
            _ => return None,
        };
        return Some(Color32::from_rgba_unmultiplied(r, g, b, a));
    }
    if let Some((f, args)) = v.split_once('(') {
        let args = args.strip_suffix(')')?;
        // "1, 2, 3, .5" and "1 2 3 / 50%"
        let parts: Vec<&str> = args.split([',', ' ', '/']).map(str::trim).filter(|s| !s.is_empty()).collect();
        let num = |s: &str, full: f32| -> Option<f32> {
            match s.strip_suffix('%') {
                Some(p) => p.parse::<f32>().ok().map(|p| p / 100.0 * full),
                None => s.trim_end_matches("deg").parse::<f32>().ok(),
            }
        };
        let alpha = |i: usize| parts.get(i).map_or(Some(1.0), |s| num(s, 1.0)).map(|a| (a.clamp(0.0, 1.0) * 255.0).round() as u8);
        return match f.trim() {
            "rgb" | "rgba" if parts.len() >= 3 => {
                let c = |i: usize| num(parts[i], 255.0).map(|x| x.round().clamp(0.0, 255.0) as u8);
                Some(Color32::from_rgba_unmultiplied(c(0)?, c(1)?, c(2)?, alpha(3)?))
            }
            "hsl" | "hsla" if parts.len() >= 3 => {
                let (h, s, l) = (num(parts[0], 360.0)?, num(parts[1], 1.0)?, num(parts[2], 1.0)?);
                let (r, g, b) = hsl(h, if s > 1.0 { s / 100.0 } else { s }, if l > 1.0 { l / 100.0 } else { l });
                Some(Color32::from_rgba_unmultiplied(r, g, b, alpha(3)?))
            }
            _ => None,
        };
    }
    Some(match v.as_str() {
        "transparent" => Color32::TRANSPARENT,
        "white" => Color32::WHITE,
        "black" => Color32::BLACK,
        "red" => Color32::from_rgb(0xE5, 0x48, 0x4D),
        "green" => Color32::from_rgb(0x46, 0xA7, 0x58),
        "blue" => Color32::from_rgb(0x3E, 0x63, 0xDD),
        "yellow" => Color32::from_rgb(0xF5, 0xD9, 0x0A),
        "orange" => Color32::from_rgb(0xF7, 0x6B, 0x15),
        "purple" => Color32::from_rgb(0x8E, 0x4E, 0xC6),
        "pink" => Color32::from_rgb(0xD6, 0x40, 0x9F),
        "gray" | "grey" => Color32::from_rgb(0x80, 0x80, 0x80),
        _ => return None,
    })
}

fn hsl(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let k = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (k(r), k(g), k(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(sheet: &Sheet, p: &str, st: u8) -> Look {
        Look::of(&sheet.compute(&path(p), st))
    }

    #[test]
    fn cascade() {
        let s = Sheet::parse(
            "
            /* a comment { with braces } */
            :root { --accent: #ff0000; --r: 6px }
            button { background: #111; border-radius: var(--r) }
            toolbar button { background: #222 }
            button.ghost:hover { background: var(--accent) }
            overlay { --accent: rgb(0 128 255 / 50%) }
            ",
            "t",
        );
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        let c = |r, g, b| Some(Fill::Color(Color32::from_rgb(r, g, b)));
        assert_eq!(look(&s, "settings button", 0).background, c(0x11, 0x11, 0x11));
        assert_eq!(look(&s, "settings button", 0).radius, Some(6.0));
        assert_eq!(look(&s, "overlay toolbar button.ghost", 0).background, c(0x22, 0x22, 0x22));
        // :hover and the class outweigh the ancestor
        assert_eq!(look(&s, "settings button.ghost", state::HOVER).background, c(0xff, 0, 0));
        assert_eq!(
            look(&s, "overlay toolbar button.ghost", state::HOVER).background,
            Some(Fill::Color(Color32::from_rgba_unmultiplied(0, 128, 255, 128)))
        );
        // a property of an ancestor isn't inherited
        assert_eq!(look(&s, "overlay toolbar", 0).background, None);
    }

    #[test]
    fn errors_and_values() {
        let s = Sheet::parse("button { colr: red; color: red }\n\n@media x { a { color: red } }\nbutton:hovr { color: red }", "theme.css");
        assert_eq!(s.errors.len(), 3, "{:?}", s.errors);
        assert!(s.errors[0].contains("строка 1"));
        assert!(s.errors[1].contains("строка 3"));
        assert_eq!(color("#fff8"), Some(Color32::from_rgba_unmultiplied(255, 255, 255, 136)));
        assert_eq!(color("hsl(0, 100%, 50%)"), Some(Color32::from_rgb(255, 0, 0)));
        assert_eq!(border("2px solid #000"), Some(Stroke::new(2.0, Color32::BLACK)));
        assert_eq!(length("1.5em"), Some(19.5));
    }
}

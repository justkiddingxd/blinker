//! Hotkey strings like "Ctrl+Shift+PrintScreen" <-> (modifiers, virtual key).
//! Keys are Windows virtual-key codes on every platform (macOS maps them to
//! its own key codes only when registering), so config files stay portable.

#[cfg(target_os = "macos")]
use crate::win::vk::*;
#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Combo {
    pub mods: u32,
    pub vk: u32,
}

#[cfg(windows)]
const MODS: [(u32, &str); 4] = [(MOD_CONTROL, "Ctrl"), (MOD_ALT, "Alt"), (MOD_SHIFT, "Shift"), (MOD_WIN, "Win")];
/// macOS order and names: ⌃ ⌥ ⇧ ⌘ (`Win` / `Alt` are still read, see `parse`).
#[cfg(target_os = "macos")]
const MODS: [(u32, &str); 4] = [(MOD_CONTROL, "Ctrl"), (MOD_ALT, "Opt"), (MOD_SHIFT, "Shift"), (MOD_WIN, "Cmd")];

/// Other spellings of the modifiers (configs written on another system).
const MOD_ALIASES: [(u32, &str); 6] =
    [(MOD_CONTROL, "Control"), (MOD_ALT, "Alt"), (MOD_ALT, "Option"), (MOD_ALT, "Opt"), (MOD_WIN, "Win"), (MOD_WIN, "Cmd")];

fn key_name(vk: u32) -> Option<String> {
    let named = match vk as u16 {
        VK_SNAPSHOT => "PrintScreen",
        VK_PAUSE => "Pause",
        VK_SCROLL => "ScrollLock",
        VK_INSERT => "Insert",
        VK_DELETE => "Delete",
        VK_HOME => "Home",
        VK_END => "End",
        VK_PRIOR => "PageUp",
        VK_NEXT => "PageDown",
        VK_LEFT => "Left",
        VK_RIGHT => "Right",
        VK_UP => "Up",
        VK_DOWN => "Down",
        VK_SPACE => "Space",
        VK_TAB => "Tab",
        VK_RETURN => "Enter",
        VK_BACK => "Backspace",
        VK_ESCAPE => "Esc",
        VK_OEM_3 => "`",
        VK_OEM_MINUS => "-",
        VK_OEM_PLUS => "=",
        VK_OEM_4 => "[",
        VK_OEM_6 => "]",
        VK_OEM_1 => ";",
        VK_OEM_7 => "'",
        VK_OEM_COMMA => ",",
        VK_OEM_PERIOD => ".",
        VK_OEM_2 => "/",
        VK_OEM_5 => "\\",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.into());
    }
    match vk {
        0x30..=0x39 | 0x41..=0x5A => Some(char::from_u32(vk).unwrap().to_string()),
        v if (VK_F1 as u32..=VK_F24 as u32).contains(&v) => Some(format!("F{}", v - VK_F1 as u32 + 1)),
        v if (VK_NUMPAD0 as u32..=VK_NUMPAD9 as u32).contains(&v) => {
            Some(format!("Num{}", v - VK_NUMPAD0 as u32))
        }
        _ => None,
    }
}

fn key_from_name(name: &str) -> Option<u32> {
    // Brute-force the reverse mapping; the table is tiny.
    (1u32..=0xFE).find(|&vk| key_name(vk).is_some_and(|n| n.eq_ignore_ascii_case(name)))
}

pub fn parse(s: &str) -> Option<Combo> {
    let mut mods = 0;
    let mut vk = None;
    for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some((m, _)) = MODS.iter().chain(MOD_ALIASES.iter()).find(|(_, n)| n.eq_ignore_ascii_case(part)) {
            mods |= m;
        } else if part.eq_ignore_ascii_case("command") || part.eq_ignore_ascii_case("super") {
            mods |= MOD_WIN;
        } else {
            vk = Some(key_from_name(part)?);
        }
    }
    vk.map(|vk| Combo { mods, vk })
}

pub fn format(c: Combo) -> String {
    let mut parts: Vec<String> =
        MODS.iter().filter(|(m, _)| c.mods & m != 0).map(|(_, n)| n.to_string()).collect();
    parts.push(key_name(c.vk).unwrap_or_else(|| format!("0x{:02X}", c.vk)));
    parts.join("+")
}

/// Short labels for keycaps.
pub fn display_parts(s: &str) -> Vec<String> {
    s.split('+')
        .filter(|p| !p.is_empty())
        .map(|p| match p {
            "PrintScreen" => "prtsc".to_string(),
            "PageUp" => "pgup".to_string(),
            "PageDown" => "pgdn".to_string(),
            other => other.to_lowercase(),
        })
        .collect()
}

/// Polls the physical keyboard; returns a combo once a non-modifier key is down.
/// Used by the hotkey recorder (global hotkeys are suspended meanwhile).
#[cfg(target_os = "macos")]
pub fn poll_pressed() -> Option<Combo> {
    // AppKit hands the key presses of our own (focused) window to a local
    // monitor: no keyboard-wide polling, so no input-monitoring permission.
    let (mods, vk) = crate::win::take_key_press()?;
    key_name(vk).map(|_| Combo { mods, vk })
}

#[cfg(windows)]
pub fn poll_pressed() -> Option<Combo> {
    use crate::win::key_down;
    let modifier_vks = [
        VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN, VK_LSHIFT, VK_RSHIFT, VK_LCONTROL,
        VK_RCONTROL, VK_LMENU, VK_RMENU,
    ];
    for vk in 0x08u32..=0xFE {
        if modifier_vks.contains(&(vk as u16)) || key_name(vk).is_none() {
            continue;
        }
        if key_down(vk) {
            let mut mods = 0;
            if key_down(VK_CONTROL as u32) {
                mods |= MOD_CONTROL;
            }
            if key_down(VK_MENU as u32) {
                mods |= MOD_ALT;
            }
            if key_down(VK_SHIFT as u32) {
                mods |= MOD_SHIFT;
            }
            if key_down(VK_LWIN as u32) || key_down(VK_RWIN as u32) {
                mods |= MOD_WIN;
            }
            return Some(Combo { mods, vk });
        }
    }
    None
}

/// Whether `binding` was pressed in this frame of an egui window (the keys
/// of a capture's actions, read by the overlay itself).
pub fn pressed(input: &eframe::egui::InputState, binding: &str) -> bool {
    use eframe::egui::{Event, Key};
    let Some(c) = parse(binding) else { return false };
    let Some(name) = key_name(c.vk) else { return false };
    let name = match name.strip_prefix("Num") {
        Some(d) if d.len() == 1 => format!("Numpad{d}"),
        _ => name,
    };
    let Some(key) = Key::from_name(&name) else { return false };
    let want = |m: u32| c.mods & m != 0;
    // the Windows key never reaches egui
    if cfg!(windows) && want(MOD_WIN) {
        return false;
    }
    let m = input.modifiers;
    let cmd = if cfg!(target_os = "macos") { m.mac_cmd == want(MOD_WIN) } else { true };
    if m.alt == want(MOD_ALT) && m.shift == want(MOD_SHIFT) && m.ctrl == want(MOD_CONTROL) && cmd && input.key_pressed(key) {
        return true;
    }
    // ctrl / cmd + c, x, v reach egui as copy, cut and paste events, before
    // (or instead of) the key press
    let command = if cfg!(target_os = "macos") { c.mods == MOD_WIN } else { c.mods == MOD_CONTROL };
    command
        && input.events.iter().any(|e| {
            matches!((key, e), (Key::C, Event::Copy) | (Key::X, Event::Cut) | (Key::V, Event::Paste(_)))
        })
}

pub fn is_backspace(c: Combo) -> bool {
    c.mods == 0 && c.vk == VK_BACK as u32
}

pub fn is_escape(c: Combo) -> bool {
    c.mods == 0 && c.vk == VK_ESCAPE as u32
}

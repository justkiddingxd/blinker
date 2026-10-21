//! Windows virtual-key codes (blinker's portable key ids, see `hotkey`) and
//! their macOS key codes. Letters and digits are key positions (ANSI), the
//! way macOS shortcuts work too.

#![allow(dead_code)]

pub const VK_BACK: u16 = 0x08;
pub const VK_TAB: u16 = 0x09;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;
pub const VK_PAUSE: u16 = 0x13;
pub const VK_ESCAPE: u16 = 0x1B;
pub const VK_SPACE: u16 = 0x20;
pub const VK_PRIOR: u16 = 0x21;
pub const VK_NEXT: u16 = 0x22;
pub const VK_END: u16 = 0x23;
pub const VK_HOME: u16 = 0x24;
pub const VK_LEFT: u16 = 0x25;
pub const VK_UP: u16 = 0x26;
pub const VK_RIGHT: u16 = 0x27;
pub const VK_DOWN: u16 = 0x28;
pub const VK_SNAPSHOT: u16 = 0x2C;
pub const VK_INSERT: u16 = 0x2D;
pub const VK_DELETE: u16 = 0x2E;
pub const VK_LWIN: u16 = 0x5B;
pub const VK_RWIN: u16 = 0x5C;
pub const VK_NUMPAD0: u16 = 0x60;
pub const VK_NUMPAD9: u16 = 0x69;
pub const VK_F1: u16 = 0x70;
pub const VK_F24: u16 = 0x87;
pub const VK_SCROLL: u16 = 0x91;
pub const VK_LSHIFT: u16 = 0xA0;
pub const VK_RSHIFT: u16 = 0xA1;
pub const VK_LCONTROL: u16 = 0xA2;
pub const VK_RCONTROL: u16 = 0xA3;
pub const VK_LMENU: u16 = 0xA4;
pub const VK_RMENU: u16 = 0xA5;
pub const VK_OEM_1: u16 = 0xBA;
pub const VK_OEM_PLUS: u16 = 0xBB;
pub const VK_OEM_COMMA: u16 = 0xBC;
pub const VK_OEM_MINUS: u16 = 0xBD;
pub const VK_OEM_PERIOD: u16 = 0xBE;
pub const VK_OEM_2: u16 = 0xBF;
pub const VK_OEM_3: u16 = 0xC0;
pub const VK_OEM_4: u16 = 0xDB;
pub const VK_OEM_5: u16 = 0xDC;
pub const VK_OEM_6: u16 = 0xDD;
pub const VK_OEM_7: u16 = 0xDE;

pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
/// ⌘ (the Windows key's place).
pub const MOD_WIN: u32 = 0x8;

/// (virtual key, macOS key code)
const TABLE: &[(u16, u16)] = &[
    (b'A' as u16, 0x00),
    (b'S' as u16, 0x01),
    (b'D' as u16, 0x02),
    (b'F' as u16, 0x03),
    (b'H' as u16, 0x04),
    (b'G' as u16, 0x05),
    (b'Z' as u16, 0x06),
    (b'X' as u16, 0x07),
    (b'C' as u16, 0x08),
    (b'V' as u16, 0x09),
    (b'B' as u16, 0x0B),
    (b'Q' as u16, 0x0C),
    (b'W' as u16, 0x0D),
    (b'E' as u16, 0x0E),
    (b'R' as u16, 0x0F),
    (b'Y' as u16, 0x10),
    (b'T' as u16, 0x11),
    (b'1' as u16, 0x12),
    (b'2' as u16, 0x13),
    (b'3' as u16, 0x14),
    (b'4' as u16, 0x15),
    (b'6' as u16, 0x16),
    (b'5' as u16, 0x17),
    (VK_OEM_PLUS, 0x18),
    (b'9' as u16, 0x19),
    (b'7' as u16, 0x1A),
    (VK_OEM_MINUS, 0x1B),
    (b'8' as u16, 0x1C),
    (b'0' as u16, 0x1D),
    (VK_OEM_6, 0x1E),
    (b'O' as u16, 0x1F),
    (b'U' as u16, 0x20),
    (VK_OEM_4, 0x21),
    (b'I' as u16, 0x22),
    (b'P' as u16, 0x23),
    (VK_RETURN, 0x24),
    (b'L' as u16, 0x25),
    (b'J' as u16, 0x26),
    (VK_OEM_7, 0x27),
    (b'K' as u16, 0x28),
    (VK_OEM_1, 0x29),
    (VK_OEM_5, 0x2A),
    (VK_OEM_COMMA, 0x2B),
    (VK_OEM_2, 0x2C),
    (b'N' as u16, 0x2D),
    (b'M' as u16, 0x2E),
    (VK_OEM_PERIOD, 0x2F),
    (VK_TAB, 0x30),
    (VK_SPACE, 0x31),
    (VK_OEM_3, 0x32),
    (VK_BACK, 0x33),
    (VK_ESCAPE, 0x35),
    (VK_RWIN, 0x36),
    (VK_LWIN, 0x37),
    (VK_SHIFT, 0x38),
    (VK_MENU, 0x3A),
    (VK_CONTROL, 0x3B),
    (VK_RSHIFT, 0x3C),
    (VK_RMENU, 0x3D),
    (VK_RCONTROL, 0x3E),
    (VK_NUMPAD0, 0x52),
    (VK_NUMPAD0 + 1, 0x53),
    (VK_NUMPAD0 + 2, 0x54),
    (VK_NUMPAD0 + 3, 0x55),
    (VK_NUMPAD0 + 4, 0x56),
    (VK_NUMPAD0 + 5, 0x57),
    (VK_NUMPAD0 + 6, 0x58),
    (VK_NUMPAD0 + 7, 0x59),
    (VK_NUMPAD0 + 8, 0x5B),
    (VK_NUMPAD0 + 9, 0x5C),
    (VK_F1 + 4, 0x60),
    (VK_F1 + 5, 0x61),
    (VK_F1 + 6, 0x62),
    (VK_F1 + 2, 0x63),
    (VK_F1 + 7, 0x64),
    (VK_F1 + 8, 0x65),
    (VK_F1 + 10, 0x67),
    // PC keyboards on a Mac: PrtSc / ScrLk / Pause arrive as F13 / F14 / F15
    (VK_SNAPSHOT, 0x69),
    (VK_F1 + 12, 0x69),
    (VK_SCROLL, 0x6B),
    (VK_F1 + 13, 0x6B),
    (VK_F1 + 15, 0x6A),
    (VK_F1 + 9, 0x6D),
    (VK_F1 + 11, 0x6F),
    (VK_PAUSE, 0x71),
    (VK_F1 + 14, 0x71),
    (VK_INSERT, 0x72),
    (VK_HOME, 0x73),
    (VK_PRIOR, 0x74),
    (VK_DELETE, 0x75),
    (VK_F1 + 3, 0x76),
    (VK_END, 0x77),
    (VK_F1 + 1, 0x78),
    (VK_NEXT, 0x79),
    (VK_F1, 0x7A),
    (VK_LEFT, 0x7B),
    (VK_RIGHT, 0x7C),
    (VK_DOWN, 0x7D),
    (VK_UP, 0x7E),
    (VK_F1 + 16, 0x40),
    (VK_F1 + 17, 0x4F),
    (VK_F1 + 18, 0x50),
    (VK_F1 + 19, 0x5A),
];

pub fn to_keycode(vk: u32) -> Option<u16> {
    TABLE.iter().find(|(v, _)| *v as u32 == vk).map(|(_, k)| *k)
}

/// The first (canonical) virtual key for a macOS key code.
pub fn from_keycode(code: u16) -> Option<u32> {
    TABLE.iter().find(|(_, k)| *k == code).map(|(v, _)| *v as u32)
}

pub const FLAG_SHIFT: u64 = 1 << 17;
pub const FLAG_CONTROL: u64 = 1 << 18;
pub const FLAG_OPTION: u64 = 1 << 19;
pub const FLAG_COMMAND: u64 = 1 << 20;

/// NSEvent / CGEvent modifier flags -> MOD_*.
pub fn mods_from_flags(flags: u64) -> u32 {
    let mut m = 0;
    if flags & FLAG_SHIFT != 0 {
        m |= MOD_SHIFT;
    }
    if flags & FLAG_CONTROL != 0 {
        m |= MOD_CONTROL;
    }
    if flags & FLAG_OPTION != 0 {
        m |= MOD_ALT;
    }
    if flags & FLAG_COMMAND != 0 {
        m |= MOD_WIN;
    }
    m
}

/// MOD_* -> NSEvent modifier flags (menu key equivalents).
pub fn flags_from_mods(mods: u32) -> u64 {
    let mut f = 0;
    if mods & MOD_SHIFT != 0 {
        f |= FLAG_SHIFT;
    }
    if mods & MOD_CONTROL != 0 {
        f |= FLAG_CONTROL;
    }
    if mods & MOD_ALT != 0 {
        f |= FLAG_OPTION;
    }
    if mods & MOD_WIN != 0 {
        f |= FLAG_COMMAND;
    }
    f
}

/// MOD_* -> Carbon hotkey modifiers.
pub fn carbon_mods(mods: u32) -> u32 {
    let mut m = 0;
    if mods & MOD_WIN != 0 {
        m |= 1 << 8; // cmdKey
    }
    if mods & MOD_SHIFT != 0 {
        m |= 1 << 9; // shiftKey
    }
    if mods & MOD_ALT != 0 {
        m |= 1 << 11; // optionKey
    }
    if mods & MOD_CONTROL != 0 {
        m |= 1 << 12; // controlKey
    }
    m
}

/// The character AppKit shows for a key in menus (key equivalent).
pub fn key_equivalent(vk: u32) -> Option<String> {
    let c = match vk as u16 {
        v @ (0x30..=0x39 | 0x41..=0x5A) => (v as u8 as char).to_ascii_lowercase(),
        VK_OEM_1 => ';',
        VK_OEM_PLUS => '=',
        VK_OEM_COMMA => ',',
        VK_OEM_MINUS => '-',
        VK_OEM_PERIOD => '.',
        VK_OEM_2 => '/',
        VK_OEM_3 => '`',
        VK_OEM_4 => '[',
        VK_OEM_5 => '\\',
        VK_OEM_6 => ']',
        VK_OEM_7 => '\'',
        VK_SPACE => ' ',
        VK_RETURN => '\r',
        VK_ESCAPE => '\u{1b}',
        VK_TAB => '\t',
        VK_LEFT => '\u{F702}',
        VK_RIGHT => '\u{F703}',
        VK_UP => '\u{F700}',
        VK_DOWN => '\u{F701}',
        VK_HOME => '\u{F729}',
        VK_END => '\u{F72B}',
        VK_PRIOR => '\u{F72C}',
        VK_NEXT => '\u{F72D}',
        VK_DELETE => '\u{F728}',
        VK_SNAPSHOT => '\u{F710}', // F13
        v if (VK_F1..=VK_F24).contains(&v) => char::from_u32(0xF704 + (v - VK_F1) as u32)?,
        _ => return None,
    };
    Some(c.to_string())
}

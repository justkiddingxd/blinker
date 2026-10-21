//! Interface language. All text is lowercase by design. Russian strings live
//! in the code; English comes from `i18n_en.rs` (plain text, `t`) or sits
//! next to the Russian template (`lf!`, for text with values in it).

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

static EN: AtomicBool = AtomicBool::new(false);

pub fn set_english(on: bool) {
    EN.store(on, Ordering::Relaxed);
}

pub fn en() -> bool {
    EN.load(Ordering::Relaxed)
}

/// Language for a config value ("ru" / "en"; anything else = system language).
pub fn apply(lang: &str) {
    set_english(match lang {
        "ru" => false,
        "en" => true,
        _ => !system_is_russian(),
    });
}

/// Russian unless nothing on the system points to a Russian speaker: the UI
/// language, the regional format or any installed keyboard layout being
/// Russian, Ukrainian, Belarusian or Kazakh.
#[cfg(target_os = "macos")]
pub fn system_is_russian() -> bool {
    crate::win::system_is_russian()
}

#[cfg(windows)]
pub fn system_is_russian() -> bool {
    let ru = |lang: u32| matches!(lang & 0x3FF, 0x19 | 0x22 | 0x23 | 0x3F);
    unsafe {
        use windows_sys::Win32::Globalization::{GetUserDefaultLCID, GetUserDefaultUILanguage};
        if ru(GetUserDefaultUILanguage() as u32) || ru(GetUserDefaultLCID()) {
            return true;
        }
        let mut layouts = [std::ptr::null_mut(); 32];
        let n = windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayoutList(32, layouts.as_mut_ptr());
        layouts[..n.max(0) as usize].iter().any(|h| ru(*h as usize as u32 & 0xFFFF))
    }
}

fn table() -> &'static HashMap<&'static str, &'static str> {
    static T: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    T.get_or_init(|| {
        let pairs: &[(&str, &str)] = &include!("i18n_en.rs");
        pairs.iter().copied().collect()
    })
}

/// UI text: the Russian original, or its English version.
pub fn t(ru: &'static str) -> &'static str {
    if en() { table().get(ru).copied().unwrap_or(ru) } else { ru }
}

/// Shortcut hints: the editors' ⌘ shortcuts are written "ctrl" in the texts;
/// on macOS they read "cmd".
pub fn kb(text: &str) -> std::borrow::Cow<'_, str> {
    if cfg!(target_os = "macos") && text.contains("ctrl") {
        std::borrow::Cow::Owned(text.replace("ctrl", "cmd"))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// Inline pair for text without a dictionary entry.
pub fn l(ru: &'static str, english: &'static str) -> &'static str {
    if en() { english } else { ru }
}

/// `format!` with a Russian and an English template (same arguments).
macro_rules! lf {
    ($ru:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::en() { format!($en $(, $arg)*) } else { format!($ru $(, $arg)*) }
    };
}

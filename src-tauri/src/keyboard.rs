//! Hotkeys from the keyboard's side: the spelling the global shortcut plugin
//! reads, and making sure the hotkey was pressed on its own. A dictation
//! starts only when nothing but the hotkey's keys is held, so pressing it by
//! accident together with other keys or mouse buttons (e.g. W while gaming)
//! does nothing.

const VK_SHIFT: u8 = 0x10;
const VK_CONTROL: u8 = 0x11;
const VK_MENU: u8 = 0x12; // Alt
const VK_LWIN: u8 = 0x5B;
const VK_RWIN: u8 = 0x5C;
const VK_LSHIFT: u8 = 0xA0;
const VK_RSHIFT: u8 = 0xA1;
const VK_LCONTROL: u8 = 0xA2;
const VK_RCONTROL: u8 = 0xA3;
const VK_LMENU: u8 = 0xA4;
const VK_RMENU: u8 = 0xA5;

/// The keys a hotkey consists of.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HotkeyKeys {
    ctrl: bool,
    shift: bool,
    alt: bool,
    win: bool,
    /// Virtual-key codes of the non-modifier keys
    keys: Vec<u8>,
    /// Non-modifier keys without a known virtual-key code; that many other
    /// held keys are let through, as one of them is the hotkey's own
    unknown: usize,
}

impl HotkeyKeys {
    /// Parses a hotkey in the saved form, e.g. `Ctrl+Shift+Space` or `Alt+KeyD`.
    pub fn parse(hotkey: &str) -> Self {
        let mut parsed = Self::default();
        for part in hotkey.split('+').map(str::trim).filter(|part| !part.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmdorctrl" | "commandorcontrol" => parsed.ctrl = true,
                "shift" => parsed.shift = true,
                "alt" | "option" => parsed.alt = true,
                "super" | "meta" | "cmd" | "command" | "win" => parsed.win = true,
                _ => match key_code(part) {
                    Some(vk) => parsed.keys.push(vk),
                    None => parsed.unknown += 1,
                },
            }
        }
        parsed
    }

    /// Names of keys and mouse buttons held right now that aren't part of the
    /// hotkey. Empty when the hotkey was pressed on its own.
    pub fn extra_keys_held(&self) -> Vec<String> {
        self.extra_keys(&held_keys())
            .into_iter()
            .map(key_name)
            .collect()
    }

    /// `held` lists the virtual-key codes that are down.
    fn extra_keys(&self, held: &[u8]) -> Vec<u8> {
        let mut unknown_left = self.unknown;
        let mut extra = Vec::new();
        for &vk in held {
            let allowed = match vk {
                VK_SHIFT | VK_LSHIFT | VK_RSHIFT => self.shift,
                VK_CONTROL | VK_LCONTROL | VK_RCONTROL => self.ctrl,
                VK_MENU | VK_LMENU | VK_RMENU => self.alt,
                VK_LWIN | VK_RWIN => self.win,
                _ if self.keys.contains(&vk) => true,
                _ if unknown_left > 0 => {
                    unknown_left -= 1;
                    true
                }
                _ => false,
            };
            if !allowed {
                extra.push(vk);
            }
        }
        extra
    }
}

/// A hotkey spelled the way the global shortcut plugin reads it: modifiers
/// by one name each, a lone letter or digit as its key code ("ctrl+shift+a"
/// becomes "Ctrl+Shift+KeyA"). Other keys stay as they are.
pub fn canonical(hotkey: &str) -> String {
    const MODIFIERS: &[(&str, &[&str])] = &[
        ("Ctrl", &["ctrl", "control"]),
        ("Shift", &["shift"]),
        ("Alt", &["alt"]),
        ("Super", &["super", "meta", "cmd"]),
        ("CmdOrCtrl", &["cmdorctrl"]),
        ("Space", &["space"]),
    ];
    let spell = |part: &str| -> String {
        let mut chars = part.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            if ch.is_ascii_alphabetic() {
                return format!("Key{}", ch.to_ascii_uppercase());
            }
            if ch.is_ascii_digit() {
                return format!("Digit{}", ch);
            }
        }
        MODIFIERS
            .iter()
            .find(|(_, names)| names.iter().any(|name| part.eq_ignore_ascii_case(name)))
            .map_or_else(|| part.to_string(), |(canonical, _)| canonical.to_string())
    };
    hotkey
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(spell)
        .collect::<Vec<_>>()
        .join("+")
}

/// Virtual-key code for a key name as the settings store it (`KeyboardEvent.code`).
fn key_code(name: &str) -> Option<u8> {
    let single = |prefix: &str, base: u8, chars: std::ops::RangeInclusive<char>| {
        let rest = name.strip_prefix(prefix)?;
        let mut letters = rest.chars();
        let ch = letters.next()?.to_ascii_uppercase();
        if letters.next().is_none() && chars.contains(&ch) {
            Some(base + (ch as u8 - *chars.start() as u8))
        } else {
            None
        }
    };
    if let Some(vk) = single("Key", 0x41, 'A'..='Z')
        .or_else(|| single("Digit", 0x30, '0'..='9'))
        .or_else(|| single("Numpad", 0x60, '0'..='9'))
    {
        return Some(vk);
    }
    if let Some(number) = name.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
        return (1..=24).contains(&number).then(|| 0x6F + number);
    }

    let vk = match name {
        // A mouse button as the whole hotkey (mouse.rs)
        "MouseMiddle" => 0x04,
        "MouseBack" => 0x05,
        "MouseForward" => 0x06,
        "Space" => 0x20,
        "Enter" | "NumpadEnter" => 0x0D,
        "Tab" => 0x09,
        "Escape" => 0x1B,
        "Backspace" => 0x08,
        "CapsLock" => 0x14,
        "Pause" => 0x13,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "End" => 0x23,
        "Home" => 0x24,
        "ArrowLeft" => 0x25,
        "ArrowUp" => 0x26,
        "ArrowRight" => 0x27,
        "ArrowDown" => 0x28,
        "PrintScreen" => 0x2C,
        "Insert" => 0x2D,
        "Delete" => 0x2E,
        "ContextMenu" => 0x5D,
        "NumpadMultiply" => 0x6A,
        "NumpadAdd" => 0x6B,
        "NumpadSubtract" => 0x6D,
        "NumpadDecimal" => 0x6E,
        "NumpadDivide" => 0x6F,
        "NumLock" => 0x90,
        "ScrollLock" => 0x91,
        "Semicolon" => 0xBA,
        "Equal" => 0xBB,
        "Comma" => 0xBC,
        "Minus" => 0xBD,
        "Period" => 0xBE,
        "Slash" => 0xBF,
        "Backquote" => 0xC0,
        "BracketLeft" => 0xDB,
        "Backslash" => 0xDC,
        "BracketRight" => 0xDD,
        "Quote" => 0xDE,
        _ => return None,
    };
    Some(vk)
}

/// Readable name for logs.
fn key_name(vk: u8) -> String {
    match vk {
        0x01 => "left mouse button".to_string(),
        0x02 => "right mouse button".to_string(),
        0x04 => "middle mouse button".to_string(),
        0x05 | 0x06 => "side mouse button".to_string(),
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => "Shift".to_string(),
        VK_CONTROL | VK_LCONTROL | VK_RCONTROL => "Ctrl".to_string(),
        VK_MENU | VK_LMENU | VK_RMENU => "Alt".to_string(),
        VK_LWIN | VK_RWIN => "Win".to_string(),
        0x20 => "Space".to_string(),
        0x30..=0x39 | 0x41..=0x5A => (vk as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        _ => format!("key 0x{:02X}", vk),
    }
}

/// Keys that never count as held: reserved codes, IME state keys and
/// injected Unicode input.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn is_ignored(vk: u8) -> bool {
    matches!(
        vk,
        0x03 | 0x07 | 0x0A | 0x0B | 0x0E | 0x0F | 0x15..=0x1A | 0x1C..=0x1F | 0x5E | 0xE5 | 0xE7
    )
}

/// Virtual-key codes of all keys and mouse buttons that are down right now.
#[cfg(target_os = "windows")]
fn held_keys() -> Vec<u8> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    (0x01u8..=0xFE)
        .filter(|&vk| !is_ignored(vk))
        .filter(|&vk| {
            // The high bit is set while the key is down
            let state = unsafe { GetAsyncKeyState(vk as i32) };
            (state as u16) & 0x8000 != 0
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn held_keys() -> Vec<u8> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hotkey() {
        let keys = HotkeyKeys::parse("Ctrl+Shift+Space");
        assert!(keys.ctrl && keys.shift && !keys.alt && !keys.win);
        assert_eq!(keys.keys, vec![0x20]);
        assert_eq!(HotkeyKeys::parse("Alt+KeyD").keys, vec![0x44]);
        assert_eq!(HotkeyKeys::parse("Ctrl+F12").keys, vec![0x7B]);
        assert_eq!(HotkeyKeys::parse("Ctrl+Digit5").keys, vec![0x35]);
        assert_eq!(HotkeyKeys::parse("Ctrl+IntlRo").unknown, 1);
        assert_eq!(HotkeyKeys::parse("MouseBack").keys, vec![0x05]);
    }

    #[test]
    fn test_only_hotkey_keys_pass() {
        let keys = HotkeyKeys::parse("Ctrl+Shift+Space");
        // Generic and left-hand codes, as Windows reports them
        let clean = [VK_SHIFT, VK_CONTROL, 0x20, VK_LSHIFT, VK_LCONTROL];
        assert!(keys.extra_keys(&clean).is_empty());
    }

    #[test]
    fn test_other_keys_block_the_hotkey() {
        let keys = HotkeyKeys::parse("Ctrl+Shift+Space");
        let with_w = [VK_SHIFT, VK_CONTROL, 0x20, VK_LSHIFT, VK_LCONTROL, 0x57];
        assert_eq!(keys.extra_keys(&with_w), vec![0x57]);
        let with_alt = [VK_SHIFT, VK_CONTROL, VK_MENU, 0x20];
        assert_eq!(keys.extra_keys(&with_alt), vec![VK_MENU]);
        let with_mouse = [VK_SHIFT, VK_CONTROL, 0x20, 0x01];
        assert_eq!(keys.extra_keys(&with_mouse), vec![0x01]);
    }

    #[test]
    fn test_unknown_key_lets_one_key_through() {
        let keys = HotkeyKeys::parse("Ctrl+IntlRo");
        assert!(keys.extra_keys(&[VK_CONTROL, 0xC1]).is_empty());
        assert_eq!(keys.extra_keys(&[VK_CONTROL, 0xC1, 0x57]), vec![0x57]);
    }

    #[test]
    fn hotkeys_are_spelled_for_the_plugin() {
        assert_eq!(canonical("ctrl+shift+a"), "Ctrl+Shift+KeyA");
        assert_eq!(canonical(" Control + Alt + 7 "), "Ctrl+Alt+Digit7");
        assert_eq!(canonical("meta+space"), "Super+Space");
        assert_eq!(canonical("CmdOrCtrl+F5"), "CmdOrCtrl+F5");
        assert_eq!(canonical("Ctrl+Shift+Space"), "Ctrl+Shift+Space");
        assert_eq!(canonical("MouseBack"), "MouseBack");
        assert_eq!(canonical("Alt++KeyD"), "Alt+KeyD");
        assert_eq!(canonical(""), "");
    }

    #[test]
    fn test_key_names() {
        assert_eq!(key_name(0x57), "W");
        assert_eq!(key_name(0x7B), "F12");
        assert_eq!(key_name(0x01), "left mouse button");
    }
}

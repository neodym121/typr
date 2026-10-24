//! A hotkey of modifier keys alone, such as Ctrl+Alt (quick ask). The
//! global shortcut plugin needs a key besides the modifiers, so a low-level
//! keyboard hook watches them on a thread of its own. The hotkey fires when
//! its keys, held together with nothing else, are let go: a key or another
//! modifier pressed meanwhile cancels it, so Ctrl+Alt+Delete, Ctrl+Alt+T or
//! typing with AltGr never trigger it. Nothing is swallowed, the keys still
//! reach the app in front. The hook exists only while such a hotkey is set.

const CTRL: u8 = 1;
const SHIFT: u8 = 2;
const ALT: u8 = 4;
const WIN: u8 = 8;

/// The modifiers of a hotkey made of them alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modifiers(u8);

impl Modifiers {
    /// A hotkey of two or more modifiers and nothing else, e.g. "Ctrl+Alt".
    pub fn parse(hotkey: &str) -> Option<Modifiers> {
        let mut bits = 0;
        let mut count = 0;
        for part in hotkey.split('+').map(str::trim).filter(|part| !part.is_empty()) {
            let bit = match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => CTRL,
                "shift" => SHIFT,
                "alt" => ALT,
                "super" | "meta" | "win" => WIN,
                _ => return None,
            };
            bits |= bit;
            count += 1;
        }
        (count >= 2 && bits.count_ones() >= 2).then_some(Modifiers(bits))
    }
}

/// Follows the keys going down and up and tells when the hotkey fires.
#[derive(Debug, Default)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
struct Chord {
    want: u8,
    held: u8,
    /// All the hotkey's modifiers are down, and only they
    armed: bool,
    /// Something else was pressed: nothing fires until all are let go
    spoiled: bool,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
impl Chord {
    fn new(want: Modifiers) -> Self {
        Self {
            want: want.0,
            ..Self::default()
        }
    }

    /// A modifier went down or up; returns whether the hotkey fires.
    fn modifier(&mut self, bit: u8, down: bool) -> bool {
        if down {
            self.held |= bit;
            if self.held & !self.want != 0 {
                self.spoil();
            } else if self.held == self.want && !self.spoiled {
                self.armed = true;
            }
            return false;
        }
        let fire = self.armed && !self.spoiled && self.want & bit != 0;
        if fire {
            // Once per press: the other keys are still being let go
            self.spoil();
        }
        self.held &= !bit;
        if self.held == 0 {
            self.armed = false;
            self.spoiled = false;
        }
        fire
    }

    /// Any other key went down.
    fn other_key(&mut self) {
        if self.held != 0 {
            self.spoil();
        }
    }

    /// Modifiers the hook saw going down whose release it missed (the lock
    /// screen, say) are forgotten.
    fn forget(&mut self, released: u8) {
        if self.held & released == 0 {
            return;
        }
        self.held &= !released;
        if self.held == 0 {
            self.armed = false;
            self.spoiled = false;
        }
    }

    fn spoil(&mut self) {
        self.spoiled = true;
        self.armed = false;
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use super::{Chord, Modifiers, ALT, CTRL, SHIFT, WIN};
    use std::sync::mpsc::{self, Sender};
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
        TranslateMessage, UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG,
        WH_KEYBOARD_LL, WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
    };

    const VK_SHIFT: u32 = 0x10;
    const VK_CONTROL: u32 = 0x11;
    const VK_MENU: u32 = 0x12;
    const VK_LWIN: u32 = 0x5B;
    const VK_RWIN: u32 = 0x5C;
    const VK_LSHIFT: u32 = 0xA0;
    const VK_RSHIFT: u32 = 0xA1;
    const VK_LCONTROL: u32 = 0xA2;
    const VK_RCONTROL: u32 = 0xA3;
    const VK_LMENU: u32 = 0xA4;
    const VK_RMENU: u32 = 0xA5;

    static CHORD: Mutex<Option<Chord>> = Mutex::new(None);
    static FIRED: Mutex<Option<Sender<()>>> = Mutex::new(None);
    static HOOK_THREAD: Mutex<Option<u32>> = Mutex::new(None);

    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn modifier_bit(vk: u32) -> Option<u8> {
        match vk {
            VK_CONTROL | VK_LCONTROL | VK_RCONTROL => Some(CTRL),
            VK_SHIFT | VK_LSHIFT | VK_RSHIFT => Some(SHIFT),
            VK_MENU | VK_LMENU | VK_RMENU => Some(ALT),
            VK_LWIN | VK_RWIN => Some(WIN),
            _ => None,
        }
    }

    fn is_down(vk: u32) -> bool {
        (unsafe { GetAsyncKeyState(vk as i32) } as u16) & 0x8000 != 0
    }

    /// The modifiers that aren't physically down now
    fn released() -> u8 {
        let mut up = 0;
        if !is_down(VK_CONTROL) {
            up |= CTRL;
        }
        if !is_down(VK_SHIFT) {
            up |= SHIFT;
        }
        if !is_down(VK_MENU) {
            up |= ALT;
        }
        if !is_down(VK_LWIN) && !is_down(VK_RWIN) {
            up |= WIN;
        }
        up
    }

    /// Runs for every key event in the system: a little bookkeeping, and the
    /// event always goes on.
    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let info = &*(lparam as *const KBDLLHOOKSTRUCT);
            let down = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            // Keys simulated by programs (Typr's own copy shortcut too) don't
            // count, nor does the Ctrl that AltGr sends along with right Alt
            let fake_ctrl = info.vkCode == VK_LCONTROL && info.scanCode & 0x200 != 0;
            if info.flags & LLKHF_INJECTED == 0 && !fake_ctrl {
                let mut fire = false;
                if let Some(chord) = lock(&CHORD).as_mut() {
                    let bit = modifier_bit(info.vkCode);
                    // The state of the other keys is current inside the hook
                    chord.forget(released() & !bit.unwrap_or(0));
                    match bit {
                        Some(bit) => fire = chord.modifier(bit, down),
                        None if down => chord.other_key(),
                        None => {}
                    }
                }
                if fire {
                    if let Some(fired) = lock(&FIRED).as_ref() {
                        let _ = fired.send(());
                    }
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    pub fn start(modifiers: Modifiers, handler: Box<dyn Fn() + Send>) -> Result<(), String> {
        stop();
        let (fired, received) = mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("modifier-hotkey".to_string())
            .spawn(move || {
                while received.recv().is_ok() {
                    handler();
                }
            })
            .map_err(|e| format!("Failed to start the hotkey: {}", e))?;
        *lock(&FIRED) = Some(fired);
        *lock(&CHORD) = Some(Chord::new(modifiers));

        let (ready, started) = mpsc::channel::<Result<u32, String>>();
        std::thread::Builder::new()
            .name("keyboard-hook".to_string())
            .spawn(move || unsafe {
                let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), GetModuleHandleW(std::ptr::null()), 0);
                if hook.is_null() {
                    let _ = ready.send(Err("Windows refused the keyboard hook".to_string()));
                    return;
                }
                let _ = ready.send(Ok(GetCurrentThreadId()));
                log::debug!("Keyboard hook installed");
                let mut message: MSG = std::mem::zeroed();
                while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                UnhookWindowsHookEx(hook);
                log::debug!("Keyboard hook removed");
            })
            .map_err(|e| format!("Failed to start the keyboard hook: {}", e))?;

        match started.recv() {
            Ok(Ok(thread)) => {
                *lock(&HOOK_THREAD) = Some(thread);
                Ok(())
            }
            Ok(Err(e)) => {
                stop();
                Err(e)
            }
            Err(_) => {
                stop();
                Err("The keyboard hook stopped unexpectedly".to_string())
            }
        }
    }

    pub fn stop() {
        lock(&CHORD).take();
        if let Some(thread) = lock(&HOOK_THREAD).take() {
            unsafe {
                PostThreadMessageW(thread, WM_QUIT, 0, 0);
            }
        }
        // Dropping the sender ends the handler thread
        lock(&FIRED).take();
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use super::Modifiers;

    pub fn start(_modifiers: Modifiers, _handler: Box<dyn Fn() + Send>) -> Result<(), String> {
        Err("A hotkey of modifier keys alone works on Windows only".to_string())
    }

    pub fn stop() {}
}

/// Makes `modifiers` a hotkey: `handler` runs each time they are pressed
/// together and let go. Replaces the one set before.
pub fn start(modifiers: Modifiers, handler: impl Fn() + Send + 'static) -> Result<(), String> {
    imp::start(modifiers, Box::new(handler))
}

/// Removes the hotkey and the hook, if there is one.
pub fn stop() {
    imp::stop();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl_alt() -> Chord {
        Chord::new(Modifiers::parse("Ctrl+Alt").unwrap())
    }

    #[test]
    fn test_parse() {
        assert_eq!(Modifiers::parse("Ctrl+Alt"), Some(Modifiers(CTRL | ALT)));
        assert_eq!(Modifiers::parse("Ctrl+Shift+Alt"), Some(Modifiers(CTRL | SHIFT | ALT)));
        assert_eq!(Modifiers::parse("Super+Shift"), Some(Modifiers(WIN | SHIFT)));
        assert_eq!(Modifiers::parse("Ctrl"), None);
        assert_eq!(Modifiers::parse("Ctrl+Ctrl"), None);
        assert_eq!(Modifiers::parse("Ctrl+Alt+KeyC"), None);
        assert_eq!(Modifiers::parse("MouseBack"), None);
    }

    #[test]
    fn test_fires_when_let_go() {
        let mut chord = ctrl_alt();
        assert!(!chord.modifier(CTRL, true));
        assert!(!chord.modifier(ALT, true));
        // Held keys repeat
        assert!(!chord.modifier(ALT, true));
        assert!(chord.modifier(ALT, false));
        // Once per press
        assert!(!chord.modifier(CTRL, false));
        // And again next time
        assert!(!chord.modifier(ALT, true));
        assert!(!chord.modifier(CTRL, true));
        assert!(chord.modifier(CTRL, false));
        assert!(!chord.modifier(ALT, false));
    }

    #[test]
    fn test_another_key_cancels() {
        let mut chord = ctrl_alt();
        chord.modifier(CTRL, true);
        chord.modifier(ALT, true);
        // Ctrl+Alt+Delete
        chord.other_key();
        assert!(!chord.modifier(ALT, false));
        assert!(!chord.modifier(CTRL, false));
        // All let go: the next press works
        chord.modifier(CTRL, true);
        chord.modifier(ALT, true);
        assert!(chord.modifier(CTRL, false));
    }

    #[test]
    fn test_another_modifier_cancels() {
        let mut chord = ctrl_alt();
        chord.modifier(CTRL, true);
        chord.modifier(SHIFT, true);
        chord.modifier(ALT, true);
        chord.modifier(SHIFT, false);
        assert!(!chord.modifier(ALT, false));
        assert!(!chord.modifier(CTRL, false));
    }

    #[test]
    fn test_one_modifier_alone_does_nothing() {
        let mut chord = ctrl_alt();
        chord.modifier(CTRL, true);
        assert!(!chord.modifier(CTRL, false));
    }

    #[test]
    fn test_a_missed_release_is_forgotten() {
        let mut chord = ctrl_alt();
        // Ctrl went down, then the lock screen took its release
        chord.modifier(CTRL, true);
        chord.forget(CTRL);
        chord.modifier(ALT, true);
        chord.modifier(CTRL, true);
        assert!(chord.modifier(ALT, false));
    }
}

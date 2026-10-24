//! A mouse button as a hotkey: the middle button or a side button (back,
//! forward), for dictation or the quick ask window. The global shortcut plugin only
//! knows keys, so a low-level mouse hook catches the buttons on a thread of
//! its own. A bound button is swallowed, so it doesn't also go back in the
//! browser while it starts a dictation. The hook exists only while a mouse
//! hotkey is active: with keyboard hotkeys, or the hotkey off in the tray,
//! nothing is hooked.
//!
//! A release is swallowed only after its press was: if the hook appears
//! while the button is down (it was just chosen as the hotkey with a click),
//! the release goes on to the window that got the press. Otherwise that
//! window keeps the mouse captured and Windows thinks the button is held,
//! and the mouse stops working everywhere else.

/// Mouse buttons that can be a hotkey. The left and right buttons can't:
/// they are needed for everything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Middle,
    /// The side button usually meaning "back" (XBUTTON1)
    Back,
    /// The side button usually meaning "forward" (XBUTTON2)
    Forward,
}

impl MouseButton {
    /// The button of a saved hotkey such as "MouseBack", if it is one.
    pub fn parse(hotkey: &str) -> Option<MouseButton> {
        match hotkey.trim() {
            "MouseMiddle" => Some(MouseButton::Middle),
            "MouseBack" => Some(MouseButton::Back),
            "MouseForward" => Some(MouseButton::Forward),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            MouseButton::Middle => "MouseMiddle",
            MouseButton::Back => "MouseBack",
            MouseButton::Forward => "MouseForward",
        }
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    fn code(self) -> u8 {
        match self {
            MouseButton::Middle => 1,
            MouseButton::Back => 2,
            MouseButton::Forward => 3,
        }
    }
}

/// What a mouse hotkey is for; each can have its own button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Dictation,
    Ask,
}

impl Slot {
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    fn index(self) -> usize {
        match self {
            Slot::Dictation => 0,
            Slot::Ask => 1,
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use super::{MouseButton, Slot};
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    use std::sync::mpsc::{self, Sender};
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
        TranslateMessage, UnhookWindowsHookEx, HC_ACTION, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
        WH_MOUSE_LL, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_QUIT, WM_XBUTTONDOWN, WM_XBUTTONUP, XBUTTON1,
        XBUTTON2,
    };

    const SLOTS: usize = 2;

    /// `MouseButton::code` of each slot's button, 0 for none
    static BUTTONS: [AtomicU8; SLOTS] = [AtomicU8::new(0), AtomicU8::new(0)];
    /// The hook swallowed the press of a slot's button, so it swallows its
    /// release too
    static HELD: [AtomicBool; SLOTS] = [AtomicBool::new(false), AtomicBool::new(false)];
    /// Presses (true) and releases (false) on their way to each slot's handler
    static EVENTS: Mutex<[Option<Sender<bool>>; SLOTS]> = Mutex::new([None, None]);
    /// The thread that owns the hook and runs its message loop; holding the
    /// lock also keeps two starts or stops from mixing
    static HOOK_THREAD: Mutex<Option<u32>> = Mutex::new(None);

    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs for every mouse event in the system, so it does the least it
    /// can: anything but a bound button goes straight on.
    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let message = wparam as u32;
            let event = match message {
                WM_MBUTTONDOWN | WM_MBUTTONUP => Some((1, message == WM_MBUTTONDOWN)),
                WM_XBUTTONDOWN | WM_XBUTTONUP => {
                    let info = &*(lparam as *const MSLLHOOKSTRUCT);
                    let which = match (info.mouseData >> 16) as u16 {
                        XBUTTON1 => 2,
                        XBUTTON2 => 3,
                        _ => 0,
                    };
                    Some((which, message == WM_XBUTTONDOWN))
                }
                _ => None,
            };
            if let Some((button, pressed)) = event {
                let info = &*(lparam as *const MSLLHOOKSTRUCT);
                // Clicks simulated by other programs are left alone
                if button != 0 && info.flags & LLMHF_INJECTED == 0 {
                    for slot in 0..SLOTS {
                        if BUTTONS[slot].load(Ordering::Relaxed) != button {
                            continue;
                        }
                        let swallow = if pressed {
                            HELD[slot].store(true, Ordering::SeqCst);
                            true
                        } else {
                            // A release whose press went to a window goes there too
                            HELD[slot].swap(false, Ordering::SeqCst)
                        };
                        if swallow {
                            if let Some(events) = lock(&EVENTS)[slot].as_ref() {
                                let _ = events.send(pressed);
                            }
                            return 1;
                        }
                    }
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    /// Installs the hook on a thread of its own; returns that thread's id.
    fn install_hook() -> Result<u32, String> {
        let (ready, started) = mpsc::channel::<Result<u32, String>>();
        std::thread::Builder::new()
            .name("mouse-hook".to_string())
            .spawn(move || unsafe {
                let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), GetModuleHandleW(std::ptr::null()), 0);
                if hook.is_null() {
                    let _ = ready.send(Err("Windows refused the mouse hook".to_string()));
                    return;
                }
                let _ = ready.send(Ok(GetCurrentThreadId()));
                log::debug!("Mouse hook installed");
                let mut message: MSG = std::mem::zeroed();
                while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                UnhookWindowsHookEx(hook);
                log::debug!("Mouse hook removed");
            })
            .map_err(|e| format!("Failed to start the mouse hook: {}", e))?;
        started
            .recv()
            .unwrap_or_else(|_| Err("The mouse hook stopped unexpectedly".to_string()))
    }

    /// Clears a slot; the hook goes once no slot is left. Takes the
    /// `HOOK_THREAD` guard the caller holds.
    fn clear(slot: usize, hook_thread: &mut Option<u32>) {
        BUTTONS[slot].store(0, Ordering::SeqCst);
        // Dropping the sender ends the handler thread
        lock(&EVENTS)[slot].take();
        let unused = BUTTONS.iter().all(|button| button.load(Ordering::SeqCst) == 0);
        if unused {
            if let Some(thread) = hook_thread.take() {
                unsafe {
                    PostThreadMessageW(thread, WM_QUIT, 0, 0);
                }
            }
        }
    }

    pub fn start(slot: Slot, button: MouseButton, handler: Box<dyn Fn(bool) + Send>) -> Result<(), String> {
        let index = slot.index();
        let mut hook_thread = lock(&HOOK_THREAD);
        clear(index, &mut hook_thread);

        // The handler runs on its own thread, never inside the hook
        let (events, received) = mpsc::channel::<bool>();
        std::thread::Builder::new()
            .name("mouse-hotkey".to_string())
            .spawn(move || {
                while let Ok(pressed) = received.recv() {
                    handler(pressed);
                }
            })
            .map_err(|e| format!("Failed to start the mouse hotkey: {}", e))?;
        lock(&EVENTS)[index] = Some(events);
        HELD[index].store(false, Ordering::SeqCst);
        BUTTONS[index].store(button.code(), Ordering::SeqCst);

        if hook_thread.is_none() {
            match install_hook() {
                Ok(thread) => *hook_thread = Some(thread),
                Err(e) => {
                    clear(index, &mut hook_thread);
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    pub fn stop(slot: Slot) {
        let mut hook_thread = lock(&HOOK_THREAD);
        clear(slot.index(), &mut hook_thread);
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use super::{MouseButton, Slot};

    pub fn start(_slot: Slot, _button: MouseButton, _handler: Box<dyn Fn(bool) + Send>) -> Result<(), String> {
        Err("A mouse button as the hotkey works on Windows only".to_string())
    }

    pub fn stop(_slot: Slot) {}
}

/// Makes `button` the hotkey of `slot`: `handler` gets `true` when it is
/// pressed and `false` when it is released. Replaces the slot's button.
pub fn start(slot: Slot, button: MouseButton, handler: impl Fn(bool) + Send + 'static) -> Result<(), String> {
    imp::start(slot, button, Box::new(handler))
}

/// Removes the mouse hotkey of `slot`, and the hook if no other is left.
pub fn stop(slot: Slot) {
    imp::stop(slot);
}

/// Removes every mouse hotkey and the hook.
pub fn stop_all() {
    imp::stop(Slot::Dictation);
    imp::stop(Slot::Ask);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse() {
        assert_eq!(MouseButton::parse("MouseBack"), Some(MouseButton::Back));
        assert_eq!(MouseButton::parse("MouseMiddle"), Some(MouseButton::Middle));
        assert_eq!(MouseButton::parse("MouseForward"), Some(MouseButton::Forward));
        assert_eq!(MouseButton::parse("Ctrl+Shift+Space"), None);
        for button in [MouseButton::Middle, MouseButton::Back, MouseButton::Forward] {
            assert_eq!(MouseButton::parse(button.id()), Some(button));
        }
    }
}

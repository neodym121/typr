//! Pastes text into the active window through the clipboard without leaving
//! it there: the dictated text is kept out of the Windows clipboard history
//! (Win+V) and cloud sync, and whatever was on the clipboard before is put
//! back right after the paste.

use std::time::Duration;

/// Time the target app gets to read the clipboard after Ctrl+V before the
/// previous content is put back. Too short, and slow apps paste the old one.
const RESTORE_DELAY: Duration = Duration::from_millis(300);

/// Between setting the clipboard and pressing Ctrl+V, so the new content is
/// in place before the target app asks for it.
const SETTLE_DELAY: Duration = Duration::from_millis(50);

/// What was on the clipboard before the dictation (or the selection capture).
pub(crate) enum Saved {
    Text(String),
    Image(arboard::ImageData<'static>),
    Nothing,
}

/// Pastes `text` into the window in front and then puts back what was on
/// the clipboard before. If the keys can't be sent, the text stays on the
/// clipboard, so it can still be pasted by hand.
pub fn paste_text(text: &str) -> Result<(), String> {
    let mut clipboard = open_clipboard()?;
    let previous = save(&mut clipboard);
    set_text_quietly(&mut clipboard, text)
        .map_err(|e| format!("Failed to copy text to the clipboard: {}", e))?;
    log::debug!("Copied {} characters to the clipboard", text.chars().count());

    std::thread::sleep(SETTLE_DELAY);
    press_paste_keys()?;

    std::thread::sleep(RESTORE_DELAY);
    restore(&mut clipboard, text, previous);
    Ok(())
}

/// Puts `text` on the clipboard and leaves it there (kept out of the
/// clipboard history), for when there is no window to paste into.
pub fn copy_text(text: &str) -> Result<(), String> {
    let mut clipboard = open_clipboard()?;
    set_text_quietly(&mut clipboard, text).map_err(|e| format!("Failed to copy the text: {}", e))
}

fn open_clipboard() -> Result<arboard::Clipboard, String> {
    arboard::Clipboard::new().map_err(|e| format!("Clipboard is unavailable: {}", e))
}

/// Ctrl+V for the window in front.
#[cfg(target_os = "windows")]
fn press_paste_keys() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};

    // Virtual-key code of the V key. Key::Unicode('v') is looked up in the
    // current keyboard layout and fails when that layout has no Latin "v"
    // (e.g. Russian), which used to break the paste — and the whole dictation.
    const VK_V: u32 = 0x56;

    let mut keys = Enigo::new(&Settings::default())
        .map_err(|e| format!("Failed to initialise keyboard simulation: {}", e))?;
    keys.key(Key::Control, Direction::Press)
        .map_err(|e| format!("Failed to press Ctrl: {}", e))?;
    let v = keys.key(Key::Other(VK_V), Direction::Click);
    // Ctrl goes up even if V failed, so it never stays logically held down
    let ctrl = keys.key(Key::Control, Direction::Release);
    v.map_err(|e| format!("Failed to send Ctrl+V: {}", e))?;
    ctrl.map_err(|e| format!("Failed to release Ctrl: {}", e))?;
    log::debug!("Sent Ctrl+V to the active window");
    Ok(())
}

/// Cmd+V, sent by System Events: enigo would have to run on the main
/// thread there.
#[cfg(target_os = "macos")]
fn press_paste_keys() -> Result<(), String> {
    let script = "tell application \"System Events\" to keystroke \"v\" using {command down}";
    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status()
        .map_err(|e| format!("Could not run osascript: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("osascript could not press Cmd+V ({})", status))
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn press_paste_keys() -> Result<(), String> {
    Err("Pasting isn't supported on this system; the text is on the clipboard".to_string())
}

/// The window in front now, where a paste lands (0 if none).
pub fn foreground_window() -> isize {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        unsafe { GetForegroundWindow() as isize }
    }
    #[cfg(not(target_os = "windows"))]
    {
        0
    }
}

/// Brings `window` (from `foreground_window`) back to the front. False when
/// it no longer exists or Windows refused.
pub fn focus_window(window: isize) -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            IsIconic, IsWindow, SetForegroundWindow, ShowWindow, SW_RESTORE,
        };
        let hwnd = window as windows_sys::Win32::Foundation::HWND;
        if window == 0 || unsafe { IsWindow(hwnd) } == 0 {
            return false;
        }
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd) != 0
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
        false
    }
}

pub(crate) fn save(clipboard: &mut arboard::Clipboard) -> Saved {
    if let Ok(text) = clipboard.get_text() {
        return Saved::Text(text);
    }
    if let Ok(image) = clipboard.get_image() {
        return Saved::Image(image);
    }
    Saved::Nothing
}

/// Puts `saved` back, unless the clipboard no longer holds `ours` (the text
/// Typr put there or read from it).
pub(crate) fn restore(clipboard: &mut arboard::Clipboard, ours: &str, saved: Saved) {
    // Something else was copied in the meantime: leave it alone
    match clipboard.get_text() {
        Ok(current) if current == ours => {}
        _ => {
            log::debug!("Clipboard changed after the paste, not restoring it");
            return;
        }
    }

    let (result, what) = match saved {
        Saved::Text(previous) => (set_text_quietly(clipboard, &previous), "previous text"),
        Saved::Image(image) => (clipboard.set_image(image), "previous image"),
        Saved::Nothing => (clipboard.clear(), "empty clipboard"),
    };
    match result {
        Ok(()) => log::debug!("Clipboard restored ({})", what),
        Err(e) => log::warn!("Could not restore the clipboard: {}", e),
    }
}

/// Sets text that Windows keeps out of the clipboard history (Win+V), cloud
/// sync and clipboard managers.
fn set_text_quietly(clipboard: &mut arboard::Clipboard, text: &str) -> Result<(), arboard::Error> {
    #[cfg(target_os = "windows")]
    let result = {
        use arboard::SetExtWindows;
        clipboard.set().exclude_from_monitoring().text(text)
    };
    #[cfg(not(target_os = "windows"))]
    let result = clipboard.set_text(text);
    result
}

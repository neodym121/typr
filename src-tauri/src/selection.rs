//! Reads the text selected in the window the user is in, for quick ask
//! (`ask/`): with something selected, the hotkey asks about it instead of
//! dictating.
//!
//! The selection is copied with a synthetic copy shortcut, then the clipboard
//! is put back. Two rules make that safe:
//!
//! * **Only a clipboard that provably changed is read.** Windows counts every
//!   clipboard change (`GetClipboardSequenceNumber`); if the copy left the
//!   count alone, nothing was selected. Reading the clipboard regardless would
//!   send whatever was copied earlier, maybe a password, to a cloud model.
//! * **The keystroke never reaches the app mixed with the user's keys.** It is
//!   sent once Ctrl, Shift, Alt and Win are all up (right after the press of a
//!   toggle hotkey, at the end of a push-to-talk one), so Ctrl+Shift+Space
//!   never turns it into Ctrl+Shift+C.
//!
//! Where a copy shortcut does something else when nothing is selected, it
//! isn't sent the usual way: terminals get none (Ctrl+C would interrupt the
//! running program), editors with a built-in terminal get Ctrl+Insert, and the
//! "copy the current line" of code editors without a selection doesn't count.
//!
//! The capture runs on its own thread while the user speaks; the dictation
//! collects it with [`take`] once the speech is transcribed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A selection longer than this is cut: a whole document in the prompt buys
/// nothing and costs time and tokens.
const MAX_CHARS: usize = 10_000;

/// How the copy is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyKeys {
    /// Ctrl+C, understood everywhere
    CtrlC,
    /// Ctrl+Insert: copies in editors too, and is no interrupt in their terminal
    CtrlInsert,
}

/// What a window is, as far as copying from it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppKind {
    /// A terminal: a copy shortcut without a selection interrupts the program
    Terminal,
    /// A code editor with a terminal inside: Ctrl+Insert is safe in both
    EditorWithTerminal,
    /// Copies the current line when nothing is selected, with no marker on the
    /// clipboard that says so
    CopiesLine,
    Other,
}

/// Window classes of terminal windows.
const TERMINAL_CLASSES: &[&str] = &[
    "ConsoleWindowClass",
    "CASCADIA_HOSTING_WINDOW_CLASS",
    "mintty",
    "PuTTY",
    "KiTTY",
    "VirtualConsoleClass",
    "org.wezfurlong.wezterm",
];

/// Executables of terminals (lowercase).
const TERMINAL_APPS: &[&str] = &[
    "windowsterminal.exe",
    "wt.exe",
    "openconsole.exe",
    "conhost.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "wezterm-gui.exe",
    "alacritty.exe",
    "mintty.exe",
    "putty.exe",
    "kitty.exe",
    "hyper.exe",
    "tabby.exe",
    "conemu.exe",
    "conemu64.exe",
    "warp.exe",
    "ghostty.exe",
    "rio.exe",
    "mobaxterm.exe",
    "termius.exe",
    "fluentterminal.app.exe",
];

/// Code editors with a built-in terminal (lowercase executables).
const EDITORS_WITH_TERMINAL: &[&str] = &[
    "code.exe",
    "code - insiders.exe",
    "cursor.exe",
    "windsurf.exe",
    "vscodium.exe",
    "codium.exe",
    "trae.exe",
    "kiro.exe",
    "positron.exe",
    "devenv.exe",
    "fleet.exe",
    "studio64.exe",
];

/// JetBrains IDEs (lowercase executables): a terminal inside, and they copy the
/// current line when nothing is selected without marking it on the clipboard.
const JETBRAINS_APPS: &[&str] = &[
    "idea64.exe",
    "idea.exe",
    "pycharm64.exe",
    "pycharm.exe",
    "webstorm64.exe",
    "phpstorm64.exe",
    "clion64.exe",
    "rider64.exe",
    "goland64.exe",
    "rubymine64.exe",
    "datagrip64.exe",
    "dataspell64.exe",
    "rustrover64.exe",
    "aqua64.exe",
];

/// Editors that copy the current line when nothing is selected, unmarked.
const LINE_COPYING_APPS: &[&str] = &["sublime_text.exe"];

fn app_kind(class: &str, exe: &str) -> AppKind {
    let exe = exe.to_ascii_lowercase();
    if TERMINAL_CLASSES.contains(&class) || TERMINAL_APPS.contains(&exe.as_str()) {
        AppKind::Terminal
    } else if JETBRAINS_APPS.contains(&exe.as_str()) {
        // Copies the line too, so this is checked as well (see `is_line_copy`)
        AppKind::EditorWithTerminal
    } else if EDITORS_WITH_TERMINAL.contains(&exe.as_str()) {
        AppKind::EditorWithTerminal
    } else if LINE_COPYING_APPS.contains(&exe.as_str()) {
        AppKind::CopiesLine
    } else {
        AppKind::Other
    }
}

fn copy_keys(kind: AppKind) -> CopyKeys {
    match kind {
        AppKind::EditorWithTerminal => CopyKeys::CtrlInsert,
        _ => CopyKeys::CtrlC,
    }
}

/// Whether `exe` copies the current line without a selection and leaves no
/// marker on the clipboard, so a lone line is all there is to go by.
fn copies_unmarked_line(exe: &str) -> bool {
    let exe = exe.to_ascii_lowercase();
    JETBRAINS_APPS.contains(&exe.as_str()) || LINE_COPYING_APPS.contains(&exe.as_str())
}

/// A single line ending in a line break: what an editor copies when nothing
/// is selected.
fn looks_like_copied_line(text: &str) -> bool {
    let body = text.strip_suffix('\n').map(|t| t.strip_suffix('\r').unwrap_or(t));
    matches!(body, Some(line) if !line.contains('\n'))
}

/// One spreadsheet cell: a spreadsheet always has an active cell, so a copy
/// without a selection copies it. Several cells (tabs, line breaks) are a
/// selection.
fn is_single_cell(text: &str) -> bool {
    let cell = text.trim_end_matches(['\r', '\n']);
    !cell.contains('\t') && !cell.contains('\n')
}

/// Trims, drops an empty selection and cuts a very long one.
fn normalize(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() <= MAX_CHARS {
        return Some(trimmed.to_string());
    }
    log::warn!("The selection is longer than {} characters, the rest is left out", MAX_CHARS);
    Some(trimmed.chars().take(MAX_CHARS).collect())
}

// ── Capture in the background ───────────────────────────
// Started when a recording starts, collected when its speech is transcribed.
// The generation keeps a capture from an earlier recording out of a later
// question.

static GENERATION: AtomicU64 = AtomicU64::new(0);
static RESULT: Mutex<Option<(u64, Option<String>)>> = Mutex::new(None);

/// Starts reading the selection of the window in front, for the recording
/// that starts now. Returns the generation to collect it with.
pub fn begin_capture() -> u64 {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    *RESULT.lock().unwrap() = None;

    #[cfg(target_os = "windows")]
    {
        let target = native::foreground();
        let spawned = std::thread::Builder::new()
            .name("typr-selection".to_string())
            .spawn(move || {
                let selection = match native::capture(target, generation) {
                    Ok(selection) => selection,
                    Err(reason) => {
                        // Not an error for the user: the recording is then a
                        // dictation. But "why didn't it see my selection" needs
                        // an answer in the log.
                        log::info!("Selection not read: {}", reason);
                        None
                    }
                };
                if GENERATION.load(Ordering::SeqCst) == generation {
                    *RESULT.lock().unwrap() = Some((generation, selection));
                }
            });
        if let Err(e) = spawned {
            log::warn!("Could not start reading the selection: {}", e);
            *RESULT.lock().unwrap() = Some((generation, None));
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        *RESULT.lock().unwrap() = Some((generation, None));
    }

    generation
}

/// Forgets a capture in progress (the recording was cancelled), so it can't
/// show up in front of a later question.
pub fn forget() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    *RESULT.lock().unwrap() = None;
}

/// The selection read for `generation`, waiting up to `wait` for the capture
/// to finish. `None`: nothing was selected, it couldn't be read, or it
/// belongs to another recording.
pub async fn take(generation: u64, wait: Duration) -> Option<String> {
    let deadline = Instant::now() + wait;
    loop {
        if GENERATION.load(Ordering::SeqCst) != generation {
            return None;
        }
        {
            let mut result = RESULT.lock().unwrap();
            if matches!(&*result, Some((g, _)) if *g == generation) {
                return result.take().and_then(|(_, selection)| selection);
            }
        }
        if Instant::now() >= deadline {
            log::info!("The selection wasn't read in time, carrying on without it");
            return None;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn is_current(generation: u64) -> bool {
    GENERATION.load(Ordering::SeqCst) == generation
}

// ── Windows ─────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod native {
    use super::{
        app_kind, copies_unmarked_line, copy_keys, is_current, is_single_cell, looks_like_copied_line, normalize,
        AppKind, CopyKeys,
    };
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
        KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_INSERT, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId};

    /// The user's keys get this long to come up (a push-to-talk hotkey is held
    /// for the whole question).
    const KEYS_UP_BUDGET: Duration = Duration::from_secs(120);
    /// How long the app gets to put the selection on the clipboard. Without a
    /// selection the whole of it passes, so it is kept short.
    const COPY_BUDGET: Duration = Duration::from_millis(300);
    const POLL: Duration = Duration::from_millis(15);

    /// Clipboard formats that mark "the current line, copied without a
    /// selection": Scintilla (Notepad++) and Visual Studio.
    const LINE_COPY_FORMATS: &[&str] = &["MSDEVLineSelect", "VisualStudioEditorOperationsLineCutCopyClipboardTag"];
    /// VS Code and its forks put their copy metadata in Chromium's custom data
    const CHROMIUM_CUSTOM_DATA: &str = "Chromium Web Custom MIME Data Format";
    const VSCODE_EMPTY_SELECTION: &str = "\"isFromEmptySelection\":true";
    /// Formats of a copy from Excel
    const SPREADSHEET_FORMATS: &[&str] = &["XML Spreadsheet", "Biff12", "Biff8"];

    /// The window in front and what it is.
    pub struct Target {
        hwnd: isize,
        class: String,
        exe: String,
        own: bool,
    }

    pub fn foreground() -> Target {
        unsafe {
            let hwnd = GetForegroundWindow();
            let mut class = [0u16; 256];
            let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
            let class = String::from_utf16_lossy(&class[..len]);

            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            Target {
                hwnd: hwnd as isize,
                class,
                exe: executable_name(pid),
                own: pid != 0 && pid == GetCurrentProcessId(),
            }
        }
    }

    /// File name of a process's executable, e.g. "Code.exe" (empty if unknown).
    unsafe fn executable_name(pid: u32) -> String {
        if pid == 0 {
            return String::new();
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return String::new();
        }
        let mut path = [0u16; 1024];
        let mut len = path.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, path.as_mut_ptr(), &mut len) != 0;
        CloseHandle(process);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&path[..len as usize]);
        path.rsplit(['\\', '/']).next().unwrap_or("").to_string()
    }

    fn key_down(vk: VIRTUAL_KEY) -> bool {
        unsafe { (GetAsyncKeyState(vk as i32) as u16) & 0x8000 != 0 }
    }

    fn modifiers_down() -> bool {
        [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN].into_iter().any(key_down)
    }

    pub fn capture(target: Target, generation: u64) -> Result<Option<String>, String> {
        if target.hwnd == 0 {
            return Err("no window is in front".to_string());
        }
        if target.own {
            return Err("Typr's own window is in front".to_string());
        }
        let kind = app_kind(&target.class, &target.exe);
        if kind == AppKind::Terminal {
            return Err(format!(
                "{} is a terminal, where a copy shortcut without a selection would interrupt the program",
                describe(&target)
            ));
        }

        // The user's own keys first come up, so the copy shortcut reaches the
        // app on its own
        let deadline = Instant::now() + KEYS_UP_BUDGET;
        while modifiers_down() {
            if !is_current(generation) {
                return Err("a newer recording started".to_string());
            }
            if Instant::now() >= deadline {
                return Err("the modifier keys stayed down".to_string());
            }
            std::thread::sleep(POLL);
        }
        if !is_current(generation) {
            return Err("a newer recording started".to_string());
        }
        if unsafe { GetForegroundWindow() } as isize != target.hwnd {
            return Err("another window came to the front".to_string());
        }

        let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("the clipboard is unavailable: {}", e))?;
        let saved = crate::paste::save(&mut clipboard);
        let before = clipboard_win::seq_num();

        let keys = copy_keys(kind);
        send_copy(keys)?;

        let deadline = Instant::now() + COPY_BUDGET;
        let mut changed = false;
        while Instant::now() < deadline {
            std::thread::sleep(POLL);
            if clipboard_win::seq_num() != before {
                changed = true;
                break;
            }
        }
        if !changed {
            log::debug!("Nothing selected in {} ({:?} changed nothing)", describe(&target), keys);
            return Ok(None);
        }
        // Let the app finish writing every format of the copy
        std::thread::sleep(Duration::from_millis(30));

        let copied = clipboard.get_text().unwrap_or_default();
        let line_copy = is_marked_line_copy() || (copies_unmarked_line(&target.exe) && looks_like_copied_line(&copied));
        let active_cell = any_format(SPREADSHEET_FORMATS) && is_single_cell(&copied);
        crate::paste::restore(&mut clipboard, &copied, saved);

        if line_copy {
            log::debug!("{} copied its current line: nothing was selected", describe(&target));
            return Ok(None);
        }
        if active_cell {
            log::debug!("{} copied its active cell: nothing was selected", describe(&target));
            return Ok(None);
        }
        let selection = normalize(&copied);
        if let Some(text) = &selection {
            log::info!("Read a selection of {} characters from {}", text.chars().count(), describe(&target));
        }
        Ok(selection)
    }

    fn describe(target: &Target) -> String {
        if target.exe.is_empty() {
            format!("the window ({})", target.class)
        } else {
            target.exe.clone()
        }
    }

    /// Whether the clipboard holds any of these formats.
    fn any_format(names: &[&str]) -> bool {
        names.iter().any(|name| {
            clipboard_win::register_format(name).is_some_and(|format| clipboard_win::is_format_avail(format.get()))
        })
    }

    /// Whether the clipboard says the copy was the current line, taken
    /// without a selection.
    fn is_marked_line_copy() -> bool {
        if any_format(LINE_COPY_FORMATS) {
            return true;
        }
        let Some(format) = clipboard_win::register_format(CHROMIUM_CUSTOM_DATA) else {
            return false;
        };
        if !clipboard_win::is_format_avail(format.get()) {
            return false;
        }
        let Ok(data) = clipboard_win::get_clipboard::<Vec<u8>, _>(clipboard_win::formats::RawData(format.get()))
        else {
            return false;
        };
        // A pickle of UTF-16 strings; the metadata reads fine decoded as a whole
        let units: Vec<u16> = data.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
        String::from_utf16_lossy(&units).contains(VSCODE_EMPTY_SELECTION)
            || String::from_utf8_lossy(&data).contains(VSCODE_EMPTY_SELECTION)
    }

    /// Presses the copy shortcut with Typr's own Ctrl (the user's is up).
    fn send_copy(keys: CopyKeys) -> Result<(), String> {
        const SCAN_CTRL: u16 = 0x1D;
        const SCAN_C: u16 = 0x2E;
        const SCAN_INSERT: u16 = 0x52;
        const VK_C: VIRTUAL_KEY = 0x43;

        let (vk, scan, extended) = match keys {
            CopyKeys::CtrlC => (VK_C, SCAN_C, 0),
            // Insert is on the navigation block, not the number pad
            CopyKeys::CtrlInsert => (VK_INSERT, SCAN_INSERT, KEYEVENTF_EXTENDEDKEY),
        };
        let inputs = [
            key_input(VK_CONTROL, SCAN_CTRL, 0),
            key_input(vk, scan, extended),
            key_input(vk, scan, extended | KEYEVENTF_KEYUP),
            key_input(VK_CONTROL, SCAN_CTRL, KEYEVENTF_KEYUP),
        ];
        let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
        if sent as usize != inputs.len() {
            // Never leave Ctrl logically held down
            let release = [key_input(VK_CONTROL, SCAN_CTRL, KEYEVENTF_KEYUP)];
            unsafe { SendInput(1, release.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
            return Err("the copy shortcut couldn't be sent (another program may block input)".to_string());
        }
        Ok(())
    }

    fn key_input(vk: VIRTUAL_KEY, scan: u16, flags: u32) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminals_editors_and_other_apps() {
        assert_eq!(app_kind("CASCADIA_HOSTING_WINDOW_CLASS", "WindowsTerminal.exe"), AppKind::Terminal);
        assert_eq!(app_kind("ConsoleWindowClass", ""), AppKind::Terminal);
        assert_eq!(app_kind("mintty", "mintty.exe"), AppKind::Terminal);
        assert_eq!(app_kind("Chrome_WidgetWin_1", "Hyper.exe"), AppKind::Terminal);
        assert_eq!(app_kind("Chrome_WidgetWin_1", "Code.exe"), AppKind::EditorWithTerminal);
        assert_eq!(app_kind("Chrome_WidgetWin_1", "Cursor.exe"), AppKind::EditorWithTerminal);
        assert_eq!(app_kind("SunAwtFrame", "idea64.exe"), AppKind::EditorWithTerminal);
        assert_eq!(app_kind("PX_WINDOW_CLASS", "sublime_text.exe"), AppKind::CopiesLine);
        assert_eq!(app_kind("Chrome_WidgetWin_1", "chrome.exe"), AppKind::Other);
        assert_eq!(app_kind("Notepad", "notepad.exe"), AppKind::Other);
    }

    #[test]
    fn test_copy_keys_never_interrupt_a_built_in_terminal() {
        assert_eq!(copy_keys(AppKind::EditorWithTerminal), CopyKeys::CtrlInsert);
        assert_eq!(copy_keys(AppKind::Other), CopyKeys::CtrlC);
        assert_eq!(copy_keys(AppKind::CopiesLine), CopyKeys::CtrlC);
    }

    #[test]
    fn test_a_copied_line_is_not_a_selection() {
        assert!(looks_like_copied_line("    let x = 1;\n"));
        assert!(looks_like_copied_line("let x = 1;\r\n"));
        assert!(!looks_like_copied_line("let x = 1;"));
        assert!(!looks_like_copied_line("one\ntwo\n"));
        assert!(copies_unmarked_line("PyCharm64.exe"));
        assert!(copies_unmarked_line("sublime_text.exe"));
        assert!(!copies_unmarked_line("chrome.exe"));
    }

    #[test]
    fn test_one_spreadsheet_cell_is_not_a_selection() {
        assert!(is_single_cell("42
"));
        assert!(is_single_cell("Total"));
        assert!(!is_single_cell("a	b
"));
        assert!(!is_single_cell("a
b
"));
    }

    #[test]
    fn test_normalize() {
        assert_eq!(normalize("  \n\t "), None);
        assert_eq!(normalize("  hello\nworld \n").as_deref(), Some("hello\nworld"));
        let long = "я".repeat(MAX_CHARS + 50);
        assert_eq!(normalize(&long).unwrap().chars().count(), MAX_CHARS);
    }

    #[tokio::test]
    async fn test_a_capture_reaches_only_its_own_recording() {
        let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        *RESULT.lock().unwrap() = Some((generation, Some("text".to_string())));
        assert_eq!(take(generation, Duration::from_millis(10)).await.as_deref(), Some("text"));
        // Taken once
        assert_eq!(take(generation, Duration::from_millis(10)).await, None);

        *RESULT.lock().unwrap() = Some((generation, Some("old".to_string())));
        forget();
        assert_eq!(take(generation, Duration::from_millis(10)).await, None);
    }
}

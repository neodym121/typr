//! The quick ask window drawn by Typr itself: Slint's software renderer on a
//! native layered window (platform.rs), no WebView. Its thread starts with
//! the first window and stays; it sleeps while no window is open.
//!
//! Commands go to the thread ([`send`]); what the window has to say comes
//! back through the callback given to [`start`], on the window's thread.

mod markdown;
pub mod platform;
mod shadow;
mod view;
pub mod demo;

slint::include_modules!();

pub use view::{strings, Command, Event, Layout, Recording, Strings};

use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;

struct Thread {
    commands: Sender<Command>,
    id: u32,
}

static THREAD: Mutex<Option<Thread>> = Mutex::new(None);

/// Starts the window's thread, unless it runs already. `on_event` is called
/// on that thread.
pub fn start(on_event: impl Fn(Event) + Send + 'static) -> Result<(), String> {
    let mut thread = THREAD.lock().unwrap();
    if thread.is_some() {
        return Ok(());
    }
    let (commands, receiver) = channel::<Command>();
    let (ready, started) = channel::<Result<u32, String>>();
    std::thread::Builder::new()
        .name("typr-ask-window".to_string())
        .spawn(move || {
            use windows_sys::Win32::System::Threading::GetCurrentThreadId;
            use windows_sys::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
            use windows_sys::Win32::UI::WindowsAndMessaging::{PeekMessageW, MSG, PM_NOREMOVE};
            unsafe {
                // The message queue exists before anyone posts to it
                let mut message: MSG = std::mem::zeroed();
                PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
                // Physical pixels whatever the process declared
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            }
            if let Err(e) = platform::install() {
                let _ = ready.send(Err(e));
                return;
            }
            let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
            let mut controller = view::Controller::new(on_event);
            platform::run(receiver, |command| controller.handle(command));
        })
        .map_err(|e| format!("The quick ask window's thread couldn't start: {}", e))?;
    let id = started
        .recv()
        .map_err(|_| "The quick ask window's thread stopped".to_string())??;
    *thread = Some(Thread { commands, id });
    Ok(())
}

/// Hands `command` to the window's thread (nothing happens before [`start`]).
pub fn send(command: Command) {
    let thread = THREAD.lock().unwrap();
    let Some(thread) = thread.as_ref() else { return };
    if thread.commands.send(command).is_ok() {
        use windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
        unsafe {
            PostThreadMessageW(thread.id, platform::WM_WAKE, 0, 0);
        }
    }
}

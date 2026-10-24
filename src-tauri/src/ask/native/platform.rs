//! Slint on a native window, without winit, OpenGL or a WebView.
//!
//! * The window is a layered popup like the recording indicator's
//!   (overlay.rs): per-pixel alpha, always on top, never activated
//!   (`WS_EX_NOACTIVATE`, `MA_NOACTIVATE` on clicks), hidden from the
//!   taskbar and Alt+Tab. Clicks pass through it everywhere but the card
//!   (`WS_EX_TRANSPARENT` while the cursor is elsewhere).
//! * Only the text field takes the keyboard ([`NativeWindow::take_keyboard`]):
//!   while it is open the window comes to the front, clicks activate it, and
//!   its keys and characters go to Slint; closing the field gives the
//!   keyboard back to the window that had it.
//! * Slint's software renderer draws straight into the window's bitmap
//!   (premultiplied BGRA, what `UpdateLayeredWindow` takes), only the parts
//!   that changed, and only those go to the screen.
//! * The thread's own loop runs Slint's timers and animations: 60 frames a
//!   second while something moves, nothing at all while it is still.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use i_slint_core::cursor::MouseCursorInner;
use i_slint_core::items::BuiltInMouseCursor;
use i_slint_core::window::WindowAdapterInternal;
use slint::platform::software_renderer::{PremultipliedRgbaColor, RepaintBufferType, SoftwareRenderer, TargetPixel};
use slint::platform::{Clipboard, Key, Platform, PointerEventButton, Renderer, WindowAdapter, WindowEvent};
use slint::{LogicalPosition, LogicalSize, PhysicalSize, PlatformError, SharedString, Window};
use windows_sys::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmFlush;
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, ScreenToClient, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, CreateWaitableTimerExW, GetCurrentThreadId, SetWaitableTimer,
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE, TIMER_ALL_ACCESS,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, MapVirtualKeyW, SetFocus, TrackMouseEvent, MAPVK_VSC_TO_VK_EX, TME_LEAVE,
    TRACKMOUSEEVENT, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F4, VK_HOME,
    VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LSHIFT, VK_MENU, VK_NEXT, VK_PRIOR, VK_RCONTROL, VK_RETURN, VK_RIGHT,
    VK_RSHIFT, VK_SHIFT, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetForegroundWindow,
    GetWindowLongPtrW, GetWindowThreadProcessId, IsWindow, KillTimer, LoadCursorW, MsgWaitForMultipleObjects,
    PeekMessageW, RegisterClassExW, SetCursor, SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos,
    TranslateMessage, UpdateLayeredWindowIndirect, GWL_EXSTYLE, HTCLIENT, HWND_TOPMOST, IDC_ARROW, IDC_HAND,
    IDC_IBEAM, MA_ACTIVATE, MA_NOACTIVATE, MSG, PM_REMOVE, QS_ALLINPUT, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, ULW_ALPHA, UPDATELAYEREDWINDOWINFO, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_CHAR, WM_CLOSE,
    WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEACTIVATE,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_SYSCHAR, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WM_TIMER, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

/// `TrackMouseEvent`'s message (Win32_UI_Controls in windows-sys)
const WM_MOUSELEAVE: u32 = 0x02A3;
/// Posted to the thread when a command is waiting
pub const WM_WAKE: u32 = WM_APP + 1;
/// The cursor is checked against the card this often
const HIT_TIMER: usize = 1;
const HIT_POLL_MS: u32 = 30;
/// Frames while something moves
const FRAME: Duration = Duration::from_micros(16_667);
/// One notch of the wheel scrolls this far (logical pixels), as Slint's
/// own backends do
const WHEEL_STEP: f32 = 60.0;

/// The modifiers Slint is told about, and the key that holds each down
const MODIFIERS: [(Key, VIRTUAL_KEY); 5] = [
    (Key::Shift, VK_LSHIFT),
    (Key::ShiftR, VK_RSHIFT),
    (Key::Control, VK_LCONTROL),
    (Key::ControlR, VK_RCONTROL),
    (Key::Alt, VK_MENU),
];

/// The mouse cursor Slint asks for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pointer {
    Arrow,
    Hand,
    Text,
}

/// A pixel as a layered window takes it: premultiplied BGRA.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Bgra {
    b: u8,
    g: u8,
    r: u8,
    a: u8,
}

impl TargetPixel for Bgra {
    fn blend(&mut self, color: PremultipliedRgbaColor) {
        let keep = (255 - color.alpha) as u16;
        let mix = |under: u8, over: u8| ((under as u16 * keep / 255) as u8).saturating_add(over);
        self.r = mix(self.r, color.red);
        self.g = mix(self.g, color.green);
        self.b = mix(self.b, color.blue);
        self.a = mix(self.a, color.alpha);
    }

    fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self { b, g, r, a: 255 }
    }

    /// Transparent: the window shows what is behind it
    fn background() -> Self {
        Self::default()
    }
}

/// The window's bitmap, which the renderer draws into
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut Bgra,
    width: u32,
    height: u32,
}

impl Surface {
    fn new(width: u32, height: u32) -> Option<Surface> {
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let dc = CreateCompatibleDC(screen);
            ReleaseDC(std::ptr::null_mut(), screen);
            if dc.is_null() {
                return None;
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width as i32;
            info.bmiHeader.biHeight = -(height as i32); // top-down rows
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                DeleteDC(dc);
                return None;
            }
            let previous = SelectObject(dc, bitmap);
            Some(Surface {
                dc,
                bitmap,
                previous,
                bits: bits as *mut Bgra,
                width,
                height,
            })
        }
    }

    fn pixels(&mut self) -> &mut [Bgra] {
        // The DIB section holds width × height pixels for as long as it lives
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.width * self.height) as usize) }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

/// Where the card is, in logical pixels inside the window
pub type HitArea = Box<dyn Fn() -> Option<(f32, f32, f32, f32)>>;
/// Called once Slint has seen the left button go down (true) or up
pub type ButtonHook = Box<dyn Fn(bool)>;

/// The window adapter: one native window, drawn by the software renderer.
pub struct NativeWindow {
    window: Window,
    renderer: SoftwareRenderer,
    hwnd: Cell<HWND>,
    origin: Cell<(i32, i32)>,
    size: Cell<PhysicalSize>,
    scale: Cell<f32>,
    needs_redraw: Cell<bool>,
    surface: RefCell<Option<Surface>>,
    /// The cursor wanted over the card (a TouchArea's `mouse-cursor`, the
    /// text field's I-beam)
    pointer: Cell<Pointer>,
    /// Clicks pass through the window now
    through: Cell<bool>,
    /// `WM_MOUSELEAVE` is asked for
    tracking: Cell<bool>,
    hit_area: RefCell<Option<HitArea>>,
    button_hook: RefCell<Option<ButtonHook>>,
    shown: Cell<bool>,
    /// The text field is open: the window takes the keyboard
    keyboard: Cell<bool>,
    /// The window in front before this one took the keyboard: it gets it back
    previous: Cell<HWND>,
    /// The modifiers Slint was told are down (bits of `MODIFIERS`)
    held: Cell<u8>,
    /// The first half of a character beyond the BMP, waiting for its pair
    surrogate: Cell<u16>,
    /// Activated or deactivated, for Slint to hear once the message loop
    /// has it: Windows sends `WM_ACTIVATE` while the window is being brought
    /// to the front, which may be in the middle of one of Slint's callbacks
    activation: Cell<Option<bool>>,
}

pub use crate::ask::window::Stats;

thread_local! {
    static STATS: Cell<Stats> = const { Cell::new(Stats { frames: 0, render: Duration::ZERO, present: Duration::ZERO, slowest: Duration::ZERO, area: 0, cycles: 0 }) };
}

/// What the thread has done so far; the slowest frame starts over.
pub fn stats() -> Stats {
    let mut stats = STATS.with(|stats| {
        let current = stats.get();
        stats.set(Stats {
            slowest: Duration::ZERO,
            ..current
        });
        current
    });
    stats.cycles = thread_cycles();
    stats
}

/// CPU cycles the calling thread has used
pub fn thread_cycles() -> u64 {
    #[link(name = "kernel32")]
    extern "system" {
        fn QueryThreadCycleTime(thread: HANDLE, cycles: *mut u64) -> i32;
    }
    let mut cycles = 0u64;
    unsafe {
        QueryThreadCycleTime(windows_sys::Win32::System::Threading::GetCurrentThread(), &mut cycles);
    }
    cycles
}

thread_local! {
    /// The window being drawn: there is one at a time
    static CURRENT: RefCell<Option<Rc<NativeWindow>>> = const { RefCell::new(None) };
}

/// The window Slint created last, if it is still open.
pub fn current() -> Option<Rc<NativeWindow>> {
    CURRENT.with(|current| current.borrow().clone())
}

impl NativeWindow {
    fn new() -> Rc<Self> {
        Rc::new_cyclic(|weak: &std::rc::Weak<Self>| Self {
            window: Window::new(weak.clone() as std::rc::Weak<dyn WindowAdapter>),
            renderer: SoftwareRenderer::new_with_repaint_buffer_type(RepaintBufferType::ReusedBuffer),
            hwnd: Cell::new(std::ptr::null_mut()),
            origin: Cell::new((0, 0)),
            size: Cell::new(PhysicalSize::new(1, 1)),
            scale: Cell::new(1.0),
            needs_redraw: Cell::new(true),
            surface: RefCell::new(None),
            pointer: Cell::new(Pointer::Arrow),
            through: Cell::new(true),
            tracking: Cell::new(false),
            hit_area: RefCell::new(None),
            button_hook: RefCell::new(None),
            shown: Cell::new(false),
            keyboard: Cell::new(false),
            previous: Cell::new(std::ptr::null_mut()),
            held: Cell::new(0),
            surrogate: Cell::new(0),
            activation: Cell::new(None),
        })
    }

    /// Creates the native window, hidden, at `x`, `y` (physical pixels).
    pub fn open(&self, x: i32, y: i32, width: u32, height: u32, scale: f32) -> Result<(), String> {
        let hwnd = unsafe { create_window(x, y, width as i32, height as i32)? };
        self.hwnd.set(hwnd);
        self.origin.set((x, y));
        self.scale.set(scale);
        self.window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
        self.size.set(PhysicalSize::new(width, height));
        self.window.dispatch_event(WindowEvent::Resized {
            size: LogicalSize::new(width as f32 / scale, height as f32 / scale),
        });
        let surface = Surface::new(width, height).ok_or("The quick ask window's bitmap couldn't be created")?;
        *self.surface.borrow_mut() = Some(surface);
        self.needs_redraw.set(true);
        Ok(())
    }

    /// The card, the only part that takes clicks
    pub fn set_hit_area(&self, area: HitArea) {
        *self.hit_area.borrow_mut() = Some(area);
    }

    /// Told about the left button after Slint has handled it
    pub fn set_button_hook(&self, hook: ButtonHook) {
        *self.button_hook.borrow_mut() = Some(hook);
    }

    /// Draws what changed and puts it on screen. Returns whether it drew.
    pub fn render(&self) -> bool {
        if !self.needs_redraw.replace(false) {
            return false;
        }
        let mut surface = self.surface.borrow_mut();
        let Some(surface) = surface.as_mut() else {
            return false;
        };
        let started = Instant::now();
        let width = surface.width as usize;
        let region = self.renderer.render(surface.pixels(), width);
        let rendered = started.elapsed();

        let origin = region.bounding_box_origin();
        let size = region.bounding_box_size();
        if size.width > 0 && size.height > 0 {
            let dirty = RECT {
                left: origin.x,
                top: origin.y,
                right: origin.x + size.width as i32,
                bottom: origin.y + size.height as i32,
            };
            self.present(surface, Some(dirty));
        }
        let total = started.elapsed();
        STATS.with(|stats| {
            let mut s = stats.get();
            s.frames += 1;
            s.render += rendered;
            s.present += total - rendered;
            s.slowest = s.slowest.max(total);
            s.area += size.width as u64 * size.height as u64;
            stats.set(s);
        });
        true
    }

    fn present(&self, surface: &Surface, dirty: Option<RECT>) {
        let hwnd = self.hwnd.get();
        if hwnd.is_null() {
            return;
        }
        let (x, y) = self.origin.get();
        let position = POINT { x, y };
        let size = SIZE {
            cx: surface.width as i32,
            cy: surface.height as i32,
        };
        let source = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let info = UPDATELAYEREDWINDOWINFO {
            cbSize: std::mem::size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
            hdcDst: std::ptr::null_mut(),
            pptDst: &position,
            psize: &size,
            hdcSrc: surface.dc,
            pptSrc: &source,
            crKey: 0,
            pblend: &blend,
            dwFlags: ULW_ALPHA,
            prcDirty: dirty.as_ref().map_or(std::ptr::null(), |rect| rect as *const RECT),
        };
        unsafe {
            UpdateLayeredWindowIndirect(hwnd, &info);
        }
    }

    /// The window's pixels as they are on screen now: straight RGBA, rows
    /// top down (for the demo's snapshots).
    pub fn pixels(&self) -> Option<(u32, u32, Vec<u8>)> {
        let mut surface = self.surface.borrow_mut();
        let surface = surface.as_mut()?;
        let (width, height) = (surface.width, surface.height);
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for pixel in surface.pixels().iter() {
            let unpremultiply = |c: u8| if pixel.a == 0 { 0 } else { (c as u16 * 255 / pixel.a as u16).min(255) as u8 };
            rgba.extend_from_slice(&[unpremultiply(pixel.r), unpremultiply(pixel.g), unpremultiply(pixel.b), pixel.a]);
        }
        Some((width, height, rgba))
    }

    /// Shows the window above all others without activating it and waits
    /// until the desktop has composed it: its first frame is on screen.
    pub fn show(&self) {
        let hwnd = self.hwnd.get();
        if hwnd.is_null() || self.shown.replace(true) {
            return;
        }
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            SetTimer(hwnd, HIT_TIMER, HIT_POLL_MS, None);
            DwmFlush();
        }
    }

    /// Destroys the native window; Slint's side goes with the component.
    pub fn destroy(&self) {
        // The app that had the keyboard gets it back, not whatever is next
        self.release_keyboard();
        let hwnd = self.hwnd.replace(std::ptr::null_mut());
        if !hwnd.is_null() {
            unsafe {
                KillTimer(hwnd, HIT_TIMER);
                DestroyWindow(hwnd);
            }
        }
        self.surface.borrow_mut().take();
        self.hit_area.borrow_mut().take();
        self.button_hook.borrow_mut().take();
        self.shown.set(false);
        CURRENT.with(|current| {
            let mut current = current.borrow_mut();
            if current.as_ref().is_some_and(|window| std::ptr::eq(Rc::as_ptr(window), self)) {
                current.take();
            }
        });
    }

    fn logical(&self, x: i32, y: i32) -> LogicalPosition {
        let scale = self.scale.get();
        LogicalPosition::new(x as f32 / scale, y as f32 / scale)
    }

    /// Clicks pass through everywhere but the card.
    fn poll_hit(&self) {
        let hwnd = self.hwnd.get();
        let mut cursor = POINT { x: 0, y: 0 };
        if hwnd.is_null() || unsafe { GetCursorPos(&mut cursor) } == 0 {
            return;
        }
        let (x, y) = self.origin.get();
        let point = self.logical(cursor.x - x, cursor.y - y);
        let on_card = self.hit_area.borrow().as_ref().and_then(|area| area()).is_some_and(|(cx, cy, cw, ch)| {
            point.x >= cx && point.x <= cx + cw && point.y >= cy && point.y <= cy + ch
        });
        let through = !on_card;
        if self.through.replace(through) == through {
            return;
        }
        unsafe { set_ex_style(hwnd, WS_EX_TRANSPARENT, through) };
        if through {
            // No more mouse messages until it is back on the card
            self.tracking.set(false);
            self.window.dispatch_event(WindowEvent::PointerExited);
        }
    }

    fn update_cursor(&self) {
        let cursor = match self.pointer.get() {
            Pointer::Arrow => IDC_ARROW,
            Pointer::Hand => IDC_HAND,
            Pointer::Text => IDC_IBEAM,
        };
        unsafe {
            SetCursor(LoadCursorW(std::ptr::null_mut(), cursor));
        }
    }

    // ── The keyboard (the text field) ────────────────────

    /// The window comes to the front and takes the keyboard; until
    /// [`release_keyboard`](Self::release_keyboard), clicks on it activate it.
    pub fn take_keyboard(&self) {
        let hwnd = self.hwnd.get();
        if hwnd.is_null() {
            return;
        }
        self.keyboard.set(true);
        unsafe {
            set_ex_style(hwnd, WS_EX_NOACTIVATE, false);
            let front = GetForegroundWindow();
            if front == hwnd {
                return;
            }
            self.previous.set(front);
            // Called on a click on the window, so Windows lets it come to
            // the front; should it not, the thread in front lends its input
            // state for a moment
            if SetForegroundWindow(hwnd) == 0 || GetForegroundWindow() != hwnd {
                let own = GetCurrentThreadId();
                let other = GetWindowThreadProcessId(front, std::ptr::null_mut());
                if other != 0 && other != own && AttachThreadInput(own, other, 1) != 0 {
                    SetForegroundWindow(hwnd);
                    AttachThreadInput(own, other, 0);
                }
            }
            SetFocus(hwnd);
        }
    }

    /// The window is never activated again, and gives the keyboard back to
    /// the window that had it, if it still has it.
    pub fn release_keyboard(&self) {
        if !self.keyboard.replace(false) {
            return;
        }
        let hwnd = self.hwnd.get();
        let previous = self.previous.replace(std::ptr::null_mut());
        if hwnd.is_null() {
            return;
        }
        unsafe {
            set_ex_style(hwnd, WS_EX_NOACTIVATE, true);
            if GetForegroundWindow() == hwnd && !previous.is_null() && IsWindow(previous) != 0 {
                SetForegroundWindow(previous);
            }
        }
    }

    /// Tells Slint the window was activated or deactivated (outside its
    /// callbacks). Keys let go in another window never come here: Slint
    /// forgets the modifiers held.
    fn settle_activation(&self) {
        let Some(active) = self.activation.take() else { return };
        if !active {
            self.release_modifiers(false);
        }
        self.window.dispatch_event(WindowEvent::WindowActiveChanged(active));
    }

    /// Types `text` into the field with the focus, as if it was typed there:
    /// at the caret, over the selection, undoable. A modifier whose key came
    /// up elsewhere is let go first, or it would turn the text into a
    /// shortcut.
    pub fn type_text(&self, text: &str) {
        self.release_modifiers(true);
        let text = SharedString::from(text);
        self.window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        self.window.dispatch_event(WindowEvent::KeyReleased { text });
    }

    /// Tells Slint about a key going down or up, keeping track of the
    /// modifiers it thinks are held.
    fn key(&self, key: Key, down: bool, repeat: bool) {
        if let Some(index) = MODIFIERS.iter().position(|(modifier, _)| *modifier == key) {
            let bit = 1u8 << index;
            let held = self.held.get();
            if down == (held & bit != 0) {
                return; // a repeat, or an up Slint never saw going down
            }
            self.held.set(held ^ bit);
        }
        let text = SharedString::from(key);
        self.window.dispatch_event(match (down, repeat) {
            (true, false) => WindowEvent::KeyPressed { text },
            (true, true) => WindowEvent::KeyPressRepeated { text },
            (false, _) => WindowEvent::KeyReleased { text },
        });
    }

    /// Lets go of the modifiers Slint thinks are held: all of them, or only
    /// those whose key is up by now (`stale`).
    fn release_modifiers(&self, stale: bool) {
        for (index, (key, vk)) in MODIFIERS.into_iter().enumerate() {
            let down = unsafe { GetAsyncKeyState(vk as i32) } < 0;
            if self.held.get() & (1 << index) != 0 && !(stale && down) {
                self.key(key, false, false);
            }
        }
    }

    /// A key went down or up: those that type no text of their own, and
    /// letters and digits with Ctrl (the shortcuts, whatever the layout).
    fn on_key(&self, vk: VIRTUAL_KEY, lparam: LPARAM, down: bool) {
        let repeat = down && lparam & (1 << 30) != 0;
        if let Some(key) = named_key(vk, lparam) {
            self.key(key, down, repeat);
            return;
        }
        let ctrl = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
        let alt = unsafe { GetKeyState(VK_MENU as i32) } < 0;
        let letter = (b'A' as u16..=b'Z' as u16).contains(&vk) || (b'0' as u16..=b'9' as u16).contains(&vk);
        // With Alt too it is AltGr, which types a character
        if ctrl && !alt && letter {
            let text = SharedString::from((vk as u8 as char).to_ascii_lowercase().to_string());
            self.window.dispatch_event(match (down, repeat) {
                (true, false) => WindowEvent::KeyPressed { text },
                (true, true) => WindowEvent::KeyPressRepeated { text },
                (false, _) => WindowEvent::KeyReleased { text },
            });
        }
    }

    /// A character typed (`WM_CHAR`, UTF-16, maybe in two halves). Control
    /// characters came as their keys already.
    fn on_char(&self, unit: u16) {
        let text = match unit {
            0xD800..=0xDBFF => {
                self.surrogate.set(unit);
                return;
            }
            0xDC00..=0xDFFF => String::from_utf16(&[self.surrogate.replace(0), unit]).ok(),
            _ => char::from_u32(unit as u32).map(String::from),
        };
        let Some(text) = text.filter(|text| !text.chars().any(char::is_control)) else {
            return;
        };
        let text = SharedString::from(text);
        self.window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        self.window.dispatch_event(WindowEvent::KeyReleased { text });
    }
}

/// The Slint key for a virtual key that types no text of its own
fn named_key(vk: VIRTUAL_KEY, lparam: LPARAM) -> Option<Key> {
    let extended = lparam & (1 << 24) != 0;
    Some(match vk {
        VK_BACK => Key::Backspace,
        VK_TAB => Key::Tab,
        VK_RETURN => Key::Return,
        VK_ESCAPE => Key::Escape,
        VK_DELETE => Key::Delete,
        VK_INSERT => Key::Insert,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        VK_PRIOR => Key::PageUp,
        VK_NEXT => Key::PageDown,
        VK_LEFT => Key::LeftArrow,
        VK_RIGHT => Key::RightArrow,
        VK_UP => Key::UpArrow,
        VK_DOWN => Key::DownArrow,
        VK_SHIFT => {
            let scan = ((lparam >> 16) & 0xff) as u32;
            if unsafe { MapVirtualKeyW(scan, MAPVK_VSC_TO_VK_EX) } == VK_RSHIFT as u32 {
                Key::ShiftR
            } else {
                Key::Shift
            }
        }
        VK_CONTROL if extended => Key::ControlR,
        VK_CONTROL => Key::Control,
        VK_MENU => Key::Alt,
        _ => return None,
    })
}

/// Turns an extended window style on or off.
unsafe fn set_ex_style(hwnd: HWND, flag: u32, on: bool) {
    let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    let style = if on { style | flag } else { style & !flag };
    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style as isize);
}

impl WindowAdapter for NativeWindow {
    fn window(&self) -> &Window {
        &self.window
    }

    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }

    fn size(&self) -> PhysicalSize {
        self.size.get()
    }

    fn request_redraw(&self) {
        self.needs_redraw.set(true);
    }

    fn internal(&self, _: i_slint_core::InternalToken) -> Option<&dyn WindowAdapterInternal> {
        Some(self)
    }
}

impl WindowAdapterInternal for NativeWindow {
    fn set_mouse_cursor(&self, cursor: MouseCursorInner) {
        let pointer = match cursor {
            MouseCursorInner::BuiltIn(BuiltInMouseCursor::Pointer) => Pointer::Hand,
            MouseCursorInner::BuiltIn(BuiltInMouseCursor::Text) => Pointer::Text,
            _ => Pointer::Arrow,
        };
        if self.pointer.replace(pointer) != pointer && self.tracking.get() {
            self.update_cursor();
        }
    }
}

struct AskPlatform;

impl Platform for AskPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = NativeWindow::new();
        CURRENT.with(|current| *current.borrow_mut() = Some(window.clone()));
        Ok(window)
    }

    /// Ctrl+C and Ctrl+X in the text field
    fn set_clipboard_text(&self, text: &str, clipboard: Clipboard) {
        if clipboard != Clipboard::DefaultClipboard {
            return;
        }
        if let Err(e) = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text)) {
            log::warn!("Copying from the quick ask field failed: {}", e);
        }
    }

    /// Ctrl+V in the text field
    fn clipboard_text(&self, clipboard: Clipboard) -> Option<String> {
        if clipboard != Clipboard::DefaultClipboard {
            return None;
        }
        arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()).ok()
    }
}

/// Makes Slint draw through this module, on the calling thread.
pub fn install() -> Result<(), String> {
    slint::platform::set_platform(Box::new(AskPlatform)).map_err(|e| format!("Slint's platform couldn't be set: {}", e))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn create_window(x: i32, y: i32, width: i32, height: i32) -> Result<HWND, String> {
    let class_name = wide("TyprQuickAsk");
    let instance = GetModuleHandleW(std::ptr::null());
    thread_local! {
        static REGISTERED: Cell<bool> = const { Cell::new(false) };
    }
    if !REGISTERED.with(|registered| registered.get()) {
        let mut class: WNDCLASSEXW = std::mem::zeroed();
        class.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        class.lpfnWndProc = Some(window_proc);
        class.hInstance = instance;
        class.lpszClassName = class_name.as_ptr();
        if RegisterClassExW(&class) == 0 {
            return Err("The quick ask window class couldn't be registered".to_string());
        }
        REGISTERED.with(|registered| registered.set(true));
    }
    // Clicks pass through until the cursor is on the card
    let hwnd = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        class_name.as_ptr(),
        wide("Typr").as_ptr(),
        WS_POPUP,
        x,
        y,
        width,
        height,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        instance,
        std::ptr::null(),
    );
    if hwnd.is_null() {
        return Err("The quick ask window couldn't be created".to_string());
    }
    Ok(hwnd)
}

fn low_word(value: isize) -> i32 {
    (value & 0xffff) as i16 as i32
}

fn high_word(value: isize) -> i32 {
    ((value >> 16) & 0xffff) as i16 as i32
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(window) = current().filter(|window| window.hwnd.get() == hwnd) else {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    };
    match message {
        // A click never activates it: the app the question is about keeps
        // the focus and its selection. Only while the text field is open
        // a click gives it the keyboard
        WM_MOUSEACTIVATE if window.keyboard.get() => {
            let front = GetForegroundWindow();
            if front != hwnd {
                window.previous.set(front);
            }
            MA_ACTIVATE as LRESULT
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_ACTIVATE => {
            window.activation.set(Some((wparam & 0xffff) as u32 != WA_INACTIVE));
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_KEYDOWN | WM_KEYUP => {
            window.on_key(wparam as VIRTUAL_KEY, lparam, message == WM_KEYDOWN);
            0
        }
        // Alt+F4 closes the window (WM_CLOSE below); no other key with Alt
        // reaches the system, which would open a menu the window hasn't got
        WM_SYSKEYDOWN if wparam as VIRTUAL_KEY == VK_F4 => DefWindowProcW(hwnd, message, wparam, lparam),
        WM_SYSKEYDOWN | WM_SYSKEYUP => {
            window.on_key(wparam as VIRTUAL_KEY, lparam, message == WM_SYSKEYDOWN);
            0
        }
        WM_CHAR => {
            window.on_char(wparam as u16);
            0
        }
        WM_SYSCHAR => 0,
        // The window says what closing means (view.rs); it is never just destroyed
        WM_CLOSE => {
            window.window.dispatch_event(WindowEvent::CloseRequested);
            0
        }
        WM_MOUSEMOVE => {
            if !window.tracking.replace(true) {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut track);
            }
            let position = window.logical(low_word(lparam), high_word(lparam));
            window.window.dispatch_event(WindowEvent::PointerMoved { position });
            0
        }
        WM_MOUSELEAVE => {
            window.tracking.set(false);
            window.window.dispatch_event(WindowEvent::PointerExited);
            0
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP => {
            let position = window.logical(low_word(lparam), high_word(lparam));
            let button = PointerEventButton::Left;
            let down = message == WM_LBUTTONDOWN;
            window.window.dispatch_event(if down {
                WindowEvent::PointerPressed { position, button }
            } else {
                WindowEvent::PointerReleased { position, button }
            });
            if let Some(hook) = window.button_hook.borrow().as_ref() {
                hook(down);
            }
            0
        }
        WM_MOUSEWHEEL => {
            // Screen coordinates
            let mut point = POINT {
                x: low_word(lparam),
                y: high_word(lparam),
            };
            ScreenToClient(hwnd, &mut point);
            let notches = high_word(wparam as isize) as f32 / 120.0;
            window.window.dispatch_event(WindowEvent::PointerScrolled {
                position: window.logical(point.x, point.y),
                delta_x: 0.0,
                delta_y: notches * WHEEL_STEP,
            });
            0
        }
        // No menu, and the side buttons never reach anything
        WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP => 0,
        WM_XBUTTONDOWN | WM_XBUTTONUP => 1,
        WM_SETCURSOR if low_word(lparam) as u32 == HTCLIENT => {
            window.update_cursor();
            1
        }
        WM_TIMER if wparam == HIT_TIMER => {
            window.poll_hit();
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// The thread's loop: window messages, `commands`, Slint's timers and
/// animations, drawing. Returns when the sender of `commands` is gone.
pub fn run<C>(commands: Receiver<C>, mut handle: impl FnMut(C)) {
    // Frames are paced by a precise timer (Windows 10 1803+), else by the
    // message wait's own timeout
    let timer: HANDLE = unsafe {
        CreateWaitableTimerExW(
            std::ptr::null(),
            std::ptr::null(),
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
            TIMER_ALL_ACCESS,
        )
    };
    let mut last_frame = Instant::now();
    loop {
        unsafe {
            let mut message: MSG = std::mem::zeroed();
            while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if message.message == WM_QUIT {
                    return;
                }
                if message.hwnd.is_null() {
                    continue; // WM_WAKE
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        loop {
            match commands.try_recv() {
                Ok(command) => handle(command),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        let window = current();
        if let Some(window) = &window {
            window.settle_activation();
        }
        slint::platform::update_timers_and_animations();
        if let Some(window) = &window {
            if window.render() {
                last_frame = Instant::now();
            }
        }

        let animating = window.as_ref().is_some_and(|w| w.shown.get() && w.window.has_active_animations());
        let frame = animating.then(|| FRAME.saturating_sub(last_frame.elapsed()));
        let wait = match (frame, slint::platform::duration_until_next_timer_update()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        wait_for(timer, wait);
    }
}

/// Sleeps until a message or command arrives, or `wait` is over.
fn wait_for(timer: HANDLE, wait: Option<Duration>) {
    unsafe {
        match wait {
            Some(wait) if wait.is_zero() => {}
            Some(wait) if !timer.is_null() => {
                let due = -((wait.as_nanos() / 100).max(1) as i64);
                SetWaitableTimer(timer, &due, 0, None, std::ptr::null(), 0);
                MsgWaitForMultipleObjects(1, &timer, 0, INFINITE, QS_ALLINPUT);
            }
            Some(wait) => {
                MsgWaitForMultipleObjects(0, std::ptr::null(), 0, wait.as_millis().max(1) as u32, QS_ALLINPUT);
            }
            None => {
                MsgWaitForMultipleObjects(0, std::ptr::null(), 0, INFINITE, QS_ALLINPUT);
            }
        }
    }
}

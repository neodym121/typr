//! Slint on a native window, without winit, OpenGL or a WebView.
//!
//! * The window is a layered popup like the recording indicator's
//!   (overlay.rs): per-pixel alpha, always on top, never activated
//!   (`WS_EX_NOACTIVATE`, `MA_NOACTIVATE` on clicks), hidden from the
//!   taskbar and Alt+Tab. Clicks pass through it everywhere but the card
//!   (`WS_EX_TRANSPARENT` while the cursor is elsewhere).
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
use slint::platform::{Platform, PointerEventButton, Renderer, WindowAdapter, WindowEvent};
use slint::{LogicalPosition, LogicalSize, PhysicalSize, PlatformError, Window};
use windows_sys::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmFlush;
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, ScreenToClient, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    CreateWaitableTimerExW, SetWaitableTimer, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE, TIMER_ALL_ACCESS,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetWindowLongPtrW, KillTimer,
    LoadCursorW, MsgWaitForMultipleObjects, PeekMessageW, RegisterClassExW, SetCursor, SetTimer, SetWindowLongPtrW,
    SetWindowPos, TranslateMessage, UpdateLayeredWindowIndirect, GWL_EXSTYLE, HTCLIENT, HWND_TOPMOST, IDC_ARROW,
    IDC_HAND, MA_NOACTIVATE, MSG, PM_REMOVE, QS_ALLINPUT, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    ULW_ALPHA, UPDATELAYEREDWINDOWINFO, WM_APP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SETCURSOR, WM_TIMER, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
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
    /// The pointer cursor is wanted (a TouchArea with `mouse-cursor: pointer`)
    hand: Cell<bool>,
    /// Clicks pass through the window now
    through: Cell<bool>,
    /// `WM_MOUSELEAVE` is asked for
    tracking: Cell<bool>,
    hit_area: RefCell<Option<HitArea>>,
    button_hook: RefCell<Option<ButtonHook>>,
    shown: Cell<bool>,
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
            hand: Cell::new(false),
            through: Cell::new(true),
            tracking: Cell::new(false),
            hit_area: RefCell::new(None),
            button_hook: RefCell::new(None),
            shown: Cell::new(false),
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
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            let style = if through {
                style | WS_EX_TRANSPARENT
            } else {
                style & !WS_EX_TRANSPARENT
            };
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style as isize);
        }
        if through {
            // No more mouse messages until it is back on the card
            self.tracking.set(false);
            self.window.dispatch_event(WindowEvent::PointerExited);
        }
    }

    fn update_cursor(&self) {
        unsafe {
            SetCursor(LoadCursorW(
                std::ptr::null_mut(),
                if self.hand.get() { IDC_HAND } else { IDC_ARROW },
            ));
        }
    }
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
        let hand = cursor == MouseCursorInner::BuiltIn(BuiltInMouseCursor::Pointer);
        if self.hand.replace(hand) != hand && self.tracking.get() {
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
        // the focus and its selection
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
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

        slint::platform::update_timers_and_animations();
        let window = current();
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

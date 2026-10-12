//! Recording indicator on the primary screen: in the top-right corner, or at
//! the bottom centre just above the taskbar (`indicatorPosition`).
//!
//! It waits out of sight behind the top edge of the screen (or behind the
//! taskbar) and slides in only while Typr records, transcribes and pastes (or
//! briefly after a failed dictation), then slides back out.
//!
//! On Windows it is a native click-through layered window drawn in Rust, so
//! Typr needs no WebView (and no browser process) while it sits in the tray.
//! Other platforms keep the original WebView overlay (`src/overlay.html`).

use crate::settings::IndicatorPosition;
use std::f32::consts::{FRAC_PI_4, PI, TAU};
use tauri::AppHandle;

/// What the indicator shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indicator {
    Idle,
    Recording,
    Transcribing,
}

/// A rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Where the indicator goes, derived from the primary screen's work area
/// (the screen without the taskbar).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub position: IndicatorPosition,
    /// Window rectangle in physical pixels. It touches the edge the indicator
    /// slides in from: the top of the work area, or its bottom (the taskbar).
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f64,
    /// Top-left corner of the window in logical pixels (used by the WebView overlay)
    pub logical_x: f64,
    pub logical_y: f64,
}

/// Square around the disc in logical pixels, with room for its shadow and halo
const INDICATOR_SIZE: f64 = 50.0;
/// Top right: from the right edge of the screen to the left side of the square
const OFFSET_FROM_RIGHT: f64 = 60.0;
/// Gap between the edge the indicator slides in from and the indicator in place
const OFFSET_FROM_EDGE: f64 = 10.0;
/// Work area assumed when the screen can't be read
const FALLBACK_AREA: ScreenRect = ScreenRect {
    x: 0,
    y: 0,
    width: 1440,
    height: 900,
};

impl Placement {
    /// `work_area` is the primary screen without the taskbar, in physical pixels.
    pub fn new(position: IndicatorPosition, work_area: ScreenRect, scale: f64) -> Self {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let width = (INDICATOR_SIZE * scale).round().max(1.0) as i32;
        let height = ((OFFSET_FROM_EDGE + INDICATOR_SIZE) * scale).round().max(1.0) as i32;
        let (x, y) = match position {
            IndicatorPosition::TopRight => (
                work_area.x + work_area.width - (OFFSET_FROM_RIGHT * scale).round() as i32,
                work_area.y,
            ),
            IndicatorPosition::BottomCenter => (
                work_area.x + (work_area.width - width) / 2,
                work_area.y + work_area.height - height,
            ),
        };
        Self {
            position,
            x,
            y,
            width,
            height,
            scale,
            logical_x: x as f64 / scale,
            logical_y: y as f64 / scale,
        }
    }

    pub fn fallback(position: IndicatorPosition) -> Self {
        Self::new(position, FALLBACK_AREA, 1.0)
    }

    /// Vertical centre of the disc inside the window in physical pixels:
    /// `shown` 0 is fully hidden beyond the window's edge (above it at the
    /// top, below it at the bottom), 1 is in place.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    fn center_y(&self, shown: f32) -> f32 {
        let s = self.scale as f32;
        let half = (INDICATOR_SIZE / 2.0) as f32 * s;
        let inset = (OFFSET_FROM_EDGE + INDICATOR_SIZE / 2.0) as f32 * s;
        let (hidden, in_place) = match self.position {
            IndicatorPosition::TopRight => (-half, inset),
            IndicatorPosition::BottomCenter => {
                let bottom = self.height as f32;
                (bottom + half, bottom - inset)
            }
        };
        hidden + (in_place - hidden) * shown
    }

    /// Centre of the disc in place, in physical screen pixels. The quick ask
    /// window grows out of exactly this point.
    pub fn dot_center(&self) -> (f64, f64) {
        let s = self.scale;
        let half = INDICATOR_SIZE / 2.0 * s;
        let inset = (OFFSET_FROM_EDGE + INDICATOR_SIZE / 2.0) * s;
        let y = match self.position {
            IndicatorPosition::TopRight => self.y as f64 + inset,
            IndicatorPosition::BottomCenter => (self.y + self.height) as f64 - inset,
        };
        (self.x as f64 + half, y)
    }
}

pub fn create(app: &AppHandle, position: IndicatorPosition) {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        native::create(position);
    }
    #[cfg(not(target_os = "windows"))]
    webview::create(app, position);
}

/// Moves the indicator to the top-right corner or the bottom centre.
pub fn set_position(app: &AppHandle, position: IndicatorPosition) {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        native::set_position(position);
    }
    #[cfg(not(target_os = "windows"))]
    webview::set_position(app, position);
}

pub fn set_indicator(app: &AppHandle, indicator: Indicator) {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        native::set_indicator(indicator);
    }
    #[cfg(not(target_os = "windows"))]
    webview::set_indicator(app, indicator);
}

/// Briefly shows the error look (an accent ring) after a failed dictation.
pub fn flash_error(app: &AppHandle) {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        native::flash_error();
    }
    #[cfg(not(target_os = "windows"))]
    webview::flash_error(app);
}

/// Allows or forbids the indicator. While forbidden it never appears: it is
/// switched off in General, or the hotkey is turned off from the tray.
pub fn set_enabled(app: &AppHandle, enabled: bool) {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        native::set_enabled(enabled);
    }
    #[cfg(not(target_os = "windows"))]
    webview::set_enabled(app, enabled);
}

/// Hides the indicator at once and keeps it away while the quick ask window
/// is open: the window took the dot's place, and its own dot listens.
pub fn set_suppressed(app: &AppHandle, suppressed: bool) {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        native::set_suppressed(suppressed);
    }
    #[cfg(not(target_os = "windows"))]
    webview::set_suppressed(app, suppressed);
}

// ── Rendering ───────────────────────────────────────────
// Everything is drawn with signed distance functions, which gives
// anti-aliased edges at any display scale without a graphics library.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
enum Look {
    Idle,
    Recording,
    Transcribing,
    Error,
}

#[derive(Clone, Copy)]
struct Paint {
    rgb: [f32; 3],
    alpha: f32,
}

fn paint(r: u8, g: u8, b: u8, alpha: f32) -> Paint {
    Paint {
        rgb: [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0],
        alpha,
    }
}

/// Renders one frame as premultiplied BGRA, top-down rows, `width` × `height`
/// pixels, with the disc centred horizontally and its centre at `center_y`
/// (outside the frame while it hides beyond the edge).
/// `t` is the time in seconds since the current look appeared (drives animations).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn render(look: Look, t: f32, width: usize, height: usize, center_y: f32, scale: f32) -> Vec<u8> {
    let mut frame = vec![0u8; width * height * 4];
    let s = scale.max(0.5);
    let center_x = width as f32 / 2.0;
    let radius = 20.0 * s;
    // Shadow and halo reach at most 4.5 logical px past the disc
    let reach = radius + 8.0 * s;

    let dark = paint(31, 30, 29, 0.94);
    let accent = paint(217, 119, 87, 1.0);
    let light = paint(248, 248, 246, 1.0);
    let muted = paint(156, 154, 146, 1.0);

    // fill, border, icon, optional halo (paint, extra radius in logical px)
    let (fill, border, icon, halo) = match look {
        Look::Idle => (dark, paint(248, 248, 246, 0.14), muted, None),
        Look::Recording => {
            // Ring that grows and fades, then comes back (1.4 s cycle)
            let phase = (t / 1.4).fract();
            let wave = if phase < 0.5 { phase * 2.0 } else { (1.0 - phase) * 2.0 };
            (
                accent,
                paint(248, 248, 246, 0.22),
                light,
                Some((paint(217, 119, 87, 0.5 * (1.0 - wave)), 4.0 * wave)),
            )
        }
        Look::Transcribing => (dark, paint(248, 248, 246, 0.2), light, None),
        Look::Error => (dark, accent, accent, Some((paint(217, 119, 87, 0.35), 3.0))),
    };
    let spinner_head = t * TAU / 0.9;

    for py in 0..height {
        let y = py as f32 + 0.5;
        let dy = y - center_y;
        if dy.abs() > reach {
            continue; // nothing drawn on this row, it stays transparent
        }
        for px in 0..width {
            let x = px as f32 + 0.5;
            let dx = x - center_x;
            let dist = (dx * dx + dy * dy).sqrt();
            let mut pixel = [0.0f32; 4];

            // Soft shadow, slightly below the disc
            let shadow_dist = (dx * dx + (dy - 2.0 * s).powi(2)).sqrt();
            let shadow = 0.28 * ((radius + 2.0 * s - shadow_dist) / (5.0 * s)).clamp(0.0, 1.0);
            over(&mut pixel, [0.0, 0.0, 0.0], shadow);

            if let Some((glow, spread)) = halo {
                over(&mut pixel, glow.rgb, glow.alpha * coverage(dist - (radius + spread * s)));
            }

            over(&mut pixel, fill.rgb, fill.alpha * coverage(dist - radius));

            // 1px border just inside the edge
            let border_sdf = (dist - radius).max(radius - s - dist);
            over(&mut pixel, border.rgb, border.alpha * coverage(border_sdf));

            if look == Look::Transcribing {
                // Quarter arc spinning around the edge (0.9 s per turn)
                let ring = (dist - radius).abs() - s;
                let angle = dx.atan2(-dy);
                let offset = wrap_angle(angle - spinner_head);
                let sector = (offset.abs() - FRAC_PI_4) * dist;
                over(&mut pixel, light.rgb, coverage(ring.max(sector)));
            }

            // Microphone glyph: 24-unit icon drawn 18 logical px wide, centred
            let glyph = mic_sdf(dx / s / 0.75 + 12.0, dy / s / 0.75 + 12.0) * 0.75 * s;
            over(&mut pixel, icon.rgb, icon.alpha * coverage(glyph));

            let i = (py * width + px) * 4;
            frame[i] = to_byte(pixel[2]);
            frame[i + 1] = to_byte(pixel[1]);
            frame[i + 2] = to_byte(pixel[0]);
            frame[i + 3] = to_byte(pixel[3]);
        }
    }

    frame
}

/// Distance to the microphone shape in its 24×24 icon space (negative inside).
fn mic_sdf(x: f32, y: f32) -> f32 {
    // Capsule body: segment (12,5)–(12,11), radius 3
    let body_y = y.clamp(5.0, 11.0);
    let body = ((x - 12.0).powi(2) + (y - body_y).powi(2)).sqrt() - 3.0;

    // Stand: lower half of a ring around (12,11), radius 6, 2 units thick
    let ring = (((x - 12.0).powi(2) + (y - 11.0).powi(2)).sqrt() - 6.0).abs() - 1.0;
    let stand = ring.max(11.0 - y);

    // Stem: box centred at (12,19), half size 1×2
    let qx = (x - 12.0).abs() - 1.0;
    let qy = (y - 19.0).abs() - 2.0;
    let stem = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0);

    body.min(stand).min(stem)
}

/// Pixel coverage from a signed distance in physical pixels.
fn coverage(sdf: f32) -> f32 {
    (0.5 - sdf).clamp(0.0, 1.0)
}

/// Premultiplied source-over blending.
fn over(pixel: &mut [f32; 4], rgb: [f32; 3], alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let alpha = alpha.min(1.0);
    let keep = 1.0 - alpha;
    pixel[0] = rgb[0] * alpha + pixel[0] * keep;
    pixel[1] = rgb[1] * alpha + pixel[1] * keep;
    pixel[2] = rgb[2] * alpha + pixel[2] * keep;
    pixel[3] = alpha + pixel[3] * keep;
}

fn wrap_angle(angle: f32) -> f32 {
    let mut a = angle % TAU;
    if a > PI {
        a -= TAU;
    } else if a < -PI {
        a += TAU;
    }
    a
}

fn to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

// ── Windows: native layered window ──────────────────────

#[cfg(target_os = "windows")]
mod native {
    use super::{render, Indicator, IndicatorPosition, Look, Placement, ScreenRect};
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, AtomicU8, Ordering};
    use std::sync::Mutex;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW,
        MonitorFromPoint, ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO,
        BITMAPINFOHEADER, BLENDFUNCTION, DIB_RGB_COLORS, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, PostMessageW,
        RegisterClassExW, SetTimer, ShowWindow, TranslateMessage, UpdateLayeredWindow, MSG,
        SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_APP, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };

    const WM_REFRESH: u32 = WM_APP + 1;
    const ANIMATION_TIMER: usize = 1;
    const ERROR_TIMER: usize = 2;
    /// Frame interval while sliding in or out
    const SLIDE_FRAME_MS: u32 = 16;
    /// Frame interval while in place and only the look animates
    const FRAME_MS: u32 = 33;
    /// How long sliding in or out takes
    const SLIDE_SECS: f32 = 0.22;
    const ERROR_MS: u64 = 2500;

    const STATE_IDLE: u8 = 0;
    const STATE_RECORDING: u8 = 1;
    const STATE_TRANSCRIBING: u8 = 2;

    // Written from any thread; the window thread reads them on WM_REFRESH
    static WINDOW: AtomicIsize = AtomicIsize::new(0);
    static STATE: AtomicU8 = AtomicU8::new(STATE_IDLE);
    static ENABLED: AtomicBool = AtomicBool::new(true);
    static SUPPRESSED: AtomicBool = AtomicBool::new(false);
    static ERROR_UNTIL_MS: AtomicU64 = AtomicU64::new(0);
    static POSITION: Mutex<IndicatorPosition> = Mutex::new(IndicatorPosition::TopRight);

    thread_local! {
        // Worked out again each time the indicator appears
        static PLACEMENT: Cell<Option<Placement>> = Cell::new(None);
        // The look on screen and when it appeared (for animations)
        static SHOWN: Cell<(Look, Instant)> = Cell::new((Look::Idle, Instant::now()));
        // How far the indicator has slid in (0 hidden, 1 in place) and when it
        // last moved; `None` while it rests
        static SLIDE: Cell<(f32, Option<Instant>)> = Cell::new((0.0, None));
        // Whether the window is shown at all (it is hidden once fully slid out)
        static ON_SCREEN: Cell<bool> = Cell::new(false);
    }

    pub fn create(position: IndicatorPosition) {
        *POSITION.lock().unwrap() = position;
        let spawned = std::thread::Builder::new()
            .name("typr-overlay".to_string())
            .spawn(|| unsafe { run() });
        if let Err(e) = spawned {
            log::error!("Failed to start the overlay thread: {}", e);
        }
    }

    pub fn set_position(position: IndicatorPosition) {
        *POSITION.lock().unwrap() = position;
        request_refresh();
    }

    pub fn set_indicator(indicator: Indicator) {
        let value = match indicator {
            Indicator::Idle => STATE_IDLE,
            Indicator::Recording => STATE_RECORDING,
            Indicator::Transcribing => STATE_TRANSCRIBING,
        };
        STATE.store(value, Ordering::SeqCst);
        request_refresh();
    }

    pub fn flash_error() {
        ERROR_UNTIL_MS.store(now_ms() + ERROR_MS, Ordering::SeqCst);
        request_refresh();
    }

    pub fn set_enabled(enabled: bool) {
        ENABLED.store(enabled, Ordering::SeqCst);
        request_refresh();
    }

    pub fn set_suppressed(suppressed: bool) {
        SUPPRESSED.store(suppressed, Ordering::SeqCst);
        request_refresh();
    }

    fn request_refresh() {
        let hwnd = WINDOW.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                PostMessageW(hwnd as HWND, WM_REFRESH, 0, 0);
            }
        }
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0)
    }

    fn current_look() -> Look {
        match STATE.load(Ordering::SeqCst) {
            STATE_RECORDING => Look::Recording,
            STATE_TRANSCRIBING => Look::Transcribing,
            _ if now_ms() < ERROR_UNTIL_MS.load(Ordering::SeqCst) => Look::Error,
            _ => Look::Idle,
        }
    }

    /// The primary screen's work area (without the taskbar) in physical
    /// pixels and its scale, the way Tauri reads them.
    unsafe fn primary_work_area() -> Option<(ScreenRect, f64)> {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        if monitor.is_null() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
        let scale = if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) == 0 && dpi_x > 0 {
            dpi_x as f64 / 96.0
        } else {
            1.0
        };
        let work = info.rcWork;
        let area = ScreenRect {
            x: work.left,
            y: work.top,
            width: work.right - work.left,
            height: work.bottom - work.top,
        };
        Some((area, scale))
    }

    /// Places the indicator on the screen as it is now: the resolution, the
    /// scale and the taskbar may have changed since Typr started.
    unsafe fn place(position: IndicatorPosition) -> Placement {
        let placement = match primary_work_area() {
            Some((area, scale)) => Placement::new(position, area, scale),
            None => Placement::fallback(position),
        };
        if PLACEMENT.with(|cell| cell.replace(Some(placement))) != Some(placement) {
            log::info!(
                "Recording indicator placed at ({}, {}), {}×{} px ({})",
                placement.x,
                placement.y,
                placement.width,
                placement.height,
                position
            );
        }
        placement
    }

    unsafe fn run() {
        let placement = place(*POSITION.lock().unwrap());

        let class_name: Vec<u16> = "TyprOverlay"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let instance = GetModuleHandleW(std::ptr::null());

        let mut class: WNDCLASSEXW = std::mem::zeroed();
        class.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        class.lpfnWndProc = Some(window_proc);
        class.hInstance = instance;
        class.lpszClassName = class_name.as_ptr();
        if RegisterClassExW(&class) == 0 {
            log::error!("Failed to register the overlay window class");
            return;
        }

        // Click-through, always on top, never activated, hidden from Alt+Tab and the taskbar
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_POPUP,
            placement.x,
            placement.y,
            placement.width,
            placement.height,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            log::error!("Failed to create the overlay window");
            return;
        }
        WINDOW.store(hwnd as isize, Ordering::SeqCst);
        log::info!("Overlay created, hidden until a dictation starts (native, no WebView)");

        update(hwnd);

        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_REFRESH => {
                update(hwnd);
                0
            }
            WM_TIMER => {
                if wparam == ERROR_TIMER {
                    KillTimer(hwnd, ERROR_TIMER);
                }
                update(hwnd);
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    /// Moves the slide position towards `target` (0 or 1) by the time since
    /// the previous frame and returns it.
    fn slide_towards(target: f32) -> f32 {
        SLIDE.with(|slide| {
            let (mut shown, last_frame) = slide.get();
            let now = Instant::now();
            if let Some(last_frame) = last_frame {
                let step = now.duration_since(last_frame).as_secs_f32() / SLIDE_SECS;
                shown = if target > shown {
                    (shown + step).min(target)
                } else {
                    (shown - step).max(target)
                };
            }
            let moving = shown != target;
            slide.set((shown, if moving { Some(now) } else { None }));
            shown
        })
    }

    /// Fast at first, gentle at the end: sliding in decelerates into place,
    /// sliding out accelerates away. Reversing midway never jumps.
    fn ease(shown: f32) -> f32 {
        1.0 - (1.0 - shown).powi(3)
    }

    /// Brings the window up to date: look, slide position, timers, visibility.
    unsafe fn update(hwnd: HWND) {
        let look = current_look();
        SHOWN.with(|shown| {
            if shown.get().0 != look {
                shown.set((look, Instant::now()));
            }
        });

        // Suppressed: the quick ask window has taken the dot's place
        let enabled = ENABLED.load(Ordering::SeqCst) && !SUPPRESSED.load(Ordering::SeqCst);
        // Out of sight while idle; slides in for a dictation or an error
        let target = if enabled && look != Look::Idle { 1.0 } else { 0.0 };
        let shown = slide_towards(target);

        if !enabled || (target == 0.0 && shown == 0.0) {
            // Fully hidden: no window, no timers, no CPU
            KillTimer(hwnd, ANIMATION_TIMER);
            KillTimer(hwnd, ERROR_TIMER);
            SLIDE.with(|slide| slide.set((0.0, None)));
            if ON_SCREEN.with(|on_screen| on_screen.replace(false)) {
                ShowWindow(hwnd, SW_HIDE);
            }
            return;
        }

        if look == Look::Error {
            let remaining = ERROR_UNTIL_MS
                .load(Ordering::SeqCst)
                .saturating_sub(now_ms())
                .max(1);
            SetTimer(hwnd, ERROR_TIMER, remaining as u32, None);
        }

        // Placed anew whenever it starts to appear, and right away when the
        // position changes while it is on screen
        let on_screen = ON_SCREEN.with(|on_screen| on_screen.get());
        let position = *POSITION.lock().unwrap();
        if !on_screen || PLACEMENT.with(|cell| cell.get()).map(|p| p.position) != Some(position) {
            place(position);
        }

        draw(hwnd, ease(shown));
        if !on_screen {
            ON_SCREEN.with(|on_screen| on_screen.set(true));
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }

        // Tick only while something moves; a still indicator costs no CPU
        if shown != target {
            SetTimer(hwnd, ANIMATION_TIMER, SLIDE_FRAME_MS, None);
        } else if look == Look::Recording || look == Look::Transcribing {
            SetTimer(hwnd, ANIMATION_TIMER, FRAME_MS, None);
        } else {
            KillTimer(hwnd, ANIMATION_TIMER);
        }
    }

    /// `shown` is the eased slide position: 0 hidden beyond the edge, 1 in place.
    unsafe fn draw(hwnd: HWND, shown: f32) {
        let Some(placement) = PLACEMENT.with(|cell| cell.get()) else {
            return;
        };
        let (look, since) = SHOWN.with(|cell| cell.get());
        let width = placement.width.max(1);
        let height = placement.height.max(1);
        let pixels = render(
            look,
            since.elapsed().as_secs_f32(),
            width as usize,
            height as usize,
            placement.center_y(shown),
            placement.scale as f32,
        );

        let screen = GetDC(std::ptr::null_mut());
        let memory = CreateCompatibleDC(screen);

        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height; // negative: top-down rows
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = 0; // BI_RGB

        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            memory,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if !bitmap.is_null() && !bits.is_null() {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
            let previous = SelectObject(memory, bitmap);

            let position = POINT {
                x: placement.x,
                y: placement.y,
            };
            let extent = SIZE {
                cx: width,
                cy: height,
            };
            let origin = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                hwnd,
                screen,
                &position,
                &extent,
                memory,
                &origin,
                0,
                &blend,
                ULW_ALPHA,
            );

            SelectObject(memory, previous);
            DeleteObject(bitmap);
        }

        DeleteDC(memory);
        ReleaseDC(std::ptr::null_mut(), screen);
    }
}

// ── Other platforms: the original WebView overlay ───────

#[cfg(not(target_os = "windows"))]
mod webview {
    use super::{Indicator, IndicatorPosition, Placement, ScreenRect, INDICATOR_SIZE, OFFSET_FROM_EDGE};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;
    use tauri::{AppHandle, LogicalPosition, Manager, WebviewUrl, WebviewWindowBuilder};

    const ERROR_MS: u64 = 2500;
    const ERROR_JS: &str = "(function(){var m=document.getElementById('mic');if(!m)return;\
                            m.className='mic error';clearTimeout(window.__typrError);\
                            window.__typrError=setTimeout(function(){\
                            if(m.className==='mic error')m.className='mic';},2500);})();";
    /// Replays the slide-in animation from behind the edge
    const SLIDE_IN_JS: &str = "(function(){var s=document.getElementById('slot');if(!s)return;\
                               s.classList.remove('enter');void s.offsetWidth;\
                               s.classList.add('enter');})();";

    static ENABLED: AtomicBool = AtomicBool::new(true);
    static SUPPRESSED: AtomicBool = AtomicBool::new(false);
    /// Recording or transcribing right now
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    /// Bumped on every error flash, so only the latest one hides the window
    static ERRORS: AtomicU64 = AtomicU64::new(0);
    static POSITION: Mutex<IndicatorPosition> = Mutex::new(IndicatorPosition::TopRight);

    fn place(app: &AppHandle, position: IndicatorPosition) -> Placement {
        match app.primary_monitor().ok().flatten() {
            Some(monitor) => {
                let area = monitor.work_area();
                let area = ScreenRect {
                    x: area.position.x,
                    y: area.position.y,
                    width: area.size.width as i32,
                    height: area.size.height as i32,
                };
                Placement::new(position, area, monitor.scale_factor())
            }
            None => Placement::fallback(position),
        }
    }

    /// At the bottom the page puts the mic at the top of the window and slides it up
    fn edge_js(position: IndicatorPosition) -> String {
        format!(
            "document.body.classList.toggle('bottom',{});",
            position == IndicatorPosition::BottomCenter
        )
    }

    pub fn create(app: &AppHandle, position: IndicatorPosition) {
        *POSITION.lock().unwrap() = position;
        let placement = place(app, position);
        let built = WebviewWindowBuilder::new(
            app,
            "overlay",
            WebviewUrl::App("src/overlay.html".into()),
        )
        .title("")
        .inner_size(INDICATOR_SIZE, OFFSET_FROM_EDGE + INDICATOR_SIZE)
        .position(placement.logical_x, placement.logical_y)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .shadow(false)
        .visible(false)
        .build();

        match built {
            Ok(_) => log::info!("Overlay window created, hidden until a dictation starts"),
            Err(e) => log::error!("Failed to create overlay: {}", e),
        }
    }

    fn eval(app: &AppHandle, js: &str) {
        if let Some(overlay) = app.get_webview_window("overlay") {
            if let Err(e) = overlay.eval(js) {
                log::debug!("Failed to update overlay: {}", e);
            }
        }
    }

    fn show(app: &AppHandle) {
        if !ENABLED.load(Ordering::SeqCst) || SUPPRESSED.load(Ordering::SeqCst) {
            return;
        }
        if let Some(overlay) = app.get_webview_window("overlay") {
            if !overlay.is_visible().unwrap_or(false) {
                let position = *POSITION.lock().unwrap();
                let _ = overlay.eval(&edge_js(position));
                let _ = overlay.eval(SLIDE_IN_JS);
                let _ = overlay.show();
            }
        }
    }

    pub fn set_position(app: &AppHandle, position: IndicatorPosition) {
        *POSITION.lock().unwrap() = position;
        let placement = place(app, position);
        if let Some(overlay) = app.get_webview_window("overlay") {
            let _ = overlay.set_position(LogicalPosition::new(placement.logical_x, placement.logical_y));
            let _ = overlay.eval(&edge_js(position));
        }
    }

    fn hide(app: &AppHandle) {
        if let Some(overlay) = app.get_webview_window("overlay") {
            let _ = overlay.hide();
        }
    }

    pub fn set_indicator(app: &AppHandle, indicator: Indicator) {
        let class = match indicator {
            Indicator::Idle => "mic",
            Indicator::Recording => "mic recording",
            Indicator::Transcribing => "mic transcribing",
        };
        eval(
            app,
            &format!("document.getElementById('mic').className = '{}';", class),
        );

        let active = indicator != Indicator::Idle;
        ACTIVE.store(active, Ordering::SeqCst);
        if active {
            show(app);
        } else {
            hide(app);
        }
    }

    pub fn flash_error(app: &AppHandle) {
        eval(app, ERROR_JS);
        show(app);

        let flash = ERRORS.fetch_add(1, Ordering::SeqCst) + 1;
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(ERROR_MS));
            if ERRORS.load(Ordering::SeqCst) == flash && !ACTIVE.load(Ordering::SeqCst) {
                hide(&app);
            }
        });
    }

    pub fn set_enabled(app: &AppHandle, enabled: bool) {
        ENABLED.store(enabled, Ordering::SeqCst);
        if !enabled {
            hide(app);
        } else if ACTIVE.load(Ordering::SeqCst) {
            show(app);
        }
    }

    pub fn set_suppressed(app: &AppHandle, suppressed: bool) {
        SUPPRESSED.store(suppressed, Ordering::SeqCst);
        if suppressed {
            hide(app);
        } else if ACTIVE.load(Ordering::SeqCst) {
            show(app);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_has_transparent_corners_and_opaque_disc() {
        let frame = render(Look::Idle, 0.0, 50, 60, 35.0, 1.0);
        assert_eq!(frame.len(), 50 * 60 * 4);
        assert_eq!(frame[3], 0);
        let inside = (35 * 50 + 10) * 4;
        assert!(frame[inside + 3] > 200);
    }

    #[test]
    fn test_render_hidden_indicator_is_fully_transparent() {
        let area = ScreenRect {
            x: 0,
            y: 0,
            width: 2560,
            height: 1392,
        };
        for &position in IndicatorPosition::ALL {
            for scale in [1.0, 1.25, 1.5, 2.0] {
                let placement = Placement::new(position, area, scale);
                for look in [Look::Idle, Look::Recording, Look::Transcribing, Look::Error] {
                    // Recording's halo is widest at 0.7 s
                    for t in [0.0, 0.35, 0.7] {
                        let frame = render(
                            look,
                            t,
                            placement.width as usize,
                            placement.height as usize,
                            placement.center_y(0.0),
                            scale as f32,
                        );
                        assert!(
                            frame.chunks(4).all(|pixel| pixel[3] == 0),
                            "{} at scale {} shows {:?} while hidden",
                            position,
                            scale,
                            look
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_render_indicator_in_place_is_not_cut_off() {
        for &position in IndicatorPosition::ALL {
            let placement = Placement::fallback(position);
            let (width, height) = (placement.width as usize, placement.height as usize);
            let frame = render(Look::Error, 0.0, width, height, placement.center_y(1.0), 1.0);
            // Nothing touches the window's edges
            for x in 0..width {
                assert_eq!(frame[x * 4 + 3], 0, "{} is cut at the top", position);
                assert_eq!(frame[((height - 1) * width + x) * 4 + 3], 0, "{} is cut at the bottom", position);
            }
            let center = (placement.center_y(1.0) as usize * width + width / 2) * 4;
            assert!(frame[center + 3] > 200);
        }
    }

    #[test]
    fn test_mic_sdf_inside_and_outside() {
        assert!(mic_sdf(12.0, 8.0) < 0.0);
        assert!(mic_sdf(2.0, 2.0) > 0.0);
    }

    /// 4K at 200% with a 96 px taskbar at the bottom
    const WORK_AREA_4K: ScreenRect = ScreenRect {
        x: 0,
        y: 0,
        width: 3840,
        height: 2064,
    };

    #[test]
    fn test_placement_top_right() {
        let placement = Placement::new(IndicatorPosition::TopRight, WORK_AREA_4K, 2.0);
        assert_eq!(placement.width, 100);
        assert_eq!(placement.height, 120);
        assert_eq!(placement.x, 3840 - 120);
        assert_eq!(placement.y, 0);
        assert_eq!(placement.center_y(1.0), 70.0);
        assert_eq!(placement.center_y(0.0), -50.0);
        assert_eq!((placement.logical_x, placement.logical_y), (1860.0, 0.0));
    }

    #[test]
    fn test_placement_bottom_center_rests_on_the_taskbar() {
        let placement = Placement::new(IndicatorPosition::BottomCenter, WORK_AREA_4K, 2.0);
        assert_eq!(placement.width, 100);
        assert_eq!(placement.height, 120);
        assert_eq!(placement.x, (3840 - 100) / 2);
        // The window ends where the taskbar starts
        assert_eq!(placement.y + placement.height, 2064);
        assert_eq!(placement.center_y(1.0), 50.0);
        assert_eq!(placement.center_y(0.0), 170.0);
    }

    #[test]
    fn test_dot_center_is_where_the_disc_is_drawn() {
        for &position in IndicatorPosition::ALL {
            let placement = Placement::new(position, WORK_AREA_4K, 2.0);
            let (x, y) = placement.dot_center();
            assert_eq!(x, placement.x as f64 + placement.width as f64 / 2.0);
            assert_eq!(y, placement.y as f64 + placement.center_y(1.0) as f64);
        }
        let top = Placement::new(IndicatorPosition::TopRight, WORK_AREA_4K, 2.0);
        assert_eq!(top.dot_center(), (3840.0 - 70.0, 70.0));
    }

    #[test]
    fn test_placement_follows_the_work_area() {
        // A taskbar at the top (Windows 10): the top-right indicator slides
        // out from under it, the bottom one rests on the screen's bottom edge
        let area = ScreenRect {
            x: 0,
            y: 40,
            width: 1920,
            height: 1040,
        };
        let top = Placement::new(IndicatorPosition::TopRight, area, 1.0);
        assert_eq!((top.x, top.y), (1860, 40));
        let bottom = Placement::new(IndicatorPosition::BottomCenter, area, 1.0);
        assert_eq!((bottom.x, bottom.y), (935, 1020));
    }
}

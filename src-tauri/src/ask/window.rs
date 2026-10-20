//! The quick ask window: a transparent WebView frame at the edge of the
//! screen where the recording indicator is. The page draws the indicator's
//! dot exactly where the native one stands, takes its place and unfolds it
//! into the answer card (`src/ask.ts`).
//!
//! * The frame has the size of the largest card, so it never moves or
//!   resizes while an answer grows. The rest of it passes clicks through:
//!   the page reports where the card is drawn and a thread watching the
//!   cursor makes the window click-through everywhere else.
//! * It never takes the focus (`WS_EX_NOACTIVATE`): the app the question is
//!   about stays in front and keeps its selection and caret, and its buttons
//!   still work without activating it.
//! * It exists only while an answer (or quick chat) is on screen; closing
//!   it destroys the WebView. With "Keep the quick ask window ready"
//!   (Developer section) a hidden one is loaded ahead of time instead and
//!   handed its layout when a question comes ([`keep_ready`]): the window
//!   opens at once, but WebView2 keeps running while idle.

use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

use super::Mode;
use crate::overlay::{self, Placement, ScreenRect};
use crate::settings::{IndicatorPosition, RecordingMode, Settings};

pub const LABEL: &str = "ask";

/// Card width in logical pixels
pub const CARD_WIDTH: f64 = 440.0;
const MAX_CARD_HEIGHT: f64 = 560.0;
/// The card never takes more of the screen's height than this
const MAX_SCREEN_SHARE: f64 = 0.7;
/// Centre of the card's status dot from the two card edges at the corner it
/// grows from: the indicator's dot turns into it
pub const ORB_INSET: f64 = 18.0;
/// Room around the card for its shadow
const SHADOW_ROOM: f64 = 28.0;
/// How often the cursor is checked against the card
const HIT_POLL: Duration = Duration::from_millis(30);
/// How long the native dot stays while the page starts drawing
const SHOWN_WAIT: Duration = Duration::from_millis(1500);
/// How long a question waits for the window kept ready to finish loading
const SPARE_WAIT: Duration = Duration::from_secs(3);
/// How long a window kept ready waits for the previous one to go
const GONE_WAIT: Duration = Duration::from_secs(3);

/// Where the window goes and what the page needs to lay out the card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    /// Window rectangle in physical pixels
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    /// The indicator's centre inside the window, in logical pixels
    pub orb_x: f64,
    pub orb_y: f64,
    /// Tallest the card may grow, in logical pixels
    pub max_height: f64,
}

impl Geometry {
    /// `work_area` is the primary screen without the taskbar, in physical
    /// pixels, as the indicator is placed (overlay.rs).
    pub fn new(position: IndicatorPosition, work_area: ScreenRect, scale: f64) -> Self {
        let s = if scale > 0.0 { scale } else { 1.0 };
        let (dot_x, dot_y) = Placement::new(position, work_area, s).dot_center();
        let screen_height = work_area.height as f64 / s;
        let max_height = MAX_CARD_HEIGHT.min(screen_height * MAX_SCREEN_SHARE).max(160.0);

        // Window edges in physical pixels; the window touches the screen edge
        // the indicator slides in from, and the card grows away from it
        let (left, top, right, bottom) = match position {
            IndicatorPosition::TopRight => {
                let card_right = dot_x + ORB_INSET * s;
                let card_top = dot_y - ORB_INSET * s;
                (
                    card_right - (CARD_WIDTH + SHADOW_ROOM) * s,
                    work_area.y as f64,
                    (work_area.x + work_area.width) as f64,
                    card_top + (max_height + SHADOW_ROOM) * s,
                )
            }
            IndicatorPosition::BottomCenter => {
                let card_bottom = dot_y + ORB_INSET * s;
                let half = (CARD_WIDTH / 2.0 + SHADOW_ROOM) * s;
                (
                    dot_x - half,
                    card_bottom - (max_height + SHADOW_ROOM) * s,
                    dot_x + half,
                    (work_area.y + work_area.height) as f64,
                )
            }
        };
        let x = left.round() as i32;
        let y = top.round() as i32;
        Self {
            x,
            y,
            width: (right - left).round().max(1.0) as u32,
            height: (bottom - top).round().max(1.0) as u32,
            scale: s,
            orb_x: (dot_x - x as f64) / s,
            orb_y: (dot_y - y as f64) / s,
            max_height,
        }
    }
}

/// What the page is told before it loads, besides the layout.
pub struct Page {
    position: IndicatorPosition,
    language: &'static str,
    mode: Mode,
    /// The dictation hotkey, for the empty chat to name
    hotkey: String,
    push_to_talk: bool,
}

impl Page {
    pub fn new(settings: &Settings, mode: Mode) -> Self {
        Self {
            position: settings.indicator_position,
            language: crate::i18n::resolve(&settings.language),
            mode,
            hotkey: settings.hotkey.clone(),
            push_to_talk: settings.recording_mode == RecordingMode::PushToTalk,
        }
    }

    /// What the page lays itself out by (`window.__TYPR_ASK__` in ask.ts)
    fn layout(&self, geometry: &Geometry) -> serde_json::Value {
        json!({
            "position": self.position.id(),
            "language": self.language,
            "mode": self.mode.id(),
            "hotkey": self.hotkey,
            "pushToTalk": self.push_to_talk,
            "orbX": geometry.orb_x,
            "orbY": geometry.orb_y,
            "orbInset": ORB_INSET,
            "cardWidth": CARD_WIDTH,
            "maxHeight": geometry.max_height,
        })
    }
}

/// The primary screen's work area and scale, as the indicator reads them.
fn screen(app: &AppHandle) -> (ScreenRect, f64) {
    match app.primary_monitor().ok().flatten() {
        Some(monitor) => {
            let area = monitor.work_area();
            (
                ScreenRect {
                    x: area.position.x,
                    y: area.position.y,
                    width: area.size.width as i32,
                    height: area.size.height as i32,
                },
                monitor.scale_factor(),
            )
        }
        None => (
            ScreenRect {
                x: 0,
                y: 0,
                width: 1440,
                height: 900,
            },
            1.0,
        ),
    }
}

/// The window on screen now, for hit testing
static FRAME: Mutex<Option<Geometry>> = Mutex::new(None);
/// Where the page draws the card: x, y, width, height in logical pixels
static CARD: Mutex<Option<[f64; 4]>> = Mutex::new(None);
/// Bumped when the window goes, which ends its cursor watch
static WATCH: AtomicU64 = AtomicU64::new(0);
/// The page draws its dot (`ask_shown`): the native one can go
static SHOWN: AtomicBool = AtomicBool::new(false);

/// The window kept ready: none (or the window is in use), its page loading,
/// or loaded and waiting for a question
const SPARE_NONE: u8 = 0;
const SPARE_LOADING: u8 = 1;
const SPARE_READY: u8 = 2;
static SPARE: AtomicU8 = AtomicU8::new(SPARE_NONE);
/// When the window kept ready started loading (for the log)
static SPARE_SINCE: Mutex<Option<Instant>> = Mutex::new(None);
/// The window being destroyed was kept ready: no conversation goes with it
static SPARE_GOING: AtomicBool = AtomicBool::new(false);
/// Whether a window is kept ready, and where the indicator is
static KEEP: AtomicBool = AtomicBool::new(false);
static KEEP_POSITION: Mutex<IndicatorPosition> = Mutex::new(IndicatorPosition::TopRight);
/// One window is built, taken or let go at a time
static BUILD: Mutex<()> = Mutex::new(());

/// Makes the window for a new conversation, hidden: takes the one kept
/// ready, or builds it. Blocks until it exists, so call it off the async
/// threads.
pub fn open(app: &AppHandle, page: &Page) -> Result<(), String> {
    let _build = BUILD.lock().unwrap();
    let (work_area, scale) = screen(app);
    let geometry = Geometry::new(page.position, work_area, scale);
    let layout = page.layout(&geometry);

    if let Some(window) = app.get_webview_window(LABEL) {
        if take_spare() {
            // Over the indicator as the screen is now; the page lays itself
            // out and reports ready
            let _ = window.set_position(PhysicalPosition::new(geometry.x, geometry.y));
            let _ = window.set_size(PhysicalSize::new(geometry.width, geometry.height));
            *FRAME.lock().unwrap() = Some(geometry);
            *CARD.lock().unwrap() = None;
            app.emit_to(LABEL, "ask-activate", layout)
                .map_err(|e| format!("The quick ask window couldn't be prepared: {}", e))?;
            log_placed(page, &geometry, " (kept ready)");
            return Ok(());
        }
        if SPARE.swap(SPARE_NONE, Ordering::SeqCst) == SPARE_LOADING {
            log::warn!("The quick ask window kept ready didn't load in time, opening a new one");
            SPARE_GOING.store(true, Ordering::SeqCst);
            let _ = window.destroy();
        }
        // Otherwise the last window is still on its way out
        if !wait_gone(app, GONE_WAIT) {
            return Err("The previous quick ask window is still closing".to_string());
        }
    }
    SPARE.store(SPARE_NONE, Ordering::SeqCst);
    build(app, &geometry, Some(format!("window.__TYPR_ASK__ = {};", layout)))?;
    *FRAME.lock().unwrap() = Some(geometry);
    *CARD.lock().unwrap() = None;
    log_placed(page, &geometry, "");
    Ok(())
}

fn log_placed(page: &Page, geometry: &Geometry, note: &str) {
    log::info!(
        "Quick {} window at ({}, {}), {}×{} px ({}){}",
        page.mode.id(),
        geometry.x,
        geometry.y,
        geometry.width,
        geometry.height,
        page.position,
        note
    );
}

/// Takes the window kept ready, waiting for its page if it is still loading.
fn take_spare() -> bool {
    let started = Instant::now();
    while SPARE.load(Ordering::SeqCst) == SPARE_LOADING && started.elapsed() < SPARE_WAIT {
        std::thread::sleep(Duration::from_millis(10));
    }
    SPARE
        .compare_exchange(SPARE_READY, SPARE_NONE, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

/// Waits until no quick ask window is left; false if one still is.
fn wait_gone(app: &AppHandle, limit: Duration) -> bool {
    let started = Instant::now();
    while app.get_webview_window(LABEL).is_some() {
        if started.elapsed() >= limit {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

/// Builds the window, hidden, where `geometry` puts it. `init` runs before
/// the page's own scripts; without the layout it sets, the page waits.
fn build(app: &AppHandle, geometry: &Geometry, init: Option<String>) -> Result<(), String> {
    let s = geometry.scale;
    let mut builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("src/ask.html".into()));
    if let Some(init) = init {
        builder = builder.initialization_script(init);
    }
    let window = builder
        .title("Typr")
        .inner_size(geometry.width as f64 / s, geometry.height as f64 / s)
        .position(geometry.x as f64 / s, geometry.y as f64 / s)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focusable(false)
        .focused(false)
        .visible(false)
        .build()
        .map_err(|e| format!("The quick ask window couldn't be created: {}", e))?;
    // Exactly over the indicator, whatever scale the builder assumed
    let _ = window.set_position(PhysicalPosition::new(geometry.x, geometry.y));
    let _ = window.set_size(PhysicalSize::new(geometry.width, geometry.height));
    Ok(())
}

/// Developer option: keeps a hidden window loaded for the next question
/// (quick ask or quick chat), or lets the one kept ready go. A window in use
/// is left alone; the next one is loaded once it closes.
pub fn keep_ready(app: &AppHandle, keep: bool, position: IndicatorPosition) {
    *KEEP_POSITION.lock().unwrap() = position;
    if KEEP.swap(keep, Ordering::SeqCst) == keep {
        return;
    }
    let app = app.clone();
    // Off the event loop: building or destroying a window there deadlocks
    std::thread::spawn(move || {
        if keep {
            prepare(&app);
            return;
        }
        let _build = BUILD.lock().unwrap();
        if SPARE.swap(SPARE_NONE, Ordering::SeqCst) != SPARE_NONE {
            if let Some(window) = app.get_webview_window(LABEL) {
                SPARE_GOING.store(true, Ordering::SeqCst);
                let _ = window.destroy();
            }
            log::info!("The quick ask window is no longer kept ready");
        }
    });
}

/// Loads a hidden window for the next question, once the last one is gone.
fn prepare(app: &AppHandle) {
    // A window in use: the next one is prepared when it is destroyed
    if !wait_gone(app, GONE_WAIT) {
        return;
    }
    let _build = BUILD.lock().unwrap();
    if !KEEP.load(Ordering::SeqCst) || app.get_webview_window(LABEL).is_some() {
        return;
    }
    let (work_area, scale) = screen(app);
    let geometry = Geometry::new(*KEEP_POSITION.lock().unwrap(), work_area, scale);
    SPARE.store(SPARE_LOADING, Ordering::SeqCst);
    *SPARE_SINCE.lock().unwrap() = Some(Instant::now());
    if let Err(e) = build(app, &geometry, None) {
        SPARE.store(SPARE_NONE, Ordering::SeqCst);
        log::warn!("{}", e);
    }
}

/// The page of the window kept ready has loaded and waits for a question.
pub fn standby_ready() {
    if SPARE
        .compare_exchange(SPARE_LOADING, SPARE_READY, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        let since = SPARE_SINCE.lock().unwrap().take();
        log::info!(
            "Quick ask window kept ready for the next question (loaded in {} ms)",
            since.map_or(0, |since| since.elapsed().as_millis())
        );
    }
}

/// The window was destroyed. Returns whether it was one kept ready, which no
/// conversation went with. With the option on, the next one is loaded.
pub fn destroyed(app: &AppHandle) -> bool {
    let spare = SPARE_GOING.swap(false, Ordering::SeqCst);
    if KEEP.load(Ordering::SeqCst) {
        let app = app.clone();
        std::thread::spawn(move || prepare(&app));
    }
    spare
}

/// Shows the window without taking the focus and hands the indicator's
/// place over to it. Returns once the page draws its dot in the native one's
/// place (WebView2 starts painting a moment after the window shows), so the
/// recorder's Ready doesn't slide the native dot away before that.
pub async fn reveal(app: &AppHandle) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    SHOWN.store(false, Ordering::SeqCst);
    let before = crate::paste::foreground_window();
    raise(&window);
    let _ = window.show();
    for _ in 0..20 {
        if window.is_visible().unwrap_or(false) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    give_focus_back(&window, before);
    if let Err(e) = app.emit_to(LABEL, "ask-reveal", ()) {
        log::debug!("Failed to emit ask-reveal: {}", e);
    }
    watch_cursor(app.clone());

    let started = std::time::Instant::now();
    while !SHOWN.load(Ordering::SeqCst) && started.elapsed() < SHOWN_WAIT {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if SHOWN.load(Ordering::SeqCst) {
        log::debug!("Quick ask window drawing after {} ms", started.elapsed().as_millis());
    } else {
        log::warn!("The quick ask page didn't report drawing in time");
        overlay::set_suppressed(app, true);
    }
}

/// The page draws its dot exactly over the native one, which can now go.
pub fn page_shown(app: &AppHandle) {
    SHOWN.store(true, Ordering::SeqCst);
    overlay::set_suppressed(app, true);
}

/// Always on top again before showing: another program can take that away,
/// and a window kept ready may wait hidden for hours.
fn raise(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "windows")]
    if let Ok(hwnd) = window.hwnd() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        };
        unsafe {
            SetWindowPos(
                hwnd.0 as _,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = window;
}

/// The window is built never to take the focus; should Windows give it the
/// focus anyway, it goes back to the window the question is about.
fn give_focus_back(window: &tauri::WebviewWindow, before: isize) {
    #[cfg(target_os = "windows")]
    if let Ok(hwnd) = window.hwnd() {
        let now = crate::paste::foreground_window();
        if now == hwnd.0 as isize && before != 0 && before != now {
            log::debug!("The quick ask window took the focus, giving it back");
            crate::paste::focus_window(before);
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (window, before);
}

/// Destroys the window and its WebView.
pub fn destroy(app: &AppHandle) {
    WATCH.fetch_add(1, Ordering::SeqCst);
    *FRAME.lock().unwrap() = None;
    *CARD.lock().unwrap() = None;
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
        if let Err(e) = window.destroy() {
            log::warn!("Failed to close the quick ask window: {}", e);
        }
    }
}

/// Where the page draws the card now (logical pixels inside the window).
pub fn set_card(x: f64, y: f64, width: f64, height: f64) {
    *CARD.lock().unwrap() = Some([x, y, width, height]);
}

/// Whether the screen point (physical pixels) is on the card.
fn on_card(point: (f64, f64)) -> bool {
    let (Some(frame), Some([x, y, width, height])) = (*FRAME.lock().unwrap(), *CARD.lock().unwrap()) else {
        return false;
    };
    let px = (point.0 - frame.x as f64) / frame.scale;
    let py = (point.1 - frame.y as f64) / frame.scale;
    px >= x && px <= x + width && py >= y && py <= y + height
}

/// While the window is up, makes it click-through everywhere but the card.
fn watch_cursor(app: AppHandle) {
    let watch = WATCH.fetch_add(1, Ordering::SeqCst) + 1;
    let spawned = std::thread::Builder::new()
        .name("typr-ask-cursor".to_string())
        .spawn(move || {
            let mut through: Option<bool> = None;
            while WATCH.load(Ordering::SeqCst) == watch {
                let Some(window) = app.get_webview_window(LABEL) else {
                    break;
                };
                let pass = !cursor_position(&app).is_some_and(on_card);
                if through != Some(pass) {
                    if window.set_ignore_cursor_events(pass).is_ok() {
                        through = Some(pass);
                    }
                }
                std::thread::sleep(HIT_POLL);
            }
        });
    if let Err(e) = spawned {
        log::warn!("Could not watch the cursor over the quick ask window: {}", e);
    }
}

fn cursor_position(app: &AppHandle) -> Option<(f64, f64)> {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut point = POINT { x: 0, y: 0 };
        (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x as f64, point.y as f64))
    }
    #[cfg(not(target_os = "windows"))]
    {
        app.cursor_position().ok().map(|p| (p.x, p.y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1080p at 125% with the taskbar at the bottom
    const AREA: ScreenRect = ScreenRect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1032,
    };

    #[test]
    fn test_top_right_window_touches_the_top_and_right_edges() {
        let g = Geometry::new(IndicatorPosition::TopRight, AREA, 1.25);
        assert_eq!(g.y, 0);
        assert_eq!(g.x + g.width as i32, 1920);
        let (dot_x, dot_y) = Placement::new(IndicatorPosition::TopRight, AREA, 1.25).dot_center();
        // The page finds the indicator's dot where it is on screen
        assert!((g.x as f64 + g.orb_x * g.scale - dot_x).abs() < 1.0);
        assert!((g.y as f64 + g.orb_y * g.scale - dot_y).abs() < 1.0);
        // Room for the whole card left of the dot and below it
        assert!(g.orb_x + ORB_INSET >= CARD_WIDTH);
        assert!(g.height as f64 / g.scale >= g.orb_y - ORB_INSET + g.max_height);
    }

    #[test]
    fn test_bottom_window_rests_on_the_taskbar_centred_on_the_dot() {
        let g = Geometry::new(IndicatorPosition::BottomCenter, AREA, 1.25);
        assert_eq!(g.y + g.height as i32, 1032);
        assert!((g.orb_x - g.width as f64 / g.scale / 2.0).abs() < 1.0);
        // The tallest card still has its shadow room above it
        assert!(g.orb_y + ORB_INSET - g.max_height >= SHADOW_ROOM - 1.0);
    }

    #[test]
    fn test_the_card_fits_a_small_screen() {
        let small = ScreenRect {
            x: 0,
            y: 0,
            width: 1366,
            height: 728,
        };
        let g = Geometry::new(IndicatorPosition::TopRight, small, 1.0);
        assert!(g.max_height <= 728.0 * MAX_SCREEN_SHARE);
        assert!(g.y as f64 + g.height as f64 <= 728.0);
    }

    #[test]
    fn test_cursor_on_the_card() {
        *FRAME.lock().unwrap() = Some(Geometry::new(IndicatorPosition::TopRight, AREA, 1.0));
        let frame = FRAME.lock().unwrap().unwrap();
        set_card(10.0, 10.0, 100.0, 50.0);
        let inside = (frame.x as f64 + 50.0, frame.y as f64 + 30.0);
        let outside = (frame.x as f64 + 200.0, frame.y as f64 + 30.0);
        assert!(on_card(inside));
        assert!(!on_card(outside));
        *FRAME.lock().unwrap() = None;
        *CARD.lock().unwrap() = None;
    }
}

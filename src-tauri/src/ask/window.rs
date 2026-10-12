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
//! * It exists only while an answer is on screen; closing it destroys the
//!   WebView.

use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

use crate::overlay::{self, Placement, ScreenRect};
use crate::settings::IndicatorPosition;

pub const LABEL: &str = "ask";

/// Card width in logical pixels
const CARD_WIDTH: f64 = 440.0;
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

/// Creates the window, hidden. Blocks until it exists, so call it off the
/// async threads.
pub fn open(app: &AppHandle, position: IndicatorPosition, language: &str) -> Result<(), String> {
    if app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }
    let (work_area, scale) = screen(app);
    let geometry = Geometry::new(position, work_area, scale);
    let layout = json!({
        "position": position.id(),
        "language": language,
        "orbX": geometry.orb_x,
        "orbY": geometry.orb_y,
        "orbInset": ORB_INSET,
        "cardWidth": CARD_WIDTH,
        "maxHeight": geometry.max_height,
    });
    let s = geometry.scale;
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("src/ask.html".into()))
        .title("Typr")
        .initialization_script(format!("window.__TYPR_ASK__ = {};", layout))
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
    *FRAME.lock().unwrap() = Some(geometry);
    *CARD.lock().unwrap() = None;
    log::info!(
        "Quick ask window at ({}, {}), {}×{} px ({})",
        geometry.x,
        geometry.y,
        geometry.width,
        geometry.height,
        position
    );
    Ok(())
}

/// Shows the window without taking the focus and hands the indicator's
/// place over to it.
pub async fn reveal(app: &AppHandle) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let before = crate::paste::foreground_window();
    let _ = window.show();
    for _ in 0..20 {
        if window.is_visible().unwrap_or(false) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    give_focus_back(&window, before);
    // Both dots are now in the same place: the native one can go
    overlay::set_suppressed(app, true);
    if let Err(e) = app.emit_to(LABEL, "ask-reveal", ()) {
        log::debug!("Failed to emit ask-reveal: {}", e);
    }
    watch_cursor(app.clone());
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

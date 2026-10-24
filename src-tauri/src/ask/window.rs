//! The quick ask window's place and its link to the window itself
//! (`native/`, drawn by Typr with Slint, no WebView).
//!
//! * The window sits at the edge of the screen where the recording
//!   indicator is, sized for the largest card, so it never moves or resizes
//!   while an answer grows. Its first frame draws the indicator's dot exactly
//!   where the native one stands; once that frame is on screen, the native
//!   dot goes and the window's own unfolds into the answer card.
//! * It never takes the focus: the app the question is about stays in front
//!   and keeps its selection and caret. Clicks pass through it everywhere
//!   but the card. Only its text field, opened with its pencil, takes the
//!   keyboard until it is closed.
//! * Its thread starts with the first window and stays, asleep while no
//!   window is open; a closed window leaves nothing on screen or in memory
//!   but that thread and its fonts (about a megabyte).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::AppHandle;

use crate::overlay::{Placement, ScreenRect};
use crate::settings::{IndicatorPosition, RecordingMode, Settings};

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
/// How long the native dot stays while the window draws its first frame
const SHOWN_WAIT: Duration = Duration::from_millis(1500);

/// Where the window goes and where the indicator's dot is in it.
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

/// What the window is opened with.
#[derive(Debug, Clone)]
pub struct Layout {
    pub geometry: Geometry,
    /// The indicator is at the bottom of the screen: the card grows upwards
    pub bottom: bool,
    /// "en" or "ru"
    pub language: &'static str,
    /// Opened empty by the quick ask hotkey: the dot slides in from behind
    /// the screen edge, and the card says how to ask
    pub empty: bool,
    /// The dictation hotkey, for the empty window to name
    pub hotkey: String,
    pub push_to_talk: bool,
}

/// What the recorder is doing, while the window is open
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recording {
    Recording,
    Transcribing,
    Ready,
}

/// What the window is told, in order, on its own thread
pub enum Command {
    /// A new window, hidden, its dot over the indicator's
    Open(Layout),
    /// On screen: the dot unfolds (an empty window's slides in first)
    Reveal,
    /// A question, with the start of the text it is about
    Question { id: u64, text: String, selection: Option<String> },
    /// An answer is coming
    Answer { id: u64 },
    /// The model searches the web
    Search { id: u64 },
    Delta { id: u64, text: String },
    /// The whole answer; `stopped` when a new question cut it short
    Done { id: u64, text: String, stopped: bool },
    Error { id: u64, message: String },
    /// Nothing was answered: the answer (and its question) go
    Cancel { id: u64 },
    Recording(Recording),
    RecordingError(String),
    /// The text field is open: the transcribed speech goes into it
    Dictated(String),
    /// Opens or closes the text field, as its pencil and the cross do (the
    /// demo)
    Field(bool),
    /// Folds the card away, as its cross does
    Fold,
    /// Gone at once
    Destroy,
    /// The demo: the window's pixels now, to the callback
    Snapshot(Box<dyn FnOnce(Option<(u32, u32, Vec<u8>)>) + Send>),
    /// The demo's measurements: what the window's thread has done so far
    Stats(Box<dyn FnOnce(Stats) + Send>),
}

/// What the window has to say, on its thread
pub enum Event {
    /// The window's first frame is on screen, its dot exactly over the
    /// indicator's: the native one can go
    Shown(Instant),
    /// Folded away: the window can be destroyed
    Closed,
    /// The text field opened or closed: while it is open, dictation goes
    /// into it instead of asking
    Typing(bool),
    /// A question typed into the field
    Submit(String),
}

/// What the window's thread has done, for the demo's measurements
#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    /// Frames drawn, and how long drawing them and putting them on screen took
    pub frames: u64,
    pub render: Duration,
    pub present: Duration,
    /// The slowest frame
    pub slowest: Duration,
    /// Pixels redrawn (the dirty regions' bounding boxes)
    pub area: u64,
    /// CPU cycles the thread used
    pub cycles: u64,
}

/// The primary screen's work area and scale, read now, as the indicator
/// reads them.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn screen() -> (ScreenRect, f64) {
    #[cfg(target_os = "windows")]
    if let Some(screen) = crate::overlay::primary_screen() {
        return screen;
    }
    (
        ScreenRect {
            x: 0,
            y: 0,
            width: 1440,
            height: 900,
        },
        1.0,
    )
}

/// The window's first frame is on screen
static SHOWN: AtomicBool = AtomicBool::new(false);

/// Opens a new window, not yet shown, its dot over the indicator's (`empty`:
/// opened by the quick ask hotkey). The window's thread starts with the
/// first one.
#[cfg(target_os = "windows")]
pub fn open(app: &AppHandle, settings: &Settings, empty: bool) -> Result<(), String> {
    use super::native;
    let handle = app.clone();
    native::start(move |event| match event {
        // Both dots are in the same place: the native one can go
        Event::Shown(_) => {
            SHOWN.store(true, Ordering::SeqCst);
            crate::overlay::set_suppressed(&handle, true);
        }
        // Folded away by its cross or the quick ask hotkey
        Event::Closed => super::close(&handle),
        Event::Typing(on) => super::set_typing(on),
        Event::Submit(text) => super::submit(&handle, text),
    })?;

    let (work_area, scale) = screen();
    let position = settings.indicator_position;
    let geometry = Geometry::new(position, work_area, scale);
    log::info!(
        "Quick ask window{} at ({}, {}), {}×{} px ({})",
        if empty { ", empty," } else { "" },
        geometry.x,
        geometry.y,
        geometry.width,
        geometry.height,
        position
    );
    SHOWN.store(false, Ordering::SeqCst);
    native::send(Command::Open(Layout {
        geometry,
        bottom: position == IndicatorPosition::BottomCenter,
        language: crate::i18n::resolve(&settings.language),
        empty,
        hotkey: settings.hotkey.clone(),
        push_to_talk: settings.recording_mode == RecordingMode::PushToTalk,
    }));
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn open(_app: &AppHandle, _settings: &Settings, _empty: bool) -> Result<(), String> {
    Err("The quick ask window is available on Windows only".to_string())
}

/// Puts the window on screen and waits until its first frame is there,
/// so the recorder's Ready doesn't slide the native dot away before that.
pub async fn reveal(app: &AppHandle) {
    send(Command::Reveal);
    let started = Instant::now();
    while !SHOWN.load(Ordering::SeqCst) && started.elapsed() < SHOWN_WAIT {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    if SHOWN.load(Ordering::SeqCst) {
        log::debug!("Quick ask window on screen after {} ms", started.elapsed().as_millis());
    } else {
        log::warn!("The quick ask window didn't show in time");
        crate::overlay::set_suppressed(app, true);
    }
}

/// Hands `command` to the open window.
pub fn send(command: Command) {
    #[cfg(target_os = "windows")]
    super::native::send(command);
    #[cfg(not(target_os = "windows"))]
    let _ = command;
}

/// The window goes at once.
pub fn destroy() {
    send(Command::Destroy);
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
        // The window finds the indicator's dot where it is on screen
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
}

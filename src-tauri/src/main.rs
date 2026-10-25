#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager, State, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use typr_lib::keyboard::{self, HotkeyKeys};
use typr_lib::local::{runtime::Paths as LocalPaths, LocalEngine, LocalStatus};
use typr_lib::logger::{self, LogEntry};
use typr_lib::postprocess::{self, ModelInfo};
use typr_lib::recorder::{self, Outcome, Recorder, RecordingState};
use typr_lib::settings::{Engine, PostProvider, RecordingMode, Settings};
use typr_lib::updates::{self, UpdateInfo};
use typr_lib::{ask, audio, autostart, i18n, links, modifier_hotkey, mouse, overlay, paste};

/// First check a little after startup, then this often
const UPDATE_CHECK_DELAY: Duration = Duration::from_secs(15);
const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);

/// The text of the last dictation and the window it was pasted into, for
/// "Paste last dictation" in the tray. Kept in memory only.
#[derive(Clone)]
struct LastDictation {
    text: String,
    window: isize,
}

/// What the whole app shares, managed by Tauri.
struct AppState {
    /// As saved in config.json; changed only through `apply_settings`
    settings: Mutex<Settings>,
    /// The folder of config.json
    config_dir: PathBuf,
    recorder: Recorder,
    /// Whether the push-to-talk hotkey is physically held down right now
    ptt_held: AtomicBool,
    /// Global hotkey listening; switched off from the tray (e.g. while gaming)
    hotkey_enabled: AtomicBool,
    /// The window records a new hotkey: Typr's own don't catch the keys
    hotkeys_paused: AtomicBool,
    last_dictation: Mutex<Option<LastDictation>>,
    /// A newer release found on GitHub
    update: Mutex<Option<UpdateInfo>>,
}

impl AppState {
    fn new(settings: Settings, config_dir: PathBuf) -> Self {
        AppState {
            settings: Mutex::new(settings),
            config_dir,
            recorder: Recorder::default(),
            ptt_held: AtomicBool::new(false),
            hotkey_enabled: AtomicBool::new(true),
            hotkeys_paused: AtomicBool::new(false),
            last_dictation: Mutex::new(None),
            update: Mutex::new(None),
        }
    }

    /// A copy of the settings as they are now, to use without holding the lock
    fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }
}

/// Tray menu items whose text or state changes while Typr runs
struct TrayMenu {
    menu: Menu<tauri::Wry>,
    open: MenuItem<tauri::Wry>,
    paste_last: MenuItem<tauri::Wry>,
    engine: Submenu<tauri::Wry>,
    engines: Vec<(Engine, CheckMenuItem<tauri::Wry>)>,
    hotkey: CheckMenuItem<tauri::Wry>,
    exit: MenuItem<tauri::Wry>,
    /// Added at the top of the menu once a newer release is found
    update: MenuItem<tauri::Wry>,
    update_shown: AtomicBool,
}

/// Set while the main window is being re-created, so two quick tray clicks
/// don't try to build it twice
static OPENING_MAIN_WINDOW: AtomicBool = AtomicBool::new(false);

fn update_tray_menu(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let (language, engine) = {
        let settings = state.settings.lock().unwrap();
        (i18n::resolve(&settings.language), settings.engine)
    };
    let enabled = state.hotkey_enabled.load(Ordering::SeqCst);
    let has_last = state.last_dictation.lock().unwrap().is_some();
    let update = state.update.lock().unwrap().clone();
    let Some(tray) = app.try_state::<TrayMenu>() else {
        return;
    };
    let _ = tray.open.set_text(i18n::tray_open(language));
    let _ = tray.paste_last.set_text(i18n::tray_paste_last(language));
    let _ = tray.paste_last.set_enabled(has_last);
    let _ = tray.engine.set_text(i18n::tray_engine(language));
    for (item_engine, item) in &tray.engines {
        let _ = item.set_text(i18n::engine_label(language, *item_engine));
        let _ = item.set_checked(*item_engine == engine);
    }
    let _ = tray.hotkey.set_text(i18n::tray_hotkey(language, enabled));
    let _ = tray.hotkey.set_checked(enabled);
    let _ = tray.exit.set_text(i18n::tray_exit(language));
    if let Some(update) = update {
        let _ = tray.update.set_text(i18n::tray_update(language, &update.version));
        if !tray.update_shown.swap(true, Ordering::SeqCst) {
            if let Err(e) = tray.menu.insert(&tray.update, 0) {
                log::debug!("Failed to add the update item to the tray menu: {}", e);
            }
        }
    }
}

/// Where config.json lives: `%APPDATA%\com.typr.app`. Development builds
/// take `TYPR_CONFIG_DIR` instead when it is set, so they can keep their
/// settings apart from the installed Typr.
fn config_dir() -> PathBuf {
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("TYPR_CONFIG_DIR") {
        return dir.into();
    }
    dirs::config_dir().unwrap_or_default().join("com.typr.app")
}

fn log_environment(config_dir: &Path, settings: &Settings) {
    log::info!(
        "Typr v{} on {} ({})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    log::info!("Config folder: {}", config_dir.display());
    log::info!("Settings: {}", settings.summary());
}

/// Builds the main window from tauri.conf.json. It isn't created at startup
/// by Tauri ("create": false): a start with Windows goes straight to the tray.
fn build_main_window(app: &tauri::AppHandle) {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
        .cloned();
    match config {
        Some(config) => {
            log::info!("Opening the main window");
            let built = WebviewWindowBuilder::from_config(app, &config).and_then(|builder| builder.build());
            match built {
                Ok(window) => {
                    #[cfg(windows)]
                    tint_window_border(&window);
                    let _ = window.set_focus();
                }
                Err(e) => log::error!("Failed to open the main window: {}", e),
            }
        }
        None => log::error!("Main window is missing from tauri.conf.json"),
    }
}

/// The main window has no system frame, yet Windows 11 still draws its 1 px
/// border, in the accent colour when that is on for title bars. Paint it the
/// app's own grey (--surface-raised) instead.
#[cfg(windows)]
fn tint_window_border(window: &tauri::WebviewWindow) {
    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR};

    const BORDER: u32 = 0x003a_3d3d; // #3d3d3a as COLORREF (0x00BBGGRR)
    if let Ok(hwnd) = window.hwnd() {
        // Windows 10 has no border colour: the call fails and nothing changes
        unsafe {
            DwmSetWindowAttribute(hwnd.0, DWMWA_BORDER_COLOR as u32, &BORDER as *const u32 as *const _, 4);
        }
    }
}

/// Focuses the main window, re-creating it if it was closed. Closing destroys
/// the window together with its WebView, so Typr idles in the tray without a
/// browser process.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }

    if OPENING_MAIN_WINDOW.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    // Building a window inside an event handler deadlocks on Windows, so use a thread
    std::thread::spawn(move || {
        build_main_window(&app);
        OPENING_MAIN_WINDOW.store(false, Ordering::SeqCst);
    });
}

/// Turns global hotkey listening on or off (tray menu).
fn set_hotkey_enabled(app: &tauri::AppHandle, enabled: bool) {
    let state = app.state::<AppState>();
    state.hotkey_enabled.store(enabled, Ordering::SeqCst);

    if enabled {
        if let Err(e) = register_hotkeys(app, &state.settings()) {
            log::error!("Failed to turn the hotkeys back on: {}", e);
        } else {
            log::info!("Hotkeys turned on from the tray");
        }
    } else {
        unregister_hotkeys(app);
        state.ptt_held.store(false, Ordering::SeqCst);
        // Never paste into whatever is focused now (e.g. a game)
        if state.recorder.cancel(app) {
            ask::recording_cancelled();
            typr_lib::transcribe_local::dictation_ended(app, &state.settings());
        }
        log::info!("Hotkey turned off from the tray, shortcuts are ignored until it is back on");
    }

    sync_overlay(app);
    update_tray_menu(app);
    if let Err(e) = app.emit("hotkey-enabled", enabled) {
        log::debug!("Failed to emit hotkey-enabled: {}", e);
    }
}

/// The recording indicator may appear only while it is switched on in General
/// and the hotkey is on (nothing pops up over games while it's off in the tray).
fn sync_overlay(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let allowed = state.hotkey_enabled.load(Ordering::SeqCst)
        && state.settings.lock().unwrap().show_indicator;
    overlay::set_enabled(app, allowed);
}

/// Applies settings saved from anywhere (the window or the tray): saves
/// them, logs what changed and updates everything that depends on them.
fn apply_settings(app: &tauri::AppHandle, settings: Settings) -> Result<(), String> {
    let state = app.state::<AppState>();
    let old = state.settings();
    // While the hotkeys are off (tray), the new ones are registered when they're turned back on
    let hotkeys_changed = settings.hotkey != old.hotkey
        || settings.ask_hotkey != old.ask_hotkey
        || settings.quick_ask.enabled != old.quick_ask.enabled;
    if hotkeys_changed && state.hotkey_enabled.load(Ordering::SeqCst) {
        if let Err(e) = register_hotkeys(app, &settings) {
            log::error!("Failed to register the new hotkeys: {}", e);
            let _ = register_hotkeys(app, &old);
            return Err(e);
        }
    }
    if let Err(e) = settings.save(&state.config_dir) {
        log::error!("Failed to save settings: {}", e);
        return Err(e);
    }

    if settings.developer_mode != old.developer_mode {
        logger::set_enabled(settings.developer_mode);
        if settings.developer_mode {
            log::info!("Developer mode enabled, logging started");
            log_environment(&state.config_dir, &settings);
        }
    }

    let changes = settings.describe_changes(&old);
    if !changes.is_empty() {
        log::info!("Settings saved: {}", changes.join("; "));
    }

    // Quick ask turned off: its window goes
    let close_ask_window = ask::is_open() && old.quick_ask.enabled && !settings.quick_ask.enabled;
    let indicator_changed = settings.show_indicator != old.show_indicator;
    let new_position = (settings.indicator_position != old.indicator_position).then_some(settings.indicator_position);
    let tray_changed = settings.language != old.language || settings.engine != old.engine;
    let updates_turned_on = settings.check_updates && !old.check_updates;
    app.state::<LocalEngine>().settings_changed(&old, &settings);
    *state.settings.lock().unwrap() = settings;
    if indicator_changed {
        sync_overlay(app);
    }
    if let Some(position) = new_position {
        overlay::set_position(app, position);
    }
    if tray_changed {
        update_tray_menu(app);
    }
    if close_ask_window {
        ask::close(app);
    }
    if updates_turned_on {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = run_update_check(&app).await;
        });
    }
    Ok(())
}

/// Engine chosen in the tray menu. The window follows through `engine-changed`.
fn set_engine_from_tray(app: &tauri::AppHandle, engine: Engine) {
    let mut settings = app.state::<AppState>().settings();
    if settings.engine != engine {
        log::info!("Engine switched to {} from the tray", engine.label());
        settings.engine = engine;
        if let Err(e) = apply_settings(app, settings) {
            log::error!("Failed to switch the engine: {}", e);
        }
    }
    // The clicked item toggled itself; put every check mark right again
    update_tray_menu(app);
    let current = app.state::<AppState>().settings.lock().unwrap().engine;
    if let Err(e) = app.emit("engine-changed", current) {
        log::debug!("Failed to emit engine-changed: {}", e);
    }
}

/// Pastes the last dictation again, into the window it went to. If that
/// window is gone, the text is put on the clipboard instead.
fn paste_last_dictation(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    if state.recorder.state() != RecordingState::Ready {
        log::info!("Paste last dictation ignored: a dictation is running");
        return;
    }
    let Some(last) = state.last_dictation.lock().unwrap().clone() else {
        return;
    };
    // Off the event loop: the paste waits for the window and the clipboard
    std::thread::spawn(move || {
        if paste::focus_window(last.window) {
            // Let the window take the focus back from the tray menu
            std::thread::sleep(Duration::from_millis(150));
            match paste::paste_text(&last.text) {
                Ok(()) => log::info!("Last dictation pasted again"),
                Err(e) => log::error!("Failed to paste the last dictation: {}", e),
            }
        } else {
            match paste::copy_text(&last.text) {
                Ok(()) => log::info!("The window of the last dictation is gone; its text is on the clipboard"),
                Err(e) => log::error!("Failed to copy the last dictation: {}", e),
            }
        }
    });
}

/// Looks for a newer release, remembers it and tells the window and the tray.
async fn run_update_check(app: &tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    let result = updates::check().await;
    match &result {
        Ok(update) => {
            *app.state::<AppState>().update.lock().unwrap() = update.clone();
            if let Err(e) = app.emit("update-available", update.clone()) {
                log::debug!("Failed to emit update-available: {}", e);
            }
            update_tray_menu(app);
        }
        Err(e) => log::warn!("{}", e),
    }
    result
}

fn start_update_checks(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(UPDATE_CHECK_DELAY).await;
        loop {
            let enabled = app.state::<AppState>().settings.lock().unwrap().check_updates;
            if enabled {
                let _ = run_update_check(&app).await;
            }
            tokio::time::sleep(UPDATE_CHECK_INTERVAL).await;
        }
    });
}

// ── Commands of the main window ─────────────────────

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings()
}

/// Every control in the window saves the whole settings object.
#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: Settings) -> Result<(), String> {
    apply_settings(&app, settings)
}

#[tauri::command]
fn list_microphones() -> Vec<audio::MicDevice> {
    let devices = audio::list_microphones();
    let names: Vec<&str> = devices.iter().map(|device| device.name.as_str()).collect();
    log::debug!("Input devices ({}): {}", names.len(), names.join(", "));
    devices
}

#[tauri::command]
fn get_recording_state(state: State<AppState>) -> RecordingState {
    state.recorder.state()
}

/// The window's own start/stop, the same as a press of the toggle hotkey.
#[tauri::command]
async fn toggle_recording(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    toggle(&app, &state).await
}

#[tauri::command]
fn get_hotkey_enabled(state: State<AppState>) -> bool {
    state.hotkey_enabled.load(Ordering::SeqCst)
}

/// While the window records a new hotkey, Typr's own hotkeys let the keys
/// and buttons through: a registered combination would never reach the page,
/// which would then see only its modifiers.
#[tauri::command]
fn pause_hotkeys(app: tauri::AppHandle, state: State<AppState>, paused: bool) {
    if state.hotkeys_paused.swap(paused, Ordering::SeqCst) == paused {
        return;
    }
    if paused {
        unregister_hotkeys(&app);
        log::debug!("Hotkeys paused while a new one is recorded");
    } else {
        resume_hotkeys(&app);
    }
}

/// Listens to the hotkeys again after a pause, unless they are off in the tray.
fn resume_hotkeys(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    if !state.hotkey_enabled.load(Ordering::SeqCst) {
        return;
    }
    if let Err(e) = register_hotkeys(app, &state.settings()) {
        log::error!("Failed to listen to the hotkeys again: {}", e);
    }
}

/// Interface language that suits the system ("en" or "ru"), used while
/// no language is chosen in General
#[tauri::command]
fn get_system_language() -> String {
    i18n::system_language().to_string()
}

/// Models of a language model provider, for Post-processing or Quick ask.
/// `api_key` is the key in the window, which may not be saved yet; without
/// it the saved one is used.
#[tauri::command]
async fn list_postprocess_models(
    state: State<'_, AppState>,
    provider: String,
    api_key: Option<String>,
) -> Result<Vec<ModelInfo>, String> {
    let provider = PostProvider::parse(&provider)
        .ok_or_else(|| format!("Unknown post-processing provider: {}", provider))?;
    let api_key = api_key
        .filter(|key| !key.trim().is_empty())
        .unwrap_or_else(|| state.settings.lock().unwrap().api_keys.model(provider).to_string());
    postprocess::list_models(provider, &api_key)
        .await
        .map_err(|e| {
            log::warn!("Could not load post-processing models: {}", e);
            e
        })
}

#[tauri::command]
fn get_autostart() -> bool {
    autostart::is_enabled()
}

#[tauri::command]
fn set_autostart(enabled: bool) -> Result<(), String> {
    autostart::set_enabled(enabled)
}

/// The newer release found by the last check, if any
#[tauri::command]
fn get_update(state: State<AppState>) -> Option<UpdateInfo> {
    state.update.lock().unwrap().clone()
}

#[tauri::command]
async fn check_for_updates(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    run_update_check(&app).await
}

/// Opens a page in the browser (release page, where to get API keys)
#[tauri::command]
fn open_link(url: String) -> Result<(), String> {
    links::open(&url)
}

// ── Local recognition ───────────────────────────────

#[tauri::command]
async fn local_status(engine: State<'_, LocalEngine>) -> Result<LocalStatus, String> {
    Ok(engine.status())
}

#[tauri::command]
async fn local_download_model(engine: State<'_, LocalEngine>, id: String) -> Result<(), String> {
    engine.download_model(&id)
}

#[tauri::command]
async fn local_download_runtime(engine: State<'_, LocalEngine>) -> Result<(), String> {
    engine.download_runtime()
}

/// `key` is "model:<id>" or "runtime"
#[tauri::command]
async fn local_cancel_download(engine: State<'_, LocalEngine>, key: String) -> Result<(), String> {
    engine.cancel_download(&key);
    Ok(())
}

#[tauri::command]
async fn local_delete_model(engine: State<'_, LocalEngine>, id: String) -> Result<(), String> {
    // Waits for a running dictation to release the model
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.delete_model(&id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_logs() -> Vec<LogEntry> {
    logger::entries()
}

#[tauri::command]
fn clear_logs() {
    logger::clear();
}

#[tauri::command]
fn frontend_log(level: String, message: String) {
    logger::record(logger::parse_level(&level), "ui", message);
}

fn begin_recording(app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    let settings = state.settings();
    if let Err(e) = state.recorder.start(app, &settings.microphone) {
        log::error!("Failed to start recording: {}", e);
        recorder::notify_error(app, &e);
        return Err(e);
    }
    // While the user speaks: the local model loads, or the connections to
    // the cloud open
    if settings.engine == Engine::Local {
        app.state::<LocalEngine>()
            .preload(&settings.local_model, &settings.local_backend);
    }
    recorder::warm_up_connections(&settings);
    // Quick ask: the selection is read meanwhile, it decides what this is
    ask::recording_started(&settings);
    Ok(())
}

async fn finish_recording(app: &tauri::AppHandle, state: &AppState) -> Result<String, String> {
    let settings = state.settings();
    match state.recorder.stop_and_transcribe(app, &settings).await? {
        Outcome::Pasted(text) => {
            if !text.is_empty() {
                // Right after the paste the target window is still in front
                *state.last_dictation.lock().unwrap() = Some(LastDictation {
                    text: text.clone(),
                    window: paste::foreground_window(),
                });
                update_tray_menu(app);
            }
            Ok(text)
        }
        Outcome::Asked => Ok("asked".to_string()),
    }
}

/// Toggle mode (and the window's button): a press starts a dictation, the
/// next one stops it. Answers "recording" or what was pasted.
async fn toggle(app: &tauri::AppHandle, state: &AppState) -> Result<String, String> {
    match state.recorder.state() {
        RecordingState::Ready => begin_recording(app, state).map(|()| "recording".to_string()),
        RecordingState::Recording => finish_recording(app, state).await,
        RecordingState::Transcribing => {
            log::warn!("Hotkey ignored: still transcribing the previous recording");
            Err("Currently transcribing, please wait".to_string())
        }
    }
}

async fn push_to_talk_pressed(app: &tauri::AppHandle, state: &AppState) {
    let current = state.recorder.state();
    if current != RecordingState::Ready {
        log::debug!("Push-to-talk press ignored, recorder is {:?}", current);
        return;
    }
    if begin_recording(app, state).is_err() {
        return;
    }
    // The key may have been released while the microphone was opening;
    // the release handler then saw no recording, so stop it here.
    if !state.ptt_held.load(Ordering::SeqCst) {
        log::debug!("Push-to-talk key was released while the microphone was opening");
        if let Err(e) = finish_recording(app, state).await {
            log::debug!("Push-to-talk stop: {}", e);
        }
    }
}

async fn push_to_talk_released(app: &tauri::AppHandle, state: &AppState) {
    if state.recorder.state() != RecordingState::Recording {
        return;
    }
    if let Err(e) = finish_recording(app, state).await {
        log::debug!("Push-to-talk stop: {}", e);
    }
}

/// The hotkey (keys or a mouse button) went down (`pressed`) or up.
fn on_hotkey(app: &tauri::AppHandle, pressed: bool, hotkey_keys: &HotkeyKeys, name: &str) {
    let state = app.state::<AppState>();
    let mode = state.settings.lock().unwrap().recording_mode;
    let push_to_talk = mode == RecordingMode::PushToTalk;
    let handle = app.clone();

    if pressed {
        log::debug!("Hotkey pressed: {} (mode: {})", name, mode);
        // A dictation starts only when nothing but the hotkey is held,
        // so pressing it by accident along with other keys (e.g. W
        // while gaming) does nothing. Stopping is never blocked.
        if state.recorder.state() == RecordingState::Ready {
            let extra = hotkey_keys.extra_keys_held();
            if !extra.is_empty() {
                log::info!("Hotkey ignored: other keys are held too ({})", extra.join(", "));
                return;
            }
        }
        if push_to_talk {
            state.ptt_held.store(true, Ordering::SeqCst);
        }
        tauri::async_runtime::spawn(async move {
            let state = handle.state::<AppState>();
            if push_to_talk {
                push_to_talk_pressed(&handle, &state).await;
            } else {
                match toggle(&handle, &state).await {
                    Ok(result) => log::debug!("Toggle result: {}", result),
                    Err(e) => log::debug!("Toggle: {}", e),
                }
            }
        });
    } else {
        log::debug!("Hotkey released: {} (mode: {})", name, mode);
        if push_to_talk {
            state.ptt_held.store(false, Ordering::SeqCst);
            tauri::async_runtime::spawn(async move {
                push_to_talk_released(&handle, &handle.state::<AppState>()).await;
            });
        }
    }
}

/// The quick ask hotkey was pressed: the window opens empty, or the window
/// open closes. `keys` are checked like the dictation hotkey's: nothing else
/// may be held (a hotkey of modifiers alone checks that itself).
fn on_ask_hotkey(app: &tauri::AppHandle, keys: Option<&HotkeyKeys>, name: &str) {
    if let Some(keys) = keys {
        let extra = keys.extra_keys_held();
        if !extra.is_empty() {
            log::info!("Quick ask hotkey ignored: other keys are held too ({})", extra.join(", "));
            return;
        }
    }
    let settings = app.state::<AppState>().settings();
    if !settings.quick_ask.enabled {
        return;
    }
    log::debug!("Quick ask hotkey pressed: {}", name);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        ask::toggle(&app, &settings).await;
    });
}

/// Stops listening to the hotkeys: keys, modifiers and mouse buttons alike.
fn unregister_hotkeys(app: &tauri::AppHandle) {
    if let Err(e) = app.global_shortcut().unregister_all() {
        log::warn!("Failed to unregister shortcuts: {}", e);
    }
    mouse::stop_all();
    modifier_hotkey::stop();
}

/// Listens to the dictation hotkey and, with quick ask on, to its own one.
fn register_hotkeys(app: &tauri::AppHandle, settings: &Settings) -> Result<(), String> {
    unregister_hotkeys(app);
    let dictation = register_dictation_hotkey(app, &settings.hotkey);
    let ask = if settings.quick_ask.enabled {
        register_ask_hotkey(app, &settings.ask_hotkey, &settings.hotkey)
    } else {
        Ok(())
    };
    dictation.and(ask)
}

/// Listens to `hotkey_str` for dictation: a key combination through the
/// global shortcut plugin, or a single mouse button (MouseMiddle, MouseBack,
/// MouseForward) through a mouse hook.
fn register_dictation_hotkey(app: &tauri::AppHandle, hotkey_str: &str) -> Result<(), String> {
    let normalized = keyboard::canonical(hotkey_str);
    let hotkey_keys = HotkeyKeys::parse(&normalized);

    if let Some(button) = mouse::MouseButton::parse(&normalized) {
        let handle = app.clone();
        mouse::start(mouse::Slot::Dictation, button, move |pressed| {
            on_hotkey(&handle, pressed, &hotkey_keys, button.id())
        })
        .map_err(|e| format!("Failed to use {} as the hotkey: {}", button.id(), e))?;
        log::info!("Mouse button hotkey registered: {}", button.id());
        return Ok(());
    }

    log::info!("Registering global shortcut: {}", normalized);
    let handle = app.clone();
    app.global_shortcut()
        .on_shortcut(normalized.as_str(), move |_app, shortcut, event| {
            let name = format!("{:?}", shortcut);
            on_hotkey(&handle, event.state == ShortcutState::Pressed, &hotkey_keys, &name);
        })
        .map_err(|e| format!("Failed to register shortcut '{}': {}", normalized, e))?;

    log::info!("Global shortcut registered: {}", normalized);
    Ok(())
}

/// Listens to `hotkey_str` for the quick ask window: modifiers alone (e.g.
/// Ctrl+Alt) through a keyboard hook, a mouse button through the mouse hook,
/// or a key combination through the global shortcut plugin. It must differ
/// from the dictation hotkey.
fn register_ask_hotkey(app: &tauri::AppHandle, hotkey_str: &str, dictation: &str) -> Result<(), String> {
    let normalized = keyboard::canonical(hotkey_str);
    if normalized.is_empty() {
        return Err("The quick ask hotkey isn't set".to_string());
    }
    if normalized.eq_ignore_ascii_case(&keyboard::canonical(dictation)) {
        return Err(format!("{} is already the dictation hotkey", normalized));
    }
    let keys = HotkeyKeys::parse(&normalized);

    if let Some(modifiers) = modifier_hotkey::Modifiers::parse(&normalized) {
        let handle = app.clone();
        let name = normalized.clone();
        modifier_hotkey::start(modifiers, move || on_ask_hotkey(&handle, None, &name))
            .map_err(|e| format!("Failed to use {} as the quick ask hotkey: {}", normalized, e))?;
        log::info!("Quick ask hotkey registered: {} (modifiers alone)", normalized);
        return Ok(());
    }

    if let Some(button) = mouse::MouseButton::parse(&normalized) {
        let handle = app.clone();
        mouse::start(mouse::Slot::Ask, button, move |pressed| {
            if pressed {
                on_ask_hotkey(&handle, Some(&keys), button.id());
            }
        })
        .map_err(|e| format!("Failed to use {} as the quick ask hotkey: {}", button.id(), e))?;
        log::info!("Quick ask hotkey registered: {}", button.id());
        return Ok(());
    }

    let handle = app.clone();
    app.global_shortcut()
        .on_shortcut(normalized.as_str(), move |_app, shortcut, event| {
            if event.state == ShortcutState::Pressed {
                on_ask_hotkey(&handle, Some(&keys), &format!("{:?}", shortcut));
            }
        })
        .map_err(|e| format!("Failed to register the quick ask shortcut '{}': {}", normalized, e))?;
    log::info!("Quick ask hotkey registered: {}", normalized);
    Ok(())
}

/// The tray icon and its menu, in the interface language.
fn build_tray(app: &tauri::App, settings: &Settings) -> tauri::Result<()> {
    let language = i18n::resolve(&settings.language);
    let open = MenuItem::with_id(app, "open", i18n::tray_open(language), true, None::<&str>)?;
    let paste_last = MenuItem::with_id(app, "paste-last", i18n::tray_paste_last(language), false, None::<&str>)?;
    let engines = Engine::ALL
        .iter()
        .map(|&engine| {
            CheckMenuItem::with_id(
                app,
                format!("engine:{}", engine.id()),
                i18n::engine_label(language, engine),
                true,
                engine == settings.engine,
                None::<&str>,
            )
            .map(|item| (engine, item))
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let engine_items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> =
        engines.iter().map(|(_, item)| item as &dyn tauri::menu::IsMenuItem<tauri::Wry>).collect();
    let engine = Submenu::with_items(app, i18n::tray_engine(language), true, &engine_items)?;
    let hotkey = CheckMenuItem::with_id(
        app,
        "toggle-hotkey",
        i18n::tray_hotkey(language, true),
        true,
        true,
        None::<&str>,
    )?;
    let exit = MenuItem::with_id(app, "exit", i18n::tray_exit(language), true, None::<&str>)?;
    let update = MenuItem::with_id(app, "update", "", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &paste_last,
            &engine,
            &PredefinedMenuItem::separator(app)?,
            &hotkey,
            &PredefinedMenuItem::separator(app)?,
            &exit,
        ],
    )?;

    // The app's icon; the 32 px file should a build come without one
    let icon = app.default_window_icon().cloned().unwrap_or_else(|| {
        tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png")).expect("tray icon")
    });

    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("Typr")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click { button, .. } = event {
                if button == tauri::tray::MouseButton::Left {
                    log::debug!("Tray icon clicked, showing the main window");
                    show_main_window(tray.app_handle());
                }
            }
        })
        .on_menu_event(|app, event| {
            let id = event.id.as_ref();
            if let Some(engine) = id.strip_prefix("engine:").and_then(Engine::parse) {
                set_engine_from_tray(app, engine);
                return;
            }
            match id {
                "open" => show_main_window(app),
                "paste-last" => paste_last_dictation(app),
                "update" => {
                    let url = app.state::<AppState>().update.lock().unwrap().as_ref().map(|u| u.url.clone());
                    if let Some(url) = url {
                        if let Err(e) = links::open(&url) {
                            log::error!("{}", e);
                        }
                    }
                }
                "exit" => {
                    log::info!("Tray Exit clicked, terminating application");
                    app.exit(0);
                }
                "toggle-hotkey" => {
                    let enabled = !app.state::<AppState>().hotkey_enabled.load(Ordering::SeqCst);
                    set_hotkey_enabled(app, enabled);
                }
                _ => {}
            }
        })
        .build(app)?;

    app.manage(TrayMenu {
        menu,
        open,
        paste_last,
        engine,
        engines,
        hotkey,
        exit,
        update,
        update_shown: AtomicBool::new(false),
    });
    Ok(())
}

fn main() {
    // Started by Typr itself to run a local model (local/worker.rs)
    if std::env::args().nth(1).as_deref() == Some(typr_lib::local::worker::ARG) {
        std::process::exit(typr_lib::local::worker::run());
    }
    // The quick ask window on its own: a demo and its measurements
    #[cfg(target_os = "windows")]
    {
        use typr_lib::ask::native::demo;
        let args: Vec<String> = std::env::args().skip(1).collect();
        match args.first().map(String::as_str) {
            Some(demo::DEMO_ARG) => return demo::run_demo(&args[1..]),
            Some(demo::BENCH_ARG) => return demo::run_bench(),
            Some(demo::CHILD_ARG) => return demo::run_child(),
            _ => {}
        }
    }

    logger::init();

    let config_dir = config_dir();
    let settings = Settings::load(&config_dir);
    logger::set_enabled(settings.developer_mode);
    let startup_settings = settings.clone();
    let startup_dir = config_dir.clone();
    let autostarted = std::env::args().any(|arg| arg == autostart::ARG);

    let app = tauri::Builder::default()
        // Must be the first plugin, so a second launch is intercepted early
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            log::info!("Second instance launched, focusing the existing window");
            show_main_window(app);
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(AppState::new(settings, config_dir))
        .manage(LocalEngine::new(LocalPaths::new(LocalPaths::default_root())))
        .invoke_handler(tauri::generate_handler![
            // Settings and the dictation
            get_settings, save_settings, get_system_language, list_microphones,
            get_recording_state, toggle_recording, get_hotkey_enabled, pause_hotkeys,
            list_postprocess_models,
            // General, updates and links
            get_autostart, set_autostart, get_update, check_for_updates, open_link,
            // Local recognition
            local_status, local_download_model, local_download_runtime, local_cancel_download,
            local_delete_model,
            // Developer
            get_logs, clear_logs, frontend_log,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                if window.label() == "main" {
                    log::info!("Main window closed and its WebView released, Typr keeps running in the tray");
                    // Closed while recording a new hotkey
                    let app = window.app_handle();
                    if app.state::<AppState>().hotkeys_paused.swap(false, Ordering::SeqCst) {
                        resume_hotkeys(app);
                    }
                }
            }
        })
        .setup(move |app| {
            logger::attach_app(app.handle().clone());
            app.state::<LocalEngine>().attach(app.handle().clone());
            app.state::<LocalEngine>().restore_runtime();
            log_environment(&startup_dir, &startup_settings);
            autostart::refresh();
            // A question typed into the quick ask window's field reads them when sent
            ask::read_settings_with(|app| app.state::<AppState>().settings());

            build_tray(app, &startup_settings)?;

            // Recording indicator (small mic, always on top, top right or bottom
            // centre). It waits behind the edge of the screen or the taskbar and
            // slides in only while dictating
            overlay::create(app.handle(), startup_settings.indicator_position);
            sync_overlay(app.handle());

            if let Err(e) = register_hotkeys(app.handle(), &startup_settings) {
                log::error!("Failed to register the hotkeys at startup: {}", e);
            }

            // Started with Windows: straight to the tray, unless the first-run
            // setup still has to be done
            if autostarted && startup_settings.setup_done {
                log::info!("Started with Windows, waiting in the tray");
            } else {
                build_main_window(app.handle());
            }

            start_update_checks(app.handle());

            // Debug builds: TYPR_ASK_PREVIEW="question" asks it about the
            // selection in front a few seconds after start (ask::preview)
            #[cfg(debug_assertions)]
            if let Some(question) = std::env::var("TYPR_ASK_PREVIEW").ok().filter(|q| !q.trim().is_empty()) {
                let app = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    let settings = app.state::<AppState>().settings();
                    ask::preview(&app, &settings, &question).await;
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app, event| {
        // Closing the main window leaves no windows open; Typr keeps running
        // in the tray. Only an explicit exit (tray → Exit) carries a code.
        if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}

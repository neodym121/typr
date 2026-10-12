# AGENTS.md — Typr

Typr is a **desktop dictation app** built with [Tauri 2](https://tauri.app/) (Rust backend) and vanilla TypeScript frontend. It records audio via a global hotkey, transcribes it locally with [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp) or in the cloud with Groq, OpenAI (or any OpenAI-compatible API), Polza.AI or AssemblyAI, optionally polishes the text with a language model, and auto-pastes the result into the active window. With text selected, the same hotkey asks a language model about it instead (**quick ask**): the recording indicator unfolds into a small window with the answer. The interface is in English and Russian.

---

## Architecture

```
typr/
├── src/                        # TypeScript frontend (Vite)
│   ├── main.ts                 # Main UI logic, settings form, hotkey binding
│   ├── setup.ts                # First-run setup wizard over the whole window
│   ├── i18n.ts                 # English/Russian UI strings, t(), data-i18n attributes, recognition language names
│   ├── dropdown.ts             # App-styled dropdowns over hidden native <select>s
│   ├── postprocess.ts          # Post-processing section: switch, styles; provider and model through modelpicker.ts
│   ├── modelpicker.ts          # Provider, API key and model list with search, shared by Post-processing and Quick ask
│   ├── quickask.ts             # Quick ask section: switch and the model that answers
│   ├── ask.html, ask.ts, ask.css # Quick ask window: the indicator's dot unfolding into the answer card
│   ├── markdown.ts             # Markdown → DOM for the answers (never parses the model's text as HTML)
│   ├── local.ts                # Engine → Local: models, acceleration, runtime components, unloading
│   ├── overlay.html            # Mic indicator overlay (WebView fallback for non-Windows)
│   └── style.css               # Global styles
├── src-tauri/                  # Rust backend (Tauri)
│   └── src/
│       ├── lib.rs              # Declares the modules (crate `typr_lib`, used by main.rs and tests/)
│       ├── main.rs             # App entry: Tauri builder, commands, AppState, tray, hotkey, applying settings
│       ├── recorder.rs         # Orchestrates recording → transcription → paste flow, local fallback
│       ├── audio.rs            # Low-level audio capture (cpal), WAV/FLAC packing in memory
│       ├── transcribe_groq.rs  # Groq Whisper API client
│       ├── transcribe_openai.rs# OpenAI / OpenAI-compatible API client
│       ├── transcribe_polza.rs # Polza.AI client (JSON body, base64 data URL)
│       ├── transcribe_assemblyai.rs # AssemblyAI client (upload → transcript job → poll)
│       ├── transcribe_local.rs # Local engine entry: in-memory samples → local::LocalEngine
│       ├── local/              # Local recognition with transcribe.cpp
│       │   ├── mod.rs          # LocalEngine: runtime + model in memory, preload, unload timer, downloads, status
│       │   ├── catalog.rs      # Models and the runtime with pinned URLs, sizes, SHA-256
│       │   ├── hardware.rs     # GPUs from the registry, whether Vulkan is available
│       │   ├── download.rs     # Resumable, cancellable, verified downloads
│       │   ├── runtime.rs      # Files on disk, unpacking and checking the runtime
│       │   ├── worker.rs       # The recognition process (`typr --local-recognition`) and Typr's side of it
│       │   └── ffi.rs          # transcribe.dll loaded at run time (C API of v0.2.4)
│       ├── keyboard.rs         # Hotkey must be pressed on its own (GetAsyncKeyState check)
│       ├── mouse.rs            # A mouse button (middle, back, forward) as the hotkey, low-level mouse hook
│       ├── i18n.rs             # System language detection, tray menu strings
│       ├── net.rs              # One shared HTTP client (timeouts, kept-alive connections, warm-up) + response/error helpers
│       ├── autostart.rs        # Start with Windows (HKCU Run value, `--autostart`)
│       ├── updates.rs          # Latest GitHub release vs this version
│       ├── links.rs            # Opens allowed web pages in the browser
│       ├── logger.rs           # `log` backend feeding the Developer section
│       ├── overlay.rs          # Recording indicator that slides in while dictating (native layered window on Windows)
│       ├── cleanup.rs          # Tidies the transcript: whitespace, capitals at sentence starts, final punctuation
│       ├── paste.rs            # Pastes via the clipboard (Ctrl+V), then restores the previous content
│       ├── postprocess.rs      # LLM post-processing: model lists, Gemini / OpenAI-compatible calls, prompts
│       ├── selection.rs        # Reads the selected text of the window in front (verified synthetic copy, clipboard restored)
│       ├── ask/                # Quick ask
│       │   ├── mod.rs          # Question or dictation, the conversation, the answer streamed to the window
│       │   ├── stream.rs       # Streaming chat: Gemini `streamGenerateContent` / OpenAI-compatible SSE, <think> filter
│       │   └── window.rs       # Transparent no-focus WebView frame over the indicator, click-through outside the card
│       └── settings.rs         # Settings struct, load/save from config.json
├── .github/workflows/
│   └── build.yml               # CI: builds Windows installer on tag push or manual trigger
├── index.html                  # Main window HTML shell
├── package.json                # npm scripts: dev, build, tauri
├── vite.config.ts              # Vite config
└── src-tauri/tauri.conf.json   # Tauri app config (window, bundle, CSP)
```

---

## Key Modules

### `recorder.rs` — Central orchestrator
Manages the `RecordingState` state machine (`Ready → Recording → Transcribing → Ready`). On toggle:
1. Starts audio capture via `AudioRecorder`
2. Packs the samples in memory for a cloud engine (FLAC for Groq, AssemblyAI and OpenAI itself; WAV for Polza and other OpenAI-compatible servers); nothing is written to disk
3. Calls the configured transcription engine
   (the local engine takes the samples straight to transcribe.cpp). If a cloud engine fails for any reason, `fallbackLocal` is on and a local model is downloaded, the local model transcribes the recording instead (`transcription-fallback` event); the next dictation tries the cloud again
4. Runs `cleanup_text()` on the result
5. If quick ask claims the recording (`ask::claim`: something was selected, or its window is open), the text is a question: `ask::ask` answers it and nothing is pasted (`Outcome::Asked`); otherwise:
6. If post-processing is on, rewrites the text with `postprocess::process()`; on failure the transcribed text is pasted anyway and the error is reported afterwards
7. Calls `paste_text()` to inject into the active window
8. Emits `recording-state` events to the frontend and updates the overlay (the indicator slides in for Recording/Transcribing and back out on Ready)

### `settings.rs` — Configuration
Persisted to `config.json` in the Tauri app data directory (`TYPR_CONFIG_DIR` overrides the folder in debug builds). `#[serde(default)]` on the structs: a missing field takes its value from `Default`. Engine, recording mode, post-processing provider and preset are enums saved as string ids (`string_enum!`); an unknown id reads as the default instead of failing the file. Key fields:
- `engine`: `"local"` | `"groq"` | `"openai"` (`"openai-compatible"` reads as it) | `"polza"` | `"assemblyai"` (default: `"groq"`)
- `groqModel`: `"whisper-large-v3-turbo"` (default) | `"whisper-large-v3"`
- `groqApiKey`, `openaiApiKey`, `openaiEndpoint`, `openaiModel`, `polzaApiKey`, `polzaModel`, `polzaProvider`, `assemblyaiApiKey`
- `assemblyaiModel`: `"universal-3-5-pro"` (default, no Russian) | `"universal-2"`
- `recognitionLanguage`: ISO 639-1 code of the speech (`RECOGNITION_LANGUAGES`), empty detects it. Sent as `language` (Groq, OpenAI, Polza), `language_code` (AssemblyAI) and the run params' `language` (local; a language the model doesn't list falls back to detection)
- `fallbackLocal`: a failed cloud transcription goes to a downloaded local model (default `true`)
- `localModel`: model id from `local/catalog.rs` used for dictation (default `"parakeet-tdt-0.6b-v3"`)
- `localBackend`: `"vulkan"` | `"cpu"`, or empty for the one recommended for this computer
- `localUnload`: when an idle model leaves memory: `"immediate"` | `"30s"` | `"5m"` (default) | `"10m"` | `"never"`
- `language`: `"en"` | `"ru"` | `""` (empty follows the system, see `i18n.rs`)
- `developerMode`: shows the Developer section and turns log collection on
- `microphone`: device name or `"default"`
- `recordingMode`: `"toggle"` | `"push-to-talk"`
- `hotkey`: default `"Ctrl+Shift+Space"`; or a single mouse button: `"MouseMiddle"` | `"MouseBack"` | `"MouseForward"`
- `appendSpace`: adds a space after the pasted text (default `false`, switch in Post-processing; works with post-processing off too)
- `showIndicator`: whether the recording indicator may appear at all (default `true`, switch in General)
- `indicatorPosition`: `"top-right"` (default) | `"bottom-center"` (just above the taskbar); General, shown while the indicator is on
- `checkUpdates`: looks for a newer GitHub release 15 s after start and every 12 h (default `true`)
- `setupDone`: the first-run wizard was finished or skipped; `false` only for a new config (a config file without the field counts as set up)
- `postProcess`: `enabled` (default `false`), `provider` (`"gemini"` | `"openrouter"` | `"groq"` | `"polza"`), `preset` (`"chill"` | `"proper"` | `"custom"`; the old `"official"` is migrated to `"proper"` on load), `customPrompt`, and `{ apiKey, model }` per provider, plus `providerId` for Polza (sub-provider sent as `provider.only`, omitted from the file when empty). Groq and Polza fall back to the Engine key when their own is empty (`Settings::post_process_key`)
- `quickAsk`: `enabled` (default `false`), `provider` (same four as post-processing) and `{ apiKey, model }` per provider, plus `providerId` for Polza. An empty key falls back to the post-processing key of that provider, then the Engine key (`Settings::quick_ask_key`). Local models aren't offered

### `transcribe_groq.rs` / `transcribe_openai.rs`
Both send a multipart form POST with the recording (`audio::EncodedAudio`) to the respective API. `transcribe_openai.rs` is also used for any OpenAI-compatible endpoint (e.g. local Whisper servers) via the configurable `openaiEndpoint`; those get WAV, only `api.openai.com` gets FLAC.

### `net.rs` — one HTTP client
`net::client()` returns one `reqwest::Client` for the whole app, so connections stay open between dictations. `recorder::warm_up_connections` sends a HEAD request (no credentials) to the engine's, post-processing's and quick ask's servers when a dictation starts, so the TLS handshake happens while the user speaks.

### `transcribe_assemblyai.rs`
Three steps: `POST /v2/upload` with the raw FLAC, `POST /v2/transcript` with `speech_models: [model]` and `language_code` (or `language_detection: true` when no language is set), then poll `GET /v2/transcript/{id}` until `completed` or `error`. Universal-3.5 Pro supports 18 languages without Russian; if it rejects the language, the error suggests Universal-2.

### `local/` — local recognition (transcribe.cpp)
- Nothing native is compiled with Typr. `ffi.rs` loads `transcribe.dll` with LoadLibrary from the **runtime** downloaded on demand, mirrors the v0.2.4 C header (`include/transcribe.h` at the tag) and refuses a DLL whose version or struct sizes (`transcribe_abi_struct_size`) differ. Bump `TRANSCRIBE_VERSION`/`HEADER_HASH` in `catalog.rs`, the structs in `ffi.rs` and the pinned bundle together.
- The runtime (`catalog.rs` `RUNTIME`) is the official `windows-x86_64-cpu-vulkan` bundle: Vulkan for any GPU with a Vulkan driver, CPU modules for every processor. Its `contract.json` must match `TRANSCRIBE_VERSION` and `HEADER_HASH`. CUDA and ROCm are left out on purpose: their kernels are compiled per GPU generation, so each needs its own builds and checks (the official CUDA bundle covers compute capability 7.5–9.0 only; ROCm has no official Windows build). Measured on an RX 7600 (Parakeet, 11 s of audio, warm): Vulkan 0.12 s, CPU 0.8 s.
- `hardware.rs`: GPUs from the display adapter class in the registry (PCI devices) and `vulkan-1.dll` in System32. A GPU with Vulkan → Vulkan recommended, otherwise the CPU (Vulkan greyed out).
- transcribe.cpp never loads into Typr itself: it can't be unloaded from a process (backend modules and the Vulkan driver stay, about 40 MB). `worker.rs` starts Typr's own executable with `--local-recognition` (checked first thing in `main()`, before the logger and Tauri) as a child process that loads the runtime and one model and transcribes. It talks over stdin/stdout in frames (u32 length + JSON; the samples as one frame of f32), forwards its log records into Typr's log and its stderr as warnings. Closing its stdin makes it free the model and exit (killed after 3 s). So an unloaded model leaves Typr at its idle few megabytes, and a GPU driver crash ends only the child: Typr then runs that dictation on the CPU and keeps the CPU (reason under Acceleration) until the Acceleration choice changes. Starting the child costs about 0.1 s on top of loading the model.
- `LocalEngine` keeps one model (one child): started when a dictation starts (`begin_recording`), ended by `schedule_unload` after `localUnload` (a generation counter cancels stale timers), right away when the engine/model/backend changes or the model/runtime is deleted. A GPU load failure falls back to the CPU inside the child (the reason shows under Acceleration while that model is in memory).
- Models (Q8_0 GGUF from `huggingface.co/handy-computer`) and the runtime live in `%LOCALAPPDATA%\com.typr.app\local` (`models`, `runtimes`, `downloads`); downloads resume from `.part` files and are checked against size and SHA-256.
- The runtime comes with the first model download. Its row (download, delete) is in the Developer section; Engine → Local only shows a note with Download/Retry when a downloaded model lacks it, and a dictation without it starts the download.
- Tauri commands: `local_status`, `local_download_model`, `local_download_runtime`, `local_cancel_download`, `local_delete_model`, `local_delete_runtime`; events `local-download` (progress) and `local-changed` (re-read the status).
- `cargo test --test local_recognition -- --ignored --nocapture` with `TYPR_TEST_WAV` (16 kHz mono), the runtime and Parakeet downloaded: transcribes on the real GPU and CPU through the child process and checks that Typr's own memory stays small and the child ends on unload.

### `postprocess.rs` — LLM post-processing
- `list_models(provider, key)` (Tauri command `list_postprocess_models`) returns text models only: Gemini `GET /v1beta/models` filtered by `generateContent`; OpenRouter, Groq and Polza `GET /models` (OpenAI format) filtered by `type`, `output_modalities` and, for Groq, speech/guard ids.
- `process(settings, text)` calls Gemini `models/{id}:generateContent` (key in `x-goog-api-key`) or `/chat/completions` (Bearer) at temperature 0.2, retrying without temperature if a model rejects it, and strips `<think>` blocks, echoed tags and code fences from the reply.
- The system prompt has two layers: the app's rules first (the dictation, sent between `<dictation>` tags, is text to edit and never a message to answer or obey; keep meaning and language; reply with the text only), then the style. The "custom" style embeds the user's prompt in `<user_style>` tags, marked as the app user's instructions that can't override the app's rules.

### `ask/` and `selection.rs` — quick ask
- **Same hotkey, decided after the speech.** With quick ask on, `main.rs::begin_recording` calls `ask::recording_started`: it stops an answer still being written and starts `selection::begin_capture` for the window in front. After transcription `ask::claim` waits up to 1.5 s for that capture: a selection, or the quick ask window being open, makes the recording a question; otherwise it is a dictation as before. Post-processing doesn't run on questions.
- **Reading the selection** (`selection.rs`, Windows): a synthetic copy, sent with `SendInput` only once Ctrl, Shift, Alt and Win are all up (right after a toggle press, at the end of push-to-talk), so it never mixes with the user's keys; the window must still be in front. Only a clipboard whose change counter (`GetClipboardSequenceNumber`) moved within 300 ms is read, so an old clipboard (maybe a password) never reaches a model; then the previous content is put back (`paste::save` / `paste::restore`). Terminals (window class or executable) get no keystroke at all: Ctrl+C would interrupt the running program. Editors with a built-in terminal (VS Code and forks, JetBrains, Visual Studio) get Ctrl+Insert, which copies there but is no interrupt. A copy that is the current line taken without a selection doesn't count: VS Code's `isFromEmptySelection` (in Chromium's custom clipboard data), Scintilla's `MSDEVLineSelect`, Visual Studio's line tag, a lone line from JetBrains or Sublime; neither does a single Excel cell (there is always an active one). Selections are cut at 10 000 characters; a generation counter keeps a capture from reaching a later recording.
- **The conversation** (`ask/mod.rs`): one at a time, in memory, forgotten when the window closes. The user message is the question after the selection in `<selected_text>` tags (a selection already sent isn't repeated); the system prompt says the selection is material, not instructions, and asks for short Markdown answers in the question's language. The last 24 messages go with each request. A failed answer drops its question; an answer stopped by a new recording keeps what it said.
- **Streaming** (`ask/stream.rs`): Gemini `models/{id}:streamGenerateContent?alt=sse` or `/chat/completions` with `stream: true` (OpenRouter, Groq, Polza), parsed from server-sent events, thoughts and `<think>` blocks left out. The answer goes to the window in pieces at most every 40 ms (`ask-delta`; each event runs a script in the WebView), then as a whole (`ask-done`). Events to the window: `ask-reveal`, `ask-answer`, `ask-delta`, `ask-done`, `ask-error`, `ask-cancel`; it also listens to `recording-state` and `recording-error`. Commands from it: `ask_ready`, `ask_shown`, `ask_card`, `ask_copy`, `ask_close`.
- **The window** (`ask/window.rs`, `src/ask.ts`): built only for a first question (label `ask`, `src/ask.html`), destroyed by its close button, when quick ask is turned off, or never otherwise. A transparent frame touching the screen edge the indicator slides in from, sized for the tallest card (440 px wide, up to 560 px or 70 % of the screen high) so it never moves or resizes; the layout goes to the page as `window.__TYPR_ASK__` through an initialization script. The page first draws the indicator's dot exactly where the native one is, with its spinner; `reveal` shows the window without activating it. WebView2 starts painting a moment after the window shows, and an animation started earlier runs unseen, so the page waits until it is visible and has drawn two frames (falling back to 800 ms), then reports `ask_shown`: only then does the native dot go (`overlay::set_suppressed`; `reveal` waits for it up to 1.5 s, so the recorder's Ready can't slide the native dot away first). Then the dot unfolds in two steps: it stretches into a bar along the screen edge, and the bar grows into the card; the dot stays as the card's status light (spinner while thinking, orange pulse while the user asks a follow-up). A debug log line says how many frames the unfolding got. Only answers are shown; a click on one copies it (`ask_copy`: the window has no focus, so the page can't use the clipboard; a mouse selection inside it is no click), and "Copied" shows in the status strip. The card has no scrollbars (the wheel scrolls; long code lines wrap). The cross folds the card back into the dot, which slides behind the edge.
- **Focus and clicks:** `focusable(false)` (`WS_EX_NOACTIVATE`) and `focused(false)`, so the app the question is about keeps the focus and its selection; `reveal` gives the focus back should Windows hand it over anyway. The page reports the card's rectangle (`ask_card`) and a thread checks the cursor every 30 ms, making the window click-through (`set_ignore_cursor_events`) everywhere else. No `title` tooltips in the page: WebView2 can leave one on screen after the window is destroyed.

### `paste.rs` — clean clipboard
Saves the clipboard (text, else image, else nothing), sets the dictated text with arboard's `exclude_from_monitoring()` (kept out of Win+V history, cloud sync and clipboard managers), sends Ctrl+V, waits `RESTORE_DELAY` (300 ms) and puts the previous content back, unless something else was copied meanwhile. If the paste fails, the text stays on the clipboard. Other formats (e.g. copied files) aren't restored.

### `dropdown.ts`
`enhanceSelect(select)` hides a native `<select>` and draws an app-styled button and list over it; the `<select>` stays the source of truth (`.value`, `change` events), option rebuilds are picked up by a `MutationObserver`, and after setting `.value` from code call `syncSelect(select)`. New `<select>`s should be enhanced too.

### Main window frame
The main window has no system title bar (`decorations: false` in tauri.conf.json) and can't be maximized (`maximizable: false`: no button, no double-click or snap to the top). Typr draws its own minimize and close buttons (`.window-controls` in `index.html`), fixed in the top-right corner above everything, the setup too. Elements with the `drag-region` class (the sidebar, the strip above the content, the setup's top strip and header) drag the window from their empty space through `startDragging` in `main.ts`; buttons, inputs and labels inside them don't. Close works like the system button did: the window and its WebView go, Typr stays in the tray. Tauri keeps the edges resizable; `tint_window_border` (main.rs) paints the 1 px border Windows 11 still draws in the app's grey instead of the accent colour.

### `mouse.rs` — a mouse button as the hotkey
`register_hotkey` (main.rs) sends key combinations to the global shortcut plugin and a mouse hotkey to `mouse::start`: a `WH_MOUSE_LL` hook on its own thread with a message loop. The hook proc only matches the bound button (ignoring injected clicks), swallows it (so "back" doesn't navigate the browser) and hands press/release to a handler thread; both paths end in `on_hotkey`. The hook exists only while a mouse hotkey is active (`unregister_hotkey` removes it, e.g. when the hotkey is turned off in the tray). The left and right buttons can't be the hotkey.

### `keyboard.rs` — hotkey pressed on its own
On `ShortcutState::Pressed` with the recorder `Ready`, `HotkeyKeys::extra_keys_held()` reads every key and mouse button via `GetAsyncKeyState`; if anything besides the hotkey's own keys is down, the press is ignored (logged). Stopping a recording is never blocked.

### Localization
`src/i18n.ts` holds the English and Russian strings. Static markup uses `data-i18n` (text), `data-i18n-placeholder` and `data-i18n-title`; code uses `t(key)`. New UI text must get keys in both dictionaries. The `language` setting picks the language; when empty, `get_system_language` (Rust, `sys-locale`) decides: Russian for ru, uk, be, kk, ky, tg, uz, tk, hy, az, English otherwise. The tray menu is translated in `i18n.rs`. Backend error messages and logs stay in English. The page is hidden (`.i18n-pending`) until the language is applied.

### `cleanup.rs`
Collapses whitespace, capitalizes the first letter of every sentence and adds a final period. A sentence ends at `.`, `!` or `?` followed by a space, not inside numbers (3.14), addresses (example.com) or abbreviations (т.е., e.g., см.), and not after an ellipsis. Words that already have a capital (iPhone) are left alone; a trailing comma becomes the period.

### Tray menu, autostart, updates
- Tray: Open settings, Paste last dictation (the last text goes back into the window it was pasted into; if that window is gone, it is put on the clipboard), Engine submenu (switches `engine`, the window follows through `engine-changed`), Hotkey On/Off, Exit, and a "Version X is available" item added on top once an update is found.
- The main window is `"create": false` in tauri.conf.json and built in `setup`; started with `--autostart` (the Run value from `autostart.rs`), Typr stays in the tray unless the setup isn't done. `autostart::refresh` repoints the Run value at the current executable (release builds only).
- `updates::check` reads `releases/latest` of `neodym121/typr-updated`; the result is kept in `AppState.update`, sent as `update-available` and shown in the sidebar and General. `links::open` opens only the release page and the providers' key pages.

### `setup.ts` — first-run wizard
Shown while `setupDone` is false (and from General → Advanced): interface language, engine (Local recommended when Vulkan is available, else Groq), the engine's model download or API key plus the speech language, then hotkey, recording mode, start with Windows and update checks. Finish or Skip sets `setupDone`.

### `logger.rs` — Developer section
Installed as the global `log` backend in `main()`. Use `log::info!` / `log::warn!` / `log::error!` / `log::debug!` everywhere in Rust (not `println!`). While `developerMode` is on, entries go to a 5000-entry ring buffer and are streamed to the main window as `log-entry` events; the frontend forwards its own console output via the `frontend_log` command. Never log API keys — use `Settings::summary()` / `describe_changes()`, which mask them.

`stop_and_transcribe` must always return the recorder to `Ready`, even on errors; failures are reported to the UI with `recorder::notify_error` (`recording-error` event).

### Memory: no WebView while idle
- `overlay.rs` draws the recording indicator itself on Windows (`windows-sys` layered window, per-pixel alpha, SDF rendering) — no WebView. Other platforms fall back to the WebView overlay `src/overlay.html`.
- The indicator is out of sight while idle: its window touches an edge of the primary screen's work area (the screen without the taskbar) and the disc is drawn beyond that edge, so it slides in from behind it when a dictation starts (or briefly after an error) and slides back out on `Ready`. `indicatorPosition` picks the edge: the top-right corner (window at the top edge, disc above it) or the bottom centre (window ending at the taskbar, disc below it). Once fully out, the window is hidden and its timers stop, so it costs no CPU.
- Each time the indicator starts to appear, `place()` in the native overlay reads the work area and scale again (`MonitorFromPoint`, `GetMonitorInfoW`, `GetDpiForMonitor`, as Tauri does), so a changed resolution, scale or taskbar is picked up without a restart. `overlay::set_position` (from `apply_settings`) moves it at once.
- `overlay::set_enabled` gates it: `main.rs::sync_overlay` allows it only while `showIndicator` is on and the hotkey is on in the tray. `overlay::set_suppressed` hides it at once while the quick ask window holds its place.
- The quick ask window is the only other WebView, and it exists only while an answer is on screen (`ask/window.rs`).
- Closing the main window destroys it (and its WebView); `show_main_window` re-creates it from `tauri.conf.json` on a separate thread (building windows in event handlers deadlocks on Windows).
- The page runs under a strict CSP (`tauri.conf.json`): scripts from the app only, styles and fonts also from Google Fonts, IPC. New external resources need a CSP entry. `RunEvent::ExitRequested` without a code is prevented, so Typr keeps running in the tray; tray → Exit calls `app.exit(0)`.
- The tray menu's "Hotkey: On/Off" item unregisters the global shortcut, cancels an unfinished recording and keeps the overlay from appearing (e.g. while gaming). The state is not persisted.

---

## Development Setup

### Prerequisites
- [Rust](https://rustup.rs/) (stable toolchain)
- [Node.js](https://nodejs.org/) 20+
- Tauri CLI v2: `npm install` pulls it as a dev dependency

### Run in dev mode
```bash
npm install
npm run tauri dev
```

- Quit the installed Typr from the tray first. Both builds take the same single-instance lock (`com.typr.app`), so the dev build would only focus the installed app's window and exit.
- Both builds read `%APPDATA%\com.typr.app\config.json`. To keep the dev config apart, set `TYPR_CONFIG_DIR` (debug builds only), e.g. `$env:TYPR_CONFIG_DIR = "$env:TEMP\typr-dev"` in PowerShell.
- Every log line also prints to the terminal as `[Typr][level][source] message`, with Developer mode on or off.
- The page runs only inside the Tauri window: in a plain browser (`npm run dev`, port 1420) `getCurrentWindow()` throws at load and the page stays hidden, so UI changes can't be previewed there. The quick ask window can: `/src/ask.html` (or `?bottom`) in a browser plays a demo of the dot unfolding, answers streaming and a follow-up.
- `TYPR_ASK_PREVIEW="question"` (debug builds) asks that question about the selection in front 4 s after start, without a microphone (`ask::preview`). With `--autostart` and a `TYPR_CONFIG_DIR` config that has `"setupDone": true` the dev build stays in the tray, so the app in front keeps its selection.

### Build for production (Windows)
```bash
npm run tauri build
# Output: src-tauri/target/release/bundle/
```

### Run Rust tests
```bash
cd src-tauri
cargo test
```

One module or test: `cargo test --lib cleanup::` or `cargo test --lib test_save_and_load`.

`build.rs` embeds the Common Controls v6 manifest (`windows-app-manifest.xml`) through the linker for every target, so test binaries that link Tauri start on Windows (otherwise `STATUS_ENTRYPOINT_NOT_FOUND`).

### Check the frontend
`npx tsc` (strict, unused locals and parameters are errors). There are no frontend tests or linters. `npm run build` (tsc + Vite) is Tauri's `beforeBuildCommand`, so a type error fails the installer build in CI.

---

## CI / Release

The GitHub Actions workflow (`.github/workflows/build.yml`) runs on:
- **Manual dispatch** from the Actions tab
- **Tag push** matching `v*` (e.g. `v0.2.0`)

It produces a Windows installer (`.msi` and `.exe`) uploaded as a build artifact.

**To cut a release:**
```bash
git tag v0.x.0
git push origin v0.x.0
```

The version bump goes into the commit with the change, whose title ends with `(vX.Y.Z)`. It changes `package.json`, `package-lock.json` (two places), `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` (the `typr` package), `src-tauri/tauri.conf.json`, the `#version-text` placeholder in `index.html` and the tag example in both READMEs. The update check compares the Cargo version with the latest release tag.

---

## AI Agent Guidelines

- **Frontend changes** (UI, styles, overlay): edit files in `src/`. The Tauri webview reloads automatically in dev mode.
- **Backend changes** (audio, transcription, settings): edit files in `src-tauri/src/`. Rust is compiled; always run `cargo check` after edits.
- **Adding a new transcription engine**: create a new `transcribe_<name>.rs` module, add it to `lib.rs` and add an `Engine` variant in `settings.rs`; the compiler then points at the matches to extend (`Engine::label`, and `transcribe`, `upload_format`, `warm_up_connections` in `recorder.rs`). The tray lists `Engine::ALL` by itself. In the UI: the engine button and its settings block in `index.html`, `setEngine` in `main.ts`, `EngineId`/`ENGINES` in `setup.ts`, strings in `i18n.ts`.
- **`main.rs` is the hub**: `AppState`, every `#[tauri::command]` (listed in `generate_handler!`), the tray, hotkey registration and `apply_settings`. Every settings change, from the window or the tray, goes through `apply_settings`: it re-registers the hotkey, saves `config.json`, logs the masked diff, tells `LocalEngine`, and updates the overlay and the tray. A setting that must take effect at once is handled there.
- **One settings object in the UI**: `currentSettings` in `main.ts`, typed by a TS `Settings` interface that mirrors the Rust struct's camelCase JSON. `local.ts`, `postprocess.ts` and `setup.ts` reach it through a host object (`settings()`, `save()`) and declare only the fields they use. Every control change sends the whole object to `save_settings`; there is no Save button.
- **Settings changes**: in `settings.rs` the field (`#[serde(rename = "camelCase")]`), `Default`, `summary()`, `describe_changes()` (API keys go in its masked `secrets` list) and the tests; `apply_settings` if it must take effect at once; in the UI the TS interface, the control in `index.html`, `bind(...)` and `fillForm()` in `main.ts`, and both dictionaries in `i18n.ts`.
- **Docs in step**: a feature change also updates this file, the feature lists in `README.md` and `README.ru.md` (the same text in Russian) and the strings in both languages. `docs/polza-ai-reference.md` has Polza.AI API notes (in Russian); `docs/superpowers/` is the original April 2026 design and plan, now outdated.
- **Never hardcode API keys.** They are stored in `config.json` in the user's app data directory, not in the source tree.
- **Do not commit** `node_modules/`, `dist/`, or `src-tauri/target/`. These are covered by `.gitignore`.
- **Prefer async Tauri commands** for any operation that touches the network or filesystem from Rust.
- After any Rust change, verify with: `cd src-tauri && cargo test`

# AGENTS.md — Typr

Typr is a **desktop dictation app** built with [Tauri 2](https://tauri.app/) (Rust backend) and vanilla TypeScript frontend. It records audio via a global hotkey, transcribes it locally with [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp) or in the cloud with Groq, AssemblyAI or Polza.AI, optionally polishes the text with a language model (and translates it into a chosen language), and auto-pastes the result into the active window. With text selected, the same hotkey asks a language model about it instead (**quick ask**): the recording indicator unfolds into a small window with the question and the answer. A hotkey of its own opens that window empty, to ask about anything; while it is open the dictation hotkey asks follow-ups, and its pencil opens a text field to type questions in (dictation then goes into the field). Each service's API key is set once, in Settings → API keys. The interface is in English and Russian.

---

## Architecture

```
typr/
├── src/                        # TypeScript frontend (Vite)
│   ├── main.ts                 # Main UI logic, settings form, the Settings window, hotkey binding
│   ├── setup.ts                # First-run setup wizard over the whole window
│   ├── i18n.ts                 # English/Russian UI strings, t(), data-i18n attributes, recognition language names, hotkey names
│   ├── apikeys.ts              # Settings → API keys: one key per service, read by the model lists
│   ├── hotkeys.ts              # Settings → Hotkeys: recording the dictation and quick ask hotkeys (the setup records the dictation one too)
│   ├── dropdown.ts             # App-styled dropdowns over hidden native <select>s
│   ├── segmented.ts            # Segmented controls: the light thumb springs between segments and can be dragged
│   ├── postprocess.ts          # Post-processing section: switch, translation and its language (flags), styles; provider and model through modelpicker.ts
│   ├── modelpicker.ts          # Provider, API key and model list with search, shared by Post-processing and Quick ask
│   ├── quickask.ts             # Quick ask section: its switch, the model that answers, web search
│   ├── icons.ts, icons/        # Lucide icons (SVG files bundled, ISC license): `<i data-icon>` placeholders, icon()
│   ├── flags/                  # Twemoji flags of the translation languages (SVG files unchanged, CC-BY 4.0)
│   ├── local.ts                # Engine → Local: models, acceleration, unloading, a note when the runtime is missing
│   ├── overlay.html            # Mic indicator overlay (WebView fallback for non-Windows)
│   ├── fonts/                  # Inter and Source Serif 4 bundled (woff2 split by script, fonts.css, OFL licenses)
│   └── style.css               # Global styles
├── src-tauri/                  # Rust backend (Tauri)
│   └── src/
│       ├── lib.rs              # Declares the modules (crate `typr_lib`, used by main.rs and tests/)
│       ├── main.rs             # App entry: Tauri builder, commands, AppState, tray, hotkey, applying settings
│       ├── recorder.rs         # Orchestrates recording → transcription → paste flow, local fallback
│       ├── audio.rs            # Low-level audio capture (cpal), WAV/FLAC packing in memory
│       ├── transcribe_groq.rs  # Groq Whisper API client
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
│       ├── mouse.rs            # A mouse button (middle, back, forward) as a hotkey, low-level mouse hook (dictation and quick ask slots)
│       ├── modifier_hotkey.rs  # Modifiers alone (Ctrl+Alt) as the quick ask hotkey, low-level keyboard hook
│       ├── i18n.rs             # System language detection, tray menu strings
│       ├── net.rs              # One shared HTTP client (timeouts, kept-alive connections, warm-up) + response/error helpers
│       ├── autostart.rs        # Start with Windows (HKCU Run value, `--autostart`)
│       ├── updates.rs          # Latest GitHub release vs this version
│       ├── links.rs            # Opens allowed web pages in the browser
│       ├── logger.rs           # `log` backend feeding Settings → Developer
│       ├── overlay.rs          # Recording indicator that slides in while dictating (native layered window on Windows)
│       ├── cleanup.rs          # Tidies the transcript: whitespace, capitals at sentence starts, final punctuation
│       ├── paste.rs            # Pastes via the clipboard (Ctrl+V), then restores the previous content
│       ├── postprocess.rs      # LLM post-processing: model lists, Gemini / OpenAI-compatible calls, prompts (style, translation)
│       ├── translate.rs        # Translation languages: names, whether a text is surely in one already
│       ├── selection.rs        # Reads the selected text of the window in front (verified synthetic copy, clipboard restored)
│       ├── ask/                # Quick ask
│       │   ├── mod.rs          # Question or dictation, the empty window, the conversation, the answer streamed to the window
│       │   ├── stream.rs       # Streaming chat: Gemini `streamGenerateContent` / OpenAI-compatible SSE, the web_search tool, <think> filter
│       │   ├── search.rs       # Exa web search for the web_search tool
│       │   ├── window.rs       # The window's place over the indicator, its commands and events (the interface to native/)
│       │   └── native/         # The window itself, drawn with Slint (Windows)
│       │       ├── mod.rs      # Its thread: start, send commands
│       │       ├── platform.rs # Slint on a native layered window: software renderer into the window's bitmap, input, message loop
│       │       ├── view.rs     # What the window shows and how it behaves: unfolding, questions and answers, copying, the text field, strings (en/ru)
│       │       ├── markdown.rs # Markdown → blocks and Slint's styled text (never read as HTML)
│       │       ├── shadow.rs   # The dot's shadow, which the software renderer can't draw, as an image
│       │       └── demo.rs     # `--ask-demo`, `--ask-bench`: the window on its own, and its measurements
│   └── ui/ask.slint            # The window's markup and animations
│       └── settings.rs         # Settings struct, load/save from config.json
├── .github/workflows/
│   └── build.yml               # CI: MSI installer + portable exe in one zip, on tag push or manual trigger
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
2. Packs the samples in memory for a cloud engine (FLAC for Groq and AssemblyAI, WAV for Polza); nothing is written to disk
3. Calls the configured transcription engine
   (the local engine takes the samples straight to transcribe.cpp). If a cloud engine fails for any reason, `fallbackLocal` is on and a local model is downloaded, the local model transcribes the recording instead (`transcription-fallback` event); the next dictation tries the cloud again
4. Runs `cleanup_text()` on the result
5. If quick ask claims the recording (`ask::claim`: something was selected, or its window is open), the text is a question: `ask::ask` answers it and nothing is pasted (`Outcome::Asked`); otherwise:
6. If translation is on and the text isn't surely in its language already (`translate::is_written_in`), it is to be translated. If post-processing is on or there is a translation to do, `postprocess::process()` rewrites the text (the style only with post-processing on, the translation only when needed): a dictation already in the target language goes to the model only for post-processing, or not at all. On failure the transcribed text is pasted anyway and the error is reported afterwards
7. Calls `paste_text()` to inject into the active window
8. Emits `recording-state` events to the frontend and updates the overlay (the indicator slides in for Recording/Transcribing and back out on Ready)

### `settings.rs` — Configuration
Persisted to `config.json` in the Tauri app data directory (`TYPR_CONFIG_DIR` overrides the folder in debug builds). `#[serde(default)]` on the structs: a missing field takes its value from `Default`. Engine, recording mode, post-processing provider and preset are enums saved as string ids (`string_enum!`); an unknown id reads as the default instead of failing the file. Key fields:
- `engine`: `"local"` | `"groq"` | `"assemblyai"` | `"polza"` (default: `"groq"`; `Engine::ALL` keeps this order, the tray's and the Engine tabs'). The OpenAI-compatible engine was removed: an old `"openai"` reads as the default, its `openaiEndpoint`, `openaiModel` and `apiKeys.openai` are dropped on the next save
- `apiKeys`: `{ groq, polza, assemblyai, gemini, mistral, exa }`, one key per service for everything (engine, post-processing, quick ask, web search); `ApiKeys::engine` / `ApiKeys::model` pick the one needed. The UI edits them in Settings → API keys (`apikeys.ts`). `Settings::from_json` moves the keys of an older config (`groqApiKey`, `polzaApiKey`, `assemblyaiApiKey`, `postProcess.<p>.apiKey`, `quickAsk.<p>.apiKey`, `quickAsk.exaApiKey`) there, the first one set winning (`migrate_keys`)
- `groqModel`: `"whisper-large-v3-turbo"` (default) | `"whisper-large-v3"`
- `polzaModel`, `polzaProvider`
- `assemblyaiModel`: `"universal-3-5-pro"` (default, no Russian) | `"universal-2"`
- `recognitionLanguage`: ISO 639-1 code of the speech (`RECOGNITION_LANGUAGES`), empty detects it. Sent as `language` (Groq, Polza), `language_code` (AssemblyAI) and the run params' `language` (local; a language the model doesn't list falls back to detection)
- `fallbackLocal`: a failed cloud transcription goes to a downloaded local model (default `true`)
- `localModel`: model id from `local/catalog.rs` used for dictation (default `"parakeet-tdt-0.6b-v3"`)
- `localBackend`: `"vulkan"` | `"cpu"`, or empty for the one recommended for this computer
- `localUnload`: when an idle model leaves memory: `"immediate"` | `"30s"` | `"5m"` (default) | `"10m"` | `"never"`
- `language`: `"en"` | `"ru"` | `""` (empty follows the system, see `i18n.rs`)
- `developerMode`: shows the Developer tab in Settings and turns log collection on
- `microphone`: device name or `"default"`
- `recordingMode`: `"toggle"` | `"push-to-talk"` (default, so a new config and the setup start with it)
- `hotkey`: default `"Ctrl+Shift+Space"`; or a single mouse button: `"MouseMiddle"` | `"MouseBack"` | `"MouseForward"`
- `askHotkey`: opens the quick ask window empty and closes it (while quick ask is on), default `"Ctrl+Alt"`; two or three modifiers alone, keys with a non-modifier, or a mouse button. Must differ from `hotkey`. An older config's `chatHotkey` is moved here (`migrate_quick_chat`)
- `appendSpace`: adds a space after the pasted text (default `false`, switch in Post-processing; works with post-processing off too)
- `showIndicator`: whether the recording indicator may appear at all (default `true`, switch in Basics)
- `indicatorPosition`: `"top-right"` (default) | `"bottom-center"` (just above the taskbar); Basics, shown while the indicator is on
- `checkUpdates`: looks for a newer GitHub release 15 s after start and every 12 h (default `true`)
- `setupDone`: the first-run wizard was finished or skipped; `false` only for a new config (a config file without the field counts as set up)
- `postProcess`: `enabled` (default `false`), `provider` (`"gemini"` | `"mistral"` | `"groq"` | `"polza"`; OpenRouter was removed, an old `"openrouter"` reads as the default and its key and models are dropped on the next save), `preset` (`"chill"` | `"proper"` | `"custom"`; the old `"official"` is migrated to `"proper"` on load), `customPrompt`, `translate` (default `false`; works with `enabled` off too, same provider and model) and `translateTo` (`TranslationLanguage`: `"en"` default, `"ru"`, `"uk"`, `"es"`, `"fr"`, `"de"`, `"zh"`, `"ja"`, `"hi"`, `"he"`, `"fa"`), and `{ model }` per provider, plus `providerId` for Polza (sub-provider sent as `provider.only`, omitted from the file when empty)
- `quickAsk`: `enabled` (default `false`; an older config's quick chat that was on, `quickAsk.chat`, turns it on), `provider` (same four as post-processing) and `{ model }` per provider, plus `providerId` for Polza. Local models aren't offered. `webSearch` (default `false`): the model may search the web with Exa (`apiKeys.exa`)

### `transcribe_groq.rs`
Sends a multipart form POST with the recording (`audio::EncodedAudio`, FLAC) to Groq's Whisper-style `/audio/transcriptions` (`net::post_transcription`).

### `net.rs` — one HTTP client
`net::client()` returns one `reqwest::Client` for the whole app, so connections stay open between dictations. `recorder::warm_up_connections` sends a HEAD request (no credentials) to the engine's, post-processing's (also with only translation on) and quick ask's servers (and Exa's with web search on) when a dictation starts, so the TLS handshake happens while the user speaks.

### `transcribe_assemblyai.rs`
Three steps: `POST /v2/upload` with the raw FLAC, `POST /v2/transcript` with `speech_models: [model]` and `language_code` (or `language_detection: true` when no language is set), then poll `GET /v2/transcript/{id}` until `completed` or `error`. Universal-3.5 Pro supports 18 languages without Russian; if it rejects the language, the error suggests Universal-2.

### `local/` — local recognition (transcribe.cpp)
- Nothing native is compiled with Typr. `ffi.rs` loads `transcribe.dll` with LoadLibrary from the **runtime** downloaded on demand, mirrors the v0.2.4 C header (`include/transcribe.h` at the tag) and refuses a DLL whose version or struct sizes (`transcribe_abi_struct_size`) differ. Bump `TRANSCRIBE_VERSION`/`HEADER_HASH` in `catalog.rs`, the structs in `ffi.rs` and the pinned bundle together.
- The runtime (`catalog.rs` `RUNTIME`) is the official `windows-x86_64-cpu-vulkan` bundle: Vulkan for any GPU with a Vulkan driver, CPU modules for every processor. Its `contract.json` must match `TRANSCRIBE_VERSION` and `HEADER_HASH`. CUDA and ROCm are left out on purpose: their kernels are compiled per GPU generation, so each needs its own builds and checks (the official CUDA bundle covers compute capability 7.5–9.0 only; ROCm has no official Windows build). Measured on an RX 7600 (Parakeet, 11 s of audio, warm): Vulkan 0.12 s, CPU 0.8 s.
- `hardware.rs`: GPUs from the display adapter class in the registry (PCI devices) and `vulkan-1.dll` in System32. A GPU with Vulkan → Vulkan recommended, otherwise the CPU (Vulkan greyed out).
- transcribe.cpp never loads into Typr itself: it can't be unloaded from a process (backend modules and the Vulkan driver stay, about 40 MB). `worker.rs` starts Typr's own executable with `--local-recognition` (checked first thing in `main()`, before the logger and Tauri) as a child process that loads the runtime and one model and transcribes. It talks over stdin/stdout in frames (u32 length + JSON; the samples as one frame of f32), forwards its log records into Typr's log and its stderr as warnings. Closing its stdin makes it free the model and exit (killed after 3 s). So an unloaded model leaves Typr at its idle few megabytes, and a GPU driver crash ends only the child: Typr then runs that dictation on the CPU and keeps the CPU (reason under Acceleration) until the Acceleration choice changes. Starting the child costs about 0.1 s on top of loading the model.
- `LocalEngine` keeps one model (one child): started when a dictation starts (`begin_recording`), ended by `schedule_unload` after `localUnload` (a generation counter cancels stale timers), right away when the engine/model/backend changes or the model is deleted. A GPU load failure falls back to the CPU inside the child (the reason shows under Acceleration while that model is in memory).
- Models (Q8_0 GGUF from `huggingface.co/handy-computer`) and the runtime live in `%LOCALAPPDATA%\com.typr.app\local` (`models`, `runtimes`, `downloads`); downloads resume from `.part` files and are checked against size and SHA-256.
- The runtime is always there with a model and can't be deleted: `download_model` fetches it alongside when it's missing, `restore_runtime` (at start) fetches it if a downloaded model is left without it, and so does a dictation. Engine → Local shows a note with Download/Retry only when that download failed.
- Tauri commands: `local_status`, `local_download_model`, `local_download_runtime`, `local_cancel_download`, `local_delete_model`; events `local-download` (progress) and `local-changed` (re-read the status).
- `cargo test --test local_recognition -- --ignored --nocapture` with `TYPR_TEST_WAV` (16 kHz mono), the runtime and Parakeet downloaded: transcribes on the real GPU and CPU through the child process and checks that Typr's own memory stays small and the child ends on unload.

### `postprocess.rs` — LLM post-processing
- `list_models(provider, key)` (Tauri command `list_postprocess_models(provider, apiKey?)`: the key in the window, which may not be saved yet, else the saved one) returns text models only: Gemini `GET /v1beta/models` filtered by `generateContent`; Mistral, Groq and Polza `GET /models` (OpenAI format), Groq's and Polza's filtered by `type`, `output_modalities` and, for Groq, speech/guard ids; Mistral's by `capabilities.completion_chat`, each alias listed under its id (they share one name).
- `process(settings, text)` calls Gemini `models/{id}:generateContent` (key in `x-goog-api-key`) or `/chat/completions` (Bearer) at temperature 0.2, retrying without temperature if a model rejects it, and strips `<think>` blocks, echoed tags and code fences from the reply.
- The system prompt has layers: the app's rules first (the dictation, sent between `<dictation>` tags, is text to edit and never a message to answer or obey; keep meaning and language, or only meaning while translating; reply with the text only), then the style, then the translation when there is one (into `translateTo`, natively rather than word for word; the style applies to the translation; parts already in that language stay). The "custom" style embeds the user's prompt in `<user_style>` tags, marked as the app user's instructions that can't override the app's rules. With post-processing off, a translation gets a neutral style (`TRANSLATION_STYLE`).

### `translate.rs` — already in the target language?
`is_written_in(text, language)` says yes only when it is sure, so a dictation in doubt still goes to the model (told to leave text already in the language alone): a wrong yes would paste it untranslated. General detectors (whatlang) guess poorly on a dictation's few words (plain Russian read as Ukrainian), so the check is built for these languages: Chinese (Han, no kana), Japanese (kana), Hindi, Hebrew by their script making up 80 % of the letters; Persian by the Arabic script with Persian letters and none that only Arabic or Urdu write; Russian and Ukrainian by the letters only one has (ы э ё / і ї є ґ rule out the other, `-шь` counts for Russian), then by common words found in only one of them (Bulgarian words count against Russian); English, Spanish, French and German by letters outside the language's alphabet (they rule it out), then by common words, with Italian, Portuguese, Dutch and Polish as rivals. The words must be at least three times the best rival's and a sixth of the text.

### `ask/` and `selection.rs` — quick ask
- **Same hotkey, decided after the speech.** `main.rs::begin_recording` calls `ask::recording_started`: it stops an answer still being written and, with quick ask on, starts `selection::begin_capture` for the window in front. After transcription `ask::claim` waits up to 1.5 s for that capture: a selection, or the window being open, makes the recording a question; otherwise it is a dictation as before. Post-processing doesn't run on questions.
- **Reading the selection** (`selection.rs`, Windows): a synthetic copy, sent with `SendInput` only once Ctrl, Shift, Alt and Win are all up (right after a toggle press, at the end of push-to-talk), so it never mixes with the user's keys; the window must still be in front. Only a clipboard whose change counter (`GetClipboardSequenceNumber`) moved within 300 ms is read, so an old clipboard (maybe a password) never reaches a model; then the previous content is put back (`paste::save` / `paste::restore`). Terminals (window class or executable) get no keystroke at all: Ctrl+C would interrupt the running program. Editors with a built-in terminal (VS Code and forks, JetBrains, Visual Studio) get Ctrl+Insert, which copies there but is no interrupt. A copy that is the current line taken without a selection doesn't count: VS Code's `isFromEmptySelection` (in Chromium's custom clipboard data), Scintilla's `MSDEVLineSelect`, Visual Studio's line tag, a lone line from JetBrains or Sublime; neither does a single Excel cell (there is always an active one). Selections are cut at 10 000 characters; a generation counter keeps a capture from reaching a later recording.
- **Questions shown, the window opened empty** (`ask::toggle`): every question is shown (`Command::Question`: its text and the start of the selection), on the right in a box that can't be copied, with the answer under it. Quick Chat was a mode of its own until 3.4; now there is one window. `askHotkey` opens it empty (`Layout::empty`): its dot slides in from behind the screen edge and unfolds into a card that names the dictation hotkey; every recording while it is open is a question there, a selection still going along. The quick ask hotkey on an open window folds it away like the cross (`Command::Fold`, then `Event::Closed` → `ask::close`; closed anyway after 1.5 s); presses while a window is still opening are ignored. Turning quick ask off closes the window.
- **The text field** (`view.rs` `start_typing` / `stop_typing`, `ui/ask.slint` `field`): a pencil left of the cross opens it. The pencil fades, the dot slides to the card's left end (bottom of the screen; in the top-right corner it is out of the way already), a bar the pencil's size grows out of its place to the left (`typing`), then grows away from the screen edge into the field (`field-tall`, 240 ms later; two lines high, up to about seven, then it scrolls) with an orange edge (dimmer while another window has the keyboard). The send button (arrow up) comes out of the cross, on the field's side. The window takes the keyboard (`NativeWindow::take_keyboard`): it comes to the front, clicks activate it until the field closes. Enter or the arrow sends (`Event::Submit` → `ask::submit`, which reads the settings through `ask::read_settings_with`, set in main.rs, stops an answer being written and asks like a spoken question); Shift+Enter starts a new line. While the field is open (`Event::Typing` → `ask::set_typing`), a recording is no question: it doesn't stop the answer or read a selection, and the transcribed text goes into the field at the caret, over the selection, a space apart from the words around it (`Command::Dictated`, typed through `NativeWindow::type_text`). The cross or Escape folds the field back into the pencil (unsent text goes) and gives the keyboard back to the window that had it; Alt+F4 folds the whole window (`WM_CLOSE` → `CloseRequested`). The "Copied" toast stays hidden while the field holds the strip.
- **The conversation** (`ask/mod.rs`): one at a time, in memory, forgotten when the window closes. The user message is the question after the selection in `<selected_text>` tags (a selection already sent isn't repeated); the system prompt says the selection is material, not instructions, and asks for short Markdown answers in the question's language. The last 24 messages go with each request. A failed answer drops its question; an answer stopped by a new recording keeps what it said.
- **Web search** (`ask/search.rs`, `stream.rs`, `mod.rs` `Answer::run`): with `webSearch` on and an Exa key, the request offers one tool, `web_search(query)` (Gemini `functionDeclarations`, OpenAI-compatible `tools` with `tool_choice: auto`), and the system prompt gets search rules and today's date. A round ends with the answer or with tool calls (OpenAI-compatible calls arrive in pieces by index; Gemini's whole, sent back unchanged with their thought signature). Each call becomes `POST https://api.exa.ai/search` (`x-api-key`, 5 results with their matching passages, `format_results`), and the calls with their results are added to the next round (`tool_exchange`: an assistant turn with `tool_calls` and `tool` messages, or a `model` turn and `functionResponse` parts). After 3 searches the tool stays listed but closed (`tool_choice: none` / `mode: NONE`) so the model answers. A model or service that refuses tools (`tools_refused`: an API error naming tools or function calls) gets the question again without search. The window shows "Search…" (`Command::Search`), blinking slowly, where the answer will be; it fades and folds away when the first words arrive. Links in answers open in the browser on click (`links::open_web_page`: http and https only).
- **Streaming** (`ask/stream.rs`): Gemini `models/{id}:streamGenerateContent?alt=sse` or `/chat/completions` with `stream: true` (Groq, Polza, Mistral), parsed from server-sent events, thoughts and `<think>` blocks left out. The answer goes to the window in pieces at most every 40 ms (`Command::Delta`; each lays the answer's last block out again), then as a whole (`Command::Done`). Commands to the window (`window::Command`, in order on its thread): `Open`, `Reveal`, `Question`, `Answer`, `Search`, `Delta`, `Done`, `Error`, `Cancel`, `Recording` and `RecordingError` (forwarded by `recorder.rs`), `Dictated` (into the text field), `Field` (the demo opens and closes the field), `Fold`, `Destroy`. Events from it (`window::Event`, on its thread): `Shown` (first frame on screen), `Closed` (folded away), `Typing` (the field opened or closed) and `Submit` (a question typed there). Copying and links are handled in the window itself.
- **The window** (`ask/window.rs`, `ask/native/`, `ui/ask.slint`): drawn by Typr with [Slint](https://slint.dev/) 1.18.1, no WebView. Opened for a first question or empty by the quick ask hotkey (`Command::Open` with a `Layout`), destroyed by its cross, that hotkey, or when quick ask is turned off. A frame touching the screen edge the indicator slides in from, sized for the tallest card (440 px wide, up to 560 px or 70 % of the screen high, `Geometry`) so it never moves or resizes. Its first frame draws the indicator's dot exactly where the native one is, with its spinner; `Reveal` puts it on screen and waits for the desktop to compose it (`DwmFlush`), then `Event::Shown` lets the native dot go (`overlay::set_suppressed`; `window::reveal` waits for it up to 1.5 s). Then the dot unfolds in two steps: it stretches into a bar along the screen edge, and the bar grows into the card; the dot stays as the card's status light (spinner while thinking, orange pulse while the user asks a follow-up). A click on an answer copies it and "Copied" shows in the status strip. A code block has a copy button in its top right corner: it copies that code alone and turns into a check mark for a moment. The card has no scrollbars (the wheel scrolls; long code lines wrap). The cross folds the card back into the dot, which slides behind the edge. Measured (`typr --ask-bench`, release): first frame about 16 ms (31 ms the first time, when the thread and the fonts start), +1.8 MB private memory with the card open, about +1 MB once closed (the thread and its fonts stay), 6 % of a core while an answer streams, nothing drawn while still.
- **How it is drawn** (`native/platform.rs`): a layered popup like the indicator's (`WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TOOLWINDOW`, `MA_NOACTIVATE` on clicks), so the app the question is about keeps the focus and its selection. Only the text field takes the keyboard: `take_keyboard` drops `WS_EX_NOACTIVATE` and brings the window to the front (`SetForegroundWindow` on the click; `AttachThreadInput` if Windows refuses), `MA_ACTIVATE` on clicks until `release_keyboard` puts the style back and returns the foreground to the window that had it. Keys go to Slint as Windows gives them: `WM_KEYDOWN`/`WM_KEYUP` for keys that type nothing (arrows, Backspace, Enter, modifiers) and for Ctrl with a letter or digit (by virtual key, so shortcuts work in any layout), `WM_CHAR` for text (surrogate pairs joined, control characters dropped); Alt alone never opens a system menu. `WM_ACTIVATE` reaches Slint from the message loop, not from inside `SetForegroundWindow` (which may be in one of Slint's callbacks); on deactivation the modifiers Slint thinks are held are let go. Ctrl+C/X/V in the field use the system clipboard (`Platform::set_clipboard_text` / `clipboard_text`, arboard). IME input works through Windows' default composition window. A Slint `Platform` of our own: the software renderer draws only what changed straight into the window's DIB (`Bgra`, premultiplied, as `UpdateLayeredWindowIndirect` takes it, with the dirty rectangle). One thread (`typr-ask-window`, started with the first window, then asleep) runs its message loop, Slint's timers and animations (60 frames a second while something moves, a high-resolution waitable timer) and the commands (`mpsc` + `PostThreadMessage`). A 30 ms timer checks the cursor against the card and sets `WS_EX_TRANSPARENT` elsewhere, so clicks pass through. The software renderer has no drop shadows and no rounded clipping: the card has no shadow, the dot's is an image (`shadow.rs`, so it looks like the indicator it takes over from), and the content keeps clear of the corners. OpenGL (femtovg) was measured out: a context alone costs 80–230 ms and about 136 MB on an RX 7600.
- **Slint gotchas** (`ui/ask.slint`, `view.rs`): box layouts measure a wrapped text's height before giving it its width, so wrapping text gets an explicit width from the text column (402 px) and the height from its `preferred-height`. Where a border and a background meet at a rounded corner, the software renderer leaves light pixels, so rounded boxes with an edge are two fills (`Box`: the edge colour, then the fill 1 px in), their colours mixed with the card's beforehand (`Theme`). Sizes round the dot are even, so the listening ring stays centred on the indicator's whole-pixel centre. While the card grows after its text, nothing slides: only the card's size is animated and its place is worked out from it (`anchor`: the edges at the dot stay put; animated apart, place and size drifted and a card at the bottom slid down), and the scroll area already has the card's coming height at its edge away from the strip (the card hides what it hasn't grown over), so `follow()` scrolls only when that height or the text changes. Once unfolded, the height follows the text on a critically damped spring (`shown-height`, a 16 ms `Timer`, ω = 22/s; `follow-from` starts it where the unfolding left off): an eased animation restarted with every streamed line went in steps. Anything bound to `animation-tick()` is redrawn every frame, so the listening ring steps from a `Timer` (20 a second), and the card's height is a plain property set by `fit()` only when it changes (bound to the text's height, every streamed piece redrew the card and its shadow). `StyledText` takes every click for its links, so the answer only reports hover (`hovered-answer`) and `view.rs` decides on button up (`set_button_hook`): no link or button took the click → copy the answer. Two internal hooks need the exact version (`=1.18.1` in Cargo.toml): the mouse cursor (`WindowAdapterInternal::set_mouse_cursor`) and `StyledText` built straight from our paragraphs (`markdown::styled`, a `repr(transparent)` transmute checked by a test against `StyledText::from_markdown`); recheck both when updating Slint. Slint's royalty-free license asks for attribution: its logo is at the very bottom of both READMEs, centred below the license, and under Quick Ask's steps with "works on your machine" beside it, on the logo's bottom line (always in English).
- **Markdown in answers** (`native/markdown.rs`, the old page's rules): code blocks and paragraphs indented under a list item belong to it (`Block::nest`, drawn under the item's text); a closing fence with a language (```` ```bash ````, a model's slip) still closes the block.

### `paste.rs` — clean clipboard
Saves the clipboard (text, else image, else nothing), sets the dictated text with arboard's `exclude_from_monitoring()` (kept out of Win+V history, cloud sync and clipboard managers), sends Ctrl+V, waits `RESTORE_DELAY` (300 ms) and puts the previous content back, unless something else was copied meanwhile. If the paste fails, the text stays on the clipboard. Other formats (e.g. copied files) aren't restored.

### Sidebar and the Settings window
The sidebar has the logo with the version beside it (`#version-text`), the status, the sections of the main page (Basics, Engine, Post-processing, Quick Ask; `showSection` in `main.ts`) and, at the very bottom, the Settings button (`#nav-settings`, with the update link above it). Basics (`section-general`) holds the microphone, the recording indicator and its position, the interface language and update checks.

Settings (`#settings` in `index.html`, `openSettings` / `closeSettings` / `showSettingsTab` in `main.ts`) is a window inside the app, not a Tauri window: a dimmed backdrop over the whole page and a fixed-size card (600 × 480 at most) with its tabs along the top as a segmented bar, like Engine's (`.segmented.full`): General (start with Windows, the setup wizard, developer mode), API keys, Hotkeys and, while developer mode is on, Developer (the log, filling the card's height). It sits above the page (`z-index` 40) and under the setup (50) and the window buttons (60). While it is open `#app-body` is `inert`; the cross, Escape (hotkeys.ts takes it first while a hotkey is being recorded) and a click on the backdrop close it, and the focus goes back to the Settings button. The arrow keys move between the tabs. Opening the setup closes Settings first. Errors logged while the Developer tab isn't on screen are counted on the Settings button and on that tab (`.dev-badge`). The hotkey named in Quick Ask's hint opens Settings on Hotkeys.

### `segmented.ts`
`enhanceSegmented(control)` gives a `.segmented` its thumb: one `.segmented-thumb` element under the segments (the active segment itself stays clear) that follows the active one on a slightly underdamped spring (`requestAnimationFrame`, a few pixels of overshoot; the width settles without overshoot), and can be grabbed (a press on the active segment) and dragged: it follows the pointer, gives a little past the end segments, blends its width between segments, and on release lands on the enabled segment under its middle (keeping a little of the release speed, so it swings a bit), which gets a `click()`, so the modules keep handling plain clicks and toggling the `active` class as before. A `MutationObserver` (the `active` class) and a `ResizeObserver` (shown, resized, relabelled) move it; a hidden control or one shown again gets it placed without animation, and so does `prefers-reduced-motion`. `main.ts` enhances every `.segmented` in the page at start; the setup enhances the one it builds. Sections and Settings tabs switch without a fade.

### `dropdown.ts`
`enhanceSelect(select)` hides a native `<select>` and draws an app-styled button and list over it; the `<select>` stays the source of truth (`.value`, `change` events), option rebuilds are picked up by a `MutationObserver`, and after setting `.value` from code call `syncSelect(select)`. New `<select>`s should be enhanced too.

### Main window frame
The main window has no system title bar (`decorations: false` in tauri.conf.json) and can't be maximized (`maximizable: false`: no button, no double-click or snap to the top). Typr draws its own minimize and close buttons (`.window-controls` in `index.html`), fixed in the top-right corner above everything, the setup too. Elements with the `drag-region` class (the sidebar, the strip above the content, the Settings window's tab row, the setup's top strip and header) drag the window from their empty space through `startDragging` in `main.ts`; buttons, inputs and labels inside them don't. Close works like the system button did: the window and its WebView go, Typr stays in the tray. Tauri keeps the edges resizable; `tint_window_border` (main.rs) paints the 1 px border Windows 11 still draws in the app's grey instead of the accent colour.

### `mouse.rs` — a mouse button as a hotkey
`register_hotkeys` (main.rs) registers the dictation hotkey and, with quick ask on, its own one. Key combinations go to the global shortcut plugin, a mouse button to `mouse::start(slot, …)`: one `WH_MOUSE_LL` hook on its own thread with a message loop, with a slot each for dictation and quick ask. The hook proc only matches the bound buttons (ignoring injected clicks), swallows them (so "back" doesn't navigate the browser) and hands press/release to the slot's handler thread; dictation ends in `on_hotkey`, quick ask in `on_ask_hotkey`. The hook exists only while a mouse hotkey is active (`unregister_hotkeys` removes it, e.g. when the hotkeys are turned off in the tray). The left and right buttons can't be a hotkey.

### `modifier_hotkey.rs` — modifiers alone as the quick ask hotkey
The plugin needs a key besides the modifiers, so a quick ask hotkey like `Ctrl+Alt` gets a `WH_KEYBOARD_LL` hook (only while it is set). It fires when the modifiers, held together with nothing else, are let go (`Chord`): any other key or modifier pressed meanwhile cancels it (Ctrl+Alt+Delete, shortcuts, typing), injected keys and the Ctrl that AltGr adds don't count, and a release the hook missed is forgotten (`GetAsyncKeyState`). Nothing is swallowed.

While the window records a new hotkey (`hotkeys.ts`), `pause_hotkeys` takes Typr's own off: Windows would swallow a registered combination before the page saw it, leaving only its modifiers. They come back when the recording ends, or when the main window is destroyed mid-recording.

### `keyboard.rs` — hotkey pressed on its own
On `ShortcutState::Pressed` with the recorder `Ready`, `HotkeyKeys::extra_keys_held()` reads every key and mouse button via `GetAsyncKeyState`; if anything besides the hotkey's own keys is down, the press is ignored (logged). Stopping a recording is never blocked.

### Icons
Every icon is [Lucide](https://lucide.dev/icons/) (v1.48.0 of `lucide-static`), its SVG file copied unchanged into `src/icons/` (license in `src/icons/LICENSE`), so nothing is loaded from the internet. Markup puts `<i data-icon="name" data-size="14">` where an icon goes (the file name; 16 px by default) and `renderIcons()` from `src/icons.ts` swaps in the SVG with the placeholder's classes (main.ts calls it at start); code calls `icon(name, size)`. CSS uses the file as a mask (`mask: url("./icons/check.svg")` with `background` in the colour; Vite inlines it). `overlay.html` has no scripts, so its `mic` is inlined there, and the native indicator draws the same `mic` in `overlay.rs`; the quick ask window strokes Lucide's paths with Slint (`Icon` in `ui/ask.slint`). The Slint logo under Quick Ask's steps is `assets/slint-logo.png` as a mask in the hints' grey (`.slint-logo` in `.slint-credit`). A new icon: copy its file from the same Lucide version into `src/icons/`. The flags of the translation languages are [Twemoji](https://github.com/jdecked/twemoji) v16.0.1, their files unchanged in `src/flags/` (CC-BY 4.0, license and attribution there), inlined by `postprocess.ts` the same way (`?raw`). The app logo (`assets/logo.svg`, `src-tauri/icons`) is Typr's own.

### Localization
`src/i18n.ts` holds the English and Russian strings. Static markup uses `data-i18n` (text), `data-i18n-placeholder` and `data-i18n-title`; code uses `t(key)`. New UI text must get keys in both dictionaries. Quick Ask is a feature name: written so in both languages, never translated. The `language` setting picks the language; when empty, `get_system_language` (Rust, `sys-locale`) decides: Russian for ru, uk, be, kk, ky, tg, uz, tk, hy, az, English otherwise. The tray menu is translated in `i18n.rs`, the quick ask window in `ask/native/view.rs` (`strings`). Backend error messages and logs stay in English. The page is hidden (`.i18n-pending`) until the language is applied.

### `cleanup.rs`
Collapses whitespace, capitalizes the first letter of every sentence and adds a final period. A sentence ends at `.`, `!` or `?` followed by a space, not inside numbers (3.14), addresses (example.com) or abbreviations (т.е., e.g., см.), and not after an ellipsis. Words that already have a capital (iPhone) are left alone; a trailing comma becomes the period.

### Tray menu, autostart, updates
- Tray: Open settings, Paste last dictation (the last text goes back into the window it was pasted into; if that window is gone, it is put on the clipboard), Engine submenu (switches `engine`, the window follows through `engine-changed`), Hotkey On/Off, Exit, and a "Version X is available" item added on top once an update is found.
- The main window is `"create": false` in tauri.conf.json and built in `setup`; started with `--autostart` (the Run value from `autostart.rs`), Typr stays in the tray unless the setup isn't done. `autostart::refresh` repoints the Run value at the current executable (release builds only).
- `updates::check` reads `releases/latest` of `neodym121/typr`; the result is kept in `AppState.update`, sent as `update-available` and shown in the sidebar and Basics. `links::open` opens only the release page and the providers' key pages.

### `setup.ts` — first-run wizard
Shown while `setupDone` is false (and from Settings → General → Advanced): interface language, engine (Local recommended when Vulkan is available, else Groq), the engine's model download or API key (saved in `apiKeys`) plus the speech language, then hotkey, recording mode (push to talk for a new config), start with Windows and update checks. A click on the hotkey records new keys right there (`bindDictationHotkey` in hotkeys.ts, the same recorder as Settings → Hotkeys). Finish or Skip sets `setupDone`.

### `logger.rs` — Settings → Developer
Installed as the global `log` backend in `main()`. Use `log::info!` / `log::warn!` / `log::error!` / `log::debug!` everywhere in Rust (not `println!`). While `developerMode` is on, entries go to a 5000-entry ring buffer and are streamed to the main window as `log-entry` events; the frontend forwards its own console output via the `frontend_log` command. Never log API keys — use `Settings::summary()` / `describe_changes()`, which only say which are set.

`stop_and_transcribe` must always return the recorder to `Ready`, even on errors; failures are reported to the UI with `recorder::notify_error` (`recording-error` event).

### Memory: no WebView while idle
- `overlay.rs` draws the recording indicator itself on Windows (`windows-sys` layered window, per-pixel alpha, SDF rendering; `mic_sdf` is Lucide's `mic` outline) — no WebView. Other platforms fall back to the WebView overlay `src/overlay.html`.
- The indicator is out of sight while idle: its window touches an edge of the primary screen's work area (the screen without the taskbar) and the disc is drawn beyond that edge, so it slides in from behind it when a dictation starts (or briefly after an error) and slides back out on `Ready`. `indicatorPosition` picks the edge: the top-right corner (window at the top edge, disc above it) or the bottom centre (window ending at the taskbar, disc below it). Once fully out, the window is hidden and its timers stop, so it costs no CPU.
- Each time the indicator starts to appear, it is put above all windows again (`SetWindowPos` with `HWND_TOPMOST`, a warning in the log if it had lost always on top: another program can take that away), and `place()` in the native overlay reads the work area and scale again (`MonitorFromPoint`, `GetMonitorInfoW`, `GetDpiForMonitor`, as Tauri does), so a changed resolution, scale or taskbar is picked up without a restart. `overlay::set_position` (from `apply_settings`) moves it at once.
- `overlay::set_enabled` gates it: `main.rs::sync_overlay` allows it only while `showIndicator` is on and the hotkey is on in the tray. `overlay::set_suppressed` hides it at once while the quick ask window holds its place.
- The quick ask window is no WebView either: Typr draws it with Slint on a layered window (`ask/native/`), and it exists only while an answer is on screen.
- Closing the main window destroys it (and its WebView); `show_main_window` re-creates it from `tauri.conf.json` on a separate thread (building windows in event handlers deadlocks on Windows).
- The page runs under a strict CSP (`tauri.conf.json`): scripts, styles and fonts from the app only (the fonts are in `src/fonts`, so no page waits for the network), IPC. New external resources need a CSP entry. `RunEvent::ExitRequested` without a code is prevented, so Typr keeps running in the tray; tray → Exit calls `app.exit(0)`.
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
- The page runs only inside the Tauri window: in a plain browser (`npm run dev`, port 1420) `getCurrentWindow()` throws at load and the page stays hidden, so UI changes can't be previewed there. The quick ask window can be tried on its own: `typr --ask-demo [empty] [bottom] [ru] [long|code|type]` plays the dot unfolding, questions, answers streaming and follow-ups (`empty`: opened by the quick ask hotkey; `empty type`: the text field opening, dictation into it, closing; a question typed into the field gets a sample answer) (with `TYPR_ASK_SNAPSHOTS=<folder>` it saves frames as PNG, over a dark desktop with `TYPR_ASK_DARK`), and `typr --ask-bench` prints its memory, first-frame time and CPU (use a release build).
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

The run is named `typr <tag>` (e.g. `typr v3.3.1`). It produces one artifact, `typr-<version>` (a zip), with two files at its root: the MSI installer and the portable executable (`Typr_<version>_x64_portable.exe`, the built `typr.exe` itself). Only the MSI bundle is built (`--bundles msi`).

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
- **Adding a new transcription engine**: create a new `transcribe_<name>.rs` module, add it to `lib.rs` and add an `Engine` variant in `settings.rs`; the compiler then points at the matches to extend (`Engine::label`, `ApiKeys::engine`, and `transcribe`, `upload_format`, `warm_up_connections` in `recorder.rs`). The tray lists `Engine::ALL` by itself. Its key goes into `ApiKeys` (field, `all()`) and `SERVICES` in `apikeys.ts`. In the UI: its segment (`data-engine`) and its settings panel (`data-engine-panel`) in `index.html`, which `setEngine` in `main.ts` picks up, `EngineId`/`ENGINES` in `setup.ts`, strings in `i18n.ts`.
- **`main.rs` is the hub**: `AppState`, every `#[tauri::command]` (listed in `generate_handler!`), the tray, hotkey registration and `apply_settings`. Every settings change, from the window or the tray, goes through `apply_settings`: it re-registers the hotkey, saves `config.json`, logs the masked diff, tells `LocalEngine`, and updates the overlay and the tray. A setting that must take effect at once is handled there.
- **One settings object in the UI**: `currentSettings` in `main.ts`, typed by a TS `Settings` interface that mirrors the Rust struct's camelCase JSON. `local.ts`, `postprocess.ts`, `quickask.ts`, `apikeys.ts`, `hotkeys.ts` and `setup.ts` reach it through a host object (`settings()`, `save()`) and declare only the fields they use. Every control change sends the whole object to `save_settings`; there is no Save button.
- **Settings changes**: in `settings.rs` the field (`#[serde(rename = "camelCase")]`), `Default`, `summary()`, `describe_changes()` (a new API key goes into `ApiKeys`, whose changes are logged masked) and the tests; `apply_settings` if it must take effect at once; in the UI the TS interface, the control in `index.html`, `bind(...)` and `fillForm()` in `main.ts`, and both dictionaries in `i18n.ts`.
- **Docs in step**: a feature change also updates this file, the feature lists in `README.md` and `README.ru.md` (the same text in Russian) and the strings in both languages. `docs/polza-ai-reference.md` has Polza.AI API notes (in Russian).
- **Never hardcode API keys.** They are stored in `config.json` in the user's app data directory, not in the source tree.
- **Do not commit** `node_modules/`, `dist/`, or `src-tauri/target/`. These are covered by `.gitignore`.
- **Prefer async Tauri commands** for any operation that touches the network or filesystem from Rust.
- After any Rust change, verify with: `cd src-tauri && cargo test`

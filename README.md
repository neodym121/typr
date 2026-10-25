<p align="center">
  <img src="assets/logo.svg" width="96" height="96" alt="Typr logo" />
</p>

<h1 align="center">Typr</h1>

<p align="center">
  <a href="https://github.com/neodym121/typr/releases/latest"><img src="https://img.shields.io/github/v/release/neodym121/typr?logo=github" alt="release" /></a>
  <a href="https://github.com/neodym121/typr/releases/latest"><img src="https://img.shields.io/badge/platform-windows-blue" alt="platform" /></a>
  <a href="https://tauri.app/"><img src="https://img.shields.io/badge/tauri-2-blue?logo=tauri" alt="tauri" /></a>
  <a href="https://github.com/neodym121/typr/stargazers"><img src="https://img.shields.io/github/stars/neodym121/typr" alt="stars" /></a>
  <a href="https://github.com/neodym121/typr/blob/main/LICENSE"><img src="https://img.shields.io/github/license/neodym121/typr" alt="license" /></a>
</p>

<p align="center">
  <a href="https://slint.dev/"><img src="assets/slint-logo.png" height="56" alt="Slint" /></a><br />
  This app uses <a href="https://slint.dev/">Slint</a> technology
</p>

<p align="center">
  <b>English</b> · <a href="README.ru.md">Русский</a>
</p>

Typr is a lightweight desktop dictation app for Windows. Press a global hotkey, speak, and the recognized text is pasted into whatever window is active. It can also answer: ask an AI about the text you selected, or chat with it by voice in a small window at the edge of the screen.

<p align="center">
  <img src="assets/screenshot.png" width="720" alt="Typr settings window" />
</p>

## Features

### Dictation & Text

- **Global hotkey** in two modes (toggle or push to talk): a key combination or a single mouse button (middle or side).
- **Local recognition** with [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp): Parakeet TDT 0.6B v3, Whisper Large v3, Large v3 Turbo and Medium run on your computer, the audio never leaves it. They run on the GPU through Vulkan (NVIDIA, AMD or Intel) or on the CPU. Models are downloaded and deleted in the settings, and an idle model leaves memory by itself.
- **Cloud speech-to-text providers:** Groq (Whisper Large v3/v3 Turbo), [AssemblyAI](https://www.assemblyai.com/) (Universal-3.5 Pro/Universal-2) and [Polza.AI](https://polza.ai/). Recordings are sent as FLAC where the service takes it, over a connection opened while you speak.
- **Post-processing:** an AI model (Gemini, Mistral, Groq or Polza) polishes the text before it is pasted, in different styles.
- **Translation** of the dictated text into many languages.

### AI assistant

- **Quick Ask:** select text in any app, press the dictation hotkey and ask about it out loud ("translate this", "what does this error mean"), or press its own hotkey (Ctrl+Alt by default) to ask about anything. The recording indicator unfolds into a small window with your question and the answer (from Gemini, Mistral, Groq or Polza), and the app you were in keeps the focus. Ask again for a follow-up, or click the pencil and type: while the text field is open, dictation goes into it, so you can edit the question before sending it. The window is drawn by Typr itself, without a browser engine: it is on screen in about 16 ms and takes about 2 MB of RAM.
- **Web search:** with an [Exa](https://exa.ai/) key, the model searches the web when a question needs fresh facts.

### App

- **API keys in one place:** each service's key is entered once, in Settings → API keys; the other sections only pick the service and the model.
- **Tiny footprint:** up to 10 MB of RAM! The settings window's WebView is released when it's closed, and the recording indicator and the Quick Ask window are drawn natively.
- **Developer mode:** a live log of everything the app does, for troubleshooting.
- **English and Russian interface**, dark and minimal, in the Anthropic style, with its own window buttons instead of the system title bar.

## Building

GitHub Actions (`.github/workflows/build.yml`) builds one zip with the `.msi` installer and the portable `.exe` when a `v*` tag is pushed or the workflow is started manually:

```bash
git tag v3.4.1
git push origin v3.4.1
```

Local development needs [Rust](https://rustup.rs/) and [Node.js](https://nodejs.org/) 20+:

```bash
npm install
npm run tauri dev
```

Built with [Tauri 2](https://tauri.app/) (Rust) and vanilla TypeScript; the Quick Ask window with [Slint](https://slint.dev/).

## Credits

- Original project by [albertshiney](https://github.com/albertshiney/typr)
- Fork maintained by [neodym121](https://github.com/neodym121), improved together with [Claude](https://claude.com/claude-code) (Anthropic)
- Quick Ask window: [Slint](https://slint.dev/) (Royalty-free license 2.0)
- Icons: [Lucide](https://lucide.dev/) (ISC); flags: [Twemoji](https://github.com/jdecked/twemoji) (CC-BY 4.0)
## License

[MIT](LICENSE)

<p align="center">
  <img src="assets/logo.svg" width="96" height="96" alt="Логотип Typr" />
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
  В этом приложении используются технологии <a href="https://slint.dev/">Slint</a>
</p>

<p align="center">
  <a href="README.md">English</a> · <b>Русский</b>
</p>

Typr — лёгкое приложение для голосового ввода на Windows. Нажимаешь глобальный хоткей, говоришь, и распознанный текст вставляется в активное окно. А ещё оно умеет отвечать: спросите ИИ о выделенном тексте или поговорите с ним голосом в маленьком окне у края экрана.

<p align="center">
  <img src="assets/screenshot.ru.png" width="720" alt="Окно настроек Typr" />
</p>

## Возможности

### Диктовка & Текст

- **Глобальный хоткей:** сочетание клавиш или одна кнопка мыши (средняя или боковая).
- **Локальное распознавание** через [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp): Parakeet TDT 0.6B v3, Whisper Large v3, Large v3 Turbo и Medium работают прямо на компьютере, звук никуда не уходит. Работают на видеокарте через Vulkan или на процессоре.
- **Облачные провайдеры распознавания:** Groq, [AssemblyAI](https://www.assemblyai.com/) и [Polza.AI](https://polza.ai/). Запись уходит в FLAC, где сервис его принимает, по соединению, открытому, пока вы говорите.
- **Постобработка:** нейросеть дорабатывает текст перед вставкой в разных стилях. Также перевод продиктованного текста на многие языки.

### ИИ-помощник

- **Quick Ask:** выделите текст в любом приложении, нажмите хоткей диктовки и спросите о нём вслух или нажмите собственный хоткей, чтобы спросить о чём угодно.
- **Поиск в интернете:** с ключом [Exa](https://exa.ai/) модель ищет в сети, когда вопросу нужны свежие факты.

### Приложение

- **Минимум памяти:** до 15 МБ ОЗУ! WebView окна настроек выгружается после закрытия, а индикатор записи и окно Quick Ask рисуются нативно.
- **Режим разработчика:** живой лог всего, что делает приложение, для поиска проблем.
- **Интерфейс на русском и английском**, тёмный и минималистичный.

## Сборка

GitHub Actions (`.github/workflows/build.yml`) собирает один zip с установщиком `.msi` и портативным `.exe` при пуше тега вида `v*` или при ручном запуске workflow:

```bash
git tag v3.4.1
git push origin v3.4.1
```

Для локальной разработки нужны [Rust](https://rustup.rs/) и [Node.js](https://nodejs.org/) 20+:

```bash
npm install
npm run tauri dev
```

Сделано на [Tauri 2](https://tauri.app/) (Rust) и чистом TypeScript; окно Quick Ask — на [Slint](https://slint.dev/).

## Авторы

- Оригинальный проект — [albertshiney](https://github.com/albertshiney/typr)
- Форк ведёт [neodym121](https://github.com/neodym121), доработан вместе с [Claude](https://claude.com/claude-code) (Anthropic)
- Окно Quick Ask: [Slint](https://slint.dev/) (лицензия Royalty-free 2.0)
- Иконки: [Lucide](https://lucide.dev/) (ISC); флаги: [Twemoji](https://github.com/jdecked/twemoji) (CC-BY 4.0)
## Лицензия

[MIT](LICENSE)

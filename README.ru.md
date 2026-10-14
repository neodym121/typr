<p align="center">
  <img src="assets/logo.svg" width="96" height="96" alt="Логотип Typr" />
</p>

<h1 align="center">Typr</h1>

<p align="center">
  <a href="https://github.com/neodym121/typr-updated/releases/latest"><img src="https://img.shields.io/github/v/release/neodym121/typr-updated?color=369eff&labelColor=black&style=flat-square&logo=github&logoColor=white" alt="release" /></a>
  <a href="https://github.com/neodym121/typr-updated/actions/workflows/build.yml"><img src="https://img.shields.io/github/actions/workflow/status/neodym121/typr-updated/build.yml?label=build&labelColor=black&style=flat-square&logo=githubactions&logoColor=white" alt="build" /></a>
  <a href="https://github.com/neodym121/typr-updated/releases"><img src="https://img.shields.io/github/downloads/neodym121/typr-updated/total?color=45cc11&labelColor=black&style=flat-square" alt="downloads" /></a>
  <a href="https://github.com/neodym121/typr-updated/releases/latest"><img src="https://img.shields.io/badge/platform-windows-0078d4?labelColor=black&style=flat-square" alt="platform" /></a>
  <a href="https://tauri.app/"><img src="https://img.shields.io/badge/tauri-2-ffc131?labelColor=black&style=flat-square&logo=tauri&logoColor=white" alt="tauri" /></a>
  <br />
  <a href="https://github.com/neodym121/typr-updated/releases"><img src="https://img.shields.io/github/release-date/neodym121/typr-updated?color=61d0ff&label=release%20date&labelColor=black&style=flat-square" alt="release date" /></a>
  <a href="https://github.com/neodym121/typr-updated/stargazers"><img src="https://img.shields.io/github/stars/neodym121/typr-updated?color=ffcb47&labelColor=black&style=flat-square" alt="stars" /></a>
  <a href="https://github.com/neodym121/typr-updated/forks"><img src="https://img.shields.io/github/forks/neodym121/typr-updated?color=8ae8ff&labelColor=black&style=flat-square" alt="forks" /></a>
  <a href="https://github.com/neodym121/typr-updated/issues"><img src="https://img.shields.io/github/issues/neodym121/typr-updated?color=ff80eb&labelColor=black&style=flat-square" alt="issues" /></a>
  <a href="https://github.com/neodym121/typr-updated/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-white?labelColor=black&style=flat-square" alt="license" /></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <b>Русский</b>
</p>

Typr — лёгкое приложение для голосового ввода на Windows. Нажимаешь глобальный хоткей, говоришь, и распознанный текст вставляется в активное окно. А ещё оно умеет отвечать: спросите ИИ о выделенном тексте или поговорите с ним голосом в маленьком окне у края экрана.

<p align="center">
  <img src="assets/screenshot.png" width="720" alt="Окно настроек Typr" />
</p>

## Возможности

### Диктовка & Текст

- **Глобальный хоткей** в двух режимах (переключение или удержание): сочетание клавиш или одна кнопка мыши (средняя или боковая). 
- **Локальное распознавание** через [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp): Parakeet TDT 0.6B v3, Whisper Large v3, Large v3 Turbo и Medium работают прямо на компьютере, звук никуда не уходит. Работают на видеокарте через Vulkan (NVIDIA, AMD или Intel) или на процессоре. Модели скачиваются и удаляются в настройках, неиспользуемая модель сама выгружается из памяти.
- **Облачные провайдеры распознавания:** Groq (Whisper Large v3/v3 Turbo), [Polza.AI](https://polza.ai/), [AssemblyAI](https://www.assemblyai.com/) (Universal-3.5 Pro/Universal-2), а также любой OpenAI-совместимый эндпоинт. Запись уходит в FLAC, где сервис его принимает, по соединению, открытому, пока вы говорите.
- **Постобработка:** нейросеть (Gemini, OpenRouter, Groq или Polza) дорабатывает текст перед вставкой в разных стилях.

### ИИ-помощник

- **Quick Ask:** выделите текст в любом приложении, нажмите хоткей диктовки и спросите о нём вслух («переведи это», «что значит эта ошибка»). Индикатор записи разворачивается в маленькое окно с ответом, а фокус остаётся в приложении, где вы были.
- **Quick Chat:** отдельный хоткей (по умолчанию Ctrl+Alt) открывает то же окно как чат с ИИ. Тот же хоткей или крестик закрывает чат.
- **Поиск в интернете:** с ключом [Exa](https://exa.ai/) модель ищет в сети, когда вопросу нужны свежие факты.

### Приложение

- **API-ключи в одном месте:** ключ каждого сервиса вводится один раз в разделе «API-ключи», в остальных разделах выбираются только сервис и модель.
- **Минимум памяти:** около 5 МБ ОЗУ в трее! WebView окна настроек выгружается после закрытия, а индикатор записи рисуется нативно.
- **Режим разработчика:** живой лог всего, что делает приложение, для поиска проблем. Там же можно держать WebView2 фоном: Quick Ask, Quick Chat и окно настроек открываются быстрее, но в простое занято больше ОЗУ (около 140 МБ сверху).
- **Интерфейс на русском и английском**, тёмный и минималистичный, в стиле Anthropic, со своими кнопками окна вместо системного заголовка.

## Сборка

Установщик собирает GitHub Actions (`.github/workflows/build.yml`) при пуше тега вида `v*` или при ручном запуске workflow:

```bash
git tag v3.0.2
git push origin v3.0.2
```

Для локальной разработки нужны [Rust](https://rustup.rs/) и [Node.js](https://nodejs.org/) 20+:

```bash
npm install
npm run tauri dev
```

Сделано на [Tauri 2](https://tauri.app/) (Rust) и чистом TypeScript.

## Авторы

- Оригинальный проект — [albertshiney](https://github.com/albertshiney/typr)
- Форк ведёт [neodym121](https://github.com/neodym121), доработан вместе с [Claude](https://claude.com/claude-code) (Anthropic)

## Лицензия

[MIT](LICENSE)

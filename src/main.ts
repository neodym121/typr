import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { fillApiKeys, initApiKeys, translateApiKeys, type ApiKeys } from "./apikeys";
import { enhanceSelect, syncSelect } from "./dropdown";
import { fillHotkeys, initHotkeys, translateHotkeys } from "./hotkeys";
import { fillLanguageOptions, getLanguage, setLanguage, t, type Lang, type MessageKey } from "./i18n";
import { renderIcons } from "./icons";
import { fillLocal, initLocal, showLocal, showLocalComponents, translateLocal } from "./local";
import { enhanceSegmented } from "./segmented";
import { initSetup, openSetup, translateSetup } from "./setup";
import {
  fillPostProcess,
  initPostProcess,
  showPostProcess,
  translatePostProcess,
  type PostProcessSettings,
} from "./postprocess";
import {
  fillQuickAsk,
  initQuickAsk,
  renderAskHint,
  showQuickAsk,
  translateQuickAsk,
  type QuickAskSettings,
} from "./quickask";

interface Settings {
  microphone: string;
  engine: string;
  /** One key per service (src/apikeys.ts) */
  apiKeys: ApiKeys;
  groqModel: string;
  openaiEndpoint: string;
  openaiModel: string;
  polzaModel: string;
  polzaProvider: string;
  assemblyaiModel: string;
  /** Language of the speech, e.g. "ru"; empty detects it */
  recognitionLanguage: string;
  /** A failed cloud transcription goes to a downloaded local model */
  fallbackLocal: boolean;
  /** Local engine: model id, backend ("" = recommended) and unload delay */
  localModel: string;
  localBackend: string;
  localUnload: string;
  recordingMode: string;
  hotkey: string;
  /** Opens the quick ask window empty and closes it (src/hotkeys.ts) */
  askHotkey: string;
  appendSpace: boolean;
  showIndicator: boolean;
  /** "top-right" | "bottom-center" (just above the taskbar) */
  indicatorPosition: string;
  /** "en", "ru", or empty to follow the system */
  language: string;
  postProcess: PostProcessSettings;
  /** Select text and ask about it with the hotkey, or open the window with
   *  its own (src/quickask.ts) */
  quickAsk: QuickAskSettings;
  checkUpdates: boolean;
  /** The first-run setup was finished or skipped */
  setupDone: boolean;
  developerMode: boolean;
}

interface MicDevice {
  name: string;
  is_default: boolean;
}

/** A newer release on GitHub (updates.rs) */
interface UpdateInfo {
  version: string;
  url: string;
}

/** A cloud engine failed and the local model transcribed instead */
interface FallbackNotice {
  engine: string;
  error: string;
}

type LogLevel = "error" | "warn" | "info" | "debug";

interface LogEntry {
  id: number;
  ts: number;
  level: LogLevel;
  source: string;
  message: string;
}

// ── Frontend logging ─────────────────────────────────
// While developer mode is on, everything the UI logs (console output and
// uncaught errors included) is forwarded to the backend log.

let developerMode = false;

const nativeConsole = {
  log: console.log.bind(console),
  info: console.info.bind(console),
  warn: console.warn.bind(console),
  error: console.error.bind(console),
  debug: console.debug.bind(console),
};

function stringify(value: unknown): string {
  if (typeof value === "string") return value;
  if (value instanceof Error) return value.stack || `${value.name}: ${value.message}`;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function uiLog(level: LogLevel, ...parts: unknown[]) {
  if (!developerMode) return;
  const message = parts.map(stringify).join(" ");
  invoke("frontend_log", { level, message }).catch((err) =>
    nativeConsole.error("Failed to forward log entry:", err),
  );
}

const consoleLevels = {
  log: "info",
  info: "info",
  warn: "warn",
  error: "error",
  debug: "debug",
} as const;

for (const method of Object.keys(consoleLevels) as (keyof typeof consoleLevels)[]) {
  console[method] = (...args: unknown[]) => {
    nativeConsole[method](...args);
    uiLog(consoleLevels[method], ...args);
  };
}

window.addEventListener("error", (event) => {
  uiLog("error", `Uncaught error: ${event.message} (${event.filename}:${event.lineno}:${event.colno})`);
});

window.addEventListener("unhandledrejection", (event) => {
  uiLog("error", "Unhandled promise rejection:", event.reason);
});

// DOM elements
const statusIndicator = document.getElementById("status-indicator")!;
const statusText = document.getElementById("status-text")!;
const statusDetail = document.getElementById("status-detail")!;
const micSelect = document.getElementById("mic-select") as HTMLSelectElement;
const indicatorToggle = document.getElementById("indicator-toggle") as HTMLInputElement;
const indicatorPositionRow = document.getElementById("indicator-position-row")!;
const indicatorPositionButtons = document.querySelectorAll<HTMLButtonElement>("#indicator-position .segment");
const autostartToggle = document.getElementById("autostart-toggle") as HTMLInputElement;
const updatesToggle = document.getElementById("updates-toggle") as HTMLInputElement;
const updatesStatus = document.getElementById("updates-status")!;
const updatesCheck = document.getElementById("updates-check") as HTMLButtonElement;
const setupRun = document.getElementById("setup-run")!;
const updateLink = document.getElementById("update-link")!;
const recognitionLanguage = document.getElementById("recognition-language") as HTMLSelectElement;
const fallbackRow = document.getElementById("fallback-row")!;
const fallbackToggle = document.getElementById("fallback-toggle") as HTMLInputElement;
const engineLocal = document.getElementById("engine-local")!;
const engineGroq = document.getElementById("engine-groq")!;
const engineOpenai = document.getElementById("engine-openai")!;
const enginePolza = document.getElementById("engine-polza")!;
const engineAssemblyai = document.getElementById("engine-assemblyai")!;
const localSettings = document.getElementById("local-settings")!;
const cloudSettings = document.getElementById("cloud-settings")!;
const groqSettings = document.getElementById("groq-settings")!;
const openaiSettings = document.getElementById("openai-settings")!;
const polzaSettings = document.getElementById("polza-settings")!;
const assemblyaiSettings = document.getElementById("assemblyai-settings")!;
const groqModel = document.getElementById("groq-model") as HTMLSelectElement;
const openaiEndpoint = document.getElementById("openai-endpoint") as HTMLInputElement;
const openaiModel = document.getElementById("openai-model") as HTMLInputElement;
const openaiKey = document.getElementById("openai-key") as HTMLInputElement;
const polzaModel = document.getElementById("polza-model") as HTMLInputElement;
const polzaProvider = document.getElementById("polza-provider") as HTMLInputElement;
const assemblyaiModel = document.getElementById("assemblyai-model") as HTMLSelectElement;
const languageButtons = document.querySelectorAll<HTMLButtonElement>("#language-select .segment");
const modeToggle = document.getElementById("mode-toggle")!;
const modePtt = document.getElementById("mode-ptt")!;
const appendSpaceToggle = document.getElementById("append-space-toggle") as HTMLInputElement;
const devModeToggle = document.getElementById("dev-mode-toggle") as HTMLInputElement;
const tabDeveloper = document.getElementById("tab-developer")!;
// On the sidebar's Settings button and on the Developer tab
const devBadges = document.querySelectorAll<HTMLElement>(".dev-badge");
const versionText = document.getElementById("version-text")!;
const logList = document.getElementById("log-list")!;
const logEmpty = document.getElementById("log-empty")!;
const logEmptyTitle = document.getElementById("log-empty-title")!;
const logEmptyHint = document.getElementById("log-empty-hint")!;
const logJump = document.getElementById("log-jump")!;
const logSearch = document.getElementById("log-search") as HTMLInputElement;
const logCopy = document.getElementById("log-copy")!;
const logClear = document.getElementById("log-clear")!;
const logLevelButtons = document.querySelectorAll<HTMLButtonElement>("#log-levels .segment");

// Section navigation
const navItems = document.querySelectorAll<HTMLElement>("#sidebar-nav .nav-item");
const sections = document.querySelectorAll<HTMLElement>(".content-section");

function showSection(name: string) {
  navItems.forEach((n) => n.classList.toggle("active", n.dataset.section === name));
  sections.forEach((s) => s.classList.toggle("active", s.id === `section-${name}`));
  if (name === "postprocess") showPostProcess();
  if (name === "ask") showQuickAsk();
  if (name === "engine") showLocal();
}

navItems.forEach((item) => {
  item.addEventListener("click", () => showSection(item.dataset.section || "general"));
});

// ── Settings window ──────────────────────────────────
// A window inside the app over everything but the setup and the window
// buttons: General, API keys, Hotkeys and, in developer mode, Developer.

type SettingsTab = "main" | "keys" | "hotkeys" | "developer";

const settingsRoot = document.getElementById("settings")!;
const settingsBody = document.getElementById("settings-body")!;
const settingsTabs = document.querySelectorAll<HTMLButtonElement>(".settings-tab");
const settingsPanels = document.querySelectorAll<HTMLElement>(".settings-panel");
const navSettings = document.getElementById("nav-settings") as HTMLButtonElement;
const appBody = document.getElementById("app-body")!;

let settingsTab: SettingsTab = "main";
let settingsCloseTimer: number | undefined;

function isSettingsOpen(): boolean {
  return !settingsRoot.classList.contains("hidden") && !settingsRoot.classList.contains("closing");
}

function showSettingsTab(tab: SettingsTab) {
  settingsTab = tab;
  settingsTabs.forEach((button) => {
    const active = button.dataset.tab === tab;
    button.classList.toggle("active", active);
    button.setAttribute("aria-selected", String(active));
    button.tabIndex = active ? 0 : -1;
  });
  settingsPanels.forEach((panel) => panel.classList.toggle("active", panel.id === `settings-${tab}`));
  settingsBody.scrollTop = 0;
  if (tab === "developer" && isSettingsOpen()) {
    showLocalComponents();
    unseenErrors = 0;
    updateDevBadge();
    scrollLogsToBottom();
  }
}

function openSettings(tab: SettingsTab = "main") {
  window.clearTimeout(settingsCloseTimer);
  settingsRoot.classList.remove("hidden", "closing");
  // The page behind takes no clicks or focus while the window is open
  appBody.inert = true;
  navSettings.classList.add("active");
  showSettingsTab(tab);
  document.querySelector<HTMLElement>(".settings-tab.active")?.focus({ preventScroll: true });
}

function closeSettings() {
  if (!isSettingsOpen()) return;
  settingsRoot.classList.add("closing");
  settingsCloseTimer = window.setTimeout(() => {
    settingsRoot.classList.add("hidden");
    settingsRoot.classList.remove("closing");
  }, 140);
  appBody.inert = false;
  navSettings.classList.remove("active");
  navSettings.focus({ preventScroll: true });
}

navSettings.addEventListener("click", () => openSettings());
document.getElementById("settings-close")!.addEventListener("click", closeSettings);
document.getElementById("settings-backdrop")!.addEventListener("click", closeSettings);

// Escape closes the window; while a hotkey is being recorded, hotkeys.ts
// takes the key first
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && isSettingsOpen() && !e.defaultPrevented) closeSettings();
});

settingsTabs.forEach((button) => {
  button.addEventListener("click", () => showSettingsTab(button.dataset.tab as SettingsTab));
  // Arrow keys move between the visible tabs
  button.addEventListener("keydown", (e) => {
    const visible = Array.from(settingsTabs).filter((tab) => !tab.classList.contains("hidden"));
    const index = visible.indexOf(button);
    const next =
      e.key === "ArrowRight" ? visible[(index + 1) % visible.length]
      : e.key === "ArrowLeft" ? visible[(index - 1 + visible.length) % visible.length]
      : e.key === "Home" ? visible[0]
      : e.key === "End" ? visible[visible.length - 1]
      : undefined;
    if (!next) return;
    e.preventDefault();
    next.focus();
    showSettingsTab(next.dataset.tab as SettingsTab);
  });
});

// The window has no system title bar. Drag it by the empty space of the
// sidebar, the strip above the content and the setup's header
const appWindow = getCurrentWindow();

document.addEventListener("mousedown", (e) => {
  const target = e.target as HTMLElement;
  if (e.button !== 0 || !target.closest(".drag-region")) return;
  if (target.closest("button, select, input, a, label, .nav-item, #status-detail")) return;
  appWindow.startDragging();
});

document.getElementById("window-minimize")!.addEventListener("click", () => appWindow.minimize());
// Like the system button: the window and its WebView go, Typr stays in the tray
document.getElementById("window-close")!.addEventListener("click", () => appWindow.close());

// A hairline under the strip once the section is scrolled
const contentArea = document.getElementById("content")!;
const titlebar = document.getElementById("titlebar")!;
contentArea.addEventListener("scroll", () => titlebar.classList.toggle("scrolled", contentArea.scrollTop > 0), {
  passive: true,
});

let currentSettings: Settings;
let microphones: MicDevice[] = [];
let microphonesLoaded = false;
let systemLanguage: Lang = "en";

const GROQ_MODELS = ["whisper-large-v3-turbo", "whisper-large-v3"];
const ASSEMBLYAI_MODELS = ["universal-3-5-pro", "universal-2"];

// The Lucide icons of the sidebar, the window buttons and the log
renderIcons();

// App-styled lists instead of the browser's native <select> popup
fillLanguageOptions(recognitionLanguage);
[micSelect, groqModel, assemblyaiModel, recognitionLanguage].forEach(enhanceSelect);
// A thumb that springs between the segments and can be dragged
document.querySelectorAll<HTMLElement>(".segmented").forEach(enhanceSegmented);

initApiKeys({ settings: () => currentSettings, save: saveQuietly });
// Quick ask names its hotkey too
initHotkeys({ settings: () => currentSettings, save: () => saveSettings().then(renderAskHint) });
initPostProcess({ settings: () => currentSettings, save: saveQuietly });
initQuickAsk({
  settings: () => currentSettings,
  save: saveQuietly,
  enabledChanged: fillHotkeys,
  showHotkeys: () => openSettings("hotkeys"),
});
initLocal({ settings: () => currentSettings, save: saveQuietly });
initSetup({
  settings: () => currentSettings,
  save: saveSettings,
  setLanguage: chooseLanguage,
  closed: () => {
    fillForm();
    loadAutostart();
    showSection("general");
  },
});

// The window stays hidden until its language is known (see .i18n-pending);
// the timeout makes sure a failed startup never leaves it blank
function revealInterface() {
  document.documentElement.classList.remove("i18n-pending");
}
window.setTimeout(revealInterface, 1500);

function resolveLanguage(choice: string): Lang {
  return choice === "en" || choice === "ru" ? choice : systemLanguage;
}

function applyLanguage(lang: Lang) {
  setLanguage(lang);
  languageButtons.forEach((b) => b.classList.toggle("active", b.dataset.lang === lang));
  // Texts set from code
  renderStatus();
  renderMicOptions();
  updateLogMeta();
  translateApiKeys();
  translateHotkeys();
  translatePostProcess();
  translateQuickAsk();
  translateLocal();
  fillLanguageOptions(recognitionLanguage);
  syncSelect(recognitionLanguage);
  renderUpdate();
  translateSetup();
}

/** The interface language picked in General or in the setup */
function chooseLanguage(lang: Lang) {
  if (currentSettings.language === lang && getLanguage() === lang) return;
  currentSettings.language = lang;
  applyLanguage(lang);
  saveQuietly();
}

// Mic dropdown: the system default first, then every input device
function renderMicOptions() {
  if (!microphonesLoaded) return;
  const selected = micSelect.value || currentSettings.microphone || "default";
  micSelect.innerHTML = "";
  const systemDefault = microphones.find((mic) => mic.is_default);
  micSelect.appendChild(
    new Option(
      systemDefault ? t("general.systemDefaultNamed", { name: systemDefault.name }) : t("general.systemDefault"),
      "default",
    ),
  );
  microphones.forEach((mic) => micSelect.appendChild(new Option(mic.name, mic.name)));
  if (selected !== "default" && !microphones.some((mic) => mic.name === selected)) {
    micSelect.appendChild(new Option(t("general.notConnected", { name: selected }), selected));
  }
  micSelect.value = selected;
  syncSelect(micSelect);
}

async function loadSettings() {
  currentSettings = await invoke<Settings>("get_settings");
  systemLanguage = await invoke<string>("get_system_language")
    .then((lang): Lang => (lang === "ru" ? "ru" : "en"))
    .catch((): Lang => "en");
  applyLanguage(resolveLanguage(currentSettings.language || ""));
  revealInterface();

  applyDeveloperMode(Boolean(currentSettings.developerMode));
  if (developerMode) {
    await loadLogs();
  }

  microphones = await invoke<MicDevice[]>("list_microphones");
  microphonesLoaded = true;
  const savedMic = currentSettings.microphone || "default";
  if (savedMic !== "default" && !microphones.some((mic) => mic.name === savedMic)) {
    uiLog("warn", `Saved microphone "${savedMic}" is not connected, the system default will be used`);
  }

  fillForm();
  uiLog("debug", `Settings loaded, ${microphones.length} microphone(s) available`);

  loadAutostart();
  invoke<UpdateInfo | null>("get_update")
    .then((info) => {
      update = info;
      renderUpdate();
    })
    .catch((err) => uiLog("warn", "Could not read the update state:", err));

  if (!currentSettings.setupDone) {
    await openSetup();
  }
}

function loadAutostart() {
  invoke<boolean>("get_autostart")
    .then((enabled) => (autostartToggle.checked = enabled))
    .catch((err) => uiLog("warn", "Could not read the startup state:", err));
}

/** Puts the settings into the form */
function fillForm() {
  micSelect.value = "";
  renderMicOptions();

  // Recording indicator (on unless switched off)
  indicatorToggle.checked = currentSettings.showIndicator !== false;
  setIndicatorPosition(currentSettings.indicatorPosition === "bottom-center" ? "bottom-center" : "top-right");
  showIndicatorPosition();
  updatesToggle.checked = currentSettings.checkUpdates !== false;
  if (getLanguage() !== resolveLanguage(currentSettings.language || "")) {
    applyLanguage(resolveLanguage(currentSettings.language || ""));
  }

  // API keys
  fillApiKeys();

  // Engine
  setEngine(currentSettings.engine || "groq");
  setChoice(groqModel, GROQ_MODELS, currentSettings.groqModel);

  // OpenAI Compatible settings
  openaiEndpoint.value = currentSettings.openaiEndpoint || "https://api.openai.com/v1";
  openaiModel.value = currentSettings.openaiModel || "whisper-1";
  openaiKey.value = currentSettings.apiKeys?.openai || "";

  // Polza settings
  polzaModel.value = currentSettings.polzaModel || "openai/whisper-large-v3";
  polzaProvider.value = currentSettings.polzaProvider || "";

  // AssemblyAI settings
  setChoice(assemblyaiModel, ASSEMBLYAI_MODELS, currentSettings.assemblyaiModel);

  // Speech
  recognitionLanguage.value = currentSettings.recognitionLanguage || "";
  syncSelect(recognitionLanguage);
  fallbackToggle.checked = currentSettings.fallbackLocal !== false;

  // Recording mode
  setRecordingMode(currentSettings.recordingMode || "push-to-talk");

  // Hotkeys
  fillHotkeys();
  appendSpaceToggle.checked = Boolean(currentSettings.appendSpace);

  // Post-processing
  fillPostProcess();

  // Quick ask
  fillQuickAsk();

  // Local engine
  fillLocal();
}

function setEngine(engine: string) {
  currentSettings.engine = engine;
  engineLocal.classList.toggle("active", engine === "local");
  engineGroq.classList.toggle("active", engine === "groq");
  engineOpenai.classList.toggle("active", engine === "openai");
  enginePolza.classList.toggle("active", engine === "polza");
  engineAssemblyai.classList.toggle("active", engine === "assemblyai");
  groqSettings.classList.toggle("hidden", engine !== "groq");
  openaiSettings.classList.toggle("hidden", engine !== "openai");
  polzaSettings.classList.toggle("hidden", engine !== "polza");
  assemblyaiSettings.classList.toggle("hidden", engine !== "assemblyai");
  localSettings.classList.toggle("hidden", engine !== "local");
  cloudSettings.classList.toggle("hidden", engine === "local");
  fallbackRow.classList.toggle("hidden", engine === "local");
}

// Selects a saved model; an unknown one falls back to the first option
function setChoice(select: HTMLSelectElement, allowed: string[], value: string | undefined) {
  select.value = value && allowed.includes(value) ? value : allowed[0];
  syncSelect(select);
}

function setRecordingMode(mode: string) {
  currentSettings.recordingMode = mode;
  modeToggle.classList.toggle("active", mode === "toggle");
  modePtt.classList.toggle("active", mode === "push-to-talk");
}

function setIndicatorPosition(position: string) {
  currentSettings.indicatorPosition = position;
  indicatorPositionButtons.forEach((b) => b.classList.toggle("active", b.dataset.position === position));
}

/** The position only matters while the indicator is on */
function showIndicatorPosition() {
  indicatorPositionRow.classList.toggle("hidden", !indicatorToggle.checked);
}

// The form writes every change into currentSettings (here and in the
// modules); saving sends that object as it is.
async function saveSettings() {
  try {
    await invoke("save_settings", { settings: currentSettings });
  } catch (err) {
    uiLog("error", "Failed to save settings:", err);
    throw err;
  }
}

function saveQuietly() {
  saveSettings().catch(() => {
    // already logged in saveSettings
  });
}

// Event listeners for settings
engineLocal.addEventListener("click", () => {
  setEngine("local");
  saveQuietly();
  showLocal();
});

engineGroq.addEventListener("click", () => {
  setEngine("groq");
  saveQuietly();
});

engineOpenai.addEventListener("click", () => {
  setEngine("openai");
  saveQuietly();
});

enginePolza.addEventListener("click", () => {
  setEngine("polza");
  saveQuietly();
});

engineAssemblyai.addEventListener("click", () => {
  setEngine("assemblyai");
  saveQuietly();
});

/** Saves `apply`'s change to the settings when `control` changes */
function bind(control: HTMLInputElement | HTMLSelectElement, apply: () => void) {
  control.addEventListener("change", () => {
    apply();
    saveQuietly();
  });
}

bind(micSelect, () => (currentSettings.microphone = micSelect.value));
bind(indicatorToggle, () => {
  currentSettings.showIndicator = indicatorToggle.checked;
  showIndicatorPosition();
});
bind(groqModel, () => (currentSettings.groqModel = groqModel.value));
bind(appendSpaceToggle, () => (currentSettings.appendSpace = appendSpaceToggle.checked));
bind(openaiEndpoint, () => (currentSettings.openaiEndpoint = openaiEndpoint.value.trim()));
bind(openaiModel, () => (currentSettings.openaiModel = openaiModel.value.trim()));
// The OpenAI-compatible server's key is set here, next to its address, not in
// Settings → API keys
bind(openaiKey, () => (currentSettings.apiKeys.openai = openaiKey.value.trim()));
bind(polzaModel, () => (currentSettings.polzaModel = polzaModel.value.trim()));
bind(polzaProvider, () => (currentSettings.polzaProvider = polzaProvider.value.trim()));
bind(assemblyaiModel, () => (currentSettings.assemblyaiModel = assemblyaiModel.value));
bind(recognitionLanguage, () => (currentSettings.recognitionLanguage = recognitionLanguage.value));
bind(fallbackToggle, () => (currentSettings.fallbackLocal = fallbackToggle.checked));
bind(updatesToggle, () => (currentSettings.checkUpdates = updatesToggle.checked));

languageButtons.forEach((button) => {
  button.addEventListener("click", () => chooseLanguage(button.dataset.lang === "ru" ? "ru" : "en"));
});

indicatorPositionButtons.forEach((button) => {
  button.addEventListener("click", () => {
    const position = button.dataset.position || "top-right";
    if (currentSettings.indicatorPosition === position) return;
    setIndicatorPosition(position);
    saveQuietly();
  });
});

autostartToggle.addEventListener("change", async () => {
  const enabled = autostartToggle.checked;
  try {
    await invoke("set_autostart", { enabled });
  } catch (err) {
    uiLog("error", "Failed to change starting with Windows:", err);
    autostartToggle.checked = !enabled;
  }
});

// The setup covers the whole window, so Settings closes under it
setupRun.addEventListener("click", () => {
  closeSettings();
  void openSetup();
});

// ── Updates ──────────────────────────────────────────

let update: UpdateInfo | null = null;
/** What the Updates row says below its label */
let updateCheck: { kind: "idle" | "checking" | "latest" | "error"; error?: string } = { kind: "idle" };

function renderUpdate() {
  updateLink.classList.toggle("hidden", !update);
  if (update) updateLink.textContent = t("sidebar.update", { version: update.version });

  if (update) {
    updatesStatus.textContent = t("general.updatesAvailable", { version: update.version });
  } else if (updateCheck.kind === "checking") {
    updatesStatus.textContent = t("general.updatesChecking");
  } else if (updateCheck.kind === "latest") {
    updatesStatus.textContent = t("general.updatesLatest");
  } else if (updateCheck.kind === "error") {
    updatesStatus.textContent = t("general.updatesError", { error: updateCheck.error || "" });
  } else {
    updatesStatus.textContent = t("general.updatesHint");
  }
  updatesStatus.classList.toggle("update-note", Boolean(update) || updateCheck.kind === "error");
  updatesCheck.textContent = t(update ? "general.updatesOpen" : "general.updatesCheck");
  updatesCheck.disabled = updateCheck.kind === "checking";
}

function openUpdate() {
  if (!update) return;
  invoke("open_link", { url: update.url }).catch((err) => uiLog("error", "Failed to open the release page:", err));
}

updatesCheck.addEventListener("click", async () => {
  if (update) {
    openUpdate();
    return;
  }
  updateCheck = { kind: "checking" };
  renderUpdate();
  try {
    update = await invoke<UpdateInfo | null>("check_for_updates");
    updateCheck = { kind: update ? "idle" : "latest" };
  } catch (err) {
    updateCheck = { kind: "error", error: String(err) };
  }
  renderUpdate();
});

updateLink.addEventListener("click", openUpdate);

listen<UpdateInfo | null>("update-available", (event) => {
  update = event.payload;
  renderUpdate();
});

modeToggle.addEventListener("click", () => {
  setRecordingMode("toggle");
  saveQuietly();
});

modePtt.addEventListener("click", () => {
  setRecordingMode("push-to-talk");
  saveQuietly();
});

// ── Developer mode ───────────────────────────────────

function applyDeveloperMode(enabled: boolean) {
  developerMode = enabled;
  devModeToggle.checked = enabled;
  tabDeveloper.classList.toggle("hidden", !enabled);
  if (!enabled) {
    if (settingsTab === "developer") showSettingsTab("main");
    resetLogView();
  }
}

devModeToggle.addEventListener("change", async () => {
  const enabled = devModeToggle.checked;
  applyDeveloperMode(enabled);
  currentSettings.developerMode = enabled;
  try {
    await saveSettings();
  } catch {
    applyDeveloperMode(!enabled);
    currentSettings.developerMode = !enabled;
    return;
  }
  if (enabled) {
    await loadLogs();
  }
});

// ── Log console ──────────────────────────────────────

const MAX_LOG_ENTRIES = 5000;
let logEntries: LogEntry[] = [];
const seenLogIds = new Set<number>();
const levelCounts: Record<LogLevel, number> = { error: 0, warn: 0, info: 0, debug: 0 };
let levelFilter: "all" | LogLevel = "all";
let searchQuery = "";
let unseenErrors = 0;

function isDeveloperTabShown(): boolean {
  return isSettingsOpen() && settingsTab === "developer";
}

function pad(value: number, width = 2): string {
  return String(value).padStart(width, "0");
}

function formatTime(ts: number): string {
  const d = new Date(ts);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
}

function formatEntryLine(entry: LogEntry): string {
  const d = new Date(entry.ts);
  const date = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  return `${date} ${formatTime(entry.ts)} [${entry.level.toUpperCase()}] ${entry.source}: ${entry.message}`;
}

function matchesFilter(entry: LogEntry): boolean {
  if (levelFilter !== "all" && entry.level !== levelFilter) return false;
  if (!searchQuery) return true;
  return `${entry.source} ${entry.message}`.toLowerCase().includes(searchQuery);
}

function createLogRow(entry: LogEntry): HTMLElement {
  const row = document.createElement("div");
  row.className = `log-row ${entry.level}`;
  row.dataset.id = String(entry.id);

  const time = document.createElement("span");
  time.className = "log-time";
  time.textContent = formatTime(entry.ts);
  time.title = new Date(entry.ts).toLocaleString();

  const level = document.createElement("span");
  level.className = `log-level ${entry.level}`;
  level.textContent = entry.level;

  const source = document.createElement("span");
  source.className = "log-source";
  source.textContent = entry.source;
  source.title = entry.source;

  const message = document.createElement("span");
  message.className = "log-message";
  message.textContent = entry.message;

  row.append(time, level, source, message);
  return row;
}

function isNearBottom(): boolean {
  return logList.scrollHeight - logList.scrollTop - logList.clientHeight < 32;
}

function scrollLogsToBottom() {
  logList.scrollTop = logList.scrollHeight;
  logJump.classList.add("hidden");
}

function updateLogMeta() {
  const total = logEntries.length;
  document.querySelectorAll<HTMLElement>("#log-levels .count").forEach((el) => {
    const key = el.dataset.count as "all" | LogLevel;
    const value = key === "all" ? total : levelCounts[key];
    el.textContent = String(value);
    el.classList.toggle("alert", key === "error" && value > 0);
  });

  const hasRows = logList.childElementCount > 0;
  logEmpty.classList.toggle("hidden", hasRows);
  if (!hasRows) {
    if (total === 0) {
      logEmptyTitle.textContent = t("developer.empty");
      logEmptyHint.textContent = t("developer.emptyHint");
    } else {
      logEmptyTitle.textContent = t("developer.noMatch");
      logEmptyHint.textContent = t("developer.noMatchHint");
    }
  }
}

function updateDevBadge() {
  devBadges.forEach((badge) => {
    badge.textContent = String(unseenErrors);
    badge.classList.toggle("hidden", unseenErrors === 0);
  });
}

function renderLogs() {
  const fragment = document.createDocumentFragment();
  for (const entry of logEntries) {
    if (matchesFilter(entry)) fragment.appendChild(createLogRow(entry));
  }
  logList.replaceChildren(fragment);
  updateLogMeta();
  scrollLogsToBottom();
}

function trackEntry(entry: LogEntry) {
  seenLogIds.add(entry.id);
  levelCounts[entry.level] = (levelCounts[entry.level] || 0) + 1;
}

function trimLogEntries() {
  let removedAny = false;
  while (logEntries.length > MAX_LOG_ENTRIES) {
    const removed = logEntries.shift()!;
    seenLogIds.delete(removed.id);
    levelCounts[removed.level] -= 1;
    removedAny = true;
  }
  if (removedAny && logEntries.length > 0) {
    const minId = logEntries[0].id;
    let first = logList.firstElementChild as HTMLElement | null;
    while (first && Number(first.dataset.id) < minId) {
      first.remove();
      first = logList.firstElementChild as HTMLElement | null;
    }
  }
}

function addLogEntry(entry: LogEntry) {
  if (!developerMode || seenLogIds.has(entry.id)) return;
  logEntries.push(entry);
  trackEntry(entry);
  trimLogEntries();

  if (entry.level === "error" && !isDeveloperTabShown()) {
    unseenErrors += 1;
    updateDevBadge();
  }

  if (matchesFilter(entry)) {
    const stickToBottom = isNearBottom();
    logList.appendChild(createLogRow(entry));
    if (stickToBottom) {
      logList.scrollTop = logList.scrollHeight;
    } else {
      logJump.classList.remove("hidden");
    }
  }
  updateLogMeta();
}

function resetLogView() {
  logEntries = [];
  seenLogIds.clear();
  (Object.keys(levelCounts) as LogLevel[]).forEach((level) => (levelCounts[level] = 0));
  unseenErrors = 0;
  updateDevBadge();
  logList.replaceChildren();
  updateLogMeta();
}

async function loadLogs() {
  try {
    const fetched = await invoke<LogEntry[]>("get_logs");
    const lastFetchedId = fetched.length > 0 ? fetched[fetched.length - 1].id : 0;
    // Keep entries that arrived as events while the request was in flight
    const newer = logEntries.filter((entry) => entry.id > lastFetchedId);
    resetLogView();
    logEntries = [...fetched, ...newer];
    logEntries.forEach(trackEntry);
    trimLogEntries();
    renderLogs();
  } catch (err) {
    nativeConsole.error("Failed to load logs:", err);
  }
}

logLevelButtons.forEach((button) => {
  button.addEventListener("click", () => {
    levelFilter = (button.dataset.level as "all" | LogLevel) || "all";
    logLevelButtons.forEach((b) => b.classList.toggle("active", b === button));
    renderLogs();
  });
});

let searchTimer: number | undefined;
logSearch.addEventListener("input", () => {
  window.clearTimeout(searchTimer);
  searchTimer = window.setTimeout(() => {
    searchQuery = logSearch.value.trim().toLowerCase();
    renderLogs();
  }, 120);
});

logList.addEventListener("scroll", () => {
  if (isNearBottom()) logJump.classList.add("hidden");
});

logJump.addEventListener("click", () => scrollLogsToBottom());

async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    const ok = document.execCommand("copy");
    area.remove();
    return ok;
  }
}

// Shows `text` on a button for a moment, then its usual (translated) label
function flashButton(button: HTMLElement, text: string) {
  button.textContent = text;
  window.clearTimeout(Number(button.dataset.flashTimer));
  button.dataset.flashTimer = String(
    window.setTimeout(() => {
      button.textContent = t(button.dataset.i18n as MessageKey);
    }, 1200),
  );
}

logCopy.addEventListener("click", async () => {
  const lines = logEntries.filter(matchesFilter).map(formatEntryLine);
  if (lines.length === 0) {
    flashButton(logCopy, t("developer.nothingToCopy"));
    return;
  }
  const ok = await copyText(lines.join("\n"));
  flashButton(logCopy, ok ? t("developer.copied", { count: lines.length }) : t("developer.copyFailed"));
});

logClear.addEventListener("click", async () => {
  try {
    await invoke("clear_logs");
  } catch (err) {
    nativeConsole.error("Failed to clear logs:", err);
  }
  resetLogView();
});

listen<LogEntry>("log-entry", (event) => addLogEntry(event.payload));

// The side mouse buttons never navigate the settings page back or forward
window.addEventListener("mouseup", (e) => {
  if (e.button === 3 || e.button === 4) e.preventDefault();
});

// ── Recording status ─────────────────────────────────

type StatusState = "ready" | "recording" | "transcribing" | "error" | "paused" | "notice";
let statusResetTimer: number | undefined;
let hotkeyEnabled = true;

interface StatusView {
  state: StatusState;
  text: MessageKey;
  /** Translated detail line */
  detail?: MessageKey;
  /** Detail shown as is (error messages from the backend) */
  rawDetail?: string;
}

let statusView: StatusView = { state: "ready", text: "status.ready" };

function setStatus(view: StatusView) {
  statusView = view;
  renderStatus();
}

function renderStatus() {
  const detail = statusView.detail ? t(statusView.detail) : statusView.rawDetail || "";
  statusIndicator.dataset.state = statusView.state;
  statusText.textContent = t(statusView.text);
  statusDetail.textContent = detail;
  statusDetail.classList.toggle("hidden", !detail);
  statusIndicator.title = detail;
}

// Idle status: "Ready", or "Hotkey off" while the tray switch is off
function setIdleStatus() {
  if (hotkeyEnabled) {
    setStatus({ state: "ready", text: "status.ready" });
  } else {
    setStatus({ state: "paused", text: "status.hotkeyOff", detail: "status.hotkeyOffHint" });
  }
}

function applyRecordingState(state: string) {
  if (state === "Recording") {
    window.clearTimeout(statusResetTimer);
    setStatus({ state: "recording", text: "status.recording" });
  } else if (state === "Transcribing") {
    setStatus({ state: "transcribing", text: "status.transcribing" });
  } else if (statusIndicator.dataset.state !== "error" && statusIndicator.dataset.state !== "notice") {
    setIdleStatus();
  }
}

function applyHotkeyEnabled(enabled: boolean) {
  hotkeyEnabled = enabled;
  const state = statusIndicator.dataset.state;
  if (state !== "recording" && state !== "transcribing") {
    window.clearTimeout(statusResetTimer);
    setIdleStatus();
  }
}

listen<string>("recording-state", (event) => applyRecordingState(event.payload));

listen<boolean>("hotkey-enabled", (event) => applyHotkeyEnabled(event.payload));

listen<string>("recording-error", (event) => {
  window.clearTimeout(statusResetTimer);
  setStatus({ state: "error", text: "status.error", rawDetail: event.payload });
  statusResetTimer = window.setTimeout(setIdleStatus, 12000);
});

// The cloud failed but the local model saved the dictation; arrives before
// the recorder is back to Ready
listen<FallbackNotice>("transcription-fallback", (event) => {
  window.clearTimeout(statusResetTimer);
  const { engine, error } = event.payload;
  setStatus({
    state: "notice",
    text: "status.fallback",
    rawDetail: t("status.fallbackDetail", { engine, error }),
  });
  statusResetTimer = window.setTimeout(setIdleStatus, 12000);
});

// Engine switched from the tray menu
listen<string>("engine-changed", (event) => {
  if (!currentSettings || currentSettings.engine === event.payload) return;
  setEngine(event.payload);
  showLocal();
});

// Initialize
getVersion()
  .then((version) => {
    versionText.textContent = `v${version}`;
  })
  .catch(() => {
    // keep the version from the markup
  });

invoke<boolean>("get_hotkey_enabled")
  .then(applyHotkeyEnabled)
  .catch((err) => uiLog("warn", "Could not read hotkey state:", err))
  .finally(() =>
    invoke<string>("get_recording_state")
      .then(applyRecordingState)
      .catch((err) => uiLog("warn", "Could not read recording state:", err)),
  );

loadSettings().catch((err) => {
  revealInterface();
  nativeConsole.error("Failed to load settings:", err);
  uiLog("error", "Failed to load settings:", err);
});

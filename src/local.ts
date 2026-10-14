// Local engine panel (Engine → Local): models to download, delete and pick
// as the default, the acceleration (Vulkan or the processor), and when an
// idle model leaves memory. The runtime components come with the models (the
// backend fetches them) and can't be deleted; a note speaks up only when a
// downloaded model is left without them.
// The settings live in the app's settings object; this module edits them in
// place and asks the host to save. Everything else comes from the backend
// (local_status) and is re-read whenever it reports a change. The setup
// wizard reads the same state through localState() and onLocalChange().

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { enhanceSelect, syncSelect } from "./dropdown";
import { getLanguage, t, type MessageKey } from "./i18n";

type Backend = "vulkan" | "cpu";

export interface ModelStatus {
  id: string;
  name: string;
  size: number;
  installed: boolean;
}

interface BackendStatus {
  id: Backend;
  available: boolean;
}

interface RuntimeStatus {
  size: number;
  installed: boolean;
}

interface LoadedView {
  model: string;
  name: string;
  backend: string;
  device: string;
  fallback: string | null;
}

export interface DownloadProgress {
  key: string;
  state: "downloading" | "installing" | "done" | "error" | "cancelled";
  downloaded: number;
  total: number | null;
  error: string | null;
}

export interface LocalStatus {
  supported: boolean;
  models: ModelStatus[];
  backends: BackendStatus[];
  recommended: Backend;
  runtime: RuntimeStatus;
  loaded: LoadedView | null;
  downloads: DownloadProgress[];
  folder: string;
}

export interface LocalSettings {
  engine: string;
  localModel: string;
  localBackend: string;
  localUnload: string;
}

export interface LocalHost {
  settings: () => LocalSettings;
  save: () => void;
}

export const BADGES: Record<string, MessageKey> = {
  "parakeet-tdt-0.6b-v3": "local.recommended",
  "whisper-large-v3": "local.accurate",
};

const UNLOAD_CHOICES = ["immediate", "30s", "5m", "10m", "never"];
const RUNTIME_KEY = "runtime";
const CONFIRM_TIMEOUT = 3000;

const panel = document.getElementById("local-settings")!;
const unsupportedNote = document.getElementById("local-unsupported")!;
const modelList = document.getElementById("local-model-list")!;
const backendButtons = document.querySelectorAll<HTMLButtonElement>("#local-backends .segment");
const fallbackLine = document.getElementById("local-fallback")!;
const runtimeProblem = document.getElementById("local-runtime-problem")!;
const runtimeProblemText = document.getElementById("local-runtime-problem-text")!;
const runtimeRetry = document.getElementById("local-runtime-retry")!;
const unloadSelect = document.getElementById("local-unload") as HTMLSelectElement;

let host: LocalHost | undefined;
let status: LocalStatus | undefined;
/** Running downloads by key ("model:<id>", "runtime") */
const downloads = new Map<string, DownloadProgress>();
/** Last failure per download key, shown until the next attempt */
const errors = new Map<string, string>();
/** Delete buttons waiting for a second click */
const confirming = new Set<string>();
let refreshTimer: number | undefined;
/** Called whenever the status, a download or an error changes */
const listeners = new Set<() => void>();

function notify() {
  listeners.forEach((listener) => listener());
}

function settings(): LocalSettings {
  return host!.settings();
}

/** Decimal units, as Hugging Face and the model docs show them (740 MB) */
export function formatSize(bytes: number): string {
  const locale = getLanguage() === "ru" ? "ru-RU" : "en-US";
  const mb = bytes / 1e6;
  if (mb >= 1000) {
    const gb = new Intl.NumberFormat(locale, { maximumFractionDigits: 2 }).format(mb / 1000);
    return t("local.gb", { n: gb });
  }
  return t("local.mb", { n: Math.max(1, Math.round(mb)) });
}

function button(label: string, onClick: () => void, extra = ""): HTMLButtonElement {
  const el = document.createElement("button");
  el.type = "button";
  el.className = `btn btn-secondary btn-compact ${extra}`.trim();
  el.textContent = label;
  el.addEventListener("click", onClick);
  return el;
}

export function progressText(progress: DownloadProgress): string {
  if (progress.state === "installing") return t("local.installing");
  if (!progress.total) {
    return progress.downloaded > 0 ? formatSize(progress.downloaded) : t("local.starting");
  }
  const percent = Math.min(100, Math.floor((progress.downloaded / progress.total) * 100));
  return t("local.progress", {
    percent,
    done: formatSize(progress.downloaded),
    total: formatSize(progress.total),
  });
}

function fillBar(bar: HTMLElement, progress: DownloadProgress) {
  const share = progress.total ? progress.downloaded / progress.total : 0;
  const fill = bar.firstElementChild as HTMLElement;
  fill.style.width = `${progress.state === "installing" ? 100 : Math.min(100, share * 100)}%`;
  bar.classList.toggle("indeterminate", progress.state === "installing" || !progress.total);
}

function progressBar(progress: DownloadProgress): HTMLElement {
  const bar = document.createElement("div");
  bar.className = "progress";
  bar.dataset.key = progress.key;
  const fill = document.createElement("div");
  fill.className = "progress-fill";
  bar.append(fill);
  fillBar(bar, progress);
  return bar;
}

function progressLabel(progress: DownloadProgress): HTMLElement {
  const text = document.createElement("span");
  text.className = "progress-text";
  text.dataset.key = progress.key;
  text.textContent = progressText(progress);
  return text;
}

/** Updates a running download without rebuilding its row (and its buttons) */
function updateInPlace(progress: DownloadProgress): boolean {
  const previous = downloads.get(progress.key);
  if (!previous || previous.state !== progress.state) return false;
  const bars = panel.querySelectorAll<HTMLElement>(`.progress[data-key="${progress.key}"]`);
  const labels = panel.querySelectorAll<HTMLElement>(`.progress-text[data-key="${progress.key}"]`);
  if (bars.length === 0 && labels.length === 0) return false;
  downloads.set(progress.key, progress);
  bars.forEach((bar) => fillBar(bar, progress));
  labels.forEach((label) => (label.textContent = progressText(progress)));
  return true;
}

const TRASH_ICON =
  '<svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">' +
  '<path d="M2.75 4.25h10.5M6.25 4.25V3a.75.75 0 01.75-.75h2a.75.75 0 01.75.75v1.25' +
  'M4.25 4.25l.6 8.4a1 1 0 001 .93h4.3a1 1 0 001-.93l.6-8.4M6.75 7v4M9.25 7v4" ' +
  'stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"/></svg>';

/** A trash button that asks "Delete?" before it acts */
function deleteButton(key: string, onConfirm: () => void): HTMLButtonElement {
  const waiting = confirming.has(key);
  const el = button(
    waiting ? t("local.confirmDelete") : "",
    () => {
      if (confirming.has(key)) {
        confirming.delete(key);
        onConfirm();
        return;
      }
      confirming.add(key);
      render();
      window.setTimeout(() => {
        if (confirming.delete(key)) render();
      }, CONFIRM_TIMEOUT);
    },
    waiting ? "danger" : "btn-icon",
  );
  if (!waiting) {
    el.innerHTML = TRASH_ICON;
    el.title = t("local.delete");
    el.setAttribute("aria-label", t("local.delete"));
  }
  return el;
}

// ── Choices ──────────────────────────────────────────

function backendStatus(id: Backend): BackendStatus | undefined {
  return status?.backends.find((b) => b.id === id);
}

/** The saved choice if this computer can use it, else the recommended one */
function currentBackend(): Backend {
  const saved = settings().localBackend as Backend;
  if (backendStatus(saved)?.available) return saved;
  return status?.recommended ?? "cpu";
}

function isInstalled(id: string): boolean {
  return Boolean(status?.models.find((m) => m.id === id)?.installed);
}

// ── Rendering ────────────────────────────────────────

function renderModels() {
  if (!status) return;
  const chosen = settings().localModel;
  modelList.replaceChildren(
    ...status.models.map((model) => {
      const key = `model:${model.id}`;
      const progress = downloads.get(key);
      const row = document.createElement("div");
      row.className = "local-model";

      const info = document.createElement("div");
      info.className = "local-model-info";

      const title = document.createElement("div");
      title.className = "local-model-title";
      const name = document.createElement("span");
      name.className = "local-model-name";
      name.textContent = model.name;
      title.append(name);
      if (BADGES[model.id]) {
        const badge = document.createElement("span");
        badge.className = "badge";
        badge.textContent = t(BADGES[model.id]);
        title.append(badge);
      }
      if (model.installed && model.id === chosen) {
        const badge = document.createElement("span");
        badge.className = "badge badge-default";
        badge.textContent = t("local.default");
        title.append(badge);
      }

      const desc = document.createElement("span");
      desc.className = "label-hint";
      desc.textContent = t(`local.about.${model.id}` as MessageKey);
      const meta = document.createElement("span");
      meta.className = "label-hint local-model-meta";
      meta.textContent = `${t(`local.languages.${model.id}` as MessageKey)} · ${formatSize(model.size)}`;
      info.append(title, desc, meta);

      const actions = document.createElement("div");
      actions.className = "local-model-actions";

      if (progress) {
        info.append(progressBar(progress));
        actions.append(progressLabel(progress));
        if (progress.state === "downloading") {
          actions.append(button(t("local.cancel"), () => cancelDownload(key)));
        }
      } else if (model.installed) {
        if (model.id !== chosen) {
          actions.append(button(t("local.makeDefault"), () => chooseModel(model.id)));
        }
        actions.append(deleteButton(key, () => void deleteModel(model.id)));
      } else {
        actions.append(button(t("local.download"), () => void downloadModel(model.id)));
      }

      const error = errors.get(key);
      if (error) {
        const line = document.createElement("span");
        line.className = "local-error";
        line.textContent = t("local.downloadError", { error });
        info.append(line);
      }

      row.append(info, actions);
      return row;
    }),
  );
}

function renderBackends() {
  if (!status) return;
  const current = currentBackend();
  backendButtons.forEach((b) => {
    const id = b.dataset.backend as Backend;
    const info = backendStatus(id);
    const available = Boolean(info?.available);
    b.disabled = !available;
    b.classList.toggle("active", id === current);
    b.title = available ? "" : t(`local.unavailable.${id}` as MessageKey);
  });

  // The model in memory couldn't start on the GPU and runs on the processor
  const fallback = status.loaded?.fallback;
  fallbackLine.textContent = fallback ? t("local.fallback", { reason: fallback }) : "";
  fallbackLine.classList.toggle("hidden", !fallback);
}

/** The components come with the models; the Local tab speaks up only when a
 *  downloaded model can't run without them (their download failed). */
function renderRuntimeProblem() {
  if (!status) return;
  const error = errors.get(RUNTIME_KEY);
  const needed = error !== undefined || status.models.some((m) => m.installed);
  const show = status.supported && !status.runtime.installed && !downloads.has(RUNTIME_KEY) && needed;
  runtimeProblem.classList.toggle("hidden", !show);
  if (!show) return;
  runtimeProblemText.textContent = error
    ? t("local.runtimeFailed", { error })
    : t("local.runtimeMissing", { size: formatSize(status.runtime.size) });
  runtimeRetry.textContent = t(error ? "local.retry" : "local.download");
}

function render() {
  if (!host || !status) return;
  unsupportedNote.classList.toggle("hidden", status.supported);
  renderModels();
  renderBackends();
  renderRuntimeProblem();
  notify();
}

// ── Actions ──────────────────────────────────────────

async function refresh() {
  try {
    status = await invoke<LocalStatus>("local_status");
  } catch (err) {
    console.error("Failed to read the local engine status:", err);
    return;
  }
  // Downloads that run in the background while the window was closed
  for (const progress of status.downloads) {
    if (!downloads.has(progress.key)) downloads.set(progress.key, progress);
  }
  for (const key of [...downloads.keys()]) {
    if (!status.downloads.some((d) => d.key === key)) downloads.delete(key);
  }
  render();
}

function scheduleRefresh() {
  window.clearTimeout(refreshTimer);
  refreshTimer = window.setTimeout(() => void refresh(), 120);
}

async function start(key: string, command: string, args: Record<string, string> = {}) {
  errors.delete(key);
  downloads.set(key, { key, state: "downloading", downloaded: 0, total: null, error: null });
  render();
  try {
    await invoke(command, args);
  } catch (err) {
    downloads.delete(key);
    errors.set(key, String(err));
    render();
  }
}

async function downloadRuntime() {
  await start(RUNTIME_KEY, "local_download_runtime");
}

/** The backend fetches the components alongside the model if they're missing */
export async function downloadModel(id: string) {
  await start(`model:${id}`, "local_download_model", { id });
}

export function cancelDownload(key: string) {
  invoke("local_cancel_download", { key }).catch((err) => console.error("Failed to cancel:", err));
}

export function chooseModel(id: string) {
  settings().localModel = id;
  host!.save();
  render();
}

async function deleteModel(id: string) {
  try {
    await invoke("local_delete_model", { id });
  } catch (err) {
    errors.set(`model:${id}`, String(err));
  }
  await refresh();
  // The default model is gone: another downloaded one takes its place
  if (settings().localModel === id) {
    const other = status?.models.find((m) => m.installed);
    if (other) chooseModel(other.id);
  }
}

function onProgress(progress: DownloadProgress) {
  const finished = progress.state === "done" || progress.state === "error" || progress.state === "cancelled";
  if (!finished) {
    if (updateInPlace(progress)) {
      notify();
    } else {
      downloads.set(progress.key, progress);
      render();
    }
    return;
  }
  downloads.delete(progress.key);
  if (progress.state === "error" && progress.error) errors.set(progress.key, progress.error);
  if (progress.state === "done" && progress.key.startsWith("model:")) {
    // The first downloaded model becomes the default one
    const id = progress.key.slice("model:".length);
    const model = status?.models.find((m) => m.id === id);
    if (model) model.installed = true;
    if (!isInstalled(settings().localModel)) chooseModel(id);
  }
  void refresh();
}

// ── Public ───────────────────────────────────────────

export function initLocal(appHost: LocalHost) {
  host = appHost;
  enhanceSelect(unloadSelect);

  backendButtons.forEach((b) => {
    b.addEventListener("click", () => {
      if (b.disabled) return;
      settings().localBackend = b.dataset.backend || "";
      host!.save();
      // The backend side may have unloaded the model; status follows
      render();
      scheduleRefresh();
    });
  });

  unloadSelect.addEventListener("change", () => {
    settings().localUnload = unloadSelect.value;
    host!.save();
  });

  runtimeRetry.addEventListener("click", () => void downloadRuntime());

  listen<DownloadProgress>("local-download", (event) => onProgress(event.payload));
  listen("local-changed", () => scheduleRefresh());
}

/** Puts the saved settings into the form (after they are loaded). */
export function fillLocal() {
  const saved = settings().localUnload;
  unloadSelect.value = UNLOAD_CHOICES.includes(saved) ? saved : "5m";
  syncSelect(unloadSelect);
  void refresh();
}

/** The Local tab was opened: show fresh status. */
export function showLocal() {
  if (host && !panel.classList.contains("hidden")) void refresh();
}

/** The Developer section, with the components row, was opened. */
export function showLocalComponents() {
  if (host) void refresh();
}

/** Texts set from code, after a language switch. */
export function translateLocal() {
  render();
}

/** What the backend reported last, for the setup wizard */
export function localState() {
  return { status, downloads, errors };
}

/** Re-reads the status from the backend */
export function refreshLocal() {
  return refresh();
}

/** Calls `listener` whenever the local state changes; returns the unsubscribe */
export function onLocalChange(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

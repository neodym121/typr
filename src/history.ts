// History section: the latest dictations, a day at a time, with the text
// post-processing made of them below the text speech recognition heard. A
// click on a text copies it; the trash forgets the entry; an audio file can
// be uploaded and transcribed with the engine chosen in Engine. The entries
// live in history.json (src-tauri/src/history.rs).

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { enhanceSelect, syncSelect } from "./dropdown";
import { t } from "./i18n";
import { icon } from "./icons";

/** A dictation or an uploaded file (history.rs `Entry`) */
interface HistoryEntry {
  id: number;
  createdAt: number;
  /** Seconds; 0 when the length isn't known */
  duration: number;
  text: string;
  processed?: string | null;
  model: string;
  file?: string | null;
  /** The saved recording's file name, when it has one */
  audio?: string | null;
}

export interface HistoryHost {
  settings: () => { historyLimit: number; engine: string };
  save: () => void;
}

/** settings.rs `history::LIMITS` */
const LIMITS = [5, 10, 20] as const;
const DEFAULT_LIMIT = 20;
/** recorder.rs `MAX_FILE_BYTES` */
const MAX_FILE_BYTES = 24 * 1024 * 1024;
const AUDIO_TYPES: Record<string, string> = {
  wav: "audio/wav",
  flac: "audio/flac",
  mp3: "audio/mpeg",
  m4a: "audio/mp4",
  ogg: "audio/ogg",
  opus: "audio/ogg",
  webm: "audio/webm",
};

const list = document.getElementById("history-list")!;
const empty = document.getElementById("history-empty")!;
const status = document.getElementById("history-status")!;
const limitSelect = document.getElementById("history-limit") as HTMLSelectElement;
const uploadButton = document.getElementById("history-upload") as HTMLButtonElement;
const uploadLabel = document.getElementById("history-upload-label")!;
const fileInput = document.getElementById("history-file") as HTMLInputElement;

let host: HistoryHost | undefined;
let entries: HistoryEntry[] = [];
let loaded = false;
let uploading = false;
let converting = false;
/** Entries whose post-processing runs right now */
const processing = new Set<number>();
/** Why the post-processing of an entry failed */
const failures = new Map<number, string>();
let statusTimer: number | undefined;

const playback = new Audio();
playback.preload = "metadata";
let playbackId: number | null = null;
let playbackUrl: string | null = null;
let playbackRequest = 0;
let loadingAudioId: number | null = null;
let playbackFrame = 0;

function pad(value: number, width = 2): string {
  return String(value).padStart(width, "0");
}

/** DD.MM.YYYY */
function formatDate(ts: number): string {
  const date = new Date(ts);
  return `${pad(date.getDate())}.${pad(date.getMonth() + 1)}.${date.getFullYear()}`;
}

/** HH:MM:SS */
function formatTime(ts: number): string {
  const date = new Date(ts);
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

function formatDuration(seconds: number): string {
  const total = Number.isFinite(seconds) ? Math.max(0, Math.round(seconds)) : 0;
  return `${Math.floor(total / 60)}:${pad(total % 60)}`;
}

function audioUrl(data: string, name: string): string {
  const binary = atob(data);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index++) bytes[index] = binary.charCodeAt(index);
  const format = name.split(".").pop()?.toLowerCase() ?? "";
  return URL.createObjectURL(new Blob([bytes], { type: AUDIO_TYPES[format] ?? "application/octet-stream" }));
}

function stopPlaybackFrames() {
  if (playbackFrame) cancelAnimationFrame(playbackFrame);
  playbackFrame = 0;
}

function advancePlayback() {
  playbackFrame = 0;
  updateAudioPlayers();
  if (playbackId !== null && !playback.paused && !playback.ended) {
    playbackFrame = requestAnimationFrame(advancePlayback);
  }
}

function handlePlaybackEvent() {
  stopPlaybackFrames();
  advancePlayback();
}

function stopAudio() {
  stopPlaybackFrames();
  playbackRequest++;
  playback.pause();
  playback.removeAttribute("src");
  playback.load();
  if (playbackUrl) URL.revokeObjectURL(playbackUrl);
  playbackUrl = null;
  playbackId = null;
  loadingAudioId = null;
  updateAudioPlayers();
}

function updateAudioPlayers() {
  list.querySelectorAll<HTMLElement>(".history-audio").forEach((row) => {
    const id = Number(row.dataset.id);
    const current = playbackId === id;
    const loading = loadingAudioId === id;
    const playing = current && !playback.paused && !playback.ended;
    const button = row.querySelector<HTMLButtonElement>(".history-play")!;
    const state = playing ? "pause" : "play";
    if (button.dataset.state !== state) {
      button.replaceChildren(icon(state, 13));
      button.dataset.state = state;
    }
    const label = t(playing ? "history.pause" : "history.play");
    button.title = label;
    button.setAttribute("aria-label", label);
    button.disabled = loading;
    button.classList.toggle("playing", playing);
    row.classList.toggle("loading", loading);

    const knownDuration = current && Number.isFinite(playback.duration) && playback.duration > 0;
    const duration = knownDuration ? playback.duration : Number(row.dataset.duration) || 0;
    const currentTime = current ? playback.currentTime : 0;
    const seek = row.querySelector<HTMLInputElement>(".history-audio-seek")!;
    const slider = row.querySelector<HTMLElement>(".history-audio-slider")!;
    const fraction = duration > 0 ? Math.min(1, Math.max(0, currentTime / duration)) : 0;
    seek.max = String(Math.max(duration, 0.1));
    seek.value = String(Math.min(currentTime, duration));
    seek.disabled = !knownDuration || loading;
    slider.style.setProperty("--progress", `${fraction * 100}%`);
    slider.classList.toggle("disabled", seek.disabled);
    row.querySelector<HTMLElement>(".history-audio-time")!.textContent =
      `${formatDuration(currentTime)} / ${formatDuration(duration)}`;
  });
}

async function toggleAudio(entry: HistoryEntry) {
  if (loadingAudioId === entry.id) return;
  if (playbackId === entry.id && playback.src) {
    if (!playback.paused) {
      playback.pause();
      return;
    }
    if (playback.ended) playback.currentTime = 0;
    try {
      await playback.play();
    } catch (error) {
      showStatus(t("history.playFailed", { error: String(error) }), true);
    }
    return;
  }

  stopAudio();
  const request = playbackRequest;
  playbackId = entry.id;
  loadingAudioId = entry.id;
  updateAudioPlayers();
  try {
    const data = await invoke<string | null>("get_history_audio", { id: entry.id });
    if (request !== playbackRequest) return;
    if (!data) {
      stopAudio();
      showStatus(t("history.audioMissing"), true);
      return;
    }
    playbackUrl = audioUrl(data, entry.audio!);
    playback.src = playbackUrl;
    await playback.play();
  } catch (error) {
    if (request !== playbackRequest) return;
    stopAudio();
    showStatus(t("history.playFailed", { error: String(error) }), true);
  } finally {
    if (request === playbackRequest) {
      loadingAudioId = null;
      updateAudioPlayers();
    }
  }
}

function audioPlayer(entry: HistoryEntry): HTMLElement {
  const row = document.createElement("div");
  row.className = "history-audio";
  row.dataset.id = String(entry.id);
  row.dataset.duration = String(entry.duration);

  const button = document.createElement("button");
  button.type = "button";
  button.className = "history-play";
  button.append(icon("play", 13));
  button.addEventListener("click", () => void toggleAudio(entry));

  const seek = document.createElement("input");
  seek.type = "range";
  seek.className = "history-audio-seek";
  seek.min = "0";
  seek.step = "any";
  seek.value = "0";
  seek.disabled = true;
  seek.setAttribute("aria-label", t("history.seek"));
  seek.addEventListener("input", () => {
    if (playbackId !== entry.id || !Number.isFinite(playback.duration)) return;
    playback.currentTime = Number(seek.value);
    updateAudioPlayers();
  });

  const slider = document.createElement("div");
  slider.className = "history-audio-slider disabled";
  const track = document.createElement("span");
  track.className = "history-audio-track";
  const fill = document.createElement("span");
  fill.className = "history-audio-fill";
  const thumb = document.createElement("span");
  thumb.className = "history-audio-thumb";
  track.append(fill, thumb);
  slider.append(track, seek);

  const time = document.createElement("span");
  time.className = "history-audio-time";
  row.append(button, slider, time);
  return row;
}

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

function showStatus(text: string, isError = false) {
  status.textContent = text;
  status.classList.toggle("error", isError);
  status.classList.remove("hidden");
  window.clearTimeout(statusTimer);
  statusTimer = window.setTimeout(() => status.classList.add("hidden"), isError ? 10000 : 4000);
}

/** The flash of a copied box, like an answer's in the quick ask window */
function flashCopied(box: HTMLElement) {
  const card = box.closest(".history-entry");
  box.classList.remove("copied");
  void box.offsetWidth;
  box.classList.add("copied");
  card?.classList.add("copied");
  window.setTimeout(() => {
    box.classList.remove("copied");
    card?.classList.remove("copied");
  }, 900);
}

/** A click (or Enter) copies the text it holds */
function textBox(value: string): HTMLElement {
  const box = document.createElement("div");
  box.className = "history-text";
  box.setAttribute("role", "button");
  box.tabIndex = 0;
  box.textContent = value;
  box.title = t("history.copy");
  const copy = async () => {
    if (await copyText(value)) flashCopied(box);
    else showStatus(t("history.copyFailed"), true);
  };
  box.addEventListener("click", () => void copy());
  box.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      void copy();
    }
  });
  return box;
}

function actionButton(entry: HistoryEntry, action: "postprocess" | "delete"): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "history-action";
  if (action === "delete") {
    button.classList.add("danger");
    button.title = t("history.delete");
    button.setAttribute("aria-label", t("history.delete"));
    button.append(icon("trash-2", 14));
    button.addEventListener("click", () => void removeEntry(entry.id));
    return button;
  }
  button.title = t("history.postprocess");
  button.setAttribute("aria-label", t("history.postprocess"));
  button.append(icon("sparkles", 14));
  if (processing.has(entry.id)) {
    button.disabled = true;
  }
  button.addEventListener("click", () => void postprocess(entry.id));
  return button;
}

function createEntry(entry: HistoryEntry): HTMLElement {
  const card = document.createElement("article");
  card.className = "history-entry";
  card.dataset.id = String(entry.id);

  const head = document.createElement("div");
  head.className = "history-head";

  const time = document.createElement("time");
  time.className = "history-time";
  time.dateTime = new Date(entry.createdAt).toISOString();
  time.textContent = formatTime(entry.createdAt);

  const meta = document.createElement("span");
  meta.className = "history-meta";
  if (entry.file) {
    meta.classList.add("history-file");
    meta.append(icon("upload", 12), document.createTextNode(entry.file));
    meta.title = entry.file;
  } else {
    meta.textContent = entry.duration > 0 ? `${entry.model} · ${formatDuration(entry.duration)}` : entry.model;
    meta.title = meta.textContent;
  }

  const end = document.createElement("div");
  end.className = "history-head-end";
  const copied = document.createElement("span");
  copied.className = "history-copied";
  copied.textContent = t("history.copied");
  end.appendChild(copied);

  const actions = document.createElement("div");
  actions.className = "history-actions";
  // Only a file has a text of its own to post-process: a dictation already
  // shows what post-processing made of it
  if (entry.file) actions.appendChild(actionButton(entry, "postprocess"));
  actions.appendChild(actionButton(entry, "delete"));
  end.appendChild(actions);

  head.append(time, meta, end);
  card.append(head, textBox(entry.text));
  if (entry.audio) card.appendChild(audioPlayer(entry));

  if (processing.has(entry.id)) {
    const pending = document.createElement("div");
    pending.className = "history-processed pending";
    pending.textContent = t("history.processing");
    card.appendChild(pending);
  } else if (entry.processed != null) {
    const block = document.createElement("div");
    block.className = "history-processed";
    const label = document.createElement("span");
    label.className = "history-processed-label";
    label.append(icon("sparkles", 12), document.createTextNode(t("history.processed")));
    block.append(label, textBox(entry.processed));
    card.appendChild(block);
  }

  const failure = failures.get(entry.id);
  if (failure) {
    const error = document.createElement("p");
    error.className = "history-error";
    error.textContent = t("history.postprocessFailed", { error: failure });
    card.appendChild(error);
  }

  return card;
}

/** Newest first, one group per day */
function groupByDay(items: HistoryEntry[]): { date: string; entries: HistoryEntry[] }[] {
  const groups: { date: string; entries: HistoryEntry[] }[] = [];
  for (const entry of [...items].sort((a, b) => b.createdAt - a.createdAt)) {
    const date = formatDate(entry.createdAt);
    const last = groups[groups.length - 1];
    if (last && last.date === date) last.entries.push(entry);
    else groups.push({ date, entries: [entry] });
  }
  return groups;
}

function render() {
  const fragment = document.createDocumentFragment();
  for (const day of groupByDay(entries)) {
    const section = document.createElement("section");
    section.className = "history-day";
    const date = document.createElement("h2");
    date.className = "history-date";
    date.textContent = day.date;
    section.appendChild(date);
    for (const entry of day.entries) section.appendChild(createEntry(entry));
    fragment.appendChild(section);
  }
  list.replaceChildren(fragment);
  empty.classList.toggle("hidden", entries.length > 0);
  uploadButton.disabled = uploading;
  uploadButton.classList.toggle("busy", uploading);
  uploadLabel.textContent = t(converting ? "history.converting" : uploading ? "history.uploading" : "history.upload");
  updateAudioPlayers();
}

function setEntries(next: HistoryEntry[]) {
  entries = next;
  loaded = true;
  // An id comes back with a later entry: its old result and error must not
  const ids = new Set(next.map((entry) => entry.id));
  if ((playbackId !== null && !ids.has(playbackId)) || (loadingAudioId !== null && !ids.has(loadingAudioId))) {
    stopAudio();
  }
  for (const id of failures.keys()) if (!ids.has(id)) failures.delete(id);
  for (const id of processing) if (!ids.has(id)) processing.delete(id);
  render();
}

async function removeEntry(id: number) {
  try {
    setEntries(await invoke<HistoryEntry[]>("delete_history_entry", { id }));
  } catch (error) {
    showStatus(t("history.deleteFailed", { error: String(error) }), true);
  }
}

async function postprocess(id: number) {
  if (processing.has(id)) return;
  processing.add(id);
  failures.delete(id);
  render();
  try {
    setEntries(await invoke<HistoryEntry[]>("postprocess_history_entry", { id }));
  } catch (error) {
    failures.set(id, String(error));
  } finally {
    processing.delete(id);
    render();
  }
}

function needsWavConversion(file: File): boolean {
  const engine = host?.settings().engine;
  return (engine === "local" || engine === "polza") && /\.(mp3|mpeg|mpga|opus)$/i.test(file.name);
}

function encodeWav(samples: Float32Array): Uint8Array {
  const buffer = new ArrayBuffer(44 + samples.length * 2);
  const view = new DataView(buffer);
  const writeText = (offset: number, text: string) => {
    for (let index = 0; index < text.length; index++) view.setUint8(offset + index, text.charCodeAt(index));
  };
  writeText(0, "RIFF");
  view.setUint32(4, 36 + samples.length * 2, true);
  writeText(8, "WAVE");
  writeText(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, 16000, true);
  view.setUint32(28, 32000, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  writeText(36, "data");
  view.setUint32(40, samples.length * 2, true);
  for (let index = 0; index < samples.length; index++) {
    const sample = Math.max(-1, Math.min(1, samples[index]));
    view.setInt16(44 + index * 2, sample < 0 ? sample * 0x8000 : sample * 0x7fff, true);
  }
  return new Uint8Array(buffer);
}

async function convertToWav(file: File): Promise<Uint8Array> {
  const context = new AudioContext();
  try {
    const decoded = await context.decodeAudioData(await file.arrayBuffer());
    const offline = new OfflineAudioContext(1, Math.max(1, Math.ceil(decoded.duration * 16000)), 16000);
    const source = offline.createBufferSource();
    source.buffer = decoded;
    source.connect(offline.destination);
    source.start();
    const rendered = await offline.startRendering();
    return encodeWav(rendered.getChannelData(0));
  } finally {
    await context.close();
  }
}

function toBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += chunk) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunk));
  }
  return btoa(binary);
}

async function upload(file: File) {
  if (uploading) return;
  if (file.size > MAX_FILE_BYTES) {
    showStatus(
      t("history.fileTooBig", { size: Math.ceil(file.size / (1024 * 1024)), limit: MAX_FILE_BYTES / (1024 * 1024) }),
      true,
    );
    return;
  }
  uploading = true;
  converting = needsWavConversion(file);
  render();
  try {
    const convertedToWav = converting;
    const bytes = convertedToWav ? await convertToWav(file) : new Uint8Array(await file.arrayBuffer());
    if (bytes.length > MAX_FILE_BYTES) {
      showStatus(
        t("history.fileTooBig", { size: Math.ceil(bytes.length / (1024 * 1024)), limit: MAX_FILE_BYTES / (1024 * 1024) }),
        true,
      );
      return;
    }
    converting = false;
    if (convertedToWav) render();
    const data = toBase64(bytes);
    setEntries(await invoke<HistoryEntry[]>("transcribe_history_file", { name: file.name, data, convertedToWav }));
  } catch (error) {
    showStatus(t(converting ? "history.convertFailed" : "history.uploadFailed", { error: String(error) }), true);
  } finally {
    uploading = false;
    converting = false;
    render();
  }
}

async function refresh() {
  try {
    setEntries(await invoke<HistoryEntry[]>("get_history"));
  } catch (error) {
    showStatus(t("history.loadFailed", { error: String(error) }), true);
  }
}

export function initHistory(appHost: HistoryHost) {
  host = appHost;
  limitSelect.replaceChildren(
    ...LIMITS.map((value) => {
      const label = String(value);
      return new Option(label, label);
    }),
  );
  enhanceSelect(limitSelect);
  limitSelect.addEventListener("change", () => {
    const limit = Number(limitSelect.value);
    host!.settings().historyLimit = LIMITS.find((allowed) => limit <= allowed) ?? DEFAULT_LIMIT;
    syncSelect(limitSelect);
    host!.save();
  });

  uploadButton.addEventListener("click", () => fileInput.click());
  fileInput.addEventListener("change", () => {
    const file = fileInput.files?.[0];
    fileInput.value = "";
    if (file) void upload(file);
  });

  for (const event of ["play", "pause", "timeupdate", "loadedmetadata", "durationchange", "ended"]) {
    playback.addEventListener(event, handlePlaybackEvent);
  }

  listen<HistoryEntry[]>("history-changed", (event) => setEntries(event.payload));
}

/** Puts the saved limit into the toolbar (after the settings are loaded) */
export function fillHistory() {
  if (!host) return;
  const saved = Number(host.settings().historyLimit);
  const limit = LIMITS.find((allowed) => saved <= allowed) ?? DEFAULT_LIMIT;
  limitSelect.value = String(limit);
  syncSelect(limitSelect);
}

/** The section was opened: show the entries as they are now */
export function showHistory() {
  void refresh();
}

/** Texts set from code, after a language switch */
export function translateHistory() {
  uploadLabel.textContent = t(converting ? "history.converting" : uploading ? "history.uploading" : "history.upload");
  if (loaded) render();
}

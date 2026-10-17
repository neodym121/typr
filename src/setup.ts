// First-run setup over the whole window: the interface language first, then
// how speech becomes text (a local model or a cloud service), that engine's
// model or key and the language of the speech, and last the hotkey (a click
// on it records new keys), the recording mode (push to talk for a new
// config) and starting with Windows. It edits the app's settings
// object in place and saves as it goes; skipping it counts as done.

import { invoke } from "@tauri-apps/api/core";
import type { ApiKeys } from "./apikeys";
import { enhanceSelect } from "./dropdown";
import { bindDictationHotkey } from "./hotkeys";
import { fillLanguageOptions, getLanguage, t, type Lang, type MessageKey } from "./i18n";
import { enhanceSegmented } from "./segmented";
import {
  BADGES,
  chooseModel,
  downloadModel,
  formatSize,
  localState,
  onLocalChange,
  progressText,
  refreshLocal,
  type DownloadProgress,
} from "./local";

export interface SetupSettings {
  engine: string;
  recognitionLanguage: string;
  /** The engine's key goes with the others (API keys section) */
  apiKeys: ApiKeys;
  groqModel: string;
  assemblyaiModel: string;
  openaiEndpoint: string;
  openaiModel: string;
  localModel: string;
  recordingMode: string;
  checkUpdates: boolean;
  setupDone: boolean;
}

export interface SetupHost {
  settings: () => SetupSettings;
  save: () => Promise<void>;
  /** Applies and saves the interface language */
  setLanguage: (lang: Lang) => void;
  /** The setup closed: put the settings it changed into the form */
  closed: () => void;
}

type Step = "language" | "engine" | "configure" | "done";
type EngineId = "local" | "groq" | "polza" | "assemblyai" | "openai";

const STEPS: Step[] = ["language", "engine", "configure", "done"];
const ENGINES: EngineId[] = ["local", "groq", "polza", "assemblyai", "openai"];
const ENGINE_NAMES: Record<EngineId, string> = {
  local: "",
  groq: "Groq",
  polza: "Polza",
  assemblyai: "AssemblyAI",
  openai: "OpenAI",
};
const ENGINE_ABOUT: Record<EngineId, MessageKey> = {
  local: "setup.engine.localAbout",
  groq: "setup.engine.groqAbout",
  polza: "setup.engine.polzaAbout",
  assemblyai: "setup.engine.assemblyaiAbout",
  openai: "setup.engine.openaiAbout",
};
/** Where each service hands out API keys (links.rs allows these sites) */
const KEY_PAGES: Partial<Record<EngineId, string>> = {
  groq: "https://console.groq.com/keys",
  polza: "https://polza.ai/dashboard",
  assemblyai: "https://www.assemblyai.com/dashboard",
  openai: "https://platform.openai.com/api-keys",
};

const root = document.getElementById("setup")!;
const body = document.getElementById("setup-body")!;
const progressLabel = document.getElementById("setup-progress")!;
const skipButton = document.getElementById("setup-skip")!;
const backButton = document.getElementById("setup-back")!;
const nextButton = document.getElementById("setup-next")!;

let host: SetupHost | undefined;
let step = 0;
/** Start with Windows; on unless the user turns it off here */
let autostart = true;
/** The engine picked on the engine step; saved when the user moves on */
let engineChoice: EngineId = "groq";
let unsubscribe: (() => void) | undefined;

function settings(): SetupSettings {
  return host!.settings();
}

function save() {
  host!.save().catch(() => {
    // logged by the host
  });
}

// ── Building blocks ──────────────────────────────────

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  if (className) element.className = className;
  if (text) element.textContent = text;
  return element;
}

function heading(title: string, text: string): HTMLElement[] {
  const h = el("h1", "setup-title", title);
  h.id = "setup-title";
  return [h, el("p", "setup-text", text)];
}

/** A card the user picks one of */
function choice(options: {
  title: string;
  about?: string;
  extra?: string;
  badge?: string;
  selected: boolean;
  disabled?: boolean;
  onPick: () => void;
}): HTMLButtonElement {
  const card = el("button", "setup-choice");
  card.type = "button";
  card.setAttribute("role", "radio");
  card.setAttribute("aria-checked", String(options.selected));
  card.disabled = Boolean(options.disabled);
  const top = el("span", "setup-choice-title");
  top.append(el("span", "", options.title));
  if (options.badge) top.append(el("span", "badge", options.badge));
  card.append(top);
  if (options.about) card.append(el("span", "setup-choice-about", options.about));
  if (options.extra) card.append(el("span", "setup-choice-extra", options.extra));
  card.addEventListener("click", () => {
    if (!card.disabled) options.onPick();
  });
  return card;
}

/** A labelled field row like the settings rows */
function field(label: string, hint: string, control: HTMLElement): HTMLElement {
  const row = el("div", "setting-row");
  const labels = el("div", "setting-label");
  labels.append(el("span", "label-text", label));
  if (hint) labels.append(el("span", "label-hint", hint));
  const holder = el("div", "setting-control");
  holder.append(control);
  row.append(labels, holder);
  return row;
}

function textInput(value: string, placeholder: string, secret: boolean, onChange: (value: string) => void) {
  const input = el("input");
  input.type = secret ? "password" : "text";
  input.value = value;
  input.placeholder = placeholder;
  input.spellcheck = false;
  input.addEventListener("input", () => onChange(input.value.trim()));
  input.addEventListener("change", () => {
    onChange(input.value.trim());
    save();
  });
  return input;
}

/** A <select> that becomes an app-styled dropdown once it is in the page */
function select(options: [string, string][], value: string, onChange: (value: string) => void) {
  const element = el("select");
  options.forEach(([id, label]) => element.append(new Option(label, id)));
  element.value = value;
  element.addEventListener("change", () => {
    onChange(element.value);
    save();
  });
  return element;
}

function switchControl(checked: boolean, onChange: (checked: boolean) => void): HTMLElement {
  const label = el("label", "switch");
  const input = el("input");
  input.type = "checkbox";
  input.setAttribute("role", "switch");
  input.checked = checked;
  input.addEventListener("change", () => onChange(input.checked));
  const track = el("span", "switch-track");
  track.append(el("span", "switch-thumb"));
  label.append(input, track);
  return label;
}

// ── Engines ──────────────────────────────────────────

function currentEngine(): EngineId {
  const engine = settings().engine as EngineId;
  return ENGINES.includes(engine) ? engine : "groq";
}

/** Local when this computer has a GPU it can use, otherwise Groq */
function recommendedEngine(): EngineId {
  const status = localState().status;
  return status?.supported && status.recommended === "vulkan" ? "local" : "groq";
}

function engineTitle(engine: EngineId): string {
  if (engine === "local") return t("setup.engine.local");
  if (engine === "openai") return t("engine.openai");
  return ENGINE_NAMES[engine];
}

// ── Steps ────────────────────────────────────────────

function renderLanguage(): HTMLElement[] {
  const current = getLanguage();
  const list = el("div", "setup-choices setup-choices-row");
  list.setAttribute("role", "radiogroup");
  const languages: [Lang, string, MessageKey][] = [
    ["en", "English", "setup.interfaceEn"],
    ["ru", "Русский", "setup.interfaceRu"],
  ];
  for (const [lang, name, sub] of languages) {
    const card = choice({
      title: name,
      about: t(sub),
      selected: current === lang,
      onPick: () => {
        host!.setLanguage(lang);
        render();
      },
    });
    card.lang = lang;
    list.append(card);
  }
  return [...heading(t("setup.welcome"), t("setup.welcomeText")), list];
}

function renderEngine(): HTMLElement[] {
  const status = localState().status;
  const recommended = recommendedEngine();
  const list = el("div", "setup-choices");
  list.setAttribute("role", "radiogroup");
  for (const engine of ENGINES) {
    const unsupported = engine === "local" && status !== undefined && !status.supported;
    let extra = "";
    if (unsupported) extra = t("local.unsupported");
    else if (engine === "local" && status?.recommended === "cpu") extra = t("setup.engine.localCpu");
    list.append(
      choice({
        title: engineTitle(engine),
        about: t(ENGINE_ABOUT[engine]),
        extra,
        badge: engine === recommended ? t("setup.recommended") : undefined,
        selected: engineChoice === engine,
        disabled: unsupported,
        onPick: () => {
          engineChoice = engine;
          settings().engine = engine;
          save();
          render();
        },
      }),
    );
  }
  return [...heading(t("setup.engineTitle"), t("setup.engineText")), list];
}

/** Model cards of the local step; re-rendered as downloads move on */
function renderLocalModels(container: HTMLElement) {
  const { status, downloads, errors } = localState();
  if (!status) {
    container.replaceChildren();
    return;
  }
  if (!status.supported) {
    container.replaceChildren(el("p", "local-note", t("local.unsupported")));
    return;
  }
  const chosen = settings().localModel;
  container.replaceChildren(
    ...status.models.map((model) => {
      const key = `model:${model.id}`;
      const progress: DownloadProgress | undefined = downloads.get(key);
      const card = choice({
        title: model.name,
        about: t(`local.about.${model.id}` as MessageKey),
        extra: `${t(`local.languages.${model.id}` as MessageKey)} · ${formatSize(model.size)}`,
        badge: BADGES[model.id] ? t(BADGES[model.id]) : undefined,
        selected: model.id === chosen,
        onPick: () => {
          chooseModel(model.id);
          renderLocalModels(container);
        },
      });

      const state = el("span", "setup-choice-state");
      if (progress) {
        const bar = el("span", "progress");
        const fill = el("span", "progress-fill");
        const share = progress.total ? progress.downloaded / progress.total : 0;
        fill.style.width = `${progress.state === "installing" ? 100 : Math.min(100, share * 100)}%`;
        bar.classList.toggle("indeterminate", progress.state === "installing" || !progress.total);
        bar.append(fill);
        state.append(bar, el("span", "progress-text", progressText(progress)));
      } else if (model.installed) {
        state.append(el("span", "setup-done-mark", t("setup.downloaded")));
      } else {
        const download = el("span", "btn btn-secondary btn-compact", t("local.download"));
        download.setAttribute("role", "button");
        download.addEventListener("click", (event) => {
          event.stopPropagation();
          chooseModel(model.id);
          void downloadModel(model.id);
        });
        state.append(download);
      }
      const error = errors.get(key);
      if (error) state.append(el("span", "local-error", t("local.downloadError", { error })));
      card.append(state);
      return card;
    }),
  );
}

function renderConfigure(): HTMLElement[] {
  const engine = currentEngine();
  const s = settings();
  const parts: HTMLElement[] = [];

  if (engine === "local") {
    parts.push(...heading(t("setup.localTitle"), t("setup.localText")));
    const models = el("div", "setup-choices");
    renderLocalModels(models);
    unsubscribe?.();
    unsubscribe = onLocalChange(() => renderLocalModels(models));
    parts.push(models);
  } else {
    const name = ENGINE_NAMES[engine];
    if (engine === "openai") parts.push(...heading(t("setup.openaiTitle"), t("setup.openaiText")));
    else parts.push(...heading(t("setup.keyTitle", { engine: name }), t("setup.keyText", { engine: name })));

    const card = el("div", "card setup-card");
    if (engine === "openai") {
      card.append(
        field(t("engine.endpoint"), t("engine.endpointHint"),
          textInput(s.openaiEndpoint, "https://api.openai.com/v1", false, (v) => (s.openaiEndpoint = v))),
        field(t("engine.modelId"), t("engine.openaiModelHint"),
          textInput(s.openaiModel, "whisper-1", false, (v) => (s.openaiModel = v))),
      );
    }
    const keyHolder = el("div", "setup-key");
    keyHolder.append(
      textInput(s.apiKeys[engine] || "", engine === "groq" ? "gsk_..." : engine === "openai" ? "sk-..." : t("engine.apiKey"), true, (v) => {
        s.apiKeys[engine] = v;
      }),
    );
    const page = KEY_PAGES[engine];
    if (page) {
      const link = el("button", "btn btn-secondary btn-compact", t("setup.getKey"));
      link.type = "button";
      link.addEventListener("click", () => {
        invoke("open_link", { url: page }).catch((err) => console.error("Failed to open the page:", err));
      });
      keyHolder.append(link);
    }
    card.append(field(t("engine.apiKey"), engine === "openai" ? t("engine.openaiKeyHint") : "", keyHolder));

    if (engine === "groq") {
      card.append(
        field(t("engine.model"), t("engine.modelHint"),
          select(
            [["whisper-large-v3-turbo", "Whisper Large v3 Turbo"], ["whisper-large-v3", "Whisper Large v3"]],
            s.groqModel || "whisper-large-v3-turbo",
            (v) => (s.groqModel = v),
          )),
      );
    }
    if (engine === "assemblyai") {
      card.append(
        field(t("engine.model"), t("setup.assemblyaiRussian"),
          select(
            [["universal-3-5-pro", "Universal-3.5 Pro"], ["universal-2", "Universal-2"]],
            s.assemblyaiModel || "universal-3-5-pro",
            (v) => (s.assemblyaiModel = v),
          )),
      );
    }
    parts.push(card);
  }

  // The language of the speech, for every engine
  const language = el("select");
  fillLanguageOptions(language);
  language.value = s.recognitionLanguage || "";
  language.addEventListener("change", () => {
    s.recognitionLanguage = language.value;
    save();
  });
  const languageCard = el("div", "card setup-card");
  languageCard.append(field(t("engine.recognitionLanguage"), t("engine.recognitionLanguageHint"), language));
  parts.push(languageCard);
  return parts;
}

function renderDone(): HTMLElement[] {
  const s = settings();
  const ptt = s.recordingMode === "push-to-talk";
  const parts = heading(t("setup.doneTitle"), t(ptt ? "setup.donePtt" : "setup.doneToggle"));

  // A click records new keys right here, as in Settings → Hotkeys
  const hotkey = el("div", "setup-hotkey");
  const keys = el("button", "hotkey-record-btn");
  keys.type = "button";
  keys.title = t("recording.hotkeyChange");
  const keysText = el("kbd");
  keys.append(keysText);
  const keysHint = el("span", "label-hint");
  hotkey.append(keys, keysHint);
  bindDictationHotkey(keys, keysText, keysHint, "setup.hotkeyHint");

  // Switched in place, so the thumb slides over instead of the step being rebuilt
  const mode = el("div", "segmented");
  for (const [id, label] of [["toggle", "recording.toggle"], ["push-to-talk", "recording.ptt"]] as const) {
    const button = el("button", `segment${s.recordingMode === id ? " active" : ""}`, t(label));
    button.type = "button";
    button.addEventListener("click", () => {
      s.recordingMode = id;
      save();
      mode.querySelectorAll(".segment").forEach((b) => b.classList.toggle("active", b === button));
      parts[1].textContent = t(id === "push-to-talk" ? "setup.donePtt" : "setup.doneToggle");
    });
    mode.append(button);
  }
  enhanceSegmented(mode);

  const card = el("div", "card setup-card");
  card.append(
    field(t("recording.mode"), t("recording.modeHint"), mode),
    field(t("general.autostart"), t("general.autostartHint"), switchControl(autostart, (on) => (autostart = on))),
    field(t("general.updates"), t("general.updatesHint"), switchControl(s.checkUpdates, (on) => {
      s.checkUpdates = on;
      save();
    })),
  );
  return [...parts, hotkey, card];
}

function render() {
  if (!host) return;
  const current = STEPS[step];
  if (current !== "configure") {
    unsubscribe?.();
    unsubscribe = undefined;
  }
  const parts =
    current === "language" ? renderLanguage()
    : current === "engine" ? renderEngine()
    : current === "configure" ? renderConfigure()
    : renderDone();
  body.replaceChildren(...parts);
  body.querySelectorAll("select").forEach((element) => enhanceSelect(element));
  body.scrollTop = 0;

  progressLabel.textContent = t("setup.step", { n: step + 1, total: STEPS.length });
  backButton.classList.toggle("invisible", step === 0);
  nextButton.textContent = t(step === STEPS.length - 1 ? "setup.finish" : "setup.next");
}

// ── Open and close ───────────────────────────────────

async function close(finished: boolean) {
  const s = settings();
  s.setupDone = true;
  try {
    await host!.save();
  } catch {
    // logged by the host; the setup shows again next time
  }
  if (finished) {
    await invoke("set_autostart", { enabled: autostart }).catch((err) =>
      console.error("Failed to change starting with Windows:", err),
    );
  }
  unsubscribe?.();
  unsubscribe = undefined;
  root.classList.add("hidden");
  host!.closed();
}

export function initSetup(appHost: SetupHost) {
  host = appHost;
  nextButton.addEventListener("click", () => {
    if (step === STEPS.length - 1) {
      void close(true);
      return;
    }
    // The preselected engine counts once the user moves on from its step;
    // keys typed but not yet "changed" are saved too
    if (STEPS[step] === "engine") settings().engine = engineChoice;
    save();
    step += 1;
    render();
  });
  backButton.addEventListener("click", () => {
    if (step === 0) return;
    step -= 1;
    render();
  });
  skipButton.addEventListener("click", () => void close(false));
}

/** Shows the setup from its first step. On the first run the engine step
 *  preselects the engine that suits this computer. */
export async function openSetup() {
  if (!host) return;
  await refreshLocal();
  const s = settings();
  engineChoice = s.setupDone ? currentEngine() : recommendedEngine();
  autostart = !s.setupDone || (await invoke<boolean>("get_autostart").catch(() => false));
  step = 0;
  render();
  root.classList.remove("hidden");
}

/** Texts set from code, after a language switch. */
export function translateSetup() {
  if (host && !root.classList.contains("hidden")) render();
}

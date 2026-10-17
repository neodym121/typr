// Post-processing section: the switch, translation and its language, the
// provider and model (modelpicker.ts) and the style presets. The settings live
// in the app's settings object; this module edits them in place and asks the
// host to save. The key comes from Settings → API keys.

import { t, type MessageKey } from "./i18n";
import { createModelPicker, type ModelPicker, type ProviderConfig, type ProviderSettings } from "./modelpicker";

export type PostProviderSettings = ProviderSettings;

export interface PostProcessSettings extends ProviderConfig {
  enabled: boolean;
  /** "chill" | "proper" | "custom" */
  preset: string;
  customPrompt: string;
  /** The same model translates every dictation into `translateTo`, with
   *  post-processing off too (translate.rs, postprocess.rs) */
  translate: boolean;
  translateTo: string;
}

type Preset = "chill" | "proper" | "custom";

export interface PostProcessHost {
  settings: () => { postProcess: PostProcessSettings };
  save: () => void;
}

const PRESETS: Preset[] = ["chill", "proper", "custom"];

/** Languages to translate into (TranslationLanguage in settings.rs), each
 *  with the Twemoji file of its flag in src/flags */
const LANGUAGES: [code: string, flag: string][] = [
  ["en", "1f1ec-1f1e7"],
  ["ru", "1f1f7-1f1fa"],
  ["uk", "1f1fa-1f1e6"],
  ["es", "1f1ea-1f1f8"],
  ["fr", "1f1eb-1f1f7"],
  ["de", "1f1e9-1f1ea"],
  ["zh", "1f1e8-1f1f3"],
  ["ja", "1f1ef-1f1f5"],
  ["hi", "1f1ee-1f1f3"],
  ["he", "1f1ee-1f1f1"],
  ["fa", "1f1ee-1f1f7"],
];

// Twemoji flags (src/flags, CC-BY 4.0), bundled like the icons
const FLAGS = import.meta.glob<string>("./flags/*.svg", { query: "?raw", import: "default", eager: true });

const enabledToggle = document.getElementById("post-enabled") as HTMLInputElement;
const translateToggle = document.getElementById("post-translate") as HTMLInputElement;
const languageList = document.getElementById("post-languages")!;
const presetButtons = document.querySelectorAll<HTMLButtonElement>("#post-presets .segment");
const presetHint = document.getElementById("post-preset-hint")!;
const customRow = document.getElementById("post-custom-row")!;
const customPrompt = document.getElementById("post-custom-prompt") as HTMLTextAreaElement;

let host: PostProcessHost | undefined;
let picker: ModelPicker | undefined;
let promptTimer: number | undefined;

function settings(): PostProcessSettings {
  return host!.settings().postProcess;
}

function currentPreset(): Preset {
  const preset = settings().preset as Preset;
  return PRESETS.includes(preset) ? preset : "proper";
}

function currentLanguage(): string {
  const code = settings().translateTo;
  return LANGUAGES.some(([id]) => id === code) ? code : "en";
}

function flag(file: string): HTMLElement {
  const holder = document.createElement("span");
  holder.className = "language-flag";
  holder.setAttribute("aria-hidden", "true");
  holder.innerHTML = FLAGS[`./flags/${file}.svg`] ?? "";
  return holder;
}

/** The language buttons, shown while translation is on */
function renderLanguages() {
  const chosen = currentLanguage();
  languageList.setAttribute("aria-label", t("post.translateTo"));
  languageList.replaceChildren(
    ...LANGUAGES.map(([code, file]) => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "language-option";
      button.dataset.lang = code;
      button.setAttribute("role", "radio");
      button.setAttribute("aria-checked", String(code === chosen));
      const name = document.createElement("span");
      name.textContent = t(`post.lang.${code}` as MessageKey);
      button.append(flag(file), name);
      button.addEventListener("click", () => chooseLanguage(code));
      return button;
    }),
  );
  languageList.classList.toggle("hidden", !settings().translate);
}

function chooseLanguage(code: string) {
  settings().translateTo = code;
  host!.save();
  languageList.querySelectorAll<HTMLElement>(".language-option").forEach((button) => {
    button.setAttribute("aria-checked", String(button.dataset.lang === code));
  });
}

function renderPreset() {
  const preset = currentPreset();
  presetButtons.forEach((b) => b.classList.toggle("active", b.dataset.preset === preset));
  presetHint.textContent = t(`post.${preset}Hint` as MessageKey);
  customRow.classList.toggle("hidden", preset !== "custom");
}

// ── Public ───────────────────────────────────────────

export function initPostProcess(appHost: PostProcessHost) {
  host = appHost;
  picker = createModelPicker({
    prefix: "post",
    config: settings,
    save: () => host!.save(),
  });

  enabledToggle.addEventListener("change", () => {
    settings().enabled = enabledToggle.checked;
    host!.save();
  });

  translateToggle.addEventListener("change", () => {
    settings().translate = translateToggle.checked;
    settings().translateTo = currentLanguage();
    host!.save();
    languageList.classList.toggle("hidden", !translateToggle.checked);
  });

  presetButtons.forEach((button) => {
    button.addEventListener("click", () => {
      settings().preset = button.dataset.preset || "proper";
      host!.save();
      renderPreset();
    });
  });

  customPrompt.addEventListener("input", () => {
    settings().customPrompt = customPrompt.value;
    window.clearTimeout(promptTimer);
    promptTimer = window.setTimeout(() => host!.save(), 800);
  });
  customPrompt.addEventListener("change", () => {
    window.clearTimeout(promptTimer);
    settings().customPrompt = customPrompt.value;
    host!.save();
  });
}

/** Puts the saved settings into the form (after they are loaded). */
export function fillPostProcess() {
  enabledToggle.checked = Boolean(settings().enabled);
  translateToggle.checked = Boolean(settings().translate);
  renderLanguages();
  customPrompt.value = settings().customPrompt || "";
  picker?.fill();
  renderPreset();
}

/** The section was opened: load the model list if it isn't there yet. */
export function showPostProcess() {
  picker?.show();
}

/** Texts set from code, after a language switch. */
export function translatePostProcess() {
  if (!host) return;
  picker?.translate();
  renderPreset();
  renderLanguages();
}

// Post-processing section: the switch, the provider and model (modelpicker.ts)
// and the style presets. The settings live in the app's settings object; this
// module edits them in place and asks the host to save.

import { t, type MessageKey } from "./i18n";
import { createModelPicker, type ModelPicker, type Provider, type ProviderConfig, type ProviderSettings } from "./modelpicker";

export type PostProviderSettings = ProviderSettings;

export interface PostProcessSettings extends ProviderConfig {
  enabled: boolean;
  /** "chill" | "proper" | "custom" */
  preset: string;
  customPrompt: string;
}

type Preset = "chill" | "proper" | "custom";

export interface PostProcessHost {
  settings: () => { postProcess: PostProcessSettings; groqApiKey: string; polzaApiKey: string };
  save: () => void;
}

const PRESETS: Preset[] = ["chill", "proper", "custom"];
const KEY_HINTS: Record<Provider, MessageKey> = {
  gemini: "post.keyHintGemini",
  openrouter: "post.keyHintOpenrouter",
  groq: "post.keyHintEngine",
  polza: "post.keyHintEngine",
};

const enabledToggle = document.getElementById("post-enabled") as HTMLInputElement;
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

/** Groq and Polza can use the key from Engine */
export function engineKey(all: { groqApiKey: string; polzaApiKey: string }, provider: Provider): string {
  if (provider === "groq") return (all.groqApiKey || "").trim();
  if (provider === "polza") return (all.polzaApiKey || "").trim();
  return "";
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
    section: "post",
    config: settings,
    fallbackKey: (provider) => engineKey(host!.settings(), provider),
    keyHint: (provider) => KEY_HINTS[provider],
    save: () => host!.save(),
  });

  enabledToggle.addEventListener("change", () => {
    settings().enabled = enabledToggle.checked;
    host!.save();
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
}

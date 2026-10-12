// Quick ask section: the switch, the model that answers (modelpicker.ts) and
// web search through Exa. The answer window itself is src/ask.ts.

import { type MessageKey } from "./i18n";
import { createModelPicker, type ModelPicker, type Provider, type ProviderConfig } from "./modelpicker";
import { engineKey, type PostProcessSettings } from "./postprocess";

export interface QuickAskSettings extends ProviderConfig {
  enabled: boolean;
  /** The model may search the web with Exa */
  webSearch: boolean;
  exaApiKey: string;
}

export interface QuickAskHost {
  settings: () => {
    quickAsk: QuickAskSettings;
    postProcess: PostProcessSettings;
    groqApiKey: string;
    polzaApiKey: string;
  };
  save: () => void;
}

const KEY_HINTS: Record<Provider, MessageKey> = {
  gemini: "ask.keyHintGemini",
  openrouter: "ask.keyHintOpenrouter",
  groq: "ask.keyHintEngine",
  polza: "ask.keyHintEngine",
};

const enabledToggle = document.getElementById("ask-enabled") as HTMLInputElement;
const searchToggle = document.getElementById("ask-search") as HTMLInputElement;
const exaRow = document.getElementById("ask-exa-row")!;
const exaKey = document.getElementById("ask-exa-key") as HTMLInputElement;

let host: QuickAskHost | undefined;
let picker: ModelPicker | undefined;

/** An empty key falls back to the one from Post-processing, then Engine */
function fallbackKey(provider: Provider): string {
  const all = host!.settings();
  return (all.postProcess[provider].apiKey || "").trim() || engineKey(all, provider);
}

export function initQuickAsk(appHost: QuickAskHost) {
  host = appHost;
  picker = createModelPicker({
    prefix: "ask",
    section: "ask",
    config: () => host!.settings().quickAsk,
    fallbackKey,
    keyHint: (provider) => KEY_HINTS[provider],
    save: () => host!.save(),
  });

  enabledToggle.addEventListener("change", () => {
    host!.settings().quickAsk.enabled = enabledToggle.checked;
    host!.save();
  });

  searchToggle.addEventListener("change", () => {
    host!.settings().quickAsk.webSearch = searchToggle.checked;
    showExaKey();
    host!.save();
  });

  exaKey.addEventListener("change", () => {
    host!.settings().quickAsk.exaApiKey = exaKey.value.trim();
    host!.save();
  });
}

/** The key only matters while search is on */
function showExaKey() {
  exaRow.classList.toggle("hidden", !searchToggle.checked);
}

/** Puts the saved settings into the form (after they are loaded). */
export function fillQuickAsk() {
  const quickAsk = host!.settings().quickAsk;
  enabledToggle.checked = Boolean(quickAsk.enabled);
  searchToggle.checked = Boolean(quickAsk.webSearch);
  exaKey.value = quickAsk.exaApiKey || "";
  showExaKey();
  picker?.fill();
}

/** The section was opened: load the model list if it isn't there yet. */
export function showQuickAsk() {
  picker?.show();
}

/** Texts set from code, after a language switch. */
export function translateQuickAsk() {
  picker?.translate();
}

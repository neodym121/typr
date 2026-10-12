// Quick ask section: the switch and the model that answers (modelpicker.ts).
// The answer window itself is src/ask.ts.

import { type MessageKey } from "./i18n";
import { createModelPicker, type ModelPicker, type Provider, type ProviderConfig } from "./modelpicker";
import { engineKey, type PostProcessSettings } from "./postprocess";

export interface QuickAskSettings extends ProviderConfig {
  enabled: boolean;
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
}

/** Puts the saved settings into the form (after they are loaded). */
export function fillQuickAsk() {
  enabledToggle.checked = Boolean(host!.settings().quickAsk.enabled);
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

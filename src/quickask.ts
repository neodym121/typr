// Quick ask section: its switch (with a hint naming its own hotkey), the
// model that answers (modelpicker.ts) and web search through Exa. The keys
// come from the API keys section; the window itself is drawn by the backend
// (src-tauri/src/ask).

import { formatHotkey, t } from "./i18n";
import { createModelPicker, type ModelPicker, type ProviderConfig } from "./modelpicker";

export interface QuickAskSettings extends ProviderConfig {
  enabled: boolean;
  /** The model may search the web with Exa */
  webSearch: boolean;
}

export interface QuickAskHost {
  settings: () => { quickAsk: QuickAskSettings; askHotkey: string };
  save: () => void;
  /** Quick ask was switched on or off: its hotkey's hint follows */
  enabledChanged: () => void;
  /** Opens Settings on the Hotkeys tab */
  showHotkeys: () => void;
}

const enabledToggle = document.getElementById("ask-enabled") as HTMLInputElement;
const enabledHint = document.getElementById("ask-enabled-hint")!;
const searchToggle = document.getElementById("ask-search") as HTMLInputElement;

let host: QuickAskHost | undefined;
let picker: ModelPicker | undefined;

export function initQuickAsk(appHost: QuickAskHost) {
  host = appHost;
  picker = createModelPicker({
    prefix: "ask",
    config: () => host!.settings().quickAsk,
    save: () => host!.save(),
  });

  enabledToggle.addEventListener("change", () => {
    host!.settings().quickAsk.enabled = enabledToggle.checked;
    host!.save();
    host!.enabledChanged();
  });

  // The hotkey named in the hint leads to where it is set
  enabledHint.addEventListener("click", (event) => {
    if ((event.target as Element).closest("kbd")) host!.showHotkeys();
  });

  searchToggle.addEventListener("change", () => {
    host!.settings().quickAsk.webSearch = searchToggle.checked;
    host!.save();
  });
}

/** "… {hotkey} opens the window …", with the hotkey as a key cap */
export function renderAskHint() {
  if (!host) return;
  const [before, after = ""] = t("ask.enabledHint").split("{hotkey}");
  const key = document.createElement("kbd");
  key.className = "inline-kbd";
  key.textContent = formatHotkey(host.settings().askHotkey || "Ctrl+Alt");
  enabledHint.replaceChildren(before, key, after);
}

/** Puts the saved settings into the form (after they are loaded). */
export function fillQuickAsk() {
  const quickAsk = host!.settings().quickAsk;
  enabledToggle.checked = Boolean(quickAsk.enabled);
  searchToggle.checked = Boolean(quickAsk.webSearch);
  renderAskHint();
  picker?.fill();
}

/** The section was opened: load the model list if it isn't there yet. */
export function showQuickAsk() {
  picker?.show();
}

/** Texts set from code, after a language switch. */
export function translateQuickAsk() {
  picker?.translate();
  renderAskHint();
}

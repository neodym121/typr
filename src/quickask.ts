// Quick ask section: the quick ask and quick chat switches, the model that
// answers (modelpicker.ts) and web search through Exa. The keys come from
// the API keys section; the window itself is src/ask.ts.

import { formatHotkey, t } from "./i18n";
import { createModelPicker, type ModelPicker, type ProviderConfig } from "./modelpicker";

export interface QuickAskSettings extends ProviderConfig {
  enabled: boolean;
  /** Quick chat: the chat hotkey opens the window as a chat */
  chat: boolean;
  /** The model may search the web with Exa */
  webSearch: boolean;
}

export interface QuickAskHost {
  settings: () => { quickAsk: QuickAskSettings; chatHotkey: string };
  save: () => void;
  /** Quick chat was switched on or off */
  chatChanged: () => void;
  /** Opens Settings on the Hotkeys tab */
  showHotkeys: () => void;
}

const enabledToggle = document.getElementById("ask-enabled") as HTMLInputElement;
const chatToggle = document.getElementById("ask-chat") as HTMLInputElement;
const chatHint = document.getElementById("ask-chat-hint")!;
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
  });

  chatToggle.addEventListener("change", () => {
    host!.settings().quickAsk.chat = chatToggle.checked;
    host!.save();
    host!.chatChanged();
  });

  // The hotkey named in the hint leads to where it is set
  chatHint.addEventListener("click", (event) => {
    if ((event.target as Element).closest("kbd")) host!.showHotkeys();
  });

  searchToggle.addEventListener("change", () => {
    host!.settings().quickAsk.webSearch = searchToggle.checked;
    host!.save();
  });
}

/** "{hotkey} opens a chat…", with the hotkey as a key cap */
export function renderChatHint() {
  if (!host) return;
  const [before, after = ""] = t("ask.chatHint").split("{hotkey}");
  const key = document.createElement("kbd");
  key.className = "inline-kbd";
  key.textContent = formatHotkey(host.settings().chatHotkey || "Ctrl+Alt");
  chatHint.replaceChildren(before, key, after);
}

/** Puts the saved settings into the form (after they are loaded). */
export function fillQuickAsk() {
  const quickAsk = host!.settings().quickAsk;
  enabledToggle.checked = Boolean(quickAsk.enabled);
  chatToggle.checked = Boolean(quickAsk.chat);
  searchToggle.checked = Boolean(quickAsk.webSearch);
  renderChatHint();
  picker?.fill();
}

/** The section was opened: load the model list if it isn't there yet. */
export function showQuickAsk() {
  picker?.show();
}

/** Texts set from code, after a language switch. */
export function translateQuickAsk() {
  picker?.translate();
  renderChatHint();
}

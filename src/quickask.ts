// Quick ask section: its switch (with a hint naming its own hotkey), the
// model that answers (modelpicker.ts), web search through Exa and Reasoning
// (how much the model thinks; the request itself is built by the backend,
// ask/stream.rs). The keys come from the API keys section; the window itself
// is drawn by the backend (src-tauri/src/ask).

import { formatHotkey, t } from "./i18n";
import { enhanceSelect, syncSelect } from "./dropdown";
import { createModelPicker, type ModelPicker, type ProviderConfig } from "./modelpicker";

export interface QuickAskSettings extends ProviderConfig {
  enabled: boolean;
  /** The model may search the web with Exa */
  webSearch: boolean;
  /** Ask the model for a set amount of reasoning (Reasoning) */
  reasoning: boolean;
  /** How much reasoning the request asks for (settings.rs `ReasoningMode`) */
  reasoningMode: string;
  /** The parameter of the custom reasoning mode */
  reasoningCustom: string;
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
const reasoningToggle = document.getElementById("ask-reasoning") as HTMLInputElement;
const reasoningMode = document.getElementById("ask-reasoning-mode") as HTMLSelectElement;
const customInput = document.getElementById("ask-reasoning-custom-input") as HTMLInputElement;
const customEdit = document.getElementById("ask-reasoning-edit")!;

/** The value ids of the reasoning presets (settings.rs `ReasoningMode`) */
const REASONING_MODES = ["none", "low", "medium", "high", "custom"];

let host: QuickAskHost | undefined;
let picker: ModelPicker | undefined;
let editingCustom = false;

/** The parameter select and its custom option follow the settings. */
function renderReasoning() {
  if (!host) return;
  const config = host.settings().quickAsk;
  reasoningMode.value = REASONING_MODES.includes(config.reasoningMode) ? config.reasoningMode : "none";
  // The custom option carries the parameter as its text, so the trigger and
  // its list row show what will be sent
  const option = reasoningMode.querySelector<HTMLOptionElement>('option[value="custom"]')!;
  option.text = config.reasoningCustom.trim() || "custom";
  syncSelect(reasoningMode);
  const custom = reasoningMode.value === "custom";
  reasoningMode.closest(".dropdown")!.classList.toggle("editing", custom && editingCustom);
  customInput.classList.toggle("hidden", !(custom && editingCustom));
  customEdit.classList.toggle("hidden", !custom || editingCustom);
}

/** Marks the custom list row with the parameter's grey "custom" label while
 *  another preset is chosen. The list renders on every open. */
function markCustomRow(list: HTMLElement) {
  const value = host?.settings().quickAsk.reasoningCustom.trim();
  if (!value || reasoningMode.value === "custom") return;
  const row = list.lastElementChild as HTMLElement | null;
  if (!row || row.querySelector(".dropdown-option-hint")) return;
  const hint = document.createElement("span");
  hint.className = "dropdown-option-hint";
  hint.textContent = "custom";
  row.append(hint);
}

function startCustomEdit() {
  if (editingCustom) return;
  editingCustom = true;
  customInput.value = host!.settings().quickAsk.reasoningCustom;
  renderReasoning();
  customInput.focus();
  customInput.select();
}

function commitCustomEdit() {
  if (!editingCustom) return;
  editingCustom = false;
  host!.settings().quickAsk.reasoningCustom = customInput.value.trim();
  host!.save();
  renderReasoning();
}

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

  reasoningToggle.addEventListener("change", () => {
    host!.settings().quickAsk.reasoning = reasoningToggle.checked;
    host!.save();
  });

  enhanceSelect(reasoningMode);
  // The parameter field lives inside the dropdown, taking the trigger's
  // place while it is edited
  const dropdownRoot = reasoningMode.closest(".dropdown")!;
  dropdownRoot.append(customInput);

  // The list renders on every open: the custom row gets its grey label then
  const list = dropdownRoot.querySelector<HTMLElement>(".dropdown-list")!;
  new MutationObserver(() => markCustomRow(list)).observe(list, { childList: true });

  reasoningMode.addEventListener("change", () => {
    host!.settings().quickAsk.reasoningMode = reasoningMode.value;
    host!.save();
    // A custom parameter without a value opens its field for typing right
    // away; with one, the pencil brings the field back
    if (reasoningMode.value === "custom" && !host!.settings().quickAsk.reasoningCustom.trim()) startCustomEdit();
    else renderReasoning();
  });

  customEdit.addEventListener("click", () => startCustomEdit());

  customInput.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      commitCustomEdit();
      reasoningMode
        .closest(".dropdown")!
        .querySelector<HTMLButtonElement>(".dropdown-trigger")!
        .focus();
    } else if (event.key === "Escape") {
      // Cancels the edit; the Settings window takes Escape otherwise
      event.stopPropagation();
      editingCustom = false;
      renderReasoning();
      reasoningMode
        .closest(".dropdown")!
        .querySelector<HTMLButtonElement>(".dropdown-trigger")!
        .focus();
    }
  });

  // A click away from the field saves what it holds
  customInput.addEventListener("blur", () => commitCustomEdit());
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
  reasoningToggle.checked = Boolean(quickAsk.reasoning);
  editingCustom = false;
  renderReasoning();
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

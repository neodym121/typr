// Quick ask section: its switch (with a hint naming its own hotkey), the
// model that answers (modelpicker.ts), web search through Exa and NoThink
// (asking the model not to reason; the request itself is built by the
// backend, ask/stream.rs). Its tester sends the same parameter and reports
// whether the response contained reasoning. The keys come from the API keys
// section; the window itself is drawn by the backend (src-tauri/src/ask).

import { invoke } from "@tauri-apps/api/core";
import { keyOf } from "./apikeys";
import { enhanceSelect, syncSelect } from "./dropdown";
import { formatHotkey, t } from "./i18n";
import { createModelPicker, type ModelPicker, type Provider, type ProviderConfig } from "./modelpicker";

export interface QuickAskSettings extends ProviderConfig {
  enabled: boolean;
  /** The model may search the web with Exa */
  webSearch: boolean;
  /** Ask the model not to reason (NoThink) */
  noThink: boolean;
  /** How the NoThink request is passed (settings.rs `NoThinkMode`) */
  noThinkMode: string;
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
const noThinkToggle = document.getElementById("ask-nothink") as HTMLInputElement;
const noThinkMode = document.getElementById("ask-nothink-mode") as HTMLSelectElement;
const noThinkTest = document.getElementById("ask-nothink-test") as HTMLButtonElement;
const noThinkTestTarget = document.getElementById("ask-nothink-test-target")!;
const noThinkTestResult = document.getElementById("ask-nothink-test-result")!;

/** The value ids of the NoThink dropdown (settings.rs `NoThinkMode`) */
const NOTHINK_MODES = ["none", "low", "disabled"];
const PROVIDER_NAMES: Record<Provider, string> = {
  gemini: "Gemini",
  mistral: "Mistral",
  groq: "Groq",
  polza: "Polza",
};

type TestResult =
  | { kind: "none" }
  | { kind: "reasoning" | "noReasoning" }
  | { kind: "error"; error: string };

let host: QuickAskHost | undefined;
let picker: ModelPicker | undefined;
let testingNoThink = false;
let testRevision = 0;
let testResult: TestResult = { kind: "none" };

function currentProvider(): Provider {
  const provider = host!.settings().quickAsk.provider as Provider;
  return PROVIDER_NAMES[provider] ? provider : "gemini";
}

/** The exact combination the next test will use, and its latest result. */
function renderNoThinkTester() {
  if (!host) return;
  const config = host.settings().quickAsk;
  const provider = currentProvider();
  const model = config[provider].model.trim() || t("post.modelNone");
  const subProvider = provider === "polza" ? config.polza.providerId?.trim() || t("ask.noThinkAutoProvider") : "";
  const target = [PROVIDER_NAMES[provider], model, subProvider].filter(Boolean).join(" · ");
  noThinkTestTarget.textContent = target;
  noThinkTestTarget.title = target;

  noThinkTest.disabled = testingNoThink;
  noThinkTest.textContent = t(testingNoThink ? "ask.noThinkTesting" : "ask.noThinkTestButton");
  noThinkTestResult.classList.toggle("reasoning", testResult.kind === "reasoning" || testResult.kind === "error");
  noThinkTestResult.classList.toggle("no-reasoning", testResult.kind === "noReasoning");
  switch (testResult.kind) {
    case "none":
      noThinkTestResult.textContent = "";
      break;
    case "reasoning":
      noThinkTestResult.textContent = t("ask.noThinkReasoning");
      break;
    case "noReasoning":
      noThinkTestResult.textContent = t("ask.noThinkNoReasoning");
      break;
    case "error":
      noThinkTestResult.textContent = t("ask.noThinkTestFailed", { error: testResult.error });
      break;
  }
}

/** A result belongs only to the combination that produced it. */
function resetNoThinkTest() {
  testRevision += 1;
  testResult = { kind: "none" };
  renderNoThinkTester();
}

async function runNoThinkTest() {
  if (testingNoThink) return;
  const config = host!.settings().quickAsk;
  const provider = currentProvider();
  const revision = ++testRevision;
  testingNoThink = true;
  testResult = { kind: "none" };
  renderNoThinkTester();
  try {
    const reasoning = await invoke<boolean>("test_no_think", {
      provider,
      apiKey: keyOf(provider),
      model: config[provider].model,
      subProvider: provider === "polza" ? config.polza.providerId || "" : "",
      mode: NOTHINK_MODES.includes(config.noThinkMode) ? config.noThinkMode : "none",
    });
    if (revision === testRevision) testResult = { kind: reasoning ? "reasoning" : "noReasoning" };
  } catch (error) {
    if (revision === testRevision) testResult = { kind: "error", error: String(error) };
  } finally {
    testingNoThink = false;
    renderNoThinkTester();
  }
}

export function initQuickAsk(appHost: QuickAskHost) {
  host = appHost;
  picker = createModelPicker({
    prefix: "ask",
    config: () => host!.settings().quickAsk,
    save: () => host!.save(),
    changed: resetNoThinkTest,
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

  enhanceSelect(noThinkMode);

  noThinkToggle.addEventListener("change", () => {
    host!.settings().quickAsk.noThink = noThinkToggle.checked;
    host!.save();
  });

  noThinkMode.addEventListener("change", () => {
    host!.settings().quickAsk.noThinkMode = noThinkMode.value;
    host!.save();
    resetNoThinkTest();
  });

  noThinkTest.addEventListener("click", () => void runNoThinkTest());
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
  noThinkToggle.checked = Boolean(quickAsk.noThink);
  noThinkMode.value = NOTHINK_MODES.includes(quickAsk.noThinkMode) ? quickAsk.noThinkMode : "none";
  syncSelect(noThinkMode);
  renderAskHint();
  picker?.fill();
  resetNoThinkTest();
}

/** The section was opened: load the model list if it isn't there yet. */
export function showQuickAsk() {
  picker?.show();
}

/** Texts set from code, after a language switch. */
export function translateQuickAsk() {
  picker?.translate();
  renderAskHint();
  renderNoThinkTester();
}

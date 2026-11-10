// Select&Edit section: its switch (with a hint naming its hotkey), the
// provider and model that rewrite the selected text (modelpicker.ts) and
// NoThink (asking the model not to reason; the request itself is built by
// the backend, postprocess.rs). Its tester sends the same parameter and
// reports whether the response contained reasoning. The keys come from the
// API keys section; the recording and the edit run in the backend
// (src-tauri/src/recorder.rs, postprocess.rs).

import { invoke } from "@tauri-apps/api/core";
import { keyOf } from "./apikeys";
import { enhanceSelect, syncSelect } from "./dropdown";
import { formatHotkey, t } from "./i18n";
import { createModelPicker, type ModelPicker, type Provider, type ProviderConfig } from "./modelpicker";

export interface SelectEditSettings extends ProviderConfig {
  enabled: boolean;
  /** Ask the model not to reason (NoThink) */
  noThink: boolean;
  /** How the NoThink request is passed (settings.rs `NoThinkMode`) */
  noThinkMode: string;
}

export interface SelectEditHost {
  settings: () => { selectEdit: SelectEditSettings; selectEditHotkey: string };
  save: () => void;
  /** Select&Edit was switched on or off: its hotkey's hint follows */
  enabledChanged: () => void;
  /** Opens Settings on the Hotkeys tab */
  showHotkeys: () => void;
}

const enabledToggle = document.getElementById("se-enabled") as HTMLInputElement;
const enabledHint = document.getElementById("se-enabled-hint")!;
const noThinkToggle = document.getElementById("se-nothink") as HTMLInputElement;
const noThinkMode = document.getElementById("se-nothink-mode") as HTMLSelectElement;
const noThinkTest = document.getElementById("se-nothink-test") as HTMLButtonElement;
const noThinkTestTarget = document.getElementById("se-nothink-test-target")!;
const noThinkTestResult = document.getElementById("se-nothink-test-result")!;

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

let host: SelectEditHost | undefined;
let picker: ModelPicker | undefined;
let testingNoThink = false;
let testRevision = 0;
let testResult: TestResult = { kind: "none" };

function currentProvider(): Provider {
  const provider = host!.settings().selectEdit.provider as Provider;
  return PROVIDER_NAMES[provider] ? provider : "gemini";
}

/** The exact combination the next test will use, and its latest result. */
function renderNoThinkTester() {
  if (!host) return;
  const config = host.settings().selectEdit;
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
  const config = host!.settings().selectEdit;
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

export function initSelectEdit(appHost: SelectEditHost) {
  host = appHost;
  picker = createModelPicker({
    prefix: "se",
    config: () => host!.settings().selectEdit,
    save: () => host!.save(),
    changed: resetNoThinkTest,
  });

  enabledToggle.addEventListener("change", () => {
    host!.settings().selectEdit.enabled = enabledToggle.checked;
    host!.save();
    host!.enabledChanged();
  });

  // The hotkey named in the hint leads to where it is set
  enabledHint.addEventListener("click", (event) => {
    if ((event.target as Element).closest("kbd")) host!.showHotkeys();
  });

  enhanceSelect(noThinkMode);

  noThinkToggle.addEventListener("change", () => {
    host!.settings().selectEdit.noThink = noThinkToggle.checked;
    host!.save();
  });

  noThinkMode.addEventListener("change", () => {
    host!.settings().selectEdit.noThinkMode = noThinkMode.value;
    host!.save();
    resetNoThinkTest();
  });

  noThinkTest.addEventListener("click", () => void runNoThinkTest());
}

/** "… {hotkey} …", with the hotkey as a key cap */
export function renderSelectHint() {
  if (!host) return;
  const [before, after = ""] = t("select.enabledHint").split("{hotkey}");
  const key = document.createElement("kbd");
  key.className = "inline-kbd";
  key.textContent = formatHotkey(host.settings().selectEditHotkey || "Ctrl+Shift+E");
  enabledHint.replaceChildren(before, key, after);
}

/** Puts the saved settings into the form (after they are loaded). */
export function fillSelectEdit() {
  const selectEdit = host!.settings().selectEdit;
  enabledToggle.checked = Boolean(selectEdit.enabled);
  noThinkToggle.checked = Boolean(selectEdit.noThink);
  noThinkMode.value = NOTHINK_MODES.includes(selectEdit.noThinkMode) ? selectEdit.noThinkMode : "none";
  syncSelect(noThinkMode);
  renderSelectHint();
  picker?.fill();
  resetNoThinkTest();
}

/** The section was opened: load the model list if it isn't there yet. */
export function showSelectEdit() {
  picker?.show();
}

/** Texts set from code, after a language switch. */
export function translateSelectEdit() {
  picker?.translate();
  renderSelectHint();
  renderNoThinkTester();
}

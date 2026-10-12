// API keys section: one key per service for the whole app. Engine,
// Post-processing and Quick ask only pick a service and a model; the model
// lists read the key from here (keyOf).

import { t, type MessageKey } from "./i18n";

export type KeyId = "groq" | "gemini" | "openrouter" | "polza" | "assemblyai" | "openai" | "exa";

export type ApiKeys = Record<KeyId, string>;

interface Service {
  id: KeyId;
  name: string;
  /** What the key is used for */
  use: MessageKey;
  placeholder: string;
}

const SERVICES: Service[] = [
  { id: "groq", name: "Groq", use: "keys.useSpeechModels", placeholder: "gsk_…" },
  { id: "gemini", name: "Gemini", use: "keys.useModels", placeholder: "AIza…" },
  { id: "openrouter", name: "OpenRouter", use: "keys.useModels", placeholder: "sk-or-…" },
  { id: "polza", name: "Polza", use: "keys.useSpeechModels", placeholder: "" },
  { id: "assemblyai", name: "AssemblyAI", use: "keys.useSpeech", placeholder: "" },
  { id: "openai", name: "OpenAI", use: "keys.useOpenai", placeholder: "sk-…" },
  { id: "exa", name: "Exa", use: "keys.useSearch", placeholder: "" },
];

export interface KeysHost {
  settings: () => { apiKeys: ApiKeys };
  save: () => void;
}

interface KeyRow {
  service: Service;
  input: HTMLInputElement;
  hint: HTMLElement;
}

const servicesCard = document.getElementById("keys-services")!;
const searchCard = document.getElementById("keys-search")!;

let host: KeysHost | undefined;
const rows = new Map<KeyId, KeyRow>();

/** The key of a service as it is now in the form (maybe not saved yet) */
export function keyOf(id: KeyId): string {
  return (host?.settings().apiKeys?.[id] || "").trim();
}

function buildRow(service: Service): HTMLElement {
  const row = document.createElement("div");
  row.className = "setting-row key-row";

  const labels = document.createElement("div");
  labels.className = "setting-label";
  const label = document.createElement("label");
  label.className = "label-text";
  label.htmlFor = `key-${service.id}`;
  label.textContent = service.name;
  const hint = document.createElement("span");
  hint.className = "label-hint";
  labels.append(label, hint);

  const control = document.createElement("div");
  control.className = "setting-control";
  const input = document.createElement("input");
  input.type = "password";
  input.id = `key-${service.id}`;
  input.spellcheck = false;
  input.autocomplete = "off";
  control.append(input);
  row.append(labels, control);

  input.addEventListener("input", () => {
    host!.settings().apiKeys[service.id] = input.value.trim();
  });
  input.addEventListener("change", () => {
    host!.settings().apiKeys[service.id] = input.value.trim();
    host!.save();
  });

  rows.set(service.id, { service, input, hint });
  return row;
}

/** Texts set from code */
function renderTexts() {
  for (const { service, input, hint } of rows.values()) {
    hint.textContent = t(service.use);
    input.placeholder = service.placeholder || t("engine.apiKey");
  }
}

// ── Public ───────────────────────────────────────────

export function initApiKeys(appHost: KeysHost) {
  host = appHost;
  for (const service of SERVICES) {
    (service.id === "exa" ? searchCard : servicesCard).append(buildRow(service));
  }
  renderTexts();
}

/** Puts the saved keys into the form (after the settings are loaded). */
export function fillApiKeys() {
  const keys = host!.settings().apiKeys;
  for (const [id, { input }] of rows) input.value = keys?.[id] || "";
}

/** Texts set from code, after a language switch. */
export function translateApiKeys() {
  if (host) renderTexts();
}

// Provider and model list with search, for a section that runs a language
// model (Post-processing, Quick ask). The section's settings hold the
// provider and the model per provider; the picker edits them in place and
// asks the host to save. The key comes from Settings → API keys
// (apikeys.ts). Its elements are found by id prefix: #<prefix>-provider,
// #<prefix>-model-list and so on.

import { invoke } from "@tauri-apps/api/core";
import { keyOf } from "./apikeys";
import { enhanceSelect, syncSelect } from "./dropdown";
import { t } from "./i18n";

export interface ProviderSettings {
  model: string;
  /** Polza only: sub-provider that serves the model, empty for automatic */
  providerId?: string;
}

export interface ProviderConfig {
  /** "gemini" | "mistral" | "groq" | "polza" */
  provider: string;
  gemini: ProviderSettings;
  mistral: ProviderSettings;
  groq: ProviderSettings;
  polza: ProviderSettings;
}

export type Provider = "gemini" | "mistral" | "groq" | "polza";

const PROVIDERS: Provider[] = ["gemini", "mistral", "groq", "polza"];

interface ModelInfo {
  id: string;
  name: string;
}

export interface PickerOptions {
  /** Prefix of the element ids */
  prefix: string;
  config: () => ProviderConfig;
  save: () => void;
}

export interface ModelPicker {
  /** Puts the saved settings into the form */
  fill(): void;
  /** The section was opened: loads the model list if it isn't there yet */
  show(): void;
  /** Texts set from code, after a language switch */
  translate(): void;
}

export function createModelPicker(options: PickerOptions): ModelPicker {
  const element = <T extends HTMLElement = HTMLElement>(id: string) =>
    document.getElementById(`${options.prefix}-${id}`) as T;
  const providerSelect = element<HTMLSelectElement>("provider");
  const polzaProviderRow = element("polza-provider-row");
  const polzaProviderInput = element<HTMLInputElement>("polza-provider");
  const modelCurrent = element("model-current");
  const refreshButton = element("model-refresh");
  const searchInput = element<HTMLInputElement>("model-search");
  const modelList = element("model-list");
  const modelStatus = element("model-status");

  /** Loaded model lists, per provider, with the key they were loaded for */
  const loaded = new Map<Provider, { key: string; models: ModelInfo[] }>();
  /** What the model area shows, per provider */
  const views = new Map<Provider, { kind: "needKey" | "loading" | "error" | "ready"; error?: string }>();
  let requestId = 0;

  function config(): ProviderConfig {
    return options.config();
  }

  function currentProvider(): Provider {
    const provider = config().provider as Provider;
    return PROVIDERS.includes(provider) ? provider : "gemini";
  }

  function renderProvider() {
    const provider = currentProvider();
    providerSelect.value = provider;
    syncSelect(providerSelect);
    polzaProviderRow.classList.toggle("hidden", provider !== "polza");
    polzaProviderInput.value = config().polza.providerId || "";
    searchInput.value = "";
    renderModels(true);
  }

  function modelName(provider: Provider, id: string): string {
    return loaded.get(provider)?.models.find((m) => m.id === id)?.name || id;
  }

  function renderCurrentModel() {
    const provider = currentProvider();
    const model = config()[provider].model;
    modelCurrent.textContent = model
      ? t("post.modelChosen", { name: modelName(provider, model) })
      : t("post.modelNone");
  }

  function renderModels(scrollToSelected = false) {
    const provider = currentProvider();
    const view = views.get(provider) ?? { kind: "needKey" };
    const models = view.kind === "ready" ? loaded.get(provider)?.models ?? [] : [];
    const query = searchInput.value.trim().toLowerCase();
    const shown = query
      ? models.filter((m) => m.id.toLowerCase().includes(query) || m.name.toLowerCase().includes(query))
      : models;
    const selected = config()[provider].model;

    modelList.replaceChildren(
      ...shown.map((model) => {
        const item = document.createElement("div");
        item.className = "model-option";
        item.setAttribute("role", "option");
        item.setAttribute("aria-selected", String(model.id === selected));
        item.dataset.id = model.id;
        item.title = model.id;

        const name = document.createElement("span");
        name.className = "model-name";
        name.textContent = model.name;
        item.append(name);
        if (model.name !== model.id) {
          const id = document.createElement("span");
          id.className = "model-id";
          id.textContent = model.id;
          item.append(id);
        }
        item.addEventListener("click", () => chooseModel(model.id));
        return item;
      }),
    );
    modelList.classList.toggle("hidden", shown.length === 0);

    let status = "";
    if (view.kind === "needKey") status = t("post.needKey");
    else if (view.kind === "loading") status = t("post.loading");
    else if (view.kind === "error") status = t("post.loadError", { error: view.error || "" });
    else if (shown.length === 0) status = t("post.nothingFound");
    modelStatus.textContent = status;
    modelStatus.classList.toggle("hidden", !status);
    modelStatus.classList.toggle("error", view.kind === "error");

    renderCurrentModel();

    if (scrollToSelected) {
      const item = modelList.querySelector<HTMLElement>('[aria-selected="true"]');
      modelList.scrollTop = item ? item.offsetTop - modelList.clientHeight / 2 + item.offsetHeight / 2 : 0;
    }
  }

  async function loadModels(force = false) {
    const provider = currentProvider();
    const key = keyOf(provider);
    if (!key) {
      views.set(provider, { kind: "needKey" });
      renderModels();
      return;
    }

    if (!force && loaded.get(provider)?.key === key && views.get(provider)?.kind === "ready") {
      renderModels();
      return;
    }

    const id = ++requestId;
    views.set(provider, { kind: "loading" });
    renderModels();
    try {
      // The key as it is in the form: it may not be saved yet
      const models = await invoke<ModelInfo[]>("list_postprocess_models", { provider, apiKey: key });
      if (id !== requestId) return;
      loaded.set(provider, { key, models });
      views.set(provider, { kind: "ready" });
    } catch (err) {
      if (id !== requestId) return;
      views.set(provider, { kind: "error", error: String(err) });
    }
    if (currentProvider() === provider) renderModels(true);
  }

  function chooseModel(id: string) {
    const provider = currentProvider();
    config()[provider].model = id;
    options.save();
    modelList.querySelectorAll<HTMLElement>(".model-option").forEach((item) => {
      item.setAttribute("aria-selected", String(item.dataset.id === id));
    });
    renderCurrentModel();
  }

  enhanceSelect(providerSelect);

  providerSelect.addEventListener("change", () => {
    config().provider = providerSelect.value;
    options.save();
    renderProvider();
    void loadModels();
  });

  polzaProviderInput.addEventListener("change", () => {
    config().polza.providerId = polzaProviderInput.value.trim();
    options.save();
  });

  refreshButton.addEventListener("click", () => void loadModels(true));
  searchInput.addEventListener("input", () => renderModels());

  return {
    fill: renderProvider,
    show: () => void loadModels(),
    translate() {
      renderModels();
    },
  };
}

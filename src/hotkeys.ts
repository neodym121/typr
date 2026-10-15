// Settings → Hotkeys: the dictation hotkey and the quick chat hotkey (the
// recording mode next to them stays in main.ts). A hotkey is recorded by
// pressing it: 2 or 3 keys, or the middle or a side mouse button. The chat
// one may also be modifiers alone, like Ctrl+Alt, which fire when they are
// let go (modifier_hotkey.rs); the plugin behind the dictation hotkey needs
// a key besides them. While a hotkey is recorded, Typr's own are paused:
// Windows would swallow a registered combination before the page saw it.

import { invoke } from "@tauri-apps/api/core";
import { formatHotkey, formatKey, t, type MessageKey } from "./i18n";

export interface HotkeysHost {
  settings: () => { hotkey: string; chatHotkey: string; quickAsk: { chat: boolean } };
  save: () => Promise<void>;
}

/** Mouse buttons that can be the whole hotkey, by MouseEvent.button (mouse.rs) */
const MOUSE_HOTKEYS: Record<number, string> = { 1: "MouseMiddle", 3: "MouseBack", 4: "MouseForward" };
const MODIFIERS = ["Ctrl", "Alt", "Shift", "Super"];

interface Recorder {
  button: HTMLElement;
  text: HTMLElement;
  hint: HTMLElement;
  /** The hint while not recording */
  idleHint: () => MessageKey;
  prompt: MessageKey;
  promptHint: MessageKey;
  get: () => string;
  set: (hotkey: string) => void;
  modifiersAlone: boolean;
  /** The other hotkey, which this one can't be */
  other: () => string;
  taken: MessageKey;
}

let host: HotkeysHost | undefined;
let recorders: Recorder[] = [];
/** The recorder listening for keys, if any */
let active: Recorder | null = null;

function render(recorder: Recorder) {
  if (active === recorder) return;
  recorder.text.textContent = formatHotkey(recorder.get());
  recorder.hint.textContent = t(recorder.idleHint());
}

function keyIdentifier(e: KeyboardEvent): string {
  if (e.key === "Control") return "Ctrl";
  if (e.key === "Shift") return "Shift";
  if (e.key === "Alt") return "Alt";
  if (e.key === "Meta") return "Super";
  if (e.code === "Space") return "Space";
  if (e.code) return e.code;
  return e.key;
}

/** Modifiers first, in a fixed order, then the keys */
function sortKeys(keys: Set<string>): string {
  return Array.from(keys)
    .sort((a, b) => {
      const aMod = MODIFIERS.indexOf(a);
      const bMod = MODIFIERS.indexOf(b);
      if (aMod !== -1 && bMod !== -1) return aMod - bMod;
      if (aMod !== -1) return -1;
      if (bMod !== -1) return 1;
      return a.localeCompare(b);
    })
    .join("+");
}

function pauseHotkeys(paused: boolean) {
  invoke("pause_hotkeys", { paused }).catch((err) => console.error("Failed to pause the hotkeys:", err));
}

function record(recorder: Recorder) {
  if (active) return;
  active = recorder;
  pauseHotkeys(true);
  const { button, text, hint } = recorder;

  const pressedKeys = new Set<string>();
  const strokeKeys = new Set<string>();
  let messageTimer: number | undefined;

  button.classList.add("recording");
  text.textContent = t(recorder.prompt);
  hint.textContent = t(recorder.promptHint);

  /** Says why that didn't work, then asks again */
  const complain = (message: string) => {
    text.textContent = message;
    window.clearTimeout(messageTimer);
    messageTimer = window.setTimeout(() => {
      if (active === recorder && strokeKeys.size === 0) text.textContent = t(recorder.prompt);
    }, 1200);
  };

  /** Why `hotkey` can't be this one, if it can't */
  const problem = (hotkey: string): string | null => {
    if (!recorder.modifiersAlone && hotkey.split("+").every((key) => MODIFIERS.includes(key))) {
      return t("hotkeys.needKey");
    }
    if (hotkey.toLowerCase() === recorder.other().toLowerCase()) return t(recorder.taken);
    return null;
  };

  const onKeyDown = (e: KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();

    if (e.key === "Escape" && pressedKeys.size === 0) {
      cleanup();
      return;
    }

    const keyName = keyIdentifier(e);
    if (!keyName) return;
    if (pressedKeys.size >= 3 && !pressedKeys.has(keyName)) {
      return; // maximum 3 keys
    }

    pressedKeys.add(keyName);
    strokeKeys.add(keyName);
    window.clearTimeout(messageTimer);
    const shown = Array.from(strokeKeys).map(formatKey);
    text.textContent = shown.join("+") + (strokeKeys.size < 2 ? " + ..." : "");
  };

  // Saves the recorded hotkey; a failed save brings the previous one back
  const saveHotkey = (hotkey: string) => {
    const previous = recorder.get();
    recorder.set(hotkey);
    text.textContent = formatHotkey(hotkey);
    removeListeners();
    host!
      .save()
      .then(() => cleanup())
      .catch(() => {
        recorder.set(previous);
        text.textContent = t("recording.saveError");
        window.setTimeout(() => cleanup(), 1200);
      });
  };

  const onKeyUp = (e: KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();

    const keyName = keyIdentifier(e);
    if (strokeKeys.size >= 2 && strokeKeys.size <= 3) {
      const hotkey = sortKeys(strokeKeys);
      const why = problem(hotkey);
      if (!why) {
        saveHotkey(hotkey);
        return;
      }
      strokeKeys.clear();
      pressedKeys.delete(keyName);
      complain(why);
      return;
    }

    pressedKeys.delete(keyName);
    if (pressedKeys.size === 0 && strokeKeys.size > 0) {
      strokeKeys.clear();
      complain(t("recording.needKeys"));
    }
  };

  // The middle or a side mouse button becomes the hotkey on its own;
  // any other click outside the button cancels. The button is saved once it
  // is released: the hook that takes it over must not catch this release
  let pressedMouse = -1;
  const onMouseDown = (e: MouseEvent) => {
    if (MOUSE_HOTKEYS[e.button] && pressedKeys.size === 0) {
      e.preventDefault();
      e.stopPropagation();
      pressedMouse = e.button;
      return;
    }
    if (!button.contains(e.target as Node)) {
      cleanup();
    }
  };

  const onMouseUp = (e: MouseEvent) => {
    if (e.button !== pressedMouse) return;
    e.preventDefault();
    e.stopPropagation();
    pressedMouse = -1;
    const hotkey = MOUSE_HOTKEYS[e.button];
    const why = problem(hotkey);
    if (why) complain(why);
    else saveHotkey(hotkey);
  };

  const removeListeners = () => {
    window.removeEventListener("keydown", onKeyDown, true);
    window.removeEventListener("keyup", onKeyUp, true);
    window.removeEventListener("mousedown", onMouseDown, true);
    window.removeEventListener("mouseup", onMouseUp, true);
  };

  const cleanup = () => {
    window.clearTimeout(messageTimer);
    removeListeners();
    pauseHotkeys(false);
    active = null;
    button.classList.remove("recording");
    render(recorder);
  };

  window.addEventListener("keydown", onKeyDown, true);
  window.addEventListener("keyup", onKeyUp, true);
  window.addEventListener("mousedown", onMouseDown, true);
  window.addEventListener("mouseup", onMouseUp, true);
}

// ── Public ───────────────────────────────────────────

export function initHotkeys(appHost: HotkeysHost) {
  host = appHost;
  const settings = () => host!.settings();
  const element = (id: string) => document.getElementById(id)!;

  recorders = [
    {
      button: element("hotkey-btn"),
      text: element("hotkey-text"),
      hint: element("hotkey-hint"),
      idleHint: () => "recording.hotkeyHint",
      prompt: "recording.pressKeys",
      promptHint: "recording.pressKeysHint",
      get: () => settings().hotkey,
      set: (hotkey) => (settings().hotkey = hotkey),
      modifiersAlone: false,
      other: () => settings().chatHotkey || "Ctrl+Alt",
      taken: "hotkeys.takenChat",
    },
    {
      button: element("chat-hotkey-btn"),
      text: element("chat-hotkey-text"),
      hint: element("chat-hotkey-hint"),
      idleHint: () => (settings().quickAsk.chat ? "hotkeys.chatHint" : "hotkeys.chatOffHint"),
      prompt: "hotkeys.chatPressKeys",
      promptHint: "hotkeys.chatPressKeysHint",
      get: () => settings().chatHotkey || "Ctrl+Alt",
      set: (hotkey) => (settings().chatHotkey = hotkey),
      modifiersAlone: true,
      other: () => settings().hotkey,
      taken: "hotkeys.takenDictation",
    },
  ];

  for (const recorder of recorders) {
    recorder.button.addEventListener("click", () => record(recorder));
  }
}

/** Shows the saved hotkeys (after the settings load or change). */
export function fillHotkeys() {
  recorders.forEach(render);
}

/** Texts set from code, after a language switch. */
export function translateHotkeys() {
  recorders.forEach(render);
}

// Quick ask window (ask.html, src-tauri/src/ask). The page first draws the
// recording indicator's dot exactly where the native one is, with its
// spinner. On "ask-reveal" the native dot is gone and this one unfolds into
// a card with the model's answers; its dot stays as the card's status light.
// The questions aren't shown, only the answers; a click on one copies it.
// While the window is open,
// every recording is a follow-up: the dot turns orange and pulses while the
// user speaks. The cross folds the card back into the dot, which slides
// behind the edge of the screen, and the window closes.
//
// Opened in a plain browser (npm run dev, then /src/ask.html, ?bottom for the
// bottom position), it plays a demo with sample answers.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { setLanguage, t } from "./i18n";
import { renderMarkdown } from "./markdown";

/** Set by the backend before the page loads (ask/window.rs) */
interface Layout {
  /** "top-right" | "bottom-center" */
  position: string;
  language: string;
  /** The indicator's centre in the window, logical pixels */
  orbX: number;
  orbY: number;
  /** The dot's centre from the card's edges at its corner */
  orbInset: number;
  cardWidth: number;
  maxHeight: number;
}

interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

type Phase = "idle" | "listening" | "thinking" | "answering";

interface Answer {
  el: HTMLElement;
  body: HTMLElement;
  /** The Markdown as written so far */
  text: string;
  pending: boolean;
  renderQueued: boolean;
}

const demo = !("__TAURI_INTERNALS__" in window);

function demoLayout(): Layout {
  // A page opened in a background tab may not have a size yet
  const width = innerWidth || 800;
  const height = innerHeight || 600;
  const language = navigator.language.startsWith("ru") ? "ru" : "en";
  const common = { language, orbInset: 18, cardWidth: 440, maxHeight: Math.min(560, height * 0.7) };
  return new URLSearchParams(location.search).has("bottom")
    ? { ...common, position: "bottom-center", orbX: width / 2, orbY: height - 35 }
    : { ...common, position: "top-right", orbX: width - 35, orbY: 35 };
}

const layout: Layout = (window as unknown as { __TYPR_ASK__?: Layout }).__TYPR_ASK__ ?? demoLayout();
const bottom = layout.position === "bottom-center";

const frame = document.getElementById("frame")!;
const card = document.getElementById("card")!;
const scroll = document.getElementById("scroll")!;
const content = document.getElementById("content")!;
const answersEl = document.getElementById("answers")!;
const notice = document.getElementById("notice")!;
const closeButton = document.getElementById("close")!;
const toast = document.getElementById("toast")!;

/** The indicator's disc */
const DOT = 40;
/** The strip with the dot and the close button (ask.css .strip) */
const STRIP = 34;
const RADIUS = 16;
/** 1px border on both sides */
const BORDERS = 2;
const MIN_HEIGHT = 72;
const NOTICE_MS = 6000;
const TOAST_MS = 1400;

document.body.classList.toggle("bottom", bottom);
document.body.classList.toggle("demo", demo);
setLanguage(layout.language === "ru" ? "ru" : "en");
closeButton.setAttribute("aria-label", t("askWindow.close"));

frame.style.setProperty("--orb-x", `${layout.orbX}px`);
frame.style.setProperty("--orb-y", `${layout.orbY}px`);
frame.style.setProperty(
  "--close-x",
  `${bottom ? layout.orbX + layout.cardWidth / 2 - 20 : layout.orbX - 30}px`,
);
// The card minus its border and the text's padding (18 on each side)
frame.style.setProperty("--content-width", `${layout.cardWidth - BORDERS - 36}px`);

let state: "dot" | "open" | "closing" = "dot";
let phase: Phase = "thinking";
let height = 0;
/** Keep the newest text in view unless the user scrolled up */
let stick = true;
/** A new window waits for its first answer to be announced */
let awaitingFirst = true;
let noticeTimer: number | undefined;
let toastTimer: number | undefined;
const answers = new Map<number, Answer>();

function call(command: string, args?: Record<string, unknown>) {
  if (demo) return;
  invoke(command, args).catch((err) => console.error(`${command} failed:`, err));
}

function setPhase(next: Phase) {
  phase = next;
  frame.dataset.phase = next;
}

// ── Geometry ─────────────────────────────────────────

function dotRect(): Rect {
  return { x: layout.orbX - DOT / 2, y: layout.orbY - DOT / 2, width: DOT, height: DOT };
}

/** The open card, `h` tall, growing away from the dot's corner */
function cardRect(h: number): Rect {
  const width = layout.cardWidth;
  if (bottom) {
    const cardBottom = layout.orbY + layout.orbInset;
    return { x: layout.orbX - width / 2, y: cardBottom - h, width, height: h };
  }
  const right = layout.orbX + layout.orbInset;
  return { x: right - width, y: layout.orbY - layout.orbInset, width, height: h };
}

/** Moves the card (animated by CSS) and tells the backend where it takes clicks */
function place(rect: Rect, radius: number) {
  card.style.left = `${rect.x}px`;
  card.style.top = `${rect.y}px`;
  card.style.width = `${rect.width}px`;
  card.style.height = `${rect.height}px`;
  card.style.borderRadius = `${radius}px`;
  call("ask_card", { x: rect.x, y: rect.y, width: rect.width, height: rect.height });
}

function naturalHeight(): number {
  const style = getComputedStyle(scroll);
  return BORDERS + STRIP + parseFloat(style.paddingTop) + parseFloat(style.paddingBottom) + content.offsetHeight;
}

let fitQueued = false;

function queueFit() {
  if (fitQueued) return;
  fitQueued = true;
  requestAnimationFrame(() => {
    fitQueued = false;
    fit();
  });
}

/** Grows (or shrinks) the card to its text, up to the tallest it may be */
function fit() {
  if (state !== "open") return;
  const next = Math.round(Math.min(layout.maxHeight, Math.max(MIN_HEIGHT, naturalHeight())));
  if (next !== height) {
    height = next;
    place(cardRect(height), RADIUS);
  }
  if (stick) scroll.scrollTop = scroll.scrollHeight;
}

// Any change of the text's size: a new piece, a notice, the web font
// replacing the fallback one and wrapping lines differently
new ResizeObserver(() => queueFit()).observe(content);

scroll.addEventListener("scroll", () => {
  stick = scroll.scrollTop + scroll.clientHeight >= scroll.scrollHeight - 24;
});

card.addEventListener("transitionend", (event) => {
  if (event.propertyName === "height" && stick) scroll.scrollTop = scroll.scrollHeight;
});

// ── Unfolding and folding ────────────────────────────

/**
 * The window is on screen. WebView2 starts painting a moment after the
 * window shows; an animation started before that runs unseen, and the card
 * just appears. So the dot unfolds only once frames are really drawn, and
 * only then may the native dot go (until then both are in the same place).
 */
function reveal() {
  if (state !== "dot") return;
  // Should frames never come (timers still run), the card unfolds anyway
  const fallback = window.setTimeout(() => {
    call("ask_shown");
    unfold();
  }, 800);
  whenPainting(() => {
    window.clearTimeout(fallback);
    call("ask_shown");
    requestAnimationFrame(unfold);
  });
}

/** Runs `then` once the page is visible and has drawn two frames */
function whenPainting(then: () => void) {
  const afterFrames = () => requestAnimationFrame(() => requestAnimationFrame(then));
  if (document.visibilityState === "visible") {
    afterFrames();
    return;
  }
  const onVisible = () => {
    if (document.visibilityState !== "visible") return;
    document.removeEventListener("visibilitychange", onVisible);
    afterFrames();
  };
  document.addEventListener("visibilitychange", onVisible);
}

/** The dot unfolds into the card */
function unfold() {
  if (state !== "dot") return;
  // The card appears exactly under the dot, in its colour, then grows
  card.style.transition = "none";
  place(dotRect(), DOT / 2);
  void card.offsetWidth;
  card.style.transition = "";
  frame.classList.add("unfolding");
  state = "open";
  frame.dataset.state = "open";
  height = 0;
  fit();
  window.setTimeout(() => frame.classList.add("content-in"), 420);
  window.setTimeout(() => frame.classList.remove("unfolding"), 640);
  countFrames(640);
}

/** Debug log: how many frames the unfolding got (0 means it wasn't seen) */
function countFrames(ms: number) {
  if (demo) return;
  const start = performance.now();
  let frames = 0;
  const tick = (now: number) => {
    frames++;
    if (now - start < ms) requestAnimationFrame(tick);
    else call("frontend_log", { level: "debug", message: `Quick ask card unfolded in ${frames} frames` });
  };
  requestAnimationFrame(tick);
}

function close() {
  if (state === "closing") return;
  state = "closing";
  frame.classList.remove("content-in");
  frame.classList.add("morphing");
  frame.dataset.state = "closing";
  place(dotRect(), DOT / 2);
  window.setTimeout(() => frame.classList.add("gone"), 340);
  window.setTimeout(() => {
    if (demo) {
      location.reload();
      return;
    }
    call("ask_close");
  }, 600);
}

closeButton.addEventListener("click", close);

// ── Answers ──────────────────────────────────────────

function hasPending(): boolean {
  return [...answers.values()].some((answer) => answer.pending);
}

function answerFor(id: number): Answer {
  const existing = answers.get(id);
  if (existing) return existing;

  const el = document.createElement("div");
  el.className = "answer pending";
  const body = document.createElement("div");
  body.className = "answer-body";
  const dots = document.createElement("span");
  dots.className = "dots";
  dots.append(document.createElement("i"), document.createElement("i"), document.createElement("i"));
  body.append(dots);

  el.append(body);
  answersEl.append(el);

  const answer: Answer = { el, body, text: "", pending: true, renderQueued: false };
  el.addEventListener("click", () => {
    // Selecting part of the text with the mouse is no click to copy
    const selection = window.getSelection();
    if (selection && !selection.isCollapsed && el.contains(selection.anchorNode)) return;
    void copyAnswer(answer);
  });
  answers.set(id, answer);
  stick = true;
  queueFit();
  return answer;
}

function settle(answer: Answer) {
  answer.pending = false;
  answer.el.classList.remove("pending");
}

function queueRender(answer: Answer) {
  if (answer.renderQueued) return;
  answer.renderQueued = true;
  requestAnimationFrame(() => {
    answer.renderQueued = false;
    answer.body.replaceChildren(renderMarkdown(answer.text));
    queueFit();
  });
}

async function copyAnswer(answer: Answer) {
  if (answer.pending || !answer.text.trim()) return;
  try {
    // A browser tab without focus refuses the clipboard: the demo shows the
    // feedback anyway
    if (demo) await navigator.clipboard.writeText(answer.text).catch(() => {});
    else await invoke("ask_copy", { text: answer.text });
  } catch (err) {
    console.error("Copy failed:", err);
    return;
  }
  // The answer lights up and the strip says so
  answer.el.classList.remove("copied");
  void answer.el.offsetWidth;
  answer.el.classList.add("copied");
  toast.textContent = t("askWindow.copied");
  toast.classList.add("visible");
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => toast.classList.remove("visible"), TOAST_MS);
}

function showNotice(message: string) {
  notice.textContent = message;
  notice.classList.remove("hidden");
  queueFit();
  window.clearTimeout(noticeTimer);
  noticeTimer = window.setTimeout(hideNotice, NOTICE_MS);
}

function hideNotice() {
  window.clearTimeout(noticeTimer);
  if (notice.classList.contains("hidden")) return;
  notice.classList.add("hidden");
  queueFit();
}

/** The phase after an answer, unless the user is already speaking again */
function afterAnswer(next: Phase) {
  if (phase !== "listening") setPhase(next);
}

function onAnswer(id: number) {
  awaitingFirst = false;
  answerFor(id);
  afterAnswer("thinking");
}

function onDelta(id: number, text: string) {
  awaitingFirst = false;
  const answer = answerFor(id);
  settle(answer);
  answer.text += text;
  queueRender(answer);
  afterAnswer("answering");
}

function onDone(id: number, text: string, stopped: boolean) {
  awaitingFirst = false;
  const answer = answerFor(id);
  settle(answer);
  answer.text = text;
  if (stopped && !answer.el.classList.contains("stopped")) {
    answer.el.classList.add("stopped");
    const note = document.createElement("div");
    note.className = "answer-note";
    note.textContent = t("askWindow.stopped");
    answer.el.append(note);
  }
  queueRender(answer);
  afterAnswer(hasPending() ? "thinking" : "idle");
}

function onError(id: number, message: string) {
  awaitingFirst = false;
  const answer = answerFor(id);
  settle(answer);
  answer.text = "";
  answer.el.classList.add("error");
  answer.body.textContent = t("askWindow.error", { error: message });
  queueFit();
  afterAnswer(hasPending() ? "thinking" : "idle");
}

function onCancel(id: number) {
  answers.get(id)?.el.remove();
  answers.delete(id);
  queueFit();
}

/** A follow-up: the recorder's own states, from the hotkey press on */
function onRecordingState(recording: string) {
  if (state !== "open") return;
  if (recording === "Recording") {
    hideNotice();
    setPhase("listening");
  } else if (recording === "Transcribing") {
    setPhase("thinking");
  } else if (!awaitingFirst && !hasPending() && phase !== "answering") {
    setPhase("idle");
  }
}

function onRecordingError(message: string) {
  if (state !== "open") return;
  showNotice(message);
  if (!hasPending() && (phase === "listening" || phase === "thinking")) setPhase("idle");
}

// ── Wiring ───────────────────────────────────────────

// No browser menu, and the side mouse buttons never navigate the page
document.addEventListener("contextmenu", (event) => event.preventDefault());
window.addEventListener("mouseup", (event) => {
  if (event.button === 3 || event.button === 4) event.preventDefault();
});

if (demo) {
  void runDemo();
} else {
  void Promise.all([
    listen("ask-reveal", () => reveal()),
    listen<{ id: number }>("ask-answer", (e) => onAnswer(e.payload.id)),
    listen<{ id: number; text: string }>("ask-delta", (e) => onDelta(e.payload.id, e.payload.text)),
    listen<{ id: number; text: string; stopped: boolean }>("ask-done", (e) =>
      onDone(e.payload.id, e.payload.text, e.payload.stopped),
    ),
    listen<{ id: number; message: string }>("ask-error", (e) => onError(e.payload.id, e.payload.message)),
    listen<{ id: number }>("ask-cancel", (e) => onCancel(e.payload.id)),
    listen<string>("recording-state", (e) => onRecordingState(e.payload)),
    listen<string>("recording-error", (e) => onRecordingError(e.payload)),
  ]).then(() => {
    // Listening, and the dot is painted: the backend may show the window
    requestAnimationFrame(() => requestAnimationFrame(() => call("ask_ready")));
  });
}

// ── Demo in a plain browser ──────────────────────────

async function runDemo() {
  const sleep = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));
  const ru = layout.language === "ru";
  const first = ru
    ? "Это **регулярное выражение** проверяет адрес почты:\n\n- `^[\\w.+-]+` — имя до `@`\n- `@[\\w-]+` — домен\n- `\\.[a-z]{2,}$` — зона, например `.com`\n\nОно пропустит `anna.k+news@mail.ru`, но не `anna@mail`."
    : "This **regular expression** checks an email address:\n\n- `^[\\w.+-]+` is the name before `@`\n- `@[\\w-]+` is the domain\n- `\\.[a-z]{2,}$` is the zone, like `.com`\n\nIt accepts `anna.k+news@mail.com`, but not `anna@mail`.";
  const second = ru
    ? "Да, вот вариант построже:\n\n```js\nconst email = /^[\\w.+-]+@[\\w-]+(\\.[\\w-]+)*\\.[a-z]{2,}$/i;\n```"
    : "Yes, here is a stricter one:\n\n```js\nconst email = /^[\\w.+-]+@[\\w-]+(\\.[\\w-]+)*\\.[a-z]{2,}$/i;\n```";

  const stream = async (id: number, text: string) => {
    for (let i = 0; i < text.length; i += 6) {
      onDelta(id, text.slice(i, i + 6));
      await sleep(28);
    }
    onDone(id, text, false);
  };

  await sleep(900);
  reveal();
  onAnswer(1);
  await sleep(700);
  await stream(1, first);
  await sleep(1800);
  onRecordingState("Recording");
  await sleep(2000);
  onRecordingState("Transcribing");
  onAnswer(2);
  onRecordingState("Ready");
  await sleep(800);
  await stream(2, second);
}

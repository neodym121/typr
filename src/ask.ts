// Quick ask window (ask.html, src-tauri/src/ask). The page first draws the
// recording indicator's dot exactly where the native one is, with its
// spinner. On "ask-reveal" the native dot is gone and this one unfolds into
// a card with the model's answers; its dot stays as the card's status light.
// The questions aren't shown, only the answers; a click on one copies it,
// a click on a link in it opens the page. While the model searches the web,
// "Search…" blinks slowly where the answer will be, and fades away once it
// starts.
// While the window is open,
// every recording is a follow-up: the dot turns orange and pulses while the
// user speaks. The cross folds the card back into the dot, which slides
// behind the edge of the screen, and the window closes.
//
// Quick chat (mode "chat") is the same window opened empty by its hotkey:
// the dot slides in from behind the edge and unfolds into a card that says
// how to ask. The questions are shown too, on the right in a box;
// they can't be copied. The chat hotkey folds it away ("ask-fold").
//
// Opened in a plain browser (npm run dev, then /src/ask.html, ?bottom for the
// bottom position, ?chat for quick chat), it plays a demo with sample answers.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { formatHotkey, setLanguage, t } from "./i18n";
import { renderMarkdown } from "./markdown";

/** Set by the backend before the page loads (ask/window.rs) */
interface Layout {
  /** "top-right" | "bottom-center" */
  position: string;
  language: string;
  /** "ask": only the answers; "chat": quick chat, the questions too */
  mode: string;
  /** The dictation hotkey, which asks in the chat */
  hotkey: string;
  pushToTalk: boolean;
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
  /** What happens before the first words: the typing dots or "Search…" */
  status: HTMLElement | null;
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
  const params = new URLSearchParams(location.search);
  const common = {
    language,
    mode: params.has("chat") ? "chat" : "ask",
    hotkey: "Ctrl+Shift+Space",
    pushToTalk: false,
    orbInset: 18,
    cardWidth: 440,
    maxHeight: Math.min(560, height * 0.7),
  };
  return params.has("bottom")
    ? { ...common, position: "bottom-center", orbX: width / 2, orbY: height - 35 }
    : { ...common, position: "top-right", orbX: width - 35, orbY: 35 };
}

const layout: Layout = (window as unknown as { __TYPR_ASK__?: Layout }).__TYPR_ASK__ ?? demoLayout();
const bottom = layout.position === "bottom-center";
const chat = layout.mode === "chat";

const frame = document.getElementById("frame")!;
const card = document.getElementById("card")!;
const scroll = document.getElementById("scroll")!;
const content = document.getElementById("content")!;
const messagesEl = document.getElementById("messages")!;
const emptyEl = document.getElementById("empty")!;
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
frame.dataset.mode = chat ? "chat" : "ask";
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
/** A quick ask window waits for its first answer to be announced */
let awaitingFirst = !chat;
let noticeTimer: number | undefined;
let toastTimer: number | undefined;
const answers = new Map<number, Answer>();
/** Quick chat: the questions, by the id of their answer */
const questions = new Map<number, HTMLElement>();

// Quick chat opens with nothing to do yet; its dot waits behind the edge of
// the screen, where the indicator would come from
if (chat) {
  setPhase("idle");
  frame.classList.add("gone");
}

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
  let started = false;
  const go = () => {
    if (started) return;
    started = true;
    call("ask_shown");
    if (chat) arrive();
    else requestAnimationFrame(unfold);
  };
  // Should frames never come (timers still run), the card unfolds anyway
  const fallback = window.setTimeout(go, 800);
  whenPainting(() => {
    window.clearTimeout(fallback);
    go();
  });
}

/** Quick chat: the dot slides in from behind the edge, then unfolds */
function arrive() {
  frame.classList.add("arriving");
  frame.classList.remove("gone");
  window.setTimeout(() => {
    frame.classList.remove("arriving");
    unfold();
  }, 300);
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
  const status = document.createElement("div");
  status.className = "answer-status";
  const dots = document.createElement("span");
  dots.className = "dots";
  dots.append(document.createElement("i"), document.createElement("i"), document.createElement("i"));
  status.append(dots);
  const body = document.createElement("div");
  body.className = "answer-body";

  el.append(status, body);
  messagesEl.append(el);

  const answer: Answer = { el, status, body, text: "", pending: true, renderQueued: false };
  el.addEventListener("click", (event) => {
    // A link opens its page in the browser instead
    const link = (event.target as Element).closest<HTMLElement>(".link");
    if (link?.dataset.href) {
      call("ask_open_link", { url: link.dataset.href });
      return;
    }
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

/** The first words (or the end) arrived: the dots or "Search…" fade away */
function settle(answer: Answer) {
  answer.pending = false;
  answer.el.classList.remove("pending");
  const status = answer.status;
  if (!status) return;
  answer.status = null;
  status.style.maxHeight = `${status.offsetHeight}px`;
  void status.offsetHeight;
  status.classList.add("leaving");
  const remove = () => {
    status.remove();
    queueFit();
  };
  status.addEventListener("transitionend", remove, { once: true });
  window.setTimeout(remove, 600);
}

/** The model searches the web */
function onSearch(id: number) {
  awaitingFirst = false;
  const answer = answerFor(id);
  if (!answer.status) return;
  const label = document.createElement("span");
  label.className = "searching";
  label.textContent = t("askWindow.searching");
  answer.status.replaceChildren(label);
  queueFit();
  afterAnswer("thinking");
}

/** Quick chat: a question, with the start of the text it is about */
function onQuestion(id: number, text: string, selection: string | null) {
  if (!chat || questions.has(id)) return;
  const el = document.createElement("div");
  el.className = "question";
  if (selection) {
    const quote = document.createElement("div");
    quote.className = "question-quote";
    quote.textContent = selection;
    el.append(quote);
  }
  const body = document.createElement("div");
  body.className = "question-text";
  body.textContent = text;
  el.append(body);
  messagesEl.append(el);
  questions.set(id, el);
  renderEmpty();
  stick = true;
  queueFit();
}

/** Quick chat before the first question: which hotkey asks */
function renderEmpty() {
  const empty = chat && messagesEl.childElementCount === 0;
  emptyEl.classList.toggle("hidden", !empty);
  if (!empty || emptyEl.childElementCount > 0) return;
  const [before, after = ""] = t(layout.pushToTalk ? "askWindow.chatEmptyPtt" : "askWindow.chatEmpty").split("{hotkey}");
  const key = document.createElement("kbd");
  key.textContent = formatHotkey(layout.hotkey || "");
  emptyEl.replaceChildren(before, key, after);
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
  // Nothing was answered: the question goes too, it has left the context
  questions.get(id)?.remove();
  questions.delete(id);
  renderEmpty();
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

renderEmpty();

if (demo) {
  void runDemo();
} else {
  void Promise.all([
    listen("ask-reveal", () => reveal()),
    listen("ask-fold", () => close()),
    listen<{ id: number; text: string; selection: string | null }>("ask-question", (e) =>
      onQuestion(e.payload.id, e.payload.text, e.payload.selection),
    ),
    listen<{ id: number }>("ask-answer", (e) => onAnswer(e.payload.id)),
    listen<{ id: number; text: string }>("ask-delta", (e) => onDelta(e.payload.id, e.payload.text)),
    listen<{ id: number; text: string; stopped: boolean }>("ask-done", (e) =>
      onDone(e.payload.id, e.payload.text, e.payload.stopped),
    ),
    listen<{ id: number; message: string }>("ask-error", (e) => onError(e.payload.id, e.payload.message)),
    listen<{ id: number }>("ask-cancel", (e) => onCancel(e.payload.id)),
    listen<{ id: number; query: string }>("ask-search", (e) => onSearch(e.payload.id)),
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

  const third = ru
    ? "По данным [Exa](https://exa.ai), последняя версия — **2.1**, вышла на прошлой неделе."
    : "According to [Exa](https://exa.ai), the latest version is **2.1**, released last week.";

  const stream = async (id: number, text: string) => {
    for (let i = 0; i < text.length; i += 6) {
      onDelta(id, text.slice(i, i + 6));
      await sleep(28);
    }
    onDone(id, text, false);
  };

  if (chat) {
    // A question as the dictation hotkey asks it
    const ask = async (id: number, question: string, selection: string | null) => {
      onRecordingState("Recording");
      await sleep(1600);
      onRecordingState("Transcribing");
      await sleep(500);
      onQuestion(id, question, selection);
      onAnswer(id);
      onRecordingState("Ready");
      await sleep(700);
    };
    await sleep(700);
    reveal();
    await sleep(1800);
    await ask(1, ru ? "Чем let отличается от const в JavaScript?" : "How is let different from const in JavaScript?", null);
    await stream(
      1,
      ru
        ? "`const` нельзя **переназначить**, а `let` можно. Оба видны только внутри блока `{}`.\n\nОбъект в `const` при этом менять можно: запрещено только присвоить переменной другое значение."
        : "`const` can't be **reassigned**, `let` can. Both live only inside their block `{}`.\n\nAn object in a `const` can still change: only giving the variable another value is forbidden.",
    );
    await sleep(1500);
    await ask(
      2,
      ru ? "Переведи на английский" : "Translate this into Russian",
      ru ? "Встреча переносится на четверг, 15:00. Пожалуйста, подтвердите участие." : "The meeting moves to Thursday at 3 pm. Please confirm you can make it.",
    );
    await stream(
      2,
      ru
        ? "The meeting has been moved to Thursday at 3 pm. Please confirm your attendance."
        : "Встреча переносится на четверг, 15:00. Пожалуйста, подтвердите, что сможете прийти.",
    );
    return;
  }

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
  await sleep(1500);
  onRecordingState("Recording");
  await sleep(1500);
  onRecordingState("Transcribing");
  onAnswer(3);
  onRecordingState("Ready");
  await sleep(500);
  onSearch(3);
  await sleep(2600);
  await stream(3, third);
}

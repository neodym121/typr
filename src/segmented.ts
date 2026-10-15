// Segmented controls (.segmented): the light thumb under the active segment
// is one element that springs from segment to segment, a little elastic, and
// it can be grabbed and dragged to another one: it lands on the segment
// under its middle. The segments stay plain
// buttons: the modules toggle their `active` class and handle their clicks
// as before; a drag ends with a click on the segment it was dropped on.

/** The position's spring: a bit underdamped, so it overshoots a few pixels */
const STIFFNESS = 520;
const DAMPING = 30;
/** The width settles without overshoot (critical damping) */
const WIDTH_DAMPING = 2 * Math.sqrt(STIFFNESS);
/** Pixels the pointer moves before a press becomes a drag */
const DRAG_THRESHOLD = 3;
/** How far past the end segments the thumb gives at most */
const RUBBER_LIMIT = 18;
/** Part of the release speed the thumb keeps on its way to the segment, and
 *  its limit (px/s): enough for a small swing, never a jump past it */
const CARRY = 0.5;
const MAX_CARRY = 500;

const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

interface Spring {
  value: number;
  velocity: number;
  target: number;
}

interface Drag {
  pointerId: number;
  startX: number;
  /** The thumb's centre when the press began */
  startCentre: number;
  moved: boolean;
  /** Recent thumb positions, for the speed at release */
  samples: { time: number; left: number }[];
}

function step(spring: Spring, damping: number, dt: number) {
  const force = -STIFFNESS * (spring.value - spring.target) - damping * spring.velocity;
  spring.velocity += force * dt;
  spring.value += spring.velocity * dt;
}

function settled(spring: Spring): boolean {
  return Math.abs(spring.value - spring.target) < 0.2 && Math.abs(spring.velocity) < 2;
}

/** Past an end the thumb follows less and less: `distance` px out moves it this far */
function rubber(distance: number): number {
  return RUBBER_LIMIT * (1 - 1 / ((distance * 0.5) / RUBBER_LIMIT + 1));
}

const centre = (segment: HTMLElement) => segment.offsetLeft + segment.offsetWidth / 2;

/** Gives `control` its sliding thumb. Safe to call once per control; controls
 *  built later (the setup) are enhanced when they are created. */
export function enhanceSegmented(control: HTMLElement) {
  if (control.classList.contains("has-thumb")) return;
  const thumb = document.createElement("span");
  thumb.className = "segmented-thumb";
  thumb.setAttribute("aria-hidden", "true");
  control.prepend(thumb);
  control.classList.add("has-thumb");

  const x: Spring = { value: 0, velocity: 0, target: 0 };
  const width: Spring = { value: 0, velocity: 0, target: 0 };
  /** The segment the thumb belongs to */
  let current: HTMLElement | null = null;
  /** The thumb sits on a laid-out segment (the control isn't hidden) */
  let placed = false;
  let frame = 0;
  let lastTime = 0;
  let drag: Drag | null = null;
  let suppressClick = false;

  const segments = () =>
    Array.from(control.querySelectorAll<HTMLElement>(":scope > .segment")).filter((s) => s.offsetWidth > 0);
  const choosable = () => segments().filter((s) => !(s as HTMLButtonElement).disabled);
  const activeSegment = () =>
    Array.from(control.querySelectorAll<HTMLElement>(":scope > .segment.active")).find((s) => s.offsetWidth > 0) ??
    null;

  function render() {
    thumb.style.transform = `translateX(${x.value}px)`;
    thumb.style.width = `${Math.max(0, width.value)}px`;
  }

  function stop() {
    cancelAnimationFrame(frame);
    frame = 0;
  }

  /** Puts the thumb on the active segment at once */
  function snap() {
    stop();
    current = activeSegment();
    placed = Boolean(current);
    thumb.classList.toggle("hidden", !current);
    if (!current) return;
    x.value = x.target = current.offsetLeft;
    width.value = width.target = current.offsetWidth;
    x.velocity = width.velocity = 0;
    thumb.style.top = `${current.offsetTop}px`;
    thumb.style.height = `${current.offsetHeight}px`;
    render();
  }

  function tick(now: number) {
    // The frame's time can be a little earlier than when the spring started
    const dt = Math.min(Math.max(now - lastTime, 0) / 1000, 1 / 20);
    lastTime = now;
    // Small fixed steps keep the spring stable on slow frames
    const steps = Math.max(1, Math.ceil(dt * 240));
    for (let i = 0; i < steps; i++) {
      step(x, DAMPING, dt / steps);
      step(width, WIDTH_DAMPING, dt / steps);
    }
    if (settled(x) && settled(width)) {
      x.value = x.target;
      width.value = width.target;
      x.velocity = width.velocity = 0;
      render();
      frame = 0;
      return;
    }
    render();
    frame = requestAnimationFrame(tick);
  }

  /** Springs the thumb from wherever it is to `segment` */
  function moveTo(segment: HTMLElement | null) {
    if (!segment || !placed || reducedMotion.matches) {
      snap();
      return;
    }
    current = segment;
    x.target = segment.offsetLeft;
    width.target = segment.offsetWidth;
    thumb.style.top = `${segment.offsetTop}px`;
    thumb.style.height = `${segment.offsetHeight}px`;
    if (!frame) {
      lastTime = performance.now();
      frame = requestAnimationFrame(tick);
    }
  }

  // The modules switch segments by their `active` class
  new MutationObserver(() => {
    const segment = activeSegment();
    if (segment !== current && !drag?.moved) moveTo(segment);
  }).observe(control, { subtree: true, attributes: true, attributeFilter: ["class"] });

  // Shown, resized, relabelled (a language switch, the log counts): the
  // thumb goes where the active segment is now
  const resize = new ResizeObserver(() => {
    if (drag?.moved) return;
    if (frame && current && current.offsetWidth > 0) moveTo(current);
    else snap();
  });
  resize.observe(control);
  control.querySelectorAll<HTMLElement>(":scope > .segment").forEach((s) => resize.observe(s));

  // ── Dragging ─────────────────────────────────────────

  /** The thumb's width at a centre between segments: blends the two nearest */
  function widthAt(at: number, list: HTMLElement[]): number {
    if (at <= centre(list[0])) return list[0].offsetWidth;
    for (let i = 0; i < list.length - 1; i++) {
      const a = centre(list[i]);
      const b = centre(list[i + 1]);
      if (at <= b) {
        const t = (at - a) / (b - a);
        return list[i].offsetWidth + (list[i + 1].offsetWidth - list[i].offsetWidth) * t;
      }
    }
    return list[list.length - 1].offsetWidth;
  }

  function nearest(at: number): HTMLElement | null {
    let best: HTMLElement | null = null;
    for (const segment of choosable()) {
      if (!best || Math.abs(centre(segment) - at) < Math.abs(centre(best) - at)) best = segment;
    }
    return best;
  }

  function markOver(segment: HTMLElement | null) {
    segments().forEach((s) => s.classList.toggle("thumb-over", s === segment));
  }

  function onMove(event: PointerEvent) {
    if (!drag || event.pointerId !== drag.pointerId) return;
    const dx = event.clientX - drag.startX;
    if (!drag.moved) {
      if (Math.abs(dx) < DRAG_THRESHOLD) return;
      drag.moved = true;
      stop();
      try {
        control.setPointerCapture(drag.pointerId);
      } catch {
        // the pointer is gone already; the release still comes to the window
      }
      control.classList.add("dragging");
      document.documentElement.classList.add("thumb-dragging");
    }
    const list = segments();
    if (list.length === 0) return;
    const at = drag.startCentre + dx;
    const size = widthAt(at, list);
    const start = list[0].offsetLeft;
    const end = list[list.length - 1].offsetLeft + list[list.length - 1].offsetWidth - size;
    let left = at - size / 2;
    if (left < start) left = start - rubber(start - left);
    if (left > end) left = end + rubber(left - end);

    x.value = left;
    width.value = size;
    render();
    markOver(nearest(left + size / 2));

    drag.samples.push({ time: event.timeStamp, left });
    while (drag.samples.length > 2 && event.timeStamp - drag.samples[0].time > 100) drag.samples.shift();
  }

  /** Pixels per second over the last moments of the drag */
  function releaseSpeed(samples: Drag["samples"]): number {
    if (samples.length < 2) return 0;
    const first = samples[0];
    const last = samples[samples.length - 1];
    const seconds = (last.time - first.time) / 1000;
    if (seconds <= 0) return 0;
    return (last.left - first.left) / seconds;
  }

  function onUp(event: PointerEvent) {
    if (!drag || event.pointerId !== drag.pointerId) return;
    const ended = drag;
    drag = null;
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
    window.removeEventListener("pointercancel", onUp);
    if (!ended.moved) return; // a plain click: the button handles it

    control.classList.remove("dragging");
    document.documentElement.classList.remove("thumb-dragging");
    markOver(null);
    if (control.hasPointerCapture(ended.pointerId)) control.releasePointerCapture(ended.pointerId);

    x.velocity = Math.max(-MAX_CARRY, Math.min(MAX_CARRY, releaseSpeed(ended.samples) * CARRY));
    width.velocity = 0;
    const target = event.type === "pointercancel" ? null : nearest(x.value + width.value / 2);
    // The module makes it active (or refuses); the thumb goes to whichever is active then
    if (target && target !== current) target.click();
    moveTo(activeSegment());

    // The click the browser sends after this release isn't a choice
    suppressClick = true;
    window.setTimeout(() => (suppressClick = false), 0);
  }

  control.addEventListener(
    "click",
    (event) => {
      if (!suppressClick) return;
      suppressClick = false;
      event.preventDefault();
      event.stopImmediatePropagation();
    },
    true,
  );

  // Only the thumb is grabbed: a press on the active segment
  control.addEventListener("pointerdown", (event) => {
    if (event.button !== 0 || drag || !placed) return;
    const segment = (event.target as Element).closest<HTMLElement>(".segment");
    if (!segment || segment !== current || segment.parentElement !== control) return;
    drag = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startCentre: x.value + width.value / 2,
      moved: false,
      samples: [{ time: event.timeStamp, left: x.value }],
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
  });

  snap();
}

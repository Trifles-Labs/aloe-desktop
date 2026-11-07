import "@fontsource-variable/geist";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

import type { AgentConfig } from "./types";

/* One widget voice left on the screen (widget.html, its own window: voice/widgets.rs). No React,
   like voice-overlay.ts: Rust owns the widget and pushes it in whole ("widget-update"); this draws
   it, ticks the clock, and sends clicks back. Everything is built with textContent.

   The whole card drags the window, except its buttons, checkboxes and the note's text. */

type Item = { text: string; done: boolean };
type Widget = {
  id: string;
  kind: "timer" | "stopwatch" | "clock" | "note";
  label: string;
  totalMs: number;
  endsAtMs: number | null;
  remainingMs: number;
  rang: boolean;
  startedAtMs: number | null;
  elapsedMs: number;
  zones: string[];
  text: string;
  items: Item[];
};

const WIDTH = 232;
/* The card's margin in widget.html, room for its shadow. */
const MARGIN = 8;

const id = new URLSearchParams(location.search).get("id") ?? "";
const card = document.getElementById("card")!;
const appWindow = getCurrentWindow();
let widget: Widget | null = null;
let ticker: number | undefined;

const el = <K extends keyof HTMLElementTagNameMap>(tag: K, className?: string, text?: string) => {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
};

const ICONS = {
  pause: "M3 2h3v12H3zM10 2h3v12h-3z",
  play: "M4 2l10 6-10 6z",
  reset: "M8 2a6 6 0 1 1-5.7 4.1l1.9.6A4 4 0 1 0 8 4v2.5L4.5 3.2 8 0z",
  close: "M3.5 2 8 6.5 12.5 2 14 3.5 9.5 8 14 12.5 12.5 14 8 9.5 3.5 14 2 12.5 6.5 8 2 3.5z",
};

const button = (icon: keyof typeof ICONS, label: string, onClick: () => void, className = "btn") => {
  const node = el("button", className);
  node.type = "button";
  node.title = label;
  node.setAttribute("aria-label", label);
  node.innerHTML = `<svg viewBox="0 0 16 16" aria-hidden="true"><path d="${ICONS[icon]}"/></svg>`;
  node.addEventListener("click", (event) => {
    event.stopPropagation();
    onClick();
  });
  return node;
};

const act = (action: Record<string, unknown>) => {
  void invoke<Widget>("widget_action", { id, action }).then(show, () => undefined);
};
const close = () => void invoke("widget_close", { id });

/* ── Time ─────────────────────────────────────────────────────────────────── */

const pad = (n: number) => String(n).padStart(2, "0");
const duration = (ms: number, tenths = false) => {
  const total = Math.max(0, ms);
  const seconds = Math.floor(total / 1000);
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  const clock = h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
  return tenths ? `${clock}.${Math.floor((total % 1000) / 100)}` : clock;
};
const timerLeft = (w: Widget, now: number) => (w.endsAtMs !== null ? Math.max(0, w.endsAtMs - now) : Math.max(0, w.remainingMs));
const stopwatchElapsed = (w: Widget, now: number) => (w.startedAtMs !== null ? Math.max(0, now - w.startedAtMs) : w.elapsedMs);

const cityOf = (zone: string) => zone.split("/").pop()!.replace(/_/g, " ");
const dayKey = (date: Date, timeZone?: string) => new Intl.DateTimeFormat("en-CA", { timeZone, year: "numeric", month: "2-digit", day: "2-digit" }).format(date);
const dayShift = (date: Date, zone: string) => {
  const here = dayKey(date);
  const there = dayKey(date, zone);
  return there === here ? "" : there > here ? "+1 day" : "−1 day";
};
const timeIn = (date: Date, timeZone?: string) => new Intl.DateTimeFormat(undefined, { timeZone, hour: "numeric", minute: "2-digit" }).format(date);

/* ── Drawing ──────────────────────────────────────────────────────────────── */

/* What changes every tick, so a tick never rebuilds the card (or loses a focused button). */
let paintTick: (() => void) | null = null;

const head = (w: Widget, title: string, controls: HTMLElement[]) => {
  const row = el("div", "head");
  row.append(el("div", "title", w.label || title), ...controls, button("close", "Close", close, "btn close"));
  return row;
};

function drawTimer(w: Widget) {
  const big = el("div", "big");
  const bar = el("div", "bar");
  const fill = el("i");
  bar.append(fill);
  const running = w.endsAtMs !== null;
  const controls = [
    running ? button("pause", "Pause", () => act({ op: "pause" })) : button("play", "Start", () => act({ op: "resume" })),
    button("reset", "Reset", () => act({ op: "reset" })),
  ];
  if (!running && timerLeft(w, Date.now()) === 0) controls.shift();
  card.append(head(w, "Timer", controls), big, bar);
  paintTick = () => {
    const left = timerLeft(w, Date.now());
    big.textContent = left === 0 ? "Done" : duration(left + 999);
    fill.style.transform = `scaleX(${w.totalMs > 0 ? left / w.totalMs : 0})`;
    card.toggleAttribute("data-finished", left === 0);
  };
}

function drawStopwatch(w: Widget) {
  const big = el("div", "big");
  const running = w.startedAtMs !== null;
  card.append(
    head(w, "Stopwatch", [
      running ? button("pause", "Pause", () => act({ op: "pause" })) : button("play", "Start", () => act({ op: "resume" })),
      button("reset", "Reset", () => act({ op: "reset" })),
    ]),
    big,
  );
  paintTick = () => {
    big.textContent = duration(stopwatchElapsed(w, Date.now()), true);
  };
}

function drawClock(w: Widget) {
  if (w.zones.length === 0) {
    const big = el("div", "big");
    const date = el("div", "date");
    card.append(head(w, "Clock", []), big, date);
    paintTick = () => {
      const now = new Date();
      big.textContent = timeIn(now);
      date.textContent = now.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
    };
    return;
  }
  const list = el("div", "zones");
  const rows = w.zones.map((zone) => {
    const row = el("div", "zone");
    const time = el("span", "time");
    const day = el("span", "day");
    row.append(el("span", "city", cityOf(zone)), time, day);
    list.append(row);
    return { zone, time, day };
  });
  card.append(head(w, w.zones.length === 1 ? "Clock" : "World clock", []), list);
  paintTick = () => {
    const now = new Date();
    for (const row of rows) {
      row.time.textContent = timeIn(now, row.zone);
      row.day.textContent = dayShift(now, row.zone);
    }
  };
}

function drawNote(w: Widget) {
  card.append(head(w, w.items.length > 0 && !w.text ? "Checklist" : "Note", []));
  if (w.text || w.items.length === 0) {
    const text = el("div", "text", w.text);
    text.contentEditable = "plaintext-only";
    text.spellcheck = false;
    text.addEventListener("blur", () => {
      const next = (text.textContent ?? "").trim().slice(0, 500);
      if (next !== w.text) act({ op: "text", text: next });
      else if (pending) show(pending);
    });
    text.addEventListener("keydown", (event) => {
      if (event.key === "Escape") text.blur();
    });
    card.append(text);
  }
  if (w.items.length > 0) {
    const list = el("ul", "items");
    w.items.forEach((item, index) => {
      const row = el("li", item.done ? "done" : undefined);
      const label = el("label");
      const box = el("input");
      box.type = "checkbox";
      box.checked = item.done;
      box.addEventListener("change", () => act({ op: "check", index, done: box.checked }));
      label.append(box, el("span", undefined, item.text));
      row.append(label);
      list.append(row);
    });
    card.append(list);
  }
  paintTick = null;
}

/* An update that lands while the note is being typed in waits for the edit to end. */
let pending: Widget | null = null;

function show(next: Widget | null) {
  if (!next) return close();
  const editing = document.activeElement instanceof HTMLElement && document.activeElement.isContentEditable;
  if (editing) {
    pending = next;
    return;
  }
  pending = null;
  widget = next;
  card.replaceChildren();
  card.removeAttribute("data-finished");
  ({ timer: drawTimer, stopwatch: drawStopwatch, clock: drawClock, note: drawNote })[next.kind]?.(next);
  paintTick?.();
  window.clearInterval(ticker);
  const live = (next.kind === "timer" && next.endsAtMs !== null) || (next.kind === "stopwatch" && next.startedAtMs !== null) || next.kind === "clock";
  if (live) ticker = window.setInterval(() => paintTick?.(), next.kind === "stopwatch" ? 100 : 250);
  card.setAttribute("data-shown", "");
  fit();
}

/* The window's height follows the card, so nothing see-through sits on screen taking clicks. */
function fit() {
  requestAnimationFrame(() => {
    const height = Math.ceil(card.getBoundingClientRect().height) + MARGIN * 2;
    void appWindow.setSize(new LogicalSize(WIDTH, height)).catch(() => undefined);
  });
}

/* ── Ringing ──────────────────────────────────────────────────────────────── */

let audio: AudioContext | null = null;
function chime() {
  try {
    audio ??= new AudioContext();
    const ctx = audio;
    void ctx.resume();
    const start = ctx.currentTime + 0.05;
    [0, 0.9, 1.8].forEach((round) =>
      [880, 1175, 1568].forEach((frequency, step) => {
        const at = start + round + step * 0.14;
        const osc = ctx.createOscillator();
        const gain = ctx.createGain();
        osc.type = "sine";
        osc.frequency.value = frequency;
        gain.gain.setValueAtTime(0, at);
        gain.gain.linearRampToValueAtTime(0.18, at + 0.015);
        gain.gain.exponentialRampToValueAtTime(0.001, at + 0.5);
        osc.connect(gain).connect(ctx.destination);
        osc.start(at);
        osc.stop(at + 0.55);
      }),
    );
  } catch {
    /* No audio: the pulse and the system notification still say it. */
  }
}

let ringTimeout: number | undefined;
const stopRinging = () => {
  card.removeAttribute("data-ringing");
  window.clearTimeout(ringTimeout);
};
function ring() {
  card.setAttribute("data-ringing", "");
  chime();
  window.clearTimeout(ringTimeout);
  ringTimeout = window.setTimeout(stopRinging, 60_000);
}

/* ── Wiring ───────────────────────────────────────────────────────────────── */

const interactive = (target: EventTarget | null) => target instanceof Element && !!target.closest("button, input, label, [contenteditable]");
card.addEventListener("mousedown", (event) => {
  if (event.button !== 0 || interactive(event.target)) return;
  stopRinging();
  void appWindow.startDragging().catch(() => undefined);
});
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") stopRinging();
});

void invoke<AgentConfig>("get_config").then(
  (config) => {
    document.documentElement.dataset.size = config.voice?.textSize || "m";
    document.documentElement.toggleAttribute("data-contrast", !!config.voice?.highContrast);
    fit();
  },
  () => undefined,
);
void listen<Widget>("widget-update", (event) => {
  if (event.payload.id === id) show(event.payload);
});
void listen("widget-ring", () => ring());
void invoke<Widget | null>("widget_state", { id }).then((state) => {
  show(state);
  // Rang while the app was closed: it shows as done, and only the notification said so.
  if (state?.kind === "timer" && state.rang && widget) card.setAttribute("data-finished", "");
}, close);

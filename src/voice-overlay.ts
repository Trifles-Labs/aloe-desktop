import "@fontsource-variable/instrument-sans";
import { invoke } from "@tauri-apps/api/core";

/* The push-to-talk overlay (voice.html). No React, like overlay.ts: voice/overlay.rs owns the
   state and pushes it in whole with window.eval; this renders it, and runs one animation loop
   that springs the island to fit its content, smooths the voice level, and draws the bars. The
   loop only runs while the overlay shows. */

type Phase = "idle" | "listening" | "transcribing" | "thinking" | "speaking" | "asking" | "done" | "error";
type PromptView = { text: string; options: string[]; hint: string };
type View = {
  phase: Phase;
  transcript: string;
  reply: string;
  status: string | null;
  error: string | null;
  prompt: PromptView | null;
  textSize: string;
  highContrast: boolean;
  control: boolean;
};
type Rect = { x: number; y: number; width: number; height: number };

const LABELS: Record<Phase, string> = {
  idle: "",
  listening: "Listening",
  transcribing: "Listening",
  thinking: "Thinking",
  speaking: "Aloe",
  asking: "Aloe is asking",
  done: "Aloe",
  error: "Aloe",
};
const CONTROL_LABEL = "Using your computer";

const $ = (id: string) => document.getElementById(id)!;
const root = $("root");
const glow = $("glow");
const orb = $("orb");
const island = $("island");
const inner = $("inner");
const body = $("body");
const label = $("label");
const said = $("said");
const reply = $("reply");
const status = $("status");
const error = $("error");
const askText = $("ask-text");
const askOptions = $("ask-options");
const askHint = $("ask-hint");
const live = $("live");
const bars = $("bars") as HTMLCanvasElement;
const pen = bars.getContext("2d")!;

const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
/* Where the island sits from the top of the screen, as in voice.html. */
const ISLAND_TOP = 12;

/* The reply is markdown; the island is a glance, not a reader, so it shows the words without the
   syntax. textContent only — nothing from the model is ever parsed as HTML. */
const plain = (markdown: string) =>
  markdown
    .replace(/```[^\n]*\n[\s\S]*?(```|$)/g, "[code]\n")
    .replace(/^\s{0,3}#{1,6}\s+/gm, "")
    .replace(/^\s{0,3}[-*+]\s+/gm, "• ")
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/(\*\*|__|~~|`)/g, "")
    .replace(/\n{3,}/g, "\n\n")
    .trim();

/* Text that grows (a live transcript, a streaming reply) only animates its new words; anything
   else is a rewrite and fades over as a whole. */
const shown = new WeakMap<HTMLElement, string>();
const flow = (element: HTMLElement, next: string) => {
  const before = shown.get(element) ?? "";
  if (next === before) return;
  shown.set(element, next);
  if (!before || !next.startsWith(before)) {
    element.replaceChildren(document.createTextNode(next));
    if (before && next) {
      element.classList.remove("rewrite");
      void element.offsetWidth; // Restart the fade.
      element.classList.add("rewrite");
    }
    return;
  }
  let word = 0;
  for (const piece of next.slice(before.length).split(/(\s+)/)) {
    if (!piece) continue;
    if (/^\s+$/.test(piece)) {
      element.append(piece);
      continue;
    }
    const span = document.createElement("span");
    span.className = "w";
    span.textContent = piece;
    span.style.animationDelay = `${Math.min(word++, 8) * 18}ms`;
    element.append(span);
  }
};

const setLabel = (text: string, control: boolean) => {
  label.toggleAttribute("data-control", control);
  if (label.textContent === text) return;
  label.textContent = text;
  label.classList.remove("swap");
  void label.offsetWidth; // Restart the fade.
  label.classList.add("swap");
};

/* What a screen reader hears, once the words settle, so it reads sentences rather than each word. */
let liveTimer = 0;
const announce = (text: string) => {
  window.clearTimeout(liveTimer);
  liveTimer = window.setTimeout(() => {
    if (live.textContent !== text) live.textContent = text;
  }, 700);
};

let phase: Phase = "idle";
let hideTimer = 0;

const render = (view: View) => {
  // Styling first, so even a fading island keeps the size and contrast the user chose.
  document.documentElement.dataset.size = view.textSize || "m";
  document.documentElement.dataset.contrast = view.highContrast ? "high" : "normal";
  const wasIdle = phase === "idle";
  phase = view.phase;
  root.dataset.phase = view.phase;
  if (view.phase === "idle") {
    // Keep the old text while the island rises away.
    delete root.dataset.open;
    report(null);
    window.clearTimeout(hideTimer);
    hideTimer = window.setTimeout(stop, 700);
    return;
  }
  window.clearTimeout(hideTimer);
  root.dataset.open = "";
  root.toggleAttribute("data-control", view.control);
  if (wasIdle) {
    // Drop in already at its size, not growing from nothing.
    snap = true;
    start();
  }

  const replyText = plain(view.reply);
  const prompt = view.phase === "asking" ? view.prompt : null;
  const working = view.control && (view.phase === "thinking" || view.phase === "transcribing");
  setLabel(working ? CONTROL_LABEL : LABELS[view.phase], working);
  flow(said, view.transcript);
  flow(reply, replyText);
  status.textContent = view.status ?? "";
  error.textContent = view.error ?? "";
  if (askText.textContent !== (prompt?.text ?? "")) {
    askText.textContent = prompt?.text ?? "";
    askHint.textContent = prompt?.hint ?? "";
    askOptions.replaceChildren(
      ...(prompt?.options ?? []).map((option, index) => {
        const item = document.createElement("li");
        item.textContent = option;
        item.style.animationDelay = `${index * 45}ms`;
        return item;
      }),
    );
  }
  root.toggleAttribute("data-replied", Boolean(replyText) && view.phase !== "listening" && view.phase !== "transcribing");
  root.toggleAttribute("data-content", Boolean(view.transcript || replyText || view.status || view.error || prompt));
  announce(
    [working ? CONTROL_LABEL : LABELS[view.phase], prompt ? [prompt.text, ...prompt.options, prompt.hint].join(". ") : replyText || view.transcript, view.error ?? ""]
      .filter(Boolean)
      .join(". "),
  );
};

/* ---- The island's hit area ----
   The window is click-through except where Rust has been told the island is. Its settled size
   is reported, not every frame of the spring. */
let reported = "";
const report = (rect: Rect | null) => {
  const key = rect ? `${Math.round(rect.x)},${Math.round(rect.y)},${Math.round(rect.width)},${Math.round(rect.height)}` : "";
  if (key === reported) return;
  reported = key;
  void invoke("voice_overlay_island", { rect }).catch(() => undefined);
};

/* ---- The animation loop ---- */

const size = { w: 0, h: 0, tw: 0, th: 0 };
let snap = true;
let level = 0;
let levelTarget = 0;
let frame = 0;
let last = 0;

const BARS = 5;
const ENVELOPE = [0.55, 0.8, 1, 0.8, 0.55];

const measure = () => {
  // Layout size, unrounded: the island's drop-in scale must not shrink it, and rounding down
  // would wrap or clip the last word by a pixel.
  const style = getComputedStyle(inner);
  size.tw = Math.ceil(parseFloat(style.width));
  size.th = Math.ceil(parseFloat(style.height));
  body.toggleAttribute("data-clipped", body.scrollHeight > body.clientHeight + 1);
  if (phase !== "idle") report({ x: (window.innerWidth - size.tw) / 2, y: ISLAND_TOP, width: size.tw, height: size.th });
};
new ResizeObserver(measure).observe(inner);

const sizeCanvas = () => {
  const ratio = window.devicePixelRatio || 1;
  const w = Math.max(1, Math.round(bars.offsetWidth * ratio));
  const h = Math.max(1, Math.round(bars.offsetHeight * ratio));
  if (bars.width !== w || bars.height !== h) {
    bars.width = w;
    bars.height = h;
  }
};

/* Each bar's height, 0–1, for this moment. Listening follows the voice; the rest have rhythms of
   their own, so Aloe never looks frozen while it speaks or thinks. */
const heights = (t: number): number[] => {
  const still = reducedMotion.matches;
  return ENVELOPE.map((weight, i) => {
    switch (phase) {
      case "listening":
        return 0.12 + level * weight * (still ? 1 : 0.75 + 0.25 * Math.sin(t * 9 + i * 1.7));
      case "speaking": {
        if (still) return 0.45 * weight;
        const swell = 0.5 + 0.5 * Math.sin(t * 2.3 + 0.6) * Math.sin(t * 5.1 + i * 0.9);
        return 0.18 + weight * (0.2 + 0.45 * swell + 0.12 * Math.sin(t * 13 + i * 2.1));
      }
      case "thinking":
      case "transcribing":
        return still ? 0.2 : 0.14 + 0.32 * Math.max(0, Math.sin(t * 4.2 - i * 0.85));
      case "asking":
        return 0.16 + weight * (still ? 0.12 : 0.12 + 0.08 * Math.sin(t * 2.2));
      default:
        return 0;
    }
  });
};
const drawn = new Array(BARS).fill(0);

const draw = (t: number, dt: number) => {
  sizeCanvas();
  const { width, height } = bars;
  pen.clearRect(0, 0, width, height);
  const targets = heights(t);
  const high = document.documentElement.dataset.contrast === "high";
  pen.fillStyle = high ? "#ffffff" : "rgba(244, 247, 240, 0.92)";
  const gap = width * 0.07;
  const bar = Math.min(height * 0.16, (width * 0.62 - gap * (BARS - 1)) / BARS);
  const span = BARS * bar + (BARS - 1) * gap;
  let x = (width - span) / 2;
  for (let i = 0; i < BARS; i++) {
    // Ease toward the target so a phase change morphs the bars rather than cutting them.
    drawn[i] += (targets[i] - drawn[i]) * (1 - Math.exp(-dt / 0.09));
    const h = Math.max(bar, drawn[i] * height * 0.72);
    if (drawn[i] > 0.01) {
      pen.beginPath();
      pen.roundRect(x, (height - h) / 2, bar, h, bar / 2);
      pen.fill();
    }
    x += bar + gap;
  }
};

const bloom = (t: number) => {
  if ("control" in root.dataset && (phase === "thinking" || phase === "transcribing")) {
    // Driving the computer breathes like the desktop-control glow.
    return reducedMotion.matches ? 0.5 : 0.5 + 0.5 * Math.sin(t * 2);
  }
  switch (phase) {
    case "listening":
      return 0.2 + level * 0.75;
    case "thinking":
    case "transcribing":
      return reducedMotion.matches ? 0.45 : 0.3 + 0.3 * (0.5 + 0.5 * Math.sin(t * 2.2));
    case "speaking":
      return 0.4;
    case "asking":
      return 0.6;
    case "error":
      return 0.35;
    default:
      return 0;
  }
};

const tick = (now: number) => {
  frame = requestAnimationFrame(tick);
  const dt = Math.min((now - (last || now)) / 1000, 1 / 30);
  last = now;
  const t = now / 1000;

  level += (levelTarget - level) * (1 - Math.exp(-dt / 0.08));
  glow.style.setProperty("--bloom", bloom(t).toFixed(3));
  orb.style.setProperty("--halo", (phase === "listening" ? level : phase === "speaking" ? 0.35 : 0.15).toFixed(3));

  // The island grows and shrinks to fit, eased so new lines open it smoothly.
  if (snap || reducedMotion.matches) {
    size.w = size.tw;
    size.h = size.th;
    snap = false;
  } else {
    const ease = 1 - Math.exp(-dt / 0.075);
    size.w += (size.tw - size.w) * ease;
    size.h += (size.th - size.h) * ease;
  }
  island.style.width = `${size.w.toFixed(1)}px`;
  island.style.height = `${size.h.toFixed(1)}px`;

  draw(t, dt);
};

const start = () => {
  if (!frame) {
    last = 0;
    measure();
    frame = requestAnimationFrame(tick);
  }
};
const stop = () => {
  cancelAnimationFrame(frame);
  frame = 0;
  levelTarget = level = 0;
};

const voiceWindow = window as Window & { __aloeVoice?: (view: View) => void; __aloeVoiceLevel?: (level: number) => void };
voiceWindow.__aloeVoice = render;
voiceWindow.__aloeVoiceLevel = (value) => {
  levelTarget = value;
};
// The window is created by the hold that turns it on, so its first state can land before this
// script runs — ask once.
void invoke<View>("voice_overlay_state").then(render, () => undefined);

$("close").addEventListener("click", () => {
  void invoke("voice_stop").catch(() => undefined);
});
$("stop").addEventListener("click", () => {
  void invoke("stop_desktop_control").catch(() => undefined);
});

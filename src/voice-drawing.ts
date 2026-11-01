import { invoke } from "@tauri-apps/api/core";

/* Marks Aloe draws over the screen during a voice reply (voice/drawing.rs): highlights, boxes,
   circles, underlines, arrows, curves, labels and notes, on top of whatever is showing. Positions
   are on the 0–1000 grid of the voice turn's screenshot, which is of this display, so a mark lands
   on what Aloe saw. Shapes are SVG; words are HTML pills, filled with textContent only. New marks
   reveal one after another in the order Aloe listed them, so they follow its explanation. */

type Point = { x: number; y: number };
type Mark = {
  type: "highlight" | "box" | "circle" | "underline" | "arrow" | "line" | "curve" | "label" | "text";
  x?: number;
  y?: number;
  w?: number;
  h?: number;
  points?: Point[];
  target?: Point;
  text?: string;
  size?: "small" | "normal" | "large";
  color: string;
  dashed?: boolean;
};
type Drawing = { marks: Mark[]; freshFrom: number };
type Rect = { x: number; y: number; width: number; height: number };

const COLORS: Record<string, string> = {
  yellow: "#ffd400",
  green: "#30d158",
  red: "#ff453a",
  blue: "#0a84ff",
  orange: "#ff9f0a",
  purple: "#bf5af2",
  white: "#ffffff",
};
const STAGGER_MS = 380;
const SVG = "http://www.w3.org/2000/svg";

const layer = document.getElementById("marks") as unknown as SVGSVGElement;
const notes = document.getElementById("notes")!;
const chip = document.getElementById("clear-marks")!;
const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

let drawing: Drawing = { marks: [], freshFrom: 0 };

const px = (p: Point) => ({ x: (p.x / 1000) * window.innerWidth, y: (p.y / 1000) * window.innerHeight });
const colorOf = (mark: Mark) => COLORS[mark.color] ?? COLORS.red;

const shape = <K extends keyof SVGElementTagNameMap>(tag: K, attributes: Record<string, string | number>) => {
  const element = document.createElementNS(SVG, tag);
  for (const [name, value] of Object.entries(attributes)) element.setAttribute(name, String(value));
  return element;
};

/* A smooth path through every point (Catmull-Rom as cubic Béziers). */
const smooth = (points: { x: number; y: number }[]) => {
  let d = `M${points[0].x},${points[0].y}`;
  for (let i = 0; i < points.length - 1; i++) {
    const p0 = points[i - 1] ?? points[i];
    const [p1, p2] = [points[i], points[i + 1]];
    const p3 = points[i + 2] ?? p2;
    d += ` C${p1.x + (p2.x - p0.x) / 6},${p1.y + (p2.y - p0.y) / 6} ${p2.x - (p3.x - p1.x) / 6},${p2.y - (p3.y - p1.y) / 6} ${p2.x},${p2.y}`;
  }
  return d;
};
const straight = (points: { x: number; y: number }[]) => points.map((p, i) => `${i ? "L" : "M"}${p.x},${p.y}`).join(" ");

/* An arrowhead at `tip`, pointing away from `from`. */
const head = (from: { x: number; y: number }, tip: { x: number; y: number }, color: string) => {
  const angle = Math.atan2(tip.y - from.y, tip.x - from.x);
  const size = 16;
  const wing = (turn: number) => `${tip.x - size * Math.cos(angle + turn)},${tip.y - size * Math.sin(angle + turn)}`;
  return shape("polygon", { points: `${tip.x},${tip.y} ${wing(0.45)} ${wing(-0.45)}`, fill: color, class: "pop" });
};

const stroke = (d: string, mark: Mark, extra: Record<string, string | number> = {}) =>
  shape("path", {
    d,
    fill: "none",
    stroke: colorOf(mark),
    "stroke-width": 3.5,
    "stroke-linecap": "round",
    "stroke-linejoin": "round",
    pathLength: 1,
    class: mark.dashed ? "dashed fade" : "ink",
    ...extra,
  });

const pill = (mark: Mark, at: { x: number; y: number }) => {
  const note = document.createElement("div");
  note.className = `note ${mark.size ?? "normal"}`;
  note.style.setProperty("--accent", colorOf(mark));
  note.style.left = `${at.x}px`;
  note.style.top = `${at.y}px`;
  note.textContent = mark.text ?? "";
  return note;
};

/* One mark as SVG pieces (in drawing order) and any pill. */
const build = (mark: Mark): { pieces: SVGElement[]; note?: HTMLElement } => {
  const color = colorOf(mark);
  const hasBox = mark.x !== undefined && mark.y !== undefined;
  const corner = hasBox ? px({ x: mark.x!, y: mark.y! }) : { x: 0, y: 0 };
  const far = hasBox ? px({ x: mark.x! + (mark.w ?? 0), y: mark.y! + (mark.h ?? 0) }) : { x: 0, y: 0 };
  const [w, h] = [far.x - corner.x, far.y - corner.y];
  switch (mark.type) {
    case "highlight":
      return { pieces: [shape("rect", { x: corner.x - 3, y: corner.y - 2, width: w + 6, height: h + 4, rx: 4, fill: color, class: "marker" })] };
    case "box":
      return { pieces: [stroke(`M${corner.x - 5},${corner.y - 5} h${w + 10} v${h + 10} h${-(w + 10)} Z`, mark)] };
    case "circle": {
      // Drawn by hand: a loop around the box that overshoots where it started.
      const [cx, cy, rx, ry] = [corner.x + w / 2, corner.y + h / 2, w * 0.6 + 10, h * 0.6 + 10];
      const loop = Array.from({ length: 41 }, (_, i) => {
        const t = -Math.PI * 0.6 + (i / 40) * Math.PI * 2.12;
        const wobble = 1 + 0.025 * Math.sin(t * 1.5);
        return { x: cx + rx * wobble * Math.cos(t), y: cy + ry * wobble * Math.sin(t) };
      });
      return { pieces: [stroke(smooth(loop), mark)] };
    }
    case "underline":
      return { pieces: [stroke(`M${corner.x},${far.y + 4} Q${corner.x + w / 2},${far.y + 7} ${far.x},${far.y + 3}`, mark)] };
    case "arrow":
    case "line":
    case "curve": {
      const points = (mark.points ?? []).map(px);
      if (points.length < 2) return { pieces: [] };
      const pieces: SVGElement[] = [stroke(mark.type === "curve" ? smooth(points) : straight(points), mark)];
      if (mark.type === "arrow") pieces.push(head(points[points.length - 2], points[points.length - 1], color));
      return { pieces };
    }
    case "label": {
      if (!hasBox || !mark.target) return { pieces: [] };
      const target = px(mark.target);
      return {
        pieces: [
          stroke(`M${corner.x},${corner.y} L${target.x},${target.y}`, mark, { "stroke-width": 2.5 }),
          shape("circle", { cx: target.x, cy: target.y, r: 5, fill: color, class: "pop" }),
        ],
        note: pill(mark, corner),
      };
    }
    case "text":
      return hasBox ? { pieces: [], note: pill(mark, corner) } : { pieces: [] };
    default:
      return { pieces: [] };
  }
};

/* ---- The clear chip's hit area, as for the island ---- */
let reported = "";
const reportChip = (rect: Rect | null) => {
  const key = rect ? `${Math.round(rect.x)},${Math.round(rect.y)},${Math.round(rect.width)},${Math.round(rect.height)}` : "";
  if (key === reported) return;
  reported = key;
  void invoke("voice_overlay_chip", { rect }).catch(() => undefined);
};
const placeChip = () => {
  if (!drawing.marks.length) return reportChip(null);
  const box = chip.getBoundingClientRect();
  reportChip({ x: box.x, y: box.y, width: box.width, height: box.height });
};

const FADE_MS = 450;
let fadeTimer = 0;

const render = (next: Drawing, animate: boolean) => {
  const root = document.getElementById("root")!;
  const hadMarks = drawing.marks.length > 0;
  drawing = next;
  window.clearTimeout(fadeTimer);
  if (!next.marks.length) {
    // Fade out what is there (CSS, off data-marks), then take it away.
    root.removeAttribute("data-marks");
    reportChip(null);
    const empty = () => {
      layer.replaceChildren();
      notes.replaceChildren();
    };
    if (hadMarks && animate && !reducedMotion.matches) fadeTimer = window.setTimeout(empty, FADE_MS);
    else empty();
    return;
  }
  layer.replaceChildren();
  notes.replaceChildren();
  root.setAttribute("data-marks", "");
  layer.setAttribute("viewBox", `0 0 ${window.innerWidth} ${window.innerHeight}`);
  let order = 0;
  next.marks.forEach((mark, index) => {
    const { pieces, note } = build(mark);
    const fresh = animate && index >= next.freshFrom && !reducedMotion.matches;
    const delay = fresh ? order++ * STAGGER_MS : 0;
    // Leader lines go under every pill, so a pill is never crossed out by another mark's line.
    const group = shape("g", { class: fresh ? "mark fresh" : "mark" });
    group.style.setProperty("--delay", `${delay}ms`);
    group.append(...pieces);
    layer.append(group);
    if (note) {
      if (fresh) note.classList.add("fresh");
      note.style.setProperty("--delay", `${delay + (pieces.length ? 260 : 0)}ms`);
      notes.append(note);
    }
  });
  // Pills near an edge are nudged back onto the screen once they have a size.
  requestAnimationFrame(() => {
    for (const note of Array.from(notes.children) as HTMLElement[]) {
      const box = note.getBoundingClientRect();
      const dx = Math.max(8 - box.left, 0) + Math.min(window.innerWidth - 8 - box.right, 0);
      const dy = Math.max(8 - box.top, 0) + Math.min(window.innerHeight - 8 - box.bottom, 0);
      if (dx || dy) note.style.translate = `${dx}px ${dy}px`;
    }
    placeChip();
  });
};

const drawWindow = window as Window & { __aloeDraw?: (next: Drawing) => void };
drawWindow.__aloeDraw = (next) => render(next, true);
window.addEventListener("resize", () => render({ marks: drawing.marks, freshFrom: drawing.marks.length }, false));
void invoke<Drawing>("voice_overlay_drawing").then((next) => render(next, false), () => undefined);

chip.addEventListener("click", () => {
  void invoke("voice_clear_drawing").catch(() => undefined);
});

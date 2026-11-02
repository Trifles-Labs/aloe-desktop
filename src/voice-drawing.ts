import { invoke } from "@tauri-apps/api/core";
import { write } from "./voice-writing";

/* Marks Aloe draws over the screen during a voice reply (voice/drawing.rs): highlights, boxes,
   circles, underlines, arrows, curves, polygons, ellipses, labels, notes and handwriting
   (voice-writing.ts), on top of whatever is showing. Positions are on the 0–1000 grid of the voice
   turn's screenshot, which is of this display, so a mark lands on what Aloe saw. Shapes are SVG;
   words are HTML, filled with textContent only.

   New marks are drawn one after another in the order Aloe listed them, so they follow its
   explanation, each at the pace of a hand: a stroke takes as long as it is long, with a pen tip
   travelling along it. Shapes wobble a little, seeded by the mark so a redraw looks the same. */

type Point = { x: number; y: number };
type Mark = {
  type: "highlight" | "box" | "circle" | "underline" | "ellipse" | "arrow" | "line" | "curve" | "polygon" | "label" | "text" | "write";
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
  fill?: boolean;
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
  black: "#1f2430",
};
/** How fast the pen moves, in pixels per millisecond. */
const PEN_SPEED = 1.5;
const MARK_GAP_MS = 140;
const SVG = "http://www.w3.org/2000/svg";

const layer = document.getElementById("marks") as unknown as SVGSVGElement;
const notes = document.getElementById("notes")!;
const chip = document.getElementById("clear-marks")!;
const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

let drawing: Drawing = { marks: [], freshFrom: 0 };

const px = (p: Point) => ({ x: (p.x / 1000) * window.innerWidth, y: (p.y / 1000) * window.innerHeight });
const colorOf = (mark: Mark) => COLORS[mark.color] ?? COLORS.red;
const clamp = (n: number, low: number, high: number) => Math.min(high, Math.max(low, n));

/* A steady source of wobble for one mark: the same mark always wobbles the same way. */
const wobbleFor = (mark: Mark) => {
  let seed = Math.round((mark.x ?? 0) * 31 + (mark.y ?? 0) * 17 + (mark.w ?? 0) * 7 + (mark.h ?? 0) * 3 + (mark.points?.[0]?.x ?? 0) * 13) % 2147483646;
  if (seed <= 0) seed += 2147483646;
  return (spread: number) => {
    seed = (seed * 16807) % 2147483647;
    return ((seed - 1) / 2147483646 - 0.5) * spread;
  };
};

const shape = <K extends keyof SVGElementTagNameMap>(tag: K, attributes: Record<string, string | number>) => {
  const element = document.createElementNS(SVG, tag);
  for (const [name, value] of Object.entries(attributes)) element.setAttribute(name, String(value));
  return element;
};

/* A smooth path through every point (Catmull-Rom as cubic Béziers). */
const smooth = (points: Point[]) => {
  let d = `M${points[0].x},${points[0].y}`;
  for (let i = 0; i < points.length - 1; i++) {
    const p0 = points[i - 1] ?? points[i];
    const [p1, p2] = [points[i], points[i + 1]];
    const p3 = points[i + 2] ?? p2;
    d += ` C${p1.x + (p2.x - p0.x) / 6},${p1.y + (p2.y - p0.y) / 6} ${p2.x - (p3.x - p1.x) / 6},${p2.y - (p3.y - p1.y) / 6} ${p2.x},${p2.y}`;
  }
  return d;
};
const straight = (points: Point[]) => points.map((p, i) => `${i ? "L" : "M"}${p.x},${p.y}`).join(" ");

/* A pen stroke, drawn on along its length with the pen tip following (unless dashed). */
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
    ...(mark.dashed ? {} : { "data-pen": d }),
    ...extra,
  });

/* An arrowhead at `tip`, pointing away from `from`: two quick strokes after the shaft. */
const head = (from: Point, tip: Point, mark: Mark) => {
  const angle = Math.atan2(tip.y - from.y, tip.x - from.x);
  const size = 17;
  const wing = (turn: number) => `${tip.x - size * Math.cos(angle + turn)},${tip.y - size * Math.sin(angle + turn)}`;
  return stroke(`M${wing(0.5)} L${tip.x},${tip.y} L${wing(-0.5)}`, { ...mark, dashed: false });
};

/* A figure's outline, with a light wash inside it after the outline is drawn when asked for. */
const figure = (d: string, mark: Mark) => {
  const pieces: SVGElement[] = [stroke(d, mark)];
  if (mark.fill) pieces.push(shape("path", { d, fill: colorOf(mark), stroke: "none", class: "wash fade" }));
  return pieces;
};

const pill = (mark: Mark, at: Point) => {
  const note = document.createElement("div");
  note.className = `note ${mark.size ?? "normal"}`;
  note.style.setProperty("--accent", colorOf(mark));
  note.style.left = `${at.x}px`;
  note.style.top = `${at.y}px`;
  note.textContent = mark.text ?? "";
  return note;
};

/* One mark as SVG pieces (in drawing order), and any words with how long they take to write. */
const build = (mark: Mark): { pieces: SVGElement[]; note?: HTMLElement; writes?: number } => {
  const color = colorOf(mark);
  const wobble = wobbleFor(mark);
  const hasBox = mark.x !== undefined && mark.y !== undefined;
  const corner = hasBox ? px({ x: mark.x!, y: mark.y! }) : { x: 0, y: 0 };
  const far = hasBox ? px({ x: mark.x! + (mark.w ?? 0), y: mark.y! + (mark.h ?? 0) }) : { x: 0, y: 0 };
  const [w, h] = [far.x - corner.x, far.y - corner.y];
  switch (mark.type) {
    case "highlight": {
      // A marker swipe: slightly uneven edges and rounded ends, swept left to right.
      const [left, top, right, bottom] = [corner.x - 4, corner.y - 2, far.x + 4, far.y + 2];
      const middle = (top + bottom) / 2;
      const d = `M${left},${top + wobble(2)} L${right},${top + wobble(3)} Q${right + 5},${middle} ${right},${bottom + wobble(3)} L${left},${bottom + wobble(2)} Q${left - 5},${middle} ${left},${top} Z`;
      return { pieces: [shape("path", { d, fill: color, class: "marker", "data-pen": `M${left},${middle} L${right},${middle}` })] };
    }
    case "box": {
      // Four sides in one go, corners a little off, ending just past where it began.
      const [left, top, right, bottom] = [corner.x - 6, corner.y - 6, far.x + 6, far.y + 6];
      const at = (x: number, y: number) => ({ x: x + wobble(4), y: y + wobble(4) });
      const points = [at(left, top + 3), at(right, top), at(right, bottom), at(left, bottom), at(left, top), { x: left + 12, y: top - 1 + wobble(2) }];
      return { pieces: [stroke(straight(points), mark)] };
    }
    case "circle": {
      // Drawn by hand: a loop all round the box, tilted a touch, drifting outward so it overshoots
      // past where it started. A wide thing (a word) gets a flat loop that hugs its ends instead
      // or the lines above and below it, a square one a round loop.
      const round = clamp((4 - Math.max(w, h) / Math.max(1, Math.min(w, h))) / 3, 0, 1);
      const [along, across] = [1.18 + (Math.SQRT2 - 1.18) * round, 1.5 + (Math.SQRT2 - 1.5) * round];
      const [sx, sy] = w >= h ? [along, across] : [across, along];
      const [cx, cy, rx, ry] = [corner.x + w / 2, corner.y + h / 2, (w / 2) * sx + 8, (h / 2) * sy + 5];
      const [tilt, start, sweep, phase] = [wobble(0.08), -Math.PI * 0.62 + wobble(0.5), Math.PI * 2 * (1.09 + wobble(0.06)), wobble(Math.PI * 2)];
      const loop = Array.from({ length: 49 }, (_, i) => {
        const along = i / 48;
        const t = start + along * sweep;
        const grow = 1 + 0.025 * Math.sin(t * 1.5 + phase) + 0.05 * along;
        const [ex, ey] = [rx * grow * Math.cos(t), ry * grow * Math.sin(t)];
        return { x: cx + ex * Math.cos(tilt) - ey * Math.sin(tilt), y: cy + ex * Math.sin(tilt) + ey * Math.cos(tilt) };
      });
      return { pieces: [stroke(smooth(loop), mark)] };
    }
    case "ellipse": {
      // Exact, as part of a figure: two half-turns from the left, clockwise.
      const [rx, ry, cy] = [w / 2, h / 2, corner.y + h / 2];
      return { pieces: figure(`M${corner.x},${cy} A${rx},${ry} 0 1,1 ${far.x},${cy} A${rx},${ry} 0 1,1 ${corner.x},${cy}`, mark) };
    }
    case "polygon": {
      const points = (mark.points ?? []).map(px);
      return points.length < 3 ? { pieces: [] } : { pieces: figure(`${straight(points)} Z`, mark) };
    }
    case "underline": {
      const base = far.y + 4;
      return { pieces: [stroke(`M${corner.x - 2},${base + wobble(2)} Q${corner.x + w / 2},${base + 3 + wobble(2)} ${far.x + 4},${base - 1 + wobble(2)}`, mark)] };
    }
    case "arrow":
    case "line":
    case "curve": {
      const points = (mark.points ?? []).map(px);
      if (points.length < 2) return { pieces: [] };
      const pieces: SVGElement[] = [stroke(mark.type === "curve" ? smooth(points) : straight(points), mark)];
      if (mark.type === "arrow") pieces.push(head(points[points.length - 2], points[points.length - 1], mark));
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
    case "write": {
      if (!hasBox) return { pieces: [] };
      const { element, duration } = write(mark.text ?? "");
      const note = document.createElement("div");
      note.className = `write ${mark.size ?? "normal"}`;
      note.dataset.ink = mark.color === "white" || mark.color === "yellow" ? "light" : "dark";
      note.style.setProperty("--accent", color);
      note.style.left = `${corner.x}px`;
      note.style.top = `${corner.y}px`;
      const room = window.innerWidth - corner.x - 16;
      note.style.maxWidth = `${Math.max(160, mark.w ? Math.min(w, room) : Math.min(window.innerWidth * 0.46, room))}px`;
      note.append(element);
      return { pieces: [], note, writes: duration };
    }
    default:
      return { pieces: [] };
  }
};

/* The pen tip, travelling along `d` while its stroke is drawn on. */
const penAlong = (d: string, at: number, ms: number, color: string) => {
  const pen = document.createElement("div");
  pen.className = "pen";
  pen.style.setProperty("offset-path", `path("${d}")`);
  pen.style.setProperty("--accent", color);
  pen.style.setProperty("--delay", `${Math.round(at)}ms`);
  pen.style.setProperty("--dur", `${Math.round(ms)}ms`);
  notes.append(pen);
};

/* When one piece is drawn and for how long, from `at`; returns when the next one starts. */
const schedule = (piece: SVGElement, at: number): number => {
  let ms = 220;
  if (piece.classList.contains("ink")) ms = clamp((piece as SVGGeometryElement).getTotalLength() / PEN_SPEED, 160, 1200);
  else if (piece.classList.contains("marker")) ms = clamp((piece as SVGGraphicsElement).getBBox().width / (PEN_SPEED * 1.3), 200, 900);
  piece.style.setProperty("--delay", `${Math.round(at)}ms`);
  piece.style.setProperty("--dur", `${Math.round(ms)}ms`);
  const pen = piece.getAttribute("data-pen");
  if (pen) penAlong(pen, at, ms, piece.getAttribute("stroke") ?? piece.getAttribute("fill") ?? "#fff");
  return at + (piece.classList.contains("pop") ? 120 : ms);
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
  let clock = 0;
  next.marks.forEach((mark, index) => {
    const { pieces, note, writes } = build(mark);
    const fresh = animate && index >= next.freshFrom && !reducedMotion.matches;
    // Leader lines go under every pill, so a pill is never crossed out by another mark's line.
    const group = shape("g", { class: fresh ? "mark fresh" : "mark" });
    group.append(...pieces);
    layer.append(group);
    if (note) {
      if (fresh) note.classList.add("fresh");
      notes.append(note);
    }
    if (!fresh) return;
    // Measured once in the page: each piece after the last, then its words.
    let at = clock;
    for (const piece of pieces) at = schedule(piece, at);
    if (note) {
      note.style.setProperty("--delay", `${Math.round(at)}ms`);
      at += writes ?? 260;
    }
    clock = at + MARK_GAP_MS;
  });
  // Words near an edge are nudged back onto the screen once they have a size.
  requestAnimationFrame(() => {
    for (const note of Array.from(notes.querySelectorAll<HTMLElement>(".note, .write"))) {
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
window.addEventListener("resize", () => render({ ...drawing, freshFrom: drawing.marks.length }, false));
void invoke<Drawing>("voice_overlay_drawing").then((next) => render(next, false), () => undefined);

chip.addEventListener("click", () => {
  void invoke("voice_clear_drawing").catch(() => undefined);
});

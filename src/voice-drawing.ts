import { invoke } from "@tauri-apps/api/core";
import { markdownBlocks, write } from "./voice-writing";

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
  blue: "#4da3ff",
  orange: "#ff9f0a",
  purple: "#c983ff",
  white: "#ffffff",
  black: "#1f2430",
};
/** How fast the pen moves, in pixels per millisecond. */
const PEN_SPEED = 1.5;
const MARK_GAP_MS = 140;
/** However many marks come at once, they are all drawn by then. */
const DRAW_BUDGET_MS = 5000;
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
  if (mark.color === "black") note.dataset.paper = "light";
  note.style.setProperty("--accent", colorOf(mark));
  note.style.left = `${at.x}px`;
  note.style.top = `${at.y}px`;
  const text = mark.text ?? "";
  // A tag with maths in it ("$8$ cm", "$45^\circ$") is set by KaTeX like handwriting is.
  if (/\$[^$]+\$|\\\(.+?\\\)/.test(text)) note.append(...markdownBlocks(text).map((block) => block.element));
  else note.textContent = text;
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
      // The tag has no chip to hide a line under it, so the line starts at the edge of its words
      // (roughly: an ellipse around the text) rather than crossing them from the centre.
      const [dx, dy] = [target.x - corner.x, target.y - corner.y];
      const length = Math.hypot(dx, dy) || 1;
      const [rx, ry] = [Math.min(150, (mark.text ?? "").length * 6.5 + 8), 20];
      const edge = Math.min(length * 0.8, 1 / Math.hypot(dx / length / rx, dy / length / ry));
      const start = { x: corner.x + (dx / length) * edge, y: corner.y + (dy / length) * edge };
      return {
        pieces: [
          stroke(`M${start.x},${start.y} L${target.x},${target.y}`, mark, { "stroke-width": 2.5 }),
          shape("circle", { cx: target.x, cy: target.y, r: 4, fill: color, class: "pop" }),
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
      if (mark.color === "black") note.dataset.paper = "light";
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

/* How long one piece takes to draw at the pen's own pace. */
const durationOf = (piece: SVGElement): number => {
  if (piece.classList.contains("ink")) return clamp((piece as SVGGeometryElement).getTotalLength() / PEN_SPEED, 160, 1200);
  if (piece.classList.contains("marker")) return clamp((piece as SVGGraphicsElement).getBBox().width / (PEN_SPEED * 1.3), 200, 900);
  return 220;
};
/* How long it holds the next piece back: a dot pops in quicker than it animates. */
const stepOf = (piece: SVGElement, ms: number) => (piece.classList.contains("pop") ? 120 : ms);

/* When one piece is drawn and for how long, from `at`, at `pace` (1 is the pen's own speed); returns
   when the next one starts. */
const schedule = (piece: SVGElement, at: number, pace: number): number => {
  const ms = durationOf(piece) * pace;
  piece.style.setProperty("--delay", `${Math.round(at)}ms`);
  piece.style.setProperty("--dur", `${Math.round(ms)}ms`);
  const pen = piece.getAttribute("data-pen");
  if (pen) penAlong(pen, at, ms, piece.getAttribute("stroke") ?? piece.getAttribute("fill") ?? "#fff");
  return at + stepOf(piece, ms);
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

type Area = { left: number; right: number; top: number; bottom: number };
/** Space kept between a moved piece of handwriting and the words above it. */
const WRITE_GAP = 8;

/** Handwriting this short ("A", "7 cm", "$45^\circ$") names part of a figure and is meant to sit
 * right by its strokes, so only words and tags move it, not the figure. */
const SHORT_WRITE = 12;
/** Spacing of the points sampled along each stroke, px. */
const INK_STEP = 6;

/* Points along every stroke drawn (lines, rays, lenses, shapes), to tell whether handwriting would
   cover a figure. Sampled along the stroke, not its bounding box, so a long diagonal ray doesn't
   claim the whole rectangle around it. */
const inkPoints = (): { x: number; y: number }[] => {
  const points: { x: number; y: number }[] = [];
  for (const shape of Array.from(layer.querySelectorAll<SVGGeometryElement>("path, line, polyline, polygon, ellipse, circle, rect"))) {
    let length = 0;
    try {
      length = shape.getTotalLength();
    } catch {
      continue;
    }
    for (let at = 0; at <= length; at += INK_STEP) {
      const p = shape.getPointAtLength(at);
      points.push({ x: p.x, y: p.y });
    }
  }
  return points;
};

/* How far down a piece of handwriting must move to clear what is already there: other writing and
   tags, and (unless it is a short label) the strokes of figures. Aloe picks where each one goes
   without knowing how tall its maths will set (a nested fraction stands three lines high), so
   working written in a column can land on the next line, or on a diagram drawn below it. */
const clearBelow = (box: DOMRect, taken: Area[], ink: { x: number; y: number }[]): number => {
  let down = 0;
  for (let round = 0; round < 200; round++) {
    const [top, bottom] = [box.top + down, box.bottom + down];
    const words = taken.filter((other) => box.left < other.right && box.right > other.left && top < other.bottom && bottom > other.top);
    if (words.length) {
      down = Math.max(...words.map((other) => other.bottom)) + WRITE_GAP - box.top;
      continue;
    }
    const pad = 4;
    if (!ink.some((p) => p.x > box.left - pad && p.x < box.right + pad && p.y > top - pad && p.y < bottom + pad)) break;
    down += INK_STEP;
  }
  return down;
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
  const fresh: { pieces: SVGElement[]; note?: HTMLElement; writes?: number }[] = [];
  next.marks.forEach((mark, index) => {
    const built = build(mark);
    const isFresh = animate && index >= next.freshFrom && !reducedMotion.matches;
    // Leader lines go under every pill, so a pill is never crossed out by another mark's line.
    const group = shape("g", { class: isFresh ? "mark fresh" : "mark" });
    group.append(...built.pieces);
    layer.append(group);
    if (built.note) {
      if (isFresh) built.note.classList.add("fresh");
      notes.append(built.note);
    }
    if (isFresh) fresh.push(built);
  });
  // Measured once in the page: each piece after the last, then its words. At the pen's own pace a
  // diagram and its working took half a minute to appear, so a batch that would run past
  // DRAW_BUDGET_MS is sped up, every stroke and word alike, to finish in it.
  const natural = fresh.reduce(
    (sum, { pieces, note, writes }) => sum + pieces.reduce((s, piece) => s + stepOf(piece, durationOf(piece)), 0) + (note ? (writes ?? 260) : 0) + MARK_GAP_MS,
    0,
  );
  const pace = natural > DRAW_BUDGET_MS ? DRAW_BUDGET_MS / natural : 1;
  let clock = 0;
  for (const { pieces, note, writes } of fresh) {
    let at = clock;
    for (const piece of pieces) at = schedule(piece, at, pace);
    if (note) {
      note.style.setProperty("--delay", `${Math.round(at)}ms`);
      note.style.setProperty("--pace", String(pace));
      at += (writes ?? 260) * pace;
    }
    clock = at + MARK_GAP_MS * pace;
  }
  // Once the words have a size: handwriting that lands on earlier words moves down below them, and
  // anything near an edge is nudged back onto the screen.
  requestAnimationFrame(() => {
    const all = Array.from(notes.querySelectorAll<HTMLElement>(".note, .write"));
    // Tags stay where they are, and every one is in the way from the start, wherever it comes in
    // the order; handwriting moves around them, around earlier handwriting, and around figures.
    const area = (box: DOMRect, dx = 0, dy = 0): Area => ({ left: box.left + dx, right: box.right + dx, top: box.top + dy, bottom: box.bottom + dy });
    const taken: Area[] = all.filter((note) => note.classList.contains("note")).map((note) => area(note.getBoundingClientRect()));
    const ink = inkPoints();
    for (const note of all) {
      const box = note.getBoundingClientRect();
      const writing = note.classList.contains("write");
      const short = (note.textContent ?? "").trim().length <= SHORT_WRITE;
      const down = writing ? clearBelow(box, taken, short ? [] : ink) : 0;
      const dx = Math.max(8 - box.left, 0) + Math.min(window.innerWidth - 8 - box.right, 0);
      const dy = down + Math.max(8 - box.top - down, 0) + Math.min(window.innerHeight - 8 - box.bottom - down, 0);
      if (dx || dy) note.style.translate = `${dx}px ${dy}px`;
      if (writing) taken.push(area(box, dx, dy));
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

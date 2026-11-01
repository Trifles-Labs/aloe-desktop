#!/usr/bin/env bun
/**
 * Build Aloe's agent-cursor theme for Cua Driver: the dotLottie source in
 * src-tauri/resources/aloe-cursor/aloe.lottie and, compiled from it by the driver's own
 * `cursor-theme build`, the aloe.cua-theme that tauri.conf.json bundles and cua.rs installs.
 *
 * Usage (needs the driver from scripts/fetch_cua_driver.py):
 *     bun scripts/build_cursor_theme.ts
 *
 * Both outputs are checked in, so a release build does not run this. Rerun it after changing
 * the artwork here, and bump THEME_VERSION so installs pick the new one up.
 *
 * Adapted from Cua's build_default_theme.py (MIT, github.com/trycua/cua,
 * libs/cua-driver/rust/crates/cursor-overlay/assets): the same pointer silhouette, hotspot and
 * twelve action cues, redrawn in Aloe's moss and cream with a leaf in the pointer. The format is
 * described in libs/cua-driver/docs/cursor-themes.md there.
 */

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

export const THEME_ID = "app.aloe.cursor";
const THEME_VERSION = "1.0.0";

const CANVAS = 128;
const FPS = 30;
// --moss-deep and --moss from overlay.html, and its --cream.
const MOSS_DEEP = [47 / 255, 107 / 255, 79 / 255, 1];
const MOSS = [123 / 255, 189 / 255, 150 / 255, 1];
const CREAM = [244 / 255, 247 / 255, 240 / 255, 1];

const ROOT = resolve(import.meta.dir, "..");
const OUT_DIR = join(ROOT, "src-tauri", "resources", "aloe-cursor");
const DRIVER = join(ROOT, "src-tauri", "resources", "cua-driver", process.platform === "win32" ? "cua-driver.exe" : "cua-driver");

type Json = Record<string, unknown>;
type Point = [number, number];
type PathPoint = [Point, Point, Point];
type Keyframes = [number, number | number[]][];

const value = (v: number | number[]) => (typeof v === "number" ? [v] : v);
const fixed = (k: unknown): Json => ({ a: 0, k });

function animated(values: Keyframes, linear = false): Json {
  const k = values.map(([t, v], index) => {
    const item: Json = { t, s: value(v) };
    if (index + 1 < values.length) {
      item.e = value(values[index + 1][1]);
      item.i = linear ? { x: [0.833], y: [0.833] } : { x: [0.25], y: [1.0] };
      item.o = linear ? { x: [0.167], y: [0.167] } : { x: [0.2], y: [0.75] };
    }
    return item;
  });
  return { a: 1, k };
}

function pathShape(pts: PathPoint[], name: string, closed = false): Json {
  return {
    ty: "sh",
    nm: name,
    ks: fixed({ i: pts.map((p) => p[1]), o: pts.map((p) => p[2]), v: pts.map((p) => p[0]), c: closed }),
  };
}

const points = (coordinates: Point[], name: string, closed = false) =>
  pathShape(coordinates.map((c): PathPoint => [c, [0, 0], [0, 0]]), name, closed);

const ellipse = (x: number, y: number, w: number, h: number, name: string): Json => ({
  ty: "el", nm: name, p: fixed([x, y]), s: fixed([w, h]), d: 1,
});

const rectangle = (x: number, y: number, w: number, h: number, roundness: number, name: string): Json => ({
  ty: "rc", nm: name, p: fixed([x, y]), s: fixed([w, h]), r: fixed(roundness), d: 1,
});

const stroke = (color: number[], width: number, opacity = 100): Json => ({
  ty: "st", c: fixed(color), o: fixed(opacity), w: fixed(width), lc: 2, lj: 2, ml: 4,
});

const fill = (color: number[], opacity = 100): Json => ({ ty: "fl", c: fixed(color), o: fixed(opacity), r: 1 });

const shapeTransform = (): Json => ({
  ty: "tr", p: fixed([0, 0]), a: fixed([0, 0]), s: fixed([100, 100]), r: fixed(0), o: fixed(100), sk: fixed(0), sa: fixed(0),
});

interface Motion {
  position?: Json;
  scale?: Json;
  rotation?: Json;
  opacity?: Json;
}

function shapeLayer(index: number, name: string, shapes: Json[], frames: number, motion: Motion = {}): Json {
  return {
    ddd: 0,
    ind: index,
    ty: 4,
    nm: name,
    sr: 1,
    ks: {
      o: motion.opacity ?? fixed(100),
      r: motion.rotation ?? fixed(0),
      p: motion.position ?? fixed([64, 64]),
      a: fixed([64, 64]),
      s: motion.scale ?? fixed([100, 100]),
    },
    ao: 0,
    shapes: [...shapes, shapeTransform()],
    ip: 0,
    op: frames,
    st: 0,
    bm: 0,
  };
}

// Cua's pointer silhouette, so the hotspot (55, 30) stays on its tip.
const CURSOR_PATH = pathShape(
  [
    [[55, 30], [0, 0], [-7, -2]],
    [[43, 41], [-1, -8], [0, 0]],
    [[64, 98], [0, 0], [3, 8]],
    [[77, 99], [-4, 7], [0, 0]],
    [[86, 79], [0, 0], [2, -4]],
    [[95, 70], [-4, 2], [0, 0]],
    [[108, 63], [0, 0], [7, -4]],
    [[107, 50], [7, 3], [0, 0]],
  ],
  "Cursor",
  true,
);

// An aloe leaf inside the pointer: rising from the tail toward the tip, with its midrib.
const LEAF_PATH = pathShape(
  [
    [[64, 88], [11, -9], [-11, -15]],
    [[74, 50], [-9, 10], [8, 12]],
  ],
  "Leaf",
  true,
);
const LEAF_RIB = points([[66, 82], [72, 57]], "Leaf rib");

function cursorLayers(frames: number, motion: Motion = {}): Json[] {
  const glow = [
    [44, 2.0], [36, 2.4], [29, 3.0], [23, 3.8], [18, 4.8], [14, 6.0], [10, 7.5], [7, 9.5],
  ].map(([width, opacity], i) => shapeLayer(i + 1, `Cursor glow ${width}`, [CURSOR_PATH, stroke(MOSS, width, opacity)], frames, motion));
  return [
    ...glow,
    shapeLayer(9, "Cursor body", [CURSOR_PATH, fill(MOSS_DEEP), stroke(CREAM, 5)], frames, motion),
    shapeLayer(10, "Leaf", [LEAF_PATH, fill(MOSS)], frames, motion),
    shapeLayer(11, "Leaf rib", [LEAF_RIB, stroke(MOSS_DEEP, 1.5)], frames, motion),
  ];
}

function cueLayers(startIndex: number, name: string, geometry: Json[], frames: number, motion: Motion = {}, width = 4): Json[] {
  const glow = [[13, 2.4], [9, 3.1], [6, 4.0], [3, 5.2]].map(([expansion, opacity], i) =>
    shapeLayer(startIndex + i, `${name} glow ${expansion}`, [...geometry, stroke(MOSS, width + expansion, opacity)], frames, motion),
  );
  return [
    ...glow,
    shapeLayer(startIndex + 4, `${name} outline`, [...geometry, stroke(CREAM, width + 1.5)], frames, motion),
    shapeLayer(startIndex + 5, `${name} color`, [...geometry, stroke(MOSS_DEEP, Math.max(width - 1, 1.5))], frames, motion),
  ];
}

const lottie = (name: string, frames: number, layers: Json[]): Json => ({
  v: "5.12.2", fr: FPS, ip: 0, op: frames, w: CANVAS, h: CANVAS, nm: name, ddd: 0, assets: [], layers: [...layers].reverse(), markers: [],
});

/** A cursor plus one action cue, the shape of every action but idle. */
function action(name: string, frames: number, geometry: Json[], motion: Motion, options: { width?: number; cursor?: Motion } = {}): Json {
  return lottie(`action_${name}`, frames, [
    ...cursorLayers(frames, options.cursor),
    ...cueLayers(20, name, geometry, frames, motion, options.width),
  ]);
}

function animations(): Record<string, Json> {
  const dragMove = animated([[0, [64, 64]], [8, [64, 64]], [30, [71, 67]], [40, [71, 67]], [48, [64, 64]]]);
  const recordAt = fixed([52, 64]);
  const record = lottie("action_record", 48, [
    ...cursorLayers(48),
    ...cueLayers(20, "Record ring", [ellipse(29, 39, 34, 34, "Record ring")], 48, { position: recordAt }),
    shapeLayer(30, "Record dot", [ellipse(29, 39, 10, 10, "Record dot"), fill(MOSS_DEEP), stroke(CREAM, 3)], 48, {
      position: recordAt,
      scale: animated([[0, [72, 72]], [24, [114, 114]], [48, [72, 72]]]),
      opacity: animated([[0, 42], [24, 100], [48, 42]]),
    }),
  ]);
  const teeth: Point[][] = [
    [[29, 20], [29, 25]], [[29, 53], [29, 58]], [[10, 39], [15, 39]], [[43, 39], [48, 39]],
    [[16, 26], [20, 30]], [[38, 48], [42, 52]], [[16, 52], [20, 48]], [[38, 30], [42, 26]],
  ];

  return {
    action_idle: lottie("action_idle", 1, cursorLayers(1)),
    action_observe: action("observe", 48, [
      pathShape([[[38, 28], [0, 0], [-11, 1]], [[20, 49], [0, -11], [0, 0]]], "Observe inner"),
      pathShape([[[42, 19], [0, 0], [-19, 0]], [[11, 51], [0, -18], [0, 0]]], "Observe outer"),
    ], {
      position: fixed([72, 54]),
      scale: animated([[0, [88, 88]], [24, [100, 100]], [48, [108, 108]]]),
      opacity: animated([[0, 0], [7, 100], [34, 100], [44, 0], [48, 0]]),
    }),
    action_click: action("click", 20, [
      points([[35, 20], [34, 11]], "Click upper"),
      points([[27, 25], [19, 19]], "Click middle"),
      points([[25, 34], [15, 34]], "Click lower"),
    ], {
      position: fixed([74, 67]),
      scale: animated([[0, [110, 110]], [8, [130, 130]], [20, [150, 150]]]),
      opacity: animated([[0, 0], [4, 100], [11, 100], [20, 0]]),
    }, { cursor: { scale: animated([[0, [100, 100]], [7, [93, 93]], [12, [103, 103]], [20, [100, 100]]]) } }),
    action_drag: action("drag", 48, [
      points([[28, 38], [16, 35]], "Drag upper"),
      points([[26, 48], [12, 45]], "Drag lower"),
    ], {
      position: dragMove,
      opacity: animated([[0, 20], [15, 100], [36, 100], [48, 20]]),
    }, { cursor: { position: dragMove } }),
    action_scroll: action("scroll", 48, [
      points([[23, 31], [31, 22], [39, 31]], "Scroll up"),
      points([[23, 49], [31, 58], [39, 49]], "Scroll down"),
    ], {
      position: animated([[0, [59, 68]], [24, [59, 60]], [48, [59, 68]]]),
      opacity: animated([[0, 42], [24, 100], [48, 42]]),
    }),
    action_text: action("text", 48, [
      points([[31, 22], [31, 58]], "Text stem"),
      points([[24, 22], [38, 22]], "Text top"),
      points([[24, 58], [38, 58]], "Text bottom"),
    ], {
      position: fixed([60, 64]),
      opacity: animated([[0, 100], [16, 100], [17, 18], [30, 18], [31, 100], [48, 100]], true),
    }),
    action_key: action("key", 48, [
      rectangle(28, 39, 28, 28, 6, "Key cap"),
      points([[23, 32], [23, 46]], "Key stem"),
      points([[24, 39], [33, 32]], "Key upper"),
      points([[25, 39], [34, 46]], "Key lower"),
    ], {
      position: animated([[0, [55, 64]], [8, [55, 67]], [17, [55, 63]], [26, [55, 64]], [48, [55, 64]]]),
      scale: animated([[0, [100, 100]], [8, [92, 92]], [17, [103, 103]], [26, [100, 100]], [48, [100, 100]]]),
      opacity: animated([[0, 0], [4, 100], [30, 100], [42, 0], [48, 0]]),
    }, { width: 3.5 }),
    action_navigate: action("navigate", 48, [
      points([[15, 29], [25, 40], [15, 51]], "Navigate first"),
      points([[29, 29], [39, 40], [29, 51]], "Navigate second"),
    ], {
      position: animated([[0, [54, 64]], [18, [59, 64]], [30, [63, 64]], [48, [63, 64]]]),
      opacity: animated([[0, 0], [8, 100], [28, 100], [42, 0], [48, 0]]),
    }),
    action_app: action("app", 48, (
      [[18, 31], [34, 31], [18, 47], [34, 47]] as Point[]
    ).map(([x, y]) => rectangle(x, y, 10, 10, 2, `App ${x} ${y}`)), {
      position: fixed([59, 64]),
      scale: animated([[0, [20, 20]], [14, [112, 112]], [24, [100, 100]], [48, [100, 100]]]),
      opacity: animated([[0, 0], [8, 100], [34, 100], [44, 0], [48, 0]]),
    }, { width: 3.5 }),
    action_transfer: action("transfer", 48, [
      points([[22, 50], [22, 20], [14, 28], [22, 20], [30, 28]], "Transfer up"),
      points([[37, 28], [37, 58], [29, 50], [37, 58], [45, 50]], "Transfer down"),
    ], {
      position: animated([[0, [55, 70]], [24, [55, 58]], [48, [55, 70]]]),
      opacity: animated([[0, 38], [24, 100], [48, 38]]),
    }),
    action_record: record,
    action_system: action("system", 48, [
      ellipse(29, 39, 24, 24, "System outer"),
      ellipse(29, 39, 8, 8, "System inner"),
      ...teeth.map((coordinates, i) => points(coordinates, `System tooth ${i}`)),
    ], {
      position: fixed([50, 64]),
      rotation: animated([[0, -18], [22, 50], [34, 0], [48, 0]]),
      opacity: animated([[0, 0], [6, 100], [34, 100], [44, 0], [48, 0]]),
    }, { width: 3.5 }),
  };
}

const ACTIONS = ["idle", "observe", "click", "drag", "scroll", "text", "key", "navigate", "app", "transfer", "record", "system"];

function themeManifest(): Json {
  return {
    schema: "cua.cursor-theme/2",
    id: THEME_ID,
    name: "Aloe",
    version: THEME_VERSION,
    author: "Trifles Labs",
    license: "MIT",
    compatibility: { profile: "cua-driver-actions-v2", semantics: 2 },
    canvas: { width: CANVAS, height: CANVAS, fps: FPS },
    hotspot: { x: 55, y: 30 },
    actions: Object.fromEntries(ACTIONS.map((name) => [name, { animation: `action_${name}`, still_frame: name === "idle" ? 0 : 8 }])),
  };
}

// ── A small deterministic ZIP writer (stored entries, fixed 1980-01-01 timestamps) ──

function zip(entries: [string, Uint8Array][]): Uint8Array {
  const local: Uint8Array[] = [];
  const central: Uint8Array[] = [];
  let offset = 0;
  for (const [name, data] of entries) {
    const nameBytes = new TextEncoder().encode(name);
    const crc = Bun.hash.crc32(data);
    const header = new DataView(new ArrayBuffer(30));
    header.setUint32(0, 0x04034b50, true);
    header.setUint16(4, 20, true);
    header.setUint16(10, 0, true); // time
    header.setUint16(12, 0x21, true); // 1980-01-01
    header.setUint32(14, crc, true);
    header.setUint32(18, data.length, true);
    header.setUint32(22, data.length, true);
    header.setUint16(26, nameBytes.length, true);
    local.push(new Uint8Array(header.buffer), nameBytes, data);

    const entry = new DataView(new ArrayBuffer(46));
    entry.setUint32(0, 0x02014b50, true);
    entry.setUint16(4, (3 << 8) | 20, true); // made by Unix, so the mode below is read
    entry.setUint16(6, 20, true);
    entry.setUint16(14, 0x21, true);
    entry.setUint32(16, crc, true);
    entry.setUint32(20, data.length, true);
    entry.setUint32(24, data.length, true);
    entry.setUint16(28, nameBytes.length, true);
    entry.setUint32(38, (0o100644 << 16) >>> 0, true);
    entry.setUint32(42, offset, true);
    central.push(new Uint8Array(entry.buffer), nameBytes);
    offset += 30 + nameBytes.length + data.length;
  }
  const centralSize = central.reduce((n, part) => n + part.length, 0);
  const end = new DataView(new ArrayBuffer(22));
  end.setUint32(0, 0x06054b50, true);
  end.setUint16(8, entries.length, true);
  end.setUint16(10, entries.length, true);
  end.setUint32(12, centralSize, true);
  end.setUint32(16, offset, true);
  return Buffer.concat([...local, ...central, new Uint8Array(end.buffer)]);
}

const encoded = (v: unknown) => new TextEncoder().encode(JSON.stringify(v) + "\n");

function main(): number {
  const all = animations();
  const manifest = { version: "2", generator: "Aloe cursor theme generator", animations: Object.keys(all).map((id) => ({ id, name: id })) };
  mkdirSync(OUT_DIR, { recursive: true });
  const source = join(OUT_DIR, "aloe.lottie");
  const compiled = join(OUT_DIR, "aloe.cua-theme");
  writeFileSync(source, zip([
    ["manifest.json", encoded(manifest)],
    ["cua/theme.json", encoded(themeManifest())],
    ...Object.entries(all).map(([id, animation]): [string, Uint8Array] => [`a/${id}.json`, encoded(animation)]),
  ]));
  console.log(source);

  if (!existsSync(DRIVER)) {
    console.error(`Cua Driver is not at ${DRIVER}; run scripts/fetch_cua_driver.py, then this again.`);
    return 1;
  }
  for (const args of [["validate", source], ["build", source, "--output", compiled], ["inspect", compiled]]) {
    const run = spawnSync(DRIVER, ["cursor-theme", ...args], { stdio: "inherit" });
    if (run.status !== 0) return run.status ?? 1;
  }
  return 0;
}

process.exit(main());

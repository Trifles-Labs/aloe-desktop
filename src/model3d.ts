import {
  AmbientLight,
  BoxGeometry,
  BufferGeometry,
  ConeGeometry,
  CylinderGeometry,
  DirectionalLight,
  EdgesGeometry,
  LineBasicMaterial,
  LineSegments,
  Material,
  Mesh,
  MeshBasicMaterial,
  MeshStandardMaterial,
  Object3D,
  PerspectiveCamera,
  Raycaster,
  Scene,
  SphereGeometry,
  Vector2,
  WebGLRenderer,
} from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { CSS2DObject, CSS2DRenderer } from "three/examples/jsm/renderers/CSS2DRenderer.js";
import { COLORS, px } from "./voice-drawing";

/* A small 3D solid Aloe puts over the screen (a draw_on_screen "model3d" mark): turned by dragging,
   zoomed with the wheel, its labelled parts clickable for a short note. One at a time, on a canvas
   in the mark's box on the voice overlay, under the marks and notes and over the glow. The window
   takes the cursor only over this box (voice/overlay.rs), and the page reports it like the clear
   chip. Nothing runs while no model is up. */

type Vec3 = [number, number, number];
export type Model3DMark = {
  type: "model3d";
  x: number;
  y: number;
  w: number;
  h: number;
  builder: "solid";
  params: {
    shape: "cube" | "cuboid" | "sphere" | "cylinder" | "cone";
    size?: Vec3;
    color?: string;
    opacity?: number;
    edges?: boolean;
    labels?: { text: string; at: Vec3; note?: string }[];
  };
  camera?: { position?: Vec3; target?: Vec3 };
};
type Rect = { x: number; y: number; width: number; height: number };

/** How far (px) the pointer may move between press and release and still be a click, not a turn. */
const CLICK_SLOP = 4;
const NOTE_MS = 6000;
const DEFAULT_CAMERA: Vec3 = [3, 2.2, 4];

type Shown = {
  key: string;
  container: HTMLDivElement;
  renderer: WebGLRenderer;
  labels: CSS2DRenderer;
  scene: Scene;
  camera: PerspectiveCamera;
  controls: OrbitControls;
  markers: Object3D[];
  pill: HTMLDivElement;
  frame: number;
  pillTimer: number;
  mark: Model3DMark;
};

let shown: Shown | null = null;
let warned = false;
const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

/* The mark's box in CSS pixels, from the 0–1000 grid. */
const boxOf = (mark: Model3DMark): Rect => {
  const corner = px({ x: mark.x, y: mark.y });
  const far = px({ x: mark.x + mark.w, y: mark.y + mark.h });
  return { x: corner.x, y: corner.y, width: Math.max(1, far.x - corner.x), height: Math.max(1, far.y - corner.y) };
};

const geometryFor = (shape: Model3DMark["params"]["shape"]): BufferGeometry => {
  switch (shape) {
    case "sphere":
      return new SphereGeometry(0.5, 48, 32);
    case "cylinder":
      return new CylinderGeometry(0.5, 0.5, 1, 48);
    case "cone":
      return new ConeGeometry(0.5, 1, 48);
    default:
      return new BoxGeometry(1, 1, 1);
  }
};

/* Positions the container, canvas and label layer on the mark's box, at the current screen size. */
const layout = (view: Shown) => {
  const box = boxOf(view.mark);
  Object.assign(view.container.style, { left: `${box.x}px`, top: `${box.y}px`, width: `${box.width}px`, height: `${box.height}px` });
  view.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.5));
  view.renderer.setSize(box.width, box.height, false);
  view.renderer.domElement.style.width = `${box.width}px`;
  view.renderer.domElement.style.height = `${box.height}px`;
  view.labels.setSize(box.width, box.height);
  view.camera.aspect = box.width / box.height;
  view.camera.updateProjectionMatrix();
};

const showNote = (view: Shown, at: { x: number; y: number }, label: string, note: string | undefined) => {
  window.clearTimeout(view.pillTimer);
  const title = document.createElement("strong");
  title.textContent = label;
  view.pill.replaceChildren(title);
  if (note) {
    const body = document.createElement("span");
    body.textContent = note;
    view.pill.append(body);
  }
  view.pill.style.left = `${at.x}px`;
  view.pill.style.top = `${at.y}px`;
  view.pill.dataset.open = "";
  view.pillTimer = window.setTimeout(() => delete view.pill.dataset.open, NOTE_MS);
};

const build = (mark: Model3DMark, key: string): Shown | null => {
  const root = document.getElementById("root")!;
  const container = document.createElement("div");
  container.className = "model3d";
  const canvas = document.createElement("canvas");
  container.append(canvas);
  // DOM order sets the stacking: after the glow, before the marks and notes.
  root.insertBefore(container, document.getElementById("marks"));

  let renderer: WebGLRenderer;
  try {
    renderer = new WebGLRenderer({ canvas, alpha: true, antialias: true, premultipliedAlpha: false });
  } catch (error) {
    container.remove();
    if (!warned) {
      warned = true;
      console.warn("3D model not shown: WebGL is unavailable.", error);
    }
    return null;
  }
  renderer.setClearColor(0x000000, 0);

  const params = mark.params;
  const size: Vec3 = params.shape === "cube" ? [params.size?.[0] ?? 1, params.size?.[0] ?? 1, params.size?.[0] ?? 1] : (params.size ?? [1, 1, 1]);
  const extent = Math.max(...size, 1);
  const color = COLORS[params.color ?? "blue"] ?? COLORS.blue;
  const opacity = params.opacity ?? 1;

  const scene = new Scene();
  scene.add(new AmbientLight(0xffffff, 0.9));
  const sun = new DirectionalLight(0xffffff, 2);
  sun.position.set(3, 5, 4);
  scene.add(sun);

  // Its labels are in the mark's ink (the accent, as the notes have it).
  container.style.setProperty("--accent", color);
  const material = new MeshStandardMaterial({ color, transparent: true, opacity, roughness: 0.55, metalness: 0.05, depthWrite: opacity >= 0.99 });
  const solid = new Mesh(geometryFor(params.shape), material);
  solid.scale.set(...size);
  scene.add(solid);
  if (params.edges) {
    const ink = params.color === "black" ? 0xffffff : 0x1f2430;
    solid.add(new LineSegments(new EdgesGeometry(solid.geometry), new LineBasicMaterial({ color: ink, transparent: true, opacity: 0.85 })));
  }

  // Each label is a word at its point, with a small dot there that a click lands on.
  const labels = new CSS2DRenderer();
  labels.domElement.className = "model3d-labels";
  container.append(labels.domElement);
  const markers: Object3D[] = [];
  const dot = new SphereGeometry(0.07 * extent, 16, 12);
  const dotMaterial = new MeshBasicMaterial({ color: 0xffffff });
  for (const label of params.labels ?? []) {
    const marker = new Mesh(dot, dotMaterial);
    marker.position.set(...label.at);
    marker.userData = { label: label.text, note: label.note };
    scene.add(marker);
    markers.push(marker);
    const tag = document.createElement("div");
    tag.className = "model3d-label";
    tag.textContent = label.text;
    const object = new CSS2DObject(tag);
    object.position.set(...label.at);
    object.center.set(0.5, 1.4);
    scene.add(object);
  }

  const camera = new PerspectiveCamera(40, 1, 0.05, 400);
  const position = mark.camera?.position ?? (DEFAULT_CAMERA.map((n) => n * extent) as Vec3);
  const target = mark.camera?.target ?? ([0, 0, 0] as Vec3);
  camera.position.set(...position);
  camera.lookAt(...target);

  const controls = new OrbitControls(camera, canvas);
  controls.target.set(...target);
  controls.enablePan = false;
  controls.enableZoom = true;
  controls.enableDamping = true;
  controls.autoRotate = !reducedMotion.matches;
  controls.autoRotateSpeed = 1.2;
  controls.addEventListener("start", () => {
    controls.autoRotate = false;
  });
  controls.update();

  const pill = document.createElement("div");
  pill.className = "model3d-note";
  container.append(pill);

  const view: Shown = { key, container, renderer, labels, scene, camera, controls, markers, pill, frame: 0, pillTimer: 0, mark };

  // A click (not a drag) on a labelled point shows its note.
  let down: { x: number; y: number } | null = null;
  canvas.addEventListener("pointerdown", (event) => {
    down = { x: event.clientX, y: event.clientY };
  });
  canvas.addEventListener("pointerup", (event) => {
    const start = down;
    down = null;
    if (!start || Math.hypot(event.clientX - start.x, event.clientY - start.y) >= CLICK_SLOP || markers.length === 0) return;
    const bounds = canvas.getBoundingClientRect();
    const pointer = new Vector2(((event.clientX - bounds.left) / bounds.width) * 2 - 1, -((event.clientY - bounds.top) / bounds.height) * 2 + 1);
    const ray = new Raycaster();
    ray.setFromCamera(pointer, camera);
    const hit = ray.intersectObjects(markers, false)[0];
    const data = hit?.object.userData as { label?: string; note?: string } | undefined;
    if (data?.label) showNote(view, { x: event.clientX - bounds.left, y: event.clientY - bounds.top }, data.label, data.note);
  });
  canvas.addEventListener("webglcontextlost", () => clearModels());

  layout(view);
  // Only now can it be turned: until the renderer is up it must not take the pointer.
  canvas.style.pointerEvents = "auto";
  const tick = () => {
    view.frame = requestAnimationFrame(tick);
    controls.update();
    renderer.render(scene, camera);
    labels.render(scene, camera);
  };
  view.frame = requestAnimationFrame(tick);
  return view;
};

/** Shows this model, replacing any other. The same model again (a resize, more marks) only moves. */
export function showModel(mark: Model3DMark): void {
  const key = JSON.stringify(mark);
  if (shown?.key === key) {
    layout(shown);
    return;
  }
  clearModels();
  shown = build(mark, key);
}

/** Takes the model away and frees everything it held. Cheap when there is none. */
export function clearModels(): void {
  const view = shown;
  if (!view) return;
  shown = null;
  cancelAnimationFrame(view.frame);
  window.clearTimeout(view.pillTimer);
  view.controls.dispose();
  const geometries = new Set<BufferGeometry>();
  const materials = new Set<Material>();
  view.scene.traverse((object) => {
    const item = object as Object3D & { geometry?: BufferGeometry; material?: Material | Material[] };
    if (item.geometry) geometries.add(item.geometry);
    const material = item.material;
    if (Array.isArray(material)) material.forEach((m) => materials.add(m));
    else if (material) materials.add(material);
  });
  geometries.forEach((geometry) => geometry.dispose());
  materials.forEach((material) => material.dispose());
  view.renderer.dispose();
  view.renderer.forceContextLoss();
  view.container.remove();
}

/** Where the model is, in CSS pixels, or null when none is up. */
export function modelRect(): Rect | null {
  if (!shown) return null;
  const box = shown.container.getBoundingClientRect();
  return { x: box.x, y: box.y, width: box.width, height: box.height };
}

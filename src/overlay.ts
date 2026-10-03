import "@fontsource-variable/instrument-sans";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

/* The desktop-control overlay (overlay.html). No React: two tiny always-on-top windows that only
   toggle one attribute. The Rust side (overlay.rs) owns when they show; this just mirrors it. */

const OVERLAY_EVENT = "desktop-control://active";
const root = document.getElementById("root")!;

document.documentElement.dataset.part = getCurrentWindow().label === "control-pill" ? "pill" : "glow";

const setActive = (active: boolean) => {
  root.dataset.active = String(active);
};

/* overlay.rs calls this directly (WebviewWindow::eval). The event below is a second path; the
   direct call is the one that can't be blocked by capability scoping. */
(window as Window & { __aloeOverlaySetActive?: (active: boolean) => void }).__aloeOverlaySetActive = setActive;
void listen<boolean>(OVERLAY_EVENT, (event) => setActive(event.payload)).catch(() => undefined);
// The window is created by the very action that turns it on, so that first event can land before
// this listener exists — ask once for the current state.
void invoke<boolean>("desktop_control_overlay_active").then(setActive, () => undefined);

/* The green halo under the pointer (glow window, not Windows). The first position places it without
   sliding in from the corner. */
const halo = document.getElementById("halo")!;
(window as Window & { __aloeOverlayPointer?: (x: number, y: number) => void }).__aloeOverlayPointer = (x, y) => {
  const first = halo.dataset.placed !== "true";
  if (first) halo.style.transition = "none";
  halo.style.transform = `translate(${x}px, ${y}px)`;
  if (first) {
    void halo.offsetWidth;
    halo.style.transition = "";
    halo.dataset.placed = "true";
  }
};

document.getElementById("stop")?.addEventListener("click", () => {
  setActive(false);
  void invoke("stop_desktop_control");
});

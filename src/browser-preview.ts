import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

import { DEFAULT_CONFIG } from "./types";

/* Vite without the Tauri shell. The real window already has these internals,
   so this never runs there. Enough for the sign-in screen and the title bar
   to paint; commands that need the computer stay no-ops. */
if (import.meta.env.DEV && !window.__TAURI_INTERNALS__) {
  document.documentElement.classList.add("dark");
  try {
    localStorage.setItem("theme", "dark");
  } catch {
    /* Storage can be blocked. The class above is enough for this paint. */
  }

  mockWindows("main");
  mockIPC(
    (cmd, args) => {
      if (cmd === "get_config") {
        return { ...DEFAULT_CONFIG, apiUrl: process.env.NEXT_PUBLIC_API_URL ?? "http://127.0.0.1:8080" };
      }
      if (cmd === "get_pending_approvals") return [];
      if (cmd === "plugin:window|is_maximized") return false;
      if (cmd === "plugin:window|is_focused") return true;
      if (cmd === "open_external_url" && args && typeof args === "object" && "url" in args && typeof args.url === "string") {
        window.location.assign(args.url);
        return null;
      }
      return null;
    },
    { shouldMockEvents: true },
  );
}

import React, { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ArrowLeft, ArrowRight, Leaf, Minus, SquarePen, X } from "lucide-react";

import { goBack, goForward, routePosition, subscribeToRoute } from "../shims/next-navigation";

const appWindow = getCurrentWindow();

/* The window has no OS chrome, so this bar *is* the chrome: it answers where
   you are, gives you the way back, and carries the window controls. Everything
   in it that isn't a control is a drag region. */

const ROUTE_TITLES: Array<[test: (path: string) => boolean, title: string]> = [
  [(p) => p === "/app/home", "New chat"],
  [(p) => p.startsWith("/app/chat"), "Chat"],
  [(p) => p === "/app/conversations", "Conversations"],
  [(p) => p === "/app/board", "Aloe Board"],
  [(p) => p === "/app/tasks", "Scheduled tasks"],
  [(p) => p === "/app/memory", "Memory Garden"],
  [(p) => p === "/app/approvals", "Approvals"],
  [(p) => p === "/app/settings", "Settings"],
  [(p) => p === "/app/plans", "Plans"],
  [(p) => p === "/app/usage", "Usage"],
  [(p) => p === "/app/onboarding", "Welcome"],
  [(p) => p === "/app/desktop", "Desktop controls"],
];

const routeTitle = (pathname: string) => ROUTE_TITLES.find(([test]) => test(pathname))?.[1] ?? "Aloe";

/* Drawn rather than imported: the maximise and restore glyphs are 10px boxes on
   a 1px grid, and a rounded icon-set square blurs at that size. */
function MaximizeGlyph({ maximized }: { maximized: boolean }) {
  return maximized ? (
    <svg viewBox="0 0 12 12" className="h-[11px] w-[11px]" fill="none" stroke="currentColor" strokeWidth="1" aria-hidden>
      <path d="M3.5 3.5V2.5h6v6h-1" />
      <rect x="1.5" y="4.5" width="6" height="6" />
    </svg>
  ) : (
    <svg viewBox="0 0 12 12" className="h-[11px] w-[11px]" fill="none" stroke="currentColor" strokeWidth="1" aria-hidden>
      <rect x="1.5" y="1.5" width="9" height="9" />
    </svg>
  );
}

function HistoryButton({ label, disabled, onClick, children }: { label: string; disabled: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
      className="press-tap inline-flex h-7 w-7 items-center justify-center rounded-lg text-ink-soft hover:bg-sage-soft hover:text-ink disabled:pointer-events-none disabled:opacity-30"
    >
      {children}
    </button>
  );
}

function WindowButton({ label, onClick, danger, children }: { label: string; onClick: () => void; danger?: boolean; children: React.ReactNode }) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      className={`flex h-10 w-[46px] items-center justify-center text-ink-soft transition-colors duration-100 hover:text-ink active:brightness-95 ${
        danger ? "hover:bg-danger hover:text-on-accent" : "hover:bg-sage-soft"
      }`}
    >
      {children}
    </button>
  );
}

export function DesktopTitleBar() {
  const [maximized, setMaximized] = useState(false);
  const [focused, setFocused] = useState(true);
  // Two scalar reads rather than one object: a getSnapshot that allocates a
  // fresh object every call never compares equal, and React re-renders forever.
  const historyIndex = useSyncExternalStore(subscribeToRoute, () => routePosition().index, () => 0);
  const historyFurthest = useSyncExternalStore(subscribeToRoute, () => routePosition().furthest, () => 0);
  const pathname = useSyncExternalStore(subscribeToRoute, () => window.location.pathname, () => "/app/chat");

  useEffect(() => {
    const refresh = () => void appWindow.isMaximized().then(setMaximized);
    refresh();
    const unlistenResize = appWindow.onResized(refresh);
    const unlistenFocus = appWindow.onFocusChanged(({ payload }) => setFocused(payload));
    return () => {
      void unlistenResize.then((dispose) => dispose());
      void unlistenFocus.then((dispose) => dispose());
    };
  }, []);

  const canGoBack = historyIndex > 0;
  const canGoForward = historyIndex < historyFurthest;

  /* Alt+←/→ and the mouse's thumb buttons are what people already press to go
     back on this platform; without them the window swallows the gesture. */
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!event.altKey || event.ctrlKey || event.metaKey) return;
      if (event.key === "ArrowLeft") { event.preventDefault(); goBack(); }
      if (event.key === "ArrowRight") { event.preventDefault(); goForward(); }
    };
    const onMouseUp = (event: MouseEvent) => {
      if (event.button === 3) { event.preventDefault(); goBack(); }
      if (event.button === 4) { event.preventDefault(); goForward(); }
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("mouseup", onMouseUp);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("mouseup", onMouseUp);
    };
  }, []);

  const openNewChat = () => window.dispatchEvent(new Event("aloe:desktop-new-chat"));
  const toggleMaximize = useCallback(async () => {
    await appWindow.toggleMaximize();
    setMaximized(await appWindow.isMaximized());
  }, []);

  const closeToTray = () => {
    // The window hides rather than quitting, which is worth saying once.
    window.dispatchEvent(new Event("aloe:desktop-tray-hint"));
    void invoke("hide_main_window");
  };

  return (
    <header
      data-tauri-drag-region
      className="liquid-glass-bar relative z-[100] flex h-10 shrink-0 select-none items-center gap-0.5 pl-2 text-ink"
    >
      {/* Identity, not a control — the leaf stays part of the drag region. */}
      <span data-tauri-drag-region className="brand-mark mr-1 h-6 w-6 shrink-0">
        <Leaf className="h-3.5 w-3.5" />
      </span>

      <HistoryButton label="New chat" disabled={false} onClick={openNewChat}>
        <SquarePen className="h-4 w-4" />
      </HistoryButton>

      <span aria-hidden className="mx-1 h-4 w-px bg-edge" />

      <HistoryButton label="Back (Alt+←)" disabled={!canGoBack} onClick={goBack}>
        <ArrowLeft className="h-4 w-4" />
      </HistoryButton>
      <HistoryButton label="Forward (Alt+→)" disabled={!canGoForward} onClick={goForward}>
        <ArrowRight className="h-4 w-4" />
      </HistoryButton>

      <div data-tauri-drag-region className="h-full flex-1" onDoubleClick={() => void toggleMaximize()} />

      {/* Centred over the drag region and click-through, so the title never
          costs you somewhere to grab the window. Text on a blurred bar takes a
          little extra weight, or the material eats the thin strokes. */}
      <div
        aria-hidden
        className={`pointer-events-none absolute inset-x-0 flex justify-center transition-opacity duration-200 ${focused ? "opacity-100" : "opacity-45"}`}
      >
        <span className="max-w-[40%] truncate text-[12px] font-medium tracking-[0.01em] text-ink-soft">{routeTitle(pathname)}</span>
      </div>

      <div className={`flex h-full items-center transition-opacity duration-200 ${focused ? "opacity-100" : "opacity-55"}`}>
        <WindowButton label="Minimize" onClick={() => void appWindow.minimize()}>
          <Minus className="h-4 w-4" />
        </WindowButton>
        <WindowButton label={maximized ? "Restore" : "Maximize"} onClick={() => void toggleMaximize()}>
          <MaximizeGlyph maximized={maximized} />
        </WindowButton>
        <WindowButton label="Close to tray — Aloe keeps running" danger onClick={closeToTray}>
          <X className="h-4 w-4" />
        </WindowButton>
      </div>
    </header>
  );
}

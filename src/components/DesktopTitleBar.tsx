import React, { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ArrowLeft, ArrowRight, Code2, Leaf, MessagesSquare, Minus, SquarePen, X } from "lucide-react";

import { readLastCodeRoute } from "../code/sessions";
import { goBack, goForward, navigateTo, routePosition, subscribeToRoute } from "../shims/next-navigation";

const appWindow = getCurrentWindow();

/* The window has no OS chrome, so this bar *is* the chrome: it answers where
   you are, gives you the way back, and carries the window controls. Everything
   in it that isn't a control is a drag region. */

const ROUTE_TITLES: Array<[test: (path: string) => boolean, title: string]> = [
  [(p) => p === "/app/code", "New session"],
  [(p) => p.startsWith("/app/code/"), "Aloe Code"],
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

const isCodeRoute = (pathname: string) => pathname === "/app/code" || pathname.startsWith("/app/code/");

/* Where Chat mode was last left. Session-only: a fresh launch opens chat at home. */
let lastChatRoute = "/app/home";

/** Chat | Code, the two ways into Aloe on this computer. Each side reopens where it was left. */
function ModeSwitch({ code }: { code: boolean }) {
  const option = (active: boolean, label: string, onClick: () => void, icon: React.ReactNode) => (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      title={label}
      onClick={onClick}
      className={`press-tap inline-flex h-6 items-center gap-1.5 rounded-md px-2 text-[12px] font-medium transition-colors ${
        active ? "bg-surface-strong text-ink shadow-[0_1px_2px_rgba(22,33,26,0.12)]" : "text-ink-soft hover:text-ink"
      }`}
    >
      {icon}
      <span>{label}</span>
    </button>
  );

  return (
    <div role="radiogroup" aria-label="Mode" className="inline-flex items-center gap-0.5 rounded-lg bg-sage-soft/70 p-0.5">
      {option(!code, "Chat", () => code && navigateTo(lastChatRoute), <MessagesSquare className="h-3.5 w-3.5" />)}
      {option(code, "Code", () => !code && navigateTo(readLastCodeRoute()), <Code2 className="h-3.5 w-3.5" />)}
    </div>
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

  const code = isCodeRoute(pathname);
  if (!code && pathname.startsWith("/app/")) lastChatRoute = `${pathname}${window.location.search}`;

  const openNewChat = () => window.dispatchEvent(new Event(code ? "aloe:new-code-session" : "aloe:new-chat"));
  const toggleMaximize = useCallback(async () => {
    await appWindow.toggleMaximize();
    setMaximized(await appWindow.isMaximized());
  }, []);

  // Hides to the tray rather than quitting — the label on the button is what
  // says so, since the window is gone before any in-app notice could be read.
  const closeToTray = () => void invoke("hide_main_window");

  return (
    <header
      data-tauri-drag-region
      className="liquid-glass-bar relative z-[100] flex h-10 shrink-0 select-none items-center gap-0.5 pl-2 text-ink"
    >
      {/* Identity, not a control — the leaf stays part of the drag region. */}
      <span data-tauri-drag-region className="brand-mark mr-1 h-6 w-6 shrink-0">
        <Leaf className="h-3.5 w-3.5" />
      </span>

      <HistoryButton label={code ? "New session" : "New chat"} disabled={false} onClick={openNewChat}>
        <SquarePen className="h-4 w-4" />
      </HistoryButton>

      <span aria-hidden className="mx-1 h-4 w-px bg-edge" />

      <HistoryButton label="Back (Alt+←)" disabled={!canGoBack} onClick={goBack}>
        <ArrowLeft className="h-4 w-4" />
      </HistoryButton>
      <HistoryButton label="Forward (Alt+→)" disabled={!canGoForward} onClick={goForward}>
        <ArrowRight className="h-4 w-4" />
      </HistoryButton>

      <span aria-hidden className="mx-1 h-4 w-px bg-edge" />

      <div className="relative z-10">
        <ModeSwitch code={code} />
      </div>

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

import React, { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Leaf, Minus, Square, X } from "lucide-react";

const appWindow = getCurrentWindow();

export function DesktopTitleBar() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const refresh = () => void appWindow.isMaximized().then(setMaximized);
    refresh();
    const unlisten = appWindow.onResized(refresh);
    return () => { void unlisten.then((dispose) => dispose()); };
  }, []);

  const openNewChat = () => window.dispatchEvent(new Event("aloe:desktop-new-chat"));
  const toggleMaximize = async () => {
    await appWindow.toggleMaximize();
    setMaximized(await appWindow.isMaximized());
  };

  return (
    <header data-tauri-drag-region className="relative z-[100] flex h-10 shrink-0 select-none items-center border-b border-edge bg-surface-strong/92 pl-3 text-ink backdrop-blur-xl">
      <button type="button" onClick={openNewChat} className="flex h-full items-center gap-2 px-1.5 text-sm font-semibold" title="New chat">
        <span className="brand-mark h-6 w-6"><Leaf className="h-3.5 w-3.5" /></span>
        <span>Aloe</span>
      </button>
      <div data-tauri-drag-region className="h-full flex-1" onDoubleClick={() => void toggleMaximize()} />
      <button type="button" onClick={() => void appWindow.minimize()} className="flex h-10 w-12 items-center justify-center text-ink-soft transition-colors hover:bg-sage-soft" aria-label="Minimize"><Minus className="h-4 w-4" /></button>
      <button type="button" onClick={() => void toggleMaximize()} className="flex h-10 w-12 items-center justify-center text-ink-soft transition-colors hover:bg-sage-soft" aria-label={maximized ? "Restore" : "Maximize"}><Square className={maximized ? "h-3.5 w-3.5" : "h-3.5 w-3.5"} /></button>
      <button type="button" onClick={() => void invoke("hide_main_window")} className="flex h-10 w-12 items-center justify-center text-ink-soft transition-colors hover:bg-danger hover:text-on-accent" aria-label="Close to tray"><X className="h-4 w-4" /></button>
    </header>
  );
}

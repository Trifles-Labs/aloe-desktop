/* The right-hand pane of Aloe Code: Changes, Files and Terminal as tabs.
 *
 * Resized by dragging its left edge. A tab stays mounted once opened, so switching away from an
 * open file or a running command's output doesn't lose its place. */

import { useEffect, useRef, useState, type ReactNode } from "react";
import { FileDiff, FolderTree, SquareTerminal, X } from "lucide-react";

import { cn } from "@aloe/ui/lib/utils";
import type { WorkbenchPrefs, WorkbenchTab } from "./sessions";

const MIN_WIDTH = 320;
const MAX_WIDTH = 960;

const TABS: Array<{ tab: WorkbenchTab; label: string; icon: typeof FileDiff }> = [
  { tab: "changes", label: "Changes", icon: FileDiff },
  { tab: "files", label: "Files", icon: FolderTree },
  { tab: "terminal", label: "Terminal", icon: SquareTerminal },
];

type Props = {
  prefs: WorkbenchPrefs;
  onChange: (prefs: WorkbenchPrefs) => void;
  panes: Record<WorkbenchTab, ReactNode>;
};

function clampWidth(width: number) {
  // Never so wide that the chat beside it is squeezed under ~420px.
  const room = typeof window === "undefined" ? MAX_WIDTH : window.innerWidth - 264 - 420;
  return Math.round(Math.max(MIN_WIDTH, Math.min(width, MAX_WIDTH, Math.max(MIN_WIDTH, room))));
}

export default function Workbench({ prefs, onChange, panes }: Props) {
  const [visited, setVisited] = useState<Set<WorkbenchTab>>(() => new Set([prefs.tab]));
  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const drag = useRef<{ startX: number; startWidth: number } | null>(null);

  useEffect(() => {
    setVisited((current) => (current.has(prefs.tab) ? current : new Set(current).add(prefs.tab)));
  }, [prefs.tab]);

  const width = clampWidth(dragWidth ?? prefs.width);

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { startX: event.clientX, startWidth: width };
    setDragWidth(width);
  };
  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    setDragWidth(clampWidth(drag.current.startWidth + drag.current.startX - event.clientX));
  };
  const endDrag = () => {
    if (!drag.current) return;
    drag.current = null;
    if (dragWidth !== null) onChange({ ...prefs, width: dragWidth });
    setDragWidth(null);
  };

  return (
    <aside aria-label="Session workbench" className="relative hidden shrink-0 flex-col border-l border-edge bg-canvas lg:flex" style={{ width }}>
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize side pane"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onDoubleClick={() => onChange({ ...prefs, width: 440 })}
        className={cn("absolute inset-y-0 -left-1 z-10 w-2 cursor-col-resize transition-colors hover:bg-moss/20", dragWidth !== null && "bg-moss/25")}
      />
      <div role="tablist" aria-label="Side pane" className="flex items-center gap-0.5 border-b border-edge px-2 py-1.5">
        {TABS.map(({ tab, label, icon: Icon }) => (
          <button
            key={tab}
            type="button"
            role="tab"
            aria-selected={prefs.tab === tab}
            onClick={() => onChange({ ...prefs, tab })}
            className={cn(
              "press-tap inline-flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors",
              prefs.tab === tab ? "bg-sage-soft text-ink" : "text-ink-soft hover:bg-sage-soft/60 hover:text-ink",
            )}
          >
            <Icon className="h-3.5 w-3.5" />
            {label}
          </button>
        ))}
        <span className="flex-1" />
        <button
          type="button"
          onClick={() => onChange({ ...prefs, open: false })}
          aria-label="Close side pane"
          title="Close"
          className="flex h-7 w-7 items-center justify-center rounded-md text-ink-soft hover:bg-sage-soft hover:text-ink"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
      {TABS.map(({ tab }) =>
        visited.has(tab) ? (
          <div key={tab} role="tabpanel" hidden={prefs.tab !== tab} className={cn("min-h-0 flex-1 flex-col", prefs.tab === tab ? "flex" : "hidden")}>
            {panes[tab]}
          </div>
        ) : null,
      )}
    </aside>
  );
}

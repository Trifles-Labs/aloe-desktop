/* The one bar above an Aloe Code session: where it works (project, branch), how commands are
 * approved, and the switches for the right-hand pane. */

import { FileDiff, FolderTree, Folder, GitBranch, ShieldAlert, SquareTerminal } from "lucide-react";
import Link from "next/link";

import { cn } from "@aloe/ui/lib/utils";
import { projectName, type WorkbenchTab } from "./sessions";

type Props = {
  project: string;
  branch: string | null;
  pendingCount: number;
  trustLabel: string;
  /** The open tab, or null when the pane is closed. */
  openTab: WorkbenchTab | null;
  changeCount: number;
  runningCount: number;
  onToggleTab: (tab: WorkbenchTab) => void;
};

const TABS: Array<{ tab: WorkbenchTab; label: string; icon: typeof FileDiff }> = [
  { tab: "changes", label: "Changes", icon: FileDiff },
  { tab: "files", label: "Files", icon: FolderTree },
  { tab: "terminal", label: "Terminal", icon: SquareTerminal },
];

export default function CodeHeader({ project, branch, pendingCount, trustLabel, openTab, changeCount, runningCount, onToggleTab }: Props) {
  const badge = (tab: WorkbenchTab) => (tab === "changes" ? changeCount : tab === "terminal" ? runningCount : 0);

  return (
    <div className="flex items-center gap-3 border-b border-edge px-4 py-1.5 text-xs text-ink-soft sm:px-6">
      <span className="inline-flex min-w-0 items-center gap-1.5 font-medium text-ink" title={project}>
        <Folder className="h-3.5 w-3.5 shrink-0 text-moss" />
        <span className="truncate">{projectName(project)}</span>
      </span>
      {branch ? (
        <span className="inline-flex min-w-0 items-center gap-1 font-mono text-[11px]" title={`Branch ${branch}`}>
          <GitBranch className="h-3 w-3 shrink-0" />
          <span className="truncate">{branch}</span>
        </span>
      ) : null}
      {pendingCount > 0 ? (
        <Link href="/app/settings?section=desktop" className="inline-flex shrink-0 items-center gap-1.5 rounded-md bg-clay/10 px-2 py-1 font-semibold text-clay hover:bg-clay/15">
          <ShieldAlert className="h-3.5 w-3.5" />
          {pendingCount} command{pendingCount === 1 ? "" : "s"} waiting
        </Link>
      ) : (
        <Link href="/app/settings?section=desktop" className="hidden shrink-0 hover:text-ink xl:inline">
          {trustLabel}
        </Link>
      )}
      <span className="flex-1" />
      <div role="toolbar" aria-label="Side pane" className="hidden items-center gap-0.5 lg:flex">
        {TABS.map(({ tab, label, icon: Icon }) => {
          const active = openTab === tab;
          const count = badge(tab);
          return (
            <button
              key={tab}
              type="button"
              aria-pressed={active}
              onClick={() => onToggleTab(tab)}
              className={cn(
                "press-tap inline-flex items-center gap-1.5 rounded-md px-2 py-1 font-medium transition-colors",
                active ? "bg-sage-soft text-ink" : "hover:bg-sage-soft/60 hover:text-ink",
              )}
            >
              <Icon className="h-3.5 w-3.5" />
              {label}
              {count > 0 ? (
                <span className={cn("rounded-full px-1.5 font-mono text-[10px] leading-4", tab === "terminal" ? "bg-moss/15 text-moss" : "bg-ink/10 text-ink")}>{count}</span>
              ) : null}
            </button>
          );
        })}
      </div>
    </div>
  );
}

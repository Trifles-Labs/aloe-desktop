/* Aloe Code's left column: one project at a time.
 *
 * The switcher at the top picks the project; below it are that project's sessions only. Sessions
 * this computer has no project for (started elsewhere, or before a reinstall) sit in their own
 * "Needs a project" list, each with a picker, instead of being mixed in with the rest. */

import { AnimatePresence, motion } from "framer-motion";
import { FolderPlus, Loader2, MessageSquareDashed, Plus, Trash2 } from "lucide-react";
import Link from "next/link";

import Select from "@aloe/ui/components/ui/Select";
import { EASE_OUT } from "@aloe/ui/lib/motion";
import { cn, relativeTime } from "@aloe/ui/lib/utils";

export type SidebarSession = { id: string; title: string; createdAt: Date };

type Props = {
  projectOptions: Array<{ value: string; label: string; description?: string }>;
  project: string | null;
  onChooseProject: (path: string) => void;
  onAddProject: () => void;
  addingProject: boolean;
  sessions: SidebarSession[];
  unassigned: SidebarSession[];
  activeSessionId: string | null;
  onNewSession: () => void;
  onDeleteSession: (id: string) => void;
  onAssignSession: (id: string, projectPath: string) => void;
};

export default function ProjectSidebar({
  projectOptions,
  project,
  onChooseProject,
  onAddProject,
  addingProject,
  sessions,
  unassigned,
  activeSessionId,
  onNewSession,
  onDeleteSession,
  onAssignSession,
}: Props) {
  const hasProjects = projectOptions.length > 0;

  return (
    <aside className="liquid-glass-side hidden w-[264px] shrink-0 flex-col px-3 py-4 md:flex">
      <p className="px-1 text-[11px] font-semibold uppercase tracking-wider text-ink-soft/70">Project</p>
      <div className="mt-1.5 flex items-center gap-1.5">
        {hasProjects && project ? (
          <Select className="min-w-0 flex-1" value={project} options={projectOptions} onChange={onChooseProject} />
        ) : (
          <p className="min-w-0 flex-1 px-1 text-sm text-ink-soft">No project yet</p>
        )}
        <button
          type="button"
          onClick={onAddProject}
          disabled={addingProject}
          aria-label="Add a project folder"
          title="Add a project folder"
          className="press-tap flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-edge bg-surface text-ink-soft transition-colors hover:bg-sage-soft hover:text-ink disabled:opacity-60"
        >
          {addingProject ? <Loader2 className="h-4 w-4 animate-spin text-moss" /> : <FolderPlus className="h-4 w-4" />}
        </button>
      </div>

      <button
        type="button"
        onClick={onNewSession}
        disabled={!project}
        className="press-tap mt-3 flex items-center gap-2 rounded-lg bg-sage-soft px-3 py-2 text-sm font-medium text-ink hover:bg-sage-soft/80 disabled:opacity-50"
      >
        <Plus className="h-4 w-4" />
        New session
      </button>

      <nav aria-label="Code sessions" className="hide-scrollbar mt-5 min-h-0 flex-1 overflow-y-auto">
        <p className="px-1 text-[11px] font-semibold uppercase tracking-wider text-ink-soft/70">Sessions</p>
        {!hasProjects ? (
          <p className="mt-2 px-1 text-xs leading-5 text-ink-soft">Add a project folder to start. Each session works inside one project.</p>
        ) : sessions.length === 0 ? (
          <p className="mt-2 px-1 text-xs leading-5 text-ink-soft">No sessions in this project yet.</p>
        ) : (
          <ul className="mt-1.5 space-y-0.5">
            <AnimatePresence initial={false}>
              {sessions.map((item) => (
                <SessionRow key={item.id} item={item} active={item.id === activeSessionId} onDelete={() => onDeleteSession(item.id)} />
              ))}
            </AnimatePresence>
          </ul>
        )}

        {unassigned.length > 0 ? (
          <section className="mt-6">
            <p className="flex items-center gap-1.5 px-1 text-[11px] font-semibold uppercase tracking-wider text-clay">
              <MessageSquareDashed className="h-3 w-3" />
              Needs a project
            </p>
            <p className="mt-1 px-1 text-[11px] leading-4 text-ink-soft">Started on another computer. Pick the folder it belongs to here.</p>
            <ul className="mt-1.5 space-y-2">
              {unassigned.map((item) => (
                <li key={item.id} className="rounded-lg border border-edge bg-surface p-2">
                  <Link href={`/app/code/${encodeURIComponent(item.id)}`} className="block truncate text-sm text-ink hover:underline">
                    {item.title}
                  </Link>
                  {hasProjects ? (
                    <Select
                      className="mt-1.5"
                      value=""
                      placeholder="Assign to a project"
                      options={projectOptions}
                      onChange={(path) => onAssignSession(item.id, path)}
                    />
                  ) : null}
                </li>
              ))}
            </ul>
          </section>
        ) : null}
      </nav>
    </aside>
  );
}

function SessionRow({ item, active, onDelete }: { item: SidebarSession; active: boolean; onDelete: () => void }) {
  return (
    <motion.li
      initial={{ opacity: 0, x: -8 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, transition: { duration: 0.15 } }}
      transition={{ duration: 0.2, ease: EASE_OUT }}
      className="group/session relative"
    >
      <Link
        href={`/app/code/${encodeURIComponent(item.id)}`}
        aria-current={active ? "page" : undefined}
        className={cn(
          "flex w-full flex-col rounded-lg py-1.5 pl-3 pr-8 text-left transition-colors",
          active ? "bg-sage-soft text-ink" : "text-ink-soft hover:bg-sage-soft/60 hover:text-ink",
        )}
      >
        <span className="truncate text-sm">{item.title}</span>
        <span className="font-mono text-[10px] uppercase tracking-wider text-ink-soft/60">{relativeTime(item.createdAt.toISOString())}</span>
      </Link>
      <button
        type="button"
        onClick={onDelete}
        aria-label={`Delete ${item.title}`}
        title="Delete session"
        className="absolute right-1.5 top-1/2 flex h-6 w-6 -translate-y-1/2 items-center justify-center rounded-md text-ink-soft opacity-0 transition-opacity hover:bg-danger/10 hover:text-danger focus-visible:opacity-100 group-hover/session:opacity-100"
      >
        <Trash2 className="h-3.5 w-3.5" />
      </button>
    </motion.li>
  );
}

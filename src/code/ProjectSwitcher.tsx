/* Aloe Code's project picker: which folder the sessions below belong to.
 *
 * One line per project, name first and its parent folder after it, so two folders with the same
 * name can still be told apart without a wall of full paths. Adding a folder is the last row. */

import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Check, ChevronsUpDown, Folder, FolderPlus, Loader2, Search } from "lucide-react";

import { EASE_OUT } from "@aloe/ui/lib/motion";
import { cn } from "@aloe/ui/lib/utils";

import { displayPath, projectName, projectParent } from "./sessions";

type Props = {
  projects: Array<{ value: string; label: string }>;
  value: string;
  onChange: (path: string) => void;
  onAdd: () => void;
  adding: boolean;
};

/* Past this many projects a filter box is worth the space it takes. */
const FILTER_FROM = 6;

export default function ProjectSwitcher({ projects, value, onChange, onAdd, adding }: Props) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [highlight, setHighlight] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const current = projects.find((project) => project.value === value);
  const showFilter = projects.length >= FILTER_FROM;
  const shown = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return projects;
    return projects.filter((project) => project.label.toLowerCase().includes(needle) || displayPath(project.value).toLowerCase().includes(needle));
  }, [projects, query]);
  // The rows the keyboard walks: every project shown, then "Add a project folder".
  const rowCount = shown.length + 1;

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setHighlight(Math.max(0, projects.findIndex((project) => project.value === value)));
    // Focus moves into the menu so the arrow keys and typing work straight away.
    requestAnimationFrame(() => (showFilter ? filterRef.current : listRef.current)?.focus());
    const close = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  }, [open, projects, value, showFilter]);

  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-row="${highlight}"]`)?.scrollIntoView({ block: "nearest" });
  }, [highlight]);

  const dismiss = () => {
    setOpen(false);
    triggerRef.current?.focus();
  };

  const choose = (index: number) => {
    if (index === shown.length) {
      setOpen(false);
      onAdd();
      return;
    }
    const project = shown[index];
    if (!project) return;
    dismiss();
    if (project.value !== value) onChange(project.value);
  };

  const onMenuKey = (event: React.KeyboardEvent) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setHighlight((index) => (index + 1) % rowCount);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setHighlight((index) => (index - 1 + rowCount) % rowCount);
    } else if (event.key === "Enter") {
      event.preventDefault();
      choose(highlight);
    } else if (event.key === "Escape") {
      event.preventDefault();
      dismiss();
    } else if (event.key === "Tab") {
      setOpen(false);
    }
  };

  return (
    <div ref={rootRef} className="relative">
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((isOpen) => !isOpen)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" && !open) {
            event.preventDefault();
            setOpen(true);
          }
        }}
        aria-haspopup="listbox"
        aria-expanded={open}
        title={displayPath(value)}
        className={cn(
          "group flex w-full items-center gap-2.5 rounded-xl border bg-surface px-2.5 py-2 text-left outline-none transition-colors",
          open ? "border-[#c8f36a]/60" : "border-edge hover:border-ink-soft/40 focus-visible:border-[#c8f36a]/60",
        )}
      >
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-sage-soft text-moss">
          <Folder className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-semibold text-ink">{current?.label ?? projectName(value)}</span>
          <span className="block truncate font-mono text-[10.5px] text-ink-soft/70">{projectParent(value)}</span>
        </span>
        <ChevronsUpDown className="h-3.5 w-3.5 shrink-0 text-ink-soft transition-colors group-hover:text-ink" />
      </button>

      <AnimatePresence>
        {open ? (
          <motion.div
            initial={{ opacity: 0, transform: "translateY(-4px) scale(0.98)" }}
            animate={{ opacity: 1, transform: "translateY(0px) scale(1)" }}
            exit={{ opacity: 0, transform: "translateY(-4px) scale(0.98)" }}
            transition={{ duration: 0.14, ease: EASE_OUT }}
            style={{ transformOrigin: "top" }}
            className="absolute inset-x-0 z-30 mt-1.5 overflow-hidden rounded-xl border border-edge bg-surface-strong shadow-[0_12px_32px_rgba(0,0,0,0.28)]"
          >
            {showFilter ? (
              <div className="flex items-center gap-2 border-b border-edge px-3 py-2">
                <Search className="h-3.5 w-3.5 shrink-0 text-ink-soft" />
                <input
                  ref={filterRef}
                  value={query}
                  onChange={(event) => {
                    setQuery(event.target.value);
                    setHighlight(0);
                  }}
                  onKeyDown={onMenuKey}
                  placeholder="Find a project"
                  aria-label="Find a project"
                  className="min-w-0 flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-ink-soft/60"
                />
              </div>
            ) : null}

            <div
              ref={listRef}
              role="listbox"
              aria-label="Projects"
              tabIndex={showFilter ? -1 : 0}
              onKeyDown={showFilter ? undefined : onMenuKey}
              className="hide-scrollbar max-h-72 overflow-y-auto p-1 outline-none"
            >
              {shown.length === 0 ? <p className="px-2.5 py-2 text-xs text-ink-soft">No project matches “{query.trim()}”.</p> : null}
              {shown.map((project, index) => {
                const active = project.value === value;
                return (
                  <button
                    key={project.value}
                    type="button"
                    role="option"
                    aria-selected={active}
                    data-row={index}
                    title={displayPath(project.value)}
                    onPointerMove={() => setHighlight(index)}
                    onClick={() => choose(index)}
                    className={cn(
                      "flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left transition-colors",
                      highlight === index ? "bg-sage-soft" : "",
                    )}
                  >
                    <Folder className={cn("h-4 w-4 shrink-0", active ? "text-moss" : "text-ink-soft/70")} />
                    <span className="flex min-w-0 flex-1 items-baseline gap-2">
                      <span className={cn("shrink-0 truncate text-[13px]", active ? "font-semibold text-ink" : "text-ink")} style={{ maxWidth: "65%" }}>
                        {project.label}
                      </span>
                      <span className="min-w-0 truncate font-mono text-[10.5px] text-ink-soft/60">{projectParent(project.value)}</span>
                    </span>
                    {active ? <Check className="h-3.5 w-3.5 shrink-0 text-[#c8f36a]" /> : null}
                  </button>
                );
              })}
            </div>

            <div className="border-t border-edge p-1">
              <button
                type="button"
                data-row={shown.length}
                onPointerMove={() => setHighlight(shown.length)}
                onClick={() => choose(shown.length)}
                disabled={adding}
                className={cn(
                  "flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-[13px] text-ink-soft transition-colors hover:text-ink disabled:opacity-60",
                  highlight === shown.length ? "bg-sage-soft text-ink" : "",
                )}
              >
                {adding ? <Loader2 className="h-4 w-4 shrink-0 animate-spin text-moss" /> : <FolderPlus className="h-4 w-4 shrink-0" />}
                Add a project folder
              </button>
            </div>
          </motion.div>
        ) : null}
      </AnimatePresence>
    </div>
  );
}

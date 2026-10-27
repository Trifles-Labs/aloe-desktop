/* Files: the project's tree and one open file.
 *
 * Files open read-only; "Edit" makes it plain that the user, not Aloe, is changing the file. When
 * Aloe writes the open file, it reloads, unless the user has unsaved edits, in which case they
 * choose which version to keep. */

import { useCallback, useEffect, useState } from "react";
import { ChevronRight, File, Folder, FolderOpen, Loader2, Pencil, Save, X } from "lucide-react";

import { cn } from "@aloe/ui/lib/utils";
import CodeEditor from "./CodeEditor";
import { PaneMessage } from "./ChangesPane";
import { listDir, readFile, useCodeChangeEvents, writeFile, type DirEntry } from "./bridge";

type Props = {
  projectPath: string | null;
  conversationId: string | null;
  /** Paths Aloe changed in this session, marked in the tree. */
  changedPaths: Set<string>;
  /** A file another pane asked to show; `nonce` repeats a request for the same path. */
  openRequest: { path: string; nonce: number } | null;
};

type OpenFile = {
  path: string;
  /** What is on disk, as of the last load or save. */
  saved: string;
  /** What the editor holds. */
  draft: string;
  editing: boolean;
  /** Bumped to remount the editor with fresh content. */
  revision: number;
  /** Aloe wrote the file while the user had unsaved edits. */
  conflict: boolean;
};

const fileName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;
const isUnder = (path: string, dir: string) => path.startsWith(dir.endsWith("\\") || dir.endsWith("/") ? dir : `${dir}${dir.includes("\\") ? "\\" : "/"}`);

export default function FilesPane({ projectPath, conversationId, changedPaths, openRequest }: Props) {
  const [open, setOpen] = useState<OpenFile | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const openPath = useCallback(async (path: string) => {
    try {
      const content = await readFile(path);
      setOpen((current) => ({ path, saved: content, draft: content, editing: false, revision: (current?.revision ?? 0) + 1, conflict: false }));
      setLoadError(null);
      setSaveError(null);
    } catch (reason) {
      setLoadError(`${fileName(path)}: ${String(reason)}`);
    }
  }, []);

  useEffect(() => {
    if (openRequest) void openPath(openRequest.path);
  }, [openPath, openRequest]);

  // A different project: whatever was open belongs to the old one.
  useEffect(() => {
    setOpen(null);
    setLoadError(null);
  }, [projectPath]);

  /* The file changed on disk: reload it if that loses nothing, else flag the conflict. */
  const syncFromDisk = useCallback(async () => {
    if (!open) return;
    let content: string;
    try {
      content = await readFile(open.path);
    } catch {
      return; // Deleted or unreadable now; keep what is shown.
    }
    if (content === open.saved) return;
    setOpen((current) => {
      if (!current || current.path !== open.path) return current;
      if (current.draft === current.saved) return { ...current, saved: content, draft: content, revision: current.revision + 1, conflict: false };
      return { ...current, conflict: true };
    });
  }, [open]);

  useCodeChangeEvents(conversationId, (path) => {
    if (open && path === open.path) void syncFromDisk();
  });
  useEffect(() => {
    const onFocus = () => void syncFromDisk();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [syncFromDisk]);

  const save = async () => {
    if (!open || !open.editing || saving) return;
    setSaving(true);
    try {
      await writeFile(open.path, open.draft);
      setOpen((current) => (current ? { ...current, saved: current.draft, conflict: false } : current));
      setSaveError(null);
    } catch (reason) {
      setSaveError(String(reason));
    } finally {
      setSaving(false);
    }
  };

  const takeDiskVersion = async () => {
    if (!open) return;
    await openPath(open.path);
  };

  if (!projectPath) {
    return <PaneMessage title="No project">Choose a project to browse its files.</PaneMessage>;
  }

  const dirty = Boolean(open && open.draft !== open.saved);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className={cn("overflow-y-auto py-1", open ? "max-h-[36%] shrink-0 border-b border-edge" : "min-h-0 flex-1")}>
        <TreeLevel path={projectPath} depth={0} selected={open?.path ?? null} changedPaths={changedPaths} onOpen={(path) => void openPath(path)} />
      </div>
      {loadError ? <p className="border-b border-edge px-3 py-1.5 text-xs text-danger">{loadError}</p> : null}

      {open ? (
        <div className="flex min-h-0 flex-1 flex-col">
          <div className="flex items-center gap-1.5 px-3 py-1.5 text-xs">
            <File className="h-3.5 w-3.5 shrink-0 text-ink-soft" />
            <span className="min-w-0 flex-1 truncate font-medium text-ink" title={open.path}>
              {fileName(open.path)}
              {dirty ? <span className="ml-1 text-gold" aria-label="Unsaved changes">●</span> : null}
            </span>
            {open.editing ? (
              <button
                type="button"
                onClick={() => void save()}
                disabled={!dirty || saving}
                title="Save (Ctrl+S)"
                className="inline-flex items-center gap-1 rounded-md bg-moss px-2 py-1 font-semibold text-on-accent hover:bg-moss/90 disabled:opacity-50"
              >
                {saving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Save className="h-3.5 w-3.5" />}
                Save
              </button>
            ) : (
              <button
                type="button"
                onClick={() => setOpen({ ...open, editing: true })}
                className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-ink-soft hover:bg-sage-soft hover:text-ink"
              >
                <Pencil className="h-3.5 w-3.5" />
                Edit
              </button>
            )}
            <button
              type="button"
              onClick={() => {
                if (dirty && !window.confirm("Discard your unsaved changes to this file?")) return;
                setOpen(null);
              }}
              aria-label="Close file"
              className="flex h-6 w-6 items-center justify-center rounded-md text-ink-soft hover:bg-sage-soft hover:text-ink"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
          {open.conflict ? (
            <div className="mx-3 mb-1.5 flex items-center gap-2 rounded-md bg-gold/10 px-2.5 py-1.5 text-xs text-ink">
              <span className="flex-1">Aloe changed this file while you were editing it.</span>
              <button type="button" onClick={() => void takeDiskVersion()} className="font-semibold text-moss hover:underline">
                Load Aloe&apos;s version
              </button>
              <button type="button" onClick={() => setOpen({ ...open, conflict: false })} className="text-ink-soft hover:underline">
                Keep mine
              </button>
            </div>
          ) : null}
          {saveError ? <p className="px-3 pb-1.5 text-xs text-danger">{saveError}</p> : null}
          <div className="mx-3 mb-3 min-h-0 flex-1 overflow-hidden rounded-lg border border-edge">
            <CodeEditor
              key={`${open.path}:${open.revision}`}
              path={open.path}
              initialValue={open.draft}
              readOnly={!open.editing}
              onChange={(draft) => setOpen((current) => (current ? { ...current, draft } : current))}
              onSave={() => void save()}
            />
          </div>
        </div>
      ) : null}
    </div>
  );
}

function TreeLevel({
  path,
  depth,
  selected,
  changedPaths,
  onOpen,
}: {
  path: string;
  depth: number;
  selected: string | null;
  changedPaths: Set<string>;
  onOpen: (path: string) => void;
}) {
  const [entries, setEntries] = useState<DirEntry[] | null>(null);
  const [truncated, setTruncated] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    listDir(path)
      .then((result) => {
        if (cancelled) return;
        setEntries(result.entries);
        setTruncated(result.truncated);
      })
      .catch((reason) => !cancelled && setError(String(reason)));
    return () => {
      cancelled = true;
    };
  }, [path]);

  const indent = { paddingLeft: 12 + depth * 14 };
  if (error) return <p className="py-1 text-xs text-danger" style={indent}>{error}</p>;
  if (!entries) return <p className="py-1 text-xs text-ink-soft" style={indent}>Loading…</p>;
  if (entries.length === 0 && depth === 0) return <p className="px-3 py-2 text-xs text-ink-soft">This folder is empty.</p>;

  return (
    <ul>
      {entries.map((entry) =>
        entry.isDir ? (
          <TreeFolder key={entry.path} entry={entry} depth={depth} selected={selected} changedPaths={changedPaths} onOpen={onOpen} />
        ) : (
          <li key={entry.path}>
            <button
              type="button"
              onClick={() => onOpen(entry.path)}
              style={indent}
              aria-current={selected === entry.path ? "true" : undefined}
              className={cn("flex w-full items-center gap-1.5 py-[3px] pr-3 text-left text-xs", selected === entry.path ? "bg-sage-soft text-ink" : "text-ink-soft hover:bg-sage-soft/60 hover:text-ink")}
            >
              <span className="w-3.5 shrink-0" />
              <File className="h-3.5 w-3.5 shrink-0 opacity-70" />
              <span className="min-w-0 flex-1 truncate">{entry.name}</span>
              {changedPaths.has(entry.path) ? <ChangedDot /> : null}
            </button>
          </li>
        ),
      )}
      {truncated ? <li className="py-1 text-[11px] text-ink-soft/70" style={indent}>Only the first 1,000 entries are listed.</li> : null}
    </ul>
  );
}

function TreeFolder({
  entry,
  depth,
  selected,
  changedPaths,
  onOpen,
}: {
  entry: DirEntry;
  depth: number;
  selected: string | null;
  changedPaths: Set<string>;
  onOpen: (path: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const hasChanges = [...changedPaths].some((path) => isUnder(path, entry.path));

  return (
    <li>
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded}
        style={{ paddingLeft: 12 + depth * 14 }}
        className="flex w-full items-center gap-1.5 py-[3px] pr-3 text-left text-xs text-ink-soft hover:bg-sage-soft/60 hover:text-ink"
      >
        <ChevronRight className={cn("h-3.5 w-3.5 shrink-0 transition-transform", expanded && "rotate-90")} />
        {expanded ? <FolderOpen className="h-3.5 w-3.5 shrink-0 text-moss" /> : <Folder className="h-3.5 w-3.5 shrink-0 text-moss" />}
        <span className="min-w-0 flex-1 truncate">{entry.name}</span>
        {hasChanges ? <ChangedDot /> : null}
      </button>
      {expanded ? <TreeLevel path={entry.path} depth={depth + 1} selected={selected} changedPaths={changedPaths} onOpen={onOpen} /> : null}
    </li>
  );
}

function ChangedDot() {
  return <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-gold" title="Changed by Aloe in this session" />;
}

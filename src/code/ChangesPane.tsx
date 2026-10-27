/* Changes: every file Aloe wrote in this session, as a diff against the file before Aloe's first
 * write to it. Data comes from changes.rs on this device, not git and not the chat transcript. */

import { useMemo, useState } from "react";
import { structuredPatch } from "diff";
import { ChevronRight, FileDiff, FileMinus, FilePen, FilePlus, FolderOpen, Undo2 } from "lucide-react";

import { cn } from "@aloe/ui/lib/utils";
import { revertFile, type ChangedFile } from "./bridge";

type Props = {
  conversationId: string | null;
  files: ChangedFile[];
  error: string | null;
  onRefresh: () => void;
  onOpenFile: (path: string) => void;
};

type DiffLine = { type: "add" | "remove" | "context"; text: string; oldNumber: number | null; newNumber: number | null };
type Hunk = { header: string; lines: DiffLine[] };

const STATUS = {
  added: { icon: FilePlus, label: "Added", className: "text-moss" },
  modified: { icon: FilePen, label: "Modified", className: "text-gold" },
  deleted: { icon: FileMinus, label: "Deleted", className: "text-clay" },
} as const;

const LINE_STYLES: Record<DiffLine["type"], { row: string; gutter: string }> = {
  add: { row: "bg-moss/10 text-moss", gutter: "+" },
  remove: { row: "bg-clay/10 text-clay", gutter: "−" },
  context: { row: "text-ink-soft/80", gutter: " " },
};

/** Hunks with line numbers on both sides, three lines of context around each change. */
export function diffHunks(original: string, current: string): { hunks: Hunk[]; added: number; removed: number } {
  const patch = structuredPatch("a", "b", original, current, "", "", { context: 3 });
  let added = 0;
  let removed = 0;
  const hunks = patch.hunks.map((hunk) => {
    let oldNumber = hunk.oldStart;
    let newNumber = hunk.newStart;
    const lines: DiffLine[] = [];
    for (const raw of hunk.lines) {
      const marker = raw[0];
      const text = raw.slice(1);
      if (marker === "\\") continue; // "\ No newline at end of file"
      if (marker === "+") {
        added += 1;
        lines.push({ type: "add", text, oldNumber: null, newNumber: newNumber++ });
      } else if (marker === "-") {
        removed += 1;
        lines.push({ type: "remove", text, oldNumber: oldNumber++, newNumber: null });
      } else {
        lines.push({ type: "context", text, oldNumber: oldNumber++, newNumber: newNumber++ });
      }
    }
    return { header: `@@ -${hunk.oldStart},${hunk.oldLines} +${hunk.newStart},${hunk.newLines} @@`, lines };
  });
  return { hunks, added, removed };
}

export default function ChangesPane({ conversationId, files, error, onRefresh, onOpenFile }: Props) {
  const [expanded, setExpanded] = useState<string | null>(null);

  if (!conversationId) {
    return <PaneMessage title="No session yet">Files Aloe changes in this session will show up here, with a diff for each.</PaneMessage>;
  }
  if (error) {
    return <PaneMessage title="Couldn't load changes">{error}</PaneMessage>;
  }
  if (files.length === 0) {
    return (
      <PaneMessage title="No changes yet">
        When Aloe edits, creates or deletes a file in this session, it shows up here with a diff you can review or revert.
      </PaneMessage>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ul className="min-h-0 flex-1 overflow-y-auto">
        {files.map((file) => (
          <ChangedFileRow
            key={file.path}
            file={file}
            conversationId={conversationId}
            open={expanded === file.path}
            onToggle={() => setExpanded((current) => (current === file.path ? null : file.path))}
            onReverted={onRefresh}
            onOpenFile={onOpenFile}
          />
        ))}
      </ul>
      <p className="border-t border-edge px-4 py-2 text-[11px] leading-4 text-ink-soft">
        Only files Aloe wrote directly are listed. Files changed by commands it ran (formatters, generators) aren&apos;t.
      </p>
    </div>
  );
}

function ChangedFileRow({
  file,
  conversationId,
  open,
  onToggle,
  onReverted,
  onOpenFile,
}: {
  file: ChangedFile;
  conversationId: string;
  open: boolean;
  onToggle: () => void;
  onReverted: () => void;
  onOpenFile: (path: string) => void;
}) {
  const diff = useMemo(() => (file.binary ? null : diffHunks(file.original ?? "", file.current ?? "")), [file]);
  const [confirming, setConfirming] = useState(false);
  const [revertError, setRevertError] = useState<string | null>(null);
  const status = STATUS[file.status];
  const name = file.relativePath.split("/").pop() ?? file.relativePath;
  const folder = file.relativePath.slice(0, file.relativePath.length - name.length).replace(/\/$/, "");

  const revert = async () => {
    try {
      await revertFile(conversationId, file.path);
      setConfirming(false);
      onReverted();
    } catch (reason) {
      setRevertError(String(reason));
    }
  };

  return (
    <li className="border-b border-edge">
      <button type="button" onClick={onToggle} aria-expanded={open} className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-sage-soft/60">
        <ChevronRight className={cn("h-3.5 w-3.5 shrink-0 text-ink-soft transition-transform", open && "rotate-90")} />
        <status.icon className={cn("h-3.5 w-3.5 shrink-0", status.className)} aria-label={status.label} />
        <span className="min-w-0 flex-1 truncate text-sm" title={file.relativePath}>
          <span className="text-ink">{name}</span>
          {folder ? <span className="ml-1.5 text-xs text-ink-soft">{folder}</span> : null}
        </span>
        {diff ? (
          <span className="shrink-0 font-mono text-[11px]">
            {diff.added ? <span className="text-moss">+{diff.added}</span> : null}
            {diff.removed ? <span className="ml-1 text-clay">−{diff.removed}</span> : null}
          </span>
        ) : null}
      </button>

      {open ? (
        <div className="pb-2">
          <div className="flex items-center gap-1 px-3 pb-1.5">
            {file.status !== "deleted" ? (
              <button type="button" onClick={() => onOpenFile(file.path)} className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs text-ink-soft hover:bg-sage-soft hover:text-ink">
                <FolderOpen className="h-3.5 w-3.5" />
                Open in Files
              </button>
            ) : null}
            <span className="flex-1" />
            {file.binary ? null : confirming ? (
              <>
                <span className="text-xs text-ink-soft">{file.status === "added" ? "Delete this new file?" : "Undo all of Aloe's changes to it?"}</span>
                <button type="button" onClick={() => void revert()} className="rounded-md bg-clay/10 px-2 py-1 text-xs font-semibold text-clay hover:bg-clay/15">
                  Revert
                </button>
                <button type="button" onClick={() => setConfirming(false)} className="rounded-md px-2 py-1 text-xs text-ink-soft hover:bg-sage-soft">
                  Cancel
                </button>
              </>
            ) : (
              <button type="button" onClick={() => setConfirming(true)} className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs text-ink-soft hover:bg-sage-soft hover:text-ink">
                <Undo2 className="h-3.5 w-3.5" />
                Revert file
              </button>
            )}
          </div>
          {revertError ? <p className="px-3 pb-1.5 text-xs text-danger">{revertError}</p> : null}
          {diff ? <DiffView hunks={diff.hunks} /> : <p className="px-3 text-xs text-ink-soft">This file is too large or isn&apos;t text, so there&apos;s no diff to show.</p>}
        </div>
      ) : null}
    </li>
  );
}

function DiffView({ hunks }: { hunks: Hunk[] }) {
  return (
    <div className="mx-3 overflow-x-auto rounded-lg border border-edge bg-surface font-mono text-[11.5px] leading-[1.55]">
      {hunks.map((hunk, index) => (
        <div key={index}>
          <div className="border-b border-edge bg-sage-soft/50 px-2 py-0.5 text-[10.5px] text-ink-soft">{hunk.header}</div>
          <table className="w-full border-collapse">
            <tbody>
              {hunk.lines.map((line, lineIndex) => (
                <tr key={lineIndex} className={LINE_STYLES[line.type].row}>
                  <td className="w-9 select-none px-1 text-right text-ink-soft/50">{line.oldNumber ?? ""}</td>
                  <td className="w-9 select-none px-1 text-right text-ink-soft/50">{line.newNumber ?? ""}</td>
                  <td className="w-3 select-none text-center">{LINE_STYLES[line.type].gutter}</td>
                  <td className="whitespace-pre pr-3">{line.text || " "}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ))}
    </div>
  );
}

export function PaneMessage({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center px-8 text-center">
      <FileDiff className="h-5 w-5 text-ink-soft/50" />
      <p className="mt-2 text-sm font-medium text-ink">{title}</p>
      <p className="mt-1 max-w-xs text-xs leading-5 text-ink-soft">{children}</p>
    </div>
  );
}

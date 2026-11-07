import React from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { Folder, FolderPlus, Trash2 } from "lucide-react";

import Button from "@aloe/ui/components/ui/Button";
import { EASE_OUT } from "@aloe/ui/lib/motion";
import { relativeTime } from "@aloe/ui/lib/utils";
import type { GrantedFolder } from "../types";
import { ControlGroup, EmptyRow } from "./ControlGroup";

type Props = {
  folders: GrantedFolder[];
  onAdd: () => void;
  onRemove: (path: string) => void;
};

/** Grants are stored canonicalized, which on Windows means `\\?\C:\...` (and
    `\\?\UNC\server\share` for a share). Those prefixes are how the OS spells a
    path to itself, not how a person writes one: the row shows the plain form.
    Only the display is rewritten — the grant, its tooltip and `onRemove` keep
    the exact stored path, since that is what the backend and Rust compare. */
function displayPath(path: string): string {
  if (path.startsWith("\\\\?\\UNC\\")) return `\\\\${path.slice(8)}`;
  return path.startsWith("\\\\?\\") ? path.slice(4) : path;
}

export function FoldersPanel({ folders, onAdd, onRemove }: Props) {
  const reduceMotion = useReducedMotion();

  return (
    <ControlGroup
      label="Folder access"
      action={
        <Button variant="secondary" size="sm" className="press-tap -mb-1 min-h-0 px-3 py-1.5 text-xs" onClick={onAdd}>
          <FolderPlus className="h-3.5 w-3.5" />
          Add folder
        </Button>
      }
      footnote="Aloe can read, search, and write only inside the folders listed here."
    >
      {folders.length === 0 ? (
        <EmptyRow>No folders granted yet. Add one to let Aloe work with files on this computer.</EmptyRow>
      ) : (
        <AnimatePresence initial={false}>
          {folders.map((folder) => (
            <motion.div
              key={folder.path}
              layout={!reduceMotion}
              initial={reduceMotion ? { opacity: 0 } : { opacity: 0, transform: "translateY(-6px)" }}
              animate={{ opacity: 1, transform: "translateY(0px)" }}
              /* Fades rather than collapsing: `.settings-row` carries a
                 min-height, so animating height to 0 stops half way and jumps.
                 The rows below close the gap with their layout animation. */
              exit={{ opacity: 0 }}
              transition={{ duration: 0.22, ease: EASE_OUT }}
              /* `group` here, not on the row's parent: the remove control belongs
                 to one folder and should surface for that folder only. */
              className="settings-row group overflow-hidden"
            >
              <div className="flex min-w-0 items-start gap-3">
                <Folder className="mt-0.5 h-4 w-4 shrink-0 text-moss" />
                <div className="min-w-0">
                  <p className="truncate text-[13px] font-medium leading-5 text-ink">{folder.label ?? "Folder"}</p>
                  <p className="truncate font-mono text-[11px] leading-5 text-ink-soft" title={displayPath(folder.path)}>
                    {displayPath(folder.path)}
                  </p>
                </div>
              </div>
              <div className="flex shrink-0 items-center gap-3">
                <span className="text-[11px] text-ink-soft">{folder.indexedAt ? `Indexed ${relativeTime(folder.indexedAt).toLowerCase()}` : "Not indexed"}</span>
                <button
                  type="button"
                  onClick={() => onRemove(folder.path)}
                  title={`Remove ${folder.label ?? folder.path}`}
                  aria-label={`Remove ${folder.label ?? folder.path}`}
                  /* Quiet until wanted, but not only where a pointer can hover: hiding it behind
                     `group-hover` left it unreachable on a pen or touch screen, where `:hover`
                     never fires. The opacity is gated on the same media query Tailwind's own
                     `hover:` uses, so a mouse still sees a clean row and every other device sees
                     the button. */
                  className="inline-flex h-8 w-8 items-center justify-center rounded-lg text-ink-soft transition-[opacity,background-color,color,transform] duration-150 hover:bg-clay/12 hover:text-danger focus-visible:opacity-100 active:scale-[0.97] motion-reduce:active:scale-100 [@media(hover:hover)]:opacity-0 [@media(hover:hover)]:group-hover:opacity-100"
                >
                  <Trash2 className="h-4 w-4" />
                </button>
              </div>
            </motion.div>
          ))}
        </AnimatePresence>
      )}

    </ControlGroup>
  );
}

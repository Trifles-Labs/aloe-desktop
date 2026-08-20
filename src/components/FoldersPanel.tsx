import React from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { Folder, FolderPlus, MessageSquare, Trash2 } from "lucide-react";

import Button from "@/components/ui/Button";
import { EASE_OUT } from "@/lib/motion";
import { relativeTime } from "@/lib/utils";
import type { ConversationFolder, GrantedFolder } from "../types";
import { ControlGroup, EmptyRow } from "./ControlGroup";

type Props = {
  folders: GrantedFolder[];
  conversationFolders: ConversationFolder[];
  onAdd: () => void;
  onRemove: (path: string) => void;
};

export function FoldersPanel({ folders, conversationFolders, onAdd, onRemove }: Props) {
  const reduceMotion = useReducedMotion();
  // One row per folder, not per grant: the same folder shared with three chats is one thing the
  // user cares about here, and the chat it belongs to is named in the web app, not on this device.
  const sharedPaths = Array.from(
    new Map(conversationFolders.map((folder) => [folder.path, folder])).values(),
  );

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
                  <p className="truncate font-mono text-[11px] leading-5 text-ink-soft" title={folder.path}>
                    {folder.path}
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
                  /* Quiet until wanted, and only where a real pointer exists —
                     Tailwind's `hover:` already carries the (hover: hover) query.
                     The transitions are spelled out rather than using `.press-tap`:
                     a `transition-*` utility sorts after that layer and would
                     replace its whole property list, silently dropping one. */
                  className="inline-flex h-8 w-8 items-center justify-center rounded-lg text-ink-soft opacity-0 transition-[opacity,background-color,color,transform] duration-150 hover:bg-clay/12 hover:text-danger focus-visible:opacity-100 active:scale-[0.97] motion-reduce:active:scale-100 group-hover:opacity-100"
                >
                  <Trash2 className="h-4 w-4" />
                </button>
              </div>
            </motion.div>
          ))}
        </AnimatePresence>
      )}

      {sharedPaths.length > 0 ? (
        <div className="settings-row flex-col items-start gap-2 border-t border-clay/12 pt-3">
          <p className="text-[11px] font-medium uppercase tracking-wide text-ink-soft">Shared with a chat</p>
          {sharedPaths.map((folder) => (
            <div key={folder.path} className="flex min-w-0 items-start gap-3">
              <MessageSquare className="mt-0.5 h-4 w-4 shrink-0 text-ink-soft" />
              <div className="min-w-0">
                <p className="truncate text-[13px] font-medium leading-5 text-ink">{folder.label ?? "Folder"}</p>
                <p className="truncate font-mono text-[11px] leading-5 text-ink-soft" title={folder.path}>
                  {folder.path}
                </p>
              </div>
            </div>
          ))}
          <p className="text-[11px] leading-4 text-ink-soft">
            Reachable only from the chat it was shared with. Remove it there to take the access away.
          </p>
        </div>
      ) : null}
    </ControlGroup>
  );
}

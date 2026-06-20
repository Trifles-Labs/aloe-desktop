import React from "react";
import { FolderPlus, Trash2 } from "lucide-react";
import type { GrantedFolder } from "../types";

type Props = { folders: GrantedFolder[]; onAdd: () => void; onRemove: (path: string) => void };

export function FoldersPanel({ folders, onAdd, onRemove }: Props) {
  return (
    <section className="liquid-glass rounded-3xl p-5 sm:p-6">
      <div className="flex items-center justify-between gap-4">
        <div><p className="eyebrow">Local access</p><h2 className="mt-1 font-display text-xl font-semibold text-[#0b3026] dark:text-[#e8f0e0]">Granted folders</h2></div>
        <button className="secondary-button min-h-0 px-3 py-2 text-xs" onClick={onAdd}><FolderPlus className="h-3.5 w-3.5" />Add folder</button>
      </div>
      <div className="mt-5 space-y-2">
        {folders.length === 0 ? <p className="rounded-2xl border border-dashed border-[#d9e0d5] px-4 py-6 text-center text-sm text-[#6b786f] dark:border-[#2a3a28] dark:text-[#78907e]">No folders granted yet.</p> : null}
        {folders.map((folder) => (
          <div className="flex items-center gap-3 rounded-2xl bg-white/45 p-3 dark:bg-[#152118]/55" key={folder.path}>
            <div className="min-w-0 flex-1"><strong className="block truncate text-sm text-[#0b3026] dark:text-[#e8f0e0]">{folder.label ?? "Folder"}</strong><span className="block truncate text-xs text-[#6b786f] dark:text-[#78907e]">{folder.path}</span></div>
            <button className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[#a65345] transition-colors hover:bg-[#fff1ed] dark:text-[#e8917f] dark:hover:bg-[#33211d]" onClick={() => onRemove(folder.path)} title="Remove folder"><Trash2 className="h-4 w-4" /></button>
          </div>
        ))}
      </div>
    </section>
  );
}

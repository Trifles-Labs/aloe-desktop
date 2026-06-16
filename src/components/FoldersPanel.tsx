import React from "react";
import { FolderPlus, Trash2 } from "lucide-react";
import type { GrantedFolder } from "../types";

type Props = {
  folders: GrantedFolder[];
  onAdd: () => void;
  onRemove: (path: string) => void;
};

export function FoldersPanel({ folders, onAdd, onRemove }: Props) {
  return (
    <div className="panel">
      <div className="panel-header">
        <h2>Granted folders</h2>
        <button className="icon-button" onClick={onAdd} title="Add folder">
          <FolderPlus size={16} />
        </button>
      </div>

      <div className="folder-list">
        {folders.length === 0 && (
          <p className="muted">No folders granted yet. Click + to add one.</p>
        )}
        {folders.map((folder) => (
          <div className="folder-row" key={folder.path}>
            <div>
              <strong>{folder.label ?? "Folder"}</strong>
              <span>{folder.path}</span>
            </div>
            <button
              className="icon-button danger"
              onClick={() => onRemove(folder.path)}
              title="Remove folder"
            >
              <Trash2 size={14} />
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}

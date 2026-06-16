import React, { useState } from "react";
import { FileText, Folder, Loader2, Search } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import type { GrantedFolder, SearchMatch, SearchMode, ToastVariant } from "../types";
import { pathBasename, pathDirname } from "../types";

type Props = {
  folders: GrantedFolder[];
  onToast: (message: string, variant: ToastVariant) => void;
};

export function SearchPanel({ folders, onToast }: Props) {
  const [mode, setMode] = useState<SearchMode>("content");
  const [folder, setFolder] = useState("");
  const [pattern, setPattern] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [results, setResults] = useState<SearchMatch[] | null>(null);
  const [running, setRunning] = useState(false);

  const changeMode = (next: SearchMode) => {
    setMode(next);
    setResults(null);
  };

  const runSearch = async () => {
    if (!pattern.trim() || !folder) return;
    setRunning(true);
    setResults(null);
    try {
      const cmd = mode === "content" ? "search_content" : "search_files";
      const found = await invoke<SearchMatch[]>(cmd, {
        path: folder,
        pattern,
        caseSensitive,
      });
      setResults(found);
      onToast(
        found.length === 0
          ? "No matches found."
          : `${found.length} match${found.length === 1 ? "" : "es"} found.`,
        found.length === 0 ? "info" : "success",
      );
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err);
      onToast(`Search failed: ${detail}`, "error");
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="panel">
      <div className="panel-header">
        <h2>Search</h2>
        <div className="search-mode-toggle">
          <button className={mode === "content" ? "active" : ""} onClick={() => changeMode("content")}>
            <FileText size={13} /> In files
          </button>
          <button className={mode === "files" ? "active" : ""} onClick={() => changeMode("files")}>
            <Folder size={13} /> Find files
          </button>
        </div>
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <label className="field-label">
          Folder
          <select value={folder} onChange={(e) => setFolder(e.target.value)}>
            <option value="">Select a granted folder…</option>
            {folders.map((f) => (
              <option key={f.path} value={f.path}>{f.label ?? f.path}</option>
            ))}
          </select>
        </label>

        <label className="field-label">
          {mode === "content" ? "Regex pattern" : "Filename regex"}
          <div className="search-input-row">
            <input
              value={pattern}
              onChange={(e) => setPattern(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void runSearch()}
              placeholder={mode === "content" ? "e.g. TODO|FIXME" : "e.g. \\.tsx?$"}
            />
            <label className="case-toggle" title="Case sensitive">
              <input
                type="checkbox"
                checked={caseSensitive}
                onChange={(e) => setCaseSensitive(e.target.checked)}
              />
              <span>Aa</span>
            </label>
          </div>
        </label>

        <button
          className="primary"
          style={{ alignSelf: "flex-start" }}
          onClick={() => void runSearch()}
          disabled={!pattern.trim() || !folder || running}
        >
          {running ? <Loader2 size={14} className="spin" /> : <Search size={14} />}
          {running ? "Searching…" : "Search"}
        </button>
      </div>

      {results !== null && (
        <div className="search-results">
          {results.length === 0 ? (
            <p className="muted">No matches found.</p>
          ) : (
            results.map((r, i) => (
              <div key={i} className="search-result-row">
                <div className="search-result-header">
                  <div className="search-result-name">
                    {r.matchType === "directory"
                      ? <Folder size={13} style={{ flexShrink: 0, opacity: 0.6 }} />
                      : <FileText size={13} style={{ flexShrink: 0, opacity: 0.6 }} />}
                    {pathBasename(r.path)}
                  </div>
                  {r.line != null && <mark className="mark-running">:{r.line}</mark>}
                </div>
                <div className="search-result-dir">{pathDirname(r.path)}</div>
                {r.preview && <div className="search-result-preview">{r.preview}</div>}
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}

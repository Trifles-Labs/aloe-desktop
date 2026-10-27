/* The Aloe Code panes' side of the native commands in src-tauri (changes.rs, workbench.rs and the
 * code_terminal_* commands in lib.rs), plus hooks that keep each pane's data current.
 *
 * Every path here is checked against the device's granted folders on the Rust side; nothing in
 * this file is a security boundary. */

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type ChangeStatus = "added" | "modified" | "deleted";

export type ChangedFile = {
  path: string;
  relativePath: string;
  status: ChangeStatus;
  /** Null when the file didn't exist before, or wasn't text. */
  original: string | null;
  current: string | null;
  /** Too large or not text: listed, but no diff and no revert. */
  binary: boolean;
};

export type TerminalStatus = { state: "running" | "exited" | "stopped" | string; exitCode: number | null };

export type TerminalSession = {
  sessionId: string;
  command: string;
  cwd: string;
  startedAt: string;
  status: TerminalStatus;
};

export type DirEntry = { name: string; path: string; isDir: boolean };

const CHANGES_EVENT = "agent://code-changes";
const TERMINAL_EVENT = "agent://terminal-output";

export const revertFile = (conversationId: string, path: string) => invoke<void>("code_revert_file", { conversationId, path });
export const stopTerminal = (sessionId: string) => invoke<unknown>("code_terminal_stop", { sessionId });
export const listDir = (path: string) => invoke<{ entries: DirEntry[]; truncated: boolean }>("code_list_dir", { path });
export const readFile = (path: string) => invoke<string>("code_read_file", { path });
export const writeFile = (path: string, content: string) => invoke<void>("code_write_file", { path, content });

/** Subscribes to a native event for the component's lifetime. */
export function useNativeEvent<T>(event: string, handler: (payload: T) => void) {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen<T>(event, (message) => handlerRef.current(message.payload)).then((stop) => {
      if (cancelled) stop();
      else unlisten = stop;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [event]);
}

/** Fires on every file write Aloe makes in this session. */
export function useCodeChangeEvents(conversationId: string | null, onChange: (path: string) => void) {
  useNativeEvent<{ conversationId: string; path: string }>(CHANGES_EVENT, (payload) => {
    if (conversationId && payload.conversationId === conversationId) onChange(payload.path);
  });
}

/** The files Aloe changed in a session, refreshed on each of its writes and when the window regains focus. */
export function useSessionChanges(conversationId: string | null, projectPath: string | null) {
  const [files, setFiles] = useState<ChangedFile[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!conversationId || !projectPath) {
      setFiles([]);
      return;
    }
    try {
      const result = await invoke<{ files: ChangedFile[] }>("code_session_changes", { conversationId, projectPath });
      setFiles(result.files);
      setError(null);
    } catch (reason) {
      setError(String(reason));
    }
  }, [conversationId, projectPath]);

  useEffect(() => {
    setFiles([]);
    void refresh();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [refresh]);
  useCodeChangeEvents(conversationId, () => void refresh());

  return { files, error, refresh };
}

/** The commands a session ran, refreshed whenever one starts or prints. */
export function useTerminalSessions(conversationId: string | null) {
  const [sessions, setSessions] = useState<TerminalSession[]>([]);

  const refresh = useCallback(async () => {
    if (!conversationId) {
      setSessions([]);
      return;
    }
    try {
      const result = await invoke<{ sessions: TerminalSession[] }>("code_terminal_sessions", { conversationId });
      setSessions(result.sessions);
    } catch {
      // Keep what is shown; the next event retries.
    }
  }, [conversationId]);

  useEffect(() => {
    setSessions([]);
    void refresh();
  }, [refresh]);

  /* Output events also mean "a session may have exited", but they come per chunk: one list
     refresh per burst is plenty. */
  const pending = useRef<number | null>(null);
  useNativeEvent<{ sessionId: string }>(TERMINAL_EVENT, () => {
    if (pending.current !== null) return;
    pending.current = window.setTimeout(() => {
      pending.current = null;
      void refresh();
    }, 400);
  });
  useEffect(() => () => {
    if (pending.current !== null) window.clearTimeout(pending.current);
  }, []);

  return { sessions, refresh };
}

/** One command's retained output, followed live. Polls once a second while it runs, since the
 *  native side throttles its events and can leave the last chunk of a quiet stream unannounced. */
export function useTerminalOutput(sessionId: string | null, running: boolean) {
  const [output, setOutput] = useState("");
  const [status, setStatus] = useState<TerminalStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!sessionId) return;
    try {
      const result = await invoke<{ output: { content: string }; status: TerminalStatus }>("code_terminal_read", { sessionId });
      setOutput(result.output.content);
      setStatus(result.status);
      setError(null);
    } catch (reason) {
      setError(String(reason));
    }
  }, [sessionId]);

  useEffect(() => {
    setOutput("");
    setStatus(null);
    setError(null);
    void refresh();
  }, [refresh]);

  useNativeEvent<{ sessionId: string }>(TERMINAL_EVENT, (payload) => {
    if (payload.sessionId === sessionId) void refresh();
  });

  useEffect(() => {
    // An error means the session is gone (stopped, or from before a restart): nothing left to follow.
    if (!running || error) return;
    const timer = window.setInterval(() => void refresh(), 1000);
    return () => window.clearInterval(timer);
  }, [error, refresh, running]);

  return { output, status, error };
}

/* Which conversations are Aloe Code sessions, and the project each one works in.
 *
 * The backend has no notion of a coding session — a session is an ordinary
 * conversation whose messages carry the project as hidden context — so the
 * mapping lives on this device. It is per-machine by nature anyway: a project
 * is a path on this disk. */

import { useSyncExternalStore } from "react";

export type CodeSession = {
  conversationId: string;
  projectPath: string;
  createdAt: string;
};

const SESSIONS_KEY = "aloe_code_sessions";
const LAST_PROJECT_KEY = "aloe_code_last_project";
const LAST_ROUTE_KEY = "aloe_code_last_route";
const CHANGE_EVENT = "aloe:code-sessions";

/* Draft ids are recorded too (the conversation only learns its real id when
   the first reply finishes), and they never resolve to a listed conversation
   afterwards. Capping the map keeps those from piling up forever. */
const MAX_SESSIONS = 500;

let cache: Record<string, CodeSession> | null = null;

function read(): Record<string, CodeSession> {
  if (cache) return cache;
  try {
    const parsed = JSON.parse(window.localStorage.getItem(SESSIONS_KEY) ?? "{}") as Record<string, CodeSession>;
    cache = parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    cache = {};
  }
  return cache;
}

function write(next: Record<string, CodeSession>) {
  const entries = Object.values(next).sort((a, b) => b.createdAt.localeCompare(a.createdAt)).slice(0, MAX_SESSIONS);
  cache = Object.fromEntries(entries.map((session) => [session.conversationId, session]));
  try {
    window.localStorage.setItem(SESSIONS_KEY, JSON.stringify(cache));
  } catch {
    // Storage full or blocked: the session still works for this run.
  }
  window.dispatchEvent(new Event(CHANGE_EVENT));
}

export function recordCodeSession(conversationId: string, projectPath: string) {
  const sessions = read();
  if (sessions[conversationId]?.projectPath === projectPath) return;
  write({ ...sessions, [conversationId]: { conversationId, projectPath, createdAt: sessions[conversationId]?.createdAt ?? new Date().toISOString() } });
}

export function forgetCodeSession(conversationId: string) {
  const sessions = read();
  if (!sessions[conversationId]) return;
  const next = { ...sessions };
  delete next[conversationId];
  write(next);
}

export function codeSessionFor(conversationId: string | null | undefined): CodeSession | null {
  return conversationId ? read()[conversationId] ?? null : null;
}

export function isCodeSession(conversationId: string): boolean {
  return Boolean(read()[conversationId]);
}

const subscribe = (listener: () => void) => {
  window.addEventListener(CHANGE_EVENT, listener);
  window.addEventListener("storage", listener);
  return () => {
    window.removeEventListener(CHANGE_EVENT, listener);
    window.removeEventListener("storage", listener);
  };
};

/** The whole map, re-rendering on change. The snapshot is the cached object, so it is stable between writes. */
export function useCodeSessions(): Record<string, CodeSession> {
  return useSyncExternalStore(subscribe, read, read);
}

export function readLastProject(): string | null {
  try {
    return window.localStorage.getItem(LAST_PROJECT_KEY);
  } catch {
    return null;
  }
}

export function storeLastProject(path: string) {
  try {
    window.localStorage.setItem(LAST_PROJECT_KEY, path);
  } catch {
    // Only a convenience.
  }
}

/** Where Code mode was last left, so switching back from Chat lands on the same session. */
export function readLastCodeRoute(): string {
  try {
    return window.localStorage.getItem(LAST_ROUTE_KEY) ?? "/app/code";
  } catch {
    return "/app/code";
  }
}

export function storeLastCodeRoute(path: string) {
  try {
    window.localStorage.setItem(LAST_ROUTE_KEY, path);
  } catch {
    // Only a convenience.
  }
}

/** Trailing segment of a path. */
export function projectName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/* What the model is told on every message of a session, beyond what the backend
   already knows. The coding instructions and the project folder live in the
   backend's fixed Aloe Code system prompt (it gets the path as `projectPath`);
   repeating them here put the same paragraph into history once per message.
   Only the branch is left, since it can change mid-session. */
export function codeSessionContext(branch: string | null): string | null {
  return branch ? `Git branch: ${branch}` : null;
}

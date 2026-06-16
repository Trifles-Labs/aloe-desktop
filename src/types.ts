// ── Domain types ──────────────────────────────────────────────────────────────

export type GrantedFolder = {
  path: string;
  label: string | null;
  indexedAt: string | null;
};

export type RecentAction = {
  jobId: string;
  kind: string;
  status: string;
  detail: string;
  timestamp: string;
  input?: unknown;
  output?: unknown;
};

export type AgentConfig = {
  apiUrl: string;
  agentId: string | null;
  credential: string | null;
  deviceName: string;
  platform: string;
  socketStatus: string;
  socketError: string | null;
  alwaysAllowCommands: boolean;
  folders: GrantedFolder[];
  recentActions: RecentAction[];
};

export type PendingApproval = {
  jobId: string;
  command: string;
  cwd: string;
  reason: string;
  requestedAt: string;
};

export type SearchMode = "content" | "files";

export type SearchMatch = {
  path: string;
  matchType: "content" | "file" | "directory";
  line: number | null;
  preview: string | null;
};

export type ToastVariant = "success" | "error" | "info";

export type Toast = {
  id: string;
  message: string;
  variant: ToastVariant;
};

// ── Default values ────────────────────────────────────────────────────────────

export const DEFAULT_CONFIG: AgentConfig = {
  apiUrl: "",
  agentId: null,
  credential: null,
  deviceName: "Aloe Desktop",
  platform: "unknown",
  socketStatus: "disconnected",
  socketError: null,
  alwaysAllowCommands: false,
  folders: [],
  recentActions: [],
};

// ── Utility functions ─────────────────────────────────────────────────────────

export function pathBasename(p: string): string {
  return p.replace(/\\/g, "/").split("/").pop() ?? p;
}

export function pathDirname(p: string): string {
  const parts = p.replace(/\\/g, "/").split("/");
  parts.pop();
  return parts.join("/");
}

export function statusMarkClass(status: string): string {
  const s = status.toLowerCase();
  if (s === "failed" || s === "error" || s === "denied") return "mark-error";
  if (s === "pending" || s === "waiting") return "mark-pending";
  if (s === "running" || s === "active" || s === "in_progress") return "mark-running";
  return "";
}

export function formatTimestamp(ts: string): string {
  try {
    return new Date(ts).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return ts;
  }
}

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
  userToken: string | null;
  userProfile: { id: string; name: string; email: string; picture: string } | null;
  deviceName: string;
  platform: string;
  socketStatus: string;
  socketError: string | null;
  alwaysAllowCommands: boolean;
  commandTrustMode: "ask" | "trusted_coding";
  runOnStartup: boolean;
  startMinimized: boolean;
  folders: GrantedFolder[];
  recentActions: RecentAction[];
  terminalSessions: Array<{ sessionId: string; command: string; cwd: string; startedAt: string; status: string; exitCode: number | null }>;
};

export type PendingApproval = {
  jobId: string;
  command: string;
  cwd: string;
  reason: string;
  requestedAt: string;
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
  userToken: null,
  userProfile: null,
  deviceName: "Aloe Desktop",
  platform: "unknown",
  socketStatus: "disconnected",
  socketError: null,
  alwaysAllowCommands: false,
  commandTrustMode: "ask",
  runOnStartup: false,
  startMinimized: false,
  folders: [],
  recentActions: [],
  terminalSessions: [],
};


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

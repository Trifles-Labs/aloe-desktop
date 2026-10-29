// ── Domain types ──────────────────────────────────────────────────────────────

export type GrantedFolder = {
  path: string;
  label: string | null;
  indexedAt: string | null;
};

/** A folder the user shared with one chat rather than granting to this device. */
export type ConversationFolder = {
  conversationId: string;
  path: string;
  label: string | null;
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

export type CommandTrustMode = "ask" | "auto" | "all";

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
  commandTrustMode: CommandTrustMode;
  desktopControlEnabled: boolean;
  voiceEnabled: boolean;
  voiceConversationId: string | null;
  voice: VoicePrefs;
  runOnStartup: boolean;
  startMinimized: boolean;
  folders: GrantedFolder[];
  conversationFolders: ConversationFolder[];
  recentActions: RecentAction[];
  terminalSessions: Array<{ sessionId: string; command: string; cwd: string; startedAt: string; status: string; exitCode: number | null }>;
};

/** Mirrors VoicePrefs in src-tauri/src/models.rs. */
export type VoicePrefs = {
  readAloud: boolean;
  pace: "slow" | "normal" | "fast";
  soundCues: boolean;
  cueVolume: number;
  tapToTalk: boolean;
  followUps: boolean;
  textSize: "s" | "m" | "l" | "xl";
  highContrast: boolean;
  linger: "short" | "long" | "until_dismissed";
};

export const DEFAULT_VOICE_PREFS: VoicePrefs = {
  readAloud: true,
  pace: "normal",
  soundCues: true,
  cueVolume: 0.6,
  tapToTalk: true,
  followUps: true,
  textSize: "l",
  highContrast: false,
  linger: "long",
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
  commandTrustMode: "auto",
  desktopControlEnabled: false,
  voiceEnabled: true,
  voiceConversationId: null,
  voice: DEFAULT_VOICE_PREFS,
  runOnStartup: false,
  startMinimized: false,
  folders: [],
  conversationFolders: [],
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

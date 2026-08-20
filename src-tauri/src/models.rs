use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantedFolder {
    pub path: String,
    pub label: Option<String>,
    pub indexed_at: Option<String>,
    /// The folder's own MEMORY.md / AGENTS.md content, if present — see fs::scan_folder_context.
    /// `default` keeps deserialization of a config.json saved before this field existed working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopUserProfile {
    pub id: String,
    pub name: String,
    pub email: String,
    pub picture: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentConfig {
    pub api_url: String,
    pub agent_id: Option<String>,
    pub credential: Option<String>,
    pub user_token: Option<String>,
    pub user_profile: Option<DesktopUserProfile>,
    pub device_name: String,
    pub platform: String,
    pub socket_status: String,
    pub socket_error: Option<String>,
    pub always_allow_commands: bool,
    pub command_trust_mode: String,
    pub run_on_startup: bool,
    pub start_minimized: bool,
    pub has_shown_tray_notification: bool,
    pub folders: Vec<GrantedFolder>,
    /// Folders the user shared with one conversation rather than with this device. Kept separate
    /// from `folders` on purpose: these must never widen what a *different* chat, a scheduled
    /// task, or a background turn can reach, and they must never be sent up as device grants.
    pub conversation_folders: Vec<ConversationFolder>,
    pub recent_actions: Vec<RecentAction>,
    pub terminal_sessions: Vec<PersistedTerminalSession>,
}

/// One folder granted to one conversation. The backend is the source of truth and pushes the full
/// live list per conversation (see the `conversation_folders` socket message); this is the local
/// copy the agent actually enforces against, so a job claiming a conversation it was never granted
/// still gets nowhere.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationFolder {
    pub conversation_id: String,
    pub path: String,
    pub label: Option<String>,
}

/// Backend relaying the current grant list for one conversation — replaces whatever this device
/// holds for that conversation, including with an empty list on revoke.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationFoldersMessage {
    pub conversation_id: String,
    #[serde(default)]
    pub folders: Vec<ConversationFolderEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationFolderEntry {
    pub path: String,
    #[serde(default)]
    pub label: Option<String>,
}

/// Backend asking this device to open its native folder dialog, because the user is sharing a
/// folder with a chat from the web app. The path never comes from the browser — only from the OS
/// dialog the user operates here.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderPickRequest {
    pub request_id: String,
    pub conversation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedTerminalSession {
    pub session_id: String,
    pub command: String,
    pub cwd: String,
    pub started_at: String,
    pub status: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentAction {
    pub job_id: String,
    pub kind: String,
    pub status: String,
    pub detail: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    pub job_id: String,
    pub job_kind: String,
    /// Conversation the job came from, carried through the approval queue so the command runs
    /// against the same folder scope it was validated against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub command: String,
    pub cwd: String,
    pub reason: String,
    pub requested_at: String,
    pub input: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterResponse {
    pub agent_id: String,
    pub credential: String,
    pub user_token: Option<String>,
    pub user: Option<DesktopUserProfile>,
}

#[derive(Debug, Deserialize)]
pub struct ErrorResponse {
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct JobList {
    pub jobs: Vec<AgentJob>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentJob {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub input: Value,
    /// Set for jobs dispatched from a chat. Selects which per-conversation folder grants apply —
    /// `None` means device grants only. `default` keeps the HTTP drain path (which predates this
    /// field) deserializing.
    #[serde(default, rename = "conversationId")]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SocketJobMessage {
    #[serde(rename = "type")]
    pub kind: String,
    pub job: Option<AgentJob>,
}

/// Backend relaying a web/desktop-queue approval decision back down to this agent —
/// see `job_decision` in aloe-backend's agent_connections.ts.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDecisionMessage {
    pub job_id: String,
    pub approved: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    pub path: String,
    pub match_type: String,
    pub line: Option<u32>,
    pub preview: Option<String>,
}

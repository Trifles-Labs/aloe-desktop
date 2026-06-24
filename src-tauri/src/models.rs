use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantedFolder {
    pub path: String,
    pub label: Option<String>,
    pub indexed_at: Option<String>,
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
    pub wake_word_enabled: bool,
    pub voice_conversation_id: Option<String>,
    pub has_shown_tray_notification: bool,
    pub folders: Vec<GrantedFolder>,
    pub recent_actions: Vec<RecentAction>,
    pub terminal_sessions: Vec<PersistedTerminalSession>,
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
}

#[derive(Debug, Deserialize)]
pub struct SocketJobMessage {
    #[serde(rename = "type")]
    pub kind: String,
    pub job: Option<AgentJob>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    pub path: String,
    pub match_type: String,
    pub line: Option<u32>,
    pub preview: Option<String>,
}

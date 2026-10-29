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
    /// Lets Aloe move the mouse and type on this computer (computer.rs). Off unless the user turns
    /// it on, and switched back off by the corner failsafe.
    pub desktop_control_enabled: bool,
    /// Hold Left Ctrl + Left Alt to talk to Aloe (voice/). On unless the user turns it off; a
    /// config.json saved before this field existed reads as on, not as the struct's `false`.
    #[serde(default = "default_true")]
    pub voice_enabled: bool,
    /// The one conversation every push-to-talk turn joins. Minted here as a client id on first use;
    /// the backend creates the conversation under it and answers to it from then on.
    pub voice_conversation_id: Option<String>,
    /// How voice sounds and looks. Missing from older config.json files, which get the defaults.
    pub voice: VoicePrefs,
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

fn default_true() -> bool {
    true
}

/// Push-to-talk preferences (Settings › Desktop app › Voice). The defaults suit someone who
/// can't see the screen well: everything spoken, with sound cues, and no need to hold keys.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct VoicePrefs {
    /// Read Aloe's replies and progress aloud. Questions and approvals are always spoken.
    pub read_aloud: bool,
    /// "slow", "normal" or "fast".
    pub pace: String,
    /// Short tones for listening, thinking, your turn, error.
    pub sound_cues: bool,
    /// 0–1.
    pub cue_volume: f32,
    /// A quick tap of the chord starts listening, and listening ends by itself when you stop.
    pub tap_to_talk: bool,
    /// After Aloe asks something, listen for the answer without a key press.
    pub follow_ups: bool,
    /// "s", "m", "l" or "xl".
    pub text_size: String,
    pub high_contrast: bool,
    /// "short" (6s), "long" (20s) or "until_dismissed".
    pub linger: String,
}

impl Default for VoicePrefs {
    fn default() -> Self {
        Self {
            read_aloud: true,
            pace: "normal".to_string(),
            sound_cues: true,
            cue_volume: 0.6,
            tap_to_talk: true,
            follow_ups: true,
            text_size: "l".to_string(),
            high_contrast: false,
            linger: "long".to_string(),
        }
    }
}

impl VoicePrefs {
    /// Coerces anything unknown (a hand-edited config.json) back to a default.
    pub fn normalized(mut self) -> Self {
        let defaults = VoicePrefs::default();
        if !matches!(self.pace.as_str(), "slow" | "normal" | "fast") {
            self.pace = defaults.pace;
        }
        if !matches!(self.text_size.as_str(), "s" | "m" | "l" | "xl") {
            self.text_size = defaults.text_size;
        }
        if !matches!(self.linger.as_str(), "short" | "long" | "until_dismissed") {
            self.linger = defaults.linger;
        }
        self.cue_volume = if self.cue_volume.is_finite() { self.cue_volume.clamp(0.0, 1.0) } else { defaults.cue_volume };
        self
    }
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
    /// Missing on sessions saved before Aloe Code listed them per conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
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

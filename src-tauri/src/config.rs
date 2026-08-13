use chrono::Utc;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs, path::PathBuf, sync::Mutex};
use tokio::sync::mpsc::UnboundedSender;
use tokio_tungstenite::tungstenite::Message;

use serde_json::Value;

use crate::models::{AgentConfig, GrantedFolder, PendingApproval, RecentAction};
use crate::terminal::TerminalSession;

// ── Tunable constants ────────────────────────────────────────────────────────
// Every limit and timeout the agent enforces lives here, including the ones that used to sit in
// browser.rs, fs.rs and terminal.rs. They are compile-time constants rather than settings: this is
// a signed desktop binary, so a "configurable" limit that a local file could raise would be a way
// around the sandbox the granted-folder model depends on.

/// Backend the agent talks to in a release build. `ALOE_BACKEND_URL` overrides it at compile time;
/// debug builds default to a local backend instead. See `default_api_url`.
pub const FALLBACK_PROD_API_URL: &str = "https://api.247autoarmy.in/";
/// Backend used by debug builds.
pub const FALLBACK_DEBUG_API_URL: &str = "http://127.0.0.1:8080";

// ── Search and file reads ────────────────────────────────────────────────────
pub const MAX_SEARCH_RESULTS: usize = 80;
pub const MAX_TEXT_BYTES: usize = 256_000;
/// Largest file the agent will read into an attachment (~15 MB raw).
pub const MAX_ATTACH_BYTES: u64 = 15_000_000;

// ── Command execution ────────────────────────────────────────────────────────
pub const COMMAND_TIMEOUT_SECONDS: u64 = 60;
/// Scrollback retained per terminal session.
pub const MAX_TERMINAL_BUFFER_BYTES: usize = 128_000;

// ── Backend socket ───────────────────────────────────────────────────────────
pub const SOCKET_HEARTBEAT_MS: u64 = 5_000;
pub const SOCKET_RECONNECT_MAX_MS: u64 = 15_000;

// ── Browser automation ───────────────────────────────────────────────────────
/// Port the agent starts Chrome's remote-debugging listener on.
pub const BROWSER_DEBUG_PORT: u16 = 9333;
pub const BROWSER_LAUNCH_TIMEOUT_SECONDS: u64 = 20;
pub const BROWSER_CDP_TIMEOUT_SECONDS: u64 = 30;
pub const BROWSER_NAVIGATION_TIMEOUT_SECONDS: u64 = 30;
pub const MAX_PAGE_TEXT_CHARS: usize = 12_000;

// ── Persisted history ────────────────────────────────────────────────────────
/// Recent actions and terminal sessions retained in config.json.
pub const MAX_PERSISTED_HISTORY: usize = 50;

pub struct AppState {
    pub config: Mutex<AgentConfig>,
    pub pending: Mutex<Vec<PendingApproval>>,
    pub terminals: Mutex<HashMap<String, TerminalSession>>,
    pub client: Client,
    /// Set while the socket is connected; lets code outside socket.rs (e.g.
    /// queue_for_approval) push a message onto the live connection. Best-effort: a
    /// message sent while this is `None` (or the send fails) is simply not synced yet.
    pub outbound: Mutex<Option<UnboundedSender<Message>>>,
}

pub fn normalize_api_url(raw: &str) -> String {
    raw.trim()
        .trim_end_matches('/')
        .strip_suffix("/api")
        .unwrap_or_else(|| raw.trim().trim_end_matches('/'))
        .to_string()
}

pub fn default_api_url() -> String {
    let fallback = if cfg!(debug_assertions) {
        FALLBACK_DEBUG_API_URL
    } else {
        FALLBACK_PROD_API_URL
    };
    normalize_api_url(option_env!("ALOE_BACKEND_URL").unwrap_or(fallback))
}

pub fn normalize_setup_token(token: &str) -> String {
    token.chars().filter(|c| !c.is_whitespace()).collect()
}

pub fn make_default_config() -> AgentConfig {
    AgentConfig {
        api_url: default_api_url(),
        device_name: std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "Aloe Desktop".to_string()),
        platform: std::env::consts::OS.to_string(),
        socket_status: "disconnected".to_string(),
        command_trust_mode: "ask".to_string(),
        ..Default::default()
    }
}

pub fn config_path() -> Result<PathBuf, String> {
    let dir = dirs::config_dir()
        .ok_or_else(|| "Could not find OS config directory.".to_string())?
        .join("Aloe Desktop");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("config.json"))
}

pub fn load_config() -> AgentConfig {
    let Ok(path) = config_path() else {
        return make_default_config();
    };
    let mut config: AgentConfig = fs::read_to_string(path)
        .ok()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_else(make_default_config);
    config.api_url = default_api_url();
    if config.socket_status.is_empty() {
        config.socket_status = "disconnected".to_string();
    }
    if config.device_name.is_empty() {
        config.device_name = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "Aloe Desktop".to_string());
    }
    if config.platform.is_empty() {
        config.platform = std::env::consts::OS.to_string();
    }
    if config.command_trust_mode.is_empty() {
        config.command_trust_mode = if config.always_allow_commands {
            "all"
        } else {
            "ask"
        }
        .to_string();
    } else if config.command_trust_mode == "trusted_coding" {
        // Renamed once the static allowlist was replaced by an LLM safety check with conversation
        // context — an existing config.json still on the old value reads as "auto" from here on.
        config.command_trust_mode = "auto".to_string();
    } else if !matches!(config.command_trust_mode.as_str(), "ask" | "auto" | "all") {
        config.command_trust_mode = "ask".to_string();
    }
    for session in &mut config.terminal_sessions {
        if session.status == "running" {
            session.status = "interrupted".to_string();
        }
    }
    config.terminal_sessions.truncate(MAX_PERSISTED_HISTORY);
    config
}

pub fn save_config(config: &AgentConfig) -> Result<(), String> {
    let path = config_path()?;
    let content = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| e.to_string())
}

fn compact_value(v: Value) -> Value {
    const MAX_STR: usize = 500;
    const MAX_ARR: usize = 30;
    match v {
        Value::String(s) if s.len() > MAX_STR => {
            Value::String(format!("{}…(+{} chars)", &s[..MAX_STR], s.len() - MAX_STR))
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, compact_value(v)))
                .collect(),
        ),
        Value::Array(arr) => {
            let total = arr.len();
            let mut items: Vec<Value> = arr.into_iter().take(MAX_ARR).map(compact_value).collect();
            if total > MAX_ARR {
                items.push(Value::String(format!("…and {} more", total - MAX_ARR)));
            }
            Value::Array(items)
        }
        other => other,
    }
}

pub fn add_recent(
    config: &mut AgentConfig,
    job_id: &str,
    kind: &str,
    status: &str,
    detail: &str,
    input: Option<Value>,
    output: Option<Value>,
) {
    config.recent_actions.insert(
        0,
        RecentAction {
            job_id: job_id.to_string(),
            kind: kind.to_string(),
            status: status.to_string(),
            detail: detail.chars().take(180).collect(),
            timestamp: Utc::now().to_rfc3339(),
            input: input.map(compact_value),
            output: output.map(compact_value),
        },
    );
    config.recent_actions.truncate(MAX_PERSISTED_HISTORY);
}

pub fn make_granted_folder(canonical: &std::path::Path) -> GrantedFolder {
    let context = crate::fs::scan_folder_context(canonical);
    let context_updated_at = context.as_ref().map(|_| Utc::now().to_rfc3339());
    GrantedFolder {
        label: canonical
            .file_name()
            .map(|n| n.to_string_lossy().to_string()),
        path: canonical.to_string_lossy().to_string(),
        indexed_at: Some(Utc::now().to_rfc3339()),
        context,
        context_updated_at,
    }
}

/// Re-reads every granted folder's MEMORY.md/AGENTS.md and refreshes `context` in place. Called
/// at startup (so edits made while the app was closed are picked up) and after Aloe itself writes
/// to one of those files mid-conversation (see executor.rs's post-write resync).
pub fn rescan_folder_contexts(config: &mut AgentConfig) {
    for folder in &mut config.folders {
        let context = crate::fs::scan_folder_context(std::path::Path::new(&folder.path));
        folder.context_updated_at = context.as_ref().map(|_| Utc::now().to_rfc3339());
        folder.context = context;
    }
}

pub fn debug_log(scope: &str, event: &str, detail: impl AsRef<str>) {
    println!("[DEBUG][aloe_desktop][{scope}] {event} {}", detail.as_ref());
}

pub fn secret_fingerprint(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hasher
        .finalize()
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect()
}

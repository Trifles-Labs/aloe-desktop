mod browser;
mod config;
mod desktop;
mod executor;
mod fs;
mod models;
mod notifications;
mod search;
mod shell;
mod socket;
mod terminal;

use serde_json::json;
use std::{collections::HashMap, fs as std_fs, sync::Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_dialog::DialogExt;
#[cfg(target_os = "linux")]
use tauri_plugin_dialog::MessageDialogKind;
use tauri_plugin_opener::OpenerExt;
use tokio_tungstenite::tungstenite::Message;

use config::{
    debug_log, load_config, make_granted_folder, normalize_setup_token, rescan_folder_contexts,
    save_config, secret_fingerprint, AppState,
};
use models::{AgentConfig, ErrorResponse, PendingApproval, RegisterResponse, SearchMatch};
use search::{search_content as search_content_fn, search_files as search_files_fn};
use socket::{clear_agent_credentials, sync_folders_with_config};

#[cfg(target_os = "linux")]
const SOFTWARE_RENDERING_ALERT_ENV: &str = "ALOE_SOFTWARE_RENDERING_ALERT";

// ── Read-only commands ────────────────────────────────────────────────────────

#[tauri::command]
fn get_config(state: State<AppState>) -> AgentConfig {
    state.config.lock().expect("config mutex").clone()
}

#[tauri::command]
fn get_pending_approvals(state: State<AppState>) -> Vec<PendingApproval> {
    state.pending.lock().expect("pending mutex").clone()
}

#[tauri::command]
fn hide_main_window(app: AppHandle) {
    desktop::hide_main_window(&app);
}

#[tauri::command]
fn open_external_url(app: AppHandle, url: String) -> Result<(), String> {
    app.opener()
        .open_url(url, None::<String>)
        .map_err(|error| error.to_string())
}

// ── Google sign-in deep link ─────────────────────────────────────────────────

/// Custom protocol the browser returns the Google OAuth token through
/// (`aloe://auth/callback?token=…`, minted by the web app's sign-in callback page).
const OAUTH_DEEP_LINK_SCHEME: &str = "aloe";
/// Event the frontend listens for when a deep link arrives while the app is running.
const OAUTH_TOKEN_EVENT: &str = "aloe-google-auth";

fn oauth_token_from_url(url: &url::Url) -> Option<String> {
    if url.scheme() != OAUTH_DEEP_LINK_SCHEME
        || url.host_str() != Some("auth")
        || url.path() != "/callback"
    {
        return None;
    }
    url.query_pairs()
        .find(|(key, _)| key == "token")
        .map(|(_, value)| value.into_owned())
        .filter(|token| !token.is_empty())
}

/// Records the signed-in token so the frontend can pick it up, and nudges the frontend if it is
/// already listening. The deep link only ever carries a fresh token, so the latest one wins.
fn store_oauth_token(app: &AppHandle, token: String) {
    {
        let state = app.state::<AppState>();
        let mut pending = state.pending_oauth.lock().expect("pending oauth mutex");
        *pending = Some(token.clone());
    }
    let _ = app.emit(OAUTH_TOKEN_EVENT, token);
    desktop::show_main_window(app);
}

/// Drains the pending Google OAuth token, if a deep link arrived before the frontend was ready.
#[tauri::command]
fn take_pending_oauth_token(state: State<AppState>) -> Option<String> {
    state
        .pending_oauth
        .lock()
        .expect("pending oauth mutex")
        .take()
}

#[tauri::command]
fn set_run_on_startup(
    app: AppHandle,
    state: State<AppState>,
    enabled: bool,
) -> Result<AgentConfig, String> {
    if enabled {
        app.autolaunch()
            .enable()
            .map_err(|error| error.to_string())?;
    } else {
        app.autolaunch()
            .disable()
            .map_err(|error| error.to_string())?;
    }
    let next = {
        let mut config = state.config.lock().expect("config mutex");
        config.run_on_startup = enabled;
        save_config(&config)?;
        config.clone()
    };
    Ok(next)
}

#[tauri::command]
fn set_start_minimized(state: State<AppState>, enabled: bool) -> Result<AgentConfig, String> {
    let mut config = state.config.lock().expect("config mutex");
    config.start_minimized = enabled;
    save_config(&config)?;
    Ok(config.clone())
}

// ── Connection management ─────────────────────────────────────────────────────

#[tauri::command]
fn reset_agent_connection(app: AppHandle, state: State<AppState>) -> Result<AgentConfig, String> {
    let mut config = state.config.lock().expect("config mutex");
    clear_agent_credentials(
        &mut config,
        "Connection reset. Paste a fresh setup token from Aloe Integrations.",
    );
    save_config(&config)?;
    let result = config.clone();
    drop(config);
    desktop::refresh_tray_menu(&app);
    Ok(result)
}

#[tauri::command]
async fn register_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
) -> Result<AgentConfig, String> {
    let config = state.config.lock().expect("config mutex").clone();
    let setup_token = normalize_setup_token(&token);
    if setup_token.is_empty() {
        return Err("Setup token is required.".to_string());
    }
    debug_log(
        "register",
        "start",
        format!(
            "api_url={} setupTokenFp={}",
            config.api_url,
            secret_fingerprint(&setup_token)
        ),
    );

    let response = state
        .client
        .post(format!("{}/api/agent/register", config.api_url))
        .json(&json!({
            "token": setup_token,
            "deviceName": config.device_name,
            "platform": config.platform,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<ErrorResponse>(&body)
            .ok()
            .and_then(|p| p.error)
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| {
                if body.trim().is_empty() {
                    format!("Registration failed with status {status}.")
                } else {
                    body
                }
            });
        return Err(format!(
            "Registration failed against {}: {message}",
            config.api_url
        ));
    }

    let registered = response
        .json::<RegisterResponse>()
        .await
        .map_err(|e| e.to_string())?;
    debug_log(
        "register",
        "created",
        format!("agent_id={}", registered.agent_id),
    );

    let verify = state
        .client
        .post(format!("{}/api/agent/heartbeat", config.api_url))
        .bearer_auth(&registered.credential)
        .send()
        .await
        .map_err(|e| format!("Registered, but credential verification failed: {e}"))?;

    if !verify.status().is_success() {
        let status = verify.status();
        let body = verify.text().await.unwrap_or_default();
        let message = serde_json::from_str::<ErrorResponse>(&body)
            .ok()
            .and_then(|p| p.error)
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| {
                format!(
                    "Registered, but backend rejected the new credential: {}",
                    if body.is_empty() {
                        status.to_string()
                    } else {
                        body
                    }
                )
            });
        return Err(format!(
            "Credential verification failed against {}: {message}",
            config.api_url
        ));
    }

    let mut next = state.config.lock().expect("config mutex");
    next.agent_id = Some(registered.agent_id);
    next.user_token = Some(
        registered
            .user_token
            .unwrap_or_else(|| registered.credential.clone()),
    );
    next.user_profile = registered.user;
    next.credential = Some(registered.credential);
    next.socket_status = "reconnecting".to_string();
    next.socket_error = None;
    save_config(&next)?;
    let result = next.clone();
    drop(next);
    desktop::refresh_tray_menu(&app);
    Ok(result)
}

// ── Settings ──────────────────────────────────────────────────────────────────

#[tauri::command]
fn set_command_trust_mode(state: State<AppState>, mode: String) -> Result<AgentConfig, String> {
    if mode != "ask" && mode != "auto" && mode != "all" {
        return Err("Unsupported command trust mode.".to_string());
    }
    let mut config = state.config.lock().expect("config mutex");
    config.command_trust_mode = mode.clone();
    config.always_allow_commands = config.command_trust_mode == "all";
    save_config(&config)?;
    let result = config.clone();
    drop(config);

    // Best-effort live push so the backend's tool-card status text (commandStatusLabel in
    // local_agent.ts) reflects the change immediately instead of waiting for the next reconnect's
    // hello. A dropped send here just means that text lags one change behind — see
    // queue_for_approval in executor.rs for the same tradeoff on the same channel.
    if let Some(sender) = state.outbound.lock().expect("outbound mutex").as_ref() {
        let _ = sender.send(Message::Text(json!({ "type": "trust_mode_updated", "mode": mode }).to_string().into()));
    }

    Ok(result)
}

// ── Folder management ─────────────────────────────────────────────────────────

#[tauri::command]
async fn sync_folders(state: State<'_, AppState>) -> Result<(), String> {
    let config = state.config.lock().expect("config mutex").clone();
    sync_folders_with_config(&state.client, &config).await
}

#[tauri::command]
async fn add_folder(app: AppHandle, state: State<'_, AppState>) -> Result<AgentConfig, String> {
    let picked = app
        .dialog()
        .file()
        .blocking_pick_folder()
        .ok_or("No folder selected.")?;
    let canonical = std_fs::canonicalize(picked.to_string()).map_err(|e| e.to_string())?;
    debug_log("folders", "add", format!("path={}", canonical.display()));
    {
        let mut config = state.config.lock().expect("config mutex");
        let path_str = canonical.to_string_lossy().to_string();
        if !config.folders.iter().any(|f| f.path == path_str) {
            config.folders.push(make_granted_folder(&canonical));
        }
        save_config(&config)?;
    }
    sync_folders(state).await?;
    Ok(app
        .state::<AppState>()
        .config
        .lock()
        .expect("config mutex")
        .clone())
}

#[tauri::command]
async fn remove_folder(state: State<'_, AppState>, path: String) -> Result<AgentConfig, String> {
    debug_log("folders", "remove", format!("path={path}"));
    {
        let mut config = state.config.lock().expect("config mutex");
        config.folders.retain(|f| f.path != path);
        save_config(&config)?;
    }
    sync_folders(state).await?;
    Ok(load_config())
}

// ── Command approval ──────────────────────────────────────────────────────────

#[tauri::command]
async fn approve_command(app: AppHandle, job_id: String, approved: bool) -> Result<(), String> {
    executor::resolve_pending_approval(&app, &job_id, approved).await
}

// ── Search commands ───────────────────────────────────────────────────────────

#[tauri::command]
fn search_content(
    state: State<AppState>,
    path: String,
    pattern: String,
    case_sensitive: bool,
) -> Result<Vec<SearchMatch>, String> {
    let config = state.config.lock().expect("config mutex");
    search_content_fn(&config, &path, &pattern, case_sensitive)
}

#[tauri::command]
fn search_files(
    state: State<AppState>,
    path: String,
    pattern: String,
    case_sensitive: bool,
) -> Result<Vec<SearchMatch>, String> {
    let config = state.config.lock().expect("config mutex");
    search_files_fn(&config, &path, &pattern, case_sensitive)
}

// ── App entry point ───────────────────────────────────────────────────────────

pub fn run() {
    let mut initial_config = load_config();
    // Picks up edits made to a granted folder's MEMORY.md/AGENTS.md while the app was closed,
    // so the very first sync after launch reflects the file's real current content.
    rescan_folder_contexts(&mut initial_config);
    debug_log(
        "app",
        "startup",
        format!(
            "api_url={} agent_id={} folders={} socket_status={}",
            initial_config.api_url,
            initial_config.agent_id.as_deref().unwrap_or("none"),
            initial_config.folders.len(),
            initial_config.socket_status,
        ),
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // On Windows/Linux a deep link launches a second instance; single-instance forwards
            // the URL argument here. `store_oauth_token` also shows the window, so a sign-in
            // deep link both lands and brings the app forward.
            //
            // argv[0] is the exe's own path, and on Windows a leading drive letter (`C:\...`)
            // parses as a valid URL with scheme "c" — so this must keep scanning past the first
            // arg that merely parses as *some* URL and find the one that's actually the deep
            // link, rather than stopping at argv[0] and silently dropping the real token.
            match argv
                .iter()
                .filter_map(|arg| url::Url::parse(arg).ok())
                .find_map(|url| oauth_token_from_url(&url))
            {
                Some(token) => store_oauth_token(app, token),
                None => desktop::show_main_window(app),
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState {
            config: Mutex::new(initial_config),
            pending: Mutex::new(Vec::new()),
            pending_oauth: Mutex::new(None),
            terminals: Mutex::new(HashMap::new()),
            client: reqwest::Client::new(),
            outbound: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            get_pending_approvals,
            take_pending_oauth_token,
            hide_main_window,
            open_external_url,
            set_run_on_startup,
            set_start_minimized,
            reset_agent_connection,
            register_agent,
            set_command_trust_mode,
            sync_folders,
            add_folder,
            remove_folder,
            approve_command,
            search_content,
            search_files,
        ])
        .setup(|app| {
            desktop::install_tray(app)?;

            // Dev builds and portable bundles are not registered by an installer, so claim the
            // `aloe://` scheme from the app itself. Best-effort — an installed app that already
            // registered it is left untouched (registry writes are idempotent).
            #[cfg(any(windows, target_os = "linux"))]
            let _ = app.deep_link().register_all();

            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    if let Some(token) = oauth_token_from_url(&url) {
                        store_oauth_token(&handle, token);
                    }
                }
            });

            // Cold start: the app was launched BY a deep link, so no listener existed when the
            // plugin parsed the argument — the URL is only available through `get_current()`.
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                for url in urls {
                    if let Some(token) = oauth_token_from_url(&url) {
                        store_oauth_token(app.handle(), token);
                    }
                }
            }

            let launched_by_autostart = std::env::args().any(|arg| arg == "--autostart");
            let preferences = app
                .state::<AppState>()
                .config
                .lock()
                .expect("config mutex")
                .clone();
            if preferences.run_on_startup {
                let _ = app.autolaunch().enable();
            } else {
                let _ = app.autolaunch().disable();
            }
            #[cfg(target_os = "linux")]
            if std::env::var_os(SOFTWARE_RENDERING_ALERT_ENV).is_some() {
                app.dialog()
                    .message(
                        "Aloe detected Wayland and is starting with software rendering to avoid known WebKitGTK graphics issues. Other display servers continue to use hardware acceleration.",
                    )
                    .title("Software rendering enabled")
                    .kind(MessageDialogKind::Warning)
                    .show(|_| {});
            }
            if !(launched_by_autostart && preferences.start_minimized) {
                desktop::show_main_window(app.handle());
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                socket::socket_loop(handle).await;
            });
            Ok(())
        })
        .on_menu_event(|app, event| desktop::handle_menu_event(app, event.id().as_ref()))
        .on_tray_icon_event(desktop::handle_tray_event)
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                desktop::hide_main_window(window.app_handle());
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Aloe Desktop");
}

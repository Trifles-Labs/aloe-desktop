mod config;
mod desktop;
mod executor;
mod fs;
mod models;
mod notifications;
mod search;
mod socket;
mod terminal;

use serde_json::json;
use std::{collections::HashMap, fs as std_fs, sync::Mutex};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_opener::OpenerExt;

use config::{
    add_recent, debug_log, load_config, make_granted_folder,
    normalize_setup_token, save_config, secret_fingerprint, AppState,
};
use executor::{dispatch_tool, post_result};
use models::{AgentConfig, AgentJob, ErrorResponse, PendingApproval, RegisterResponse, SearchMatch};
use search::{search_content as search_content_fn, search_files as search_files_fn};
use socket::{clear_agent_credentials, sync_folders_with_config};

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
    app.opener().open_url(url, None::<String>).map_err(|error| error.to_string())
}

#[tauri::command]
fn set_run_on_startup(app: AppHandle, state: State<AppState>, enabled: bool) -> Result<AgentConfig, String> {
    if enabled {
        app.autolaunch().enable().map_err(|error| error.to_string())?;
    } else {
        app.autolaunch().disable().map_err(|error| error.to_string())?;
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
async fn register_agent(app: AppHandle, state: State<'_, AppState>, token: String) -> Result<AgentConfig, String> {
    let config = state.config.lock().expect("config mutex").clone();
    let setup_token = normalize_setup_token(&token);
    if setup_token.is_empty() {
        return Err("Setup token is required.".to_string());
    }
    debug_log(
        "register", "start",
        format!("setupTokenFp={}", secret_fingerprint(&setup_token)),
    );

    let response = state.client
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
        return Err(message);
    }

    let registered = response.json::<RegisterResponse>().await.map_err(|e| e.to_string())?;
    debug_log("register", "created", format!("agent_id={}", registered.agent_id));

    let verify = state.client
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
                format!("Registered, but backend rejected the new credential: {}", if body.is_empty() { status.to_string() } else { body })
            });
        return Err(message);
    }

    let mut next = state.config.lock().expect("config mutex");
    next.agent_id = Some(registered.agent_id);
    next.user_token = Some(registered.user_token.unwrap_or_else(|| registered.credential.clone()));
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
fn set_always_allow_commands(state: State<AppState>, enabled: bool) -> Result<AgentConfig, String> {
    let mut config = state.config.lock().expect("config mutex");
    config.always_allow_commands = enabled;
    save_config(&config)?;
    Ok(config.clone())
}

// ── Folder management ─────────────────────────────────────────────────────────

#[tauri::command]
async fn sync_folders(state: State<'_, AppState>) -> Result<(), String> {
    let config = state.config.lock().expect("config mutex").clone();
    sync_folders_with_config(&state.client, &config).await
}

#[tauri::command]
async fn add_folder(app: AppHandle, state: State<'_, AppState>) -> Result<AgentConfig, String> {
    let picked = app.dialog().file().blocking_pick_folder()
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
    Ok(app.state::<AppState>().config.lock().expect("config mutex").clone())
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
async fn approve_command(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
    approved: bool,
) -> Result<(), String> {
    let pending = {
        let mut list = state.pending.lock().expect("pending mutex");
        let idx = list.iter().position(|i| i.job_id == job_id)
            .ok_or("Approval request not found.")?;
        list.remove(idx)
    };
    let config = state.config.lock().expect("config mutex").clone();

    if !approved {
        post_result(&state, &config, &pending.job_id, "denied", None,
            Some("User denied command.".to_string())).await;
        return Ok(());
    }

    let input_val = pending.input.clone();
    let job = AgentJob {
        id: pending.job_id.clone(),
        kind: pending.job_kind.clone(),
        input: input_val.clone(),
    };
    let result = dispatch_tool(&app, &state, &config, &job).await;
    let (status, output, error) = match result {
        Ok(v) => ("completed", Some(v), None),
        Err(m) => ("failed", None, Some(m)),
    };
    post_result(&state, &config, &pending.job_id, status, output.clone(), error.clone()).await;
    {
        let mut cfg = state.config.lock().expect("config mutex");
        add_recent(&mut cfg, &pending.job_id, &pending.job_kind, status, error.as_deref().unwrap_or("Completed"), Some(input_val), output);
        let _ = save_config(&cfg);
    }
    Ok(())
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
    let initial_config = load_config();
    debug_log(
        "app", "startup",
        format!(
            "api_url={} agent_id={} folders={} socket_status={}",
            initial_config.api_url,
            initial_config.agent_id.as_deref().unwrap_or("none"),
            initial_config.folders.len(),
            initial_config.socket_status,
        ),
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            desktop::show_main_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--autostart"])))
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            config: Mutex::new(initial_config),
            pending: Mutex::new(Vec::new()),
            terminals: Mutex::new(HashMap::new()),
            client: reqwest::Client::new(),
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            get_pending_approvals,
            hide_main_window,
            open_external_url,
            set_run_on_startup,
            set_start_minimized,
            reset_agent_connection,
            register_agent,
            set_always_allow_commands,
            sync_folders,
            add_folder,
            remove_folder,
            approve_command,
            search_content,
            search_files,
        ])
        .setup(|app| {
            desktop::install_tray(app)?;
            let launched_by_autostart = std::env::args().any(|arg| arg == "--autostart");
            let preferences = app.state::<AppState>().config.lock().expect("config mutex").clone();
            if preferences.run_on_startup {
                let _ = app.autolaunch().enable();
            } else {
                let _ = app.autolaunch().disable();
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

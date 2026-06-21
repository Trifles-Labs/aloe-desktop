use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tokio::time::{sleep, Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
};

use crate::config::{
    debug_log, normalize_api_url, save_config, secret_fingerprint, AppState,
    SOCKET_HEARTBEAT_MS, SOCKET_RECONNECT_MAX_MS,
};
use crate::executor::execute_job;
use crate::models::{AgentConfig, JobList, SocketJobMessage};

// ── Socket state helpers ──────────────────────────────────────────────────────

pub fn set_socket_state(app: &AppHandle, status: &str, error: Option<String>) {
    debug_log("socket", "state", format!("status={status}"));
    let state = app.state::<AppState>();
    {
        let mut config = state.config.lock().expect("config mutex");
        config.socket_status = status.to_string();
        config.socket_error = error;
        let _ = save_config(&config);
    }
    crate::desktop::refresh_tray_menu(app);
    let _ = app.emit("agent://socket-status", ());
}

pub fn clear_agent_credentials(config: &mut AgentConfig, reason: &str) {
    debug_log("config", "clear_credentials", format!("reason={reason}"));
    config.agent_id = None;
    config.credential = None;
    config.user_token = None;
    config.user_profile = None;
    config.socket_status = "disconnected".to_string();
    config.socket_error = Some(reason.to_string());
}

// ── Folder sync ───────────────────────────────────────────────────────────────

pub async fn sync_folders_with_config(client: &Client, config: &AgentConfig) -> Result<(), String> {
    let credential = config.credential.clone().ok_or("Agent is not registered.")?;
    debug_log("folders", "sync_start", format!("count={}", config.folders.len()));
    let response = client
        .put(format!("{}/api/agent/folders", config.api_url))
        .bearer_auth(credential)
        .json(&json!({ "folders": config.folders }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        debug_log("folders", "sync_failed", format!("status={status}"));
        Err(body)
    }
}

// ── WebSocket loop ────────────────────────────────────────────────────────────

pub async fn socket_loop(app: AppHandle) {
    let mut reconnect_ms = 1_000u64;
    debug_log("socket", "loop_started", "");

    loop {
        let state = app.state::<AppState>();
        let config = state.config.lock().expect("config mutex").clone();

        let Some(credential) = config.credential.clone() else {
            set_socket_state(&app, "disconnected", Some("Agent is not registered yet.".to_string()));
            sleep(Duration::from_millis(reconnect_ms)).await;
            continue;
        };

        let fp = secret_fingerprint(&credential);
        let ws_url = websocket_url(&config.api_url);
        debug_log("socket", "connect_attempt", format!("fp={fp} delay_ms={reconnect_ms}"));

        let mut request = match ws_url.into_client_request() {
            Ok(r) => r,
            Err(e) => {
                set_socket_state(&app, "reconnecting", Some(format!("WebSocket request failed: {e}")));
                reconnect_ms = backoff(reconnect_ms);
                sleep(Duration::from_millis(reconnect_ms)).await;
                continue;
            }
        };

        match HeaderValue::from_str(&format!("Bearer {credential}")) {
            Ok(v) => { request.headers_mut().insert("Authorization", v); }
            Err(e) => {
                set_socket_state(&app, "reconnecting", Some(format!("Authorization header failed: {e}")));
                reconnect_ms = backoff(reconnect_ms);
                sleep(Duration::from_millis(reconnect_ms)).await;
                continue;
            }
        }

        match connect_async(request).await {
            Ok((socket, _)) => {
                debug_log("socket", "connected", format!("fp={fp}"));
                set_socket_state(&app, "connected", None);
                if let Err(e) = sync_folders_with_config(&state.client, &config).await {
                    debug_log("folders", "sync_error", e);
                }
                // Drain any jobs (e.g. notifications) that were queued while the desktop was offline
                drain_queued_jobs(&app, &state, &config, &credential, &fp).await;
                reconnect_ms = 1_000;
                run_connected_loop(&app, socket, &credential, &fp).await;
            }
            Err(err) => {
                let err_str = err.to_string();
                debug_log("socket", "connect_error", format!("fp={fp} err={err_str}"));
                set_socket_state(&app, "reconnecting", Some(err_str));
                handle_offline_fallback(&app, &state, &config, &credential, &fp).await;
            }
        }

        sleep(Duration::from_millis(reconnect_ms)).await;
        reconnect_ms = backoff(reconnect_ms);
    }
}

fn websocket_url(api_url: &str) -> String {
    let base = normalize_api_url(api_url);
    let ws_base = base
        .strip_prefix("https://").map(|r| format!("wss://{r}"))
        .or_else(|| base.strip_prefix("http://").map(|r| format!("ws://{r}")))
        .unwrap_or(base);
    format!("{ws_base}/api/agent/socket")
}

fn backoff(current_ms: u64) -> u64 {
    (current_ms * 2).min(SOCKET_RECONNECT_MAX_MS)
}

async fn run_connected_loop(
    app: &AppHandle,
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    credential: &str,
    fp: &str,
) {
    let (mut write, mut read) = socket.split();
    let _ = write.send(Message::Text(json!({ "type": "hello" }).to_string().into())).await;
    let mut heartbeat = tokio::time::interval(Duration::from_millis(SOCKET_HEARTBEAT_MS));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                let current = app.state::<AppState>().config.lock()
                    .expect("config mutex").credential.clone();
                if current.as_deref() != Some(credential) {
                    debug_log("socket", "closing_after_logout", format!("fp={fp}"));
                    let _ = write.close().await;
                    set_socket_state(app, "disconnected", Some("Logged out.".to_string()));
                    return;
                }
                if write.send(Message::Text(json!({ "type": "heartbeat" }).to_string().into())).await.is_err() {
                    set_socket_state(app, "reconnecting", Some("Heartbeat failed.".to_string()));
                    return;
                }
            }
            next = read.next() => {
                let Some(Ok(message)) = next else {
                    set_socket_state(app, "reconnecting", Some("WebSocket closed.".to_string()));
                    return;
                };
                if !message.is_text() {
                    continue;
                }
                let Ok(payload) = serde_json::from_str::<SocketJobMessage>(
                    message.to_text().unwrap_or_default()
                ) else {
                    continue;
                };
                if payload.kind != "job_request" {
                    continue;
                }
                if let Some(job) = payload.job {
                    debug_log("socket", "job_request", format!("job_id={} kind={}", job.id, job.kind));
                    let handle = app.clone();
                    tokio::spawn(async move { execute_job(handle, job).await; });
                }
            }
        }
    }
}

async fn drain_queued_jobs(
    app: &AppHandle,
    state: &tauri::State<'_, AppState>,
    config: &AgentConfig,
    credential: &str,
    fp: &str,
) {
    if let Ok(resp) = state.client
        .get(format!("{}/api/agent/jobs", config.api_url))
        .bearer_auth(credential)
        .send()
        .await
    {
        if let Ok(list) = resp.json::<JobList>().await {
            if !list.jobs.is_empty() {
                debug_log("socket", "drain_queued", format!("fp={fp} count={}", list.jobs.len()));
            }
            for job in list.jobs {
                let handle = app.clone();
                tokio::spawn(async move { execute_job(handle, job).await; });
            }
        }
    }
}

async fn handle_offline_fallback(
    app: &AppHandle,
    state: &tauri::State<'_, AppState>,
    config: &AgentConfig,
    credential: &str,
    fp: &str,
) {
    let heartbeat = state.client
        .post(format!("{}/api/agent/heartbeat", config.api_url))
        .bearer_auth(credential)
        .send()
        .await;

    match &heartbeat {
        Ok(r) => debug_log("http_fallback", "heartbeat", format!("fp={fp} status={}", r.status())),
        Err(e) => debug_log("http_fallback", "heartbeat_error", format!("fp={fp} err={e}")),
    }

    if heartbeat.as_ref().is_ok_and(|r| r.status().as_u16() == 401) {
        let mut cfg = state.config.lock().expect("config mutex");
        if cfg.credential.as_deref() == Some(credential) {
            clear_agent_credentials(&mut cfg, "Credential was rejected. Paste a new setup token.");
            let _ = save_config(&cfg);
        }
        drop(cfg);
        let _ = app.emit("agent://socket-status", ());
        return;
    }

    drain_queued_jobs(app, state, config, credential, fp).await;
}

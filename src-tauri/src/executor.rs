use chrono::Utc;
use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tauri::{AppHandle, Emitter, Manager};
use tokio::process::Command;
use tokio_tungstenite::tungstenite::Message;

use crate::browser::dispatch_browser;
use crate::config::{add_recent, debug_log, rescan_folder_contexts, save_config, scoped_config, AppState, COMMAND_TIMEOUT_SECONDS};
use crate::fs::{
    apply_patch, assert_granted, attach_file, create_file, create_folder, delete_file,
    delete_folder, input_string, list_files, read_file, truncate_text, update_file, update_folder,
    write_binary_file,
};
use crate::models::{AgentConfig, AgentJob, PendingApproval};
use crate::notifications;
use crate::search::search_codebase;
use crate::shell::{build_shell_command, hide_command_window};
use crate::socket::sync_folders_with_config;
use crate::terminal::{
    list_terminal_sessions, read_terminal_session, start_terminal_session, stop_terminal_session, wait_terminal_session,
    write_terminal_session,
};

/// Job kinds that can change a granted folder's MEMORY.md/AGENTS.md. A completed job of one of
/// these kinds triggers a background re-scan + re-sync so "remember X for this project" is
/// reflected on the very next turn instead of waiting for the app to restart.
const FOLDER_CONTEXT_TRIGGER_KINDS: [&str; 5] =
    ["create_file", "update_file", "write_local_file", "apply_local_patch", "delete_file"];

/// Fire-and-forget: re-scans every granted folder's context and pushes the refresh to the
/// backend. Best-effort, same as the other outbound syncs in this file — a failure here just
/// means the next natural sync (folder add, app restart) picks up the change instead.
fn maybe_resync_folder_context(app: &AppHandle, job_kind: &str, status: &str) {
    if status != "completed" || !FOLDER_CONTEXT_TRIGGER_KINDS.contains(&job_kind) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let config_snapshot = {
            let mut config = state.config.lock().expect("config mutex");
            rescan_folder_contexts(&mut config);
            let _ = save_config(&config);
            config.clone()
        };
        if let Err(e) = sync_folders_with_config(&state.client, &config_snapshot).await {
            debug_log("folders", "context_resync_error", e);
        }
    });
}

// ── LLM safety check (Auto mode) ─────────────────────────────────────────────
// "Auto" asks Aloe's own model — via the backend, which holds the API key and the conversation
// this command came from — whether the command is safe to run unattended, instead of matching it
// against a fixed allowlist. The backend re-derives the command from the job it already dispatched
// rather than trusting whatever this call sends, so a compromised desktop client can't talk the
// judge into approving a different command than the one that will actually run.
//
// This check is the ONLY gate in auto mode. There is deliberately no local hard veto in front of
// it any more: the previous token/compound blocklist rejected pipes, `&&`, and any command merely
// containing "curl" or "deploy", which is most real development work, so auto mode behaved as ask
// mode for everything worth automating. Everything now rides on `checkCommandSafety` in the
// backend, which fails closed on error, timeout, or an unparseable answer.

/// Generous because the judge may be a reasoning model: deepseek-reasoner answers in 2-5s warm but
/// has been measured at 11s on a cold call, and a timeout here fails closed — i.e. shows the very
/// approval prompt auto mode exists to avoid. Better to wait than to ask spuriously.
const SAFETY_CHECK_TIMEOUT_SECONDS: u64 = 40;

#[derive(serde::Deserialize)]
struct SafetyCheckResponse {
    safe: bool,
    reason: String,
    /// True when the backend's check errored out instead of reaching a verdict. It arrives as
    /// `safe: false` either way, but a check that judged nothing must not refuse the command —
    /// see the match in `execute_job`. Defaults to false so an older backend, which never sends
    /// this field, reads as a genuine verdict rather than a silent failure.
    #[serde(default)]
    failed: bool,
}

async fn check_command_safety(state: &tauri::State<'_, AppState>, config: &AgentConfig, job_id: &str) -> Result<SafetyCheckResponse, String> {
    let credential = config.credential.as_ref().ok_or("No credential configured.")?;
    let response = tokio::time::timeout(
        Duration::from_secs(SAFETY_CHECK_TIMEOUT_SECONDS),
        state.client
            .post(format!("{}/api/agent/jobs/{job_id}/check-command", config.api_url))
            .bearer_auth(credential)
            .send(),
    )
    .await
    .map_err(|_| "Safety check timed out.".to_string())?
    .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(format!("Safety check returned HTTP {}.", response.status()));
    }
    response.json::<SafetyCheckResponse>().await.map_err(|e| e.to_string())
}

// ── Shell command execution ───────────────────────────────────────────────────

pub async fn run_command(config: AgentConfig, input: Value) -> Result<Value, String> {
    let cwd = assert_granted(&config, &input_string(&input, "cwd")?)?;
    let command = input_string(&input, "command")?;

    let mut process = build_shell_command(&command);
    process.current_dir(&cwd).stdout(Stdio::piped()).stderr(Stdio::piped());
    hide_command_window(&mut process);

    let output = tokio::time::timeout(Duration::from_secs(COMMAND_TIMEOUT_SECONDS), process.output()).await
    .map_err(|_| "Command timed out.".to_string())?
    .map_err(|e| e.to_string())?;

    Ok(json!({
        "cwd": cwd.to_string_lossy(),
        "command": command,
        "exitCode": output.status.code(),
        "stdout": truncate_text(String::from_utf8_lossy(&output.stdout).to_string()),
        "stderr": truncate_text(String::from_utf8_lossy(&output.stderr).to_string()),
    }))
}

// ── Result reporting ──────────────────────────────────────────────────────────

pub async fn post_result(
    state: &AppState,
    config: &AgentConfig,
    job_id: &str,
    status: &str,
    result: Option<Value>,
    error: Option<String>,
) {
    let Some(credential) = &config.credential else {
        debug_log("job", "post_result_skipped", format!("job_id={job_id} reason=no_credential"));
        return;
    };
    let resp = state.client
        .post(format!("{}/api/agent/jobs/{job_id}/result", config.api_url))
        .bearer_auth(credential)
        .json(&json!({ "status": status, "result": result, "error": error }))
        .send()
        .await;
    match resp {
        Ok(r) => debug_log("job", "post_result", format!("job_id={job_id} status={status} http={}", r.status())),
        Err(e) => debug_log("job", "post_result_error", format!("job_id={job_id} status={status} err={e}")),
    }
}

// ── Job dispatch ──────────────────────────────────────────────────────────────

pub async fn execute_job(app: AppHandle, job: AgentJob) {
    debug_log("job", "received", format!("job_id={} kind={}", job.id, job.kind));
    let state = app.state::<AppState>();
    // Folder scope for this job: device grants, plus anything the user shared with the conversation
    // this job came from. Resolved once here so every path check below — and every tool dispatched
    // from it — sees the same set. Never saved; see scoped_config in config.rs.
    let config = {
        let stored = state.config.lock().expect("config mutex");
        scoped_config(&stored, job.conversation_id.as_deref())
    };

    // Commands require explicit approval, unless "all" (run anything) or "auto" (ask Aloe's own
    // model to judge this specific command against the conversation it came from) says otherwise.
    if job.kind == "run_command" || job.kind == "run_local_command" || job.kind == "start_terminal_session" {
        if config.command_trust_mode == "all" {
            run_approved_command(&app, &state, &config, job).await;
            return;
        }

        if config.command_trust_mode == "auto" {
            match check_command_safety(&state, &config, &job.id).await {
                Ok(decision) if decision.safe => {
                    run_approved_command(&app, &state, &config, job).await;
                    return;
                }
                // The check ran and said no: refuse outright and hand the reason to the model.
                Ok(decision) if !decision.failed => {
                    debug_log("job", "safety_check_declined", format!("job_id={} reason={}", job.id, decision.reason));
                    reject_command(&app, &state, &config, job, &decision.reason).await;
                    return;
                }
                // The check itself broke, so nothing has actually been judged. Fall through to
                // asking the user rather than refusing a command on no evidence.
                Ok(decision) => {
                    debug_log("job", "safety_check_unavailable", format!("job_id={} reason={}", job.id, decision.reason));
                }
                Err(e) => {
                    // Fail closed: a broken/offline safety check falls back to asking, never to
                    // running unattended.
                    debug_log("job", "safety_check_error", format!("job_id={} err={e}", job.id));
                }
            }
        }

        queue_for_approval(&state, &app, job);
        return;
    }

    let input_snapshot = job.input.clone();
    let result = dispatch_tool(&app, &state, &config, &job).await;
    let (status, output, error) = outcome(result);
    post_result(&state, &config, &job.id, status, output.clone(), error.clone()).await;
    debug_log("job", "completed", format!("job_id={} kind={} status={status}", job.id, job.kind));
    record_and_emit(&app, &state, &job.id, &job.kind, status, error.as_deref(), Some(input_snapshot), output);
    maybe_resync_folder_context(&app, &job.kind, status);
}

/// Runs a run_command/start_terminal_session job that's already been cleared to execute without
/// asking — either "all" mode, or "auto" mode after `check_command_safety` came back safe.
async fn run_approved_command(app: &AppHandle, state: &tauri::State<'_, AppState>, config: &AgentConfig, job: AgentJob) {
    let input_snapshot = job.input.clone();
    let result = if job.kind == "start_terminal_session" {
        start_terminal_session(state, config.clone(), job.input.clone()).await
    } else {
        run_command(config.clone(), job.input.clone()).await
    };
    let (status, output, error) = outcome(result);
    post_result(state, config, &job.id, status, output.clone(), error.clone()).await;
    record_and_emit(app, state, &job.id, &job.kind, status, error.as_deref(), Some(input_snapshot), output);
}

fn outcome(result: Result<Value, String>) -> (&'static str, Option<Value>, Option<String>) {
    match result {
        Ok(v) => ("completed", Some(v), None),
        Err(m) => ("failed", None, Some(m)),
    }
}

fn record_and_emit(
    app: &AppHandle,
    state: &tauri::State<AppState>,
    job_id: &str,
    kind: &str,
    status: &str,
    error: Option<&str>,
    input: Option<Value>,
    output: Option<Value>,
) {
    let mut config = state.config.lock().expect("config mutex");
    add_recent(&mut config, job_id, kind, status, error.unwrap_or("Completed"), input, output);
    let _ = save_config(&config);
    drop(config);
    let _ = app.emit("agent://recent-actions", ());
}

/// Refuses a command outright in "auto" mode and tells the model why.
///
/// The alternative — queueing it for manual approval — meant the judge's reasoning went only to a
/// UI the user often is not looking at, while the model saw nothing at all and was left waiting on
/// a card it could not explain. Returning the reason as the tool's error puts it in the one place
/// that can act on it: the model can say what happened, propose a narrower command, or ask the
/// user directly, all within the same turn.
///
/// The cost is that "auto" no longer offers a way to approve something the judge rejected. That is
/// what makes the wording of the judge's own prompt load-bearing — see `buildPrompt` in the
/// backend's command_safety.ts, which tells it a NOT-safe verdict stops the command.
async fn reject_command(
    app: &AppHandle,
    state: &tauri::State<'_, AppState>,
    config: &AgentConfig,
    job: AgentJob,
    reason: &str,
) {
    let input_snapshot = job.input.clone();
    let error = format!(
        "Aloe Desktop refused to run this command. Its safety check declined it: {reason} \
         Do not retry the same command or route it through another tool. Tell the user what was \
         blocked and why, and either propose a narrower command or ask them to run it themselves."
    );
    post_result(state, config, &job.id, "denied", None, Some(error.clone())).await;
    record_and_emit(app, state, &job.id, &job.kind, "denied", Some(&error), Some(input_snapshot), None);
}

/// Queues a command for manual approval — "ask" mode, and the fail-closed path when "auto"'s
/// safety check could not be reached at all. A check that ran and said no does not come here any
/// more; see `reject_command`.
fn queue_for_approval(state: &tauri::State<AppState>, app: &AppHandle, job: AgentJob) {
    let pending = PendingApproval {
        job_id: job.id.clone(),
        job_kind: job.kind.clone(),
        conversation_id: job.conversation_id.clone(),
        command: input_string(&job.input, "command").unwrap_or_default(),
        cwd: input_string(&job.input, "cwd").unwrap_or_default(),
        reason: input_string(&job.input, "reason")
            .unwrap_or_else(|_| "Aloe requested this command.".to_string()),
        requested_at: Utc::now().to_rfc3339(),
        input: job.input,
    };
    debug_log("job", "approval_queued", format!("job_id={} kind={}", job.id, job.kind));

    // Mirrors this into the backend's approval queue, so it shows up in the web /app/approvals
    // page alongside email/calendar/GitHub actions — not just in this device's own panel.
    // Best-effort: if the socket isn't up right now, the row simply doesn't exist remotely yet.
    if let Some(sender) = state.outbound.lock().expect("outbound mutex").as_ref() {
        let sync_message = json!({
            "type": "approval_requested",
            "jobId": pending.job_id.clone(),
            "reason": pending.reason.clone(),
            "command": pending.command.clone(),
            "cwd": pending.cwd.clone(),
        });
        let _ = sender.send(Message::Text(sync_message.to_string().into()));
    }

    state.pending.lock().expect("pending mutex").push(pending);
    let _ = app.emit("agent://pending-approval", ());
}

/// Resolves one pending command approval, whether the decision came from this device's own
/// approvals panel or was relayed down from the backend after a decision on the web queue
/// (see the `job_decision` branch in socket.rs). Both paths converge here so there is exactly
/// one place that clears `state.pending`, dispatches the command, and posts the result.
pub async fn resolve_pending_approval(app: &AppHandle, job_id: &str, approved: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let pending = {
        let mut list = state.pending.lock().expect("pending mutex");
        let idx = list
            .iter()
            .position(|i| i.job_id == job_id)
            .ok_or("Approval request not found.")?;
        list.remove(idx)
    };
    let config = {
        let stored = state.config.lock().expect("config mutex");
        scoped_config(&stored, pending.conversation_id.as_deref())
    };

    if !approved {
        // Reaches the model as the tool call's error (see runLocalJob in the backend), so it is
        // phrased as something the model can act on rather than a status code.
        let error = "The user denied this command, so it did not run. Do not retry it or work \
                     around it with another tool — acknowledge the decision and ask what they \
                     would like instead."
            .to_string();
        post_result(&state, &config, &pending.job_id, "denied", None, Some(error)).await;
        return Ok(());
    }

    let input_val = pending.input.clone();
    let job = AgentJob {
        id: pending.job_id.clone(),
        kind: pending.job_kind.clone(),
        input: input_val.clone(),
        conversation_id: pending.conversation_id.clone(),
    };
    let result = dispatch_tool(app, &state, &config, &job).await;
    let (status, output, error) = outcome(result);
    post_result(&state, &config, &pending.job_id, status, output.clone(), error.clone()).await;
    record_and_emit(app, &state, &pending.job_id, &pending.job_kind, status, error.as_deref(), Some(input_val), output);
    maybe_resync_folder_context(app, &pending.job_kind, status);
    Ok(())
}

pub async fn dispatch_tool(
    app: &AppHandle,
    state: &tauri::State<'_, AppState>,
    config: &AgentConfig,
    job: &AgentJob,
) -> Result<Value, String> {
    match job.kind.as_str() {
        "search_local_codebase"  => search_codebase(config, &job.input),
        "list_local_files"       => list_files(config, &job.input),
        "read_local_file"        => read_file(config, &job.input),
        "write_local_file"       => update_file(config, &job.input),
        "apply_local_patch"      => apply_patch(config.clone(), job.input.clone()).await,
        "create_file"            => create_file(config, &job.input),
        "write_binary_file"      => write_binary_file(config, &job.input),
        "read_file"              => read_file(config, &job.input),
        "attach_file"            => attach_file(config, &job.input),
        "update_file"            => update_file(config, &job.input),
        "delete_file"            => delete_file(config, &job.input),
        "create_folder"          => create_folder(config, &job.input),
        "read_folder"            => list_files(config, &job.input),
        "update_folder"          => update_folder(config, &job.input),
        "delete_folder"          => delete_folder(config, &job.input),
        "run_command"            => run_command(config.clone(), job.input.clone()).await,
        "run_local_command"      => run_command(config.clone(), job.input.clone()).await,
        "start_terminal_session" => start_terminal_session(state, config.clone(), job.input.clone()).await,
        "read_terminal_session"  => read_terminal_session(state, job.input.clone()).await,
        "write_terminal_session" => write_terminal_session(state, job.input.clone()).await,
        "stop_terminal_session"  => stop_terminal_session(state, job.input.clone()).await,
        "list_terminal_sessions" => list_terminal_sessions(state).await,
        "wait_terminal_session"  => wait_terminal_session(state, job.input.clone()).await,
        "open_local_url"         => open_local_url(&job.input).await,
        "capture_desktop_screenshot" => capture_desktop_screenshot().await,
        "get_editor_context"     => get_editor_context(config, &job.input).await,
        "show_notification"      => show_notification(app, &job.input).await,
        // Browser automation is entirely self-contained (its own Chrome process and CDP
        // connection) and touches no granted folder, so it needs neither config nor AppState.
        kind if kind.starts_with("browser_") => dispatch_browser(kind, &job.input).await,
        _                        => Err(format!("Unknown job type: {}", job.kind)),
    }
}

async fn open_local_url(input: &Value) -> Result<Value, String> {
    let url = input_string(input, "url")?;
    let allowed = url.contains("://") || url.starts_with("mailto:") || url.starts_with("tel:");
    if !allowed {
        return Err("URL must include a scheme such as https://, http://, mailto:, or tel:.".to_string());
    }

    let status = if cfg!(target_os = "windows") {
        let mut cmd = Command::new("rundll32");
        cmd.arg("url.dll,FileProtocolHandler")
            .arg(&url)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_command_window(&mut cmd);
        cmd.status().await
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(&url).status().await
    } else {
        Command::new("xdg-open").arg(&url).status().await
    }
    .map_err(|e| e.to_string())?;

    Ok(json!({ "url": url, "opened": status.success(), "exitCode": status.code() }))
}

async fn capture_desktop_screenshot() -> Result<Value, String> {
    if !cfg!(target_os = "windows") {
        return Err("Desktop screenshot capture is currently implemented for Windows only.".to_string());
    }

    let script = r#"
Add-Type -AssemblyName System.Windows.Forms;
Add-Type -AssemblyName System.Drawing;
$bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds;
$bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height;
$graphics = [System.Drawing.Graphics]::FromImage($bitmap);
$graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size);
$maxDimension = 960;
$largest = [Math]::Max($bounds.Width, $bounds.Height);
$scale = if ($largest -gt $maxDimension) { $maxDimension / $largest } else { 1.0 };
$targetWidth = [Math]::Max(1, [int][Math]::Round($bounds.Width * $scale));
$targetHeight = [Math]::Max(1, [int][Math]::Round($bounds.Height * $scale));
$finalBitmap = if ($scale -lt 1.0) {
    $resized = New-Object System.Drawing.Bitmap $targetWidth, $targetHeight;
    $resizeGraphics = [System.Drawing.Graphics]::FromImage($resized);
    $resizeGraphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic;
    $resizeGraphics.DrawImage($bitmap, 0, 0, $targetWidth, $targetHeight);
    $resizeGraphics.Dispose();
    $resized;
} else {
    $bitmap;
};
$path = Join-Path $env:TEMP ("aloe-screenshot-" + [guid]::NewGuid().ToString() + ".png");
$finalBitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png);
$graphics.Dispose();
if ($finalBitmap -ne $bitmap) { $finalBitmap.Dispose(); }
$bitmap.Dispose();
$bytes = [System.IO.File]::ReadAllBytes($path);
[System.IO.File]::Delete($path);
@{
    mimeType = "image/png";
    base64 = [Convert]::ToBase64String($bytes);
    width = $targetWidth;
    height = $targetHeight;
    originalWidth = $bounds.Width;
    originalHeight = $bounds.Height;
    downscaled = ($scale -lt 1.0);
    maxDimension = $maxDimension;
} | ConvertTo-Json -Compress;
"#;

    let output = tokio::time::timeout(
        Duration::from_secs(20),
        {
            let mut cmd = Command::new("powershell");
            cmd.arg("-NoProfile")
                .arg("-Command")
                .arg(script)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            hide_command_window(&mut cmd);
            cmd.output()
        },
    )
    .await
    .map_err(|_| "Screenshot capture timed out.".to_string())?
    .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let payload = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let mut value: Value = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "note".to_string(),
            Value::String("Primary display screenshot captured by Aloe Desktop.".to_string()),
        );
    }
    Ok(value)
}

async fn get_editor_context(config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let cwd = input
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(|path| assert_granted(config, path))
        .transpose()?;

    let output = tokio::time::timeout(
        Duration::from_secs(5),
        {
            let mut cmd = if cfg!(target_os = "windows") {
            let mut cmd = Command::new("cmd");
            cmd.arg("/C").arg("code --status");
            cmd
        } else {
            let mut cmd = Command::new("sh");
            cmd.arg("-lc").arg("code --status");
            cmd
        };
            if let Some(cwd) = &cwd {
                cmd.current_dir(cwd);
            }
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
            hide_command_window(&mut cmd);
            cmd.output()
        }
    )
    .await
    .map_err(|_| "VS Code status probe timed out.".to_string())?
    .map_err(|e| e.to_string())?;

    Ok(json!({
        "cwd": cwd.map(|p| p.to_string_lossy().to_string()),
        "available": output.status.success(),
        "status": truncate_text(String::from_utf8_lossy(&output.stdout).to_string()),
        "stderr": truncate_text(String::from_utf8_lossy(&output.stderr).to_string()),
        "selectionSupport": "Install a companion editor extension to expose highlighted text and active selections.",
    }))
}

async fn show_notification(app: &AppHandle, input: &Value) -> Result<Value, String> {
    let title = input_string(input, "title")?;
    let message = input_string(input, "message")?;
    notifications::show_clickable(app, &title, &message)?;

    Ok(json!({
        "title": title,
        "message": message,
        "shown": true,
    }))
}

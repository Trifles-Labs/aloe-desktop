use chrono::Utc;
use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tauri::{AppHandle, Emitter, Manager};
use tokio::process::Command;

use crate::config::{add_recent, debug_log, save_config, AppState, COMMAND_TIMEOUT_SECONDS};
use crate::fs::{
    apply_patch, assert_granted, create_file, create_folder, delete_file, delete_folder,
    input_string, list_files, read_file, truncate_text, update_file, update_folder,
};
use crate::models::{AgentConfig, AgentJob, PendingApproval};
use crate::notifications;
use crate::search::search_codebase;
use crate::terminal::{
    list_terminal_sessions, read_terminal_session, start_terminal_session, stop_terminal_session,
    write_terminal_session,
};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(target_os = "windows")]
fn hide_command_window(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn hide_command_window(_command: &mut Command) {}

// ── Shell command execution ───────────────────────────────────────────────────

pub async fn run_command(config: AgentConfig, input: Value) -> Result<Value, String> {
    let cwd = assert_granted(&config, &input_string(&input, "cwd")?)?;
    let command = input_string(&input, "command")?;

    let mut process = if cfg!(target_os = "windows") {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(&command);
        cmd
    } else {
        let mut cmd = Command::new("sh");
        cmd.arg("-lc").arg(&command);
        cmd
    };
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
    let config = state.config.lock().expect("config mutex").clone();

    // Commands require explicit approval (or auto-approval if always_allow is set)
    if job.kind == "run_command" || job.kind == "run_local_command" || job.kind == "start_terminal_session" {
        if config.always_allow_commands {
            let input_snapshot = job.input.clone();
            let result = if job.kind == "start_terminal_session" {
                start_terminal_session(&state, config.clone(), job.input.clone()).await
            } else {
                run_command(config.clone(), job.input.clone()).await
            };
            let (status, output, error) = outcome(result);
            post_result(&state, &config, &job.id, status, output.clone(), error.clone()).await;
            record_and_emit(&app, &state, &job.id, &job.kind, status, error.as_deref(), Some(input_snapshot), output);
            return;
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

fn queue_for_approval(state: &tauri::State<AppState>, app: &AppHandle, job: AgentJob) {
    let pending = PendingApproval {
        job_id: job.id.clone(),
        job_kind: job.kind.clone(),
        command: input_string(&job.input, "command").unwrap_or_default(),
        cwd: input_string(&job.input, "cwd").unwrap_or_default(),
        reason: input_string(&job.input, "reason")
            .unwrap_or_else(|_| "Aloe requested this command.".to_string()),
        requested_at: Utc::now().to_rfc3339(),
        input: job.input,
    };
    debug_log("job", "approval_queued", format!("job_id={} kind={}", job.id, job.kind));
    state.pending.lock().expect("pending mutex").push(pending);
    let _ = app.emit("agent://pending-approval", ());
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
        "read_file"              => read_file(config, &job.input),
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
        "open_local_url"         => open_local_url(&job.input).await,
        "capture_desktop_screenshot" => capture_desktop_screenshot().await,
        "get_editor_context"     => get_editor_context(config, &job.input).await,
        "show_notification"      => show_notification(app, &job.input).await,
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

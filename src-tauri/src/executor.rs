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
use crate::search::search_codebase;

// ── Shell command execution ───────────────────────────────────────────────────

pub async fn run_command(config: AgentConfig, input: Value) -> Result<Value, String> {
    let cwd = assert_granted(&config, &input_string(&input, "cwd")?)?;
    let command = input_string(&input, "command")?;

    let output = if cfg!(target_os = "windows") {
        tokio::time::timeout(
            Duration::from_secs(COMMAND_TIMEOUT_SECONDS),
            Command::new("cmd").arg("/C").arg(&command)
                .current_dir(&cwd).stdout(Stdio::piped()).stderr(Stdio::piped()).output(),
        ).await
    } else {
        tokio::time::timeout(
            Duration::from_secs(COMMAND_TIMEOUT_SECONDS),
            Command::new("sh").arg("-lc").arg(&command)
                .current_dir(&cwd).stdout(Stdio::piped()).stderr(Stdio::piped()).output(),
        ).await
    }
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
    if job.kind == "run_command" || job.kind == "run_local_command" {
        if config.always_allow_commands {
            let input_snapshot = job.input.clone();
            let result = run_command(config.clone(), job.input.clone()).await;
            let (status, output, error) = outcome(result);
            post_result(&state, &config, &job.id, status, output.clone(), error.clone()).await;
            record_and_emit(&app, &state, &job.id, &job.kind, status, error.as_deref(), Some(input_snapshot), output);
            return;
        }
        queue_for_approval(&state, &app, job);
        return;
    }

    let input_snapshot = job.input.clone();
    let result = dispatch_tool(&config, &job).await;
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
        command: input_string(&job.input, "command").unwrap_or_default(),
        cwd: input_string(&job.input, "cwd").unwrap_or_default(),
        reason: input_string(&job.input, "reason")
            .unwrap_or_else(|_| "Aloe requested this command.".to_string()),
        requested_at: Utc::now().to_rfc3339(),
    };
    debug_log("job", "approval_queued", format!("job_id={} kind={}", job.id, job.kind));
    state.pending.lock().expect("pending mutex").push(pending);
    let _ = app.emit("agent://pending-approval", ());
}

async fn dispatch_tool(config: &AgentConfig, job: &AgentJob) -> Result<Value, String> {
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
        _                        => Err(format!("Unknown job type: {}", job.kind)),
    }
}

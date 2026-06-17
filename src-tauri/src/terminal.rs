use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::{
    process::Stdio,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, Command},
};
use uuid::Uuid;

use crate::{
    config::{AppState, COMMAND_TIMEOUT_SECONDS},
    fs::{assert_granted, input_string, truncate_text},
    models::AgentConfig,
};

const MAX_BUFFER_BYTES: usize = 128_000;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(target_os = "windows")]
fn hide_command_window(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn hide_command_window(_command: &mut Command) {}

#[derive(Clone)]
pub struct TerminalSession {
    pub id: String,
    pub command: String,
    pub cwd: String,
    pub started_at: DateTime<Utc>,
    pub output: Arc<Mutex<String>>,
    pub child: Arc<tokio::sync::Mutex<Child>>,
    pub stdin: Arc<tokio::sync::Mutex<Option<ChildStdin>>>,
}

fn append_output(buffer: &Arc<Mutex<String>>, stream: &str, text: &str) {
    let mut output = buffer.lock().expect("terminal output mutex");
    output.push_str(&format!("[{stream}] {text}"));
    if output.len() > MAX_BUFFER_BYTES {
        let keep_from = output.len().saturating_sub(MAX_BUFFER_BYTES);
        let next = output[keep_from..].to_string();
        *output = next;
    }
}

async fn pipe_output<R>(mut reader: R, buffer: Arc<Mutex<String>>, stream: &'static str)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut chunk = vec![0u8; 4096];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => append_output(&buffer, stream, &String::from_utf8_lossy(&chunk[..n])),
            Err(err) => {
                append_output(&buffer, stream, &format!("read error: {err}\n"));
                break;
            }
        }
    }
}

pub async fn start_terminal_session(
    state: &AppState,
    config: AgentConfig,
    input: Value,
) -> Result<Value, String> {
    let cwd = assert_granted(&config, &input_string(&input, "cwd")?)?;
    let command = input_string(&input, "command")?;
    let session_id = Uuid::new_v4().to_string();

    let mut process = if cfg!(target_os = "windows") {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(&command);
        cmd
    } else {
        let mut cmd = Command::new("sh");
        cmd.arg("-lc").arg(&command);
        cmd
    };
    process
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_command_window(&mut process);

    let mut child = process.spawn().map_err(|e| e.to_string())?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdin = Arc::new(tokio::sync::Mutex::new(child.stdin.take()));
    let output = Arc::new(Mutex::new(String::new()));
    let child = Arc::new(tokio::sync::Mutex::new(child));

    if let Some(stdout) = stdout {
        tokio::spawn(pipe_output(stdout, output.clone(), "stdout"));
    }
    if let Some(stderr) = stderr {
        tokio::spawn(pipe_output(stderr, output.clone(), "stderr"));
    }

    let session = TerminalSession {
        id: session_id.clone(),
        command: command.clone(),
        cwd: cwd.to_string_lossy().to_string(),
        started_at: Utc::now(),
        output: output.clone(),
        child,
        stdin,
    };

    state
        .terminals
        .lock()
        .expect("terminal sessions mutex")
        .insert(session_id.clone(), session);

    Ok(json!({
        "sessionId": session_id,
        "cwd": cwd.to_string_lossy(),
        "command": command,
        "status": "running",
        "readHint": "Use read_terminal_session with this sessionId to fetch logs.",
    }))
}

pub async fn read_terminal_session(state: &AppState, input: Value) -> Result<Value, String> {
    let session_id = input_string(&input, "sessionId")?;
    let session = state
        .terminals
        .lock()
        .expect("terminal sessions mutex")
        .get(&session_id)
        .cloned()
        .ok_or_else(|| format!("Terminal session not found: {session_id}"))?;

    let mut child = session.child.lock().await;
    let status = match child.try_wait().map_err(|e| e.to_string())? {
        Some(exit) => json!({ "state": "exited", "exitCode": exit.code() }),
        None => json!({ "state": "running", "exitCode": null }),
    };
    let output = session.output.lock().expect("terminal output mutex").clone();

    Ok(json!({
        "sessionId": session.id,
        "cwd": session.cwd,
        "command": session.command,
        "startedAt": session.started_at.to_rfc3339(),
        "status": status,
        "output": truncate_text(output),
    }))
}

pub async fn write_terminal_session(state: &AppState, input: Value) -> Result<Value, String> {
    let session_id = input_string(&input, "sessionId")?;
    let text = input_string(&input, "input")?;
    let session = state
        .terminals
        .lock()
        .expect("terminal sessions mutex")
        .get(&session_id)
        .cloned()
        .ok_or_else(|| format!("Terminal session not found: {session_id}"))?;

    let mut stdin = session.stdin.lock().await;
    let Some(stdin) = stdin.as_mut() else {
        return Err("Terminal session stdin is closed.".to_string());
    };
    stdin.write_all(text.as_bytes()).await.map_err(|e| e.to_string())?;
    stdin.flush().await.map_err(|e| e.to_string())?;
    Ok(json!({ "sessionId": session_id, "writtenBytes": text.len() }))
}

pub async fn stop_terminal_session(state: &AppState, input: Value) -> Result<Value, String> {
    let session_id = input_string(&input, "sessionId")?;
    let session = state
        .terminals
        .lock()
        .expect("terminal sessions mutex")
        .remove(&session_id)
        .ok_or_else(|| format!("Terminal session not found: {session_id}"))?;

    let mut child = session.child.lock().await;
    let _ = child.kill().await;
    Ok(json!({ "sessionId": session_id, "status": "stopped" }))
}

pub async fn list_terminal_sessions(state: &AppState) -> Result<Value, String> {
    let sessions: Vec<TerminalSession> = state
        .terminals
        .lock()
        .expect("terminal sessions mutex")
        .values()
        .cloned()
        .collect();

    let mut items = Vec::new();
    for session in sessions {
        let mut child = session.child.lock().await;
        let status = match child.try_wait().map_err(|e| e.to_string())? {
            Some(exit) => json!({ "state": "exited", "exitCode": exit.code() }),
            None => json!({ "state": "running", "exitCode": null }),
        };
        items.push(json!({
            "sessionId": session.id,
            "cwd": session.cwd,
            "command": session.command,
            "startedAt": session.started_at.to_rfc3339(),
            "status": status,
        }));
    }

    Ok(json!({ "sessions": items, "commandTimeoutSeconds": COMMAND_TIMEOUT_SECONDS }))
}

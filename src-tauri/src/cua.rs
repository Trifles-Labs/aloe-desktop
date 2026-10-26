//! Desktop control through Cua Driver (github.com/trycua/cua), bundled with the app.
//!
//! The driver runs as one long-lived `cua-driver mcp` child speaking MCP over stdio. It has to be
//! long-lived: element tokens, window snapshots and the agent session all live inside it, so a
//! process per call would forget every `get_window_state` the model just read.
//!
//! The model calls Cua's own tools by name (`desktop_cua` jobs carry `{ tool, arguments }`), but
//! only the ones in `ALLOWED_TOOLS`, and every call passes through `prepare_arguments` first. The
//! guards the driver knows nothing about live there:
//!  - Aloe Desktop's own process and windows are off limits, by pid and by window handle.
//!  - A desktop-scope point on Aloe's window, or desktop-scope keys while it has focus, is refused.
//!  - Nothing that writes files, starts an arbitrary program, or runs a script gets through.
//! The on/off switch and the corner failsafe are checked by the caller in computer.rs.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use image::codecs::jpeg::JpegEncoder;
use serde_json::{json, Map, Value};
use std::{path::PathBuf, process::Stdio, sync::Mutex, time::Duration};
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
};

use crate::config::{debug_log, MAX_DESKTOP_TYPE_CHARS, SCREENSHOT_JPEG_QUALITY, SCREENSHOT_MAX_DIMENSION};
use crate::executor::checked_open_url;
use crate::shell::hide_command_window;

/// Cua tools the desktop agent may call — the ones aloe-backend's computer.ts exposes. Left out on
/// purpose: run_script / run_steps (bypass the per-call guards), kill_app, clipboard_*, the
/// browser_* / page CDP tools, recording, config, sessions (managed here), and everything that
/// installs or updates.
const ALLOWED_TOOLS: [&str; 13] = [
    "list_apps",
    "list_windows",
    "launch_app",
    "get_window_state",
    "get_desktop_state",
    "zoom",
    "click",
    "drag",
    "scroll",
    "type_text",
    "press_key",
    "hotkey",
    "set_value",
];

/// Arguments that make the driver write files wherever the caller says.
const FILE_WRITING_ARGUMENTS: [&str; 3] = ["screenshot_out_file", "debug_image_out", "output_path"];

/// The driver's own walk budget can go to 120s; a step that slow is a stuck step.
const MAX_TREE_TIMEOUT_MS: u64 = 20_000;
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
const START_TIMEOUT: Duration = Duration::from_secs(20);
/// Text beyond this is cut: a 5 000-element tree would otherwise fill the model's context.
const MAX_TEXT_CHARS: usize = 60_000;
/// Action results (`effect`, `route`, `escalation`) are small and worth passing on as-is;
/// observation payloads repeat the text and are dropped.
const MAX_STRUCTURED_CHARS: usize = 4_000;
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

const MISSING_DRIVER_MESSAGE: &str = "Cua Driver is missing from this Aloe Desktop install, so desktop \
    control is unavailable. Tell the user to reinstall or update Aloe Desktop.";
const OWN_PROCESS_MESSAGE: &str = "That window belongs to Aloe Desktop itself, which Aloe cannot control. \
    Target another app, or ask the user to do this step themselves.";

struct Driver {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

/// One call at a time: the desktop agent is a single sequential loop, and serialising here keeps
/// request/response matching trivial.
static DRIVER: tokio::sync::Mutex<Option<Driver>> = tokio::sync::Mutex::const_new(None);
/// The Cua session label for the current run. A fresh one per run, since an ended session name
/// is never revived by ordinary actions.
static SESSION: Mutex<Option<String>> = Mutex::new(None);
/// Screen pixels per pixel of the last `get_desktop_state` image, for mapping desktop-scope points
/// onto Aloe's window rectangle.
static DESKTOP_SCALE: Mutex<Option<f64>> = Mutex::new(None);

/// What the own-window guards need to know about Aloe's main window, in physical screen pixels.
#[derive(Clone, Copy)]
pub struct OwnWindow {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub focused: bool,
}

/// Runs one Cua tool for the desktop agent. `own_window` is Aloe's main window when it is on
/// screen; `display_width` is the primary display's width in physical pixels.
pub async fn call(app: &AppHandle, input: &Value, own_window: Option<OwnWindow>, display_width: Option<i32>) -> Result<Value, String> {
    let tool = input.get("tool").and_then(Value::as_str).ok_or("tool is required")?.to_string();
    let arguments = input.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let mut arguments = prepare_arguments(&tool, arguments, &session_label())?;
    guard_own_process(&arguments)?;
    guard_desktop_scope(&tool, &arguments, own_window)?;
    if let Some(urls) = arguments.get_mut("urls").and_then(Value::as_array_mut) {
        for url in urls.iter_mut() {
            let raw = url.as_str().ok_or("urls must be strings")?;
            *url = Value::String(checked_open_url(raw)?);
        }
    }

    let result = request(app, "tools/call", json!({ "name": tool, "arguments": arguments })).await?;
    let shaped = shape_result(&result)?;
    if tool == "get_desktop_state" {
        let scale = match (display_width, shaped.get("width").and_then(Value::as_f64)) {
            (Some(display), Some(image)) if image > 0.0 => Some(display as f64 / image),
            _ => None,
        };
        *DESKTOP_SCALE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = scale;
    }
    Ok(shaped)
}

/// The run is over: end its Cua session (dropping its snapshots and agent cursor) so the next run
/// starts clean. Best-effort and quick — it must never hold up `desktop_release`.
pub async fn end_run(app: &AppHandle) {
    let Some(session) = SESSION.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() else {
        return;
    };
    *DESKTOP_SCALE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    if DRIVER.lock().await.is_none() {
        return;
    }
    let ended = tokio::time::timeout(
        Duration::from_secs(5),
        request(app, "tools/call", json!({ "name": "end_session", "arguments": { "session": session } })),
    )
    .await;
    if !matches!(ended, Ok(Ok(_))) {
        debug_log("cua", "end_session_failed", &session);
    }
}

/// Desktop control was switched off: stop the driver entirely.
pub async fn shutdown() {
    SESSION.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
    if let Some(mut driver) = DRIVER.lock().await.take() {
        let _ = driver.child.kill().await;
    }
}

fn session_label() -> String {
    SESSION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(|| format!("aloe-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]))
        .clone()
}

// ── Arguments and guards ─────────────────────────────────────────────────────

fn prepare_arguments(tool: &str, arguments: Value, session: &str) -> Result<Value, String> {
    if !ALLOWED_TOOLS.contains(&tool) {
        return Err(format!("Aloe Desktop does not allow the Cua tool \"{tool}\". Use one of: {}.", ALLOWED_TOOLS.join(", ")));
    }
    let Value::Object(mut arguments) = arguments else {
        return Err("arguments must be an object.".to_string());
    };
    for key in FILE_WRITING_ARGUMENTS {
        arguments.remove(key);
    }
    arguments.insert("session".to_string(), Value::String(session.to_string()));

    match tool {
        "get_window_state" | "get_desktop_state" => {
            arguments
                .entry("max_image_dimension")
                .or_insert_with(|| json!(SCREENSHOT_MAX_DIMENSION));
            if let Some(timeout) = arguments.get("timeout_ms").and_then(Value::as_u64) {
                arguments.insert("timeout_ms".to_string(), json!(timeout.min(MAX_TREE_TIMEOUT_MS)));
            }
        }
        // Opening an app by name is fine; an arbitrary executable path or command line would let
        // the model start any program without the command approval terminal tools go through.
        "launch_app" => {
            for key in ["path", "launch_path", "additional_arguments"] {
                if arguments.contains_key(key) {
                    return Err(format!(
                        "launch_app does not accept {key} in Aloe. Open the app by name (or aumid), or use the terminal tools to run a program."
                    ));
                }
            }
        }
        "type_text" => {
            let length = arguments.get("text").and_then(Value::as_str).map(|text| text.chars().count()).unwrap_or(0);
            if length > MAX_DESKTOP_TYPE_CHARS {
                return Err(format!("Text is {length} characters; type at most {MAX_DESKTOP_TYPE_CHARS} per call."));
            }
        }
        _ => {}
    }
    Ok(Value::Object(arguments))
}

/// Every pid and window handle the call names, at the top level or inside `target`.
fn named_targets(arguments: &Value) -> (Vec<u64>, Vec<u64>) {
    let mut pids = Vec::new();
    let mut windows = Vec::new();
    for scope in [Some(arguments), arguments.get("target")].into_iter().flatten() {
        if let Some(pid) = scope.get("pid").and_then(Value::as_u64) {
            pids.push(pid);
        }
        if let Some(window) = scope.get("window_id").and_then(Value::as_u64) {
            windows.push(window);
        }
    }
    (pids, windows)
}

fn guard_own_process(arguments: &Value) -> Result<(), String> {
    let own = std::process::id() as u64;
    let (pids, windows) = named_targets(arguments);
    if pids.contains(&own) || windows.into_iter().any(|window| window_owner(window) == Some(own)) {
        return Err(OWN_PROCESS_MESSAGE.to_string());
    }
    Ok(())
}

#[cfg(windows)]
fn window_owner(window: u64) -> Option<u64> {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    let mut pid = 0u32;
    let thread = unsafe { GetWindowThreadProcessId(window as usize as _, &mut pid) };
    (thread != 0).then_some(pid as u64)
}

/// Elsewhere window ids are not handles this process can look up; the pid check still applies.
#[cfg(not(windows))]
fn window_owner(_window: u64) -> Option<u64> {
    None
}

/// A call aimed at the desktop as a whole rather than one window: it acts wherever the point is,
/// or on whatever has keyboard focus.
fn is_desktop_scope(arguments: &Value) -> bool {
    let (pids, windows) = named_targets(arguments);
    if !pids.is_empty() || !windows.is_empty() || arguments.get("element_token").is_some() {
        return false;
    }
    let target_kind = arguments.get("target").and_then(|target| target.get("kind")).and_then(Value::as_str);
    let scope = arguments.get("scope").or_else(|| arguments.get("coordinate_frame")).and_then(Value::as_str);
    target_kind == Some("desktop") || scope == Some("desktop")
}

fn guard_desktop_scope(tool: &str, arguments: &Value, own_window: Option<OwnWindow>) -> Result<(), String> {
    let Some(own) = own_window else { return Ok(()) };
    if !is_desktop_scope(arguments) {
        return Ok(());
    }
    match tool {
        "type_text" | "press_key" | "hotkey" if own.focused => Err(crate::computer::OWN_FOCUS_MESSAGE.to_string()),
        "click" | "drag" | "scroll" => {
            let scale = DESKTOP_SCALE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).unwrap_or(1.0);
            let points = [("x", "y"), ("from_x", "from_y"), ("to_x", "to_y")];
            for (x_key, y_key) in points {
                let (Some(x), Some(y)) = (arguments.get(x_key).and_then(Value::as_f64), arguments.get(y_key).and_then(Value::as_f64)) else {
                    continue;
                };
                let (sx, sy) = ((x * scale).round() as i32, (y * scale).round() as i32);
                if sx >= own.x && sx < own.x + own.w && sy >= own.y && sy < own.y + own.h {
                    return Err(crate::computer::OWN_WINDOW_MESSAGE.to_string());
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

// ── Results ──────────────────────────────────────────────────────────────────

/// An MCP tool result → what the backend expects from a desktop job: the driver's text, a small
/// structured action result when there is one, and the first image as top-level
/// `mimeType`/`base64` (re-encoded as JPEG, same pixel size, so coordinates read off it still hold).
fn shape_result(result: &Value) -> Result<Value, String> {
    let mut texts = Vec::new();
    let mut image = None;
    for item in result.get("content").and_then(Value::as_array).into_iter().flatten() {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => texts.extend(item.get("text").and_then(Value::as_str).map(str::to_string)),
            Some("image") if image.is_none() => {
                if let (Some(data), Some(mime)) = (item.get("data").and_then(Value::as_str), item.get("mimeType").and_then(Value::as_str)) {
                    image = Some((data.to_string(), mime.to_string()));
                }
            }
            _ => {}
        }
    }
    let text = truncate(texts.join("\n"));
    if result.get("isError").and_then(Value::as_bool).unwrap_or(false) {
        return Err(if text.is_empty() { "Cua Driver reported an error without details.".to_string() } else { text });
    }

    let mut shaped = Map::new();
    shaped.insert("text".to_string(), Value::String(text));
    if let Some(structured) = result.get("structuredContent") {
        if structured.to_string().len() <= MAX_STRUCTURED_CHARS {
            shaped.insert("structured".to_string(), structured.clone());
        }
    }
    if let Some((data, mime)) = image {
        let (mime, data, size) = match to_jpeg(&data) {
            Ok((jpeg, width, height)) => ("image/jpeg".to_string(), jpeg, Some((width, height))),
            Err(error) => {
                debug_log("cua", "jpeg_reencode_failed", error);
                (mime, data, None)
            }
        };
        shaped.insert("mimeType".to_string(), Value::String(mime));
        shaped.insert("base64".to_string(), Value::String(data));
        if let Some((width, height)) = size {
            shaped.insert("width".to_string(), json!(width));
            shaped.insert("height".to_string(), json!(height));
        }
    }
    Ok(Value::Object(shaped))
}

fn truncate(text: String) -> String {
    if text.chars().count() <= MAX_TEXT_CHARS {
        return text;
    }
    let cut: String = text.chars().take(MAX_TEXT_CHARS).collect();
    format!("{cut}\n… [cut at {MAX_TEXT_CHARS} characters; narrow the read with query or max_elements]")
}

fn to_jpeg(base64_png: &str) -> Result<(String, u32, u32), String> {
    let bytes = BASE64.decode(base64_png).map_err(|e| e.to_string())?;
    let rgb = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.to_rgb8();
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, SCREENSHOT_JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(|e| e.to_string())?;
    Ok((BASE64.encode(&jpeg), rgb.width(), rgb.height()))
}

// ── The driver process ───────────────────────────────────────────────────────

fn driver_path(app: &AppHandle) -> Result<PathBuf, String> {
    // For development against a separately installed driver.
    if let Ok(path) = std::env::var("ALOE_CUA_DRIVER") {
        return Ok(PathBuf::from(path));
    }
    let name = if cfg!(windows) { "cua-driver.exe" } else { "cua-driver" };
    let bundled = app.path().resource_dir().map_err(|e| e.to_string())?.join("cua-driver").join(name);
    if bundled.is_file() {
        Ok(bundled)
    } else {
        debug_log("cua", "driver_missing", bundled.display().to_string());
        Err(MISSING_DRIVER_MESSAGE.to_string())
    }
}

async fn start(app: &AppHandle) -> Result<Driver, String> {
    let path = driver_path(app)?;
    let mut command = Command::new(&path);
    command.arg("mcp");
    // Bare `mcp` on macOS proxies to a separately installed CuaDriver.app; --direct keeps the
    // runtime in this child, so Accessibility and Screen Recording are Aloe Desktop's grants.
    if cfg!(target_os = "macos") {
        command.arg("--direct");
    }
    command
        // Cua's product telemetry is on by default; Aloe's users never opted into it.
        .env("DO_NOT_TRACK", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    hide_command_window(&mut command);
    let mut child = command.spawn().map_err(|e| format!("Could not start Cua Driver: {e}"))?;
    let stdin = child.stdin.take().ok_or("Cua Driver has no stdin")?;
    let stdout = BufReader::new(child.stdout.take().ok_or("Cua Driver has no stdout")?).lines();
    let mut driver = Driver { child, stdin, stdout, next_id: 1 };

    let initialize = json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": { "name": "aloe-desktop", "version": env!("CARGO_PKG_VERSION") },
    });
    tokio::time::timeout(START_TIMEOUT, driver.exchange("initialize", initialize))
        .await
        .map_err(|_| "Cua Driver did not start in time.".to_string())??;
    driver.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await?;
    debug_log("cua", "started", path.display().to_string());
    Ok(driver)
}

async fn request(app: &AppHandle, method: &str, params: Value) -> Result<Value, String> {
    let mut slot = DRIVER.lock().await;
    let alive = match slot.as_mut() {
        Some(driver) => matches!(driver.child.try_wait(), Ok(None)),
        None => false,
    };
    if !alive {
        *slot = Some(start(app).await?);
    }
    let driver = slot.as_mut().expect("driver was just started");
    match tokio::time::timeout(CALL_TIMEOUT, driver.exchange(method, params)).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(error)) => {
            // A broken pipe or garbled stream: start over on the next call.
            if !matches!(driver.child.try_wait(), Ok(None)) {
                *slot = None;
            }
            Err(error)
        }
        Err(_) => {
            // The reply may still arrive and would then answer the wrong request, so the process
            // goes; its snapshots go with it, and the model is told to look again.
            *slot = None;
            Err("Cua Driver did not answer in time, so it was restarted. Take a fresh get_window_state before acting again.".to_string())
        }
    }
}

impl Driver {
    async fn send(&mut self, message: &Value) -> Result<(), String> {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await.map_err(|e| format!("Could not reach Cua Driver: {e}"))?;
        self.stdin.flush().await.map_err(|e| format!("Could not reach Cua Driver: {e}"))
    }

    /// Sends a request and reads lines until its response, skipping the server's notifications
    /// and log lines.
    async fn exchange(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await?;
        loop {
            let line = self
                .stdout
                .next_line()
                .await
                .map_err(|e| format!("Could not read from Cua Driver: {e}"))?
                .ok_or("Cua Driver exited unexpectedly.")?;
            let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                let detail = error.get("message").and_then(Value::as_str).unwrap_or("unknown error");
                return Err(format!("Cua Driver refused the call: {detail}"));
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_tools_outside_the_allowlist() {
        assert!(prepare_arguments("run_script", json!({}), "s").is_err());
        assert!(prepare_arguments("kill_app", json!({ "pid": 4 }), "s").is_err());
        assert!(prepare_arguments("list_windows", json!({}), "s").is_ok());
    }

    #[test]
    fn pins_the_session_and_strips_file_writes() {
        let prepared = prepare_arguments(
            "get_window_state",
            json!({ "pid": 1, "window_id": 2, "session": "other", "screenshot_out_file": "C:/x.png", "timeout_ms": 120000 }),
            "aloe-1",
        )
        .unwrap();
        assert_eq!(prepared["session"], "aloe-1");
        assert!(prepared.get("screenshot_out_file").is_none());
        assert_eq!(prepared["timeout_ms"], MAX_TREE_TIMEOUT_MS);
        assert_eq!(prepared["max_image_dimension"], SCREENSHOT_MAX_DIMENSION);
    }

    #[test]
    fn launch_app_takes_names_not_command_lines() {
        assert!(prepare_arguments("launch_app", json!({ "name": "Notepad" }), "s").is_ok());
        assert!(prepare_arguments("launch_app", json!({ "path": "C:/Windows/System32/cmd.exe" }), "s").is_err());
        assert!(prepare_arguments("launch_app", json!({ "name": "x", "additional_arguments": ["/c"] }), "s").is_err());
    }

    #[test]
    fn refuses_aloes_own_pid() {
        let own = std::process::id();
        assert!(guard_own_process(&json!({ "pid": own })).is_err());
        assert!(guard_own_process(&json!({ "target": { "kind": "window", "pid": own, "window_id": 1 } })).is_err());
        assert!(guard_own_process(&json!({ "pid": own + 1 })).is_ok());
    }

    #[test]
    fn desktop_scope_points_on_aloes_window_are_refused() {
        let own = Some(OwnWindow { x: 100, y: 100, w: 200, h: 200, focused: true });
        let inside = json!({ "scope": "desktop", "x": 150, "y": 150 });
        let outside = json!({ "scope": "desktop", "x": 50, "y": 50 });
        let in_a_window = json!({ "pid": 5, "window_id": 6, "x": 150, "y": 150 });
        assert!(guard_desktop_scope("click", &inside, own).is_err());
        assert!(guard_desktop_scope("click", &outside, own).is_ok());
        assert!(guard_desktop_scope("click", &in_a_window, own).is_ok());
        assert!(guard_desktop_scope("press_key", &json!({ "target": { "kind": "desktop", "display_id": "primary" }, "key": "a" }), own).is_err());
        assert!(guard_desktop_scope("click", &inside, None).is_ok());
    }

    #[test]
    fn shapes_text_structured_and_errors() {
        let ok = shape_result(&json!({
            "content": [{ "type": "text", "text": "clicked" }],
            "structuredContent": { "effect": "confirmed" },
        }))
        .unwrap();
        assert_eq!(ok["text"], "clicked");
        assert_eq!(ok["structured"]["effect"], "confirmed");
        assert!(ok.get("base64").is_none());

        let failed = shape_result(&json!({ "content": [{ "type": "text", "text": "background_unavailable" }], "isError": true }));
        assert_eq!(failed.unwrap_err(), "background_unavailable");
    }
}

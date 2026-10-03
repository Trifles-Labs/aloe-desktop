//! Desktop control: screenshots of the primary display, plus synthetic mouse and keyboard input.
//!
//! Every coordinate the model sends is on a 0–1000 grid over the screenshot on both axes, not in
//! pixels: (0, 0) is the top-left corner and (1000, 1000) the bottom-right. That is the space
//! Gemini locates things in natively whatever the image size — given pixel space it answered in
//! 0–1000 anyway (y=975 on a 720px-tall screenshot), so most clicks landed in the wrong place.
//! `ScreenGeometry` maps the grid onto the display without remembering any earlier screenshot.
//!
//! The guards all live here because this is the side that actually moves the pointer:
//!  - `desktop_control_enabled` must be on. It is off by default and only this app's own UI sets it.
//!  - The failsafe: a pointer the *user* parked in a display corner refuses the action and turns
//!    control back off.
//!  - Aloe Desktop's own window is off limits, so the model can never click its own approvals or
//!    flip its own settings.
//!
//! Only the primary display is captured or targeted.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use image::{
    codecs::jpeg::JpegEncoder,
    imageops::{self, FilterType},
    DynamicImage,
};
use serde_json::{json, Value};
use std::{
    sync::Mutex,
    thread,
    time::Duration,
};
use tauri::{AppHandle, Manager};
use xcap::Monitor;

use crate::config::{
    debug_log, save_config, AppState, DESKTOP_CONTROL_FAILSAFE_PX, DESKTOP_CONTROL_SETTLE_MS, MAX_DESKTOP_TYPE_CHARS,
    SCREENSHOT_JPEG_QUALITY, SCREENSHOT_MAX_DIMENSION,
};
use crate::fs::input_string;
use crate::notifications;
use crate::overlay;

// These reach the model as the tool call's error, so each one says what to do next.
const DISABLED_MESSAGE: &str = "Desktop control is turned off on this computer, and Aloe Desktop asked the \
    user to turn it on but they chose not to. Do not ask again or retry; continue without it, or tell \
    the user what they would need to do themselves. Do not try to get the same result through \
    terminal commands or other tools.";
const FAILSAFE_MESSAGE: &str = "The user stopped desktop control by moving the pointer into a screen \
    corner, and it is now turned off. Stop the task, tell the user it was stopped, and wait for them \
    to turn it back on.";
const OWN_WINDOW_MESSAGE: &str = "That point is on Aloe Desktop's own window, which Aloe cannot click. \
    Target another app, or ask the user to do this step themselves.";
const OWN_FOCUS_MESSAGE: &str = "Aloe Desktop's own window has keyboard focus, so typing would land \
    in it. Click into the target app first.";

/// Where Aloe last left the pointer. The failsafe only fires for a corner the pointer reached some
/// other way; otherwise clicking something that sits in a corner (the Windows Start button) would
/// trip it on the very next action.
static LAST_POINTER: Mutex<Option<(i32, i32)>> = Mutex::new(None);

enum ControlError {
    Failsafe,
    Failed(String),
}

impl From<String> for ControlError {
    fn from(message: String) -> Self {
        ControlError::Failed(message)
    }
}

fn input_error(error: enigo::InputError) -> ControlError {
    ControlError::Failed(format!("Input failed: {error}"))
}

// ── Geometry ─────────────────────────────────────────────────────────────────

/// Extent of the coordinate grid the model points with, on both axes.
const GRID: f64 = 1000.0;
/// Minimize animation plus repaint of whatever was behind Aloe's window.
const MAIN_WINDOW_CLEAR_MS: u64 = 400;

/// The primary display in input coordinates (physical pixels, or points on macOS — whatever
/// enigo moves the pointer in), and the screenshot size that display is shown to the model at.
#[derive(Clone, Copy)]
struct ScreenGeometry {
    display_w: i32,
    display_h: i32,
    shot_w: u32,
    shot_h: u32,
}

impl ScreenGeometry {
    fn from_display(display_w: i32, display_h: i32) -> Self {
        let display_w = display_w.max(1);
        let display_h = display_h.max(1);
        let largest = display_w.max(display_h) as f64;
        let scale = (SCREENSHOT_MAX_DIMENSION as f64 / largest).min(1.0);
        Self {
            display_w,
            display_h,
            shot_w: ((display_w as f64 * scale).round() as u32).max(1),
            shot_h: ((display_h as f64 * scale).round() as u32).max(1),
        }
    }

    /// A point on the 0–1000 grid → input coordinates on the display.
    fn to_display(&self, x: f64, y: f64) -> (i32, i32) {
        let dx = (x / GRID * self.display_w as f64).round() as i32;
        let dy = (y / GRID * self.display_h as f64).round() as i32;
        (dx.clamp(0, self.display_w - 1), dy.clamp(0, self.display_h - 1))
    }

    fn in_corner(&self, (x, y): (i32, i32)) -> bool {
        let margin = DESKTOP_CONTROL_FAILSAFE_PX;
        let near_x = x <= margin || x >= self.display_w - 1 - margin;
        let near_y = y <= margin || y >= self.display_h - 1 - margin;
        near_x && near_y
    }
}

fn new_enigo() -> Result<Enigo, String> {
    Enigo::new(&Settings::default()).map_err(|e| format!("Could not access mouse and keyboard input: {e}.{}", permission_hint()))
}

fn display_geometry(enigo: &Enigo) -> Result<ScreenGeometry, String> {
    let (w, h) = enigo.main_display().map_err(|e| format!("Could not read the display size: {e}"))?;
    Ok(ScreenGeometry::from_display(w, h))
}

fn permission_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        " On macOS, allow Aloe Desktop under System Settings > Privacy & Security > Accessibility and Screen Recording."
    } else if cfg!(target_os = "linux") {
        " On Linux, desktop control needs an X11 session."
    } else {
        ""
    }
}

/// Runs `f` where synthetic input is allowed to happen. macOS requires its text-input APIs on the
/// main thread (enigo crashes elsewhere on recent releases); everywhere else a blocking worker keeps
/// the event loop free during a drag or a long `type`.
async fn on_input_thread<T: Send + 'static>(app: &AppHandle, f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let _ = tx.send(f());
        })
        .map_err(|e| e.to_string())?;
        return rx.await.map_err(|e| e.to_string());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())
    }
}

// ── Screenshots ──────────────────────────────────────────────────────────────

fn capture(geometry: Option<ScreenGeometry>) -> Result<Value, String> {
    let monitors = Monitor::all().map_err(|e| format!("Could not list displays: {e}"))?;
    let monitor = monitors
        .iter()
        .find(|m| m.is_primary())
        .or_else(|| monitors.first())
        .ok_or_else(|| "No display found.".to_string())?;
    // Input access can be missing (Wayland, no Accessibility grant) while capture still works, so a
    // plain screenshot falls back to the monitor's own size rather than failing outright.
    let geometry = geometry.unwrap_or_else(|| ScreenGeometry::from_display(monitor.width() as i32, monitor.height() as i32));

    let captured = monitor
        .capture_image()
        .map_err(|e| format!("Screen capture failed: {e}.{}", permission_hint()))?;
    let rgb = DynamicImage::ImageRgba8(captured).to_rgb8();
    let rgb = if rgb.dimensions() == (geometry.shot_w, geometry.shot_h) {
        rgb
    } else {
        imageops::resize(&rgb, geometry.shot_w, geometry.shot_h, FilterType::Triangle)
    };

    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, SCREENSHOT_JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(|e| format!("Could not encode the screenshot: {e}"))?;

    Ok(json!({
        "mimeType": "image/jpeg",
        "base64": BASE64.encode(&jpeg),
        "width": geometry.shot_w,
        "height": geometry.shot_h,
        "note": "Primary display captured by Aloe Desktop. Point at things on a 0–1000 grid over this image on both axes: (0, 0) top-left, (1000, 1000) bottom-right.",
    }))
}

pub async fn capture_desktop_screenshot(app: &AppHandle) -> Result<Value, String> {
    on_input_thread(app, || {
        let geometry = new_enigo().and_then(|enigo| display_geometry(&enigo)).ok();
        capture(geometry)
    })
    .await?
}

// ── Asking to turn control on ────────────────────────────────────────────────

/// After "Not now", further requests are refused without asking again for this long, so a model
/// retrying in a loop cannot turn the dialog into a nag.
const DECLINE_QUIET_PERIOD: Duration = Duration::from_secs(120);

/// One dialog at a time: actions that arrive while it is open wait for that answer.
static ASK_LOCK: tokio::sync::Mutex<Option<std::time::Instant>> = tokio::sync::Mutex::const_new(None);

fn control_enabled(app: &AppHandle) -> bool {
    app.state::<AppState>().config.lock().expect("config mutex").desktop_control_enabled
}

/// True when desktop control is on, asking the user first if it is off. The question is a native
/// dialog owned by this app: it is not something the model can click (control is off while it is
/// up, and Aloe's own windows are off limits anyway), so the switch is still only ever flipped by
/// the user.
async fn ensure_enabled_or_ask(app: &AppHandle) -> bool {
    if control_enabled(app) {
        return true;
    }
    let mut declined_at = ASK_LOCK.lock().await;
    // Answered "Turn on" while this request was waiting behind the dialog.
    if control_enabled(app) {
        return true;
    }
    if declined_at.map(|at| at.elapsed() < DECLINE_QUIET_PERIOD).unwrap_or(false) {
        return false;
    }

    let handle = app.clone();
    let accepted = tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
        handle
            .dialog()
            .message(
                "Aloe wants to see and control your screen (mouse and keyboard) to carry out a task. \
                 You can stop it any time with the Stop button or by moving the pointer into a screen corner.",
            )
            .title("Turn on desktop control?")
            .kind(MessageDialogKind::Info)
            .buttons(MessageDialogButtons::OkCancelCustom("Turn on".to_string(), "Not now".to_string()))
            .blocking_show()
    })
    .await
    .unwrap_or(false);

    debug_log("desktop", "control_prompt", format!("accepted={accepted}"));
    if !accepted {
        *declined_at = Some(std::time::Instant::now());
        return false;
    }
    *declined_at = None;
    let state = app.state::<AppState>();
    let mut config = state.config.lock().expect("config mutex");
    config.desktop_control_enabled = true;
    let _ = save_config(&config);
    true
}

// ── Actions ──────────────────────────────────────────────────────────────────

/// Entry point for every `desktop_*` job. Each successful action comes back with a fresh
/// screenshot, since the model's next step almost always needs to see what the action did.
pub async fn dispatch_desktop_control(app: &AppHandle, kind: &str, input: &Value) -> Result<Value, String> {
    // Always honoured, even with control switched off: it only takes the overlay down.
    if kind == "desktop_release" {
        overlay::release(app);
        return Ok(json!({ "released": true }));
    }

    if !ensure_enabled_or_ask(app).await {
        return Err(DISABLED_MESSAGE.to_string());
    }
    if overlay::mark_active(app) {
        // Aloe's window just minimized; let the desktop behind it finish repainting before the
        // screenshot or action that follows sees it.
        tokio::time::sleep(Duration::from_millis(MAIN_WINDOW_CLEAR_MS)).await;
    }
    // The desktop agent's own screenshot: part of the run (overlay up, Aloe's window cleared),
    // unlike local_desktop_screenshot, which captures whatever is on screen as-is.
    if kind == "desktop_screenshot" {
        return capture_desktop_screenshot(app).await;
    }

    let window = aloe_window(app);
    let action = kind.to_string();
    let job_input = input.clone();
    // The Stop pill goes click-through for the action itself, so a click Aloe aims at the top of
    // the screen reaches the app beneath it rather than stopping Aloe.
    overlay::set_pill_interactive(app, false);
    let performed = on_input_thread(app, move || perform(&action, &job_input, window)).await;
    overlay::set_pill_interactive(app, true);
    let performed = performed?;
    let mut summary = match performed {
        Ok(summary) => summary,
        Err(ControlError::Failsafe) => {
            trip_failsafe(app);
            return Err(FAILSAFE_MESSAGE.to_string());
        }
        Err(ControlError::Failed(message)) => return Err(message),
    };

    tokio::time::sleep(Duration::from_millis(DESKTOP_CONTROL_SETTLE_MS)).await;
    // The action already happened, so a failed follow-up screenshot is reported, not raised —
    // raising would tell the model the click never landed.
    match capture_desktop_screenshot(app).await {
        Ok(Value::Object(screenshot)) => {
            if let Some(map) = summary.as_object_mut() {
                map.extend(screenshot);
            }
        }
        Ok(_) => {}
        Err(error) => {
            if let Some(map) = summary.as_object_mut() {
                map.insert("screenshotError".to_string(), Value::String(error));
            }
        }
    }
    Ok(summary)
}

fn perform(kind: &str, input: &Value, window: Option<AloeWindow>) -> Result<Value, ControlError> {
    let mut enigo = new_enigo()?;
    let geometry = display_geometry(&enigo)?;

    let pointer = enigo.location().map_err(input_error)?;
    let left_by_aloe = *LAST_POINTER.lock().expect("pointer mutex") == Some(pointer);
    if geometry.in_corner(pointer) && !left_by_aloe {
        return Err(ControlError::Failsafe);
    }

    let summary = match kind {
        "desktop_click" => click(&mut enigo, geometry, window, input)?,
        "desktop_move" => {
            let point = target(input, geometry, "x", "y")?;
            move_to(&mut enigo, point)?;
            json!({ "action": "move" })
        }
        "desktop_drag" => drag(&mut enigo, geometry, window, input)?,
        "desktop_scroll" => scroll(&mut enigo, geometry, window, input)?,
        "desktop_type" => type_text(&mut enigo, window, input)?,
        "desktop_key" => press_keys(&mut enigo, window, input)?,
        _ => return Err(format!("Unknown desktop control action: {kind}").into()),
    };

    *LAST_POINTER.lock().expect("pointer mutex") = enigo.location().ok();
    Ok(summary)
}

fn click(enigo: &mut Enigo, geometry: ScreenGeometry, window: Option<AloeWindow>, input: &Value) -> Result<Value, ControlError> {
    let point = target(input, geometry, "x", "y")?;
    guard_point(window, point)?;
    let (button, button_name) = match input.get("button").and_then(Value::as_str).unwrap_or("left") {
        "left" => (Button::Left, "left"),
        "right" => (Button::Right, "right"),
        "middle" => (Button::Middle, "middle"),
        other => return Err(format!("Unknown mouse button \"{other}\". Use left, right, or middle.").into()),
    };
    let clicks = input.get("clicks").and_then(Value::as_u64).unwrap_or(1).clamp(1, 3);

    move_to(enigo, point)?;
    for _ in 0..clicks {
        enigo.button(button, Direction::Click).map_err(input_error)?;
        thread::sleep(Duration::from_millis(40));
    }
    Ok(json!({ "action": "click", "button": button_name, "clicks": clicks }))
}

fn drag(enigo: &mut Enigo, geometry: ScreenGeometry, window: Option<AloeWindow>, input: &Value) -> Result<Value, ControlError> {
    let from = target(input, geometry, "startX", "startY")?;
    let to = target(input, geometry, "x", "y")?;
    guard_point(window, from)?;
    guard_point(window, to)?;

    move_to(enigo, from)?;
    enigo.button(Button::Left, Direction::Press).map_err(input_error)?;
    // Released no matter how the glide went, so a failure can't leave the button held down. The
    // glide moves in steps, which matters here too: many apps only start a drag after seeing motion.
    let glided = glide(enigo, to);
    let released = enigo.button(Button::Left, Direction::Release).map_err(input_error);
    glided?;
    released?;
    Ok(json!({ "action": "drag" }))
}

/// Pointer travel is timed by distance — a short hop is quick, a trip across the screen still
/// reads as one motion — and eased in and out, so the user can follow where Aloe is going.
const GLIDE_MIN_MS: f64 = 180.0;
const GLIDE_MAX_MS: f64 = 520.0;
const GLIDE_MS_PER_PX: f64 = 0.22;
/// Roughly one display frame per step.
const GLIDE_STEP: Duration = Duration::from_millis(8);
/// Further than this from where Aloe last put it, the pointer was moved by the user's own hand.
const USER_NUDGE_PX: i32 = 3;
const USER_TOOK_POINTER_MESSAGE: &str = "The user moved the mouse while Aloe was moving it, so the action \
    was not performed. Take a fresh screenshot and decide whether to continue.";

fn glide_duration(from: (i32, i32), to: (i32, i32)) -> f64 {
    let distance = ((to.0 - from.0) as f64).hypot((to.1 - from.1) as f64);
    (GLIDE_MIN_MS + distance * GLIDE_MS_PER_PX).min(GLIDE_MAX_MS)
}

fn ease_in_out_cubic(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// Glides the pointer from wherever it is to `to`. Paced by the clock rather than by counting
/// steps, so a coarse system timer makes the motion choppier, never slower.
fn glide(enigo: &mut Enigo, to: (i32, i32)) -> Result<(), ControlError> {
    let from = enigo.location().map_err(input_error)?;
    if from == to {
        return Ok(());
    }
    let total_ms = glide_duration(from, to);
    let started = std::time::Instant::now();
    let mut placed = from;
    loop {
        let t = (started.elapsed().as_secs_f64() * 1000.0 / total_ms).min(1.0);
        let k = ease_in_out_cubic(t);
        let point = (
            from.0 + ((to.0 - from.0) as f64 * k).round() as i32,
            from.1 + ((to.1 - from.1) as f64 * k).round() as i32,
        );
        if point != placed {
            enigo.move_mouse(point.0, point.1, Coordinate::Abs).map_err(input_error)?;
            placed = point;
        }
        if t >= 1.0 {
            return Ok(());
        }
        thread::sleep(GLIDE_STEP);
        let now = enigo.location().map_err(input_error)?;
        if (now.0 - placed.0).abs() > USER_NUDGE_PX || (now.1 - placed.1).abs() > USER_NUDGE_PX {
            return Err(USER_TOOK_POINTER_MESSAGE.to_string().into());
        }
    }
}

fn scroll(enigo: &mut Enigo, geometry: ScreenGeometry, window: Option<AloeWindow>, input: &Value) -> Result<Value, ControlError> {
    let point = target(input, geometry, "x", "y")?;
    guard_point(window, point)?;
    let amount = input.get("amount").and_then(Value::as_u64).unwrap_or(3).clamp(1, 25) as i32;
    let direction = input.get("direction").and_then(Value::as_str).unwrap_or("down");
    let (axis, length) = match direction {
        "down" => (Axis::Vertical, amount),
        "up" => (Axis::Vertical, -amount),
        "right" => (Axis::Horizontal, amount),
        "left" => (Axis::Horizontal, -amount),
        other => return Err(format!("Unknown scroll direction \"{other}\". Use up, down, left, or right.").into()),
    };

    move_to(enigo, point)?;
    enigo.scroll(length, axis).map_err(input_error)?;
    Ok(json!({ "action": "scroll", "direction": direction, "amount": amount }))
}

fn type_text(enigo: &mut Enigo, window: Option<AloeWindow>, input: &Value) -> Result<Value, ControlError> {
    guard_keyboard(window)?;
    let text = input_string(input, "text")?;
    let length = text.chars().count();
    if length > MAX_DESKTOP_TYPE_CHARS {
        return Err(format!("Text is {length} characters; type at most {MAX_DESKTOP_TYPE_CHARS} per call.").into());
    }
    enigo.text(&text).map_err(input_error)?;
    Ok(json!({ "action": "type", "characters": length }))
}

fn press_keys(enigo: &mut Enigo, window: Option<AloeWindow>, input: &Value) -> Result<Value, ControlError> {
    guard_keyboard(window)?;
    let combo = input_string(input, "keys")?;
    let keys = parse_combo(&combo)?;
    let Some((main, modifiers)) = keys.split_last() else {
        return Err(format!("No keys found in \"{combo}\".").into());
    };
    let repeat = input.get("repeat").and_then(Value::as_u64).unwrap_or(1).clamp(1, 20);

    let mut held = Vec::new();
    let mut outcome = Ok(());
    for modifier in modifiers {
        match enigo.key(modifier.clone(), Direction::Press) {
            Ok(()) => held.push(modifier.clone()),
            Err(error) => {
                outcome = Err(input_error(error));
                break;
            }
        }
    }
    if outcome.is_ok() {
        for _ in 0..repeat {
            if let Err(error) = enigo.key(main.clone(), Direction::Click) {
                outcome = Err(input_error(error));
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
    }
    // Released in reverse whatever happened, so a failed press can't leave Ctrl stuck down.
    for modifier in held.into_iter().rev() {
        let _ = enigo.key(modifier, Direction::Release);
    }
    outcome?;
    Ok(json!({ "action": "key", "keys": combo, "repeat": repeat }))
}

/// `"ctrl+shift+t"` → [Control, Shift, Unicode('t')]. The last key is pressed; the rest are held.
fn parse_combo(combo: &str) -> Result<Vec<Key>, String> {
    combo.split('+').map(str::trim).filter(|part| !part.is_empty()).map(parse_key).collect()
}

fn parse_key(name: &str) -> Result<Key, String> {
    let lower = name.to_lowercase();
    let key = match lower.as_str() {
        "ctrl" | "control" => Key::Control,
        "shift" => Key::Shift,
        "alt" | "option" => Key::Alt,
        "cmd" | "command" | "meta" | "super" | "win" | "windows" => Key::Meta,
        "enter" | "return" => Key::Return,
        "tab" => Key::Tab,
        "esc" | "escape" => Key::Escape,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "capslock" => Key::CapsLock,
        "plus" => Key::Unicode('+'),
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        _ => {
            let mut chars = lower.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::Unicode(c),
                _ => return Err(format!("Unknown key \"{name}\". Use names like ctrl, shift, alt, cmd, enter, tab, esc, up, f5, or a single character.")),
            }
        }
    };
    Ok(key)
}

fn target(input: &Value, geometry: ScreenGeometry, x_key: &str, y_key: &str) -> Result<(i32, i32), ControlError> {
    let x = number(input, x_key)?;
    let y = number(input, y_key)?;
    if !(0.0..=GRID).contains(&x) || !(0.0..=GRID).contains(&y) {
        return Err(format!(
            "({x}, {y}) is off the screen. Coordinates are 0–1000 on both axes: (0, 0) is the top-left of the screenshot, (1000, 1000) the bottom-right."
        )
        .into());
    }
    Ok(geometry.to_display(x, y))
}

fn number(input: &Value, key: &str) -> Result<f64, String> {
    input.get(key).and_then(Value::as_f64).ok_or_else(|| format!("{key} is required and must be a number."))
}

/// Glides there, then holds for a beat so the click (or scroll, or drag) reads as the end of the
/// motion rather than part of it — and so hover effects at the target have time to settle.
fn move_to(enigo: &mut Enigo, point: (i32, i32)) -> Result<(), ControlError> {
    glide(enigo, point)?;
    thread::sleep(Duration::from_millis(70));
    Ok(())
}

// ── Guards ───────────────────────────────────────────────────────────────────

/// Aloe Desktop's main window, in the same coordinate space as the pointer.
#[derive(Clone, Copy)]
struct AloeWindow {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    focused: bool,
}

fn aloe_window(app: &AppHandle) -> Option<AloeWindow> {
    let window = app.get_webview_window("main")?;
    if !window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(false) {
        return None;
    }
    let position = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    // Tauri reports physical pixels; enigo moves in points on macOS and in physical pixels elsewhere.
    let scale = if cfg!(target_os = "macos") { window.scale_factor().unwrap_or(1.0) } else { 1.0 };
    let to_input = |value: f64| (value / scale).round() as i32;
    Some(AloeWindow {
        x: to_input(position.x as f64),
        y: to_input(position.y as f64),
        w: to_input(size.width as f64),
        h: to_input(size.height as f64),
        focused: window.is_focused().unwrap_or(false),
    })
}

fn guard_point(window: Option<AloeWindow>, (x, y): (i32, i32)) -> Result<(), ControlError> {
    let inside = window.is_some_and(|w| x >= w.x && x < w.x + w.w && y >= w.y && y < w.y + w.h);
    if inside {
        return Err(OWN_WINDOW_MESSAGE.to_string().into());
    }
    Ok(())
}

fn guard_keyboard(window: Option<AloeWindow>) -> Result<(), ControlError> {
    if window.is_some_and(|w| w.focused) {
        return Err(OWN_FOCUS_MESSAGE.to_string().into());
    }
    Ok(())
}

/// Switches desktop control off and takes the overlay down. Shared by the corner failsafe, the
/// overlay's Stop button, and the settings toggle.
pub fn disable_desktop_control(app: &AppHandle) {
    {
        let state = app.state::<AppState>();
        let mut config = state.config.lock().expect("config mutex");
        config.desktop_control_enabled = false;
        let _ = save_config(&config);
    }
    overlay::close(app);
}

fn trip_failsafe(app: &AppHandle) {
    debug_log("desktop_control", "failsafe", "pointer parked in a display corner");
    disable_desktop_control(app);
    let _ = notifications::show_clickable(
        app,
        "Desktop control stopped",
        "You moved the pointer into a screen corner, so Aloe stopped controlling this computer. Turn it back on in Aloe Desktop when you're ready.",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_scales_a_large_display_screenshot_down() {
        let geometry = ScreenGeometry::from_display(3840, 2160);
        assert_eq!((geometry.shot_w, geometry.shot_h), (1280, 720));
    }

    #[test]
    fn grid_points_map_onto_the_whole_display() {
        let geometry = ScreenGeometry::from_display(1920, 1080);
        assert_eq!(geometry.to_display(500.0, 500.0), (960, 540));
        assert_eq!(geometry.to_display(0.0, 0.0), (0, 0));
        assert_eq!(geometry.to_display(1000.0, 1000.0), (1919, 1079));
        // The y=975 Gemini sent for the Windows taskbar lands on the taskbar, not off-screen.
        assert_eq!(geometry.to_display(440.0, 975.0), (845, 1053));
    }

    #[test]
    fn geometry_leaves_a_small_display_screenshot_alone() {
        let geometry = ScreenGeometry::from_display(1024, 768);
        assert_eq!((geometry.shot_w, geometry.shot_h), (1024, 768));
    }

    #[test]
    fn corners_trip_the_failsafe_but_edges_do_not() {
        let geometry = ScreenGeometry::from_display(1920, 1080);
        assert!(geometry.in_corner((0, 0)));
        assert!(geometry.in_corner((1919, 1079)));
        assert!(geometry.in_corner((1918, 1)));
        assert!(!geometry.in_corner((960, 0)));
        assert!(!geometry.in_corner((0, 540)));
    }

    /// Manual check: `cargo test capture_writes_a_real_screenshot -- --ignored` writes the capture to
    /// the temp dir, to eyeball what the model is actually shown.
    #[test]
    #[ignore]
    fn capture_writes_a_real_screenshot() {
        let geometry = new_enigo().and_then(|enigo| display_geometry(&enigo)).ok();
        let shot = capture(geometry).expect("capture");
        let bytes = BASE64.decode(shot["base64"].as_str().unwrap()).unwrap();
        let path = std::env::temp_dir().join("aloe-capture-check.jpg");
        std::fs::write(&path, bytes).unwrap();
        println!("wrote {} ({}x{})", path.display(), shot["width"], shot["height"]);
    }

    #[test]
    fn glides_take_longer_for_longer_trips_within_bounds() {
        assert_eq!(glide_duration((0, 0), (1, 0)), GLIDE_MIN_MS + GLIDE_MS_PER_PX);
        assert!(glide_duration((0, 0), (300, 400)) > glide_duration((0, 0), (30, 40)));
        assert_eq!(glide_duration((0, 0), (3840, 2160)), GLIDE_MAX_MS);
    }

    #[test]
    fn easing_starts_and_ends_at_rest() {
        assert_eq!(ease_in_out_cubic(0.0), 0.0);
        assert_eq!(ease_in_out_cubic(0.5), 0.5);
        assert_eq!(ease_in_out_cubic(1.0), 1.0);
        assert!(ease_in_out_cubic(0.1) < 0.1 && ease_in_out_cubic(0.9) > 0.9);
    }

    /// Manual check: `cargo test glide_lands_on_target -- --ignored` glides the real pointer
    /// across the screen and back. Hands off the mouse while it runs.
    #[test]
    #[ignore]
    fn glide_lands_on_target() {
        let mut enigo = new_enigo().unwrap();
        let geometry = display_geometry(&enigo).unwrap();
        let start = enigo.location().unwrap();
        let far = geometry.to_display(800.0, 700.0);
        let began = std::time::Instant::now();
        glide(&mut enigo, far).unwrap_or_else(|_| panic!("glide was interrupted"));
        println!("glide took {:?}", began.elapsed());
        assert_eq!(enigo.location().unwrap(), far);
        glide(&mut enigo, start).unwrap_or_else(|_| panic!("glide was interrupted"));
    }

    #[test]
    fn parses_key_combos() {
        assert!(matches!(parse_combo("ctrl+shift+t").unwrap().as_slice(), [Key::Control, Key::Shift, Key::Unicode('t')]));
        assert!(matches!(parse_combo("Enter").unwrap().as_slice(), [Key::Return]));
        assert!(matches!(parse_combo("cmd + plus").unwrap().as_slice(), [Key::Meta, Key::Unicode('+')]));
        assert!(parse_combo("ctrl+banana").is_err());
    }
}

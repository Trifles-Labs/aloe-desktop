//! The push-to-talk overlay: a glow around the display the pointer is on, and a panel at its
//! bottom centre with the live transcript and Aloe's reply. Both are windows of voice.html.
//!
//! The same rules as the desktop-control overlay (overlay.rs) apply, for the same reasons: created
//! unfocused and never `show()`n again (that would activate it on Windows and steal the keyboard
//! from the app the user is talking over), faded in and out by the page, content-protected so
//! Aloe's own screenshots never contain them, and fed by `eval` rather than app events.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::config::{debug_log, AppState};

const GLOW_LABEL: &str = "voice-glow";
const PANEL_LABEL: &str = "voice-panel";
/// Panel size and gap from the bottom of the display, in logical pixels. Tall enough for a few
/// lines of reply; the page fades older lines out at the top.
const PANEL_WIDTH: f64 = 560.0;
const PANEL_HEIGHT: f64 = 260.0;
const PANEL_BOTTOM: f64 = 96.0;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    #[default]
    Idle,
    Listening,
    Transcribing,
    Thinking,
    Speaking,
    /// Aloe asked something (a question, an approval) and is waiting for a spoken answer.
    Asking,
    Done,
    Error,
}

/// What Aloe is waiting on, shown large while it is read out.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PromptView {
    pub text: String,
    /// Numbered choices, empty for yes/no and free answers.
    pub options: Vec<String>,
    /// What to say, as a reminder: "Say yes or no", "Say the number".
    pub hint: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub phase: Phase,
    /// What the user said, live while listening.
    pub transcript: String,
    /// Aloe's reply so far, as markdown.
    pub reply: String,
    /// What Aloe is doing right now ("Searching the web…"), between bursts of text.
    pub status: Option<String>,
    pub error: Option<String>,
    pub prompt: Option<PromptView>,
    /// From the voice preferences, filled in by `show`.
    pub text_size: String,
    pub high_contrast: bool,
}

static VIEW: Mutex<Option<View>> = Mutex::new(None);

/// The current view, for a page that loaded after it was last pushed.
pub fn current() -> View {
    VIEW.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().unwrap_or_default()
}

pub fn show(app: &AppHandle, mut view: View) {
    {
        let config = app.state::<AppState>().config.lock().unwrap_or_else(std::sync::PoisonError::into_inner).voice.clone();
        view.text_size = config.text_size;
        view.high_contrast = config.high_contrast;
    }
    let json = serde_json::to_string(&view).unwrap_or_else(|_| "{}".to_string());
    let idle = view.phase == Phase::Idle;
    *VIEW.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(view);
    for label in [GLOW_LABEL, PANEL_LABEL] {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.eval(&format!("window.__aloeVoice?.({json})"));
        }
    }
    // A faded-out panel must not eat clicks meant for whatever is under it.
    if let Some(panel) = app.get_webview_window(PANEL_LABEL) {
        let _ = panel.set_ignore_cursor_events(idle);
    }
}

/// Microphone loudness, 0–1, for the glow and the panel's dot. Sent on its own, as one CSS
/// variable, so it never re-renders the text.
pub fn level(app: &AppHandle, level: f32) {
    let script = format!("window.__aloeVoiceLevel?.({:.3})", level.clamp(0.0, 1.0));
    for label in [GLOW_LABEL, PANEL_LABEL] {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.eval(&script);
        }
    }
}

/// Creates the windows on first use and moves them to the display the pointer is on. Blocking:
/// reading the pointer position waits on the event loop, so this must not run on it.
pub fn prepare(app: &AppHandle) {
    if let Err(error) = place_windows(app) {
        // The overlay is a courtesy: listening goes on without it.
        debug_log("voice", "overlay_error", error);
    }
}

/// The panel grows with the text size, within most of the display.
fn panel_size(text_size: &str, display_width: f64, display_height: f64) -> (f64, f64) {
    let factor = match text_size {
        "s" => 0.9,
        "l" => 1.25,
        "xl" => 1.55,
        _ => 1.0,
    };
    ((PANEL_WIDTH * factor).min(display_width * 0.9), (PANEL_HEIGHT * factor).min(display_height * 0.55))
}

fn place_windows(app: &AppHandle) -> Result<(), String> {
    let monitor = monitor_under_pointer(app)?;
    let scale = monitor.scale_factor();
    let x = monitor.position().x as f64 / scale;
    let y = monitor.position().y as f64 / scale;
    let width = monitor.size().width as f64 / scale;
    let height = monitor.size().height as f64 / scale;
    let text_size = app.state::<AppState>().config.lock().unwrap_or_else(std::sync::PoisonError::into_inner).voice.text_size.clone();
    let (panel_width, panel_height) = panel_size(&text_size, width, height);
    let panel_x = x + (width - panel_width) / 2.0;
    let panel_y = y + height - panel_height - PANEL_BOTTOM;

    match app.get_webview_window(GLOW_LABEL) {
        Some(glow) => reposition(&glow, x, y, width, height)?,
        None => {
            let glow = overlay_window(app, GLOW_LABEL, x, y, width, height)?;
            glow.set_ignore_cursor_events(true).map_err(|e| e.to_string())?;
        }
    }
    match app.get_webview_window(PANEL_LABEL) {
        Some(panel) => reposition(&panel, panel_x, panel_y, panel_width, panel_height)?,
        None => {
            let panel = overlay_window(app, PANEL_LABEL, panel_x, panel_y, panel_width, panel_height)?;
            panel.set_ignore_cursor_events(true).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn monitor_under_pointer(app: &AppHandle) -> Result<tauri::Monitor, String> {
    let primary = || app.primary_monitor().map_err(|e| e.to_string())?.ok_or_else(|| "No display.".to_string());
    let Ok(pointer) = app.cursor_position() else { return primary() };
    let monitors = app.available_monitors().map_err(|e| e.to_string())?;
    let found = monitors.into_iter().find(|monitor| {
        let (origin, size) = (monitor.position(), monitor.size());
        pointer.x >= origin.x as f64
            && pointer.y >= origin.y as f64
            && pointer.x < origin.x as f64 + size.width as f64
            && pointer.y < origin.y as f64 + size.height as f64
    });
    match found {
        Some(monitor) => Ok(monitor),
        None => primary(),
    }
}

/// Moving and resizing a window doesn't activate it, unlike showing one.
fn reposition(window: &WebviewWindow, x: f64, y: f64, width: f64, height: f64) -> Result<(), String> {
    window.set_position(tauri::LogicalPosition::new(x, y)).map_err(|e| e.to_string())?;
    window.set_size(tauri::LogicalSize::new(width, height)).map_err(|e| e.to_string())?;
    Ok(())
}

fn overlay_window(app: &AppHandle, label: &str, x: f64, y: f64, width: f64, height: f64) -> Result<WebviewWindow, String> {
    WebviewWindowBuilder::new(app, label, WebviewUrl::App("voice.html".into()))
        .title("Aloe is listening")
        .position(x, y)
        .inner_size(width, height)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .content_protected(true)
        .focused(false)
        .visible(true)
        .build()
        .map_err(|e| e.to_string())
}

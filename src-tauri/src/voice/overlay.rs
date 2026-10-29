//! The push-to-talk overlay: one window of voice.html over the whole display the pointer is on.
//! It draws a glow around the edge and an island that drops down from the top centre, with the
//! live transcript and Aloe's reply. While Aloe uses the computer during a voice turn, the island
//! also carries the desktop-control status and its Stop, in place of overlay.rs's own pill.
//!
//! The same rules as the desktop-control overlay (overlay.rs) apply, for the same reasons: created
//! unfocused and never `show()`n again (that would activate it on Windows and steal the keyboard
//! from the app the user is talking over), faded in and out by the page, content-protected so
//! Aloe's own screenshots never contain it, and fed by `eval` rather than app events.
//!
//! The window covers the display but is click-through everywhere except the island: the page
//! reports the island's rectangle, and a watcher lets the window take the cursor only while the
//! pointer is over it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::config::{debug_log, AppState};

const STAGE_LABEL: &str = "voice-stage";
/// How often the pointer is checked against the island while the overlay shows.
const HIT_INTERVAL: Duration = Duration::from_millis(30);

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
    /// Aloe is using the computer (overlay.rs), shown in the island while voice is up.
    pub control: bool,
}

/// The island's rectangle, in logical pixels relative to the stage.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where the stage sits, in physical pixels, so the pointer can be made relative to it.
#[derive(Debug, Clone, Copy)]
struct Stage {
    x: f64,
    y: f64,
    scale: f64,
}

static VIEW: Mutex<Option<View>> = Mutex::new(None);
static STAGE: Mutex<Option<Stage>> = Mutex::new(None);
static WATCHING: AtomicBool = AtomicBool::new(false);
static ISLAND: Mutex<Option<Rect>> = Mutex::new(None);
/// Cleared while Aloe itself clicks, so a synthetic click at the top of the screen reaches the
/// app beneath rather than the island.
static CLICKABLE: AtomicBool = AtomicBool::new(true);

/// The overlay is up: overlay.rs then leaves the desktop-control status to the island.
pub fn is_showing() -> bool {
    VIEW.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().is_some_and(|view| view.phase != Phase::Idle)
}

/// Desktop control started or stopped: re-render with the new status.
pub fn sync_control(app: &AppHandle) {
    if is_showing() {
        render(app, current());
    }
}

/// See `CLICKABLE`.
pub fn set_clickable(clickable: bool) {
    CLICKABLE.store(clickable, Ordering::SeqCst);
}

/// The page's island moved or resized; `None` once it has gone.
pub fn set_island(rect: Option<Rect>) {
    *ISLAND.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = rect;
}

/// The current view, for a page that loaded after it was last pushed.
pub fn current() -> View {
    VIEW.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().unwrap_or_default()
}

pub fn show(app: &AppHandle, view: View) {
    let was_showing = is_showing();
    render(app, view);
    if was_showing != is_showing() {
        // The island takes over (or hands back) the desktop-control pill.
        crate::overlay::sync_windows(app);
    }
}

fn render(app: &AppHandle, mut view: View) {
    view.control = crate::overlay::is_active();
    {
        let config = app.state::<AppState>().config.lock().unwrap_or_else(std::sync::PoisonError::into_inner).voice.clone();
        view.text_size = config.text_size;
        view.high_contrast = config.high_contrast;
    }
    let json = serde_json::to_string(&view).unwrap_or_else(|_| "{}".to_string());
    let idle = view.phase == Phase::Idle;
    *VIEW.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(view);
    if let Some(window) = app.get_webview_window(STAGE_LABEL) {
        let _ = window.eval(&format!("window.__aloeVoice?.({json})"));
    }
    if idle {
        WATCHING.store(false, Ordering::SeqCst);
        set_island(None);
        if let Some(window) = app.get_webview_window(STAGE_LABEL) {
            let _ = window.set_ignore_cursor_events(true);
        }
    } else if !WATCHING.swap(true, Ordering::SeqCst) {
        watch_pointer(app.clone());
    }
}

/// Microphone loudness, 0–1, for the glow and the companion. Sent on its own, as one number, so
/// it never re-renders the text.
pub fn level(app: &AppHandle, level: f32) {
    if let Some(window) = app.get_webview_window(STAGE_LABEL) {
        let _ = window.eval(&format!("window.__aloeVoiceLevel?.({:.3})", level.clamp(0.0, 1.0)));
    }
}

/// Lets the stage take the cursor only while the pointer is over the island, until the overlay
/// goes idle. A thread of its own: reading the pointer waits on the event loop, so it must not run
/// on it or on the async workers.
fn watch_pointer(app: AppHandle) {
    std::thread::spawn(move || {
        let mut taking = false;
        while WATCHING.load(Ordering::SeqCst) {
            let over = match (app.cursor_position(), *STAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner), *ISLAND.lock().unwrap_or_else(std::sync::PoisonError::into_inner)) {
                (Ok(pointer), Some(stage), Some(island)) => {
                    let x = (pointer.x - stage.x) / stage.scale;
                    let y = (pointer.y - stage.y) / stage.scale;
                    x >= island.x && y >= island.y && x < island.x + island.width && y < island.y + island.height
                }
                _ => false,
            };
            let take = over && CLICKABLE.load(Ordering::SeqCst);
            if take != taking {
                if let Some(window) = app.get_webview_window(STAGE_LABEL) {
                    if window.set_ignore_cursor_events(!take).is_ok() {
                        taking = take;
                    }
                }
            }
            std::thread::sleep(HIT_INTERVAL);
        }
    });
}

/// Creates the window on first use and moves it to the display the pointer is on. Blocking:
/// reading the pointer position waits on the event loop, so this must not run on it.
pub fn prepare(app: &AppHandle) {
    if let Err(error) = place_window(app) {
        // The overlay is a courtesy: listening goes on without it.
        debug_log("voice", "overlay_error", error);
    }
}

fn place_window(app: &AppHandle) -> Result<(), String> {
    let monitor = monitor_under_pointer(app)?;
    let scale = monitor.scale_factor();
    let (position, size) = (monitor.position(), monitor.size());
    *STAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
        Some(Stage { x: position.x as f64, y: position.y as f64, scale });
    let x = position.x as f64 / scale;
    let y = position.y as f64 / scale;
    let width = size.width as f64 / scale;
    let height = size.height as f64 / scale;

    match app.get_webview_window(STAGE_LABEL) {
        Some(stage) => reposition(&stage, x, y, width, height),
        None => {
            let stage = overlay_window(app, x, y, width, height)?;
            stage.set_ignore_cursor_events(true).map_err(|e| e.to_string())
        }
    }
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

fn overlay_window(app: &AppHandle, x: f64, y: f64, width: f64, height: f64) -> Result<WebviewWindow, String> {
    WebviewWindowBuilder::new(app, STAGE_LABEL, WebviewUrl::App("voice.html".into()))
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

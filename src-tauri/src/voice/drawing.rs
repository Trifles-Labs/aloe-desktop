//! Marks Aloe draws over the screen during a voice reply: highlights, circles, arrows and labels on
//! top of whatever is showing (the backend's draw_on_screen tool, run as a `draw_on_screen` job).
//!
//! They go on the voice overlay's page, over the primary display, because that is the display the
//! voice turn's screenshot shows, and the marks use that screenshot's 0–1000 grid. The backend has
//! already checked and clamped every mark; this keeps the list and hands it to the page, which
//! draws text with textContent only.
//!
//! Marks fade with the voice overlay, once the answer they were drawn for is over (overlay.rs
//! `show`), or sooner when the user starts talking again, presses Esc or the ✕, or Aloe clears
//! them. Drawn with no voice overlay up (computer use from a typed chat), they fade after
//! UNATTENDED_FOR instead.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use super::overlay;
use crate::config::debug_log;

/// The most marks kept at once, however many "add"s pile up.
const MAX_MARKS: usize = 200;
/// How long marks stay when no voice answer is on screen to fade with.
const UNATTENDED_FOR: Duration = Duration::from_secs(30);
/// Bumped by every change, so a fade timer only clears the marks it was started for.
static VERSION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Drawing {
    pub marks: Vec<Value>,
    /// Marks before this index were already on screen; only the rest animate in.
    pub fresh_from: usize,
}

static DRAWING: Mutex<Drawing> = Mutex::new(Drawing { marks: Vec::new(), fresh_from: 0 });

/// What a freshly loaded page should draw, all of it already in place.
pub fn current() -> Drawing {
    let drawing = DRAWING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Drawing { marks: drawing.marks.clone(), fresh_from: drawing.marks.len() }
}

pub fn is_showing() -> bool {
    !DRAWING.lock().unwrap_or_else(std::sync::PoisonError::into_inner).marks.is_empty()
}

/// Runs one draw_on_screen job: `{ action: "draw" | "add" | "clear", items }`.
pub async fn apply(app: &AppHandle, input: &Value) -> Result<Value, String> {
    let action = input.get("action").and_then(Value::as_str).unwrap_or("draw");
    if action == "clear" {
        clear(app);
        return Ok(json!({ "cleared": true }));
    }
    let items = input.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
    if items.is_empty() {
        return Err("Nothing to draw.".to_string());
    }
    // On the display the screenshot showed, which may not be the one the pointer is on.
    let placing = {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || overlay::prepare_primary(&app))
    };
    placing.await.map_err(|e| e.to_string())?;

    let drawing = {
        let mut drawing = DRAWING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if action != "add" {
            drawing.marks.clear();
        }
        drawing.fresh_from = drawing.marks.len();
        drawing.marks.extend(items);
        let over = drawing.marks.len().saturating_sub(MAX_MARKS);
        drawing.marks.drain(..over);
        drawing.fresh_from = drawing.fresh_from.saturating_sub(over);
        drawing.clone()
    };
    debug_log("voice", "drawing", format!("action={action} marks={}", drawing.marks.len()));
    if cfg!(debug_assertions) {
        // Where each mark landed, to compare with the locate_shot images.
        debug_log("voice", "drawing_marks", serde_json::to_string(&drawing.marks).unwrap_or_default());
    }
    let version = VERSION.fetch_add(1, Ordering::SeqCst) + 1;
    push(app, &drawing);
    if !overlay::is_showing() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(UNATTENDED_FOR).await;
            if VERSION.load(Ordering::SeqCst) == version && !overlay::is_showing() {
                clear(&app);
            }
        });
    }
    Ok(json!({ "shown": true, "marks": drawing.marks.len() }))
}

/// Takes every mark away. Cheap when there are none.
pub fn clear(app: &AppHandle) {
    {
        let mut drawing = DRAWING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if drawing.marks.is_empty() {
            return;
        }
        *drawing = Drawing::default();
    }
    VERSION.fetch_add(1, Ordering::SeqCst);
    push(app, &Drawing::default());
}

fn push(app: &AppHandle, drawing: &Drawing) {
    overlay::drawing_changed(app, !drawing.marks.is_empty());
    if let Some(window) = app.get_webview_window(overlay::STAGE_LABEL) {
        let json = serde_json::to_string(drawing).unwrap_or_else(|_| "{}".to_string());
        let _ = window.eval(&format!("window.__aloeDraw?.({json})"));
    }
}

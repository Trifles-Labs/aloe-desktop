//! Widgets voice puts on the screen and leaves there: timers, stopwatches, clocks and notes (the
//! backend's screen_widgets tool, run as a `screen_widgets` job).
//!
//! Each widget is its own small always-on-top window (widget.html), so the user can drag it
//! anywhere, onto any display, and clicks on it never need the voice stage's pointer watcher. Like
//! the voice stage, a widget window is created unfocused and never `show()`n or focused again, so
//! one appearing never takes the keyboard from the app the user is talking over.
//!
//! The widgets live here, not in the page: they are saved in config.json with where they were last
//! put, and come back on the next launch. Timers and stopwatches are kept as wall-clock times, so
//! they stay right while a window is closed, throttled or the app is not running at all, and a
//! timer that ran out while Aloe was closed rings when it comes back.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::config::{debug_log, save_config, AppState};

pub const LABEL_PREFIX: &str = "widget-";
/// The most widgets kept at once.
const MAX_WIDGETS: usize = 8;
const MAX_TIMER_MS: i64 = 24 * 60 * 60 * 1000;
/// Gap between a widget and the screen's edge, and between stacked widgets, in logical pixels.
const MARGIN: f64 = 24.0;
/// The size a widget opens at, in logical pixels; the page then fits the height to what it shows.
const WIDTH: f64 = 232.0;
const HEIGHT: f64 = 104.0;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Item {
    pub text: String,
    pub done: bool,
}

/// One widget. One flat shape for every kind, so config.json and the page read it the same way;
/// each kind uses its own fields and leaves the rest at their defaults.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Widget {
    pub id: String,
    /// "timer", "stopwatch", "clock" or "note".
    pub kind: String,
    pub label: String,
    /// The window's top-left, in physical screen pixels, once it has been placed.
    pub x: Option<i32>,
    pub y: Option<i32>,
    /// Timer: its full length.
    pub total_ms: i64,
    /// Timer: when it runs out (Unix ms), while running.
    pub ends_at_ms: Option<i64>,
    /// Timer: what was left when it was paused.
    pub remaining_ms: i64,
    /// Timer: it ran out and has rung.
    pub rang: bool,
    /// Stopwatch: when it would have started had it never been paused (Unix ms), while running.
    pub started_at_ms: Option<i64>,
    /// Stopwatch: the time counted while paused.
    pub elapsed_ms: i64,
    /// Clock: IANA zones; empty for the computer's own.
    pub zones: Vec<String>,
    /// Note.
    pub text: String,
    pub items: Vec<Item>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Widget {
    fn label_name(&self) -> String {
        format!("{LABEL_PREFIX}{}", self.id)
    }

    fn timer_left(&self, now: i64) -> i64 {
        match self.ends_at_ms {
            Some(ends) => (ends - now).max(0),
            None => self.remaining_ms.max(0),
        }
    }

    fn stopwatch_elapsed(&self, now: i64) -> i64 {
        match self.started_at_ms {
            Some(started) => (now - started).max(0),
            None => self.elapsed_ms,
        }
    }

    /// What the model is told about it: no positions, times in whole seconds.
    fn summary(&self, now: i64) -> Value {
        let mut out = json!({ "id": self.id, "kind": self.kind });
        if !self.label.is_empty() {
            out["label"] = json!(self.label);
        }
        match self.kind.as_str() {
            "timer" => {
                let left = self.timer_left(now);
                out["secondsLeft"] = json!((left + 999) / 1000);
                out["totalSeconds"] = json!(self.total_ms / 1000);
                out["state"] = json!(if left == 0 { "finished" } else if self.ends_at_ms.is_some() { "running" } else { "paused" });
            }
            "stopwatch" => {
                out["elapsedSeconds"] = json!(self.stopwatch_elapsed(now) / 1000);
                out["state"] = json!(if self.started_at_ms.is_some() { "running" } else { "paused" });
            }
            "clock" => out["zones"] = json!(self.zones),
            "note" => {
                if !self.text.is_empty() {
                    out["text"] = json!(self.text);
                }
                if !self.items.is_empty() {
                    out["items"] = json!(self.items);
                }
            }
            _ => {}
        }
        out
    }

    fn pause(&mut self, now: i64) {
        match self.kind.as_str() {
            "timer" if self.ends_at_ms.is_some() => {
                self.remaining_ms = self.timer_left(now);
                self.ends_at_ms = None;
            }
            "stopwatch" if self.started_at_ms.is_some() => {
                self.elapsed_ms = self.stopwatch_elapsed(now);
                self.started_at_ms = None;
            }
            _ => {}
        }
    }

    fn resume(&mut self, now: i64) {
        match self.kind.as_str() {
            "timer" if self.ends_at_ms.is_none() && self.remaining_ms > 0 => {
                self.ends_at_ms = Some(now + self.remaining_ms);
            }
            "stopwatch" if self.started_at_ms.is_none() => {
                self.started_at_ms = Some(now - self.elapsed_ms);
            }
            _ => {}
        }
    }

    /// Back to the start, stopped: a timer full again, a stopwatch at zero.
    fn reset(&mut self) {
        match self.kind.as_str() {
            "timer" => {
                self.ends_at_ms = None;
                self.remaining_ms = self.total_ms;
                self.rang = false;
            }
            "stopwatch" => {
                self.started_at_ms = None;
                self.elapsed_ms = 0;
            }
            _ => {}
        }
    }

    fn add_time(&mut self, now: i64, delta_ms: i64) {
        if self.kind != "timer" {
            return;
        }
        let left = (self.timer_left(now) + delta_ms).clamp(0, MAX_TIMER_MS);
        self.total_ms = (self.total_ms + delta_ms).clamp(left, MAX_TIMER_MS);
        if left > 0 {
            self.rang = false;
        }
        // A finished timer given more time starts running again.
        if self.ends_at_ms.is_some() || (left > 0 && self.remaining_ms == 0) {
            self.ends_at_ms = Some(now + left);
        } else {
            self.remaining_ms = left;
        }
    }
}

fn str_field<'a>(input: &'a Value, key: &str) -> &'a str {
    input.get(key).and_then(Value::as_str).unwrap_or("")
}

fn items_field(input: &Value) -> Vec<Item> {
    input
        .get("items")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(|item| serde_json::from_value::<Item>(item.clone()).ok()).filter(|item| !item.text.is_empty()).take(20).collect())
        .unwrap_or_default()
}

/// A new widget from an `open` job. The backend has already checked and capped it.
fn build(input: &Value, now: i64) -> Result<Widget, String> {
    let kind = str_field(input, "kind");
    let mut widget = Widget {
        id: uuid::Uuid::new_v4().simple().to_string()[..6].to_string(),
        kind: kind.to_string(),
        label: str_field(input, "label").chars().take(40).collect(),
        ..Widget::default()
    };
    match kind {
        "timer" => {
            let seconds = input.get("seconds").and_then(Value::as_i64).unwrap_or(0);
            if seconds <= 0 {
                return Err("A timer needs seconds.".into());
            }
            widget.total_ms = (seconds * 1000).min(MAX_TIMER_MS);
            widget.ends_at_ms = Some(now + widget.total_ms);
        }
        "stopwatch" => widget.started_at_ms = Some(now),
        "clock" => {
            widget.zones = input
                .get("zones")
                .and_then(Value::as_array)
                .map(|zones| zones.iter().filter_map(Value::as_str).map(str::to_string).take(4).collect())
                .unwrap_or_default();
        }
        "note" => {
            widget.text = str_field(input, "text").chars().take(500).collect();
            widget.items = items_field(input);
        }
        other => return Err(format!("Unknown widget kind: {other}")),
    }
    Ok(widget)
}

/// Applies one `update` to a widget. `Ok(Some(position))` asks for the window to move there.
fn update(widget: &mut Widget, input: &Value, now: i64) -> Result<Option<String>, String> {
    match str_field(input, "op") {
        "pause" => widget.pause(now),
        "resume" => widget.resume(now),
        "reset" => widget.reset(),
        "add_seconds" => {
            if widget.kind != "timer" {
                return Err("Only a timer takes more time.".into());
            }
            widget.add_time(now, input.get("seconds").and_then(Value::as_i64).unwrap_or(0) * 1000);
        }
        "label" => widget.label = str_field(input, "label").chars().take(40).collect(),
        "text" => widget.text = str_field(input, "text").chars().take(500).collect(),
        "items" => widget.items = items_field(input),
        "check" => {
            let index = input.get("index").and_then(Value::as_u64).unwrap_or(u64::MAX) as usize;
            let done = input.get("done").and_then(Value::as_bool).unwrap_or(true);
            let item = widget.items.get_mut(index).ok_or_else(|| format!("That note has no item {index}."))?;
            item.done = done;
        }
        "move" => return Ok(Some(str_field(input, "position").to_string())),
        other => return Err(format!("Unknown op: {other}")),
    }
    Ok(None)
}

fn with_widgets<T>(app: &AppHandle, change: impl FnOnce(&mut Vec<Widget>) -> T) -> T {
    let state = app.state::<AppState>();
    let mut config = state.config.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = change(&mut config.widgets);
    if let Err(error) = save_config(&config) {
        debug_log("widgets", "save_failed", error);
    }
    result
}

fn all(app: &AppHandle) -> Vec<Widget> {
    app.state::<AppState>().config.lock().unwrap_or_else(std::sync::PoisonError::into_inner).widgets.clone()
}

fn find(app: &AppHandle, id: &str) -> Option<Widget> {
    all(app).into_iter().find(|widget| widget.id == id)
}

fn listing(app: &AppHandle) -> Value {
    let now = now_ms();
    json!({ "widgets": all(app).iter().map(|widget| widget.summary(now)).collect::<Vec<_>>() })
}

/// Tells a widget's page what it now holds.
fn push(app: &AppHandle, widget: &Widget) {
    let _ = app.emit_to(widget.label_name(), "widget-update", widget);
}

fn close_window(app: &AppHandle, id: &str) {
    if let Some(window) = app.get_webview_window(&format!("{LABEL_PREFIX}{id}")) {
        let _ = window.destroy();
    }
}

/// Runs one screen_widgets job: `{ action: "open" | "update" | "close" | "list", ... }`.
pub async fn apply(app: &AppHandle, input: &Value) -> Result<Value, String> {
    let now = now_ms();
    match str_field(input, "action") {
        "list" => Ok(listing(app)),
        "open" => {
            if all(app).len() >= MAX_WIDGETS {
                return Err(format!("There are already {MAX_WIDGETS} widgets on the screen. Close one first."));
            }
            let widget = build(input, now)?;
            let position = input.get("position").and_then(Value::as_str).map(str::to_string);
            with_widgets(app, |widgets| widgets.push(widget.clone()));
            if let Err(error) = open_window(app, &widget, position.as_deref()) {
                close(app, &widget.id);
                return Err(error);
            }
            debug_log("widgets", "opened", &widget.kind);
            Ok(json!({ "opened": widget.summary(now) }))
        }
        "update" => {
            let id = str_field(input, "id");
            let updated = with_widgets(app, |widgets| {
                let widget = widgets.iter_mut().find(|widget| widget.id == id).ok_or_else(|| format!("No widget {id} on the screen. List them to find its id."))?;
                let moved = update(widget, input, now)?;
                Ok::<_, String>((widget.clone(), moved))
            })?;
            let (widget, moved) = updated;
            if let Some(position) = moved {
                place(app, &widget, &position)?;
            }
            push(app, &widget);
            Ok(json!({ "updated": widget.summary(now) }))
        }
        "close" => {
            let closing: Vec<String> = if input.get("all").and_then(Value::as_bool) == Some(true) {
                all(app).into_iter().map(|widget| widget.id).collect()
            } else {
                let id = str_field(input, "id");
                if find(app, id).is_none() {
                    return Err(format!("No widget {id} on the screen. List them to find its id."));
                }
                vec![id.to_string()]
            };
            for id in &closing {
                close(app, id);
            }
            Ok(json!({ "closed": closing.len() }))
        }
        other => Err(format!("Unknown screen_widgets action: {other}")),
    }
}

/// Takes a widget away, from the job or its own ✕.
pub fn close(app: &AppHandle, id: &str) {
    with_widgets(app, |widgets| widgets.retain(|widget| widget.id != id));
    close_window(app, id);
}

/// The window is closing on its own (Alt+F4): forget the widget, and let the window go.
pub fn forget(app: &AppHandle, id: &str) {
    with_widgets(app, |widgets| widgets.retain(|widget| widget.id != id));
}

/// What a widget's freshly loaded page should show; `None` once it has been closed.
pub fn state(app: &AppHandle, id: &str) -> Option<Widget> {
    find(app, id)
}

/// A click on the widget itself: pause, resume, reset, tick an item, edit a note.
pub fn act(app: &AppHandle, id: &str, input: &Value) -> Result<Widget, String> {
    let now = now_ms();
    let widget = with_widgets(app, |widgets| {
        let widget = widgets.iter_mut().find(|widget| widget.id == id).ok_or_else(|| "That widget has been closed.".to_string())?;
        update(widget, input, now)?;
        Ok::<_, String>(widget.clone())
    })?;
    push(app, &widget);
    Ok(widget)
}

/// Saving positions is held back until the user has stopped dragging for a moment.
static MOVE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// A widget window moved: by the user's drag, or by `place`.
pub fn moved(app: &AppHandle, label: &str, position: PhysicalPosition<i32>) {
    let Some(id) = label.strip_prefix(LABEL_PREFIX) else { return };
    {
        let state = app.state::<AppState>();
        let mut config = state.config.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(widget) = config.widgets.iter_mut().find(|widget| widget.id == id) else { return };
        widget.x = Some(position.x);
        widget.y = Some(position.y);
    }
    let generation = MOVE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if MOVE_GENERATION.load(Ordering::SeqCst) == generation {
            with_widgets(&app, |_| ());
        }
    });
}

fn open_window(app: &AppHandle, widget: &Widget, position: Option<&str>) -> Result<(), String> {
    let label = widget.label_name();
    if app.get_webview_window(&label).is_some() {
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(format!("widget.html?id={}", widget.id).into()))
        .title(if widget.label.is_empty() { "Aloe widget" } else { &widget.label })
        .inner_size(WIDTH, HEIGHT)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(true)
        .build()
        .map_err(|e| e.to_string())?;
    let saved = match (widget.x, widget.y) {
        (Some(x), Some(y)) if on_a_display(app, x, y) => Some(PhysicalPosition::new(x, y)),
        _ => None,
    };
    match saved {
        Some(at) => window.set_position(at).map_err(|e| e.to_string()),
        None => place(app, widget, position.unwrap_or("")),
    }
}

/// Whether a saved top-left still lands on a connected display (one may have been unplugged).
fn on_a_display(app: &AppHandle, x: i32, y: i32) -> bool {
    app.available_monitors().unwrap_or_default().iter().any(|monitor| {
        let (origin, size) = (monitor.position(), monitor.size());
        x >= origin.x && y >= origin.y && x < origin.x + size.width as i32 - 40 && y < origin.y + size.height as i32 - 40
    })
}

/// Moves a widget to a named spot ("top_left", "center", "bottom_right"...) on the display it is
/// on. Anything else stacks it down the top-right of the primary display, below the others.
fn place(app: &AppHandle, widget: &Widget, position: &str) -> Result<(), String> {
    let window = app.get_webview_window(&widget.label_name()).ok_or_else(|| "The widget's window has gone.".to_string())?;
    let monitor = match window.current_monitor().ok().flatten() {
        Some(monitor) if !position.is_empty() => monitor,
        _ => app.primary_monitor().map_err(|e| e.to_string())?.ok_or_else(|| "No display.".to_string())?,
    };
    let scale = monitor.scale_factor();
    let origin = monitor.position();
    let area = monitor.size();
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let margin = (MARGIN * scale) as i32;
    let (left, top) = (origin.x + margin, origin.y + margin);
    let right = origin.x + area.width as i32 - size.width as i32 - margin;
    let bottom = origin.y + area.height as i32 - size.height as i32 - margin;
    let (mid_x, mid_y) = ((left + right) / 2, (top + bottom) / 2);
    let at = match position {
        "top_left" => (left, top),
        "top" => (mid_x, top),
        "top_right" => (right, top),
        "left" => (left, mid_y),
        "center" => (mid_x, mid_y),
        "right" => (right, mid_y),
        "bottom_left" => (left, bottom),
        "bottom" => (mid_x, bottom),
        "bottom_right" => (right, bottom),
        _ => {
            // Below the island's drop-down, under the widgets already stacked there.
            let stacked = all(app).iter().filter(|other| other.id != widget.id).count() as i32;
            let step = (HEIGHT * scale) as i32 + margin / 2;
            (right, top + (64.0 * scale) as i32 + stacked * step)
        }
    };
    window.set_position(PhysicalPosition::new(at.0, at.1.min(bottom).max(top))).map_err(|e| e.to_string())
}

/// Brings back every saved widget, and starts the watch that rings timers.
pub fn start(app: &AppHandle) {
    for widget in all(app) {
        if let Err(error) = open_window(app, &widget, None) {
            debug_log("widgets", "restore_failed", error);
        }
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tick.tick().await;
            ring_due(&app);
        }
    });
}

/// Rings every timer that has run out and not rung yet: the page chimes and pulses, and a system
/// notification says so in case the widget is hidden behind something or the user is away.
fn ring_due(app: &AppHandle) {
    let now = now_ms();
    let due: Vec<Widget> = {
        let state = app.state::<AppState>();
        let config = state.config.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        config.widgets.iter().filter(|widget| widget.kind == "timer" && !widget.rang && widget.ends_at_ms.is_some_and(|ends| ends <= now)).cloned().collect()
    };
    if due.is_empty() {
        return;
    }
    let rung = with_widgets(app, |widgets| {
        widgets
            .iter_mut()
            .filter(|widget| due.iter().any(|done| done.id == widget.id))
            .map(|widget| {
                widget.rang = true;
                widget.ends_at_ms = None;
                widget.remaining_ms = 0;
                widget.clone()
            })
            .collect::<Vec<_>>()
    });
    for widget in rung {
        push(app, &widget);
        let _ = app.emit_to(widget.label_name(), "widget-ring", ());
        let title = if widget.label.is_empty() { "Timer done".to_string() } else { format!("{} – timer done", widget.label) };
        if let Err(error) = crate::notifications::show_clickable(app, &title, "Your timer has finished.") {
            debug_log("widgets", "notify_failed", error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer(seconds: i64, now: i64) -> Widget {
        build(&json!({ "kind": "timer", "seconds": seconds }), now).unwrap()
    }

    #[test]
    fn a_timer_counts_down_from_when_it_opened() {
        let widget = timer(60, 1_000);
        assert_eq!(widget.timer_left(31_000), 30_000);
        assert_eq!(widget.timer_left(99_000), 0);
        assert_eq!(widget.summary(31_000)["state"], "running");
        assert_eq!(widget.summary(99_000)["state"], "finished");
    }

    #[test]
    fn pausing_a_timer_keeps_what_was_left() {
        let mut widget = timer(60, 0);
        widget.pause(20_000);
        assert_eq!(widget.timer_left(500_000), 40_000);
        widget.resume(500_000);
        assert_eq!(widget.timer_left(510_000), 30_000);
    }

    #[test]
    fn more_time_restarts_a_finished_timer() {
        let mut widget = timer(10, 0);
        widget.rang = true;
        widget.ends_at_ms = None;
        widget.remaining_ms = 0;
        widget.add_time(50_000, 60_000);
        assert!(!widget.rang);
        assert_eq!(widget.timer_left(50_000), 60_000);
        assert_eq!(widget.summary(50_000)["state"], "running");
    }

    #[test]
    fn taking_time_off_never_goes_below_zero() {
        let mut widget = timer(30, 0);
        widget.add_time(0, -120_000);
        assert_eq!(widget.timer_left(0), 0);
    }

    #[test]
    fn a_stopwatch_pauses_and_resumes() {
        let mut widget = build(&json!({ "kind": "stopwatch" }), 0).unwrap();
        widget.pause(5_000);
        assert_eq!(widget.stopwatch_elapsed(60_000), 5_000);
        widget.resume(60_000);
        assert_eq!(widget.stopwatch_elapsed(62_000), 7_000);
        widget.reset();
        assert_eq!(widget.stopwatch_elapsed(90_000), 0);
    }

    #[test]
    fn a_note_ticks_items_and_refuses_ones_it_does_not_have() {
        let mut widget = build(&json!({ "kind": "note", "items": [{ "text": "milk" }, { "text": "" }, { "text": "eggs" }] }), 0).unwrap();
        assert_eq!(widget.items.len(), 2);
        update(&mut widget, &json!({ "op": "check", "index": 1 }), 0).unwrap();
        assert!(widget.items[1].done);
        assert!(update(&mut widget, &json!({ "op": "check", "index": 5 }), 0).is_err());
    }

    #[test]
    fn moving_is_left_to_the_window() {
        let mut widget = timer(60, 0);
        assert_eq!(update(&mut widget, &json!({ "op": "move", "position": "bottom_left" }), 0).unwrap(), Some("bottom_left".to_string()));
    }

    #[test]
    fn saved_widgets_read_back() {
        let widget = timer(60, 0);
        let text = serde_json::to_string(&widget).unwrap();
        assert_eq!(serde_json::from_str::<Widget>(&text).unwrap(), widget);
    }
}

//! The on-screen sign that Aloe is driving the mouse and keyboard: a glow around the primary
//! display plus a small "Aloe is using your computer · Stop" pill at its top centre.
//!
//! Both are windows of overlay.html, created on the first control action and kept alive (just
//! faded out) until desktop control is switched off. Showing and hiding is done in the page, not
//! with `show()`/`hide()`: re-showing a native window activates it on Windows, and an overlay that
//! steals keyboard focus would swallow the very text Aloe is about to type.
//!
//! Both windows are content-protected, which keeps them out of Aloe's own screenshots on Windows
//! and macOS, so the model never sees (or tries to click) its own indicator.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::config::{debug_log, DESKTOP_CONTROL_OVERLAY_IDLE_SECONDS};

const GLOW_LABEL: &str = "control-glow";
const PILL_LABEL: &str = "control-pill";
const OVERLAY_EVENT: &str = "desktop-control://active";
/// Pill size and offset from the top of the display, in logical pixels.
const PILL_WIDTH: f64 = 300.0;
const PILL_HEIGHT: f64 = 40.0;
const PILL_TOP: f64 = 12.0;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static WATCHER_RUNNING: AtomicBool = AtomicBool::new(false);
static LAST_ACTIVITY: Mutex<Option<Instant>> = Mutex::new(None);

pub fn is_active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

/// Called on every control action, screenshots included. Shows the overlay and pushes its idle
/// fade-out back. Returns true when this call just started a run and moved Aloe's own window out of
/// the way, so the caller can let the desktop repaint before it looks at or touches the screen.
pub fn mark_active(app: &AppHandle) -> bool {
    *LAST_ACTIVITY.lock().expect("overlay mutex") = Some(Instant::now());
    let starting = !is_active();
    let cleared = starting && minimize_main_window(app);
    if let Err(error) = ensure_windows(app) {
        // The overlay is a courtesy, never a gate: a failure here must not stop the action.
        debug_log("overlay", "create_error", error);
        return cleared;
    }
    set_active(app, true);
    spawn_idle_watcher(app);
    cleared
}

/// The run is over (computer_use finished): fade out now rather than after the idle timeout, and
/// bring Aloe's window back so the user sees the reply.
pub fn release(app: &AppHandle) {
    *LAST_ACTIVITY.lock().expect("overlay mutex") = None;
    set_active(app, false);
}

/// Desktop control was switched off (toggle, Stop, or the corner failsafe): fade out, then close.
pub fn close(app: &AppHandle) {
    set_active(app, false);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Long enough for the 250ms fade in overlay.html to finish.
        tokio::time::sleep(Duration::from_millis(300)).await;
        if is_active() {
            return;
        }
        for label in [GLOW_LABEL, PILL_LABEL] {
            if let Some(window) = app.get_webview_window(label) {
                let _ = window.destroy();
            }
        }
    });
}

/// The pill is the one clickable part. While Aloe itself clicks, it goes click-through too, so a
/// synthetic click at the top of the screen lands on the app underneath instead of on Stop.
pub fn set_pill_interactive(app: &AppHandle, interactive: bool) {
    if let Some(pill) = app.get_webview_window(PILL_LABEL) {
        let _ = pill.set_ignore_cursor_events(!(interactive && is_active()));
    }
}

fn set_active(app: &AppHandle, active: bool) {
    ACTIVE.store(active, Ordering::SeqCst);
    // Pushed straight into each overlay page rather than as an app event: an event a page can't
    // listen for (capability scoping) left the glow up after a run ended, since the page's
    // one-time state query could turn it on but nothing could ever turn it off.
    for label in [GLOW_LABEL, PILL_LABEL] {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.eval(&format!("window.__aloeOverlaySetActive?.({active})"));
        }
    }
    let _ = app.emit(OVERLAY_EVENT, active);
    // An invisible pill must not eat clicks meant for whatever is beneath it.
    set_pill_interactive(app, active);
    if !active {
        restore_main_window(app);
    }
}

/// Set only when Aloe itself minimized the window, so a window the user minimized stays down.
static MINIMIZED_FOR_RUN: AtomicBool = AtomicBool::new(false);

/// Aloe's own window covered most of the screen and held focus — the user is usually chatting in
/// it — so the model spent most of its first live run bouncing off the own-window guard. Getting it
/// out of the way for the run lets the model see and use the desktop behind it.
fn minimize_main_window(app: &AppHandle) -> bool {
    let Some(main) = app.get_webview_window("main") else { return false };
    if !main.is_visible().unwrap_or(false) || main.is_minimized().unwrap_or(false) {
        return false;
    }
    if main.minimize().is_err() {
        return false;
    }
    MINIMIZED_FOR_RUN.store(true, Ordering::SeqCst);
    true
}

/// Un-minimizing alone is not enough: the run usually leaves another app in the foreground, and
/// on Windows a restored window then sits behind it (or just flashes in the taskbar). Showing and
/// focusing it puts the reply in front of the user.
fn restore_main_window(app: &AppHandle) {
    if !MINIMIZED_FOR_RUN.swap(false, Ordering::SeqCst) {
        return;
    }
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.unminimize();
        let _ = main.show();
        if let Err(error) = main.set_focus() {
            debug_log("overlay", "restore_focus_error", error.to_string());
        }
    }
}

/// Fades the overlay once Aloe has been quiet for a while. The model thinks for a few seconds
/// between actions, so this is deliberately longer than that gap — a glow that blinks off and on
/// between every step reads as broken.
fn spawn_idle_watcher(app: &AppHandle) {
    if WATCHER_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if !is_active() || is_idle() {
                break;
            }
        }
        WATCHER_RUNNING.store(false, Ordering::SeqCst);
        // An action that landed while this was winding down saw a watcher still running and did
        // not start its own, so pick it up here rather than fading out under it.
        if is_active() && !is_idle() {
            spawn_idle_watcher(&app);
            return;
        }
        set_active(&app, false);
    });
}

fn is_idle() -> bool {
    let idle = Duration::from_secs(DESKTOP_CONTROL_OVERLAY_IDLE_SECONDS);
    LAST_ACTIVITY.lock().expect("overlay mutex").map_or(true, |at| at.elapsed() >= idle)
}

fn ensure_windows(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window(GLOW_LABEL).is_some() && app.get_webview_window(PILL_LABEL).is_some() {
        return Ok(());
    }
    let monitor = app
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No primary display.".to_string())?;
    let scale = monitor.scale_factor();
    let x = monitor.position().x as f64 / scale;
    let y = monitor.position().y as f64 / scale;
    let width = monitor.size().width as f64 / scale;
    let height = monitor.size().height as f64 / scale;

    if app.get_webview_window(GLOW_LABEL).is_none() {
        let glow = overlay_window(app, GLOW_LABEL, x, y, width, height)?;
        glow.set_ignore_cursor_events(true).map_err(|e| e.to_string())?;
    }
    if app.get_webview_window(PILL_LABEL).is_none() {
        let pill = overlay_window(app, PILL_LABEL, x + (width - PILL_WIDTH) / 2.0, y + PILL_TOP, PILL_WIDTH, PILL_HEIGHT)?;
        pill.set_ignore_cursor_events(true).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Created visible but unfocused — `focused(false)` at creation shows the window without
/// activating it, which a later `show()` would not do. The page starts fully transparent.
fn overlay_window(app: &AppHandle, label: &str, x: f64, y: f64, width: f64, height: f64) -> Result<WebviewWindow, String> {
    WebviewWindowBuilder::new(app, label, WebviewUrl::App("overlay.html".into()))
        .title("Aloe is using your computer")
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

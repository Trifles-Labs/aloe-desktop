//! One-step jobs: the things someone asks their computer for in a few words — "open Notepad",
//! "snap this left", "volume to thirty", "lock my computer", "type this" — done here directly
//! instead of by a desktop-control run that looks at the screen and clicks.
//!
//! Three jobs, each with a fixed list of actions and plain values, so none of them can be made to
//! start an arbitrary program or run a command:
//!  - `quick_app`: open an installed app by its Start-menu name, or act on a window. Behind the
//!    desktop-control switch, like everything else that acts on the screen, and never on Aloe
//!    Desktop's own windows.
//!  - `system_control`: volume, brightness, dark mode, lock, sleep, a screenshot saved to Pictures,
//!    and a status report. A fixed action and a number: no switch, no approval, like the media keys.
//!  - `quick_type`: type or paste text where the cursor is (behind the desktop-control switch, and
//!    refused while Aloe's own window has the keyboard), copy text to the clipboard, or read the
//!    selection or the clipboard back.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use tauri::AppHandle;

use crate::computer::{quick_keys, quick_text, require_control, save_screenshot};
use crate::config::{debug_log, MAX_DESKTOP_TYPE_CHARS};
use crate::desk::{self, Show, StartApp, WindowInfo};

fn text_field<'a>(input: &'a Value, key: &str) -> &'a str {
    input.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn action_of(input: &Value) -> String {
    text_field(input, "action").trim().to_lowercase()
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| e.to_string())
}

// ── Apps and windows ─────────────────────────────────────────────────────────

/// The Start menu's apps take a second to list, and don't change from one request to the next.
const START_APPS_TTL: Duration = Duration::from_secs(300);
static START_APPS: Mutex<Option<(Instant, Vec<StartApp>)>> = Mutex::new(None);

async fn start_apps() -> Result<Vec<StartApp>, String> {
    if let Some((at, apps)) = START_APPS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
        if at.elapsed() < START_APPS_TTL {
            return Ok(apps.clone());
        }
    }
    let apps = tokio::time::timeout(Duration::from_secs(10), blocking(desk::start_apps_blocking))
        .await
        .map_err(|_| "Listing the installed apps took too long.".to_string())???;
    *START_APPS.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((Instant::now(), apps.clone()));
    Ok(apps)
}

/// Starts an app the Start menu lists, by the id Windows gave for it. The id goes to the shell as
/// one argument; it is never built from what the job said.
#[cfg(target_os = "windows")]
async fn launch(app: &StartApp) -> Result<(), String> {
    let mut command = tokio::process::Command::new("explorer.exe");
    command.arg(format!("shell:AppsFolder\\{}", app.id));
    crate::shell::hide_command_window(&mut command);
    // explorer.exe hands the request on and exits non-zero even when the app opens.
    command.status().await.map(drop).map_err(|e| format!("Couldn't open {}: {e}", app.name))
}

#[cfg(not(target_os = "windows"))]
async fn launch(_app: &StartApp) -> Result<(), String> {
    Err("Aloe Desktop can only open apps this way on Windows for now. Use computer_use instead.".to_string())
}

/// The window an action is about: the one named, or the one in front when nothing was.
fn target_window(target: &str) -> Result<WindowInfo, String> {
    if target.is_empty() {
        return match desk::foreground() {
            Some(window) if !window.own => Ok(window),
            Some(_) => Err("Aloe Desktop's own window is in front, which Aloe leaves alone. Say which app or window you mean.".to_string()),
            None => Err("No window is in front. Say which app or window you mean.".to_string()),
        };
    }
    let windows = desk::open_windows();
    desk::find_window(&windows, target).cloned().ok_or_else(|| {
        let open: Vec<String> = windows.iter().take(8).map(|window| window.app.clone()).filter(|app| !app.is_empty()).collect();
        format!("No open window matches \"{target}\". Open right now: {}.", if open.is_empty() { "nothing".to_string() } else { open.join(", ") })
    })
}

/// Runs one `quick_app` job: `{ action, target? }`.
pub async fn quick_app(app: &AppHandle, input: &Value) -> Result<Value, String> {
    let action = action_of(input);
    let target: String = text_field(input, "target").trim().chars().take(120).collect();
    if !matches!(action.as_str(), "open" | "switch" | "minimize" | "maximize" | "restore" | "close" | "snap_left" | "snap_right" | "show_desktop") {
        return Err("action must be open, switch, minimize, maximize, restore, close, snap_left, snap_right or show_desktop.".to_string());
    }
    if matches!(action.as_str(), "open" | "switch") && target.is_empty() {
        return Err(format!("{action} needs a target: the app's name."));
    }
    require_control(app).await?;
    debug_log("quick", "app", format!("{action} target_chars={}", target.chars().count()));

    if action == "show_desktop" {
        quick_keys(app, "win+d", false).await?;
        return Ok(json!({ "action": action, "done": true }));
    }
    if action == "open" {
        // Already running: bring it forward rather than start a second copy.
        let running = {
            let target = target.clone();
            blocking(move || desk::find_window(&desk::open_windows(), &target).cloned()).await?
        };
        if let Some(window) = running {
            let handle = window.handle;
            blocking(move || desk::focus(handle)).await??;
            return Ok(json!({ "action": action, "alreadyOpen": true, "window": window.to_json() }));
        }
        let apps = start_apps().await?;
        let Some(found) = desk::find_start_app(&apps, &target).cloned() else {
            return Err(format!("No installed app is called \"{target}\". If it is a website, open it in the browser instead."));
        };
        launch(&found).await?;
        return Ok(json!({ "action": action, "opened": found.name }));
    }

    let outcome = blocking(move || -> Result<Value, String> {
        let window = target_window(&target)?;
        match action.as_str() {
            "switch" => desk::focus(window.handle)?,
            "minimize" => desk::show(window.handle, Show::Minimize)?,
            "maximize" => {
                desk::show(window.handle, Show::Maximize)?;
                let _ = desk::focus(window.handle);
            }
            "restore" => {
                desk::show(window.handle, Show::Restore)?;
                let _ = desk::focus(window.handle);
            }
            "close" => desk::close(window.handle)?,
            "snap_left" | "snap_right" => {
                desk::snap(window.handle, action == "snap_left")?;
                let _ = desk::focus(window.handle);
            }
            _ => unreachable!("checked above"),
        }
        Ok(json!({ "action": action, "done": true, "window": window.to_json() }))
    })
    .await??;
    Ok(outcome)
}

// ── The computer itself ──────────────────────────────────────────────────────

fn percent(input: &Value) -> Result<u8, String> {
    let value = input.get("value").and_then(Value::as_f64).ok_or("value is required: a number from 0 to 100.")?;
    if !(0.0..=100.0).contains(&value) {
        return Err("value must be from 0 to 100.".to_string());
    }
    Ok(value.round() as u8)
}

/// A fixed PowerShell or reg line with nothing in it but a number this module formatted.
#[cfg(target_os = "windows")]
async fn run_fixed(program: &str, args: &[&str]) -> Result<bool, String> {
    let mut command = tokio::process::Command::new(program);
    command.args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    crate::shell::hide_command_window(&mut command);
    let status = tokio::time::timeout(Duration::from_secs(10), command.status()).await.map_err(|_| "That took too long.".to_string())?.map_err(|e| e.to_string())?;
    Ok(status.success())
}

#[cfg(target_os = "windows")]
async fn set_brightness(value: u8) -> Result<(), String> {
    let script = format!("(Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods -ErrorAction Stop | Invoke-CimMethod -MethodName WmiSetBrightness -Arguments @{{Timeout=1;Brightness={value}}} -ErrorAction Stop) | Out-Null");
    if run_fixed("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script]).await? {
        Ok(())
    } else {
        Err("This screen's brightness can't be set from the computer (usual for a desktop monitor). They can use the monitor's own buttons.".to_string())
    }
}

#[cfg(target_os = "windows")]
async fn set_dark_mode(dark: bool) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE};
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    let light = if dark { "0" } else { "1" };
    for name in ["AppsUseLightTheme", "SystemUsesLightTheme"] {
        if !run_fixed("reg", &["add", KEY, "/v", name, "/t", "REG_DWORD", "/d", light, "/f"]).await? {
            return Err("Couldn't change the theme setting.".to_string());
        }
    }
    // Tells running apps the colours changed, so they repaint without a sign-out.
    blocking(|| {
        let what: Vec<u16> = "ImmersiveColorSet\0".encode_utf16().collect();
        unsafe { SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, 0, what.as_ptr() as isize, SMTO_ABORTIFHUNG, 200, std::ptr::null_mut()) };
    })
    .await
}

#[cfg(target_os = "windows")]
fn lock_screen() -> Result<(), String> {
    if unsafe { windows_sys::Win32::System::Shutdown::LockWorkStation() } == 0 {
        return Err("Windows wouldn't lock the screen.".to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn sleep_now() {
    unsafe { windows_sys::Win32::System::Power::SetSuspendState(0, 0, 0) };
}

#[cfg(not(target_os = "windows"))]
const WINDOWS_ONLY: &str = "Aloe Desktop can only do that on Windows for now. Use computer_use instead.";
#[cfg(not(target_os = "windows"))]
async fn set_brightness(_value: u8) -> Result<(), String> {
    Err(WINDOWS_ONLY.to_string())
}
#[cfg(not(target_os = "windows"))]
async fn set_dark_mode(_dark: bool) -> Result<(), String> {
    Err(WINDOWS_ONLY.to_string())
}
#[cfg(not(target_os = "windows"))]
fn lock_screen() -> Result<(), String> {
    Err(WINDOWS_ONLY.to_string())
}
#[cfg(not(target_os = "windows"))]
fn sleep_now() {}

/// Runs one `system_control` job: `{ action, value?, on? }`.
pub async fn system_control(app: &AppHandle, input: &Value) -> Result<Value, String> {
    let action = action_of(input);
    debug_log("quick", "system", &action);
    match action.as_str() {
        "status" => Ok(desk::snapshot(false, false).await),
        "set_volume" => {
            let value = percent(input)?;
            blocking(move || desk::set_volume(value)).await??;
            Ok(json!({ "action": action, "volume": value }))
        }
        "set_brightness" => {
            let value = percent(input)?;
            set_brightness(value).await?;
            Ok(json!({ "action": action, "brightness": value }))
        }
        "dark_mode" => {
            let dark = input.get("on").and_then(Value::as_bool).ok_or("on is required: true for dark, false for light.")?;
            set_dark_mode(dark).await?;
            Ok(json!({ "action": action, "dark": dark }))
        }
        "lock" => {
            blocking(lock_screen).await??;
            Ok(json!({ "action": action, "locked": true }))
        }
        "sleep" => {
            if cfg!(not(target_os = "windows")) {
                return Err("Aloe Desktop can only do that on Windows for now. Use computer_use instead.".to_string());
            }
            // After the answer has gone back and been spoken: asleep, nothing more is sent.
            tauri::async_runtime::spawn(async {
                tokio::time::sleep(Duration::from_secs(6)).await;
                let _ = blocking(sleep_now).await;
            });
            Ok(json!({ "action": action, "sleepingInSeconds": 6 }))
        }
        "save_screenshot" => save_screenshot(app).await,
        _ => Err("action must be status, set_volume, set_brightness, dark_mode, lock, sleep or save_screenshot.".to_string()),
    }
}

// ── Typing and the clipboard ─────────────────────────────────────────────────

/// The most `paste` puts down in one go; `type` keeps desktop control's own, smaller limit.
const MAX_PASTE_CHARS: usize = 20_000;

/// Runs one `quick_type` job: `{ action, text?, enter? }`.
pub async fn quick_type(app: &AppHandle, input: &Value) -> Result<Value, String> {
    let action = action_of(input);
    let text = text_field(input, "text").to_string();
    let enter = input.get("enter").and_then(Value::as_bool).unwrap_or(false);
    let length = text.chars().count();
    debug_log("quick", "type", format!("{action} chars={length} enter={enter}"));
    match action.as_str() {
        "type" => {
            if text.is_empty() {
                return Err("text is required: what to type.".to_string());
            }
            if length > MAX_DESKTOP_TYPE_CHARS {
                return Err(format!("Text is {length} characters; type at most {MAX_DESKTOP_TYPE_CHARS}, or use paste."));
            }
            require_control(app).await?;
            let typed = quick_text(app, text, enter).await?;
            Ok(json!({ "action": action, "characters": typed, "enter": enter }))
        }
        "paste" => {
            if text.is_empty() {
                return Err("text is required: what to paste.".to_string());
            }
            if length > MAX_PASTE_CHARS {
                return Err(format!("Text is {length} characters; paste at most {MAX_PASTE_CHARS} at a time."));
            }
            require_control(app).await?;
            // Their clipboard is borrowed for the paste and handed back afterwards.
            let before = blocking(desk::clipboard_text).await?;
            blocking(move || desk::set_clipboard_text(&text)).await??;
            let pasted = quick_keys(app, if cfg!(target_os = "macos") { "cmd+v" } else { "ctrl+v" }, true).await;
            // The app reads the clipboard a moment after the keys, so it is put back a beat later.
            tokio::time::sleep(Duration::from_millis(300)).await;
            if let Some(before) = before {
                let _ = blocking(move || desk::set_clipboard_text(&before)).await;
            }
            pasted?;
            if enter {
                quick_keys(app, "enter", true).await?;
            }
            Ok(json!({ "action": action, "characters": length, "enter": enter }))
        }
        "copy" => {
            if text.is_empty() {
                return Err("text is required: what to copy.".to_string());
            }
            blocking(move || desk::set_clipboard_text(&text)).await??;
            Ok(json!({ "action": action, "characters": length }))
        }
        "read_selection" => match desk::selected_text().await {
            Some(selected) => Ok(json!({ "text": desk::clip(&selected, desk::MAX_SELECTION_CHARS) })),
            None => Err("Nothing is selected, or that app doesn't share its selection. Ask them to copy it, then read the clipboard.".to_string()),
        },
        "read_clipboard" => match blocking(desk::clipboard_text).await? {
            Some(copied) => Ok(json!({ "text": desk::clip(&copied, desk::MAX_CLIPBOARD_CHARS) })),
            None => Err("There is no text on the clipboard.".to_string()),
        },
        _ => Err("action must be type, paste, copy, read_selection or read_clipboard.".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_percentage_is_a_number_from_0_to_100() {
        assert_eq!(percent(&json!({ "value": 30 })).unwrap(), 30);
        assert_eq!(percent(&json!({ "value": 29.6 })).unwrap(), 30);
        assert!(percent(&json!({ "value": 130 })).is_err());
        assert!(percent(&json!({ "value": -1 })).is_err());
        assert!(percent(&json!({ "value": "30; calc.exe" })).is_err());
        assert!(percent(&json!({})).is_err());
    }
}

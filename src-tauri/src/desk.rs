//! What is on the computer right now: the app in front, the other windows, the volume, the battery,
//! the text that is selected and what is on the clipboard. And the handful of things that can be
//! done to a window without looking at the screen: bring it forward, minimise, maximise, close,
//! put it on one half of the display.
//!
//! Two callers. A push-to-talk turn sends a `snapshot` along with what was said, so Aloe knows what
//! "this" and "that app" are (voice/mod.rs). And the one-step jobs in quick_actions.rs read and move
//! windows through here instead of driving the screen.
//!
//! Windows is the whole of it. Elsewhere only the clipboard is read: asking macOS which app is in
//! front means an Automation prompt the user never asked for.

use std::time::Duration;

use serde_json::{json, Map, Value};

/// How much of each piece leaves this module. The backend clips again (voice_reply.ts).
const MAX_TITLE_CHARS: usize = 200;
const MAX_WINDOWS: usize = 12;
pub const MAX_SELECTION_CHARS: usize = 4_000;
const MAX_SNAPSHOT_CLIPBOARD_CHARS: usize = 2_000;
pub const MAX_CLIPBOARD_CHARS: usize = 20_000;

/// A top-level window. `handle` is only ever one this module read off the system itself.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowInfo {
    pub handle: isize,
    /// The program's name, from its executable: "chrome", "Code", "Spotify".
    pub app: String,
    pub title: String,
    /// One of Aloe Desktop's own windows, which nothing here acts on.
    pub own: bool,
}

impl WindowInfo {
    pub fn to_json(&self) -> Value {
        json!({ "app": clip(&self.app, 80), "title": clip(&self.title, MAX_TITLE_CHARS) })
    }
}

pub fn clip(text: &str, max: usize) -> String {
    let mut clipped: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        clipped.push('…');
    }
    clipped
}

/// `C:\Program Files\Google\Chrome\Application\chrome.exe` → `chrome`.
pub fn app_label(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file);
    stem.to_string()
}

/// What people call an app → the name its window goes by here.
fn app_alias(spoken: &str) -> &str {
    match spoken {
        "vs code" | "vscode" | "visual studio code" => "code",
        "file explorer" | "files" | "windows explorer" => "explorer",
        "edge" | "microsoft edge" => "msedge",
        "word" | "microsoft word" => "winword",
        "powerpoint" => "powerpnt",
        "terminal" | "windows terminal" => "windowsterminal",
        "google chrome" => "chrome",
        "task manager" => "taskmgr",
        other => other,
    }
}

fn normalized(text: &str) -> String {
    text.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The window someone means by a name or a few words of its title. Best match first: the app's own
/// name, then a name that contains (or is contained in) what was said, then the title.
pub fn find_window<'a>(windows: &'a [WindowInfo], target: &str) -> Option<&'a WindowInfo> {
    let said = normalized(target);
    if said.is_empty() {
        return None;
    }
    let alias = app_alias(&said);
    let app = |window: &WindowInfo| window.app.to_lowercase();
    let title = |window: &WindowInfo| window.title.to_lowercase();
    let candidates = || windows.iter().filter(|window| !window.own);
    candidates()
        .find(|window| app(window) == alias || app(window) == said)
        .or_else(|| candidates().find(|window| app(window).len() >= 3 && (app(window).contains(alias) || alias.contains(&app(window)))))
        .or_else(|| candidates().find(|window| title(window).contains(&said)))
        .or_else(|| {
            let words: Vec<&str> = said.split(' ').collect();
            candidates().find(|window| {
                let haystack = format!("{} {}", app(window), title(window));
                words.iter().all(|word| haystack.contains(word))
            })
        })
}

/// An installed app from the Start menu: what it is called and the id the shell opens it by.
#[derive(Clone, Debug, PartialEq)]
pub struct StartApp {
    pub name: String,
    pub id: String,
}

/// The installed app someone means by name: exact, then a name that starts with it, then one that
/// contains it. Nothing looser, so "note" never opens something that merely mentions notes.
pub fn find_start_app<'a>(apps: &'a [StartApp], target: &str) -> Option<&'a StartApp> {
    let said = normalized(target);
    let said = match said.as_str() {
        "vs code" | "vscode" => "visual studio code".to_string(),
        "edge" => "microsoft edge".to_string(),
        "explorer" | "files" => "file explorer".to_string(),
        _ => said,
    };
    if said.is_empty() {
        return None;
    }
    let name = |app: &StartApp| normalized(&app.name);
    apps.iter()
        .find(|app| name(app) == said)
        .or_else(|| apps.iter().filter(|app| said.len() >= 4 && name(app).starts_with(&said)).min_by_key(|app| app.name.len()))
        .or_else(|| apps.iter().filter(|app| name(app).split(' ').any(|word| word == said)).min_by_key(|app| app.name.len()))
        .or_else(|| apps.iter().filter(|app| said.len() >= 4 && name(app).contains(&said)).min_by_key(|app| app.name.len()))
}

/// Start-menu entries that aren't apps to open: uninstallers, links to websites and manuals.
pub fn is_launchable(app: &StartApp) -> bool {
    let name = app.name.to_lowercase();
    let id = app.id.to_lowercase();
    !(name.is_empty()
        || id.is_empty()
        || name.starts_with("uninstall")
        || id.starts_with("http")
        || id.ends_with(".url")
        || id.ends_with(".chm")
        || id.ends_with(".txt")
        || id.ends_with(".html"))
}

pub fn clipboard_text() -> Option<String> {
    let text = arboard::Clipboard::new().ok()?.get_text().ok()?;
    (!text.trim().is_empty()).then_some(text)
}

pub fn set_clipboard_text(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text.to_string()))
        .map_err(|e| format!("Couldn't reach the clipboard: {e}"))
}

/// The text selected in whatever has the keyboard, if that app shares it. Waits at most a second:
/// an app that has stopped responding must not hold up the caller.
pub async fn selected_text() -> Option<String> {
    let task = tauri::async_runtime::spawn_blocking(platform::selected_text);
    tokio::time::timeout(Duration::from_millis(1_000), task).await.ok()?.ok()?
}

/// Everything in the module's first paragraph, as the `deviceContext` a spoken turn sends. Each
/// part is best-effort and simply missing when it couldn't be read. `selection` and `clipboard` are
/// the user's two switches (Settings › Voice): off means that part is never read at all.
pub async fn snapshot(selection: bool, clipboard: bool) -> Value {
    let facts = tauri::async_runtime::spawn_blocking(move || {
        let mut out = Map::new();
        if let Some(window) = platform::foreground() {
            out.insert("focused".to_string(), if window.own { json!({ "app": "Aloe Desktop" }) } else { window.to_json() });
        }
        let windows: Vec<Value> = platform::open_windows().iter().take(MAX_WINDOWS).map(WindowInfo::to_json).collect();
        if !windows.is_empty() {
            out.insert("windows".to_string(), Value::Array(windows));
        }
        if let Some((percent, muted)) = platform::volume() {
            out.insert("volume".to_string(), json!(percent));
            out.insert("muted".to_string(), json!(muted));
        }
        if let Some((percent, charging)) = platform::battery() {
            out.insert("battery".to_string(), json!({ "percent": percent, "charging": charging }));
        }
        if let Some(playing) = crate::media::now_playing_blocking() {
            out.insert("nowPlaying".to_string(), playing);
        }
        if clipboard {
            if let Some(text) = clipboard_text() {
                out.insert("clipboard".to_string(), json!(clip(&text, MAX_SNAPSHOT_CLIPBOARD_CHARS)));
            }
        }
        out
    });
    // Apart from the rest: reading a selection asks another app, the one part that can be slow.
    let selected = async {
        if selection {
            selected_text().await
        } else {
            None
        }
    };
    let (facts, selected) = tokio::join!(tokio::time::timeout(Duration::from_millis(1_500), facts), selected);
    let mut out = facts.ok().and_then(Result::ok).unwrap_or_default();
    if let Some(text) = selected {
        out.insert("selectedText".to_string(), json!(clip(&text, MAX_SELECTION_CHARS)));
    }
    Value::Object(out)
}

pub use platform::{battery, close, foreground, focus, open_windows, set_mute, set_volume, show, snap, start_apps_blocking, volume, Show};

#[cfg(target_os = "windows")]
mod platform {
    use std::ptr::{null, null_mut};

    use windows::Win32::{
        Foundation::BOOL as WinBool,
        Media::Audio::{eConsole, eRender, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator, MMDeviceEnumerator},
        System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED},
        UI::Accessibility::{CUIAutomation, IUIAutomation, IUIAutomationTextPattern, UIA_TextPatternId},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, BOOL, HWND, LPARAM, RECT},
        Graphics::{
            Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED},
            Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST},
        },
        System::{
            Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
            Threading::{AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION},
        },
        UI::WindowsAndMessaging::{
            BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindow, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
            IsIconic, IsWindow, IsWindowVisible, PostMessageW, SetForegroundWindow, SetWindowPos, ShowWindow, GWL_EXSTYLE, GW_OWNER, SWP_NOACTIVATE,
            SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, WM_CLOSE, WS_EX_TOOLWINDOW,
        },
    };

    use super::{app_label, StartApp, WindowInfo, MAX_SELECTION_CHARS};

    fn title_of(hwnd: HWND) -> String {
        unsafe {
            let length = GetWindowTextLengthW(hwnd);
            if length <= 0 {
                return String::new();
            }
            let mut buffer = vec![0u16; length as usize + 1];
            let copied = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
            String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
        }
    }

    fn pid_of(hwnd: HWND) -> u32 {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        pid
    }

    fn app_of(pid: u32) -> String {
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return String::new();
            }
            let mut buffer = [0u16; 1024];
            let mut size = buffer.len() as u32;
            let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut size);
            CloseHandle(process);
            if ok == 0 {
                return String::new();
            }
            app_label(&String::from_utf16_lossy(&buffer[..size as usize]))
        }
    }

    fn info(hwnd: HWND) -> WindowInfo {
        let pid = pid_of(hwnd);
        WindowInfo { handle: hwnd as isize, app: app_of(pid), title: title_of(hwnd), own: pid == std::process::id() }
    }

    /// Windows keeps some windows "visible" that are on no screen (a suspended Store app, another
    /// virtual desktop): DWM calls those cloaked.
    fn cloaked(hwnd: HWND) -> bool {
        let mut value = 0u32;
        let result = unsafe { DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, &mut value as *mut u32 as *mut _, std::mem::size_of::<u32>() as u32) };
        result == 0 && value != 0
    }

    pub fn foreground() -> Option<WindowInfo> {
        let hwnd = unsafe { GetForegroundWindow() };
        (!hwnd.is_null()).then(|| info(hwnd))
    }

    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = &mut *(lparam as *mut Vec<HWND>);
        list.push(hwnd);
        1
    }

    /// The windows a person would say are open, front to back: visible, titled, on the taskbar, and
    /// not Aloe's own.
    pub fn open_windows() -> Vec<WindowInfo> {
        let mut handles: Vec<HWND> = Vec::new();
        unsafe { EnumWindows(Some(collect), &mut handles as *mut Vec<HWND> as LPARAM) };
        handles
            .into_iter()
            .filter(|&hwnd| unsafe {
                IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() && (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW) == 0
            })
            .filter(|&hwnd| !cloaked(hwnd))
            .map(info)
            .filter(|window| !window.own && !window.title.trim().is_empty() && window.title != "Program Manager")
            .collect()
    }

    /// A handle from `open_windows` or `foreground`, checked again: the window may have closed.
    fn live(handle: isize) -> Result<HWND, String> {
        let hwnd = handle as HWND;
        if unsafe { IsWindow(hwnd) } == 0 {
            return Err("That window has closed.".to_string());
        }
        if pid_of(hwnd) == std::process::id() {
            return Err("That is Aloe Desktop's own window, which Aloe leaves alone.".to_string());
        }
        Ok(hwnd)
    }

    /// Brings a window to the front. Windows only lets the foreground app hand the foreground on, so
    /// this joins that app's input queue for the moment it takes.
    pub fn focus(handle: isize) -> Result<(), String> {
        let hwnd = live(handle)?;
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            let front = GetForegroundWindow();
            let front_thread = if front.is_null() { 0 } else { GetWindowThreadProcessId(front, null_mut()) };
            let this_thread = GetCurrentThreadId();
            let attached = front_thread != 0 && front_thread != this_thread && AttachThreadInput(this_thread, front_thread, 1) != 0;
            BringWindowToTop(hwnd);
            let moved = SetForegroundWindow(hwnd) != 0;
            if attached {
                AttachThreadInput(this_thread, front_thread, 0);
            }
            if moved || GetForegroundWindow() == hwnd {
                Ok(())
            } else {
                Err("Windows wouldn't bring that window to the front. Use computer_use to click it.".to_string())
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Show {
        Minimize,
        Maximize,
        Restore,
    }

    pub fn show(handle: isize, how: Show) -> Result<(), String> {
        let hwnd = live(handle)?;
        let command = match how {
            Show::Minimize => SW_MINIMIZE,
            Show::Maximize => SW_MAXIMIZE,
            Show::Restore => SW_RESTORE,
        };
        unsafe { ShowWindow(hwnd, command) };
        Ok(())
    }

    /// Asks the window to close, as its ✕ would: the app may still ask about unsaved work.
    pub fn close(handle: isize) -> Result<(), String> {
        let hwnd = live(handle)?;
        if unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) } == 0 {
            return Err("That window didn't accept the request to close.".to_string());
        }
        Ok(())
    }

    /// Puts the window on the left or right half of the display it is on.
    pub fn snap(handle: isize, left: bool) -> Result<(), String> {
        let hwnd = live(handle)?;
        unsafe {
            ShowWindow(hwnd, SW_RESTORE);
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut details: MONITORINFO = std::mem::zeroed();
            details.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut details) == 0 {
                return Err("Couldn't read the display's size.".to_string());
            }
            let RECT { left: x0, top: y0, right: x1, bottom: y1 } = details.rcWork;
            let half = (x1 - x0) / 2;
            let x = if left { x0 } else { x0 + half };
            if SetWindowPos(hwnd, null_mut(), x, y0, half, y1 - y0, SWP_NOZORDER | SWP_NOACTIVATE) == 0 {
                return Err("That window can't be moved.".to_string());
            }
        }
        Ok(())
    }

    fn endpoint() -> Option<IAudioEndpointVolume> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let device = devices.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
            device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
        }
    }

    /// The speakers' volume, 0–100, and whether they are muted.
    pub fn volume() -> Option<(u8, bool)> {
        let endpoint = endpoint()?;
        unsafe {
            let level = endpoint.GetMasterVolumeLevelScalar().ok()?;
            let muted = endpoint.GetMute().ok()?.as_bool();
            Some(((level * 100.0).round().clamp(0.0, 100.0) as u8, muted))
        }
    }

    /// Sets the volume, and unmutes unless it was set to nothing.
    pub fn set_volume(percent: u8) -> Result<(), String> {
        let endpoint = endpoint().ok_or("Couldn't reach this computer's speakers.")?;
        unsafe {
            endpoint.SetMasterVolumeLevelScalar(f32::from(percent.min(100)) / 100.0, null()).map_err(|e| format!("Couldn't set the volume: {e}"))?;
            if percent > 0 {
                let _ = endpoint.SetMute(WinBool::from(false), null());
            }
        }
        Ok(())
    }

    pub fn set_mute(muted: bool) -> Result<(), String> {
        let endpoint = endpoint().ok_or("Couldn't reach this computer's speakers.")?;
        unsafe { endpoint.SetMute(WinBool::from(muted), null()) }.map_err(|e| format!("Couldn't change mute: {e}"))
    }

    /// Battery percent and whether it is plugged in; `None` on a computer without a battery.
    pub fn battery() -> Option<(u8, bool)> {
        let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
        if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
            return None;
        }
        // 128: no system battery. 255: unknown.
        if status.BatteryFlag & 128 != 0 || status.BatteryFlag == 255 || status.BatteryLifePercent > 100 {
            return None;
        }
        Some((status.BatteryLifePercent, status.ACLineStatus == 1))
    }

    /// Through UI Automation, the way a screen reader reads a selection: nothing is pressed and the
    /// clipboard is left alone. A password field is never read.
    pub fn selected_text() -> Option<String> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let automation: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
            let element = automation.GetFocusedElement().ok()?;
            if element.CurrentIsPassword().map(|password| password.as_bool()).unwrap_or(false) {
                return None;
            }
            let pattern: IUIAutomationTextPattern = element.GetCurrentPatternAs(UIA_TextPatternId).ok()?;
            let ranges = pattern.GetSelection().ok()?;
            let mut text = String::new();
            for index in 0..ranges.Length().ok()? {
                let range = ranges.GetElement(index).ok()?;
                let part = range.GetText(MAX_SELECTION_CHARS as i32 + 1).ok()?.to_string();
                if !text.is_empty() && !part.is_empty() {
                    text.push('\n');
                }
                text.push_str(&part);
                if text.chars().count() > MAX_SELECTION_CHARS {
                    break;
                }
            }
            (!text.trim().is_empty()).then_some(text)
        }
    }

    /// The Start menu's apps, as PowerShell's Get-StartApps lists them. A fixed command: nothing the
    /// model said is ever part of it.
    pub fn start_apps_blocking() -> Result<Vec<StartApp>, String> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-StartApps | ConvertTo-Json -Compress"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Couldn't list the installed apps: {e}"))?;
        if !output.status.success() {
            return Err("Couldn't list the installed apps.".to_string());
        }
        let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|_| "Couldn't read the list of installed apps.".to_string())?;
        // One app comes back as an object, several as a list.
        let rows = match parsed {
            serde_json::Value::Array(rows) => rows,
            row @ serde_json::Value::Object(_) => vec![row],
            _ => Vec::new(),
        };
        Ok(rows
            .iter()
            .filter_map(|row| {
                let text = |key: &str| row.get(key).and_then(serde_json::Value::as_str).unwrap_or_default().trim().to_string();
                let app = StartApp { name: text("Name"), id: text("AppID") };
                super::is_launchable(&app).then_some(app)
            })
            .collect())
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::{StartApp, WindowInfo};

    const UNSUPPORTED: &str = "Aloe Desktop can only do that on Windows for now. Use computer_use instead.";

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Show {
        Minimize,
        Maximize,
        Restore,
    }

    pub fn foreground() -> Option<WindowInfo> {
        None
    }
    pub fn open_windows() -> Vec<WindowInfo> {
        Vec::new()
    }
    pub fn focus(_handle: isize) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }
    pub fn show(_handle: isize, _how: Show) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }
    pub fn close(_handle: isize) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }
    pub fn snap(_handle: isize, _left: bool) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }
    pub fn volume() -> Option<(u8, bool)> {
        None
    }
    pub fn set_volume(_percent: u8) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }
    pub fn set_mute(_muted: bool) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }
    pub fn battery() -> Option<(u8, bool)> {
        None
    }
    pub fn selected_text() -> Option<String> {
        None
    }
    pub fn start_apps_blocking() -> Result<Vec<StartApp>, String> {
        Err(UNSUPPORTED.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(app: &str, title: &str) -> WindowInfo {
        WindowInfo { handle: 1, app: app.to_string(), title: title.to_string(), own: false }
    }

    #[test]
    fn app_names_come_from_the_executable() {
        assert_eq!(app_label(r"C:\Program Files\Google\Chrome\Application\chrome.exe"), "chrome");
        assert_eq!(app_label(r"C:\Users\me\AppData\Local\Programs\Microsoft VS Code\Code.exe"), "Code");
        assert_eq!(app_label("/Applications/Safari.app/Contents/MacOS/Safari"), "Safari");
    }

    #[test]
    fn a_window_is_found_by_app_then_by_title() {
        let windows = vec![
            window("chrome", "Notepad++ download - Google Chrome"),
            window("Notepad", "shopping.txt - Notepad"),
            window("Code", "chat.ts - aloe-backend - Visual Studio Code"),
            window("explorer", "Downloads"),
        ];
        assert_eq!(find_window(&windows, "Notepad").unwrap().app, "Notepad", "the app wins over a title that mentions it");
        assert_eq!(find_window(&windows, "  Google   Chrome ").unwrap().app, "chrome");
        assert_eq!(find_window(&windows, "VS Code").unwrap().app, "Code");
        assert_eq!(find_window(&windows, "file explorer").unwrap().title, "Downloads");
        assert_eq!(find_window(&windows, "shopping").unwrap().app, "Notepad");
        assert_eq!(find_window(&windows, "backend chat").unwrap().app, "Code", "every word said is somewhere in it");
        assert!(find_window(&windows, "Spotify").is_none());
        assert!(find_window(&windows, "  ").is_none());
    }

    #[test]
    fn aloes_own_windows_are_never_matched() {
        let mut own = window("aloe-desktop", "Aloe");
        own.own = true;
        assert!(find_window(&[own], "aloe").is_none());
    }

    #[test]
    fn an_installed_app_is_found_by_name_not_by_a_passing_mention() {
        let apps = vec![
            StartApp { name: "Notepad".into(), id: "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App".into() },
            StartApp { name: "Notepad++".into(), id: r"{6D809377}\Notepad++\notepad++.exe".into() },
            StartApp { name: "Sticky Notes".into(), id: "Microsoft.MicrosoftStickyNotes_8wekyb3d8bbwe!App".into() },
            StartApp { name: "Google Chrome".into(), id: "Chrome".into() },
            StartApp { name: "Visual Studio Code".into(), id: "Microsoft.VisualStudioCode".into() },
        ];
        assert_eq!(find_start_app(&apps, "notepad").unwrap().name, "Notepad");
        assert_eq!(find_start_app(&apps, "Chrome").unwrap().name, "Google Chrome");
        assert_eq!(find_start_app(&apps, "visual studio").unwrap().name, "Visual Studio Code");
        assert_eq!(find_start_app(&apps, "sticky").unwrap().name, "Sticky Notes");
        assert_eq!(find_start_app(&apps, "VS Code").unwrap().name, "Visual Studio Code");
        assert!(find_start_app(&apps, "not").is_none(), "three letters inside a name is not a match");
        assert!(find_start_app(&apps, "Photoshop").is_none());
    }

    #[test]
    fn start_menu_entries_that_are_not_apps_are_left_out() {
        let entry = |name: &str, id: &str| StartApp { name: name.into(), id: id.into() };
        assert!(is_launchable(&entry("Notepad", "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App")));
        assert!(!is_launchable(&entry("Uninstall Foo", r"C:\Foo\unins000.exe")));
        assert!(!is_launchable(&entry("Foo on the Web", "http://foo.example")));
        assert!(!is_launchable(&entry("Foo Manual", r"C:\Foo\manual.chm")));
        assert!(!is_launchable(&entry("", "x")));
    }

    #[test]
    fn clipped_text_says_it_was_clipped() {
        assert_eq!(clip("hello", 10), "hello");
        assert_eq!(clip("hello world", 5), "hello…");
    }
}

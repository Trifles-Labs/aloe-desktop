//! Browser automation over the Chrome DevTools Protocol.
//!
//! Design notes worth knowing before changing anything here:
//!
//! * **All DOM logic lives in JavaScript, not Rust.** Rust owns process launch, the websocket
//!   transport, and real input events; everything about *finding* and *describing* elements is a
//!   `Runtime.evaluate` payload. Reimplementing DOM traversal in Rust across a protocol boundary
//!   would be far more code for a worse result.
//!
//! * **One websocket per job, not one long-lived session.** Jobs arrive independently from the
//!   backend and can be minutes apart; a persistent CDP connection would have to survive tab
//!   crashes, navigations, and reconnects for no benefit, since connecting to localhost costs
//!   about a millisecond. State that must persist between jobs (which port, which tab) is small
//!   enough to keep in `BrowserState`.
//!
//! * **Clicks and typing go through `Input.dispatch*`, not `element.click()`.** Synthetic DOM
//!   events are untrusted and a good number of real sites ignore them. Dispatching at viewport
//!   coordinates produces events indistinguishable from a person's.
//!
//! * **Refs are attributes, not indices.** `read` stamps `data-aloe-ref` onto each interactive
//!   element and later calls look the attribute back up, so a ref survives re-renders that would
//!   invalidate a positional index — and goes stale honestly (element not found) after navigation
//!   rather than silently addressing the wrong control.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::process::Command;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::config::debug_log;
use crate::fs::input_string;
use crate::shell::hide_command_window;

// Limits live in config.rs; these bind the durations that Rust cannot express as consts there.
use crate::config::{
    BROWSER_CDP_TIMEOUT_SECONDS, BROWSER_DEBUG_PORT, BROWSER_LAUNCH_TIMEOUT_SECONDS,
    BROWSER_NAVIGATION_TIMEOUT_SECONDS, MAX_PAGE_TEXT_CHARS,
};

const DEFAULT_DEBUG_PORT: u16 = BROWSER_DEBUG_PORT;
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(BROWSER_LAUNCH_TIMEOUT_SECONDS);
const CDP_TIMEOUT: Duration = Duration::from_secs(BROWSER_CDP_TIMEOUT_SECONDS);
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(BROWSER_NAVIGATION_TIMEOUT_SECONDS);

#[derive(Default)]
pub struct BrowserState {
    port: Option<u16>,
    profile: Option<String>,
    /// Target id of the tab subsequent commands act on, so a multi-step task doesn't drift onto
    /// whichever tab happens to be first in the list.
    active_target: Option<String>,
}

static BROWSER: Mutex<BrowserState> = Mutex::new(BrowserState {
    port: None,
    profile: None,
    active_target: None,
});

fn remembered() -> (Option<u16>, Option<String>, Option<String>) {
    let guard = BROWSER.lock().unwrap();
    (guard.port, guard.profile.clone(), guard.active_target.clone())
}

fn remember_session(port: u16, profile: &str) {
    let mut guard = BROWSER.lock().unwrap();
    guard.port = Some(port);
    guard.profile = Some(profile.to_string());
}

fn remember_target(target_id: &str) {
    BROWSER.lock().unwrap().active_target = Some(target_id.to_string());
}

fn forget_session() {
    let mut guard = BROWSER.lock().unwrap();
    guard.port = None;
    guard.profile = None;
    guard.active_target = None;
}

// ── Chrome discovery and launch ────────────────────────────────────────────────

fn env_path(key: &str, suffix: &str) -> Option<PathBuf> {
    std::env::var_os(key).map(|base| PathBuf::from(base).join(suffix))
}

fn chrome_binary() -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = if cfg!(target_os = "windows") {
        [
            env_path("PROGRAMFILES", r"Google\Chrome\Application\chrome.exe"),
            env_path("PROGRAMFILES(X86)", r"Google\Chrome\Application\chrome.exe"),
            env_path("LOCALAPPDATA", r"Google\Chrome\Application\chrome.exe"),
            env_path("PROGRAMFILES", r"Microsoft\Edge\Application\msedge.exe"),
            env_path("PROGRAMFILES(X86)", r"Microsoft\Edge\Application\msedge.exe"),
        ]
        .into_iter()
        .flatten()
        .collect()
    } else if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
            PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
        ]
    } else {
        ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "microsoft-edge"]
            .iter()
            .flat_map(|name| {
                ["/usr/bin", "/usr/local/bin", "/snap/bin", "/opt/google/chrome"]
                    .iter()
                    .map(move |dir| PathBuf::from(dir).join(name))
            })
            .collect()
    };

    candidates.into_iter().find(|path| path.exists())
}

/// The user's everyday Chrome profile — the point of driving a local browser rather than a
/// headless one is that the sessions in here are already signed in.
fn default_user_data_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        env_path("LOCALAPPDATA", r"Google\Chrome\User Data")
    } else if cfg!(target_os = "macos") {
        dirs::home_dir().map(|home| home.join("Library/Application Support/Google/Chrome"))
    } else {
        dirs::home_dir().map(|home| home.join(".config/google-chrome"))
    }
}

fn aloe_user_data_dir() -> Result<PathBuf, String> {
    let dir = dirs::config_dir()
        .ok_or("Could not resolve a configuration directory for the Aloe browser profile.")?
        .join("Aloe Desktop")
        .join("browser-profile");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

async fn debugger_version(port: u16) -> Option<Value> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://127.0.0.1:{port}/json/version"))
        .timeout(Duration::from_millis(800))
        .send()
        .await
        .ok()?;
    response.json::<Value>().await.ok()
}

async fn launch_chrome(port: u16, profile: &str) -> Result<(), String> {
    let binary = chrome_binary().ok_or(
        "No Chrome, Chromium, or Edge installation was found. Install Google Chrome to use browser automation.",
    )?;

    let user_data_dir = if profile == "aloe" {
        aloe_user_data_dir()?
    } else {
        default_user_data_dir().ok_or("Could not locate the default Chrome profile directory.")?
    };

    let mut command = Command::new(&binary);
    command
        .arg(format!("--remote-debugging-port={port}"))
        .arg(format!("--user-data-dir={}", user_data_dir.display()))
        // Chrome refuses remote debugging on the default profile path unless it is told the
        // origin is trusted; this pins the debugging endpoint to loopback either way.
        .arg("--remote-allow-origins=*")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-background-timer-throttling")
        .arg("about:blank")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_command_window(&mut command);

    // Deliberately detached: Chrome outlives the job that started it, so a later job can reuse
    // the same tabs and logged-in state instead of relaunching.
    command.spawn().map_err(|e| format!("Could not start {}: {e}", binary.display()))?;

    let deadline = Instant::now() + LAUNCH_TIMEOUT;
    while Instant::now() < deadline {
        if debugger_version(port).await.is_some() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    if profile == "default" {
        Err(format!(
            "Chrome did not expose a debugging port within {}s. This usually means Chrome is already running with your normal profile — close every Chrome window and try again, or pass profile=\"aloe\" to use a separate Aloe browser profile that can run alongside it.",
            LAUNCH_TIMEOUT.as_secs()
        ))
    } else {
        Err(format!("Chrome did not expose a debugging port within {}s.", LAUNCH_TIMEOUT.as_secs()))
    }
}

/// Returns the debugging port, attaching to an already-listening Chrome when there is one.
async fn ensure_browser(profile_hint: Option<&str>) -> Result<u16, String> {
    let (remembered_port, remembered_profile, _) = remembered();
    let profile = profile_hint
        .map(str::to_string)
        .or(remembered_profile)
        .unwrap_or_else(|| "default".to_string());

    for port in [remembered_port.unwrap_or(DEFAULT_DEBUG_PORT), DEFAULT_DEBUG_PORT] {
        if debugger_version(port).await.is_some() {
            remember_session(port, &profile);
            return Ok(port);
        }
    }

    launch_chrome(DEFAULT_DEBUG_PORT, &profile).await?;
    remember_session(DEFAULT_DEBUG_PORT, &profile);
    Ok(DEFAULT_DEBUG_PORT)
}

// ── Target (tab) management ────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct PageTarget {
    pub id: String,
    pub title: String,
    pub url: String,
    pub ws_url: String,
}

async fn list_pages(port: u16) -> Result<Vec<PageTarget>, String> {
    let client = reqwest::Client::new();
    let targets: Value = client
        .get(format!("http://127.0.0.1:{port}/json/list"))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| format!("Could not reach the browser debugging endpoint: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Browser debugging endpoint returned unreadable JSON: {e}"))?;

    Ok(targets
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
        .filter_map(|target| {
            Some(PageTarget {
                id: target.get("id")?.as_str()?.to_string(),
                title: target.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
                url: target.get("url").and_then(Value::as_str).unwrap_or_default().to_string(),
                ws_url: target.get("webSocketDebuggerUrl")?.as_str()?.to_string(),
            })
        })
        .collect())
}

async fn open_tab(port: u16, url: &str) -> Result<PageTarget, String> {
    let client = reqwest::Client::new();
    // /json/new only accepts PUT on current Chrome builds; the URL rides in the query string.
    let target: Value = client
        .put(format!("http://127.0.0.1:{port}/json/new?{}", urlencode(url)))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Could not open a new tab: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Could not read the new tab's details: {e}"))?;

    Ok(PageTarget {
        id: target.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
        title: target.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
        url: target.get("url").and_then(Value::as_str).unwrap_or_default().to_string(),
        ws_url: target
            .get("webSocketDebuggerUrl")
            .and_then(Value::as_str)
            .ok_or("The new tab did not expose a debugger websocket.")?
            .to_string(),
    })
}

fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' | b'/' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// The tab commands act on: the remembered one while it still exists, else the frontmost page.
async fn active_page(port: u16) -> Result<PageTarget, String> {
    let pages = list_pages(port).await?;
    if pages.is_empty() {
        return Err("The browser has no open pages. Use action=\"open\" with a URL first.".to_string());
    }

    let (_, _, remembered_target) = remembered();
    if let Some(target_id) = remembered_target {
        if let Some(page) = pages.iter().find(|page| page.id == target_id) {
            return Ok(page.clone());
        }
    }

    let page = pages[0].clone();
    remember_target(&page.id);
    Ok(page)
}

// ── CDP transport ──────────────────────────────────────────────────────────────

struct Cdp {
    socket: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    next_id: u64,
}

impl Cdp {
    async fn connect(ws_url: &str) -> Result<Self, String> {
        let (socket, _) = connect_async(ws_url)
            .await
            .map_err(|e| format!("Could not attach to the browser tab: {e}"))?;
        Ok(Self { socket, next_id: 1 })
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;

        let request = json!({ "id": id, "method": method, "params": params });
        self.socket
            .send(Message::Text(request.to_string().into()))
            .await
            .map_err(|e| format!("Browser command {method} could not be sent: {e}"))?;

        let deadline = Instant::now() + CDP_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("Browser command {method} timed out."));
            }

            let message = tokio::time::timeout(remaining, self.socket.next())
                .await
                .map_err(|_| format!("Browser command {method} timed out."))?
                .ok_or_else(|| format!("Browser closed the connection during {method}."))?
                .map_err(|e| format!("Browser connection error during {method}: {e}"))?;

            let text = match message {
                Message::Text(text) => text.to_string(),
                Message::Close(_) => return Err(format!("Browser closed the connection during {method}.")),
                // CDP also emits binary frames and pings; neither can be a reply to `id`.
                _ => continue,
            };

            let Ok(parsed) = serde_json::from_str::<Value>(&text) else { continue };
            if parsed.get("id").and_then(Value::as_u64) != Some(id) {
                continue; // An event, or a reply to an earlier call — keep reading.
            }
            if let Some(error) = parsed.get("error") {
                let message = error.get("message").and_then(Value::as_str).unwrap_or("unknown error");
                return Err(format!("Browser rejected {method}: {message}"));
            }
            return Ok(parsed.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// Evaluates an expression in the page and returns its value.
    async fn eval(&mut self, expression: &str) -> Result<Value, String> {
        let result = self
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                    "userGesture": true,
                }),
            )
            .await?;

        if let Some(details) = result.get("exceptionDetails") {
            let text = details
                .get("exception")
                .and_then(|exception| exception.get("description"))
                .and_then(Value::as_str)
                .or_else(|| details.get("text").and_then(Value::as_str))
                .unwrap_or("script error");
            return Err(format!("Page script failed: {text}"));
        }

        Ok(result
            .get("result")
            .and_then(|inner| inner.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    async fn wait_until_loaded(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + NAVIGATION_TIMEOUT;
        while Instant::now() < deadline {
            let state = self.eval("document.readyState").await.unwrap_or(Value::Null);
            if state.as_str() == Some("complete") {
                // Frameworks paint after readyState flips; a short settle avoids reading a
                // skeleton screen on every single-page app.
                tokio::time::sleep(Duration::from_millis(350)).await;
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(()) // A page that never finishes loading is still readable — report what's there.
    }
}

// ── Page scripts ───────────────────────────────────────────────────────────────

/// Tags interactive elements with `data-aloe-ref` and returns an outline plus visible text.
fn read_script(max_chars: usize) -> String {
    format!(
        r#"(() => {{
    const SELECTOR = 'a[href],button,input,textarea,select,summary,[role="button"],[role="link"],[role="checkbox"],[role="radio"],[role="tab"],[role="menuitem"],[role="switch"],[contenteditable="true"]';
    const MAX_ELEMENTS = 200;

    for (const stale of document.querySelectorAll('[data-aloe-ref]')) stale.removeAttribute('data-aloe-ref');

    const label = (el) => {{
        const candidates = [
            el.getAttribute('aria-label'),
            el.getAttribute('placeholder'),
            el.tagName === 'INPUT' && el.type !== 'password' ? el.value : '',
            (el.innerText || el.textContent || '').trim(),
            el.getAttribute('title'),
            el.getAttribute('alt'),
            el.getAttribute('name'),
        ];
        for (const candidate of candidates) {{
            if (candidate && candidate.trim()) return candidate.trim().replace(/\s+/g, ' ').slice(0, 140);
        }}
        return '';
    }};

    const kind = (el) => {{
        const tag = el.tagName.toLowerCase();
        if (tag === 'input') return 'input:' + (el.type || 'text');
        if (tag === 'a') return 'link';
        if (tag === 'select') return 'select';
        if (tag === 'textarea') return 'textarea';
        return el.getAttribute('role') || tag;
    }};

    const elements = [];
    let counter = 0;
    for (const el of document.querySelectorAll(SELECTOR)) {{
        if (elements.length >= MAX_ELEMENTS) break;
        const rect = el.getBoundingClientRect();
        if (rect.width < 1 || rect.height < 1) continue;
        const style = getComputedStyle(el);
        if (style.visibility === 'hidden' || style.display === 'none' || style.opacity === '0') continue;

        const ref = 'ref_' + (++counter);
        el.setAttribute('data-aloe-ref', ref);

        const entry = {{ ref, kind: kind(el), label: label(el) }};
        if (el.disabled) entry.disabled = true;
        if (el.checked === true) entry.checked = true;
        if (el.tagName === 'A' && el.href) entry.href = el.href.slice(0, 300);
        if (el.tagName === 'SELECT') {{
            entry.options = Array.from(el.options).slice(0, 30).map((option) => option.text.trim().slice(0, 80));
        }}
        elements.push(entry);
    }}

    const text = (document.body ? document.body.innerText : '').replace(/\n{{3,}}/g, '\n\n').trim();
    return {{
        url: location.href,
        title: document.title,
        elements,
        text: text.slice(0, {max_chars}),
        textTruncated: text.length > {max_chars},
        scroll: {{ y: Math.round(window.scrollY), height: Math.round(document.body ? document.body.scrollHeight : 0), viewport: Math.round(window.innerHeight) }},
    }};
}})()"#
    )
}

/// Scrolls a ref into view and reports where to click it.
fn locate_script(reference: &str) -> String {
    let literal = serde_json::to_string(reference).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        r#"(() => {{
    const el = document.querySelector('[data-aloe-ref=' + JSON.stringify({literal}) + ']');
    if (!el) return {{ found: false }};
    el.scrollIntoView({{ block: 'center', inline: 'center', behavior: 'instant' }});
    const rect = el.getBoundingClientRect();
    const visible = rect.width > 0 && rect.height > 0 && rect.bottom > 0 && rect.right > 0
        && rect.top < window.innerHeight && rect.left < window.innerWidth;
    return {{
        found: true,
        visible,
        x: rect.left + rect.width / 2,
        y: rect.top + rect.height / 2,
        tag: el.tagName.toLowerCase(),
        type: el.getAttribute('type') || '',
        label: (el.getAttribute('aria-label') || el.innerText || el.value || '').trim().slice(0, 120),
    }};
}})()"#
    )
}

fn focus_and_clear_script(reference: &str) -> String {
    let literal = serde_json::to_string(reference).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        r#"(() => {{
    const el = document.querySelector('[data-aloe-ref=' + JSON.stringify({literal}) + ']');
    if (!el) return {{ found: false }};
    el.focus();
    if ('value' in el) {{
        // Reacts and Vues track the previous value on the node; setting through the native
        // setter is what makes their onChange actually fire for a programmatic clear.
        const setter = Object.getOwnPropertyDescriptor(el.constructor.prototype, 'value')?.set;
        if (setter) setter.call(el, ''); else el.value = '';
        el.dispatchEvent(new Event('input', {{ bubbles: true }}));
    }} else if (el.isContentEditable) {{
        el.textContent = '';
    }}
    return {{ found: true, tag: el.tagName.toLowerCase() }};
}})()"#
    )
}

fn select_script(reference: &str, value: &str) -> String {
    let ref_literal = serde_json::to_string(reference).unwrap_or_else(|_| "\"\"".to_string());
    let value_literal = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        r#"(() => {{
    const el = document.querySelector('[data-aloe-ref=' + JSON.stringify({ref_literal}) + ']');
    if (!el) return {{ found: false }};
    if (el.tagName !== 'SELECT') return {{ found: true, error: 'Element ' + el.tagName + ' is not a dropdown.' }};

    const wanted = {value_literal};
    const normalized = String(wanted).trim().toLowerCase();
    const match = Array.from(el.options).find((option) =>
        option.value === wanted
        || option.text.trim() === String(wanted).trim()
        || option.text.trim().toLowerCase() === normalized);

    if (!match) {{
        return {{ found: true, error: 'No option matched.', options: Array.from(el.options).slice(0, 30).map((o) => o.text.trim()) }};
    }}
    el.value = match.value;
    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
    return {{ found: true, selected: match.text.trim(), value: match.value }};
}})()"#
    )
}

fn find_script(query: &str) -> String {
    let literal = serde_json::to_string(query).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        r#"(() => {{
    const needle = String({literal}).trim().toLowerCase();
    const matches = [];
    for (const el of document.querySelectorAll('[data-aloe-ref]')) {{
        const haystack = [
            el.getAttribute('aria-label'), el.getAttribute('placeholder'), el.getAttribute('name'),
            el.getAttribute('title'), el.innerText, el.value, el.getAttribute('href'),
        ].filter(Boolean).join(' ').toLowerCase();
        if (!haystack.includes(needle)) continue;
        matches.push({{
            ref: el.getAttribute('data-aloe-ref'),
            kind: el.tagName.toLowerCase(),
            label: (el.getAttribute('aria-label') || el.innerText || el.value || '').trim().replace(/\s+/g, ' ').slice(0, 140),
        }});
        if (matches.length >= 25) break;
    }}
    return {{ query: {literal}, matches, hint: matches.length === 0 ? 'Nothing matched. Call action="read" first — refs only exist after a read.' : undefined }};
}})()"#
    )
}

// ── Input helpers ──────────────────────────────────────────────────────────────

async fn mouse_click(cdp: &mut Cdp, x: f64, y: f64) -> Result<(), String> {
    let base = json!({ "x": x, "y": y, "button": "left", "clickCount": 1, "buttons": 1 });

    let mut pressed = base.clone();
    pressed["type"] = json!("mousePressed");
    cdp.call("Input.dispatchMouseEvent", pressed).await?;

    let mut released = base;
    released["type"] = json!("mouseReleased");
    released["buttons"] = json!(0);
    cdp.call("Input.dispatchMouseEvent", released).await?;
    Ok(())
}

async fn press_enter(cdp: &mut Cdp) -> Result<(), String> {
    for event in ["rawKeyDown", "char", "keyUp"] {
        cdp.call(
            "Input.dispatchKeyEvent",
            json!({
                "type": event,
                "key": "Enter",
                "code": "Enter",
                "text": "\r",
                "unmodifiedText": "\r",
                "windowsVirtualKeyCode": 13,
                "nativeVirtualKeyCode": 13,
            }),
        )
        .await?;
    }
    Ok(())
}

fn located(value: &Value) -> Result<(f64, f64, String), String> {
    if value.get("found").and_then(Value::as_bool) != Some(true) {
        return Err("That ref is not on the page any more. Call action=\"read\" again to get current refs — they are cleared by navigation.".to_string());
    }
    let x = value.get("x").and_then(Value::as_f64).unwrap_or(0.0);
    let y = value.get("y").and_then(Value::as_f64).unwrap_or(0.0);
    let label = value.get("label").and_then(Value::as_str).unwrap_or("").to_string();
    Ok((x, y, label))
}

// ── Job entry point ────────────────────────────────────────────────────────────

pub async fn dispatch_browser(kind: &str, input: &Value) -> Result<Value, String> {
    debug_log("browser", "job", format!("kind={kind}"));

    // `close` must not resurrect a browser just to shut it down.
    if kind == "browser_close" {
        return close_browser().await;
    }

    let profile_hint = input.get("profile").and_then(Value::as_str);
    let port = ensure_browser(profile_hint).await?;

    match kind {
        "browser_open" => open(port, input).await,
        "browser_read" => read(port, input).await,
        "browser_find" => find(port, input).await,
        "browser_click" => click(port, input).await,
        "browser_type" => type_text(port, input).await,
        "browser_select" => select(port, input).await,
        "browser_screenshot" => screenshot(port, input).await,
        "browser_scroll" => scroll(port, input).await,
        "browser_back" => history(port, input).await,
        "browser_tabs" => tabs(port, input).await,
        _ => Err(format!("Unknown browser action: {kind}")),
    }
}

async fn open(port: u16, input: &Value) -> Result<Value, String> {
    let url = input_string(input, "url")?;
    let url = if url.contains("://") { url } else { format!("https://{url}") };
    let new_tab = input.get("newTab").and_then(Value::as_bool).unwrap_or(false);

    let page = if new_tab || list_pages(port).await?.is_empty() {
        open_tab(port, &url).await?
    } else {
        active_page(port).await?
    };
    remember_target(&page.id);

    let mut cdp = Cdp::connect(&page.ws_url).await?;
    cdp.call("Page.enable", json!({})).await.ok();
    // Bring the tab to the front so the user can see what Aloe is doing; a failure here is
    // cosmetic and must not fail the navigation.
    cdp.call("Page.bringToFront", json!({})).await.ok();

    if !new_tab {
        cdp.call("Page.navigate", json!({ "url": url })).await?;
    }
    cdp.wait_until_loaded().await?;

    let summary = cdp.eval(&read_script(1_500)).await?;
    Ok(json!({
        "opened": true,
        "url": summary.get("url").cloned().unwrap_or(json!(url)),
        "title": summary.get("title").cloned().unwrap_or(Value::Null),
        "elements": summary.get("elements").cloned().unwrap_or(json!([])),
        "preview": summary.get("text").cloned().unwrap_or(Value::Null),
        "note": "Refs above are valid until the page navigates. Call action=\"read\" for the full page text.",
    }))
}

async fn read(port: u16, input: &Value) -> Result<Value, String> {
    let max_chars = input
        .get("maxChars")
        .and_then(Value::as_u64)
        .map(|value| (value as usize).min(MAX_PAGE_TEXT_CHARS))
        .unwrap_or(MAX_PAGE_TEXT_CHARS);

    let page = active_page(port).await?;
    let mut cdp = Cdp::connect(&page.ws_url).await?;
    cdp.eval(&read_script(max_chars)).await
}

async fn find(port: u16, input: &Value) -> Result<Value, String> {
    let query = input_string(input, "query")?;
    let page = active_page(port).await?;
    let mut cdp = Cdp::connect(&page.ws_url).await?;

    // Refs only exist after a read, and `find` is a natural first call — so seed them rather
    // than returning a confusing empty result.
    cdp.eval(&read_script(1)).await?;
    cdp.eval(&find_script(&query)).await
}

async fn click(port: u16, input: &Value) -> Result<Value, String> {
    let reference = input_string(input, "ref")?;
    let page = active_page(port).await?;
    let mut cdp = Cdp::connect(&page.ws_url).await?;

    let location = cdp.eval(&locate_script(&reference)).await?;
    let (x, y, label) = located(&location)?;
    let url_before = cdp.eval("location.href").await?;

    mouse_click(&mut cdp, x, y).await?;
    // Long enough for a client-side route change or a form post to commit; short enough that a
    // click which does nothing visible doesn't stall the turn.
    tokio::time::sleep(Duration::from_millis(700)).await;
    cdp.wait_until_loaded().await?;

    let after = cdp.eval(&read_script(1_500)).await?;
    let navigated = after.get("url") != url_before.as_str().map(|value| json!(value)).as_ref();

    Ok(json!({
        "clicked": label,
        "navigated": navigated,
        "url": after.get("url").cloned().unwrap_or(Value::Null),
        "title": after.get("title").cloned().unwrap_or(Value::Null),
        "elements": after.get("elements").cloned().unwrap_or(json!([])),
        "preview": after.get("text").cloned().unwrap_or(Value::Null),
    }))
}

async fn type_text(port: u16, input: &Value) -> Result<Value, String> {
    let reference = input_string(input, "ref")?;
    let text = input_string(input, "text")?;
    let submit = input.get("submit").and_then(Value::as_bool).unwrap_or(false);

    let page = active_page(port).await?;
    let mut cdp = Cdp::connect(&page.ws_url).await?;

    let focused = cdp.eval(&focus_and_clear_script(&reference)).await?;
    if focused.get("found").and_then(Value::as_bool) != Some(true) {
        return Err("That ref is not on the page any more. Call action=\"read\" again to get current refs.".to_string());
    }

    // insertText delivers a real composition/input event, which is what framework-controlled
    // inputs listen for — assigning .value directly leaves many of them out of sync.
    cdp.call("Input.insertText", json!({ "text": text })).await?;

    if submit {
        press_enter(&mut cdp).await?;
        tokio::time::sleep(Duration::from_millis(700)).await;
        cdp.wait_until_loaded().await?;
    }

    let after = cdp.eval(&read_script(1_200)).await?;
    Ok(json!({
        "typed": text.chars().count(),
        "submitted": submit,
        "url": after.get("url").cloned().unwrap_or(Value::Null),
        "title": after.get("title").cloned().unwrap_or(Value::Null),
        "elements": after.get("elements").cloned().unwrap_or(json!([])),
        "preview": after.get("text").cloned().unwrap_or(Value::Null),
    }))
}

async fn select(port: u16, input: &Value) -> Result<Value, String> {
    let reference = input_string(input, "ref")?;
    let value = input_string(input, "value")?;

    let page = active_page(port).await?;
    let mut cdp = Cdp::connect(&page.ws_url).await?;
    let result = cdp.eval(&select_script(&reference, &value)).await?;

    if result.get("found").and_then(Value::as_bool) != Some(true) {
        return Err("That ref is not on the page any more. Call action=\"read\" again to get current refs.".to_string());
    }
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        return Err(match result.get("options") {
            Some(options) => format!("{error} Available options: {options}"),
            None => error.to_string(),
        });
    }
    Ok(result)
}

async fn screenshot(port: u16, input: &Value) -> Result<Value, String> {
    let full_page = input.get("fullPage").and_then(Value::as_bool).unwrap_or(false);

    let page = active_page(port).await?;
    let mut cdp = Cdp::connect(&page.ws_url).await?;

    let result = cdp
        .call(
            "Page.captureScreenshot",
            json!({ "format": "png", "captureBeyondViewport": full_page, "optimizeForSpeed": true }),
        )
        .await?;

    let data = result
        .get("data")
        .and_then(Value::as_str)
        .ok_or("The browser returned an empty screenshot.")?;
    let bytes = BASE64.decode(data).map_err(|e| format!("Screenshot was not valid base64: {e}"))?;

    Ok(json!({
        "mimeType": "image/png",
        "base64": data,
        "sizeBytes": bytes.len(),
        "url": page.url,
        "title": page.title,
        "fullPage": full_page,
    }))
}

async fn scroll(port: u16, input: &Value) -> Result<Value, String> {
    let direction = input.get("direction").and_then(Value::as_str).unwrap_or("down");
    let amount = input.get("amount").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.1, 10.0);

    let page = active_page(port).await?;
    let mut cdp = Cdp::connect(&page.ws_url).await?;

    let expression = match direction {
        "up" => format!("window.scrollBy(0, -window.innerHeight * {amount})"),
        "top" => "window.scrollTo(0, 0)".to_string(),
        "bottom" => "window.scrollTo(0, document.body.scrollHeight)".to_string(),
        _ => format!("window.scrollBy(0, window.innerHeight * {amount})"),
    };
    cdp.eval(&expression).await?;
    tokio::time::sleep(Duration::from_millis(400)).await;

    cdp.eval(&read_script(4_000)).await
}

async fn history(port: u16, input: &Value) -> Result<Value, String> {
    let forward = input.get("direction").and_then(Value::as_str) == Some("forward");

    let page = active_page(port).await?;
    let mut cdp = Cdp::connect(&page.ws_url).await?;
    cdp.eval(if forward { "history.forward()" } else { "history.back()" }).await?;
    tokio::time::sleep(Duration::from_millis(600)).await;
    cdp.wait_until_loaded().await?;

    cdp.eval(&read_script(2_000)).await
}

async fn tabs(port: u16, input: &Value) -> Result<Value, String> {
    let action = input.get("tabAction").and_then(Value::as_str).unwrap_or("list");
    let pages = list_pages(port).await?;

    match action {
        "switch" | "close" => {
            let target_id = input_string(input, "targetId")?;
            let page = pages
                .iter()
                .find(|page| page.id == target_id)
                .ok_or_else(|| format!("No open tab has id {target_id}. Use tabAction=\"list\" first."))?;

            if action == "switch" {
                let mut cdp = Cdp::connect(&page.ws_url).await?;
                cdp.call("Page.bringToFront", json!({})).await.ok();
                remember_target(&page.id);
                Ok(json!({ "switched": true, "targetId": page.id, "url": page.url, "title": page.title }))
            } else {
                let client = reqwest::Client::new();
                client
                    .get(format!("http://127.0.0.1:{port}/json/close/{target_id}"))
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await
                    .map_err(|e| format!("Could not close the tab: {e}"))?;
                if remembered().2.as_deref() == Some(target_id.as_str()) {
                    BROWSER.lock().unwrap().active_target = None;
                }
                Ok(json!({ "closed": true, "targetId": target_id }))
            }
        }
        _ => {
            let (_, _, active) = remembered();
            Ok(json!({
                "tabs": pages.iter().map(|page| json!({
                    "targetId": page.id,
                    "title": page.title,
                    "url": page.url,
                    "active": Some(page.id.clone()) == active,
                })).collect::<Vec<_>>(),
            }))
        }
    }
}

async fn close_browser() -> Result<Value, String> {
    let (port, _, _) = remembered();
    let Some(port) = port else {
        return Ok(json!({ "closed": false, "reason": "No Aloe browser session is running." }));
    };

    // Browser.close needs any page target to attach to; if there is none the browser is
    // already effectively gone and forgetting the session is the whole job.
    let result = match list_pages(port).await {
        Ok(pages) if !pages.is_empty() => {
            let mut cdp = Cdp::connect(&pages[0].ws_url).await?;
            cdp.call("Browser.close", json!({})).await.map(|_| true)
        }
        _ => Ok(false),
    };

    forget_session();
    Ok(json!({ "closed": result.unwrap_or(false) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencode_escapes_query_characters_but_keeps_url_structure() {
        assert_eq!(urlencode("https://a.test/x?y=1&z=2"), "https://a.test/x%3Fy%3D1%26z%3D2");
    }

    #[test]
    fn scripts_embed_refs_as_json_literals_so_quotes_cannot_break_out() {
        let script = locate_script("ref\"_1");
        assert!(script.contains(r#""ref\"_1""#), "ref must be embedded as an escaped JSON literal: {script}");
    }

    #[test]
    fn located_rejects_a_stale_ref_with_actionable_guidance() {
        let error = located(&json!({ "found": false })).unwrap_err();
        assert!(error.contains("read"), "stale-ref error should tell the caller to re-read: {error}");
    }

    #[test]
    fn located_returns_the_element_centre() {
        let (x, y, label) = located(&json!({ "found": true, "x": 12.5, "y": 40.0, "label": "Sign in" })).unwrap();
        assert_eq!((x, y, label.as_str()), (12.5, 40.0, "Sign in"));
    }
}

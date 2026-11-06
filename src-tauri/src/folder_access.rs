//! `request_folder_access`: Aloe asks the user for a folder it needs. The device opens the native
//! folder picker titled with Aloe's reason; the folder the user picks becomes an ordinary granted
//! folder (Settings › Desktop app › Folder access), and closing the picker refuses. Aloe may suggest
//! where the picker starts, but only the user's pick is ever granted.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::config::{debug_log, make_granted_folder, save_config, AppState};
use crate::socket::sync_folders_with_config;

const MAX_REASON_CHARS: usize = 160;

fn text(input: &Value, key: &str) -> String {
    input.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string()
}

/// Where the picker opens: the suggested folder, or the nearest folder above it that exists.
fn start_directory(suggested: &str) -> Option<PathBuf> {
    if suggested.is_empty() {
        return None;
    }
    let mut path = Path::new(suggested);
    loop {
        if path.is_dir() {
            return Some(path.to_path_buf());
        }
        path = path.parent()?;
    }
}

pub fn picker_title(reason: &str) -> String {
    let reason: String = reason.chars().take(MAX_REASON_CHARS).collect();
    if reason.is_empty() {
        "Aloe is asking for access to a folder".to_string()
    } else {
        format!("Aloe is asking for a folder: {reason}")
    }
}

pub async fn request_folder_access(app: &AppHandle, input: &Value) -> Result<Value, String> {
    let title = picker_title(&text(input, "reason"));
    let start = start_directory(&text(input, "suggestedPath"));

    // The picker belongs to the main window; bring it forward so the dialog isn't hidden behind
    // whatever the user is looking at.
    crate::desktop::show_main_window(app);
    let dialog_app = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        let mut picker = dialog_app.dialog().file().set_title(title);
        if let Some(start) = start {
            picker = picker.set_directory(start);
        }
        picker.blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(picked) = picked else {
        debug_log("folders", "request_declined", "");
        return Ok(json!({
            "granted": false,
            "message": "The user closed the folder picker without choosing a folder. Don't ask again unless they say they want to.",
        }));
    };

    let canonical = std::fs::canonicalize(picked.to_string()).map_err(|e| e.to_string())?;
    let path = canonical.to_string_lossy().to_string();
    let state = app.state::<AppState>();
    let (config, already_granted) = {
        let mut config = state.config.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let already_granted = config.folders.iter().any(|folder| folder.path == path);
        if !already_granted {
            debug_log("folders", "request_granted", format!("path={path}"));
            config.folders.push(make_granted_folder(&canonical));
            save_config(&config)?;
        }
        (config.clone(), already_granted)
    };
    // The backend checks every file job against the folders it was last sent, so the grant has to
    // reach it before Aloe's next step uses the folder.
    if !already_granted {
        sync_folders_with_config(&state.client, &config).await?;
    }

    Ok(json!({ "granted": true, "path": path, "alreadyGranted": already_granted }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picker_title_carries_the_reason() {
        assert_eq!(picker_title(""), "Aloe is asking for access to a folder");
        assert_eq!(picker_title("to read your thesis drafts"), "Aloe is asking for a folder: to read your thesis drafts");
        assert_eq!(picker_title(&"x".repeat(500)).len(), "Aloe is asking for a folder: ".len() + MAX_REASON_CHARS);
    }

    #[test]
    fn the_picker_starts_at_the_nearest_existing_folder() {
        let temp = std::env::temp_dir();
        let missing = temp.join("aloe-no-such-folder").join("deeper");
        assert_eq!(start_directory(&missing.to_string_lossy()), Some(temp.clone()));
        assert_eq!(start_directory(""), None);
    }
}

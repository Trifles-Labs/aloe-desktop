//! Files Aloe hands the user in chat (the backend's local_file_share_file).
//!
//! Nothing is uploaded. The `share_file` job checks the grant, remembers the canonical path under a
//! fresh id and returns that id; the chat card carries only the id. The card's Open / Show in folder
//! buttons come back here with it, so the UI never names a path, and the grant is checked again at
//! open time in case it was revoked since.

use chrono::Utc;
use serde_json::{json, Value};
use std::path::Path;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::config::{save_config, AppState};
use crate::fs::{assert_granted, input_string};
use crate::models::{AgentConfig, SharedFile};

/// Shared files remembered in config.json. Cards older than this stop opening, which is fine:
/// asking Aloe again finds the file again.
const MAX_SHARED_FILES: usize = 200;

/// What Open may hand to the OS. An allowlist, not a blocklist: the OS opener runs whatever is
/// associated with an extension, and for .exe, .bat, .ps1, .js (Windows Script Host), .lnk, .url,
/// .hta, .chm, macro-enabled Office files and many more, that means running code from a click on a
/// card the model put in the chat. Anything else can still be revealed in its folder.
const OPENABLE_EXTENSIONS: &[&str] = &[
    // Documents
    "pdf", "txt", "md", "markdown", "rtf", "csv", "tsv", "log", "json", "xml", "yaml", "yml",
    "doc", "docx", "odt", "xls", "xlsx", "ods", "ppt", "pptx", "odp", "pages", "numbers", "key", "epub",
    // Images
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif", "avif", "ico",
    // Audio and video
    "mp3", "wav", "m4a", "aac", "flac", "ogg", "opus", "mp4", "m4v", "mov", "mkv", "avi", "webm", "wmv",
];

fn is_openable(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| OPENABLE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// The `share_file` job. Same grant check as every other file job.
pub fn share_file(state: &State<'_, AppState>, config: &AgentConfig, input: &Value) -> Result<Value, String> {
    let path = assert_granted(config, &input_string(input, "path")?)?;
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(format!("Path is not a file: {}", path.display()));
    }
    let path_text = path.to_string_lossy().to_string();
    let filename = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_else(|| path_text.clone());

    let id = {
        let mut current = state.config.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // Sharing the same file again reuses its id, so a card in an older message keeps working.
        let id = current
            .shared_files
            .iter()
            .position(|entry| entry.path == path_text)
            .map(|index| current.shared_files.remove(index).id)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        current.shared_files.insert(0, SharedFile { id: id.clone(), path: path_text.clone(), shared_at: Utc::now().to_rfc3339() });
        current.shared_files.truncate(MAX_SHARED_FILES);
        save_config(&current)?;
        id
    };

    Ok(json!({
        "shareId": id,
        "path": path_text,
        "filename": filename,
        "sizeBytes": meta.len(),
        "openable": is_openable(&path),
    }))
}

/// Resolves a share id to a path this device may still open, or says why not.
fn resolve(config: &AgentConfig, id: &str) -> Result<std::path::PathBuf, String> {
    let entry = config
        .shared_files
        .iter()
        .find(|entry| entry.id == id)
        .ok_or("This file was shared on another computer, or too long ago. Ask Aloe to find it again.")?;
    let path = assert_granted(config, &entry.path)
        .map_err(|_| "Aloe no longer has access to the folder this file is in.".to_string())?;
    if !path.is_file() {
        return Err("This file has been moved or deleted.".to_string());
    }
    Ok(path)
}

/// Open or Show in folder, from a card in the chat.
#[tauri::command]
pub fn shared_file_open(app: AppHandle, state: State<'_, AppState>, id: String, reveal: bool) -> Result<(), String> {
    let config = state.config.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    let path = resolve(&config, &id)?;
    if reveal {
        return app.opener().reveal_item_in_dir(&path).map_err(|e| e.to_string());
    }
    if !is_openable(&path) {
        return Err("Aloe doesn't open this kind of file directly. Use Show in folder.".to_string());
    }
    app.opener().open_path(path.to_string_lossy().to_string(), None::<String>).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::GrantedFolder;

    fn granting(dir: &Path) -> AgentConfig {
        AgentConfig {
            folders: vec![GrantedFolder { path: dir.to_string_lossy().to_string(), label: None, indexed_at: None, context: None, context_updated_at: None }],
            ..Default::default()
        }
    }

    #[test]
    fn opens_documents_but_never_programs_or_scripts() {
        for name in ["report.PDF", "photo.jpg", "notes.md", "budget.xlsx", "clip.mp4"] {
            assert!(is_openable(Path::new(name)), "{name} should open");
        }
        for name in ["setup.exe", "run.bat", "x.ps1", "a.js", "link.lnk", "site.url", "help.chm", "macro.xlsm", "page.html", "noext"] {
            assert!(!is_openable(Path::new(name)), "{name} must not open");
        }
    }

    #[test]
    fn resolve_refuses_unknown_ids_revoked_grants_and_missing_files() {
        let dir = std::env::temp_dir().join(format!("aloe-share-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.pdf");
        std::fs::write(&file, "x").unwrap();
        let canonical = std::fs::canonicalize(&file).unwrap().to_string_lossy().to_string();

        let mut config = granting(&dir);
        config.shared_files.push(SharedFile { id: "one".into(), path: canonical, shared_at: String::new() });

        assert!(resolve(&config, "one").is_ok());
        assert!(resolve(&config, "other").is_err());

        let revoked = AgentConfig { folders: vec![], ..config.clone() };
        assert!(resolve(&revoked, "one").unwrap_err().contains("no longer has access"));

        std::fs::remove_file(&file).unwrap();
        assert!(resolve(&config, "one").unwrap_err().contains("moved or deleted"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

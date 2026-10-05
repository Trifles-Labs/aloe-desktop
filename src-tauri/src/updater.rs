//! Self-update, driven from Rust instead of the updater plugin's JS commands.
//!
//! The plugin's own commands install with a hardcoded `on_before_exit` hook that calls
//! `cleanup_before_exit()`. On Windows that hook runs on a worker thread and tears the windows down
//! while the main event loop is still pumping messages, so the next message panics inside tao
//! ("cannot move state from Destroyed") and the process aborts — often before the installer has been
//! launched, so the next start finds the same update and crashes again. Building the updater here
//! lets us replace that hook with one that touches no windows: the installer starts, the process
//! exits, and the OS reclaims everything.
//!
//! Splitting download from install also means a new version waits for the Restart button instead of
//! closing the app out from under the user the moment it finishes downloading.

use std::sync::Mutex;

use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{config::debug_log, cursor};

#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<(Update, Vec<u8>)>>);

/// Checks for and downloads a newer version without installing it. Returns the version, or None
/// when already up to date.
#[tauri::command]
pub async fn download_update(app: AppHandle) -> Result<Option<String>, String> {
    // The hook travels with the `Update` this check returns, so it has to be set here, not at
    // install time. The pointer is the one thing that outlives the process, so it is all the hook
    // needs to put back.
    let updater = app
        .updater_builder()
        .on_before_exit(cursor::restore)
        .build()
        .map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    let version = update.version.clone();
    debug_log("updater", "downloaded", format!("version={version}"));
    *app.state::<PendingUpdate>().0.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((update, bytes));
    Ok(Some(version))
}

/// Installs the downloaded update and restarts into it. On Windows this never returns: the
/// installer is launched and the process exits.
#[tauri::command]
pub fn install_update(app: AppHandle) -> Result<(), String> {
    let Some((update, bytes)) = app.state::<PendingUpdate>().0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() else {
        return Err("No update has been downloaded.".into());
    };
    update.install(bytes).map_err(|e| e.to_string())?;
    // Reached on macOS/Linux, where install swaps the bundle in place.
    app.restart();
}

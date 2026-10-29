//! Cleanup for the green system pointer older Aloe Desktop versions swapped in.
//!
//! The pointer is no longer recolored during desktop control: the overlay's glow and pill
//! (overlay.rs) are the only "Aloe is using your computer" indicators now. This module remains so
//! an upgrade from a version that crashed mid-run can put the user's own cursor scheme back — that
//! swap was system-wide and outlived the process, and the marker file is the only record of it.

#[cfg(windows)]
pub use windows_impl::{restore, restore_left_over};

#[cfg(not(windows))]
pub fn restore() {}
#[cfg(not(windows))]
pub fn restore_left_over() {}

#[cfg(windows)]
mod windows_impl {
    use std::path::PathBuf;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_SETCURSORS};

    use crate::config::{config_path, debug_log};

    fn marker() -> Option<PathBuf> {
        config_path().ok().map(|path| path.with_file_name("pointer-swapped"))
    }

    /// Puts the user's own cursor scheme back if an older Aloe Desktop left the green pointer
    /// swapped in. Safe to call at startup and on quit; no marker means there is nothing to do.
    pub fn restore() {
        if !marker().is_some_and(|path| path.exists()) {
            return;
        }
        debug_log("cursor", "restore_left_over", "");
        unsafe { SystemParametersInfoW(SPI_SETCURSORS, 0, std::ptr::null_mut(), 0) };
        if let Some(path) = marker() {
            let _ = std::fs::remove_file(path);
        }
    }

    pub fn restore_left_over() {
        restore();
    }
}

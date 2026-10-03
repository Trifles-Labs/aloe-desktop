//! The green pointer that shows Aloe is in control.
//!
//! On Windows the real system pointer itself turns green for the run: `SetSystemCursor` swaps the
//! arrow (and the shapes that usually replace it — text beam, link hand, busy arrow) for a green
//! arrow, and `SPI_SETCURSORS` puts the user's own scheme back. An overlay can't do this, because
//! the OS draws the real pointer above every window, overlays included.
//!
//! That swap is system-wide and outlives the process, so a crash mid-run would leave the pointer
//! green. A marker file records that a swap is in effect: `restore` clears it at the end of every
//! run, on Stop, on quit, and `restore_left_over` clears it again at the next launch.
//!
//! Elsewhere the system pointer can't be changed from a background app (macOS only lets the
//! frontmost app touch it), so the overlay draws a green halo under it instead (overlay.rs).

#[cfg(windows)]
pub use windows_impl::{apply, restore, restore_left_over};

#[cfg(not(windows))]
pub fn apply() {}
#[cfg(not(windows))]
pub fn restore() {}
#[cfg(not(windows))]
pub fn restore_left_over() {}

/// The classic arrow, tip at the hotspot, on a 32px cursor. Scaled to whatever size Windows uses.
const ARROW: [(f64, f64); 7] = [(1.0, 1.0), (1.0, 19.0), (5.5, 14.8), (8.6, 21.6), (11.4, 20.4), (8.4, 13.8), (14.2, 13.8)];
const FILL: [u8; 3] = [0x2f, 0xb3, 0x6b]; // Aloe moss, bright enough for dark backgrounds.
const OUTLINE: [u8; 3] = [0xff, 0xff, 0xff]; // White edge, as on the stock arrow, for light ones.
const OUTLINE_WIDTH: f64 = 1.4;

/// The arrow as straight-alpha RGBA, `size`×`size`, 4×4 supersampled so the edges are smooth.
fn arrow_rgba(size: usize) -> Vec<u8> {
    let scale = size as f64 / 32.0;
    let polygon: Vec<(f64, f64)> = ARROW.iter().map(|&(x, y)| (x * scale, y * scale)).collect();
    let outline = OUTLINE_WIDTH * scale;
    const SAMPLES: usize = 4;
    let mut rgba = vec![0u8; size * size * 4];
    for py in 0..size {
        for px in 0..size {
            let (mut fill, mut edge) = (0usize, 0usize);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = px as f64 + (sx as f64 + 0.5) / SAMPLES as f64;
                    let y = py as f64 + (sy as f64 + 0.5) / SAMPLES as f64;
                    if !inside(&polygon, x, y) {
                        continue;
                    }
                    if distance_to_edge(&polygon, x, y) < outline {
                        edge += 1;
                    } else {
                        fill += 1;
                    }
                }
            }
            let covered = fill + edge;
            if covered == 0 {
                continue;
            }
            let mix = |f: u8, o: u8| ((f as usize * fill + o as usize * edge) / covered) as u8;
            let i = (py * size + px) * 4;
            rgba[i] = mix(FILL[0], OUTLINE[0]);
            rgba[i + 1] = mix(FILL[1], OUTLINE[1]);
            rgba[i + 2] = mix(FILL[2], OUTLINE[2]);
            rgba[i + 3] = (covered * 255 / (SAMPLES * SAMPLES)) as u8;
        }
    }
    rgba
}

fn inside(polygon: &[(f64, f64)], x: f64, y: f64) -> bool {
    let mut inside = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let ((xi, yi), (xj, yj)) = (polygon[i], polygon[j]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn distance_to_edge(polygon: &[(f64, f64)], x: f64, y: f64) -> f64 {
    let mut best = f64::MAX;
    for i in 0..polygon.len() {
        let (ax, ay) = polygon[i];
        let (bx, by) = polygon[(i + 1) % polygon.len()];
        let (dx, dy) = (bx - ax, by - ay);
        let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        best = best.min(((x - ax - t * dx).powi(2) + (y - ay - t * dy).powi(2)).sqrt());
    }
    best
}

#[cfg(windows)]
mod windows_impl {
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicBool, Ordering},
    };
    use windows_sys::Win32::{
        Graphics::Gdi::{CreateBitmap, DeleteObject},
        UI::WindowsAndMessaging::{
            CreateIconIndirect, GetSystemMetrics, SetSystemCursor, SystemParametersInfoW, ICONINFO, OCR_APPSTARTING,
            OCR_HAND, OCR_IBEAM, OCR_NORMAL, SM_CXCURSOR, SPI_SETCURSORS,
        },
    };

    use crate::config::{config_path, debug_log};

    /// The shapes a pointer usually takes over other apps. All become the green arrow, so the
    /// pointer reads as Aloe's wherever it is.
    const SWAPPED: [u32; 4] = [OCR_NORMAL, OCR_IBEAM, OCR_HAND, OCR_APPSTARTING];

    static APPLIED: AtomicBool = AtomicBool::new(false);

    fn marker() -> Option<PathBuf> {
        config_path().ok().map(|path| path.with_file_name("pointer-swapped"))
    }

    pub fn apply() {
        if APPLIED.swap(true, Ordering::SeqCst) {
            return;
        }
        // Written first: if anything below crashes, the next launch still knows to restore.
        if let Some(path) = marker() {
            let _ = std::fs::write(path, b"");
        }
        let size = match unsafe { GetSystemMetrics(SM_CXCURSOR) } {
            n if n > 0 => n as usize,
            _ => 32,
        };
        let bgra: Vec<u8> = super::arrow_rgba(size).chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        for id in SWAPPED {
            // SetSystemCursor takes ownership of (and later destroys) the handle, so each shape
            // needs a cursor of its own.
            let cursor = unsafe { create_cursor(size, &bgra) };
            if cursor.is_null() || unsafe { SetSystemCursor(cursor, id) } == 0 {
                debug_log("cursor", "swap_failed", format!("id={id}"));
            }
        }
    }

    pub fn restore() {
        if !APPLIED.swap(false, Ordering::SeqCst) {
            return;
        }
        reload_user_scheme();
    }

    /// A swap a crashed run never undid.
    pub fn restore_left_over() {
        if marker().is_some_and(|path| path.exists()) {
            debug_log("cursor", "restore_left_over", "");
            reload_user_scheme();
        }
    }

    fn reload_user_scheme() {
        unsafe { SystemParametersInfoW(SPI_SETCURSORS, 0, std::ptr::null_mut(), 0) };
        if let Some(path) = marker() {
            let _ = std::fs::remove_file(path);
        }
    }

    unsafe fn create_cursor(size: usize, bgra: &[u8]) -> *mut core::ffi::c_void {
        // 1bpp rows are word-aligned. An all-zero AND mask lets the colour bitmap's alpha decide.
        let mask_bits = vec![0u8; size.div_ceil(16) * 2 * size];
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, mask_bits.as_ptr().cast());
        let color = CreateBitmap(size as i32, size as i32, 1, 32, bgra.as_ptr().cast());
        // The arrow's tip, wherever ARROW put it at this size.
        let tip = (super::ARROW[0].0 * size as f64 / 32.0).round() as u32;
        let info = ICONINFO { fIcon: 0, xHotspot: tip, yHotspot: tip, hbmMask: mask, hbmColor: color };
        let cursor = CreateIconIndirect(&info);
        DeleteObject(mask);
        DeleteObject(color);
        cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_is_green_inside_and_clear_outside() {
        let rgba = arrow_rgba(32);
        let at = |x: usize, y: usize| &rgba[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4];
        assert_eq!(at(4, 10), [FILL[0], FILL[1], FILL[2], 255]);
        assert_eq!(at(25, 5)[3], 0);
        assert_eq!(at(1, 30)[3], 0);
    }

    /// Manual check: `cargo test pointer_turns_green_and_back -- --ignored` turns the real pointer
    /// green for two seconds, then restores it.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn pointer_turns_green_and_back() {
        apply();
        std::thread::sleep(std::time::Duration::from_secs(2));
        restore();
        let marker = crate::config::config_path().unwrap().with_file_name("pointer-swapped");
        assert!(!marker.exists());
    }
}

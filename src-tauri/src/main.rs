#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    enable_wayland_software_rendering();

    aloe_desktop_lib::run();
}

#[cfg(target_os = "linux")]
fn enable_wayland_software_rendering() {
    if !is_wayland_session() {
        return;
    }

    // WebKitGTK's DMA-BUF/GBM compositing path can be unreliable on Wayland,
    // especially with proprietary NVIDIA and hybrid graphics drivers. This env
    // var must be set before WebKit initializes, so do it before Tauri starts.
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    std::env::set_var("ALOE_SOFTWARE_RENDERING_ALERT", "1");
}

#[cfg(target_os = "linux")]
fn is_wayland_session() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|session_type| session_type.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false)
        || std::env::var_os("WAYLAND_DISPLAY")
            .filter(|display| !display.is_empty())
            .is_some()
}

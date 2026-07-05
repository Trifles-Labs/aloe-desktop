#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // WebKitGTK's DMA-BUF/GBM compositing path is unreliable against the proprietary
    // NVIDIA driver on Intel+NVIDIA hybrid-graphics laptops: it's a fatal Wayland
    // protocol error natively ("Gdk-Message: Error 71"), or a silently blank window
    // (failed GBM buffer allocation) if GTK is forced onto X11 instead. Pinning the
    // render device to the Intel GPU (via the undocumented
    // WEBKIT_WEB_RENDER_DEVICE_FILE) avoids it sometimes, but proved flaky under
    // repeated testing — so fall back to WebKit's software renderer, which is the
    // workaround WebKitGTK itself recommends for this class of NVIDIA bug.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    aloe_desktop_lib::run();
}

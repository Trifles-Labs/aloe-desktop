use notify_rust::Notification;
use tauri::AppHandle;

use crate::desktop::show_main_window;

pub fn show_clickable(app: &AppHandle, title: &str, body: &str) -> Result<(), String> {
    let mut notification = Notification::new();
    notification.summary(title).body(body).auto_icon().action("default", "Open Aloe");
    #[cfg(windows)]
    notification.app_id("com.aloe.desktop");
    let handle = notification.show().map_err(|error| error.to_string())?;
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        handle.wait_for_action(|action| {
            if action == "default" {
                show_main_window(&app);
            }
        });
    });
    Ok(())
}

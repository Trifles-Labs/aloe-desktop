use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, AppHandle, Manager, Runtime,
};

use crate::config::AppState;

pub const TRAY_ID: &str = "aloe-tray";
const OPEN_ID: &str = "tray-open";
const QUIT_ID: &str = "tray-quit";

pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn hide_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

fn tray_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let config = app.state::<AppState>().config.lock().expect("config mutex").clone();
    let name = config.user_profile.as_ref().map(|user| user.name.as_str()).unwrap_or("Not signed in");
    let email = config.user_profile.as_ref().map(|user| user.email.as_str()).unwrap_or("No account connected");
    let connected = config.socket_status == "connected";

    let user_item = MenuItem::with_id(app, "tray-user", name, false, None::<&str>)?;
    let email_item = MenuItem::with_id(app, "tray-email", email, false, None::<&str>)?;
    let status_item = MenuItem::with_id(app, "tray-status", if connected { "Agent: Connected" } else { "Agent: Disconnected" }, false, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let open = MenuItem::with_id(app, OPEN_ID, "Open Aloe", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ID, "Quit", true, None::<&str>)?;
    Menu::with_items(app, &[&user_item, &email_item, &status_item, &separator, &open, &quit])
}

pub fn refresh_tray_menu<R: Runtime>(app: &AppHandle<R>) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(menu) = tray_menu(app) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

pub fn install_tray(app: &mut App) -> tauri::Result<()> {
    let menu = tray_menu(app.handle())?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Aloe Desktop");
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

pub fn handle_tray_event(app: &AppHandle, event: TrayIconEvent) {
    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
        show_main_window(app);
    }
}

pub fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        OPEN_ID => show_main_window(app),
        QUIT_ID => app.exit(0),
        _ => {}
    }
}

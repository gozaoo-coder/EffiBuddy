//! Window commands.

use crate::core::window_mgr::WindowManager;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub fn create_window(
    app: AppHandle,
    mgr: State<'_, WindowManager>,
    label: String,
    title: String,
    url: String,
    width: u32,
    height: u32,
) -> Result<(), String> {
    mgr.create_overlay(&app, &label, &title, &url, width, height)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn close_window(app: AppHandle, mgr: State<'_, WindowManager>, label: String) -> Result<(), String> {
    mgr.close(&app, &label).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn show_window(app: AppHandle, mgr: State<'_, WindowManager>, label: String) -> Result<(), String> {
    mgr.show(&app, &label).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn hide_window(app: AppHandle, mgr: State<'_, WindowManager>, label: String) -> Result<(), String> {
    mgr.hide(&app, &label).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_always_on_top(
    app: AppHandle,
    mgr: State<'_, WindowManager>,
    label: String,
    top: bool,
) -> Result<(), String> {
    mgr.set_always_on_top(&app, &label, top).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn start_dragging(app: AppHandle, label: String) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(&label) {
        win.start_dragging().map_err(|e| e.to_string())
    } else {
        Err("window not found".into())
    }
}

//! Boot autostart helper. Wraps tauri-plugin-autostart.

use anyhow::Result;
use tauri_plugin_autostart::{MacosLauncher, Manager};

pub fn enable(app: &tauri::AppHandle) -> Result<()> {
    let mgr = app.autolaunch();
    if !mgr.is_enabled().unwrap_or(false) {
        mgr.enable()?;
    }
    Ok(())
}

pub fn disable(app: &tauri::AppHandle) -> Result<()> {
    app.autolaunch().disable()?;
    Ok(())
}

pub fn is_enabled(app: &tauri::AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[allow(dead_code)]
fn _link_macos_launcher() -> MacosLauncher {
    MacosLauncher::LaunchAgent
}

//! Multi-window lifecycle manager. Tracks open windows and provides
//! helpers to create/show/hide them by label.

use parking_lot::Mutex;
use std::collections::HashSet;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

#[derive(Default)]
pub struct WindowManager {
    open: Mutex<HashSet<String>>,
}

impl WindowManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self, label: &str) -> bool {
        self.open.lock().contains(label)
    }

    pub fn register(&self, label: impl Into<String>) {
        self.open.lock().insert(label.into());
    }

    pub fn unregister(&self, label: &str) {
        self.open.lock().remove(label);
    }

    /// Create a transparent, always-on-top, frameless window.
    pub fn create_overlay(
        &self,
        app: &AppHandle,
        label: &str,
        title: &str,
        url: &str,
        width: u32,
        height: u32,
    ) -> tauri::Result<()> {
        if self.is_open(label) || app.get_webview_window(label).is_some() {
            self.show(app, label)?;
            return Ok(());
        }
        let win = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
            .title(title)
            .inner_size(width as f64, height as f64)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false)
            .build()?;
        let label_owned = label.to_string();
        win.on_window_event(move |e| {
            if matches!(e, tauri::WindowEvent::Destroyed) {
                log::debug!("window destroyed: {label_owned}");
            }
        });
        self.register(label);
        Ok(())
    }

    pub fn show(&self, app: &AppHandle, label: &str) -> tauri::Result<()> {
        if let Some(win) = app.get_webview_window(label) {
            win.show()?;
            win.set_focus()?;
        }
        Ok(())
    }

    pub fn hide(&self, app: &AppHandle, label: &str) -> tauri::Result<()> {
        if let Some(win) = app.get_webview_window(label) {
            win.hide()?;
        }
        Ok(())
    }

    pub fn close(&self, app: &AppHandle, label: &str) -> tauri::Result<()> {
        if let Some(win) = app.get_webview_window(label) {
            win.close()?;
        }
        self.unregister(label);
        Ok(())
    }

    pub fn set_always_on_top(
        &self,
        app: &AppHandle,
        label: &str,
        top: bool,
    ) -> tauri::Result<()> {
        if let Some(win) = app.get_webview_window(label) {
            win.set_always_on_top(top)?;
        }
        Ok(())
    }
}

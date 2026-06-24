//! CoreContext: capabilities a plugin can call on Core.
//!
//! MVP version: thin handle holding plugin id + a list of pending requests
//! the plugin wants Core to perform (register shortcut, open window, emit
//! event). Core drains this list after `on_enable` / `handle_event`.

use serde::{Deserialize, Serialize};

/// A request a plugin makes to Core.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload")]
pub enum CoreRequest {
    /// Register a global shortcut (accelerator string, e.g. "Ctrl+Space").
    RegisterShortcut { accelerator: String },
    /// Open a window by preset name (dock / widget / store / settings) or
    /// a custom label.
    OpenWindow { label: String },
    /// Emit an event on the global event bus.
    EmitEvent { name: String, payload: serde_json::Value },
    /// Log a message to Core's logger.
    Log { level: String, message: String },
}

/// Handle passed to plugins. Plugins push requests; Core reads them.
pub struct CoreContext {
    pub plugin_id: String,
    pub requests: Vec<CoreRequest>,
}

impl CoreContext {
    pub fn new(plugin_id: impl Into<String>) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            requests: Vec::new(),
        }
    }

    pub fn request(&mut self, req: CoreRequest) {
        self.requests.push(req);
    }

    pub fn drain(&mut self) -> Vec<CoreRequest> {
        std::mem::take(&mut self.requests)
    }

    // Convenience helpers -----------------------------------------------

    pub fn register_shortcut(&mut self, accelerator: impl Into<String>) {
        self.request(CoreRequest::RegisterShortcut {
            accelerator: accelerator.into(),
        });
    }

    pub fn open_window(&mut self, label: impl Into<String>) {
        self.request(CoreRequest::OpenWindow {
            label: label.into(),
        });
    }

    pub fn emit(&mut self, name: impl Into<String>, payload: serde_json::Value) {
        self.request(CoreRequest::EmitEvent {
            name: name.into(),
            payload,
        });
    }

    pub fn log(&mut self, level: impl Into<String>, message: impl Into<String>) {
        self.request(CoreRequest::Log {
            level: level.into(),
            message: message.into(),
        });
    }
}

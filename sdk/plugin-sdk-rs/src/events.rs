//! Event types flowing between Core and plugins.

use serde::{Deserialize, Serialize};

/// Events Core dispatches to plugins.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload")]
pub enum CoreEvent {
    /// Ticked by Core's internal timer (e.g. every second).
    Tick { ts_ms: i64 },
    /// A registered global shortcut was pressed.
    Shortcut { accelerator: String },
    /// A custom event emitted by another plugin or the frontend.
    Custom { name: String, payload: serde_json::Value },
    /// System power / session event.
    System { name: String },
}

/// Responses a plugin can return from `handle_event`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload")]
pub enum PluginResponse {
    /// JSON payload to forward to the frontend via the event bus.
    Frontend { event: String, payload: serde_json::Value },
    /// Plain log line.
    Log { level: String, message: String },
    /// No-op ack.
    Ack,
}

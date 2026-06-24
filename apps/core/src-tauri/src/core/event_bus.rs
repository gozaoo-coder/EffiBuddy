//! Global event bus. Wraps Tauri's emit/listen with a typed in-process
//! channel for plugin-to-plugin and backend-to-frontend messaging.

use parking_lot::Mutex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

/// Topic -> list of subscriber callbacks.
type Subscribers = HashMap<String, Vec<Box<dyn Fn(Value) + Send + Sync>>>;

pub struct EventBus {
    subscribers: Arc<Mutex<Subscribers>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            subscribers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Subscribe to a topic. Returns a guard that unsubscribes on drop.
    pub fn subscribe<'a, F>(&'a self, topic: impl Into<String>, f: F) -> Subscription<'a>
    where
        F: Fn(Value) + Send + Sync + 'static,
    {
        let topic = topic.into();
        self.subscribers
            .lock()
            .entry(topic.clone())
            .or_default()
            .push(Box::new(f));
        Subscription { bus: self, topic }
    }

    /// Dispatch a payload to all in-process subscribers AND emit to the
    /// frontend via Tauri (so Vue windows can listen too).
    pub fn dispatch(&self, app: &AppHandle, topic: &str, payload: Value) {
        if let Some(subs) = self.subscribers.lock().get(topic) {
            for cb in subs.iter() {
                cb(payload.clone());
            }
        }
        let _ = app.emit(topic, payload);
    }
}

pub struct Subscription<'a> {
    bus: &'a EventBus,
    topic: String,
}

impl<'a> Drop for Subscription<'a> {
    fn drop(&mut self) {
        // Note: this removes the last pushed callback; for MVP we accept
        // the imprecision rather than tracking subscription ids.
        if let Some(subs) = self.bus.subscribers.lock().get_mut(&self.topic) {
            subs.pop();
        }
    }
}

/// Convenience accessor used by commands.
pub fn bus(app: &AppHandle) -> &EventBus {
    app.state::<EventBus>().inner()
}

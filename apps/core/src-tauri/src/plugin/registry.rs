//! Plugin registry. Tracks installed manifests + loaded backends + enabled
//! state. Thread-safe via parking_lot.

use crate::plugin::loader::SharedLoaded;
use crate::plugin::manifest::Manifest;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct PluginEntry {
    pub manifest: Manifest,
    pub enabled: bool,
    pub loaded: SharedLoaded,
}

pub struct PluginRegistry {
    entries: RwLock<HashMap<String, Arc<RwLock<PluginEntry>>>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub fn register(&self, manifest: Manifest) -> Arc<RwLock<PluginEntry>> {
        let entry = Arc::new(RwLock::new(PluginEntry {
            enabled: false,
            loaded: Arc::new(parking_lot::Mutex::new(None)),
            manifest,
        }));
        self.entries
            .write()
            .insert(entry.read().manifest.id.clone(), entry.clone());
        entry
    }

    pub fn unregister(&self, id: &str) {
        self.entries.write().remove(id);
    }

    pub fn get(&self, id: &str) -> Option<Arc<RwLock<PluginEntry>>> {
        self.entries.read().get(id).cloned()
    }

    pub fn list(&self) -> Vec<Arc<RwLock<PluginEntry>>> {
        self.entries.read().values().cloned().collect()
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> bool {
        if let Some(e) = self.get(id) {
            e.write().enabled = enabled;
            true
        } else {
            false
        }
    }
}

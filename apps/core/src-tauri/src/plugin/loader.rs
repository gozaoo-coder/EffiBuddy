//! Dynamic library loader. Uses `libloading` to open a plugin's backend
//! `.dll`/`.so`/`.dylib` and call `_plugin_init`.

use crate::traits::{PluginInitFn, PluginTrait};
use anyhow::{anyhow, Context, Result};
use libloading::{Library, Symbol};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;

/// Loaded plugin backend: holds the dynamic library + the trait object.
pub struct LoadedPlugin {
    pub id: String,
    pub _lib: Library,
    pub instance: Option<*mut dyn PluginTrait>,
}

unsafe impl Send for LoadedPlugin {}
unsafe impl Sync for LoadedPlugin {}

impl LoadedPlugin {
    pub fn instance(&self) -> &mut dyn PluginTrait {
        unsafe { &mut *self.instance.expect("plugin instance already dropped") }
    }

    pub fn drop_instance(&mut self) {
        if let Some(ptr) = self.instance.take() {
            unsafe { drop(Box::from_raw(ptr)) };
        }
    }
}

impl Drop for LoadedPlugin {
    fn drop(&mut self) {
        self.drop_instance();
    }
}

/// Load a backend dynamic library and call `_plugin_init`.
///
/// # Safety
/// The library must be built against the same `plugin-sdk-rs` version as Core.
pub unsafe fn load(path: PathBuf, expected_id: &str) -> Result<LoadedPlugin> {
    let lib = Library::new(&path)
        .with_context(|| format!("load plugin lib {}", path.display()))?;
    let init: Symbol<PluginInitFn> = lib
        .get(b"_plugin_init")
        .context("symbol _plugin_init not found")?;
    let instance = init();
    if instance.is_null() {
        return Err(anyhow!("_plugin_init returned null"));
    }
    let id = (*instance).id().to_string();
    if id != expected_id {
        return Err(anyhow!(
            "plugin id mismatch: manifest={expected_id} lib={id}"
        ));
    }
    Ok(LoadedPlugin {
        id,
        _lib: lib,
        instance: Some(instance),
    })
}

/// Shared handle stored in the registry.
pub type SharedLoaded = Arc<Mutex<Option<LoadedPlugin>>>;

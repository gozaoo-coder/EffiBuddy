//! Plugin lifecycle: install / enable / disable / uninstall + autoload.

use crate::core::config::packages_dir;
use crate::plugin::loader::{self, LoadedPlugin};
use crate::plugin::manifest::Manifest;
use crate::plugin::permissions;
use crate::plugin::registry::PluginRegistry;
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

/// Scan the packages dir and register every manifest found.
pub async fn autoload_enabled() -> Result<()> {
    let dir = packages_dir()?;
    if !dir.exists() {
        return Ok(());
    }
    let mut entries = vec![];
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let manifest_path = entry.path().join("manifest.json");
        if manifest_path.exists() {
            entries.push(manifest_path);
        }
    }
    for mp in entries {
        match Manifest::from_path(&mp) {
            Ok(m) => {
                log::info!("autoload manifest: {} ({})", m.id, m.version);
                let _ = permissions::validate(&m);
                // Registry is process-local; we register lazily on enable.
                registry_handle().register(m);
            }
            Err(e) => log::warn!("skip manifest {}: {e}", mp.display()),
        }
    }
    Ok(())
}

/// Install a package from an already-extracted directory.
pub fn install_from_dir(src: PathBuf) -> Result<Manifest> {
    let manifest_path = src.join("manifest.json");
    let manifest = Manifest::from_path(&manifest_path)?;
    permissions::validate(&manifest)?;
    let dest = Manifest::dir_for(&manifest.id)?;
    if dest.exists() {
        return Err(anyhow!("package {} already installed", manifest.id));
    }
    copy_dir_recursive(&src, &dest)?;
    log::info!("installed {} -> {}", manifest.id, dest.display());
    Ok(manifest)
}

/// Enable a plugin: load backend (if any) + call on_enable.
pub fn enable(app: &AppHandle, id: &str) -> Result<()> {
    let reg = registry_handle();
    let entry = reg
        .get(id)
        .ok_or_else(|| anyhow!("plugin not found: {id}"))?;
    {
        let e = entry.read();
        if e.enabled {
            return Ok(());
        }
    }

    let manifest = entry.read().manifest.clone();
    let mut loaded: Option<LoadedPlugin> = None;
    if let Some(backend) = &manifest.entry.backend {
        let lib_path = Manifest::dir_for(id)?.join(backend);
        if !lib_path.exists() {
            return Err(anyhow!("backend lib missing: {}", lib_path.display()));
        }
        unsafe {
            loaded = Some(loader::load(lib_path, id)?);
        }
    }

    let mut ctx = plugin_sdk_rs::CoreContext::new(id);
    if let Some(ref mut p) = loaded {
        p.instance().on_enable(&mut ctx);
    }
    drain_ctx(app, &ctx);

    if let Some(p) = loaded {
        *entry.read().loaded.lock() = Some(p);
    }
    reg.set_enabled(id, true);
    log::info!("enabled plugin {id}");
    Ok(())
}

/// Disable a plugin: call on_disable + drop backend.
pub fn disable(_app: &AppHandle, id: &str) -> Result<()> {
    let reg = registry_handle();
    let entry = reg.get(id).ok_or_else(|| anyhow!("plugin not found: {id}"))?;
    {
        let entry_guard = entry.read();
        let mut guard = entry_guard.loaded.lock();
        if let Some(p) = guard.as_mut() {
            p.instance().on_disable();
        }
        *guard = None;
    }
    reg.set_enabled(id, false);
    log::info!("disabled plugin {id}");
    Ok(())
}

/// Uninstall: disable + remove dir + unregister.
pub fn uninstall(id: &str) -> Result<()> {
    let reg = registry_handle();
    let _ = reg.get(id); // existence check
    let dir = Manifest::dir_for(id)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .with_context(|| format!("remove {}", dir.display()))?;
    }
    reg.unregister(id);
    log::info!("uninstalled {id}");
    Ok(())
}

fn drain_ctx(app: &AppHandle, ctx: &plugin_sdk_rs::CoreContext) {
    use plugin_sdk_rs::CoreRequest;
    for req in &ctx.requests {
        match req {
            CoreRequest::RegisterShortcut { accelerator } => {
                log::info!("plugin {} wants shortcut {accelerator}", ctx.plugin_id);
                // Real registration wired in P5 (global-shortcut plugin).
            }
            CoreRequest::OpenWindow { label } => {
                if let Some(win) = app.get_webview_window(label) {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
            CoreRequest::EmitEvent { name, payload } => {
                let _ = app.emit(name, payload.clone());
            }
            CoreRequest::Log { level, message } => {
                log::info!("[{}] {}: {message}", ctx.plugin_id, level);
            }
        }
    }
}

fn copy_dir_recursive(src: &PathBuf, dest: &PathBuf) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Process-wide registry handle. For MVP we use a global; in production
/// this would live in Tauri state. The global is fine because Core is a
/// single process.
static REGISTRY: parking_lot::Mutex<Option<Arc<PluginRegistry>>> = parking_lot::const_mutex(None);

pub fn registry_handle() -> Arc<PluginRegistry> {
    let mut guard = REGISTRY.lock();
    if guard.is_none() {
        *guard = Some(Arc::new(PluginRegistry::new()));
    }
    guard.as_ref().unwrap().clone()
}

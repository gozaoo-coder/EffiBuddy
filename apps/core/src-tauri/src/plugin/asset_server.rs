//! Asset server: serves plugin frontend static assets so the Core WebView
//! can `import()` plugin Vue bundles via a stable URL scheme.
//!
//! MVP: maps `plugin://<id>/<path>` to `<packages_dir>/<id>/frontend/<path>`
//! and returns file bytes. A real implementation would register a Tauri
//! URI scheme protocol; here we expose a resolver used by the frontend
//! via a Tauri command.

use crate::core::config::packages_dir;
use crate::plugin::manifest::Manifest;
use anyhow::Result;
use std::path::PathBuf;

pub struct AssetServer;

impl AssetServer {
    pub fn new() -> Self {
        Self
    }

    /// Resolve a plugin asset path to an absolute filesystem path.
    pub fn resolve(plugin_id: &str, rel_path: &str) -> Result<PathBuf> {
        let base = packages_dir()?.join(plugin_id).join("frontend");
        let resolved = base.join(rel_path);
        // Prevent path traversal.
        let canonical = resolved.canonicalize().unwrap_or(resolved.clone());
        if !canonical.starts_with(base.canonicalize().unwrap_or(base)) {
            anyhow::bail!("path traversal blocked: {rel_path}");
        }
        Ok(canonical)
    }

    /// Read a plugin asset as bytes.
    pub fn read(plugin_id: &str, rel_path: &str) -> Result<Vec<u8>> {
        let p = Self::resolve(plugin_id, rel_path)?;
        Ok(std::fs::read(&p)?)
    }

    /// Frontend entry URL for a plugin (used by Vue async component loader).
    pub fn entry_url(plugin_id: &str, entry: &str) -> String {
        format!("/plugin-asset/{plugin_id}/{entry}")
    }

    /// Directory containing a plugin's frontend bundle.
    pub fn frontend_dir(plugin_id: &str) -> Result<PathBuf> {
        Ok(Manifest::dir_for(plugin_id)?.join("frontend"))
    }
}

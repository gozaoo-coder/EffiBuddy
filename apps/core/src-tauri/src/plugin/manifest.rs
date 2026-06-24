//! manifest.json parsing + validation.

use crate::core::config::packages_dir;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(rename = "core_version")]
    pub core_version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub entry: Entry,
    #[serde(default)]
    pub widgets: Vec<WidgetDecl>,
    #[serde(default)]
    pub hooks: Hooks,
    #[serde(default)]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub frontend: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WidgetDecl {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub default_size: Option<[u32; 2]>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Hooks {
    #[serde(default)]
    pub on_install: Option<String>,
    #[serde(default)]
    pub on_uninstall: Option<String>,
}

impl Manifest {
    pub fn from_path(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read manifest at {}", path.display()))?;
        let m: Manifest = serde_json::from_str(&raw).context("parse manifest")?;
        m.validate()?;
        Ok(m)
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty() {
            return Err(anyhow!("manifest.id is empty"));
        }
        if !self.id.contains('.') {
            return Err(anyhow!("manifest.id must be reverse-DNS (got {})", self.id));
        }
        if self.name.is_empty() {
            return Err(anyhow!("manifest.name is empty"));
        }
        if self.version.is_empty() {
            return Err(anyhow!("manifest.version is empty"));
        }
        if self.core_version.is_empty() {
            return Err(anyhow!("manifest.core_version is empty"));
        }
        Ok(())
    }

    /// Resolve the package directory for a given plugin id.
    pub fn dir_for(id: &str) -> Result<PathBuf> {
        Ok(packages_dir()?.join(id))
    }

    pub fn has_backend(&self) -> bool {
        self.entry.backend.is_some()
    }

    pub fn has_frontend(&self) -> bool {
        self.entry.frontend.is_some()
    }
}

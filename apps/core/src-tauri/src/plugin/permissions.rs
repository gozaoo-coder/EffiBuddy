//! Permission model. MVP: manifest-declared permissions + simple allow-list.

use crate::plugin::manifest::Manifest;
use anyhow::{anyhow, Result};

pub const KNOWN: &[&str] = &[
    "none",
    "filesystem",
    "global-shortcut",
    "network",
    "native-api",
];

pub fn validate(manifest: &Manifest) -> Result<()> {
    for p in &manifest.permissions {
        if !KNOWN.contains(&p.as_str()) {
            return Err(anyhow!("unknown permission: {p}"));
        }
    }
    Ok(())
}

/// True if the manifest requests a sensitive (system-level) permission
/// that requires explicit user consent at install time.
pub fn requires_consent(manifest: &Manifest) -> bool {
    manifest
        .permissions
        .iter()
        .any(|p| matches!(p.as_str(), "native-api" | "global-shortcut"))
}

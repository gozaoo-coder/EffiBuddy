//! Package management commands.

use crate::plugin::lifecycle;
use crate::plugin::manifest::Manifest;
use crate::plugin::registry::PluginEntry;
use serde::Serialize;
use std::sync::Arc;
use tauri::AppHandle;

#[derive(Debug, Serialize)]
pub struct PackageInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub permissions: Vec<String>,
    pub enabled: bool,
    pub has_backend: bool,
    pub has_frontend: bool,
    pub widgets: Vec<WidgetInfo>,
}

#[derive(Debug, Serialize)]
pub struct WidgetInfo {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub default_size: Option<[u32; 2]>,
}

impl From<&Arc<parking_lot::RwLock<PluginEntry>>> for PackageInfo {
    fn from(entry: &Arc<parking_lot::RwLock<PluginEntry>>) -> Self {
        let e = entry.read();
        let m = &e.manifest;
        PackageInfo {
            id: m.id.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            description: m.description.clone(),
            author: m.author.clone(),
            permissions: m.permissions.clone(),
            enabled: e.enabled,
            has_backend: m.has_backend(),
            has_frontend: m.has_frontend(),
            widgets: m
                .widgets
                .iter()
                .map(|w| WidgetInfo {
                    kind: w.kind.clone(),
                    name: w.name.clone(),
                    default_size: w.default_size,
                })
                .collect(),
        }
    }
}

#[tauri::command]
pub fn list_packages() -> Vec<PackageInfo> {
    let reg = lifecycle::registry_handle();
    reg.list().iter().map(PackageInfo::from).collect()
}

#[tauri::command]
pub fn install_package(src_dir: String) -> Result<PackageInfo, String> {
    let manifest = lifecycle::install_from_dir(std::path::PathBuf::from(src_dir))
        .map_err(|e| e.to_string())?;
    let reg = lifecycle::registry_handle();
    reg.register(manifest.clone());
    let entry = reg
        .get(&manifest.id)
        .ok_or("registration failed")?;
    Ok(PackageInfo::from(&entry))
}

#[tauri::command]
pub fn uninstall_package(id: String) -> Result<(), String> {
    lifecycle::uninstall(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn enable_plugin(app: AppHandle, id: String) -> Result<(), String> {
    lifecycle::enable(&app, &id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn disable_plugin(app: AppHandle, id: String) -> Result<(), String> {
    lifecycle::disable(&app, &id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_plugin_manifest(id: String) -> Result<Manifest, String> {
    let reg = lifecycle::registry_handle();
    let entry = reg.get(&id).ok_or("plugin not found")?;
    let manifest = entry.read().manifest.clone();
    Ok(manifest)
}

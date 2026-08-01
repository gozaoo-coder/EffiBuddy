//! P2P 命令：设备扫描、列表与配对。

use effisuite_core::Device;

use crate::state::AppState;

#[tauri::command]
pub(crate) async fn scan_devices(
    state: tauri::State<'_, AppState>,
    _app_handle: tauri::AppHandle,
) -> Result<Vec<Device>, String> {
    let p2p = state.p2p.clone();
    p2p.scan_once().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn get_devices(state: tauri::State<'_, AppState>) -> Result<Vec<Device>, String> {
    let p2p = state.p2p.clone();
    Ok(p2p.list_devices().await)
}

#[tauri::command]
pub(crate) async fn pair_device(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let p2p = state.p2p.clone();
    p2p.pair_device(&id).await.map_err(|e| e.to_string())
}

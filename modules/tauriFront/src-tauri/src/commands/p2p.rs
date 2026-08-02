//! P2P 命令：设备扫描、列表与配对。
//!
//! P2pManager 的 `scan_once` / `list_devices` 来自 [`DiscoveryService`] trait，
//! `accept_pair` 来自 [`PairingService`] trait，需显式 import trait 才能用方法语法调用。
//! `pair_device` 命令语义：对已发现设备发起配对（默认镜像角色 Mirror）。

use effisuite_core::Device;
use effisuite_p2p::{DiscoveryService, PairingService};
use effisuite_p2p::trust::PairRole;

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

/// 对已发现设备发起配对。
///
/// 调用 [`PairingService::accept_pair`]，默认使用镜像角色（Mirror）：
/// 双向同步聊天/插件/用户缓存。若需主机/副本模式，未来可扩展为接收 role 参数。
#[tauri::command]
pub(crate) async fn pair_device(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let p2p = state.p2p.clone();
    p2p.accept_pair(&id, PairRole::Mirror)
        .await
        .map_err(|e| e.to_string())
}

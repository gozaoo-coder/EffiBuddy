//! P2P 命令：设备发现、配对、同步与状态查询。
//!
//! P2pManager 的方法来自三个 trait（需显式 import 才能用方法语法）：
//! - [`DiscoveryService`]：`scan_once` / `list_devices` / `start_discovery` / `stop_discovery`
//! - [`PairingService`]：`pair_by_address` / `accept_pair` / `reject_pair` / `unpair`
//! - [`SyncService`]：`pull` / `push` / `sync_cursor`
//!
//! 另有 `pending_pairing_requests` / `is_started` / `self_device_id` / `stop` 为 P2pManager 内置方法。
//!
//! # 命令一览
//! - `scan_devices`：触发一次 UDP 广播扫描
//! - `get_devices`：返回可信 + 在线状态合并列表
//! - `get_online_devices`：返回当前在线且已配对的设备（dispatch_remote_task 用）
//! - `pair_by_address`：通过 IP/链接直连配对（方法一）
//! - `pair_device`：对已发现设备发起配对（方法二，接受 role 参数）
//! - `reject_pair`：拒绝配对请求
//! - `unpair`：取消已配对设备
//! - `start_discovery`：启动后台持续发现
//! - `stop_discovery`：停止后台持续发现
//! - `pending_pairing_requests`：列出待处理配对请求（前端展示 bubble）
//! - `sync_pull`：从指定设备拉取数据（镜像模式）
//! - `sync_push`：向指定设备推送数据（镜像模式）
//! - `sync_cursor`：查询与指定设备的同步进度
//! - `get_p2p_status`：查询 P2P 服务状态（started / self_device_id）
//! - `stop_p2p`：停止 P2P 服务（关闭所有连接与监听）

use serde::Serialize;

use effisuite_core::Device;
use effisuite_p2p::trust::PairRole;
use effisuite_p2p::{
    DiscoveryService, PairingRequest, PairingService, SyncKind, SyncService,
};

use crate::state::AppState;

/// P2P 服务状态（供前端展示连接状态与设备 id）
#[derive(Debug, Serialize)]
pub(crate) struct P2pStatus {
    /// 是否已启动（trust store 已加载 + transport/discovery/pairing/sync 已运行）
    pub started: bool,
    /// 本机设备 id（启动前为占位 `dev-anon-xxxx`）
    pub self_device_id: String,
}

/// 解析前端传入的角色字符串为 PairRole。
/// 支持 "mirror" / "host" / "replica"（大小写不敏感），默认 mirror。
fn parse_role(role: &str) -> PairRole {
    match role.trim().to_ascii_lowercase().as_str() {
        "host" => PairRole::Host,
        "replica" => PairRole::Replica,
        _ => PairRole::Mirror,
    }
}

/// 解析前端传入的同步种类字符串数组为 SyncKind 列表。
/// 支持 "conversations" / "plugins" / "user_cache"（snake_case，与枚举 serde 一致）。
/// 空列表视为全部种类。
fn parse_kinds(kinds: &[String]) -> Vec<SyncKind> {
    if kinds.is_empty() {
        return vec![SyncKind::Conversations, SyncKind::Plugins, SyncKind::UserCache];
    }
    kinds
        .iter()
        .filter_map(|k| match k.trim() {
            "conversations" => Some(SyncKind::Conversations),
            "plugins" => Some(SyncKind::Plugins),
            "user_cache" => Some(SyncKind::UserCache),
            _ => None,
        })
        .collect()
}

// ── 设备发现 ──────────────────────────────────────────────────────────────

#[tauri::command]
pub(crate) async fn scan_devices(
    state: tauri::State<'_, AppState>,
    _app_handle: tauri::AppHandle,
) -> Result<Vec<Device>, String> {
    let p2p = state.p2p.clone();
    p2p.scan_once().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn get_devices(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Device>, String> {
    let p2p = state.p2p.clone();
    Ok(p2p.list_devices().await)
}

/// 当前在线且已配对的设备列表（供 dispatch_remote_task 工具与前端协作面板使用）
#[tauri::command]
pub(crate) async fn get_online_devices(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Device>, String> {
    use effisuite_core::RemoteTaskDispatcher;
    let p2p = state.p2p.clone();
    Ok(p2p.list_online_devices().await)
}

/// 启动后台持续发现（UDP 广播 + 心跳监听）
#[tauri::command]
pub(crate) async fn start_discovery(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let p2p = state.p2p.clone();
    p2p.start_discovery().await.map_err(|e| e.to_string())
}

/// 停止后台持续发现
#[tauri::command]
pub(crate) async fn stop_discovery(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let p2p = state.p2p.clone();
    p2p.stop_discovery().await.map_err(|e| e.to_string())
}

// ── 配对 ──────────────────────────────────────────────────────────────────

/// 通过 IP/链接直连配对（方法一）。
///
/// `address` 形如 `192.168.1.10:47823` 或 `host:port`。
/// 首次配对交换可信密钥，后续连接使用动态会话密钥。
/// `role` 为 "mirror" / "host" / "replica"，默认 "mirror"。
#[tauri::command]
pub(crate) async fn pair_by_address(
    state: tauri::State<'_, AppState>,
    address: String,
    role: Option<String>,
) -> Result<Device, String> {
    let p2p = state.p2p.clone();
    let role = parse_role(role.as_deref().unwrap_or("mirror"));
    p2p.pair_by_address(&address, role)
        .await
        .map_err(|e| e.to_string())
}

/// 对已发现设备发起配对（方法二：广播发现 → 对端请求 → 本机准许）。
///
/// `role` 为 "mirror" / "host" / "replica"，默认 "mirror"。
#[tauri::command]
pub(crate) async fn pair_device(
    state: tauri::State<'_, AppState>,
    id: String,
    role: Option<String>,
) -> Result<(), String> {
    let p2p = state.p2p.clone();
    let role = parse_role(role.as_deref().unwrap_or("mirror"));
    p2p.accept_pair(&id, role)
        .await
        .map_err(|e| e.to_string())
}

/// 拒绝配对请求
#[tauri::command]
pub(crate) async fn reject_pair(
    state: tauri::State<'_, AppState>,
    device_id: String,
) -> Result<(), String> {
    let p2p = state.p2p.clone();
    p2p.reject_pair(&device_id)
        .await
        .map_err(|e| e.to_string())
}

/// 取消已配对设备（从信任库移除，断开传输）
#[tauri::command]
pub(crate) async fn unpair(
    state: tauri::State<'_, AppState>,
    device_id: String,
) -> Result<(), String> {
    let p2p = state.p2p.clone();
    p2p.unpair(&device_id).await.map_err(|e| e.to_string())
}

/// 当前待处理配对请求列表（前端据此展示 pairing-request bubble）
#[tauri::command]
pub(crate) async fn pending_pairing_requests(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<PairingRequest>, String> {
    let p2p = state.p2p.clone();
    Ok(p2p.pending_pairing_requests().await)
}

// ── 镜像同步 ──────────────────────────────────────────────────────────────

/// 从指定设备拉取数据（镜像模式，按时间增量同步）。
///
/// `since`：Unix 秒时间戳，拉取此时间点之后的数据；传 0 拉取全量。
/// `kinds`：同步种类数组，支持 "conversations" / "plugins" / "user_cache"；空数组 = 全部。
#[tauri::command]
pub(crate) async fn sync_pull(
    state: tauri::State<'_, AppState>,
    device_id: String,
    since: u64,
    kinds: Vec<String>,
) -> Result<(), String> {
    let p2p = state.p2p.clone();
    let kinds = parse_kinds(&kinds);
    p2p.pull(&device_id, since, &kinds)
        .await
        .map_err(|e| e.to_string())
}

/// 向指定设备推送本地新增数据（镜像模式）
///
/// `kinds`：同步种类数组，支持 "conversations" / "plugins" / "user_cache"；空数组 = 全部。
#[tauri::command]
pub(crate) async fn sync_push(
    state: tauri::State<'_, AppState>,
    device_id: String,
    kinds: Vec<String>,
) -> Result<(), String> {
    let p2p = state.p2p.clone();
    let kinds = parse_kinds(&kinds);
    p2p.push(&device_id, &kinds)
        .await
        .map_err(|e| e.to_string())
}

/// 查询与指定设备的同步进度（返回 last_sync_ts Unix 秒时间戳）
#[tauri::command]
pub(crate) async fn sync_cursor(
    state: tauri::State<'_, AppState>,
    device_id: String,
) -> Result<u64, String> {
    let p2p = state.p2p.clone();
    Ok(p2p.sync_cursor(&device_id).await)
}

// ── 状态与控制 ────────────────────────────────────────────────────────────

/// 查询 P2P 服务状态（started / self_device_id）
#[tauri::command]
pub(crate) async fn get_p2p_status(
    state: tauri::State<'_, AppState>,
) -> Result<P2pStatus, String> {
    let p2p = state.p2p.clone();
    Ok(P2pStatus {
        started: p2p.is_started(),
        self_device_id: p2p.self_device_id().await,
    })
}

/// 停止 P2P 服务（关闭所有连接与监听）
#[tauri::command]
pub(crate) async fn stop_p2p(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let p2p = state.p2p.clone();
    p2p.stop().await;
    Ok(())
}

//! P2P 服务抽象 trait
//!
//! 业务层（tauriFront / agent）依赖 trait 而非具体实现，便于替换底层
//! （UDP 广播 ↔ mDNS ↔ Mock；TCP 传输 ↔ WebSocket 等未来扩展）时调用方零改动。
//!
//! `RemoteTaskDispatcher` 定义在 `effisuite-core`（供 agent crate 不依赖 p2p 即可使用）。

use async_trait::async_trait;
use effisuite_core::{Device, Result};

use crate::protocol::SyncKind;
use crate::trust::PairRole;

/// 设备发现服务抽象
#[async_trait]
pub trait DiscoveryService: Send + Sync {
    /// 启动后台持续发现（UDP 广播 + 心跳监听）
    async fn start_discovery(&self) -> Result<()>;

    /// 停止后台持续发现
    async fn stop_discovery(&self) -> Result<()>;

    /// 触发一次主动扫描并返回结果（不影响后台持续发现）
    async fn scan_once(&self) -> Result<Vec<Device>>;

    /// 当前已知设备列表（已发现 + 已配对 + 在线状态）
    async fn list_devices(&self) -> Vec<Device>;
}

/// 设备配对服务抽象
#[async_trait]
pub trait PairingService: Send + Sync {
    /// 主动发起配对（方法一：IP/链接直连配对）
    ///
    /// `address` 形如 `192.168.1.10:47823` 或 `host:port`。
    /// 首次配对交换可信密钥，后续连接使用动态会话密钥。
    async fn pair_by_address(&self, address: &str, role: PairRole) -> Result<Device>;

    /// 接受远端发来的配对请求（方法二：广播发现 → 对端请求 → 本机准许）
    async fn accept_pair(&self, device_id: &str, role: PairRole) -> Result<()>;

    /// 拒绝远端发来的配对请求
    async fn reject_pair(&self, device_id: &str) -> Result<()>;

    /// 取消已配对设备（从信任库移除，断开传输）
    async fn unpair(&self, device_id: &str) -> Result<()>;
}

/// 镜像同步服务抽象（镜像模式：双向按时间顺序同步）
#[async_trait]
pub trait SyncService: Send + Sync {
    /// 拉取指定设备指定时间点之后的指定种类数据
    async fn pull(
        &self,
        device_id: &str,
        since: u64,
        kinds: &[SyncKind],
    ) -> Result<()>;

    /// 主动推送本地新增数据到指定设备
    async fn push(&self, device_id: &str, kinds: &[SyncKind]) -> Result<()>;

    /// 查询与指定设备的同步进度（返回 last_sync_ts）
    async fn sync_cursor(&self, device_id: &str) -> u64;
}

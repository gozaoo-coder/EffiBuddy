//! P2P 服务抽象 trait
//!
//! 业务层（如 tauriFront）依赖这两个 trait 而非具体实现，
//! 便于未来用真实 mDNS / UDP 广播实现替换 Mock 时，调用方代码零改动
//! （依赖倒置 + 零成本抽象）。

use async_trait::async_trait;
use effisuite_core::{Device, Result};

/// 设备发现服务抽象
#[async_trait]
pub trait DiscoveryService: Send + Sync {
    /// 启动后台持续发现
    async fn start_discovery(&self) -> Result<()>;

    /// 停止后台持续发现
    async fn stop_discovery(&self) -> Result<()>;

    /// 当前已知设备列表（已发现 + 已配对）
    async fn list_devices(&self) -> Vec<Device>;
}

/// 设备配对服务抽象
#[async_trait]
pub trait PairingService: Send + Sync {
    /// 主动发起配对请求
    async fn pair_device(&self, device_id: &str) -> Result<()>;

    /// 接受远端发来的配对请求
    async fn accept_pair(&self, device_id: &str) -> Result<()>;

    /// 拒绝远端发来的配对请求
    async fn reject_pair(&self, device_id: &str) -> Result<()>;
}

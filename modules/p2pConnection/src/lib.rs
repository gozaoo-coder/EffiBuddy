//! EffiSuite p2pConnection 模块
//!
//! 局域网可信设备发现、配对、加密传输与镜像/主机模式同步。
//!
//! # 模块组织
//! - [`crypto`]：Ed25519 身份密钥 + X25519 ECDH + AES-256-GCM 会话密钥
//! - [`protocol`]：线路消息（握手 / 心跳 / 同步 / 远端任务 / 主机模式 RPC）
//! - [`trust`]：信任库（持久化已配对设备公钥与角色）
//! - [`transport`]：加密 TCP 传输层（监听 / 握手 / 心跳 / 帧收发）
//! - [`discovery`]：UDP 广播设备发现（持续扫描 + 主动扫描）
//! - [`pairing`]：配对协议（IP 直连 / 广播请求两种方法）
//! - [`sync`]：镜像同步（按时间顺序同步会话/插件/用户缓存）
//! - [`manager`]：[`P2pManager`] 协调器，对业务层暴露统一 trait
//! - [`traits`]：业务层面向 trait 编程的抽象（[`DiscoveryService`] /
//!   [`PairingService`] / [`SyncService`]）
//!
//! 业务层（tauriFront / agent）依赖 trait 而非具体实现，便于未来用真实 mDNS /
//! WebSocket 替换底层时调用方零改动（依赖倒置 + 零成本抽象）。

pub mod crypto;
pub mod discovery;
pub mod manager;
pub mod pairing;
pub mod protocol;
pub mod sync;
pub mod transport;
pub mod traits;
pub mod trust;

pub use manager::P2pManager;
pub use pairing::PairingRequest;
pub use protocol::{SyncKind, SyncManifest, ConvManifestEntry, WireMessage};
pub use sync::SyncDataStore;
pub use traits::{DiscoveryService, PairingService, SyncService};
pub use transport::DEFAULT_P2P_PORT;
pub use trust::{PairRole, TrustedPeer, TrustStore};

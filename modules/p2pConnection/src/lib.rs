//! EffiSuite p2pConnection 模块
//!
//! 提供局域网可信设备发现、配对与数据共享能力。
//! 通过 [`DiscoveryService`] / [`PairingService`] 两个 trait 抽象具体后端，
//! 业务层依赖 trait 而非具体实现，便于未来用真实 mDNS / UDP 广播实现
//! 替换 Mock 实现时调用方零改动（依赖倒置 + 零成本抽象）。
//!
//! 当前最小版本提供 [`P2pManager`]（Mock 实现），不依赖真实网络：
//! - `scan_once`：生成 3 台假设备并发布 `DeviceFound` 事件
//! - `pair_device` / `accept_pair` / `reject_pair`：模拟配对握手
//! - 内部状态用 `tokio::sync::RwLock` + 原子类型，临界区极短

pub mod manager;
pub mod traits;

pub use manager::P2pManager;
pub use traits::{DiscoveryService, PairingService};

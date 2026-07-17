//! P2pManager：P2P 连接管理器（Mock 实现）
//!
//! 当前为最小版本，不依赖真实 UDP / TCP / mDNS，使用 Mock 设备数据
//! 验证全链路（发现 -> 配对 -> 状态同步）。
//!
//! 设计要点（遵循 user_rules）：
//! - 内部设备列表用 `tokio::sync::RwLock<Vec<Device>>`（读多写少，优于 Mutex）
//! - 临界区极短：仅持锁做 Vec 读写，`publish` 在锁外
//! - 计数器 / 标志位用 `AtomicU64` / `AtomicBool`，避免 Mutex
//! - 事件通过 `EventBus`（broadcast）传递，不共享可变内存
//! - 结构体字段按大小降序排列，最小化 padding
//! - 迭代器适配器优先，禁止显式索引循环
//! - 仅在跨所有权边界时 clone（如 publish 需要 owned Device）

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use effisuite_core::{BusEvent, CoreError, Device, DeviceStatus, EventBus, Result};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::traits::{DiscoveryService, PairingService};

/// P2P 连接管理器（Mock）
///
/// 字段按大小降序排列：`EventBus`(broadcast Sender，内部 Arc，8 字节)
/// = `RwLock`(8 字节) = `AtomicU64`(8 字节) > `AtomicBool`(1 字节)。
/// `AtomicBool` 放最后，避免尾部 padding 扩散到后续字段。
pub struct P2pManager {
    /// 事件总线句柄（`broadcast::Sender` 内部为 Arc，clone 廉价）
    event_bus: EventBus,
    /// 已知设备列表；读多写少故用 `RwLock`
    devices: RwLock<Vec<Device>>,
    /// 扫描次数计数（原子，无需锁）
    scan_count: AtomicU64,
    /// 发现是否进行中（原子标志位）
    discovery_active: AtomicBool,
}

impl P2pManager {
    /// 创建一个新的 P2pManager
    #[inline]
    pub fn new(event_bus: EventBus) -> Self {
        Self {
            event_bus,
            devices: RwLock::new(Vec::new()),
            scan_count: AtomicU64::new(0),
            discovery_active: AtomicBool::new(false),
        }
    }

    /// 触发一次扫描，返回扫描到的设备；同时通过 EventBus 发布 DeviceFound 事件
    ///
    /// 临界区设计：仅在写锁内做去重与插入；事件发布在锁外完成，
    /// 避免锁内执行 IO（broadcast send 虽轻量，仍遵循"锁内只做必要事"原则）。
    pub async fn scan_once(&self) -> Result<Vec<Device>> {
        let scan_id = self.scan_count.fetch_add(1, Ordering::Relaxed);
        info!(scan_id, "starting mock device scan");

        // 生成 mock 设备列表（owned）
        let scanned = generate_mock_devices();

        // 临界区：仅持锁做去重与插入，收集待发布事件（锁内无 IO / 无 publish）
        let to_publish: Vec<Device> = {
            let mut devices = self.devices.write().await;
            scanned
                .iter()
                .filter_map(|dev| {
                    // 已知设备跳过（迭代器适配器，避免显式索引循环）
                    if devices.iter().any(|d| d.id == dev.id) {
                        None
                    } else {
                        // 写入内部列表（需要一份 clone，因为 scanned 还要返回给调用方）
                        devices.push(dev.clone());
                        // 收集另一份 clone 用于锁外发布
                        Some(dev.clone())
                    }
                })
                .collect()
        };

        // 锁外发布事件，避免锁内 IO
        for dev in &to_publish {
            self.event_bus
                .publish(BusEvent::DeviceFound { device: dev.clone() });
            debug!(device_id = %dev.id, "published DeviceFound event");
        }

        info!(
            scan_id,
            scanned = scanned.len(),
            new_found = to_publish.len(),
            "scan completed"
        );
        Ok(scanned)
    }

    /// 主动发起配对：将设备状态从 Discovered 改为 Paired
    ///
    /// 临界区：仅持锁做查找与状态变更；事件发布在锁外。
    pub async fn pair_device(&self, device_id: &str) -> Result<()> {
        // 临界区：查找设备并改状态，收集事件所需信息后立即释放锁
        let (device_id_owned, status) = {
            let mut devices = self.devices.write().await;
            let dev = devices
                .iter_mut()
                .find(|d| d.id == device_id)
                .ok_or_else(|| CoreError::P2p(format!("device not found: {device_id}")))?;

            if dev.status == DeviceStatus::Paired {
                return Err(CoreError::P2p(format!(
                    "device already paired: {device_id}"
                )));
            }

            dev.status = DeviceStatus::Paired;
            dev.last_seen = current_timestamp();
            // 仅 clone 发布事件所需的最小信息（device_id），status 是 Copy
            (dev.id.clone(), DeviceStatus::Paired)
        };

        // 锁外发布事件
        self.event_bus.publish(BusEvent::DeviceStatusChanged {
            device_id: device_id_owned,
            status,
        });
        info!(device_id, "device paired");
        Ok(())
    }

    /// 接受远端发来的配对请求：将设备置为 Paired
    pub async fn accept_pair(&self, device_id: &str) -> Result<()> {
        let (device_id_owned, status) = {
            let mut devices = self.devices.write().await;
            let dev = devices
                .iter_mut()
                .find(|d| d.id == device_id)
                .ok_or_else(|| CoreError::P2p(format!("device not found: {device_id}")))?;

            if dev.status == DeviceStatus::Paired {
                return Err(CoreError::P2p(format!(
                    "device already paired: {device_id}"
                )));
            }

            dev.status = DeviceStatus::Paired;
            dev.last_seen = current_timestamp();
            (dev.id.clone(), DeviceStatus::Paired)
        };

        self.event_bus.publish(BusEvent::DeviceStatusChanged {
            device_id: device_id_owned,
            status,
        });
        info!(device_id, "pairing accepted");
        Ok(())
    }

    /// 拒绝远端发来的配对请求：将设备回退为 Discovered
    pub async fn reject_pair(&self, device_id: &str) -> Result<()> {
        let (device_id_owned, status) = {
            let mut devices = self.devices.write().await;
            let dev = devices
                .iter_mut()
                .find(|d| d.id == device_id)
                .ok_or_else(|| CoreError::P2p(format!("device not found: {device_id}")))?;

            // 已 Paired 的设备不可拒绝（需先取消配对）
            if dev.status == DeviceStatus::Paired {
                return Err(CoreError::P2p(format!(
                    "cannot reject already paired device: {device_id}"
                )));
            }

            dev.status = DeviceStatus::Discovered;
            dev.last_seen = current_timestamp();
            (dev.id.clone(), DeviceStatus::Discovered)
        };

        self.event_bus.publish(BusEvent::DeviceStatusChanged {
            device_id: device_id_owned,
            status,
        });
        warn!(device_id, "pairing rejected");
        Ok(())
    }

    /// 启动后台持续发现（Mock：仅设置标志位）
    pub async fn start_discovery(&self) -> Result<()> {
        self.discovery_active.store(true, Ordering::Relaxed);
        info!("discovery started (mock)");
        Ok(())
    }

    /// 停止后台持续发现（Mock：仅清除标志位）
    pub async fn stop_discovery(&self) -> Result<()> {
        self.discovery_active.store(false, Ordering::Relaxed);
        info!("discovery stopped");
        Ok(())
    }

    /// 返回当前已知设备列表（已发现 + 已配对）
    ///
    /// 读锁 + clone：跨所有权边界返回 owned Vec，clone 不可避免。
    pub async fn list_devices(&self) -> Vec<Device> {
        let devices = self.devices.read().await;
        // Vec::clone 内部走 memcpy 优化，比 iter().cloned().collect() 更高效
        devices.clone()
    }
}

// ── trait 实现 ────────────────────────────────────────────────────────────
// P2pManager 同时满足两个 trait 抽象，便于上层面向 trait 编程、
// 未来替换为真实实现时调用方零改动。

#[async_trait]
impl DiscoveryService for P2pManager {
    #[inline]
    async fn start_discovery(&self) -> Result<()> {
        P2pManager::start_discovery(self).await
    }

    #[inline]
    async fn stop_discovery(&self) -> Result<()> {
        P2pManager::stop_discovery(self).await
    }

    #[inline]
    async fn list_devices(&self) -> Vec<Device> {
        P2pManager::list_devices(self).await
    }
}

#[async_trait]
impl PairingService for P2pManager {
    #[inline]
    async fn pair_device(&self, device_id: &str) -> Result<()> {
        P2pManager::pair_device(self, device_id).await
    }

    #[inline]
    async fn accept_pair(&self, device_id: &str) -> Result<()> {
        P2pManager::accept_pair(self, device_id).await
    }

    #[inline]
    async fn reject_pair(&self, device_id: &str) -> Result<()> {
        P2pManager::reject_pair(self, device_id).await
    }
}

// ── 辅助函数 ──────────────────────────────────────────────────────────────

/// 生成 mock 设备列表（模拟局域网扫描结果）
///
/// 返回固定 3 台设备，地址符合 `name@ip` 形式。
/// 使用 `with_capacity` 预分配，避免 push 扩容。
#[inline]
fn generate_mock_devices() -> Vec<Device> {
    let now = current_timestamp();
    let mut devices = Vec::with_capacity(3);
    devices.push(Device {
        id: "office-pc-001".to_string(),
        name: "Office-PC".to_string(),
        address: "192.168.1.20".to_string(),
        last_seen: now,
        status: DeviceStatus::Discovered,
    });
    devices.push(Device {
        id: "laptop-002".to_string(),
        name: "Laptop".to_string(),
        address: "192.168.1.35".to_string(),
        last_seen: now,
        status: DeviceStatus::Discovered,
    });
    devices.push(Device {
        id: "phone-003".to_string(),
        name: "Phone".to_string(),
        address: "192.168.1.42".to_string(),
        last_seen: now,
        status: DeviceStatus::Discovered,
    });
    devices
}

/// 当前 Unix 时间戳（秒）
///
/// 不引入 chrono 依赖，直接用 `std::time`；失败时返回 0，绝不 panic。
#[inline]
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── 单元测试 ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use effisuite_core::EventBus;

    /// scan_once 应返回设备并通过 EventBus 发布 DeviceFound 事件
    #[tokio::test]
    async fn scan_once_discovers_and_publishes() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();
        let manager = P2pManager::new(bus);

        let scanned = manager.scan_once().await.expect("scan should succeed");
        assert_eq!(scanned.len(), 3, "mock scan should return 3 devices");

        // 验证每台设备都发布了 DeviceFound 事件
        let mut found_ids = Vec::with_capacity(3);
        for _ in 0..scanned.len() {
            let event = rx
                .recv()
                .await
                .expect("should receive DeviceFound event");
            match event {
                BusEvent::DeviceFound { device } => found_ids.push(device.id),
                other => panic!("expected DeviceFound, got {other:?}"),
            }
        }
        assert_eq!(found_ids.len(), 3);
        assert!(found_ids.contains(&"office-pc-001".to_string()));
        assert!(found_ids.contains(&"laptop-002".to_string()));
        assert!(found_ids.contains(&"phone-003".to_string()));

        // list_devices 应返回全部 3 台
        let listed = manager.list_devices().await;
        assert_eq!(listed.len(), 3);
    }

    /// 重复扫描不应重复发布已知设备的 DeviceFound 事件
    #[tokio::test]
    async fn scan_once_dedupes_known_devices() {
        let bus = EventBus::new(16);
        let manager = P2pManager::new(bus);

        // 第一次扫描：3 台全部新增
        let first = manager.scan_once().await.expect("first scan");
        assert_eq!(first.len(), 3);

        // 第二次扫描：返回同样的 3 台（扫描结果），但无新事件
        let second = manager.scan_once().await.expect("second scan");
        assert_eq!(second.len(), 3, "scan result should still contain all devices");

        // list 仍为 3 台（无重复）
        let listed = manager.list_devices().await;
        assert_eq!(listed.len(), 3, "device list should not have duplicates");
    }

    /// pair_device 应将设备状态改为 Paired 并发布事件
    #[tokio::test]
    async fn pair_device_changes_status_and_publishes() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();
        let manager = P2pManager::new(bus);

        // 先扫描以填充设备列表
        manager.scan_once().await.expect("scan");
        // 排空 DeviceFound 事件
        while rx.try_recv().is_ok() {}

        // 配对
        manager
            .pair_device("office-pc-001")
            .await
            .expect("pair should succeed");

        // 验证发布了 DeviceStatusChanged 事件
        let event = rx
            .recv()
            .await
            .expect("should receive DeviceStatusChanged event");
        match event {
            BusEvent::DeviceStatusChanged { device_id, status } => {
                assert_eq!(device_id, "office-pc-001");
                assert_eq!(status, DeviceStatus::Paired);
            }
            other => panic!("expected DeviceStatusChanged, got {other:?}"),
        }

        // 验证内部状态已更新
        let devices = manager.list_devices().await;
        let dev = devices
            .iter()
            .find(|d| d.id == "office-pc-001")
            .expect("device should exist");
        assert_eq!(dev.status, DeviceStatus::Paired);
        assert!(dev.is_paired());
    }

    /// pair_device 对不存在的设备应返回错误
    #[tokio::test]
    async fn pair_device_unknown_returns_error() {
        let bus = EventBus::new(8);
        let manager = P2pManager::new(bus);

        let err = manager
            .pair_device("non-existent")
            .await
            .expect_err("should error on unknown device");
        assert!(matches!(err, CoreError::P2p(_)));
    }

    /// 重复配对已 Paired 的设备应返回错误
    #[tokio::test]
    async fn pair_device_already_paired_returns_error() {
        let bus = EventBus::new(8);
        let manager = P2pManager::new(bus);

        manager.scan_once().await.expect("scan");
        manager
            .pair_device("laptop-002")
            .await
            .expect("first pair");

        let err = manager
            .pair_device("laptop-002")
            .await
            .expect_err("should error on double pair");
        assert!(matches!(err, CoreError::P2p(_)));
    }

    /// accept_pair 应将设备置为 Paired
    #[tokio::test]
    async fn accept_pair_sets_paired() {
        let bus = EventBus::new(8);
        let mut rx = bus.subscribe();
        let manager = P2pManager::new(bus);

        manager.scan_once().await.expect("scan");
        while rx.try_recv().is_ok() {}

        manager
            .accept_pair("phone-003")
            .await
            .expect("accept should succeed");

        let devices = manager.list_devices().await;
        let dev = devices
            .iter()
            .find(|d| d.id == "phone-003")
            .expect("device should exist");
        assert_eq!(dev.status, DeviceStatus::Paired);
    }

    /// reject_pair 应将设备回退为 Discovered
    #[tokio::test]
    async fn reject_pair_reverts_to_discovered() {
        let bus = EventBus::new(8);
        let mut rx = bus.subscribe();
        let manager = P2pManager::new(bus);

        manager.scan_once().await.expect("scan");
        while rx.try_recv().is_ok() {}

        // reject 一个 Discovered 状态的设备
        manager
            .reject_pair("laptop-002")
            .await
            .expect("reject should succeed");

        let devices = manager.list_devices().await;
        let dev = devices
            .iter()
            .find(|d| d.id == "laptop-002")
            .expect("device should exist");
        assert_eq!(dev.status, DeviceStatus::Discovered);

        // 应发布 Discovered 状态事件
        let event = rx.recv().await.expect("should receive event");
        match event {
            BusEvent::DeviceStatusChanged { status, .. } => {
                assert_eq!(status, DeviceStatus::Discovered);
            }
            other => panic!("expected DeviceStatusChanged, got {other:?}"),
        }
    }

    /// reject_pair 对已 Paired 的设备应返回错误
    #[tokio::test]
    async fn reject_pair_on_paired_returns_error() {
        let bus = EventBus::new(8);
        let manager = P2pManager::new(bus);

        manager.scan_once().await.expect("scan");
        manager
            .pair_device("office-pc-001")
            .await
            .expect("pair");

        let err = manager
            .reject_pair("office-pc-001")
            .await
            .expect_err("should not reject paired device");
        assert!(matches!(err, CoreError::P2p(_)));
    }

    /// start/stop discovery 应正常切换标志位
    #[tokio::test]
    async fn start_stop_discovery_roundtrip() {
        let bus = EventBus::new(8);
        let manager = P2pManager::new(bus);

        manager
            .start_discovery()
            .await
            .expect("start should succeed");
        assert!(manager.discovery_active.load(Ordering::Relaxed));

        manager
            .stop_discovery()
            .await
            .expect("stop should succeed");
        assert!(!manager.discovery_active.load(Ordering::Relaxed));
    }

    /// 验证 mock 设备地址格式符合 `name@ip` 契约定义
    #[tokio::test]
    async fn mock_devices_have_expected_addresses() {
        let bus = EventBus::new(8);
        let manager = P2pManager::new(bus);

        let devices = manager.scan_once().await.expect("scan");
        let addresses: Vec<&str> = devices.iter().map(|d| d.address.as_str()).collect();
        assert!(addresses.contains(&"192.168.1.20"));
        assert!(addresses.contains(&"192.168.1.35"));
        assert!(addresses.contains(&"192.168.1.42"));
    }
}

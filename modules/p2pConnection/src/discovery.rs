//! UDP 广播设备发现：局域网内设备的实时感知与主动扫描。
//!
//! # 职责
//! - 周期性向 `255.255.255.255:{DEFAULT_DISCOVERY_PORT}` 广播本机 `device_id` +
//!   TCP 监听端口，让局域网内其他 EffiSuite 设备发现自己；
//! - 监听同端口广播，解析为 [`Device`]，按信任库过滤后发布 [`BusEvent`]：
//!   - 已配对设备 → [`DeviceFound`](BusEvent::DeviceFound) +
//!     [`DeviceStatusChanged(Paired)`](BusEvent::DeviceStatusChanged)（边沿触发，仅首次上线）
//!   - 未配对设备 → [`PairingRequest`](BusEvent::PairingRequest)（供前端展示可配对气泡）
//! - [`Discovery::scan_once`] 发送一次性广播，等待 1.5s 后返回当前已知设备列表。
//!
//! # 协议
//! [`DiscoveryBroadcast`] 为单条明文 JSON UDP 数据报（≤ ~1400B）。广播仅用于发现，
//! 不传输业务数据，故无需加密。`timestamp` 字段用于防重放（±60s 窗口）。
//!
//! # 并发与内存
//! - 后台两个 task：announce / listen，`JoinHandle` 存于 `Mutex<Option<..>>`，
//!   `stop` 时 `abort`（幂等）；
//! - 共享态 `known` 用 `RwLock<HashMap<String, Device>>`（读多写少），临界区仅
//!   HashMap 读写，事件发布在锁外；
//! - 启停标志用 `AtomicBool`，TCP 监听端口用 `AtomicU16`，不使用 `Mutex<bool>`；
//! - 字段按大小降序排列以最小化 padding；
//! - 已知设备表预分配 `with_capacity(32)`（局域网通常 < 32 台）。

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use effisuite_core::{remote_task_now, BusEvent, CoreError, Device, DeviceStatus, EventBus, Result};

use crate::trust::TrustStore;

/// 默认 UDP 发现端口
pub const DEFAULT_DISCOVERY_PORT: u16 = 47823;
/// 广播周期
const BROADCAST_INTERVAL: Duration = Duration::from_secs(5);
/// `scan_once` 收集响应窗口
const SCAN_WINDOW: Duration = Duration::from_millis(1500);
/// 防重放时间窗口（秒）：忽略与当前时间偏差 > 60s 的广播
const REPLAY_WINDOW: u64 = 60;
/// 已知设备表预估容量（局域网通常 < 32 台）
const KNOWN_CAPACITY: usize = 32;
/// 单数据报最大缓冲（留足 MTU 1500B 余量）
const MAX_DATAGRAM: usize = 2048;
/// 受限广播地址
const BROADCAST_IP: &str = "255.255.255.255";

/// UDP 发现广播消息（明文 JSON，仅用于发现，不传输业务数据）。
///
/// 字段按大小降序：String(24) → u64(8) → u16(2)。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiscoveryBroadcast {
    /// 发送方 device_id
    device_id: String,
    /// 设备展示名（如"电脑"/"手机"）
    name: String,
    /// Unix 秒时间戳，防重放
    timestamp: u64,
    /// 本机 TCP 监听端口（供对端 connect 配对）
    listen_port: u16,
}

/// UDP 广播设备发现器。
///
/// 字段按大小降序：`TrustStore`(~32B) → `String`/`RwLock`(24B) → `EventBus`/`Mutex`(8B)
/// → `AtomicU16`/`u16`(2B) → `AtomicBool`(1B)。
///
/// 通过 `Arc<Discovery>` 共享（[`Discovery::start`] 需 `&Arc<Self>` 以克隆入后台 task）。
pub struct Discovery {
    /// 信任库（判定对端是否已配对、刷新 `last_seen`）
    trust: TrustStore,
    /// 本机 device_id（构造后不可变）
    self_device_id: String,
    /// 本机设备展示名（用户改名时通过 `set_self_name` 更新）
    self_name: RwLock<String>,
    /// 已知设备表：device_id → Device（读多写少）
    known: RwLock<HashMap<String, Device>>,
    /// 事件总线（发布 `BusEvent`）
    event_bus: EventBus,
    /// 广播 task 句柄
    broadcast_handle: Mutex<Option<JoinHandle<()>>>,
    /// 监听 task 句柄
    listen_handle: Mutex<Option<JoinHandle<()>>>,
    /// 本机 TCP 监听端口（广播时携带，供对端 connect）
    listen_port: AtomicU16,
    /// UDP 发现端口（默认 [`DEFAULT_DISCOVERY_PORT`]，测试可置 0 用随机端口）
    discovery_port: u16,
    /// 启停标志
    running: AtomicBool,
}

impl Discovery {
    /// 构造发现器。
    ///
    /// - `trust`：信任库（用于判定对端是否已配对、刷新 `last_seen`）。
    /// - `event_bus`：发布 [`BusEvent`]。
    /// - `self_device_id`：本机 device_id（来自 TrustStore）。
    ///
    /// 默认 `self_name = self_device_id`，可通过 [`Discovery::set_self_name`] 覆盖。
    /// 默认 `listen_port = 0`，需在 transport 启动后通过 [`Discovery::set_listen_port`] 设置。
    pub fn new(trust: TrustStore, event_bus: EventBus, self_device_id: String) -> Self {
        let self_name = self_device_id.clone();
        Self {
            trust,
            self_device_id,
            self_name: RwLock::new(self_name),
            known: RwLock::new(HashMap::with_capacity(KNOWN_CAPACITY)),
            event_bus,
            broadcast_handle: Mutex::new(None),
            listen_handle: Mutex::new(None),
            listen_port: AtomicU16::new(0),
            discovery_port: DEFAULT_DISCOVERY_PORT,
            running: AtomicBool::new(false),
        }
    }

    /// 启动后台广播 + 监听两个 task。幂等：已启动时直接返回 `Ok(())`。
    pub async fn start(self: &Arc<Self>) -> Result<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let result = self.start_inner().await;
        if result.is_err() {
            self.running.store(false, Ordering::SeqCst);
        }
        result
    }

    /// 实际启动逻辑（绑定 socket + spawn task），失败时由 `start` 重置 running。
    async fn start_inner(self: &Arc<Self>) -> Result<()> {
        // 监听 socket：绑定发现端口，启用广播接收。
        let listen_sock = UdpSocket::bind(("0.0.0.0", self.discovery_port)).await?;
        listen_sock.set_broadcast(true)?;
        let actual_port = listen_sock.local_addr()?.port();

        // 广播 socket：绑定临时端口（仅发送），与监听端口隔离避免争用。
        let announce_sock = UdpSocket::bind("0.0.0.0:0").await?;
        announce_sock.set_broadcast(true)?;

        let bcast: SocketAddr = format!("{BROADCAST_IP}:{actual_port}")
            .parse()
            .map_err(|e| CoreError::P2p(format!("invalid broadcast addr: {e}")))?;

        info!(
            device_id = %self.self_device_id,
            port = actual_port,
            "discovery started"
        );

        let self_clone = Arc::clone(self);
        let announce_handle = tokio::spawn(async move {
            announce_loop(self_clone, announce_sock, bcast).await;
        });

        let self_clone = Arc::clone(self);
        let listen_handle = tokio::spawn(async move {
            listen_loop(self_clone, listen_sock).await;
        });

        *self.broadcast_handle.lock().await = Some(announce_handle);
        *self.listen_handle.lock().await = Some(listen_handle);
        Ok(())
    }

    /// 停止后台 task。幂等：未启动时直接返回 `Ok(())`。
    pub async fn stop(&self) -> Result<()> {
        if !self.running.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        if let Some(h) = self.broadcast_handle.lock().await.take() {
            h.abort();
        }
        if let Some(h) = self.listen_handle.lock().await.take() {
            h.abort();
        }
        info!("discovery stopped");
        Ok(())
    }

    /// 触发一次性发现：发送一条广播，等待 [`SCAN_WINDOW`] 后返回当前已知设备列表。
    ///
    /// 独立于后台 `start`/`listen`：使用临时 socket 发送，不绑定发现端口。
    /// 若后台监听正在运行，监听 task 会处理对端广播并更新已知表；本方法等待窗口
    /// 结束后返回该表快照。
    pub async fn scan_once(&self) -> Result<Vec<Device>> {
        let sock = UdpSocket::bind("0.0.0.0:0").await?;
        sock.set_broadcast(true)?;
        let pkt = self.build_broadcast().await;
        let bytes = serde_json::to_vec(&pkt)?;
        let bcast: SocketAddr = format!("{BROADCAST_IP}:{}", self.discovery_port)
            .parse()
            .map_err(|e| CoreError::P2p(format!("invalid broadcast addr: {e}")))?;
        if let Err(e) = sock.send_to(&bytes, bcast).await {
            warn!(error = %e, "scan_once broadcast send failed");
        } else {
            debug!(device_id = %pkt.device_id, "scan_once broadcast sent");
        }
        tokio::time::sleep(SCAN_WINDOW).await;
        Ok(self.list_devices().await)
    }

    /// 当前已知设备列表（已发现 + 已配对 + 在线状态）。
    pub async fn list_devices(&self) -> Vec<Device> {
        self.known.read().await.values().cloned().collect()
    }

    /// 设置本机 TCP 监听端口（在 transport::start 后由 manager 调用，广播时携带供对端 connect）。
    pub async fn set_listen_port(&self, port: u16) {
        self.listen_port.store(port, Ordering::Relaxed);
    }

    /// 设置本机设备展示名（用户在设置中改名时调用）。
    pub async fn set_self_name(&self, name: String) {
        *self.self_name.write().await = name;
    }

    /// 构造一条广播消息（读取当前 self_name / listen_port）。
    async fn build_broadcast(&self) -> DiscoveryBroadcast {
        let name = self.self_name.read().await.clone();
        let listen_port = self.listen_port.load(Ordering::Relaxed);
        DiscoveryBroadcast {
            device_id: self.self_device_id.clone(),
            name,
            timestamp: remote_task_now(),
            listen_port,
        }
    }

    /// 处理一条已解析的广播：过滤自回环 / 重放，更新已知表，发布事件。
    ///
    /// 将"解析广播"与"网络收发"分离，便于单元测试覆盖过滤与事件逻辑。
    ///
    /// - `pkt`：已反序列化的广播消息。
    /// - `src_ip`：UDP 数据报来源 IP（用于构造 Device.address）。
    /// - `now`：当前 Unix 秒（传入以便测试 mock 时间）。
    async fn handle_incoming(&self, pkt: &DiscoveryBroadcast, src_ip: IpAddr, now: u64) {
        // 防自回环：忽略自身广播
        if pkt.device_id == self.self_device_id {
            return;
        }
        // 防重放：忽略与当前时间偏差 > 60s 的广播
        if pkt.timestamp.abs_diff(now) > REPLAY_WINDOW {
            debug!(device_id = %pkt.device_id, "discovery packet stale, ignoring");
            return;
        }

        let address = SocketAddr::new(src_ip, pkt.listen_port).to_string();
        let trusted = self.trust.get_peer(&pkt.device_id).await.is_some();
        let status = if trusted {
            DeviceStatus::Paired
        } else {
            DeviceStatus::Discovered
        };
        let device = Device {
            id: pkt.device_id.clone(),
            name: pkt.name.clone(),
            address,
            last_seen: now,
            status,
        };

        // 原子 check-and-insert：临界区仅 HashMap 读写，无 IO / 事件发布。
        // 已知设备（常见路径）：get_mut 借 key 更新，零 clone。
        // 新设备：clone 一份入表，原 device 留给事件发布。
        let device_for_event: Option<Device> = {
            let mut map = self.known.write().await;
            if let Some(existing) = map.get_mut(&pkt.device_id) {
                *existing = device;
                None
            } else {
                map.insert(pkt.device_id.clone(), device.clone());
                Some(device)
            }
        };

        match device_for_event {
            Some(device) if trusted => {
                // 已配对设备首次上线：刷新 trust last_seen + 发布 DeviceFound
                // + DeviceStatusChanged(Paired)（边沿触发，避免每 5s 洪泛）
                let _ = self.trust.touch_last_seen(&pkt.device_id, now).await;
                self.event_bus
                    .publish(BusEvent::DeviceFound { device });
                self.event_bus.publish(BusEvent::DeviceStatusChanged {
                    device_id: pkt.device_id.clone(),
                    status: DeviceStatus::Paired,
                });
            }
            Some(device) => {
                // 未配对设备：发布 PairingRequest 供前端展示可配对气泡
                self.event_bus
                    .publish(BusEvent::PairingRequest { device });
            }
            None if trusted => {
                // 已知在线的已配对设备：仅刷新 last_seen，不重复发事件
                let _ = self.trust.touch_last_seen(&pkt.device_id, now).await;
            }
            None => {}
        }
    }
}

// ── 后台 task ────────────────────────────────────────────────────────────

/// 周期广播 `DiscoveryBroadcast`。
async fn announce_loop(self_arc: Arc<Discovery>, sock: UdpSocket, bcast: SocketAddr) {
    let mut ticker = tokio::time::interval(BROADCAST_INTERVAL);
    info!("discovery announce loop started");
    loop {
        ticker.tick().await;
        let pkt = self_arc.build_broadcast().await;
        match serde_json::to_vec(&pkt) {
            Ok(bytes) => {
                if let Err(e) = sock.send_to(&bytes, bcast).await {
                    warn!(error = %e, "discovery announce send failed");
                } else {
                    debug!(device_id = %pkt.device_id, "announce sent");
                }
            }
            Err(e) => warn!(error = %e, "discovery announce serialize failed"),
        }
    }
}

/// 监听广播：解析 → 过滤 → 更新已知表 → 发布事件。
async fn listen_loop(self_arc: Arc<Discovery>, sock: UdpSocket) {
    let mut buf = [0u8; MAX_DATAGRAM];
    info!("discovery listen loop started");
    loop {
        match sock.recv_from(&mut buf).await {
            Ok((n, src)) => {
                let pkt = match serde_json::from_slice::<DiscoveryBroadcast>(&buf[..n]) {
                    Ok(p) => p,
                    Err(e) => {
                        debug!(error = %e, "discarded malformed discovery packet");
                        continue;
                    }
                };
                self_arc
                    .handle_incoming(&pkt, src.ip(), remote_task_now())
                    .await;
            }
            Err(e) => {
                warn!(error = %e, "discovery recv_from error");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

// ── 单元测试 ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 1. DiscoveryBroadcast serde roundtrip
    #[test]
    fn discovery_broadcast_roundtrip() {
        let pkt = DiscoveryBroadcast {
            device_id: "dev-abc".to_string(),
            name: "My Laptop".to_string(),
            timestamp: 1_700_000_000u64,
            listen_port: 47823,
        };
        let json = serde_json::to_string(&pkt).unwrap();
        let back: DiscoveryBroadcast = serde_json::from_str(&json).unwrap();
        assert_eq!(back.device_id, "dev-abc");
        assert_eq!(back.name, "My Laptop");
        assert_eq!(back.timestamp, 1_700_000_000);
        assert_eq!(back.listen_port, 47823);
    }

    /// 2. start/stop 切换 running 标志（绑定随机端口避免冲突）
    #[tokio::test]
    async fn discovery_starts_and_stops() {
        let trust = TrustStore::placeholder();
        let bus = EventBus::new(8);
        let mut d = Discovery::new(trust, bus, "dev-test".to_string());
        d.discovery_port = 0; // 随机端口，避免端口冲突
        let d = Arc::new(d);

        d.start().await.unwrap();
        assert!(
            d.running.load(Ordering::SeqCst),
            "running should be true after start"
        );

        d.stop().await.unwrap();
        assert!(
            !d.running.load(Ordering::SeqCst),
            "running should be false after stop"
        );
    }

    /// 3. 自回环过滤：device_id == self 的广播被忽略
    #[tokio::test]
    async fn discovery_ignores_self_broadcast() {
        let trust = TrustStore::placeholder();
        let bus = EventBus::new(8);
        let d = Discovery::new(trust, bus, "dev-self".to_string());
        let now = remote_task_now();
        let pkt = DiscoveryBroadcast {
            device_id: "dev-self".to_string(),
            name: "Self".to_string(),
            timestamp: now,
            listen_port: 47823,
        };
        d.handle_incoming(&pkt, "127.0.0.1".parse().unwrap(), now)
            .await;
        assert!(
            d.list_devices().await.is_empty(),
            "self broadcast should be ignored"
        );
    }

    /// 4. 防重放：时间戳超出 60s 窗口的广播被忽略
    #[tokio::test]
    async fn discovery_filters_replay() {
        let trust = TrustStore::placeholder();
        let bus = EventBus::new(8);
        let d = Discovery::new(trust, bus, "dev-self".to_string());
        let now = 1_000_000u64;
        let pkt = DiscoveryBroadcast {
            device_id: "dev-other".to_string(),
            name: "Other".to_string(),
            // 超出窗口 1 秒
            timestamp: now - REPLAY_WINDOW - 1,
            listen_port: 47823,
        };
        d.handle_incoming(&pkt, "127.0.0.1".parse().unwrap(), now)
            .await;
        assert!(
            d.list_devices().await.is_empty(),
            "stale broadcast should be filtered"
        );
    }

    /// 补充：有效的未配对设备广播应入表并触发 PairingRequest
    #[tokio::test]
    async fn discovery_detects_unpaired_device() {
        let trust = TrustStore::placeholder();
        let bus = EventBus::new(8);
        let mut rx = bus.subscribe();
        let d = Discovery::new(trust, bus, "dev-self".to_string());
        let now = remote_task_now();
        let pkt = DiscoveryBroadcast {
            device_id: "dev-stranger".to_string(),
            name: "Stranger".to_string(),
            timestamp: now,
            listen_port: 47823,
        };
        d.handle_incoming(&pkt, "192.168.1.50".parse().unwrap(), now)
            .await;

        let devices = d.list_devices().await;
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "dev-stranger");
        assert_eq!(devices[0].status, DeviceStatus::Discovered);
        assert_eq!(devices[0].address, "192.168.1.50:47823");

        // 应发布 PairingRequest
        let evt = tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .expect("event timeout")
            .expect("event channel closed");
        assert!(matches!(evt, BusEvent::PairingRequest { .. }));
    }

    /// 补充：同一设备重复广播不重复发事件（边沿触发）
    #[tokio::test]
    async fn discovery_edge_triggered_no_flood() {
        let trust = TrustStore::placeholder();
        let bus = EventBus::new(8);
        let d = Discovery::new(trust, bus, "dev-self".to_string());
        let now = remote_task_now();
        let pkt = DiscoveryBroadcast {
            device_id: "dev-repeat".to_string(),
            name: "Repeat".to_string(),
            timestamp: now,
            listen_port: 47823,
        };
        // 连续处理 3 次同一设备广播
        for _ in 0..3 {
            d.handle_incoming(&pkt, "127.0.0.1".parse().unwrap(), now)
                .await;
        }
        let devices = d.list_devices().await;
        assert_eq!(devices.len(), 1, "should not duplicate entry");
    }
}

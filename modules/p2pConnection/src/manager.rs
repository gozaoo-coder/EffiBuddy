//! P2pManager：P2P 连接管理器（协调器）
//!
//! 整合 transport / trust / discovery / pairing / sync 子模块，对业务层暴露
//! 统一的 [`DiscoveryService`] / [`PairingService`] / [`SyncService`] trait
//! 与 [`effisuite_core::RemoteTaskDispatcher`] 实现。
//!
//! 设计要点（遵循 user_rules）：
//! - 内部状态用 `tokio::sync::RwLock`（读多写少）+ 原子类型，临界区极短
//! - 事件通过 `EventBus`（broadcast）传递，不共享可变内存
//! - 结构体字段按大小降序排列，最小化 padding

use std::sync::Arc;

use async_trait::async_trait;
use effisuite_core::{BusEvent, CoreError, Device, DeviceStatus, EventBus, Result};
use tokio::sync::RwLock;
use tracing::info;

use crate::discovery::Discovery;
use crate::pairing::{Pairing, PairingRequest};
use crate::protocol::SyncKind;
use crate::sync::{Sync, SyncDataStore};
use crate::traits::{DiscoveryService, PairingService, SyncService};
use crate::transport::Transport;
use crate::trust::{PairRole, TrustStore};

/// P2P 连接管理器（协调器）
///
/// 字段按大小降序：`Arc<...>`（1 usize）集群在前，`RwLock<...>`（1 usize）居中。
pub struct P2pManager {
    /// 信任库（持久化已配对设备公钥与角色）
    trust: TrustStore,
    /// 事件总线
    event_bus: EventBus,
    /// 加密 TCP 传输层
    transport: RwLock<Option<Arc<Transport>>>,
    /// UDP 广播设备发现
    discovery: RwLock<Option<Arc<Discovery>>>,
    /// 配对协议
    pairing: RwLock<Option<Arc<Pairing>>>,
    /// 镜像同步
    sync: RwLock<Option<Arc<Sync>>>,
    /// 待处理配对请求列表（广播发现 → 对端请求 → 本机准许）
    pending_pairing_requests: RwLock<Vec<PairingRequest>>,
    /// 本机 device_id
    self_device_id: RwLock<String>,
    /// 启动标志
    started: std::sync::atomic::AtomicBool,
}

impl P2pManager {
    /// 创建一个新的 P2pManager（不启动，需调用 `start_with_trust` 启动广播与监听）。
    /// 身份与信任库在 `start_with_trust` 时注入，此前各模块句柄均为空。
    pub fn new(event_bus: EventBus) -> Self {
        Self {
            trust: TrustStore::placeholder(),
            event_bus,
            transport: RwLock::new(None),
            discovery: RwLock::new(None),
            pairing: RwLock::new(None),
            sync: RwLock::new(None),
            pending_pairing_requests: RwLock::new(Vec::new()),
            self_device_id: RwLock::new("dev-anon".to_string()),
            started: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// 用已加载的 trust store 启动 P2P 服务：
    /// 1. 启动 TCP 传输层监听（动态会话密钥握手）
    /// 2. 启动 UDP 广播发现（持续扫描可信设备是否在线）
    /// 3. 启动配对协议（处理入站配对请求）
    /// 4. 启动镜像同步（按时间顺序同步会话/插件/用户缓存）
    pub async fn start_with_trust(
        self: &Arc<Self>,
        trust: TrustStore,
        identity: crate::crypto::IdentityKey,
        bind_addr: std::net::SocketAddr,
    ) -> Result<()> {
        if self
            .started
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(CoreError::P2p("P2pManager already started".to_string()));
        }

        let self_device_id = trust.self_device_id().await;
        *self.self_device_id.write().await = self_device_id.clone();
        // 构造 transport（用正式 trust + identity）
        let transport = Arc::new(Transport::new(
            trust.clone(),
            identity.clone(),
            self.event_bus.clone(),
            self_device_id.clone(),
        ));
        let incoming_rx = transport.start(bind_addr).await?;
        // 读取实际监听端口（bind 用 0 端口时由 OS 分配），让 discovery 广播携带可连接端口
        let actual_bind = transport
            .bind_addr()
            .await
            .ok_or_else(|| CoreError::P2p("transport bind_addr missing after start".to_string()))?;
        // 构造 discovery（UDP 广播）
        let discovery = Arc::new(Discovery::new(
            trust.clone(),
            self.event_bus.clone(),
            self_device_id.clone(),
        ));
        discovery.set_listen_port(actual_bind.port()).await;
        discovery.start().await?;
        // 构造 pairing（处理入站 pairing request）
        let pairing = Arc::new(Pairing::new(
            trust.clone(),
            identity.clone(),
            Arc::clone(&transport),
            self.event_bus.clone(),
            self_device_id.clone(),
        ));
        // 构造 sync
        let sync = Arc::new(Sync::new(Arc::clone(&transport)));

        // 启动入站消息路由 task：把 transport 收到的消息分发到 pairing / sync / 事件总线
        let pairing_handle = Arc::clone(&pairing);
        let sync_handle = Arc::clone(&sync);
        let transport_for_task = Arc::clone(&transport);
        tokio::spawn(async move {
            let mut rx = incoming_rx;
            while let Some(msg) = rx.recv().await {
                use crate::protocol::WireMessage;
                match msg.message {
                    WireMessage::Ping { ts } => {
                        // 收到 Ping → 回 Pong（如通道还在）
                        if let Some(ch) = transport_for_task.get_channel(&msg.device_id).await {
                            let _ = ch.send(WireMessage::Pong { ts }).await;
                        }
                    }
                    WireMessage::Pong { .. } => {
                        // Pong 已在 transport reader 静默消费，不应到达
                    }
                    WireMessage::TaskRequest { request_id, task } => {
                        // 远端任务请求：接收端需本机 agent 处理后回 TaskResponse。
                        // agent 集成尚未接入，明确回错误，避免发送方挂起 120s 超时。
                        tracing::warn!(
                            device = %msg.device_id,
                            request_id = %request_id,
                            task = %task.chars().take(64).collect::<String>(),
                            "remote task request received but receiver not wired to agent"
                        );
                        if let Some(ch) = transport_for_task.get_channel(&msg.device_id).await {
                            let _ = ch
                                .send(WireMessage::TaskResponse {
                                    request_id,
                                    result: "远端任务执行器尚未接入本机 agent".to_string(),
                                    is_error: true,
                                })
                                .await;
                        }
                    }
                    WireMessage::SyncRequest { .. }
                    | WireMessage::SyncFetch { .. }
                    | WireMessage::SyncManifest { .. }
                    | WireMessage::SyncMessages { .. }
                    | WireMessage::SyncData { .. } => {
                        // 同步相关消息统一由 sync 路由（请求→响应，响应→等待者/落盘）
                        let _ = sync_handle.handle_incoming(&msg.device_id, msg.message).await;
                    }
                    WireMessage::TaskResponse { .. }
                    | WireMessage::HostListConversations
                    | WireMessage::HostGetConversation { .. }
                    | WireMessage::HostSendMessage { .. }
                    | WireMessage::HostReply { .. } => {
                        let _ = pairing_handle
                            .handle_incoming(&msg.device_id, msg.message)
                            .await;
                    }
                    WireMessage::Ack { .. } => {
                        // 通用 Ack 由 pairing 内部 pending map 匹配
                        let _ = pairing_handle
                            .handle_incoming(&msg.device_id, msg.message)
                            .await;
                    }
                    // Hello / HelloAck 在 transport 握手阶段已消费，不应到达 manager
                    WireMessage::Hello { .. } | WireMessage::HelloAck { .. } => {
                        tracing::warn!(
                            device = %msg.device_id,
                            "unexpected Hello/HelloAck in manager routing"
                        );
                    }
                }
            }
        });

        // 订阅 EventBus PairingRequest 事件，把未配对设备的广播请求收集到
        // pending_pairing_requests（前端据此展示可配对气泡 + 接受/拒绝按钮）。
        // 与 pairing 内部的 pending_peers 互补：后者仅存地址供 accept_pair 用，
        // 前者存完整 PairingRequest 供前端展示设备名/公钥/时间戳。
        let self_arc = Arc::clone(self);
        let mut req_rx = self.event_bus.subscribe();
        tokio::spawn(async move {
            loop {
                match req_rx.recv().await {
                    Ok(BusEvent::PairingRequest { device }) => {
                        let req = PairingRequest {
                            device_id: device.id.clone(),
                            name: device.name.clone(),
                            address: device.address.clone(),
                            pubkey_hex: String::new(), // 广播阶段未携带公钥，配对握手时交换
                            timestamp: device.last_seen,
                        };
                        self_arc.push_pairing_request(req).await;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(lagged = n, "manager pairing-request subscriber lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        // 保存各模块句柄
        *self.transport.write().await = Some(transport);
        *self.discovery.write().await = Some(discovery);
        *self.pairing.write().await = Some(pairing);
        *self.sync.write().await = Some(sync);
        info!(device_id = %self_device_id, "P2pManager started");
        Ok(())
    }

    /// 停止 P2P 服务
    pub async fn stop(&self) {
        if let Some(d) = self.discovery.write().await.take() {
            let _ = d.stop().await;
        }
        if let Some(s) = self.sync.write().await.take() {
            let _ = s.stop().await;
        }
        if let Some(p) = self.pairing.write().await.take() {
            let _ = p.stop().await;
        }
        if let Some(t) = self.transport.write().await.take() {
            t.stop().await;
        }
        self.started
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// P2P 服务是否已启动（`start_with_trust` 成功后为 true）
    #[inline]
    pub fn is_started(&self) -> bool {
        self.started.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// 本机设备 id（启动前为占位 `dev-anon`，启动后为信任库中的正式 id）
    pub async fn self_device_id(&self) -> String {
        self.self_device_id.read().await.clone()
    }

    /// 注入镜像同步数据源（业务层在 `start_with_trust` 后调用）。
    /// 同步器据此读写会话 / 插件 / 永久记忆与同步游标。
    pub async fn set_sync_data_store(&self, store: Arc<dyn SyncDataStore>) {
        if let Some(s) = self.sync.read().await.as_ref() {
            s.set_data_store(store).await;
        }
    }

    /// 当前待处理配对请求列表（前端据此展示 pairing-request bubble）
    pub async fn pending_pairing_requests(&self) -> Vec<PairingRequest> {
        self.pending_pairing_requests.read().await.clone()
    }

    /// 注入一个待处理配对请求（discovery 收到对端广播配对请求时调用）
    pub async fn push_pairing_request(&self, req: PairingRequest) {
        let mut list = self.pending_pairing_requests.write().await;
        // 去重：相同 device_id 不重复入队
        if !list.iter().any(|r| r.device_id == req.device_id) {
            list.push(req);
        }
    }

    /// 取出已处理的配对请求（accept/reject 后调用）
    pub async fn pop_pairing_request(&self, device_id: &str) -> Option<PairingRequest> {
        let mut list = self.pending_pairing_requests.write().await;
        let pos = list.iter().position(|r| r.device_id == device_id)?;
        Some(list.remove(pos))
    }
}

// ── trait 实现 ────────────────────────────────────────────────────────────
// P2pManager 满足三个 trait 抽象 + RemoteTaskDispatcher，便于上层面向 trait 编程、
// 未来替换为真实实现时调用方零改动。

#[async_trait]
impl DiscoveryService for P2pManager {
    #[inline]
    async fn start_discovery(&self) -> Result<()> {
        if let Some(d) = self.discovery.read().await.as_ref() {
            d.start().await?;
        }
        Ok(())
    }

    #[inline]
    async fn stop_discovery(&self) -> Result<()> {
        if let Some(d) = self.discovery.read().await.as_ref() {
            d.stop().await?;
        }
        Ok(())
    }

    #[inline]
    async fn scan_once(&self) -> Result<Vec<Device>> {
        if let Some(d) = self.discovery.read().await.as_ref() {
            d.scan_once().await
        } else {
            Ok(Vec::new())
        }
    }

    #[inline]
    async fn list_devices(&self) -> Vec<Device> {
        let trusted = self.trust.list_peers().await;
        let online_ids = if let Some(t) = self.transport.read().await.as_ref() {
            t.online_device_ids().await
        } else {
            Vec::new()
        };
        let now = effisuite_core::remote_task_now();
        trusted
            .iter()
            .map(|p| {
                let online = online_ids.contains(&p.device_id);
                Device {
                    id: p.device_id.clone(),
                    name: p.name.clone(),
                    address: p.address.clone(),
                    last_seen: if online { now } else { p.last_seen },
                    status: if online {
                        DeviceStatus::Paired
                    } else {
                        DeviceStatus::Offline
                    },
                }
            })
            .collect()
    }
}

#[async_trait]
impl PairingService for P2pManager {
    #[inline]
    async fn pair_by_address(&self, address: &str, role: PairRole) -> Result<Device> {
        let pairing = self
            .pairing
            .read()
            .await
            .as_ref()
            .ok_or_else(|| CoreError::P2p("P2pManager not started".to_string()))?
            .clone();
        pairing.pair_by_address(address, role).await
    }

    #[inline]
    async fn accept_pair(&self, device_id: &str, role: PairRole) -> Result<()> {
        let pairing = self
            .pairing
            .read()
            .await
            .as_ref()
            .ok_or_else(|| CoreError::P2p("P2pManager not started".to_string()))?
            .clone();
        // 从待处理列表移除
        self.pop_pairing_request(device_id).await;
        pairing.accept_pair(device_id, role).await
    }

    #[inline]
    async fn reject_pair(&self, device_id: &str) -> Result<()> {
        // 从待处理列表移除并发布 Discovered 状态
        self.pop_pairing_request(device_id).await;
        self.event_bus.publish(BusEvent::DeviceStatusChanged {
            device_id: device_id.to_string(),
            status: DeviceStatus::Discovered,
        });
        Ok(())
    }

    #[inline]
    async fn unpair(&self, device_id: &str) -> Result<()> {
        // 关闭传输连接
        if let Some(t) = self.transport.read().await.as_ref() {
            if let Some(ch) = t.get_channel(device_id).await {
                ch.close().await;
            }
        }
        // 从信任库移除
        self.trust.remove_peer(device_id).await?;
        self.event_bus.publish(BusEvent::DeviceStatusChanged {
            device_id: device_id.to_string(),
            status: DeviceStatus::Offline,
        });
        Ok(())
    }
}

#[async_trait]
impl SyncService for P2pManager {
    #[inline]
    async fn pull(
        &self,
        device_id: &str,
        since: u64,
        kinds: &[SyncKind],
    ) -> Result<()> {
        let sync = self
            .sync
            .read()
            .await
            .as_ref()
            .ok_or_else(|| CoreError::P2p("P2pManager not started".to_string()))?
            .clone();
        sync.pull(device_id, since, kinds).await
    }

    #[inline]
    async fn push(&self, device_id: &str, kinds: &[SyncKind]) -> Result<()> {
        let sync = self
            .sync
            .read()
            .await
            .as_ref()
            .ok_or_else(|| CoreError::P2p("P2pManager not started".to_string()))?
            .clone();
        sync.push(device_id, kinds).await
    }

    #[inline]
    async fn sync_cursor(&self, device_id: &str) -> u64 {
        if let Some(s) = self.sync.read().await.as_ref() {
            s.sync_cursor(device_id).await
        } else {
            0
        }
    }
}

#[async_trait]
impl effisuite_core::RemoteTaskDispatcher for P2pManager {
    async fn list_online_devices(&self) -> Vec<Device> {
        let trusted = self.trust.list_peers().await;
        let online_ids = if let Some(t) = self.transport.read().await.as_ref() {
            t.online_device_ids().await
        } else {
            Vec::new()
        };
        let now = effisuite_core::remote_task_now();
        trusted
            .into_iter()
            .filter(|p| online_ids.contains(&p.device_id))
            .map(|p| Device {
                id: p.device_id,
                name: p.name,
                address: p.address,
                last_seen: now,
                status: DeviceStatus::Paired,
            })
            .collect()
    }

    async fn dispatch_remote_task(&self, device_id: &str, task: &str) -> Result<String> {
        let pairing = self
            .pairing
            .read()
            .await
            .as_ref()
            .ok_or_else(|| CoreError::P2p("P2pManager not started".to_string()))?
            .clone();
        pairing.dispatch_remote_task(device_id, task).await
    }
}

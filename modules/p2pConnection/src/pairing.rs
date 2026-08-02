//! P2P 配对协议：可信密钥交换 + 动态加密通道建立 + 远端任务派发 + 主机模式 RPC。
//!
//! # 职责
//! - **方法一（IP 直连）**：[`Pairing::pair_by_address`] — TCP 直连对端配对端口，
//!   明文交换 [`PairingHello`]/[`PairingAck`]（Ed25519 签名），双方 upsert 入信任库，
//!   再由 [`Transport::connect`] 建立加密通道。
//! - **方法二（广播发现→准许）**：[`Pairing::accept_pair`] — 从 [`EventBus`] 订阅
//!   [`BusEvent::PairingRequest`] 自动收集待配对设备地址，用户准许后发起配对握手。
//! - **远端任务派发**：[`Pairing::dispatch_remote_task`] — 经加密通道发送
//!   `TaskRequest`，pending map + oneshot 等待 `TaskResponse`，120s 超时。
//! - **主机模式 RPC**：[`Pairing::host_list_conversations`] 等三个方法 — replica 发起
//!   请求，pending map 等待 `HostReply`；host 端由 manager 注入 [`HostRpcHandler`]。
//!
//! # 信任引导
//! 首次配对时对端不在信任库，`Transport::connect` 会拒绝。Pairing 先在**独立 TCP 端口**
//! （`PAIRING_PORT = DEFAULT_P2P_PORT + 1`）明文交换 Ed25519 公钥（PairingHello/Ack
//! 均经签名，防篡改），upsert 入信任库后再调 `Transport::connect` 建立加密通道。
//!
//! # 并发模型
//! - pending map 用 `RwLock<HashMap>`（读多写少），临界区仅 HashMap 读写，
//!   `oneshot::send` 在锁外执行。
//! - 配对监听器在 `new()` 中 `tokio::spawn` 启动，bind 失败仅 warn 不致命。
//! - `EventBus` 订阅 task 自动收集 `PairingRequest` 事件填充 `pending_peers`。
//! - 标志位用 `AtomicBool`，不用 `Mutex<bool>`。
//!
//! # 内存
//! - 结构体字段按大小降序排列，`AtomicBool` 置末尾最小化 padding。
//! - 帧读写用 `with_capacity` 预分配。
//! - 迭代器适配器优先，禁止显式索引迭代。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use effisuite_core::{
    remote_task_now, BusEvent, CoreError, Device, DeviceStatus, EventBus, Message, Result,
};

use crate::crypto::{
    hex_encode, now_ts, ts_within_window, verify_signature, IdentityKey, PUBKEY_LEN,
};
use crate::protocol::{ConvManifestEntry, WireMessage};
use crate::transport::{Transport, DEFAULT_P2P_PORT, TASK_TIMEOUT};
use crate::trust::{PairRole, TrustStore, TrustedPeer};

/// 配对监听端口（与传输层 TCP 端口隔离，避免 Hello 帧与 PairingHello 帧冲突）。
const PAIRING_PORT: u16 = DEFAULT_P2P_PORT + 1;
/// 配对握手单帧最大长度（防恶意大帧 OOM，1 MiB）。
const MAX_PAIRING_FRAME: usize = 1024 * 1024;
/// 配对握手 TCP 操作超时（连接 / 读 / 写）。
const PAIRING_IO_TIMEOUT: Duration = Duration::from_secs(10);
/// 主机模式 RPC 默认超时。
const HOST_RPC_TIMEOUT: Duration = Duration::from_secs(30);

// ── 配对专属线路消息（明文 JSON，加密通道建立前的信任引导） ─────────────

/// 配对发起方 → 接收方：携带本机 Ed25519 公钥 + 签名 + 期望角色。
///
/// 字段按大小降序：`[u8;32]`(32) → String(24) → Vec(24) → u64(8) → PairRole(1)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingHello {
    pub pubkey: [u8; PUBKEY_LEN],
    pub device_id: String,
    pub name: String,
    pub signature: Vec<u8>,
    pub timestamp: u64,
    pub role: PairRole,
}

/// 配对接收方 → 发起方：对称结构，确认配对并回传本机公钥。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingAck {
    pub pubkey: [u8; PUBKEY_LEN],
    pub device_id: String,
    pub name: String,
    pub signature: Vec<u8>,
    pub timestamp: u64,
    pub role: PairRole,
}

impl PairingHello {
    /// 构造待签名载荷：`[pubkey || ts_be(8) || device_id || name]`。
    #[inline]
    fn signed_payload(&self) -> Vec<u8> {
        pairing_signed_payload(&self.pubkey, self.timestamp, &self.device_id, &self.name)
    }
}

impl PairingAck {
    #[inline]
    fn signed_payload(&self) -> Vec<u8> {
        pairing_signed_payload(&self.pubkey, self.timestamp, &self.device_id, &self.name)
    }
}

/// 构造 `[pubkey || ts_be(8) || device_id || name]` 待签名缓冲区。
#[inline]
fn pairing_signed_payload(
    pubkey: &[u8; PUBKEY_LEN],
    ts: u64,
    device_id: &str,
    name: &str,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(PUBKEY_LEN + 8 + device_id.len() + name.len());
    buf.extend_from_slice(pubkey);
    buf.extend_from_slice(&ts.to_be_bytes());
    buf.extend_from_slice(device_id.as_bytes());
    buf.extend_from_slice(name.as_bytes());
    buf
}

// ── 公共数据结构 ───────────────────────────────────────────────────────

/// 待处理配对请求（discovery 收集 → 前端展示 → 用户准许）。
///
/// 字段按大小降序：String(24) × 3 → u64(8)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingRequest {
    pub device_id: String,
    pub name: String,
    pub address: String,
    pub pubkey_hex: String,
    pub timestamp: u64,
}

/// 主机模式 RPC 处理器（manager 注入，避免 pairing 直接依赖 ConversationStore）。
#[async_trait]
pub trait HostRpcHandler: Send + Sync {
    /// 列出本机（host）所有会话清单。
    async fn list_conversations(&self) -> Result<Vec<ConvManifestEntry>>;
    /// 拉取本机（host）指定会话的消息列表。
    async fn get_conversation(&self, conv_id: &str) -> Result<Vec<Message>>;
    /// 向本机（host）指定会话发送消息，返回新消息 id。
    async fn send_message(&self, conv_id: &str, content: &str) -> Result<String>;
}

/// 远端任务响应数据（pending map value，经 oneshot 传递）。
type TaskResult = std::result::Result<String, String>;
/// 主机模式 RPC 响应数据（pending map value，经 oneshot 传递）。
type HostResult = std::result::Result<String, String>;

/// 内部待配对设备信息（由 EventBus PairingRequest 事件自动填充）。
struct PendingPeer {
    address: String,
}

// ── Pairing ────────────────────────────────────────────────────────────

/// P2P 配对协议管理器。
///
/// 字段按大小降序排列以最小化 padding：
/// `RwLock<HashMap>`（含内联 HashMap，~80B）→ `IdentityKey`(32B) → `String`(24B) →
/// `Arc`/`EventBus`/`TrustStore`/`StdMutex`(8B) → `AtomicBool`(1B)。
pub struct Pairing {
    /// 远端任务 pending map：request_id → oneshot sender。
    task_pending: RwLock<HashMap<String, oneshot::Sender<TaskResult>>>,
    /// 主机模式 RPC pending map：device_id → oneshot sender。
    host_rpc_pending: RwLock<HashMap<String, oneshot::Sender<HostResult>>>,
    /// 本机身份密钥（Ed25519，32 字节）。
    identity: IdentityKey,
    /// 本机 device_id（24 字节 String）。
    self_device_id: String,
    /// 待配对设备（由 EventBus PairingRequest 事件自动填充）。
    pending_peers: Arc<RwLock<HashMap<String, PendingPeer>>>,
    /// 主机模式 RPC 处理器（manager 注入）。
    host_rpc_handler: Arc<RwLock<Option<Arc<dyn HostRpcHandler>>>>,
    /// 加密 TCP 传输层。
    transport: Arc<Transport>,
    /// 事件总线。
    event_bus: EventBus,
    /// 信任库。
    trust: TrustStore,
    /// 配对监听器 task 句柄（stop 时 abort）。
    listener_handle: StdMutex<Option<JoinHandle<()>>>,
    /// 运行标志。
    running: AtomicBool,
}

impl Pairing {
    /// 构造配对管理器，自动启动配对监听器 + EventBus 订阅 task。
    ///
    /// `trust` / `identity` / `transport` / `event_bus` / `self_device_id`
    /// 由 manager 在 `start_with_trust` 中注入。
    pub fn new(
        trust: TrustStore,
        identity: IdentityKey,
        transport: Arc<Transport>,
        event_bus: EventBus,
        self_device_id: String,
    ) -> Self {
        let pending_peers: Arc<RwLock<HashMap<String, PendingPeer>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let host_rpc_handler: Arc<RwLock<Option<Arc<dyn HostRpcHandler>>>> =
            Arc::new(RwLock::new(None));

        // 订阅 EventBus PairingRequest 事件，自动填充 pending_peers。
        let mut rx = event_bus.subscribe();
        let pending_clone = Arc::clone(&pending_peers);
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(BusEvent::PairingRequest { device }) => {
                        let mut map = pending_clone.write().await;
                        map.insert(
                            device.id.clone(),
                            PendingPeer {
                                address: device.address.clone(),
                            },
                        );
                        debug!(device_id = %device.id, "pending peer registered from PairingRequest");
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(lagged = n, "pairing event subscriber lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        // 启动配对监听器（bind 失败仅 warn，不致命——本机仍可主动发起配对）。
        let listener_trust = trust.clone();
        let listener_identity = identity.clone();
        let listener_bus = event_bus.clone();
        let listener_dev_id = self_device_id.clone();
        let handle = tokio::spawn(async move {
            pairing_listener_loop(listener_trust, listener_identity, listener_bus, listener_dev_id)
                .await;
        });

        Self {
            task_pending: RwLock::new(HashMap::new()),
            host_rpc_pending: RwLock::new(HashMap::new()),
            identity,
            self_device_id,
            pending_peers,
            host_rpc_handler,
            transport,
            event_bus,
            trust,
            listener_handle: StdMutex::new(Some(handle)),
            running: AtomicBool::new(true),
        }
    }

    /// 方法一：通过 IP/链接配对。
    ///
    /// 1. TCP connect 到配对端口（`addr.port() + 1`）
    /// 2. 明文交换 PairingHello/PairingAck（首次配对交换公钥）
    /// 3. 双方 upsert_peer 入信任库
    /// 4. transport.connect 建立加密通道
    /// 5. 发布 DeviceStatusChanged(Paired) 事件
    pub async fn pair_by_address(&self, address: &str, role: PairRole) -> Result<Device> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(CoreError::P2p("pairing not running".to_string()));
        }

        let addr: SocketAddr = address
            .parse()
            .map_err(|e| CoreError::P2p(format!("invalid address {address}: {e}")))?;
        let pairing_addr = SocketAddr::new(addr.ip(), addr.port() + 1);

        info!(%address, ?role, "initiating pairing by address");

        let ack = self.initiate_pairing_exchange(pairing_addr, role).await?;

        // upsert 对端入信任库
        let peer = TrustedPeer {
            device_id: ack.device_id.clone(),
            name: ack.name.clone(),
            pubkey_hex: hex_encode(&ack.pubkey),
            address: address.to_string(),
            paired_at: now_ts(),
            last_seen: now_ts(),
            role,
        };
        self.trust.upsert_peer(peer).await?;

        // 建立加密通道（失败不影响配对本身，密钥已交换）
        match self.transport.connect(addr).await {
            Ok(_) => debug!(device_id = %ack.device_id, "encrypted channel established after pairing"),
            Err(e) => warn!(error = %e, device_id = %ack.device_id, "encrypted channel failed after pairing (pairing still succeeded)"),
        }

        self.event_bus.publish(BusEvent::DeviceStatusChanged {
            device_id: ack.device_id.clone(),
            status: DeviceStatus::Paired,
        });

        info!(device_id = %ack.device_id, "pairing by address succeeded");

        Ok(Device {
            id: ack.device_id,
            name: ack.name,
            address: address.to_string(),
            last_seen: remote_task_now(),
            status: DeviceStatus::Paired,
        })
    }

    /// 方法二：接受远端配对请求。
    ///
    /// address 已通过 discovery 收集（EventBus PairingRequest 事件自动填充
    /// `pending_peers`），向对端发起 PairingHello 流程。
    pub async fn accept_pair(&self, device_id: &str, role: PairRole) -> Result<()> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(CoreError::P2p("pairing not running".to_string()));
        }

        // 从 pending_peers 获取对端地址
        let peer_addr = {
            let map = self.pending_peers.read().await;
            map.get(device_id).map(|p| p.address.clone())
        };

        let peer_addr = peer_addr
            .ok_or_else(|| CoreError::P2p(format!("no pending pairing request for {device_id}")))?;

        info!(%device_id, address = %peer_addr, ?role, "accepting pairing");

        let addr: SocketAddr = peer_addr
            .parse()
            .map_err(|e| CoreError::P2p(format!("invalid peer address {peer_addr}: {e}")))?;
        let pairing_addr = SocketAddr::new(addr.ip(), addr.port() + 1);

        let ack = self.initiate_pairing_exchange(pairing_addr, role).await?;

        let trusted_peer = TrustedPeer {
            device_id: ack.device_id.clone(),
            name: ack.name.clone(),
            pubkey_hex: hex_encode(&ack.pubkey),
            address: peer_addr.clone(),
            paired_at: now_ts(),
            last_seen: now_ts(),
            role,
        };
        self.trust.upsert_peer(trusted_peer).await?;

        match self.transport.connect(addr).await {
            Ok(_) => debug!(device_id = %ack.device_id, "encrypted channel established after accept"),
            Err(e) => warn!(error = %e, device_id = %ack.device_id, "encrypted channel failed after accept (pairing still succeeded)"),
        }

        // 从 pending_peers 移除
        {
            let mut map = self.pending_peers.write().await;
            map.remove(device_id);
        }

        self.event_bus.publish(BusEvent::DeviceStatusChanged {
            device_id: ack.device_id.clone(),
            status: DeviceStatus::Paired,
        });

        info!(device_id = %ack.device_id, "accept_pair succeeded");
        Ok(())
    }

    /// 处理入站消息（由 manager 路由调用）。
    ///
    /// 处理 TaskResponse / HostReply / Ack / HostListConversations /
    /// HostGetConversation / HostSendMessage。
    pub async fn handle_incoming(&self, device_id: &str, msg: WireMessage) -> Result<()> {
        match msg {
            WireMessage::TaskResponse {
                request_id,
                result,
                is_error,
            } => {
                self.handle_task_response(request_id, result, is_error)
                    .await
            }
            WireMessage::HostReply { ok, payload } => {
                self.handle_host_reply(device_id, ok, payload).await
            }
            WireMessage::HostListConversations => {
                self.handle_host_list(device_id).await
            }
            WireMessage::HostGetConversation { conversation_id } => {
                self.handle_host_get(device_id, &conversation_id).await
            }
            WireMessage::HostSendMessage {
                conversation_id,
                content,
            } => {
                self.handle_host_send(device_id, &conversation_id, &content)
                    .await
            }
            WireMessage::Ack { ok, msg: ack_msg } => {
                debug!(%device_id, ok, %ack_msg, "received Ack");
                Ok(())
            }
            _ => {
                warn!(%device_id, "unexpected wire message in pairing handle_incoming");
                Ok(())
            }
        }
    }

    /// 远端任务派发（AI 跨设备指派）。
    ///
    /// 1. 生成 request_id（uuid）
    /// 2. pending map 注册 request_id → oneshot::Sender
    /// 3. 通过 transport channel 发送 TaskRequest
    /// 4. 等待 TaskResponse（120s 超时）
    pub async fn dispatch_remote_task(&self, device_id: &str, task: &str) -> Result<String> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(CoreError::P2p("pairing not running".to_string()));
        }

        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel::<TaskResult>();

        // 注册 pending（临界区仅 HashMap insert）
        {
            let mut map = self.task_pending.write().await;
            map.insert(request_id.clone(), tx);
        }

        // 获取 channel 并发送 TaskRequest
        let channel = self
            .transport
            .get_channel(device_id)
            .await
            .ok_or_else(|| CoreError::P2p(format!("device {device_id} not connected")))?;

        if let Err(e) = channel
            .send(WireMessage::TaskRequest {
                request_id: request_id.clone(),
                task: task.to_string(),
            })
            .await
        {
            // 发送失败，清理 pending
            let mut map = self.task_pending.write().await;
            map.remove(&request_id);
            return Err(e);
        }

        debug!(%device_id, %request_id, "task request dispatched, awaiting response");

        // 等待响应（带超时）
        match tokio::time::timeout(TASK_TIMEOUT, rx).await {
            Ok(Ok(task_result)) => task_result.map_err(CoreError::P2p),
            Ok(Err(_)) => Err(CoreError::P2p("task response channel closed".to_string())),
            Err(_) => {
                // 超时，清理 pending
                let mut map = self.task_pending.write().await;
                map.remove(&request_id);
                Err(CoreError::P2p("task timeout".to_string()))
            }
        }
    }

    /// 主机模式 RPC：列出主机会话清单（replica → host）。
    pub async fn host_list_conversations(
        &self,
        device_id: &str,
    ) -> Result<Vec<ConvManifestEntry>> {
        let payload = self
            .do_host_rpc(device_id, WireMessage::HostListConversations)
            .await?;
        serde_json::from_str(&payload).map_err(CoreError::Serde)
    }

    /// 主机模式 RPC：拉取主机指定会话消息（replica → host）。
    pub async fn host_get_conversation(
        &self,
        device_id: &str,
        conv_id: &str,
    ) -> Result<Vec<Message>> {
        let payload = self
            .do_host_rpc(
                device_id,
                WireMessage::HostGetConversation {
                    conversation_id: conv_id.to_string(),
                },
            )
            .await?;
        serde_json::from_str(&payload).map_err(CoreError::Serde)
    }

    /// 主机模式 RPC：向主机指定会话发送消息（replica → host）。
    pub async fn host_send_message(
        &self,
        device_id: &str,
        conv_id: &str,
        content: &str,
    ) -> Result<String> {
        let payload = self
            .do_host_rpc(
                device_id,
                WireMessage::HostSendMessage {
                    conversation_id: conv_id.to_string(),
                    content: content.to_string(),
                },
            )
            .await?;
        Ok(payload)
    }

    /// 注入主机模式 RPC 处理器（manager 在启动时注入）。
    pub async fn set_host_rpc_handler(&self, handler: Option<Arc<dyn HostRpcHandler>>) {
        let mut guard = self.host_rpc_handler.write().await;
        *guard = handler;
    }

    /// 停止配对管理器：中止监听器 task，清理 pending maps。
    pub async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);

        // 中止监听器 task
        if let Some(h) = self
            .listener_handle
            .lock()
            .ok()
            .and_then(|mut g| g.take())
        {
            h.abort();
        }

        // 清理 pending maps，让等待者收到错误
        self.task_pending.write().await.clear();
        self.host_rpc_pending.write().await.clear();

        info!("pairing stopped");
        Ok(())
    }

    // ── 内部方法 ──────────────────────────────────────────────────────

    /// 构造 PairingHello（本机公钥 + 签名 + 角色）。
    #[inline]
    fn build_pairing_hello(&self, role: PairRole) -> PairingHello {
        let pubkey = self.identity.public_bytes();
        let timestamp = now_ts();
        let payload =
            pairing_signed_payload(&pubkey, timestamp, &self.self_device_id, &self.self_device_id);
        let signature = self.identity.sign(&payload).to_vec();
        PairingHello {
            pubkey,
            device_id: self.self_device_id.clone(),
            name: self.self_device_id.clone(),
            signature,
            timestamp,
            role,
        }
    }

    /// 发起配对握手：TCP connect → 发 PairingHello → 收 PairingAck → 验签。
    async fn initiate_pairing_exchange(
        &self,
        pairing_addr: SocketAddr,
        role: PairRole,
    ) -> Result<PairingAck> {
        // 1. TCP connect 到配对端口
        let stream = tokio::time::timeout(PAIRING_IO_TIMEOUT, TcpStream::connect(pairing_addr))
            .await
            .map_err(|_| CoreError::P2p("pairing connect timeout".to_string()))?
            .map_err(|e| CoreError::P2p(format!("pairing connect {pairing_addr}: {e}")))?;

        let mut stream = stream;

        // 2. 发送 PairingHello
        let hello = self.build_pairing_hello(role);
        write_pairing_frame(&mut stream, &hello).await?;

        // 3. 接收 PairingAck
        let ack: PairingAck =
            tokio::time::timeout(PAIRING_IO_TIMEOUT, read_pairing_frame(&mut stream))
                .await
                .map_err(|_| CoreError::P2p("pairing ack timeout".to_string()))??;

        // 4. 验证 ack 签名
        verify_pairing_signature(&ack.pubkey, &ack.signed_payload(), &ack.signature)?;

        // 5. 校验时间戳防重放
        if !ts_within_window(ack.timestamp, now_ts()) {
            return Err(CoreError::P2p(
                "pairing ack timestamp out of window".to_string(),
            ));
        }

        Ok(ack)
    }

    /// 处理 TaskResponse：从 task_pending 取出 oneshot sender 发送结果。
    async fn handle_task_response(
        &self,
        request_id: String,
        result: String,
        is_error: bool,
    ) -> Result<()> {
        // 临界区仅 HashMap remove，oneshot send 在锁外
        let sender = {
            let mut map = self.task_pending.write().await;
            map.remove(&request_id)
        };

        if let Some(sender) = sender {
            let data = if is_error {
                Err(result)
            } else {
                Ok(result)
            };
            let _ = sender.send(data);
            debug!(%request_id, is_error, "task response delivered to pending caller");
        } else {
            warn!(%request_id, "task response received but no pending caller");
        }
        Ok(())
    }

    /// 处理 HostReply：从 host_rpc_pending 取出 oneshot sender 发送结果。
    async fn handle_host_reply(
        &self,
        device_id: &str,
        ok: bool,
        payload: String,
    ) -> Result<()> {
        let sender = {
            let mut map = self.host_rpc_pending.write().await;
            map.remove(device_id)
        };

        if let Some(sender) = sender {
            let data = if ok {
                Ok(payload)
            } else {
                Err(payload)
            };
            let _ = sender.send(data);
            debug!(%device_id, ok, "host reply delivered to pending caller");
        } else {
            warn!(%device_id, "host reply received but no pending caller");
        }
        Ok(())
    }

    /// host 端处理 HostListConversations：调用 handler → 回 HostReply。
    async fn handle_host_list(&self, device_id: &str) -> Result<()> {
        let handler = self.get_host_rpc_handler().await;

        let (ok, payload) = match handler {
            Some(h) => match h.list_conversations().await {
                Ok(list) => {
                    let json = serde_json::to_string(&list)?;
                    (true, json)
                }
                Err(e) => (false, e.to_string()),
            },
            None => (
                false,
                "host rpc handler not configured".to_string(),
            ),
        };

        self.send_host_reply(device_id, ok, payload).await
    }

    /// host 端处理 HostGetConversation：调用 handler → 回 HostReply。
    async fn handle_host_get(&self, device_id: &str, conv_id: &str) -> Result<()> {
        let handler = self.get_host_rpc_handler().await;

        let (ok, payload) = match handler {
            Some(h) => match h.get_conversation(conv_id).await {
                Ok(msgs) => {
                    let json = serde_json::to_string(&msgs)?;
                    (true, json)
                }
                Err(e) => (false, e.to_string()),
            },
            None => (
                false,
                "host rpc handler not configured".to_string(),
            ),
        };

        self.send_host_reply(device_id, ok, payload).await
    }

    /// host 端处理 HostSendMessage：调用 handler → 回 HostReply。
    async fn handle_host_send(
        &self,
        device_id: &str,
        conv_id: &str,
        content: &str,
    ) -> Result<()> {
        let handler = self.get_host_rpc_handler().await;

        let (ok, payload) = match handler {
            Some(h) => match h.send_message(conv_id, content).await {
                Ok(msg_id) => (true, msg_id),
                Err(e) => (false, e.to_string()),
            },
            None => (
                false,
                "host rpc handler not configured".to_string(),
            ),
        };

        self.send_host_reply(device_id, ok, payload).await
    }

    /// 读取 host_rpc_handler（短临界区：仅 read + Arc clone，无 IO）。
    #[inline]
    async fn get_host_rpc_handler(&self) -> Option<Arc<dyn HostRpcHandler>> {
        let guard = self.host_rpc_handler.read().await;
        guard.as_ref().map(Arc::clone)
    }

    /// 发送 HostReply 到对端。
    async fn send_host_reply(
        &self,
        device_id: &str,
        ok: bool,
        payload: String,
    ) -> Result<()> {
        let channel = self
            .transport
            .get_channel(device_id)
            .await
            .ok_or_else(|| CoreError::P2p(format!("no channel to {device_id}")))?;
        channel
            .send(WireMessage::HostReply { ok, payload })
            .await
    }

    /// 主机模式 RPC 通用流程：注册 pending → 发请求 → 等待 HostReply（带超时）。
    async fn do_host_rpc(&self, device_id: &str, msg: WireMessage) -> Result<String> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(CoreError::P2p("pairing not running".to_string()));
        }

        let (tx, rx) = oneshot::channel::<HostResult>();

        // 注册 pending（device_id 为 key，同一设备同时只能有一个待处理 host RPC）
        {
            let mut map = self.host_rpc_pending.write().await;
            if map.contains_key(device_id) {
                return Err(CoreError::P2p(format!(
                    "concurrent host rpc to {device_id} already pending"
                )));
            }
            map.insert(device_id.to_string(), tx);
        }

        // 获取 channel 并发送请求
        let channel = self
            .transport
            .get_channel(device_id)
            .await
            .ok_or_else(|| CoreError::P2p(format!("device {device_id} not connected")))?;

        if let Err(e) = channel.send(msg).await {
            let mut map = self.host_rpc_pending.write().await;
            map.remove(device_id);
            return Err(e);
        }

        // 等待 HostReply
        match tokio::time::timeout(HOST_RPC_TIMEOUT, rx).await {
            Ok(Ok(host_result)) => host_result.map_err(CoreError::P2p),
            Ok(Err(_)) => Err(CoreError::P2p("host reply channel closed".to_string())),
            Err(_) => {
                let mut map = self.host_rpc_pending.write().await;
                map.remove(device_id);
                Err(CoreError::P2p("host rpc timeout".to_string()))
            }
        }
    }
}

impl Drop for Pairing {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        // 兜底：stop() 未调用时中止监听器 task，防止泄漏
        if let Some(h) = self
            .listener_handle
            .lock()
            .ok()
            .and_then(|mut g| g.take())
        {
            h.abort();
        }
    }
}

// ── 配对监听器（服务端：接收 PairingHello → 回 PairingAck） ──────────────

/// 配对监听器主循环：accept → 每连接 spawn handler。
async fn pairing_listener_loop(
    trust: TrustStore,
    identity: IdentityKey,
    event_bus: EventBus,
    self_device_id: String,
) {
    let listener = match TcpListener::bind(("0.0.0.0", PAIRING_PORT)).await {
        Ok(l) => {
            info!(port = PAIRING_PORT, "pairing listener started");
            l
        }
        Err(e) => {
            warn!(error = %e, port = PAIRING_PORT, "pairing listener bind failed (incoming pairing disabled)");
            return;
        }
    };

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                debug!(%peer, "incoming pairing connection");
                let trust = trust.clone();
                let identity = identity.clone();
                let event_bus = event_bus.clone();
                let self_device_id = self_device_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_pairing_connection(
                        stream,
                        trust,
                        identity,
                        event_bus,
                        self_device_id,
                    )
                    .await
                    {
                        warn!(error = %e, %peer, "pairing connection handler error");
                    }
                });
            }
            Err(e) => {
                warn!(error = %e, "pairing listener accept error");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

/// 处理单条配对连接：读 PairingHello → 验签 → upsert → 回 PairingAck。
async fn handle_pairing_connection(
    mut stream: TcpStream,
    trust: TrustStore,
    identity: IdentityKey,
    event_bus: EventBus,
    self_device_id: String,
) -> Result<()> {
    // 1. 读取 PairingHello
    let hello: PairingHello =
        tokio::time::timeout(PAIRING_IO_TIMEOUT, read_pairing_frame::<PairingHello>(&mut stream))
            .await
            .map_err(|_| CoreError::P2p("pairing hello read timeout".to_string()))??;

    // 2. 验证签名
    verify_pairing_signature(&hello.pubkey, &hello.signed_payload(), &hello.signature)?;

    // 3. 校验时间戳
    if !ts_within_window(hello.timestamp, now_ts()) {
        return Err(CoreError::P2p(
            "pairing hello timestamp out of window".to_string(),
        ));
    }

    // 4. upsert 发起方入信任库
    let peer = TrustedPeer {
        device_id: hello.device_id.clone(),
        name: hello.name.clone(),
        pubkey_hex: hex_encode(&hello.pubkey),
        address: stream
            .peer_addr()
            .map(|a| a.to_string())
            .unwrap_or_default(),
        paired_at: now_ts(),
        last_seen: now_ts(),
        role: hello.role,
    };
    trust.upsert_peer(peer).await?;

    // 5. 构造并发送 PairingAck
    let ack = build_pairing_ack(&identity, &self_device_id, hello.role);
    write_pairing_frame(&mut stream, &ack).await?;

    // 6. 发布事件
    event_bus.publish(BusEvent::DeviceStatusChanged {
        device_id: hello.device_id.clone(),
        status: DeviceStatus::Paired,
    });

    info!(device_id = %hello.device_id, "pairing connection handled (server side)");
    Ok(())
}

/// 构造 PairingAck（服务端：用本机身份签名）。
#[inline]
fn build_pairing_ack(
    identity: &IdentityKey,
    self_device_id: &str,
    role: PairRole,
) -> PairingAck {
    let pubkey = identity.public_bytes();
    let timestamp = now_ts();
    let payload = pairing_signed_payload(&pubkey, timestamp, self_device_id, self_device_id);
    let signature = identity.sign(&payload).to_vec();
    PairingAck {
        pubkey,
        device_id: self_device_id.to_string(),
        name: self_device_id.to_string(),
        signature,
        timestamp,
        role,
    }
}

// ── 配对帧读写（4 字节大端长度前缀，与传输层帧格式一致但独立定义） ──────

/// 写 4B 长度前缀 + payload。
async fn write_frame<W: AsyncWriteExt + Unpin>(w: &mut W, payload: &[u8]) -> Result<()> {
    if payload.len() > MAX_PAIRING_FRAME {
        return Err(CoreError::P2p(format!(
            "pairing frame too large: {} > {MAX_PAIRING_FRAME}",
            payload.len()
        )));
    }
    let len = (payload.len() as u32).to_be_bytes();
    w.write_all(&len).await.map_err(CoreError::Io)?;
    w.write_all(payload).await.map_err(CoreError::Io)?;
    w.flush().await.map_err(CoreError::Io)?;
    Ok(())
}

/// 读 4B 长度前缀 + payload。EOF 返回 None。
async fn read_frame<R: AsyncReadExt + Unpin>(r: &mut R) -> Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(CoreError::Io(e)),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 {
        return Ok(Some(Vec::new()));
    }
    if len > MAX_PAIRING_FRAME {
        return Err(CoreError::P2p(format!(
            "pairing frame too large: {len} > {MAX_PAIRING_FRAME}"
        )));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await.map_err(CoreError::Io)?;
    Ok(Some(buf))
}

/// 序列化 + 写帧。
async fn write_pairing_frame<T: Serialize>(stream: &mut TcpStream, msg: &T) -> Result<()> {
    let json = serde_json::to_vec(msg)?;
    write_frame(stream, &json).await
}

/// 读帧 + 反序列化。
async fn read_pairing_frame<T: serde::de::DeserializeOwned>(
    stream: &mut TcpStream,
) -> Result<T> {
    let frame = read_frame(stream)
        .await?
        .ok_or_else(|| CoreError::P2p("eof during pairing exchange".to_string()))?;
    serde_json::from_slice(&frame).map_err(CoreError::Serde)
}

/// 验证配对签名，失败时包装为 CoreError::P2p。
#[inline]
fn verify_pairing_signature(
    pubkey: &[u8; PUBKEY_LEN],
    msg: &[u8],
    signature: &[u8],
) -> Result<()> {
    verify_signature(pubkey, msg, signature)
        .map_err(|e| CoreError::P2p(format!("pairing signature invalid: {e}")))
}

// ── 单元测试 ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_hello_serde_roundtrip() {
        let identity = IdentityKey::generate();
        let pubkey = identity.public_bytes();
        let ts = now_ts();
        let payload = pairing_signed_payload(&pubkey, ts, "dev-1", "Laptop");
        let sig = identity.sign(&payload).to_vec();

        let hello = PairingHello {
            pubkey,
            device_id: "dev-1".to_string(),
            name: "Laptop".to_string(),
            signature: sig.clone(),
            timestamp: ts,
            role: PairRole::Mirror,
        };

        let json = serde_json::to_string(&hello).unwrap();
        let back: PairingHello = serde_json::from_str(&json).unwrap();
        assert_eq!(back.device_id, "dev-1");
        assert_eq!(back.name, "Laptop");
        assert_eq!(back.pubkey, pubkey);
        assert_eq!(back.timestamp, ts);
        assert_eq!(back.signature, sig);
        assert_eq!(back.role, PairRole::Mirror);

        // 签名经 serde 往返后仍可验证
        assert!(verify_signature(&back.pubkey, &back.signed_payload(), &back.signature).is_ok());
    }

    #[test]
    fn pairing_ack_serde_roundtrip() {
        let identity = IdentityKey::generate();
        let pubkey = identity.public_bytes();
        let ts = now_ts();
        let payload = pairing_signed_payload(&pubkey, ts, "dev-2", "Phone");
        let sig = identity.sign(&payload).to_vec();

        let ack = PairingAck {
            pubkey,
            device_id: "dev-2".to_string(),
            name: "Phone".to_string(),
            signature: sig.clone(),
            timestamp: ts,
            role: PairRole::Host,
        };

        let json = serde_json::to_string(&ack).unwrap();
        let back: PairingAck = serde_json::from_str(&json).unwrap();
        assert_eq!(back.device_id, "dev-2");
        assert_eq!(back.name, "Phone");
        assert_eq!(back.pubkey, pubkey);
        assert_eq!(back.timestamp, ts);
        assert_eq!(back.signature, sig);
        assert_eq!(back.role, PairRole::Host);

        // 签名经验证有效
        assert!(verify_signature(&back.pubkey, &back.signed_payload(), &back.signature).is_ok());
    }

    #[test]
    fn pairing_request_construct() {
        let req = PairingRequest {
            device_id: "dev-abc".to_string(),
            name: "My Device".to_string(),
            address: "192.168.1.10:47823".to_string(),
            pubkey_hex: hex_encode(&[1u8; PUBKEY_LEN]),
            timestamp: 12345,
        };
        assert_eq!(req.device_id, "dev-abc");
        assert_eq!(req.name, "My Device");
        assert_eq!(req.address, "192.168.1.10:47823");
        assert_eq!(req.timestamp, 12345);
        assert!(!req.pubkey_hex.is_empty());
        // pubkey_hex 应为 64 字符（32 字节 × 2 hex 字符）
        assert_eq!(req.pubkey_hex.len(), PUBKEY_LEN * 2);
    }

    #[tokio::test]
    async fn host_rpc_handler_trait_object() {
        struct DummyHandler;

        #[async_trait]
        impl HostRpcHandler for DummyHandler {
            async fn list_conversations(&self) -> Result<Vec<ConvManifestEntry>> {
                Ok(Vec::new())
            }
            async fn get_conversation(&self, _conv_id: &str) -> Result<Vec<Message>> {
                Ok(Vec::new())
            }
            async fn send_message(&self, _conv_id: &str, _content: &str) -> Result<String> {
                Ok("msg-id".to_string())
            }
        }

        // 验证可被构造为 trait object（编译期验证 Send + Sync 约束）
        let handler: Arc<dyn HostRpcHandler> = Arc::new(DummyHandler);

        // 运行时验证方法可调用
        let list = handler.list_conversations().await.unwrap();
        assert!(list.is_empty());

        let msgs = handler.get_conversation("c1").await.unwrap();
        assert!(msgs.is_empty());

        let msg_id = handler.send_message("c1", "hi").await.unwrap();
        assert_eq!(msg_id, "msg-id");
    }

    #[test]
    fn pairing_signed_payload_layout() {
        let pubkey = [7u8; PUBKEY_LEN];
        let ts = 1000u64;
        let device_id = "dev-1";
        let name = "Laptop";
        let p = pairing_signed_payload(&pubkey, ts, device_id, name);

        let mut expected = Vec::with_capacity(PUBKEY_LEN + 8 + device_id.len() + name.len());
        expected.extend_from_slice(&pubkey);
        expected.extend_from_slice(&ts.to_be_bytes());
        expected.extend_from_slice(device_id.as_bytes());
        expected.extend_from_slice(name.as_bytes());
        assert_eq!(p, expected);
        assert_eq!(p.len(), PUBKEY_LEN + 8 + device_id.len() + name.len());
    }

    #[test]
    fn verify_pairing_signature_rejects_tampered() {
        let identity = IdentityKey::generate();
        let pubkey = identity.public_bytes();
        let ts = now_ts();
        let payload = pairing_signed_payload(&pubkey, ts, "dev", "name");
        let sig = identity.sign(&payload);

        // 正确签名验证通过
        assert!(verify_pairing_signature(&pubkey, &payload, &sig).is_ok());

        // 篡改消息验证失败
        let tampered = pairing_signed_payload(&pubkey, ts, "evil", "name");
        assert!(verify_pairing_signature(&pubkey, &tampered, &sig).is_err());

        // 错误公钥验证失败
        let other = IdentityKey::generate();
        assert!(verify_pairing_signature(&other.public_bytes(), &payload, &sig).is_err());
    }

    #[test]
    fn build_pairing_ack_produces_valid_signature() {
        let identity = IdentityKey::generate();
        let dev_id = "dev-test";
        let ack = build_pairing_ack(&identity, dev_id, PairRole::Replica);

        assert_eq!(ack.device_id, dev_id);
        assert_eq!(ack.name, dev_id);
        assert_eq!(ack.role, PairRole::Replica);
        assert_eq!(ack.pubkey, identity.public_bytes());

        // 签名应有效
        let payload = pairing_signed_payload(&ack.pubkey, ack.timestamp, &ack.device_id, &ack.name);
        assert!(verify_signature(&ack.pubkey, &payload, &ack.signature).is_ok());
    }
}

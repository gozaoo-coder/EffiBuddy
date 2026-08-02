//! 加密 TCP 传输层：可信对等节点间的长连接、握手、心跳与帧收发。
//!
//! # 协议栈
//! ```text
//!  ┌─────────────────────────────────────────┐
//!  │  WireMessage (JSON, 见 protocol.rs)      │  ← 业务消息
//!  ├─────────────────────────────────────────┤
//!  │  AES-256-GCM 加密（见 crypto.rs）         │  ← 机密性 + 完整性
//!  ├─────────────────────────────────────────┤
//!  │  长度前缀帧 [4B BE len][payload]         │  ← 裸 read_exact/write_all
//!  ├─────────────────────────────────────────┤
//!  │  TCP                                     │  ← 可靠字节流
//!  └─────────────────────────────────────────┘
//! ```
//!
//! # 握手（前向保密）
//! 1. 客户端 connect → 发 `Hello`（**明文** JSON，含本端 device_id + 临时公钥 + Ed25519 签名）
//! 2. 服务端校验签名（用信任库中对端公钥）→ 派生会话密钥 → 发 `HelloAck` → 安装 `SessionCipher`
//! 3. 客户端校验 HelloAck 签名 → 派生会话密钥 → 安装 `SessionCipher`
//! 4. 之后所有帧均加密。会话密钥每次连接一次性，前向保密。
//!
//! # 心跳与离线判定
//! 心跳巡检 task 每 5s 给所有在线连接发 `Ping`。reader task 每收到一帧即更新
//! `last_activity`。巡检 task 检查 `now - last_activity > 15s` 则判定离线，
//! 关闭连接、移除连接表条目、发布 `DeviceStatusChanged(Offline)`。
//!
//! # 并发模型
//! - 每连接两个 task：reader（解码→解密→推入 incoming mpsc）/ writer（从 channel mpsc 取→加密→写）
//! - 连接表 `RwLock<HashMap<device_id, ConnEntry>>`：读多写少；临界区仅 HashMap 读写
//! - `IncomingMessage` 经 mpsc 单消费者交付 manager 路由
//! - reader task 持 `Arc<Transport>`，退出时调用 `cleanup_conn` 做表清理 + 离线事件

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use effisuite_core::{BusEvent, CoreError, DeviceStatus, EventBus, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::crypto::{
    now_ts, ts_within_window, verify_signature, EphemeralKeypair, IdentityKey, SessionCipher,
};
use crate::protocol::WireMessage;
use crate::trust::TrustStore;

/// P2P 默认监听端口
pub const DEFAULT_P2P_PORT: u16 = 47823;
/// 心跳间隔
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// 离线判定超时（无任何帧到达）
const OFFLINE_TIMEOUT: Duration = Duration::from_secs(15);
/// 单帧最大长度（防恶意大帧 OOM，4 MiB）
const MAX_FRAME_LEN: usize = 4 * 1024 * 1024;
/// 派发远端任务默认超时
pub const TASK_TIMEOUT: Duration = Duration::from_secs(120);

/// 交付 manager 的入站消息（已解密、已解析）
#[derive(Debug)]
pub struct IncomingMessage {
    /// 来源设备 id（握手后填充）
    pub device_id: String,
    /// 解密后的业务消息
    pub message: WireMessage,
}

/// 单连接的发送句柄（克隆廉价，内部 mpsc::Sender 为 Arc）
#[derive(Clone)]
pub struct Channel {
    device_id: String,
    sender: mpsc::Sender<FrameOut>,
}

impl Channel {
    /// 向对端发送一条业务消息（异步排队，由 writer task 加密写出）
    pub async fn send(&self, msg: WireMessage) -> Result<()> {
        self.sender
            .send(FrameOut::Message(msg))
            .await
            .map_err(|_| CoreError::P2p("channel closed".to_string()))
    }

    /// 主动关闭连接
    pub async fn close(&self) {
        let _ = self.sender.send(FrameOut::Close).await;
    }

    #[inline]
    pub fn device_id(&self) -> &str {
        &self.device_id
    }
}

/// writer task 的输出指令
enum FrameOut {
    Message(WireMessage),
    Close,
}

/// 连接表条目
struct ConnEntry {
    channel: Channel,
    /// 最近一次收到对端帧的时间戳（毫秒，AtomicU64）
    last_activity: Arc<AtomicU64>,
    /// 关闭信号：drop/abort 时触发 reader/writer task 退出
    cancel: tokio::sync::oneshot::Sender<()>,
}

/// 传输层（管理监听器 + 连接表 + 入站消息流）
///
/// 字段按大小降序：多个 8 字节句柄在前，AtomicBool(1) 在后。
pub struct Transport {
    trust: TrustStore,
    identity: RwLock<IdentityKey>,
    event_bus: EventBus,
    conns: RwLock<HashMap<String, ConnEntry>>,
    /// 入站消息发送端（start 时回填，供 server/client 两条握手路径共用）
    incoming_tx: RwLock<Option<mpsc::Sender<IncomingMessage>>>,
    listener_handle: Mutex<Option<JoinHandle<()>>>,
    heartbeat_handle: Mutex<Option<JoinHandle<()>>>,
    self_device_id: RwLock<String>,
    bind_addr: RwLock<Option<SocketAddr>>,
    running: AtomicBool,
}

impl Transport {
    /// 构造（不启动监听）。`identity` 在 trust store 首次加载后注入。
    /// `self_device_id` 由 manager 在 trust store 加载后传入（避免在 new 中阻塞）。
    pub fn new(
        trust: TrustStore,
        identity: IdentityKey,
        event_bus: EventBus,
        self_device_id: String,
    ) -> Self {
        Self {
            trust,
            identity: RwLock::new(identity),
            event_bus,
            conns: RwLock::new(HashMap::new()),
            incoming_tx: RwLock::new(None),
            listener_handle: Mutex::new(None),
            heartbeat_handle: Mutex::new(None),
            self_device_id: RwLock::new(self_device_id),
            bind_addr: RwLock::new(None),
            running: AtomicBool::new(false),
        }
    }

    /// 异步构造：从 trust store 读取 self_device_id 后构造 Transport。
    /// 推荐入口，避免在 new 中阻塞 runtime。
    pub async fn from_trust(
        trust: TrustStore,
        identity: IdentityKey,
        event_bus: EventBus,
    ) -> Self {
        let self_device_id = trust.self_device_id().await;
        Self::new(trust, identity, event_bus, self_device_id)
    }

    /// 启动 TCP 监听 + 心跳巡检。返回入站消息接收器（manager 消费）。
    pub async fn start(
        self: &Arc<Self>,
        bind: SocketAddr,
    ) -> Result<mpsc::Receiver<IncomingMessage>> {
        if self.running.swap(true, Ordering::Relaxed) {
            return Err(CoreError::P2p("transport already running".to_string()));
        }
        let listener = TcpListener::bind(bind)
            .await
            .map_err(|e| CoreError::P2p(format!("bind {bind}: {e}")))?;
        // 写入实际监听地址（bind 用 0 端口时由 OS 分配，需读 listener.local_addr）
        let actual_bind = listener
            .local_addr()
            .map_err(|e| CoreError::P2p(format!("local_addr: {e}")))?;
        *self.bind_addr.write().await = Some(actual_bind);
        info!(addr = %actual_bind, "P2P transport listening");

        let (incoming_tx, incoming_rx) = mpsc::channel(64);
        *self.incoming_tx.write().await = Some(incoming_tx.clone());

        // 接受连接 task
        let self_clone = Arc::clone(self);
        let tx_for_accept = incoming_tx.clone();
        let listener_handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, peer)) => {
                        debug!(%peer, "incoming tcp connection");
                        let self_clone = Arc::clone(&self_clone);
                        let tx = tx_for_accept.clone();
                        tokio::spawn(async move {
                            if let Err(e) = self_clone.handle_incoming(stream, tx).await {
                                warn!(error = %e, "incoming connection handler exited");
                            }
                        });
                    }
                    Err(e) => {
                        warn!(error = %e, "tcp accept error");
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                }
            }
        });
        *self.listener_handle.lock().await = Some(listener_handle);

        // 心跳巡检 task
        let self_clone = Arc::clone(self);
        let heartbeat_handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
            loop {
                ticker.tick().await;
                self_clone.heartbeat_round().await;
            }
        });
        *self.heartbeat_handle.lock().await = Some(heartbeat_handle);

        Ok(incoming_rx)
    }

    /// 停止监听与心跳，关闭所有连接
    pub async fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.listener_handle.lock().await.take() {
            h.abort();
        }
        if let Some(h) = self.heartbeat_handle.lock().await.take() {
            h.abort();
        }
        self.close_all().await;
    }

    /// 关闭所有连接
    pub async fn close_all(&self) {
        let entries: Vec<(String, Channel)> = {
            let mut conns = self.conns.write().await;
            conns
                .drain()
                .map(|(id, e)| (id, e.channel))
                .collect()
        };
        for (id, ch) in entries {
            ch.close().await;
            self.publish_status(&id, DeviceStatus::Offline).await;
        }
    }

    /// 主动连接到对端（已配对设备）。成功后建立加密通道。
    pub async fn connect(self: &Arc<Self>, addr: SocketAddr) -> Result<Channel> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| CoreError::P2p(format!("connect {addr}: {e}")))?;
        Arc::clone(self).client_handshake(stream).await
    }

    /// 取已建立通道
    pub async fn get_channel(&self, device_id: &str) -> Option<Channel> {
        self.conns
            .read()
            .await
            .get(device_id)
            .map(|e| e.channel.clone())
    }

    /// 当前在线设备 id 列表
    pub async fn online_device_ids(&self) -> Vec<String> {
        self.conns.read().await.keys().cloned().collect()
    }

    /// 更新本机身份（信任库重载时调用）
    pub async fn set_identity(&self, identity: IdentityKey) {
        *self.identity.write().await = identity;
    }

    /// 更新本机 device_id
    pub async fn set_self_device_id(&self, id: String) {
        *self.self_device_id.write().await = id;
    }

    /// 信任库句柄（pairing 模块配对成功后写入新对端）
    pub fn trust_store(&self) -> TrustStore {
        self.trust.clone()
    }

    /// 当前 TCP 监听地址（`start` 后为实际绑定地址，`0` 端口由 OS 分配）。
    /// manager 据此读取端口并调 `Discovery::set_listen_port`，让广播携带可连接端口。
    pub async fn bind_addr(&self) -> Option<SocketAddr> {
        *self.bind_addr.read().await
    }

    // ── 内部：服务端握手 ────────────────────────────────────────────────

    async fn handle_incoming(
        self: Arc<Self>,
        mut stream: TcpStream,
        incoming_tx: mpsc::Sender<IncomingMessage>,
    ) -> Result<()> {
        let hello = read_plaintext_message(&mut stream).await?;
        let (client_device_id, client_eph_pub, client_sig, client_ts) = match hello {
            WireMessage::Hello {
                device_id,
                ephemeral_pub,
                signature,
                timestamp,
            } => (device_id, ephemeral_pub, signature, timestamp),
            _ => return Err(CoreError::P2p("expected Hello as first frame".to_string())),
        };

        if !ts_within_window(client_ts, now_ts()) {
            return Err(CoreError::P2p("hello timestamp out of window".to_string()));
        }

        let peer = self
            .trust
            .get_peer(&client_device_id)
            .await
            .ok_or_else(|| CoreError::P2p(format!("unknown peer: {client_device_id}")))?;
        let peer_pubkey = peer.pubkey_bytes()?;
        let signed = IdentityKey::handshake_signed_payload(&client_eph_pub, client_ts, &client_device_id);
        verify_signature(&peer_pubkey, &signed, &client_sig)
            .map_err(|e| CoreError::P2p(format!("hello signature invalid: {e}")))?;

        let eph = EphemeralKeypair::generate();
        let session_key = eph.derive_session_key(&client_eph_pub);
        let cipher = Arc::new(SessionCipher::new(&session_key));

        let self_identity = self.identity.read().await.clone();
        let self_device_id = self.self_device_id.read().await.clone();
        let ts = now_ts();
        let signed_ack =
            IdentityKey::handshake_signed_payload(&eph.public_bytes(), ts, &self_device_id);
        let ack = WireMessage::HelloAck {
            device_id: self_device_id,
            ephemeral_pub: eph.public_bytes(),
            signature: self_identity.sign(&signed_ack).to_vec(),
            timestamp: ts,
        };
        write_plaintext_message(&mut stream, &ack).await?;

        self.spawn_connection(
            client_device_id.clone(),
            stream,
            cipher,
            incoming_tx,
        )
        .await?;
        self.publish_status(&client_device_id, DeviceStatus::Paired).await;
        self.trust.touch_last_seen(&client_device_id, ts).await.ok();
        Ok(())
    }

    // ── 内部：客户端握手 ────────────────────────────────────────────────

    async fn client_handshake(self: Arc<Self>, mut stream: TcpStream) -> Result<Channel> {
        let eph = EphemeralKeypair::generate();
        let self_identity = self.identity.read().await.clone();
        let self_device_id = self.self_device_id.read().await.clone();
        let ts = now_ts();
        let signed = IdentityKey::handshake_signed_payload(&eph.public_bytes(), ts, &self_device_id);
        let hello = WireMessage::Hello {
            device_id: self_device_id,
            ephemeral_pub: eph.public_bytes(),
            signature: self_identity.sign(&signed).to_vec(),
            timestamp: ts,
        };
        write_plaintext_message(&mut stream, &hello).await?;

        let ack = read_plaintext_message(&mut stream).await?;
        let (server_device_id, server_eph_pub, server_sig, server_ts) = match ack {
            WireMessage::HelloAck {
                device_id,
                ephemeral_pub,
                signature,
                timestamp,
            } => (device_id, ephemeral_pub, signature, timestamp),
            _ => return Err(CoreError::P2p("expected HelloAck".to_string())),
        };

        if !ts_within_window(server_ts, now_ts()) {
            return Err(CoreError::P2p("hello_ack timestamp out of window".to_string()));
        }
        let peer = self
            .trust
            .get_peer(&server_device_id)
            .await
            .ok_or_else(|| CoreError::P2p(format!("unknown peer: {server_device_id}")))?;
        let peer_pubkey = peer.pubkey_bytes()?;
        let signed_ack =
            IdentityKey::handshake_signed_payload(&server_eph_pub, server_ts, &server_device_id);
        verify_signature(&peer_pubkey, &signed_ack, &server_sig)
            .map_err(|e| CoreError::P2p(format!("hello_ack signature invalid: {e}")))?;

        let session_key = eph.derive_session_key(&server_eph_pub);
        let cipher = Arc::new(SessionCipher::new(&session_key));

        // 客户端侧入站消息走主 incoming_tx（start 时回填）
        let incoming_tx = self
            .incoming_tx
            .read()
            .await
            .clone()
            .ok_or_else(|| CoreError::P2p("transport not started (no incoming_tx)".to_string()))?;

        self.spawn_connection(
            server_device_id.clone(),
            stream,
            cipher,
            incoming_tx,
        )
        .await?;
        self.publish_status(&server_device_id, DeviceStatus::Paired).await;
        self.trust.touch_last_seen(&server_device_id, ts).await.ok();

        self.get_channel(&server_device_id)
            .await
            .ok_or_else(|| CoreError::P2p("channel vanished right after handshake".to_string()))
    }

    /// 注册一条已握手连接：创建 channel + spawn reader/writer task
    async fn spawn_connection(
        self: &Arc<Self>,
        device_id: String,
        stream: TcpStream,
        cipher: Arc<SessionCipher>,
        incoming_tx: mpsc::Sender<IncomingMessage>,
    ) -> Result<()> {
        let (write_tx, mut write_rx) = mpsc::channel::<FrameOut>(64);
        let last_activity = Arc::new(AtomicU64::new(now_ms()));
        let channel = Channel {
            device_id: device_id.clone(),
            sender: write_tx,
        };
        let (cancel_tx, _cancel_rx) = tokio::sync::oneshot::channel::<()>();

        // 注册到连接表（若已存在旧连接，先关闭）
        {
            let mut conns = self.conns.write().await;
            if let Some(old) = conns.remove(&device_id) {
                let _ = old.cancel.send(());
            }
            conns.insert(
                device_id.clone(),
                ConnEntry {
                    channel: channel.clone(),
                    last_activity: Arc::clone(&last_activity),
                    cancel: cancel_tx,
                },
            );
        }

        let (read_half, write_half) = stream.into_split();

        // writer task
        let cipher_w = Arc::clone(&cipher);
        let dev_id_w = device_id.clone();
        tokio::spawn(async move {
            let mut writer = write_half;
            while let Some(frame) = write_rx.recv().await {
                match frame {
                    FrameOut::Message(msg) => {
                        let json = match serde_json::to_vec(&msg) {
                            Ok(j) => j,
                            Err(e) => {
                                warn!(error = %e, device = %dev_id_w, "serialize wire message failed");
                                continue;
                            }
                        };
                        let encrypted = match cipher_w.encrypt(&json) {
                            Ok(e) => e,
                            Err(e) => {
                                warn!(error = %e, device = %dev_id_w, "encrypt frame failed");
                                continue;
                            }
                        };
                        if let Err(e) = write_frame(&mut writer, &encrypted).await {
                            warn!(error = %e, device = %dev_id_w, "write frame failed; closing writer");
                            break;
                        }
                    }
                    FrameOut::Close => break,
                }
            }
            debug!(device = %dev_id_w, "writer task exited");
        });

        // reader task：持 Arc<Transport>，退出时清理连接表 + 发离线事件
        let cipher_r = Arc::clone(&cipher);
        let dev_id_r = device_id.clone();
        let transport = Arc::clone(self);
        tokio::spawn(async move {
            let mut reader = read_half;
            loop {
                let frame = match read_frame(&mut reader).await {
                    Ok(Some(f)) => f,
                    Ok(None) => {
                        debug!(device = %dev_id_r, "peer closed connection");
                        break;
                    }
                    Err(e) => {
                        warn!(error = %e, device = %dev_id_r, "read frame failed");
                        break;
                    }
                };
                // 收到任何帧即更新 last_activity（含 Pong）
                last_activity.store(now_ms(), Ordering::Relaxed);

                let plaintext = match cipher_r.decrypt(&frame) {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(error = %e, device = %dev_id_r, "decrypt frame failed");
                        continue;
                    }
                };
                let msg: WireMessage = match serde_json::from_slice(&plaintext) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(error = %e, device = %dev_id_r, "parse wire message failed");
                        continue;
                    }
                };
                // Pong 静默消费（last_activity 已更新）；其余推入 incoming
                if matches!(msg, WireMessage::Pong { .. }) {
                    continue;
                }
                if incoming_tx
                    .send(IncomingMessage {
                        device_id: dev_id_r.clone(),
                        message: msg,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
            debug!(device = %dev_id_r, "reader task exited");
            // reader 退出 → 清理连接表 + 发布离线
            transport.cleanup_conn(&dev_id_r).await;
        });

        Ok(())
    }

    /// 清理单条连接（reader 退出时调用）
    async fn cleanup_conn(&self, device_id: &str) {
        let removed = self.conns.write().await.remove(device_id).is_some();
        if removed {
            self.publish_status(device_id, DeviceStatus::Offline).await;
        }
    }

    /// 心跳巡检一轮：给所有在线连接发 Ping，并摘除超时连接
    async fn heartbeat_round(&self) {
        let now = now_ms();
        let offline_threshold = now.saturating_sub(OFFLINE_TIMEOUT.as_millis() as u64);

        let (to_ping, to_drop) = {
            let conns = self.conns.read().await;
            let mut ping: Vec<Channel> = Vec::with_capacity(conns.len());
            let mut drop_ids: Vec<String> = Vec::new();
            for (id, entry) in conns.iter() {
                let last = entry.last_activity.load(Ordering::Relaxed);
                if last < offline_threshold {
                    drop_ids.push(id.clone());
                } else {
                    ping.push(entry.channel.clone());
                }
            }
            (ping, drop_ids)
        };

        for ch in to_ping {
            let _ = ch.send(WireMessage::Ping { ts: now_ts() }).await;
        }
        for id in to_drop {
            // 关闭 channel（writer 退出）→ reader 也会因 EOF 退出 → cleanup_conn
            if let Some(entry) = self.conns.read().await.get(&id) {
                entry.channel.close().await;
            }
            // 兜底直接清理（防止 reader 阻塞未退出）
            self.cleanup_conn(&id).await;
        }
    }

    /// 发布设备状态变更事件
    async fn publish_status(&self, device_id: &str, status: DeviceStatus) {
        self.event_bus.publish(BusEvent::DeviceStatusChanged {
            device_id: device_id.to_string(),
            status,
        });
    }
}

// ── 帧读写工具（4 字节大端长度前缀） ────────────────────────────────────

async fn write_frame<W: AsyncWriteExt + Unpin>(w: &mut W, payload: &[u8]) -> Result<()> {
    if payload.len() > MAX_FRAME_LEN {
        return Err(CoreError::P2p(format!(
            "frame too large: {} > {MAX_FRAME_LEN}",
            payload.len()
        )));
    }
    let len = (payload.len() as u32).to_be_bytes();
    w.write_all(&len).await.map_err(CoreError::Io)?;
    w.write_all(payload).await.map_err(CoreError::Io)?;
    w.flush().await.map_err(CoreError::Io)?;
    Ok(())
}

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
    if len > MAX_FRAME_LEN {
        return Err(CoreError::P2p(format!("frame too large: {len} > {MAX_FRAME_LEN}")));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await.map_err(CoreError::Io)?;
    Ok(Some(buf))
}

async fn write_plaintext_message<W: AsyncWriteExt + Unpin>(
    w: &mut W,
    msg: &WireMessage,
) -> Result<()> {
    let json = serde_json::to_vec(msg)?;
    write_frame(w, &json).await
}

async fn read_plaintext_message<R: AsyncReadExt + Unpin>(r: &mut R) -> Result<WireMessage> {
    let frame = read_frame(r)
        .await?
        .ok_or_else(|| CoreError::P2p("eof during handshake".to_string()))?;
    serde_json::from_slice(&frame).map_err(CoreError::Serde)
}

/// 当前 Unix 毫秒时间戳
#[inline]
fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── 单元测试 ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    fn temp_dir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("effisuite-p2p-test-{}", uuid::Uuid::new_v4()));
        // 创建目录（TrustStore::load_or_create 写文件时需要父目录存在）
        let _ = std::fs::create_dir_all(&p);
        p
    }

    /// 两条可信身份互相注入对方公钥，构造可互通的信任库对
    async fn make_trusted_pair() -> (TrustStore, TrustStore, String, String) {
        let id_a = IdentityKey::generate();
        let id_b = IdentityKey::generate();
        let dev_a = format!("dev-{}", &crate::crypto::hex_encode(&id_a.public_bytes())[..8]);
        let dev_b = format!("dev-{}", &crate::crypto::hex_encode(&id_b.public_bytes())[..8]);

        let path_a = temp_dir().join("trust_a.json");
        let path_b = temp_dir().join("trust_b.json");

        let file_a = serde_json::json!({
            "self_device_id": dev_a,
            "self_seed_hex": crate::crypto::hex_encode(&id_a.to_seed()),
            "peers": { dev_b.clone(): {
                "device_id": dev_b, "name": "B",
                "pubkey_hex": crate::crypto::hex_encode(&id_b.public_bytes()),
                "address": "127.0.0.1:0", "paired_at": 0, "last_seen": 0, "role": "mirror"
            }}
        });
        std::fs::write(&path_a, file_a.to_string()).unwrap();
        let store_a = TrustStore::load_or_create(path_a).await.unwrap();

        let file_b = serde_json::json!({
            "self_device_id": dev_b,
            "self_seed_hex": crate::crypto::hex_encode(&id_b.to_seed()),
            "peers": { dev_a.clone(): {
                "device_id": dev_a, "name": "A",
                "pubkey_hex": crate::crypto::hex_encode(&id_a.public_bytes()),
                "address": "127.0.0.1:0", "paired_at": 0, "last_seen": 0, "role": "mirror"
            }}
        });
        std::fs::write(&path_b, file_b.to_string()).unwrap();
        let store_b = TrustStore::load_or_create(path_b).await.unwrap();

        (store_a, store_b, dev_a, dev_b)
    }

    #[tokio::test]
    async fn transport_full_handshake_and_roundtrip() {
        let (store_a, store_b, dev_a, dev_b) = make_trusted_pair().await;
        let bus = EventBus::new(32);
        let identity_a = store_a.self_identity().await.unwrap();
        let identity_b = store_b.self_identity().await.unwrap();

        let transport_b = Arc::new(Transport::from_trust(store_b, identity_b, bus.clone()).await);
        let mut incoming_rx = transport_b
            .start("127.0.0.1:0".parse::<SocketAddr>().unwrap())
            .await
            .unwrap();
        let port_b = transport_b.bind_addr.read().await.unwrap().port();

        let transport_a = Arc::new(Transport::from_trust(store_a, identity_a, bus.clone()).await);
        // 保留 incoming_rx：drop 会导致 incoming_tx.send 失败 → reader task 退出 → cleanup_conn 移除 channel
        let mut _incoming_rx_a = transport_a
            .start("127.0.0.1:0".parse::<SocketAddr>().unwrap())
            .await
            .unwrap();

        let channel = transport_a
            .connect(SocketAddr::from(([127, 0, 0, 1], port_b)))
            .await
            .expect("client handshake should succeed");

        channel
            .send(WireMessage::Ping { ts: now_ts() })
            .await
            .unwrap();

        let msg = tokio::time::timeout(Duration::from_secs(3), incoming_rx.recv())
            .await
            .expect("timed out waiting for incoming")
            .expect("incoming channel closed");
        assert_eq!(msg.device_id, dev_a);
        assert!(matches!(msg.message, WireMessage::Ping { .. }));

        assert!(transport_b.online_device_ids().await.contains(&dev_a));
        assert!(transport_a.online_device_ids().await.contains(&dev_b));

        transport_a.stop().await;
        transport_b.stop().await;
    }

    #[tokio::test]
    async fn transport_rejects_unknown_peer() {
        let path_a = temp_dir().join("trust_solo.json");
        let store_a = TrustStore::load_or_create(path_a).await.unwrap();
        let identity_a = store_a.self_identity().await.unwrap();
        let dev_a = store_a.self_device_id().await;

        let id_b = IdentityKey::generate();
        let dev_b = format!("dev-{}", &crate::crypto::hex_encode(&id_b.public_bytes())[..8]);
        let path_b = temp_dir().join("trust_b_solo.json");
        let file_b = serde_json::json!({
            "self_device_id": dev_b,
            "self_seed_hex": crate::crypto::hex_encode(&id_b.to_seed()),
            "peers": { dev_a.clone(): {
                "device_id": dev_a, "name": "A",
                "pubkey_hex": crate::crypto::hex_encode(&identity_a.public_bytes()),
                "address": "127.0.0.1:0", "paired_at": 0, "last_seen": 0, "role": "mirror"
            }}
        });
        std::fs::write(&path_b, file_b.to_string()).unwrap();
        let store_b = TrustStore::load_or_create(path_b).await.unwrap();

        let bus = EventBus::new(8);
        let transport_a = Arc::new(Transport::from_trust(store_a, identity_a, bus.clone()).await);
        transport_a
            .start("127.0.0.1:0".parse::<SocketAddr>().unwrap())
            .await
            .unwrap();
        let port_a = transport_a.bind_addr.read().await.unwrap().port();

        let identity_b = store_b.self_identity().await.unwrap();
        let transport_b = Arc::new(Transport::from_trust(store_b, identity_b, bus.clone()).await);
        transport_b
            .start("127.0.0.1:0".parse::<SocketAddr>().unwrap())
            .await
            .unwrap();

        let err = transport_b
            .connect(SocketAddr::from(([127, 0, 0, 1], port_a)))
            .await;
        assert!(err.is_err(), "unknown peer should be rejected");

        transport_a.stop().await;
        transport_b.stop().await;
    }

    #[tokio::test]
    async fn read_frame_handles_eof() {
        use tokio::io::duplex;
        let (mut _client, mut server) = duplex(64);
        drop(_client);
        let res = read_frame(&mut server).await.unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn frame_constants_sane() {
        assert!(MAX_FRAME_LEN > 1024 * 1024);
        assert!(HEARTBEAT_INTERVAL < OFFLINE_TIMEOUT);
        assert!(TASK_TIMEOUT > Duration::from_secs(10));
    }
}

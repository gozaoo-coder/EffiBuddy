//! 镜像同步：按时间顺序同步会话/插件/用户缓存。
//!
//! # 职责
//! - **拉取**（[`Sync::pull`]）：向对端发送 `SyncRequest`，按时间戳增量拉取
//!   会话清单 → 逐会话 `SyncFetch` → 消息批次 → 合并写入本地 store。
//! - **推送**（[`Sync::push`]）：读取本地 store 中同步游标之后的新增数据，
//!   主动发送 `SyncMessages` / `SyncData` 到对端（对端直接落盘）。
//! - **服务端响应**（[`Sync::handle_request`] / [`Sync::handle_fetch`]）：
//!   响应远端拉取请求，返回清单 / 数据块 / 消息批次。
//! - **入站路由**（[`Sync::handle_incoming`]）：manager 把 transport 收到的
//!   同步消息路由到此；响应消息优先转发给本机 pull 等待者，无等待者视为
//!   对端主动推送 → 直接合并落盘。
//!
//! # 数据源抽象
//! 通过 [`SyncDataStore`] trait 依赖倒置，sync 不直接依赖具体 store 实现，
//! 由业务层（tauri）注入 `Arc<dyn SyncDataStore>`。
//!
//! # 并发模型
//! - `pending_pull`：`RwLock<HashMap<device_id, mpsc::UnboundedSender>>`，
//!   同一设备同时只允许一个 pull（互斥），响应经 channel 一次性送达等待者。
//! - 临界区仅 HashMap 读写，事件发送在锁外；`sync_cursor` 读锁零 IO。
//! - 响应等待用 `tokio::time::timeout`（30s）包裹，超时返回错误不悬挂。
//!
//! # 内存
//! - 字段按大小降序排列；`HashMap` 用 `with_capacity` 预分配。
//! - 帧收发经 `Transport::Channel`（mpsc 队列），不在锁内做序列化。

use std::sync::Arc;

use async_trait::async_trait;
use effisuite_core::{
    remote_task_now, CoreError, InstalledPlugin, Message, PinnedMemory, Result,
};
use tokio::sync::{mpsc, RwLock};
use tokio::time::{timeout, Duration};
use tracing::{debug, info};

use crate::protocol::{ConvManifestEntry, SyncKind, SyncManifest, WireMessage};
use crate::transport::Transport;

/// 同步响应超时（清单 / 数据块 / 消息批次，单次等待）
const SYNC_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
/// 同步数据块 JSON payload 上限（防恶意大帧 OOM，16 MiB）
const MAX_SYNC_PAYLOAD: usize = 16 * 1024 * 1024;
/// 会话清单预估容量（局域网设备通常 < 32 会话）
const MANIFEST_CAPACITY: usize = 16;

/// 镜像同步数据源（manager 注入，sync 不直接依赖具体 store 实现）。
#[async_trait]
pub trait SyncDataStore: Send + ::std::marker::Sync {
    /// 会话清单（轻量，不含消息体），按 updated_at 升序。
    async fn list_conversations(&self) -> Result<Vec<ConvManifestEntry>>;
    /// 拉取指定会话 `since`（Unix 秒）之后的消息，按 timestamp 升序。
    async fn get_messages_since(&self, conv_id: &str, since: u64) -> Result<Vec<Message>>;
    /// 合并写入远端消息：按消息 id 去重，保留时间戳升序。
    async fn upsert_messages(&self, conv_id: &str, messages: &[Message]) -> Result<()>;
    /// 已安装插件清单。
    async fn list_plugins(&self) -> Result<Vec<InstalledPlugin>>;
    /// 合并写入远端插件：按 id 去重，已存在则覆盖。
    async fn upsert_plugins(&self, plugins: &[InstalledPlugin]) -> Result<()>;
    /// 永久记忆清单。
    async fn list_pinned(&self) -> Result<Vec<PinnedMemory>>;
    /// 合并写入远端永久记忆：按 id 去重，已存在则跳过。
    async fn upsert_pinned(&self, memories: &[PinnedMemory]) -> Result<()>;
    /// 与指定设备的同步进度游标（Unix 秒，0 = 未同步过 → 全量）。
    async fn cursor(&self, device_id: &str) -> u64;
    /// 更新与指定设备的同步进度游标。
    async fn set_cursor(&self, device_id: &str, ts: u64) -> Result<()>;
}

/// 镜像同步器。
///
/// 字段按大小降序：`Arc<dyn SyncDataStore>` 集群 → `RwLock<HashMap>` →
/// `Arc<Transport>`（1 usize）。
pub struct Sync {
    /// 数据源（业务层注入，未注入时同步操作返回错误）
    data_store: Arc<RwLock<Option<Arc<dyn SyncDataStore>>>>,
    /// pull 等待者表：device_id → 响应转发 channel（同设备互斥）
    pending_pull: RwLock<std::collections::HashMap<String, mpsc::UnboundedSender<WireMessage>>>,
    /// 加密 TCP 传输层
    transport: Arc<Transport>,
}

impl Sync {
    /// 构造镜像同步器（数据源通过 [`Sync::set_data_store`] 注入）。
    pub fn new(transport: Arc<Transport>) -> Self {
        Self {
            data_store: Arc::new(RwLock::new(None)),
            pending_pull: RwLock::new(std::collections::HashMap::with_capacity(4)),
            transport,
        }
    }

    /// 注入数据源（manager 启动时由业务层调用，幂等可覆盖）。
    pub async fn set_data_store(&self, store: Arc<dyn SyncDataStore>) {
        *self.data_store.write().await = Some(store);
    }

    /// 与指定设备的同步进度游标（未启动 / 未注入时返回 0）。
    pub async fn sync_cursor(&self, device_id: &str) -> u64 {
        match self.data_store().await {
            Ok(store) => store.cursor(device_id).await,
            Err(_) => 0,
        }
    }

    /// 镜像拉取：向对端请求 `since` 之后指定种类的数据并合并写入本地。
    ///
    /// 流程：发 `SyncRequest` → 收清单 + 数据块 → 逐会话 `SyncFetch` →
    /// 合并写入 → 更新同步游标。同一设备同时只允许一个 pull。
    pub async fn pull(&self, device_id: &str, since: u64, kinds: &[SyncKind]) -> Result<()> {
        self.data_store().await?;
        self.channel_for(device_id).await?;

        // 注册等待者 channel（同设备互斥）
        let (tx, mut rx) = mpsc::unbounded_channel();
        {
            let mut pending = self.pending_pull.write().await;
            if pending.contains_key(device_id) {
                return Err(CoreError::P2p(format!(
                    "sync with {device_id} already in progress"
                )));
            }
            pending.insert(device_id.to_string(), tx);
        }

        let result = self.pull_inner(device_id, since, kinds, &mut rx).await;
        self.pending_pull.write().await.remove(device_id);
        result
    }

    /// 实际拉取逻辑（pending 注册后执行，退出由 `pull` 清理）。
    async fn pull_inner(
        &self,
        device_id: &str,
        since: u64,
        kinds: &[SyncKind],
        rx: &mut mpsc::UnboundedReceiver<WireMessage>,
    ) -> Result<()> {
        let store = self.data_store().await?;
        let channel = self.channel_for(device_id).await?;
        channel
            .send(WireMessage::SyncRequest {
                since,
                kinds: kinds.to_vec(),
            })
            .await?;

        let want_manifest = kinds.contains(&SyncKind::Conversations);
        // 期望接收的非会话数据块数（插件 / 用户缓存各 1 块）
        let expect_data = kinds
            .iter()
            .filter(|k| **k != SyncKind::Conversations)
            .count();

        // 1. 等待清单 + 非会话数据块（对端按 kinds 顺序发送，发起方精确匹配直到收齐）
        let (entries, data_blocks) = timeout(SYNC_RESPONSE_TIMEOUT, async {
            let mut entries: Vec<ConvManifestEntry> = Vec::with_capacity(MANIFEST_CAPACITY);
            let mut data_blocks: Vec<(SyncKind, String)> = Vec::with_capacity(expect_data);
            let mut manifest_received = !want_manifest;
            while !manifest_received || data_blocks.len() < expect_data {
                match rx.recv().await {
                    Some(WireMessage::SyncManifest { manifest }) => {
                        entries = manifest.entries;
                        manifest_received = true;
                    }
                    Some(WireMessage::SyncData { kind, payload }) => {
                        data_blocks.push((kind, payload));
                    }
                    Some(_) => continue,
                    None => {
                        return Err(CoreError::P2p("sync channel closed".to_string()));
                    }
                }
            }
            Ok((entries, data_blocks))
        })
        .await
        .map_err(|_| CoreError::P2p("sync manifest/data timeout".to_string()))??;

        // 2. 逐会话拉取消息（对端按请求顺序回 SyncMessages）
        for entry in &entries {
            channel
                .send(WireMessage::SyncFetch {
                    conversation_id: entry.id.clone(),
                    since_msg_ts: since,
                })
                .await?;
            let msg = timeout(
                SYNC_RESPONSE_TIMEOUT,
                recv_until(rx, |m| match m {
                    WireMessage::SyncMessages {
                        conversation_id,
                        messages,
                    } if conversation_id == &entry.id && !messages.is_empty() => true,
                    WireMessage::SyncMessages { .. } => true,
                    _ => false,
                }),
            )
            .await
            .map_err(|_| CoreError::P2p(format!("sync messages timeout for {}", entry.id)))??;
            if let WireMessage::SyncMessages { messages, .. } = msg {
                store.upsert_messages(&entry.id, &messages).await?;
                debug!(conversation_id = %entry.id, count = messages.len(), "messages applied");
            }
        }

        // 3. 数据块落盘（插件 / 用户缓存）
        for (kind, payload) in data_blocks {
            if payload.len() > MAX_SYNC_PAYLOAD {
                return Err(CoreError::P2p("sync payload exceeds limit".to_string()));
            }
            match kind {
                SyncKind::Plugins => {
                    let plugins: Vec<InstalledPlugin> = serde_json::from_str(&payload)?;
                    store.upsert_plugins(&plugins).await?;
                    info!(device_id, count = plugins.len(), "plugins synced");
                }
                SyncKind::UserCache => {
                    let memories: Vec<PinnedMemory> = serde_json::from_str(&payload)?;
                    store.upsert_pinned(&memories).await?;
                    info!(device_id, count = memories.len(), "user cache synced");
                }
                SyncKind::Conversations => {}
            }
        }

        // 4. 更新同步游标
        store.set_cursor(device_id, remote_task_now()).await?;
        info!(device_id, conversations = entries.len(), "pull completed");
        Ok(())
    }

    /// 镜像推送：把本地同步游标之后的新增数据主动发送到对端。
    ///
    /// 对端收到后直接合并落盘（见 [`Sync::handle_remote_push`]）。
    pub async fn push(&self, device_id: &str, kinds: &[SyncKind]) -> Result<()> {
        let store = self.data_store().await?;
        let channel = self.channel_for(device_id).await?;
        let since = store.cursor(device_id).await;
        let now = remote_task_now();

        for kind in kinds {
            match kind {
                SyncKind::Conversations => {
                    let entries = store.list_conversations().await?;
                    let mut sent = 0usize;
                    for entry in entries.iter().filter(|e| e.updated_at > since) {
                        let messages = store.get_messages_since(&entry.id, since).await?;
                        if messages.is_empty() {
                            continue;
                        }
                        channel
                            .send(WireMessage::SyncMessages {
                                conversation_id: entry.id.clone(),
                                messages,
                            })
                            .await?;
                        sent += 1;
                    }
                    if sent > 0 {
                        info!(device_id, conversations = sent, "pushed conversations");
                    }
                }
                SyncKind::Plugins => {
                    let plugins = store.list_plugins().await?;
                    let payload = serde_json::to_string(&plugins)?;
                    channel
                        .send(WireMessage::SyncData {
                            kind: SyncKind::Plugins,
                            payload,
                        })
                        .await?;
                }
                SyncKind::UserCache => {
                    let memories = store.list_pinned().await?;
                    let payload = serde_json::to_string(&memories)?;
                    channel
                        .send(WireMessage::SyncData {
                            kind: SyncKind::UserCache,
                            payload,
                        })
                        .await?;
                }
            }
        }

        store.set_cursor(device_id, now).await?;
        info!(device_id, "push completed");
        Ok(())
    }

    /// 处理远端 SyncRequest：按种类返回清单 / 数据块。
    pub async fn handle_request(
        &self,
        device_id: &str,
        since: u64,
        kinds: &[SyncKind],
    ) -> Result<()> {
        let store = self.data_store().await?;
        let channel = self.channel_for(device_id).await?;
        for kind in kinds {
            match kind {
                SyncKind::Conversations => {
                    let entries = store.list_conversations().await?;
                    let manifest = SyncManifest { entries };
                    channel
                        .send(WireMessage::SyncManifest { manifest })
                        .await?;
                }
                SyncKind::Plugins => {
                    let plugins = store.list_plugins().await?;
                    let payload = serde_json::to_string(&plugins)?;
                    channel
                        .send(WireMessage::SyncData {
                            kind: SyncKind::Plugins,
                            payload,
                        })
                        .await?;
                }
                SyncKind::UserCache => {
                    let memories = store.list_pinned().await?;
                    let payload = serde_json::to_string(&memories)?;
                    channel
                        .send(WireMessage::SyncData {
                            kind: SyncKind::UserCache,
                            payload,
                        })
                        .await?;
                }
            }
        }
        debug!(device_id, since, kinds = ?kinds, "sync request served");
        Ok(())
    }

    /// 处理远端 SyncFetch：返回指定会话 since 之后的消息。
    pub async fn handle_fetch(
        &self,
        device_id: &str,
        conversation_id: &str,
        since_msg_ts: u64,
    ) -> Result<()> {
        let store = self.data_store().await?;
        let channel = self.channel_for(device_id).await?;
        let messages = store
            .get_messages_since(conversation_id, since_msg_ts)
            .await?;
        channel
            .send(WireMessage::SyncMessages {
                conversation_id: conversation_id.to_string(),
                messages,
            })
            .await?;
        Ok(())
    }

    /// 处理入站同步消息（manager 路由调用）。
    ///
    /// - `SyncRequest` / `SyncFetch` → 服务端响应
    /// - `SyncManifest` / `SyncMessages` / `SyncData` → 优先转发给本机 pull
    ///   等待者；无等待者视为对端主动推送 → 直接合并落盘
    pub async fn handle_incoming(&self, device_id: &str, msg: WireMessage) -> Result<()> {
        match msg {
            WireMessage::SyncRequest { since, kinds } => {
                self.handle_request(device_id, since, &kinds).await
            }
            WireMessage::SyncFetch {
                conversation_id,
                since_msg_ts,
            } => self
                .handle_fetch(device_id, &conversation_id, since_msg_ts)
                .await,
            WireMessage::SyncManifest { .. }
            | WireMessage::SyncMessages { .. }
            | WireMessage::SyncData { .. } => {
                // 转发给等待中的 pull 任务；无等待者视为对端主动推送 → 直接落盘
                let forwarded = {
                    let pending = self.pending_pull.read().await;
                    match pending.get(device_id) {
                        Some(tx) => tx.send(msg.clone()).is_ok(),
                        None => false,
                    }
                };
                if forwarded {
                    Ok(())
                } else {
                    self.handle_remote_push(device_id, msg).await
                }
            }
            _ => Ok(()),
        }
    }

    /// 处理对端主动推送：直接合并写入本地 store。
    async fn handle_remote_push(&self, device_id: &str, msg: WireMessage) -> Result<()> {
        let store = self.data_store().await?;
        match msg {
            WireMessage::SyncMessages {
                conversation_id,
                messages,
            } => {
                store.upsert_messages(&conversation_id, &messages).await?;
                info!(
                    device_id,
                    conversation_id, count = messages.len(),
                    "remote push applied"
                );
            }
            WireMessage::SyncData { kind, payload } => {
                if payload.len() > MAX_SYNC_PAYLOAD {
                    return Err(CoreError::P2p("sync payload exceeds limit".to_string()));
                }
                match kind {
                    SyncKind::Plugins => {
                        let plugins: Vec<InstalledPlugin> = serde_json::from_str(&payload)?;
                        store.upsert_plugins(&plugins).await?;
                        info!(device_id, count = plugins.len(), "remote plugins applied");
                    }
                    SyncKind::UserCache => {
                        let memories: Vec<PinnedMemory> = serde_json::from_str(&payload)?;
                        store.upsert_pinned(&memories).await?;
                        info!(device_id, count = memories.len(), "remote user cache applied");
                    }
                    SyncKind::Conversations => {}
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// 停止同步器：清空等待者表与数据源。
    pub async fn stop(&self) -> Result<()> {
        self.pending_pull.write().await.clear();
        *self.data_store.write().await = None;
        Ok(())
    }

    /// 取数据源（未注入时报错）。
    async fn data_store(&self) -> Result<Arc<dyn SyncDataStore>> {
        self.data_store
            .read()
            .await
            .clone()
            .ok_or_else(|| CoreError::P2p("sync data store not injected".to_string()))
    }

    /// 取对端加密通道（离线时返回错误）。
    async fn channel_for(&self, device_id: &str) -> Result<crate::transport::Channel> {
        self.transport
            .get_channel(device_id)
            .await
            .ok_or_else(|| CoreError::P2p(format!("no connection to {device_id}")))
    }
}

/// 从 channel 接收消息直到满足谓词（EOF 返回错误）。
async fn recv_until<F>(
    rx: &mut mpsc::UnboundedReceiver<WireMessage>,
    mut pred: F,
) -> Result<WireMessage>
where
    F: FnMut(&WireMessage) -> bool,
{
    loop {
        match rx.recv().await {
            Some(msg) if pred(&msg) => return Ok(msg),
            Some(_) => continue,
            None => return Err(CoreError::P2p("sync channel closed".to_string())),
        }
    }
}

// ── 单元测试 ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use effisuite_core::{EventBus, Role};
    use std::collections::{HashMap, HashSet};

    /// 内存 mock 数据源：模拟会话/插件/永久记忆存储 + 游标。
    /// 内部用 `Mutex` 提供内部可变性（trait 方法仅 &self），锁内零 await。
    #[derive(Default)]
    struct MockStore {
        inner: std::sync::Mutex<MockInner>,
    }

    #[derive(Default)]
    struct MockInner {
        conversations: HashMap<String, Vec<Message>>,
        plugins: Vec<InstalledPlugin>,
        pinned: Vec<PinnedMemory>,
        cursors: HashMap<String, u64>,
    }

    #[async_trait]
    impl SyncDataStore for MockStore {
        async fn list_conversations(&self) -> Result<Vec<ConvManifestEntry>> {
            let inner = self.inner.lock().unwrap();
            Ok(inner
                .conversations
                .iter()
                .map(|(id, msgs)| ConvManifestEntry {
                    id: id.clone(),
                    title: None,
                    updated_at: msgs.last().map(|m| m.timestamp).unwrap_or(0),
                    message_count: msgs.len(),
                })
                .collect())
        }

        async fn get_messages_since(&self, conv_id: &str, since: u64) -> Result<Vec<Message>> {
            let inner = self.inner.lock().unwrap();
            Ok(inner
                .conversations
                .get(conv_id)
                .map(|msgs| {
                    msgs.iter()
                        .filter(|m| m.timestamp > since)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default())
        }

        async fn upsert_messages(&self, conv_id: &str, messages: &[Message]) -> Result<()> {
            let mut inner = self.inner.lock().unwrap();
            let list = inner.conversations.entry(conv_id.to_string()).or_default();
            let existing: HashSet<String> = list.iter().map(|m| m.id.clone()).collect();
            for m in messages {
                if !existing.contains(&m.id) {
                    list.push(m.clone());
                }
            }
            Ok(())
        }

        async fn list_plugins(&self) -> Result<Vec<InstalledPlugin>> {
            Ok(self.inner.lock().unwrap().plugins.clone())
        }

        async fn upsert_plugins(&self, plugins: &[InstalledPlugin]) -> Result<()> {
            let mut inner = self.inner.lock().unwrap();
            let existing: HashSet<String> = inner.plugins.iter().map(|p| p.id.clone()).collect();
            for p in plugins {
                if !existing.contains(&p.id) {
                    inner.plugins.push(p.clone());
                }
            }
            Ok(())
        }

        async fn list_pinned(&self) -> Result<Vec<PinnedMemory>> {
            Ok(self.inner.lock().unwrap().pinned.clone())
        }

        async fn upsert_pinned(&self, memories: &[PinnedMemory]) -> Result<()> {
            let mut inner = self.inner.lock().unwrap();
            let existing: HashSet<String> = inner.pinned.iter().map(|m| m.id.clone()).collect();
            for m in memories {
                if !existing.contains(&m.id) {
                    inner.pinned.push(m.clone());
                }
            }
            Ok(())
        }

        async fn cursor(&self, device_id: &str) -> u64 {
            self.inner.lock().unwrap().cursors.get(device_id).copied().unwrap_or(0)
        }

        async fn set_cursor(&self, device_id: &str, ts: u64) -> Result<()> {
            self.inner
                .lock()
                .unwrap()
                .cursors
                .insert(device_id.to_string(), ts);
            Ok(())
        }
    }

    /// 构造两个已互相信任的 transport（返回各自的入站消息接收器，供路由 task 消费）。
    #[allow(clippy::type_complexity)]
    async fn pair_transports(
    ) -> (
        Arc<Transport>,
        tokio::sync::mpsc::Receiver<crate::transport::IncomingMessage>,
        Arc<Transport>,
        tokio::sync::mpsc::Receiver<crate::transport::IncomingMessage>,
    ) {
        use crate::transport::Transport;
        use crate::trust::TrustStore;

        let dir_a = std::env::temp_dir().join(format!("sync-a-{}", uuid::Uuid::new_v4()));
        let dir_b = std::env::temp_dir().join(format!("sync-b-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();

        let trust_a = TrustStore::load_or_create(dir_a.join("trust.json"))
            .await
            .unwrap();
        let trust_b = TrustStore::load_or_create(dir_b.join("trust.json"))
            .await
            .unwrap();
        let id_a = trust_a.self_identity().await.unwrap();
        let id_b = trust_b.self_identity().await.unwrap();
        let dev_a = trust_a.self_device_id().await;
        let dev_b = trust_b.self_device_id().await;

        // 互相信任：交换公钥
        trust_a
            .upsert_peer(crate::trust::TrustedPeer {
                device_id: dev_b.clone(),
                name: "B".to_string(),
                pubkey_hex: crate::crypto::hex_encode(&id_b.public_bytes()),
                address: "127.0.0.1:0".to_string(),
                paired_at: 1,
                last_seen: 1,
                role: crate::trust::PairRole::Mirror,
            })
            .await
            .unwrap();
        trust_b
            .upsert_peer(crate::trust::TrustedPeer {
                device_id: dev_a.clone(),
                name: "A".to_string(),
                pubkey_hex: crate::crypto::hex_encode(&id_a.public_bytes()),
                address: "127.0.0.1:0".to_string(),
                paired_at: 1,
                last_seen: 1,
                role: crate::trust::PairRole::Mirror,
            })
            .await
            .unwrap();

        let ta = Arc::new(Transport::new(
            trust_a.clone(),
            id_a.clone(),
            EventBus::new(8),
            dev_a,
        ));
        let tb = Arc::new(Transport::new(
            trust_b.clone(),
            id_b.clone(),
            EventBus::new(8),
            dev_b,
        ));
        let rx_a = ta.start("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let rx_b = tb.start("127.0.0.1:0".parse().unwrap()).await.unwrap();

        let addr_a = ta.bind_addr().await.unwrap();
        let _ch = tb.connect(addr_a).await.unwrap();
        // 等待双向连接建立（握手 + 连接表注册）
        tokio::time::sleep(Duration::from_millis(300)).await;
        (ta, rx_a, tb, rx_b)
    }

    /// 构造带路由的同步对：入站消息 → sync.handle_incoming（模拟 manager 路由）。
    fn sample_plugin(id: &str) -> InstalledPlugin {
        InstalledPlugin {
            id: id.to_string(),
            name: id.to_string(),
            display_name: id.to_uppercase(),
            summary: String::new(),
            family: String::new(),
            channel: String::new(),
            owner_handle: String::new(),
            version: String::new(),
            install_path: None,
            installed_at: 1,
        }
    }

    fn sample_pinned(id: &str) -> PinnedMemory {
        PinnedMemory {
            id: id.to_string(),
            content: format!("remember {id}"),
            category: None,
            source_conversation_id: None,
            created_at: 1,
            source: effisuite_core::PinnedMemorySource::Manual,
        }
    }

    #[tokio::test]
    async fn sync_pull_transfers_conversations_and_data() {
        let (ta, rx_a, tb, rx_b) = pair_transports().await;
        let dev_b = ta.online_device_ids().await.pop().unwrap();

        let store_a = Arc::new(MockStore::default());
        let store_b = Arc::new(MockStore::default());
        let sync_a = Arc::new(Sync::new(Arc::clone(&ta)));
        let sync_b = Arc::new(Sync::new(Arc::clone(&tb)));
        sync_a
            .set_data_store(Arc::clone(&store_a) as Arc<dyn SyncDataStore>)
            .await;
        sync_b
            .set_data_store(Arc::clone(&store_b) as Arc<dyn SyncDataStore>)
            .await;

        // 对端 B 有数据：1 会话 + 1 插件 + 1 永久记忆
        {
            let mut inner = store_b.inner.lock().unwrap();
            inner.conversations.insert(
                "conv-b1".to_string(),
                vec![
                    Message::new("m1", Role::User, "hi from B", 100),
                    Message::new("m2", Role::Assistant, "hello A", 200),
                ],
            );
            inner.plugins.push(sample_plugin("p1"));
            inner.pinned.push(sample_pinned("mem1"));
        }

        // 路由 task：transport 入站消息 → sync.handle_incoming（模拟 manager）
        let sync_a_route = Arc::clone(&sync_a);
        let route_a = tokio::spawn(async move {
            let mut rx = rx_a;
            while let Some(incoming) = rx.recv().await {
                let _ = sync_a_route
                    .handle_incoming(&incoming.device_id, incoming.message)
                    .await;
            }
        });
        let sync_b_route = Arc::clone(&sync_b);
        let route_b = tokio::spawn(async move {
            let mut rx = rx_b;
            while let Some(incoming) = rx.recv().await {
                let _ = sync_b_route
                    .handle_incoming(&incoming.device_id, incoming.message)
                    .await;
            }
        });

        let kinds = [
            SyncKind::Conversations,
            SyncKind::Plugins,
            SyncKind::UserCache,
        ];
        sync_a
            .pull(&dev_b, 0, &kinds)
            .await
            .expect("pull should succeed");

        // 验证 A 的 store 合并了 B 的数据
        {
            let inner = store_a.inner.lock().unwrap();
            let msgs = inner.conversations.get("conv-b1").unwrap();
            assert_eq!(msgs.len(), 2, "两条消息都应同步");
            assert_eq!(msgs[0].content, "hi from B");
            assert_eq!(msgs[1].content, "hello A");
            assert_eq!(inner.plugins.len(), 1);
            assert_eq!(inner.pinned.len(), 1);
            assert!(
                inner.cursors.contains_key(&dev_b),
                "同步后应更新游标"
            );
        }

        route_a.abort();
        route_b.abort();
    }

    #[tokio::test]
    async fn sync_push_delivers_remote_data() {
        let (ta, rx_a, tb, rx_b) = pair_transports().await;
        // 本机 A 向对端 B 推送：取 A 视角下在线对端（B）的 id
        let dev_b = ta.online_device_ids().await.pop().unwrap();

        let store_a = Arc::new(MockStore::default());
        let store_b = Arc::new(MockStore::default());
        let sync_a = Arc::new(Sync::new(Arc::clone(&ta)));
        let sync_b = Arc::new(Sync::new(Arc::clone(&tb)));
        sync_a
            .set_data_store(Arc::clone(&store_a) as Arc<dyn SyncDataStore>)
            .await;
        sync_b
            .set_data_store(Arc::clone(&store_b) as Arc<dyn SyncDataStore>)
            .await;

        // 本机 A 有新增数据
        {
            let mut inner = store_a.inner.lock().unwrap();
            inner.conversations.insert(
                "conv-a1".to_string(),
                vec![Message::new("m1", Role::User, "from A", 100)],
            );
            inner.plugins.push(sample_plugin("pa"));
            inner.pinned.push(sample_pinned("ma"));
        }

        let sync_a_route = Arc::clone(&sync_a);
        let route_a = tokio::spawn(async move {
            let mut rx = rx_a;
            while let Some(incoming) = rx.recv().await {
                let _ = sync_a_route
                    .handle_incoming(&incoming.device_id, incoming.message)
                    .await;
            }
        });
        let sync_b_route = Arc::clone(&sync_b);
        let route_b = tokio::spawn(async move {
            let mut rx = rx_b;
            while let Some(incoming) = rx.recv().await {
                let _ = sync_b_route
                    .handle_incoming(&incoming.device_id, incoming.message)
                    .await;
            }
        });

        let kinds = [
            SyncKind::Conversations,
            SyncKind::Plugins,
            SyncKind::UserCache,
        ];
        sync_a
            .push(&dev_b, &kinds)
            .await
            .expect("push should succeed");

        // 等待对端异步落盘（handle_remote_push 由路由 task 执行）
        tokio::time::sleep(Duration::from_millis(300)).await;

        // 验证 B 收到了 A 推送的数据
        {
            let inner = store_b.inner.lock().unwrap();
            let msgs = inner.conversations.get("conv-a1").unwrap();
            assert_eq!(msgs.len(), 1);
            assert_eq!(msgs[0].content, "from A");
            assert_eq!(inner.plugins.len(), 1);
            assert_eq!(inner.pinned.len(), 1);
        }

        route_a.abort();
        route_b.abort();
    }

    #[tokio::test]
    async fn sync_without_data_store_errors() {
        let (ta, rx_a, tb, _rx_b) = pair_transports().await;
        let dev_b = ta.online_device_ids().await.pop().unwrap();
        let sync_a = Arc::new(Sync::new(Arc::clone(&ta)));
        let sync_b = Arc::new(Sync::new(Arc::clone(&tb)));

        let sync_a_route = Arc::clone(&sync_a);
        let route_a = tokio::spawn(async move {
            let mut rx = rx_a;
            while let Some(incoming) = rx.recv().await {
                let _ = sync_a_route
                    .handle_incoming(&incoming.device_id, incoming.message)
                    .await;
            }
        });

        // 未注入数据源时 pull 应报错
        let err = sync_a.pull(&dev_b, 0, &[SyncKind::Conversations]).await;
        assert!(err.is_err(), "未注入数据源时 pull 应失败");
        let _ = sync_b;

        route_a.abort();
    }
}

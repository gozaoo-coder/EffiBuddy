//! P2P 镜像同步数据源适配器：把本地持久化存储暴露为 `SyncDataStore`。
//!
//! 由 AppState 的会话 / 插件 / 永久记忆存储实现，供 P2P 同步器读写，
//! 实现依赖倒置（`effisuite_p2p::sync::SyncDataStore`），业务层零侵入。
//!
//! # 并发与内存
//! - 会话合并用"读-合并-整写"（低频操作，失败可重试），消息按 id 去重、按时间戳排序
//! - 插件按 id 去重、版本相同跳过；永久记忆经 `PinnedMemoryStore::add`（同 id 幂等）
//! - 同步游标存内存 `RwLock<HashMap>`（读多写少），重启后 from-scratch 全量对齐

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use effisuite_core::{
    Conversation, ConversationStore, InstalledPlugin, Message, PinnedMemory, PinnedMemoryStore,
    PluginStore, Result,
};
use effisuite_p2p::protocol::ConvManifestEntry;
use effisuite_p2p::SyncDataStore;
use tokio::sync::RwLock;

/// P2P 镜像同步数据源（基于本地持久化存储）。
pub struct P2pSyncStore {
    /// 会话存储（含消息）
    conversations: Arc<ConversationStore>,
    /// 已安装插件存储
    plugins: PluginStore,
    /// 永久记忆存储
    pinned: Arc<PinnedMemoryStore>,
    /// 与各设备的同步游标：device_id → last_sync_ts（Unix 秒）
    cursors: RwLock<std::collections::HashMap<String, u64>>,
}

impl P2pSyncStore {
    /// 构造数据源（共享 AppState 中已有的存储句柄）。
    pub fn new(
        conversations: Arc<ConversationStore>,
        plugins: PluginStore,
        pinned: Arc<PinnedMemoryStore>,
    ) -> Self {
        Self {
            conversations,
            plugins,
            pinned,
            cursors: RwLock::new(std::collections::HashMap::with_capacity(8)),
        }
    }
}

#[async_trait]
impl SyncDataStore for P2pSyncStore {
    async fn list_conversations(&self) -> Result<Vec<ConvManifestEntry>> {
        let metas = self.conversations.list_meta().await?;
        Ok(metas
            .into_iter()
            .map(|m| ConvManifestEntry {
                id: m.id,
                title: m.title,
                updated_at: m.updated_at,
                message_count: m.message_count,
            })
            .collect())
    }

    async fn get_messages_since(&self, conv_id: &str, since: u64) -> Result<Vec<Message>> {
        let conv = self.conversations.load(conv_id).await?;
        Ok(conv
            .map(|c| {
                c.messages
                    .into_iter()
                    .filter(|m| m.timestamp > since)
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn upsert_messages(&self, conv_id: &str, messages: &[Message]) -> Result<()> {
        let mut conv = match self.conversations.load(conv_id).await? {
            Some(c) => c,
            None => {
                let created = messages
                    .first()
                    .map(|m| m.timestamp)
                    .unwrap_or_else(effisuite_core::remote_task_now);
                Conversation::new(conv_id.to_string(), created)
            }
        };
        // 按消息 id 去重，远端消息优先（保持时间戳升序）
        let existing: HashSet<String> = conv.messages.iter().map(|m| m.id.clone()).collect();
        let mut merged: Vec<Message> = messages
            .iter()
            .filter(|m| !existing.contains(&m.id))
            .cloned()
            .collect();
        conv.messages.append(&mut merged);
        conv.messages.sort_by_key(|m| m.timestamp);
        if let Some(last) = conv.messages.last() {
            conv.updated_at = last.timestamp;
        }
        self.conversations.save(&conv).await?;
        Ok(())
    }

    async fn list_plugins(&self) -> Result<Vec<InstalledPlugin>> {
        self.plugins.list().await
    }

    async fn upsert_plugins(&self, plugins: &[InstalledPlugin]) -> Result<()> {
        for p in plugins {
            let same_version = match self.plugins.get(&p.id).await? {
                Some(existing) => existing.version == p.version,
                None => false,
            };
            if !same_version {
                self.plugins.save(p).await?;
            }
        }
        Ok(())
    }

    async fn list_pinned(&self) -> Result<Vec<PinnedMemory>> {
        Ok(self.pinned.list().await)
    }

    async fn upsert_pinned(&self, memories: &[PinnedMemory]) -> Result<()> {
        for m in memories {
            // PinnedMemoryStore::add 同 id 幂等跳过
            self.pinned.add(m.clone()).await?;
        }
        Ok(())
    }

    async fn cursor(&self, device_id: &str) -> u64 {
        self.cursors.read().await.get(device_id).copied().unwrap_or(0)
    }

    async fn set_cursor(&self, device_id: &str, ts: u64) -> Result<()> {
        self.cursors
            .write()
            .await
            .insert(device_id.to_string(), ts);
        Ok(())
    }
}

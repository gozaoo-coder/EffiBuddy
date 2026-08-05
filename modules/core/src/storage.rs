//! 聊天记录持久化存储
//!
//! 基于 JSON 文件的简单持久化方案：每个 conversation 一个文件，
//! 存放在 `appdata/conversations/<id>.json`。
//!
//! 设计要点：
//! - 读多写少：`load`/`list_meta`/`search`/`save`/`delete` 无锁，直接 IO
//! - 读-改-写原子性：`rename`/`set_pinned`/`set_working_dir`/`append_message`
//!   使用**每会话独立锁**，仅阻塞同一会话的并发操作，不同会话互不阻塞
//! - 使用 `with_capacity` 预分配 list 返回值，避免多次扩容
//! - 使用 `with_capacity` 预分配 list 返回值，避免多次扩容
//! - 所有方法返回 `Result`，错误以 `CoreError::Io`/`Serde` 上抛
//!
//! ## 版本控制（git 风格）
//!
//! 用 `with_versions(...)` 构造可启用会话历史版本控制：每次 `append_message`
//! 自动追加一个 `Append` 提交（提交/消息池/引用仓库位于 `<root>/.versions/`）；
//! `version_*` 委托方法在会话锁内同步执行「版本操作 + 工作区覆盖」，
//! 支持开启分支 / 保存临时版本 / 回溯版本 / 撤回至此消息前 / 检出引用。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex as StdMutex;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{Conversation, CoreError, Message, Result};
use crate::versions::{RefSummary, VersionList, VersionOpResult, VersionStore};

/// 会话元信息（轻量，不含消息体），用于侧栏列表展示
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMeta {
    pub id: String,
    pub title: Option<String>,
    pub pinned_at: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
    pub message_count: usize,
    pub pinned: bool,
    /// 会话级工作区路径（hover 提示卡展示项目路径用），旧数据无此字段时为 None
    #[serde(default)]
    pub working_dir: Option<String>,
}

/// 搜索命中结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub conversation_id: String,
    pub conversation_title: String,
    pub message_id: String,
    pub snippet: String,
    pub score: usize,
    pub timestamp: u64,
    pub updated_at: u64,
    pub pinned: bool,
}

/// 查询分词：复用 `memory::tokenize`（CJK 单字+bigram 拆分），保证索引侧与查询侧一致
///
/// 详见 [`crate::memory::tokenize`] 的说明。
fn tokenize_query(query: &str) -> Vec<String> {
    crate::tokenize(query)
}

/// 计算消息对关键词的命中数
fn score_message(content: &str, keywords: &[String]) -> usize {
    let lower = content.to_lowercase();
    keywords
        .iter()
        .filter(|kw| lower.contains(kw.as_str()))
        .count()
}

/// 聊天记录存储，线程安全可廉价 clone（内部 `Arc` 共享）
///
/// 读-改-写操作（rename/set_pinned/set_working_dir/append_message）使用每会话
/// 独立锁，仅阻塞同一会话的并发操作。纯读（load/list_meta/search）和纯写
/// （save/delete）无锁，直接 IO。
#[derive(Clone)]
pub struct ConversationStore {
    root: PathBuf,
    /// 每会话独立锁表：`conversation_id → Arc<Mutex<()>>`。
    /// 外层 `StdMutex` 仅短暂持有（无 IO/await），内层 `tokio::Mutex` 跨 await 持有。
    locks: std::sync::Arc<StdMutex<HashMap<String, std::sync::Arc<Mutex<()>>>>>,
    /// 可选的会话版本仓库存储（git 风格历史）。`None` 表示未启用版本控制。
    versions: Option<VersionStore>,
}

impl ConversationStore {
    /// 创建存储，root 不存在时自动创建
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        Ok(Self {
            root,
            locks: std::sync::Arc::new(StdMutex::new(HashMap::new())),
            versions: None,
        })
    }

    /// 创建启用版本控制的会话存储（git 风格历史，仓库位于 `<root>/.versions/`）。
    pub fn with_versions(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        let versions = VersionStore::new(root.join(".versions"))?;
        Ok(Self {
            root,
            locks: std::sync::Arc::new(StdMutex::new(HashMap::new())),
            versions: Some(versions),
        })
    }

    /// 获取指定会话的独立锁（不存在则创建）。
    /// 外层锁是 `std::sync::Mutex`，仅短暂持有以查表，无 IO/await。
    #[inline]
    fn conv_lock(&self, id: &str) -> std::sync::Arc<Mutex<()>> {
        let mut map = self.locks.lock().unwrap();
        map.entry(id.to_string())
            .or_insert_with(|| std::sync::Arc::new(Mutex::new(())))
            .clone()
    }

    /// conversation 文件路径：`<root>/<id>.json`
    #[inline]
    fn path_for(&self, id: &str) -> PathBuf {
        // 防止 id 含路径分隔符导致越权访问：仅取文件名部分
        let safe = Path::new(id)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(id));
        self.root.join(safe).with_extension("json")
    }

    /// 加载单个 conversation，不存在返回 None
    pub async fn load(&self, id: &str) -> Result<Option<Conversation>> {
        let path = self.path_for(id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = tokio::fs::read(&path).await.map_err(CoreError::Io)?;
        let conv: Conversation = serde_json::from_slice(&bytes).map_err(CoreError::Serde)?;
        Ok(Some(conv))
    }

    /// 列出所有 conversation 元信息（不含消息体）。
    /// 排序规则：置顶在前（组内按 pinned_at 降序），未置顶按 updated_at 降序。
    /// 返回 ConversationMeta 结构体。
    pub async fn list_meta(&self) -> Result<Vec<ConversationMeta>> {
        let mut entries = tokio::fs::read_dir(&self.root)
            .await
            .map_err(CoreError::Io)?;
        let mut out = Vec::with_capacity(16);
        while let Some(entry) = entries.next_entry().await.map_err(CoreError::Io)? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(_) => continue,
            };
            let conv: Conversation = match serde_json::from_slice(&bytes) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            out.push(ConversationMeta {
                id,
                title: conv.title.clone(),
                pinned: conv.pinned,
                pinned_at: conv.pinned_at,
                created_at: conv.created_at,
                updated_at: conv.updated_at,
                message_count: conv.messages.len(),
                working_dir: conv.working_dir.clone(),
            });
        }
        // 排序：置顶在前 → 组内按 pinned_at/created_at 降序 → 未置顶按 updated_at 降序
        out.sort_by(|a, b| match (a.pinned, b.pinned) {
            (true, true) => {
                let ap = a.pinned_at.unwrap_or(a.created_at);
                let bp = b.pinned_at.unwrap_or(b.created_at);
                bp.cmp(&ap)
            }
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (false, false) => b.updated_at.cmp(&a.updated_at),
        });
        Ok(out)
    }

    /// 兼容旧调用：返回 (id, created_at, message_count) 三元组
    pub async fn list(&self) -> Result<Vec<(String, u64, usize)>> {
        Ok(self
            .list_meta()
            .await?
            .into_iter()
            .map(|m| (m.id, m.created_at, m.message_count))
            .collect())
    }

    /// 重命名会话
    pub async fn rename(&self, id: &str, title: String) -> Result<()> {
        let lock = self.conv_lock(id);
        let _guard = lock.lock().await;
        let mut conv = self.load(id).await?.ok_or_else(|| {
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "conversation not found",
            ))
        })?;
        conv.title = Some(title);
        self.save(&conv).await?;
        Ok(())
    }

    /// 设置/取消置顶
    pub async fn set_pinned(&self, id: &str, pinned: bool, now: u64) -> Result<()> {
        let lock = self.conv_lock(id);
        let _guard = lock.lock().await;
        let mut conv = self.load(id).await?.ok_or_else(|| {
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "conversation not found",
            ))
        })?;
        conv.pinned = pinned;
        conv.pinned_at = if pinned { Some(now) } else { None };
        self.save(&conv).await?;
        Ok(())
    }

    /// 设置会话级工作区路径。传入 None 清除工作区（回退到技能级或进程默认）。
    pub async fn set_working_dir(&self, id: &str, working_dir: Option<String>) -> Result<()> {
        let lock = self.conv_lock(id);
        let _guard = lock.lock().await;
        let mut conv = self.load(id).await?.ok_or_else(|| {
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "conversation not found",
            ))
        })?;
        conv.working_dir = working_dir;
        self.save(&conv).await?;
        Ok(())
    }

    /// 跨会话搜索消息内容。
    /// 遍历所有会话文件，对每条消息做关键词匹配，返回命中结果。
    /// 简单实现：按空白与标点分词后 contains 匹配。
    pub async fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let keywords: Vec<String> = tokenize_query(query);
        if keywords.is_empty() {
            return Ok(Vec::new());
        }
        let mut entries = tokio::fs::read_dir(&self.root)
            .await
            .map_err(CoreError::Io)?;
        let mut hits = Vec::with_capacity(16);
        while let Some(entry) = entries.next_entry().await.map_err(CoreError::Io)? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(_) => continue,
            };
            let conv: Conversation = match serde_json::from_slice(&bytes) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let conv_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let conv_title = conv.title.clone().unwrap_or_else(|| {
                conv.messages
                    .first()
                    .map(|m| m.content.chars().take(20).collect())
                    .unwrap_or_default()
            });
            for msg in &conv.messages {
                let score = score_message(&msg.content, &keywords);
                if score > 0 {
                    let snippet: String = msg.content.chars().take(80).collect();
                    hits.push(SearchHit {
                        conversation_id: conv_id.clone(),
                        conversation_title: conv_title.clone(),
                        message_id: msg.id.clone(),
                        snippet,
                        score,
                        timestamp: msg.timestamp,
                        pinned: conv.pinned,
                        updated_at: conv.updated_at,
                    });
                }
            }
            // 也匹配会话标题
            if let Some(title) = &conv.title {
                let score = score_message(title, &keywords) * 2; // 标题命中加权
                if score > 0 {
                    hits.push(SearchHit {
                        conversation_id: conv_id.clone(),
                        conversation_title: conv_title.clone(),
                        message_id: String::new(),
                        snippet: title.clone(),
                        score,
                        timestamp: conv.updated_at,
                        pinned: conv.pinned,
                        updated_at: conv.updated_at,
                    });
                }
            }
        }
        // 按分数降序，再按时间降序
        hits.sort_by(|a, b| b.score.cmp(&a.score).then(b.updated_at.cmp(&a.updated_at)));
        Ok(hits)
    }

    /// 保存（或覆盖）整个 conversation
    pub async fn save(&self, conv: &Conversation) -> Result<()> {
        let path = self.path_for(&conv.id);
        let bytes = serde_json::to_vec(conv).map_err(CoreError::Serde)?;
        tokio::fs::write(&path, bytes)
            .await
            .map_err(CoreError::Io)?;
        Ok(())
    }

    /// 追加消息到指定 conversation；若不存在则创建
    pub async fn append_message(
        &self,
        conv_id: &str,
        msg: Message,
        created_at: u64,
    ) -> Result<Conversation> {
        let lock = self.conv_lock(conv_id);
        let _guard = lock.lock().await;
        let mut conv = self
            .load(conv_id)
            .await?
            .unwrap_or_else(|| Conversation::new(conv_id.to_string(), created_at));
        conv.push(msg);
        self.save(&conv).await?;
        // git 风格版本控制：每次追加自动提交（失败仅告警，不阻塞消息持久化）
        if let Some(versions) = &self.versions {
            if let Err(e) = versions
                .commit_append(conv_id, &conv.messages, created_at)
                .await
            {
                tracing::warn!(
                    error = %e,
                    conversation_id = %conv_id,
                    "版本提交失败（不影响消息持久化）"
                );
            }
        }
        Ok(conv)
    }

    /// 用指定消息列表覆盖会话（版本操作回溯/撤回/检出后同步工作区）。
    /// 自动更新 `updated_at` 为当前时间戳。
    pub async fn replace_messages(
        &self,
        id: &str,
        messages: Vec<Message>,
        now: u64,
    ) -> Result<Conversation> {
        let mut conv = self
            .load(id)
            .await?
            .unwrap_or_else(|| Conversation::new(id.to_string(), now));
        conv.messages = messages;
        conv.updated_at = now;
        self.save(&conv).await?;
        Ok(conv)
    }

    // ---------- 会话版本控制（git 风格，委托 versions 模块） ----------

    /// 版本仓库句柄；未启用时返回友好错误
    fn version_store(&self) -> Result<&VersionStore> {
        self.versions
            .as_ref()
            .ok_or_else(|| CoreError::Msg("会话版本控制未启用（使用 ConversationStore::with_versions 构造）".into()))
    }

    /// 开启分支：从包含 `message_id` 的消息点创建新分支并切换 HEAD，
    /// 工作区同步为该消息点快照（其后的消息被留在原分支）。
    pub async fn version_create_branch(
        &self,
        id: &str,
        message_id: &str,
        now: u64,
    ) -> Result<VersionOpResult> {
        let lock = self.conv_lock(id);
        let _guard = lock.lock().await;
        let versions = self.version_store()?;
        let result = versions.create_branch(id, message_id, now).await?;
        self.replace_messages(id, result.messages.clone(), now).await?;
        Ok(result)
    }

    /// 保存临时版本：在包含 `message_id` 的消息点打 `temp-*` 书签（不移动 HEAD）
    pub async fn version_save_temp(
        &self,
        id: &str,
        message_id: &str,
        note: String,
        now: u64,
    ) -> Result<RefSummary> {
        let lock = self.conv_lock(id);
        let _guard = lock.lock().await;
        self.version_store()?
            .save_temp_version(id, message_id, note, now)
            .await
    }

    /// 回溯版本：重置 HEAD 到包含 `message_id` 的提交（丢弃其后消息）
    pub async fn version_rollback(
        &self,
        id: &str,
        message_id: &str,
        now: u64,
    ) -> Result<VersionOpResult> {
        let lock = self.conv_lock(id);
        let _guard = lock.lock().await;
        let versions = self.version_store()?;
        let result = versions.rollback_to_message(id, message_id, now).await?;
        self.replace_messages(id, result.messages.clone(), now).await?;
        Ok(result)
    }

    /// 撤回至此消息前：重置 HEAD 到该消息提交的父提交（丢弃该消息及其后全部）
    pub async fn version_undo_before(
        &self,
        id: &str,
        message_id: &str,
        now: u64,
    ) -> Result<VersionOpResult> {
        let lock = self.conv_lock(id);
        let _guard = lock.lock().await;
        let versions = self.version_store()?;
        let result = versions.undo_before_message(id, message_id, now).await?;
        self.replace_messages(id, result.messages.clone(), now).await?;
        Ok(result)
    }

    /// 检出到指定引用（分支/临时版本/检查点），工作区同步为对应快照
    pub async fn version_checkout(
        &self,
        id: &str,
        ref_name: &str,
        now: u64,
    ) -> Result<VersionOpResult> {
        let lock = self.conv_lock(id);
        let _guard = lock.lock().await;
        let versions = self.version_store()?;
        let result = versions.checkout_ref(id, ref_name, now).await?;
        self.replace_messages(id, result.messages.clone(), now).await?;
        Ok(result)
    }

    /// 会话版本列表（当前分支提交链 + 全部引用）
    pub async fn version_list(&self, id: &str) -> Result<VersionList> {
        match &self.versions {
            Some(v) => v.list_versions(id).await,
            None => Ok(VersionList {
                head: "main".to_string(),
                refs: Vec::new(),
                commits: Vec::new(),
            }),
        }
    }

    /// 删除引用（临时版本/检查点/分支；main 不可删除）
    pub async fn version_delete_ref(&self, id: &str, ref_name: &str) -> Result<()> {
        let lock = self.conv_lock(id);
        let _guard = lock.lock().await;
        self.version_store()?.delete_ref(id, ref_name).await
    }

    /// 删除指定 conversation，不存在返回 Ok(())
    pub async fn delete(&self, id: &str) -> Result<()> {
        let path = self.path_for(id);
        if path.exists() {
            tokio::fs::remove_file(&path).await.map_err(CoreError::Io)?;
        }
        // 联动清理会话版本仓库（git 风格历史）
        if let Some(versions) = &self.versions {
            versions.clear(id).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Role;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("effisuite-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn create_load_append_list_delete() {
        let dir = tmp_dir();
        let store = ConversationStore::new(&dir).unwrap();

        // 不存在时 load 返回 None
        assert!(store.load("c1").await.unwrap().is_none());

        // 追加消息会自动创建 conversation
        let m1 = Message::new("m1", Role::User, "hello", 1000);
        store.append_message("c1", m1, 1000).await.unwrap();
        let conv = store.load("c1").await.unwrap().unwrap();
        assert_eq!(conv.messages.len(), 1);
        assert_eq!(conv.id, "c1");

        // 再追加
        let m2 = Message::new("m2", Role::Assistant, "hi there", 1001);
        store.append_message("c1", m2, 1000).await.unwrap();
        let conv = store.load("c1").await.unwrap().unwrap();
        assert_eq!(conv.messages.len(), 2);

        // 第二个 conversation
        let m3 = Message::new("m3", Role::User, "another", 2000);
        store.append_message("c2", m3, 2000).await.unwrap();

        // list 应返回两个，按时间降序
        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0, "c2"); // created_at=2000 在前
        assert_eq!(list[1].0, "c1"); // created_at=1000 在后

        // delete
        store.delete("c1").await.unwrap();
        assert!(store.load("c1").await.unwrap().is_none());

        // 清理
        std::fs::remove_dir_all(&dir).ok();
    }
}

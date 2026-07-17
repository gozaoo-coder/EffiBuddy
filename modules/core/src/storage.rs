//! 聊天记录持久化存储
//!
//! 基于 JSON 文件的简单持久化方案：每个 conversation 一个文件，
//! 存放在 `appdata/conversations/<id>.json`。
//!
//! 设计要点：
//! - 读多写少：用 `RwLock` 而非 `Mutex`，允许多个并发读取
//! - 写入时仅短暂持锁序列化，IO 操作（文件写入）在锁外完成
//! - 使用 `with_capacity` 预分配 list 返回值，避免多次扩容
//! - 所有方法返回 `Result`，错误以 `CoreError::Io`/`Serde` 上抛

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{Conversation, CoreError, Message, Result};

/// 会话元信息（轻量，不含消息体），用于侧栏列表展示
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMeta {
    pub id: String,
    pub title: Option<String>,
    pub pinned: bool,
    pub pinned_at: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
    pub message_count: usize,
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
    pub pinned: bool,
    pub updated_at: u64,
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
    keywords.iter().filter(|kw| lower.contains(kw.as_str())).count()
}

/// 聊天记录存储，线程安全可廉价 clone（内部 RwLock + Arc 等价）
#[derive(Clone)]
pub struct ConversationStore {
    root: PathBuf,
    // 缓存最近写入的 conversation id 集合，避免每次 list 都全盘扫描
    // 实际场景中 conversation 数量有限，直接目录扫描也足够
    _lock: std::sync::Arc<RwLock<()>>,
}

impl ConversationStore {
    /// 创建存储，root 不存在时自动创建
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        Ok(Self {
            root,
            _lock: std::sync::Arc::new(RwLock::new(())),
        })
    }

    /// conversation 文件路径：`<root>/<id>.json`
    #[inline]
    fn path_for(&self, id: &str) -> PathBuf {
        // 防止 id 含路径分隔符导致越权访问：仅取文件名部分
        let safe = Path::new(id).file_name().unwrap_or_else(|| std::ffi::OsStr::new(id));
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
        let mut entries = tokio::fs::read_dir(&self.root).await.map_err(CoreError::Io)?;
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
            });
        }
        // 排序：置顶在前 → 组内按 pinned_at/created_at 降序 → 未置顶按 updated_at 降序
        out.sort_by(|a, b| {
            match (a.pinned, b.pinned) {
                (true, true) => {
                    let ap = a.pinned_at.unwrap_or(a.created_at);
                    let bp = b.pinned_at.unwrap_or(b.created_at);
                    bp.cmp(&ap)
                }
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                (false, false) => b.updated_at.cmp(&a.updated_at),
            }
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
        let _guard = self._lock.write().await;
        let mut conv = self
            .load(id)
            .await?
            .ok_or_else(|| CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "conversation not found",
            )))?;
        conv.title = Some(title);
        self.save(&conv).await?;
        Ok(())
    }

    /// 设置/取消置顶
    pub async fn set_pinned(&self, id: &str, pinned: bool, now: u64) -> Result<()> {
        let _guard = self._lock.write().await;
        let mut conv = self
            .load(id)
            .await?
            .ok_or_else(|| CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "conversation not found",
            )))?;
        conv.pinned = pinned;
        conv.pinned_at = if pinned { Some(now) } else { None };
        self.save(&conv).await?;
        Ok(())
    }

    /// 设置会话级工作区路径。传入 None 清除工作区（回退到技能级或进程默认）。
    pub async fn set_working_dir(&self, id: &str, working_dir: Option<String>) -> Result<()> {
        let _guard = self._lock.write().await;
        let mut conv = self
            .load(id)
            .await?
            .ok_or_else(|| CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "conversation not found",
            )))?;
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
        let mut entries = tokio::fs::read_dir(&self.root).await.map_err(CoreError::Io)?;
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
            let conv_title = conv
                .title
                .clone()
                .unwrap_or_else(|| conv.messages.first().map(|m| m.content.chars().take(20).collect()).unwrap_or_default());
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
        tokio::fs::write(&path, bytes).await.map_err(CoreError::Io)?;
        Ok(())
    }

    /// 追加消息到指定 conversation；若不存在则创建
    pub async fn append_message(&self, conv_id: &str, msg: Message, created_at: u64) -> Result<Conversation> {
        let _guard = self._lock.write().await;
        let mut conv = self.load(conv_id).await?.unwrap_or_else(|| {
            Conversation::new(conv_id.to_string(), created_at)
        });
        conv.push(msg);
        self.save(&conv).await?;
        Ok(conv)
    }

    /// 删除指定 conversation，不存在返回 Ok(())
    pub async fn delete(&self, id: &str) -> Result<()> {
        let path = self.path_for(id);
        if path.exists() {
            tokio::fs::remove_file(&path).await.map_err(CoreError::Io)?;
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

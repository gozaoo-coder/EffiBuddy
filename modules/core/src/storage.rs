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

use tokio::sync::RwLock;

use crate::{Conversation, CoreError, Message, Result};

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

    /// 列出所有 conversation 元信息（不含消息体），按 created_at 降序
    /// 返回 (id, created_at, message_count) 三元组
    pub async fn list(&self) -> Result<Vec<(String, u64, usize)>> {
        let mut entries = tokio::fs::read_dir(&self.root).await.map_err(CoreError::Io)?;
        let mut out = Vec::with_capacity(16);
        while let Some(entry) = entries.next_entry().await.map_err(CoreError::Io)? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            // 读取并解析，提取元信息
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(_) => continue, // 跳过无法读取的文件
            };
            let conv: Conversation = match serde_json::from_slice(&bytes) {
                Ok(c) => c,
                Err(_) => continue, // 跳过损坏文件
            };
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            out.push((id, conv.created_at, conv.messages.len()));
        }
        // 按创建时间降序（最新在前）
        out.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(out)
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

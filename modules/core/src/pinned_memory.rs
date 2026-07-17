//! 永久记忆（Pinned Memory）：用户主动要求"永久记住"的片段
//!
//! 与 `memory.rs` 的 RAG 记忆增强不同，永久记忆**不依赖检索相关性**：
//! 一旦被加入，每轮对话都会被注入到 prompt 上下文的 `[永久记忆]` 段，
//! 始终参与 LLM 推理。典型场景：
//!
//! - 用户在对话中明确说："请记住我的工作邮箱是 xxx@xxx.com"
//!   → LLM 调用 `pin_memory` 工具落盘
//! - 用户在 UI 面板手动新增一条偏好/事实/指令
//!   → 前端调用 `add_pinned_memory` 命令
//!
//! # 设计要点（对齐 user_rules）
//!
//! - 线程安全 + 廉价 clone：内部 `Arc<RwLock<..>>`，读多写少
//! - 锁内零 IO：`persist` 在锁外做 `tokio::fs::write`
//! - `format_for_context` 用 `with_capacity` 预分配，迭代器链排序
//! - `PinnedMemory` 字段按大小降序：String(24) → Option<String>(24)
//!   → Option<u64>(8) → u64(8) → enum(1)
//! - 落盘为单个 JSON 数组文件 `pinned_memories.json`，便于备份与 P2P 同步

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{CoreError, Result};

/// 永久记忆来源
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinnedMemorySource {
    /// 用户通过 UI 面板手动添加
    Manual,
    /// 用户在对话中明确要求 AI 记住（AI 调用 pin_memory 工具触发）
    UserRequest,
    /// AI 主动建议并经用户确认（预留）
    Assistant,
}

/// 一条永久记忆：每次对话都会被注入到 prompt 上下文
///
/// 字段按大小降序排列以最小化 padding：
/// `id`/`content`（String, 24B）→ `category`/`source_conversation_id`（Option<String>, 24B）
/// → `created_at`（u64, 8B）→ `source`（enum, 1B）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinnedMemory {
    pub id: String,
    pub content: String,
    /// 可选分类标签，如 "preference" / "fact" / "instruction"
    #[serde(default)]
    pub category: Option<String>,
    pub created_at: u64,
    pub source: PinnedMemorySource,
    /// 来源会话 id（若通过对话触发），用于审计回溯
    #[serde(default)]
    pub source_conversation_id: Option<String>,
}

/// 永久记忆存储，线程安全且可廉价 clone（内部 `Arc<RwLock<..>>`）
///
/// 与 `SkillStore` 落盘到单文件不同，本存储使用 `Arc<RwLock<State>>` 缓存
/// 全部条目，避免每次 `format_for_context` 都触发磁盘 IO——因为每轮对话
/// 都会调用它，热路径必须零 IO。
#[derive(Clone)]
pub struct PinnedMemoryStore {
    inner: Arc<RwLock<PinnedMemoryState>>,
    path: PathBuf,
}

struct PinnedMemoryState {
    memories: Vec<PinnedMemory>,
}

impl PinnedMemoryStore {
    /// 创建存储并加载磁盘上已存在的数据。
    ///
    /// `path` 为持久化文件路径（通常是 `<appdata>/pinned_memories.json`）。
    /// 父目录不存在时自动创建；文件不存在或解析失败时返回空列表（best-effort）。
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(CoreError::Io)?;
        }
        let memories = if path.exists() {
            match std::fs::read(&path) {
                Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };
        Ok(Self {
            inner: Arc::new(RwLock::new(PinnedMemoryState { memories })),
            path,
        })
    }

    /// 列出全部永久记忆（按 `created_at` 降序，新的在前）
    pub async fn list(&self) -> Vec<PinnedMemory> {
        let s = self.inner.read().await;
        let mut out: Vec<PinnedMemory> = s.memories.iter().cloned().collect();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        out
    }

    /// 新增一条永久记忆，返回新 id。`id` 为空则自动生成；相同 id 幂等跳过。
    pub async fn add(&self, mut memory: PinnedMemory) -> Result<String> {
        if memory.id.is_empty() {
            memory.id = uuid::Uuid::new_v4().to_string();
        }
        let id = memory.id.clone();
        {
            let mut s = self.inner.write().await;
            if !s.memories.iter().any(|m| m.id == memory.id) {
                s.memories.push(memory);
            }
        }
        self.persist().await?;
        Ok(id)
    }

    /// 便捷构造并新增一条记忆
    pub async fn add_simple(
        &self,
        content: impl Into<String>,
        category: Option<String>,
        source: PinnedMemorySource,
        source_conversation_id: Option<String>,
        created_at: u64,
    ) -> Result<String> {
        let memory = PinnedMemory {
            id: String::new(),
            content: content.into(),
            category,
            created_at,
            source,
            source_conversation_id,
        };
        self.add(memory).await
    }

    /// 更新指定 id 的 content 与/或 category。
    ///
    /// `category` 用 `Option<Option<String>>`：外层 `Some` 表示要更新，
    /// 内层 `None` 表示清空分类。
    pub async fn update(
        &self,
        id: &str,
        content: Option<String>,
        category: Option<Option<String>>,
    ) -> Result<()> {
        {
            let mut s = self.inner.write().await;
            let m = s
                .memories
                .iter_mut()
                .find(|m| m.id == id)
                .ok_or_else(|| {
                    CoreError::NotFound(format!("pinned memory {id} not found"))
                })?;
            if let Some(c) = content {
                m.content = c;
            }
            if let Some(cat) = category {
                m.category = cat;
            }
        }
        self.persist().await
    }

    /// 删除指定 id；不存在视为成功（幂等）
    pub async fn delete(&self, id: &str) -> Result<()> {
        let changed = {
            let mut s = self.inner.write().await;
            let before = s.memories.len();
            s.memories.retain(|m| m.id != id);
            before != s.memories.len()
        };
        if changed {
            self.persist().await?;
        }
        Ok(())
    }

    /// 清空所有永久记忆
    pub async fn clear(&self) -> Result<()> {
        {
            let mut s = self.inner.write().await;
            s.memories.clear();
        }
        self.persist().await
    }

    /// 按 id 查询单条
    pub async fn get(&self, id: &str) -> Option<PinnedMemory> {
        let s = self.inner.read().await;
        s.memories.iter().find(|m| m.id == id).cloned()
    }

    /// 格式化为 prompt 注入文本。
    ///
    /// 输出格式（按 `created_at` 升序，旧的在前，符合"先记住的优先展示"）：
    /// ```text
    /// [永久记忆]（用户要求永久记住的内容，请始终遵守/参考）
    /// 1. [preference] 用户偏好深色主题
    /// 2. 我的工作邮箱是 hr@effisuite.com
    /// ```
    ///
    /// 空列表返回空字符串，调用方据此跳过注入。
    pub async fn format_for_context(&self) -> String {
        let s = self.inner.read().await;
        if s.memories.is_empty() {
            return String::new();
        }
        // 按创建时间升序：先记住的排前面
        let mut sorted: Vec<&PinnedMemory> = s.memories.iter().collect();
        sorted.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        let mut out = String::with_capacity(sorted.len() * 64 + 64);
        out.push_str("[永久记忆]（用户要求永久记住的内容，请始终遵守/参考）\n");
        for (i, m) in sorted.iter().enumerate() {
            match m.category.as_deref() {
                Some(cat) if !cat.is_empty() => {
                    out.push_str(&format!("{}. [{}] {}\n", i + 1, cat, m.content));
                }
                _ => {
                    out.push_str(&format!("{}. {}\n", i + 1, m.content));
                }
            }
        }
        out
    }

    /// 持久化到磁盘（锁外 IO）
    async fn persist(&self) -> Result<()> {
        let bytes = {
            let s = self.inner.read().await;
            serde_json::to_vec_pretty(&s.memories).map_err(CoreError::Serde)?
        };
        tokio::fs::write(&self.path, bytes)
            .await
            .map_err(CoreError::Io)
    }
}

impl PinnedMemory {
    /// 快速构造一条新记忆（id 留空，由 store 自动生成）
    #[inline]
    pub fn new(
        content: impl Into<String>,
        category: Option<String>,
        source: PinnedMemorySource,
        created_at: u64,
    ) -> Self {
        Self {
            id: String::new(),
            content: content.into(),
            category,
            created_at,
            source,
            source_conversation_id: None,
        }
    }
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "effisuite-pinned-test-{}.json",
            uuid::Uuid::new_v4()
        ))
    }

    #[tokio::test]
    async fn add_list_delete_roundtrip() {
        let store = PinnedMemoryStore::new(tmp_path()).unwrap();
        assert!(store.list().await.is_empty());

        let id1 = store
            .add_simple(
                "用户偏好深色主题",
                Some("preference".into()),
                PinnedMemorySource::Manual,
                None,
                100,
            )
            .await
            .unwrap();
        let id2 = store
            .add_simple(
                "工作邮箱是 hr@effisuite.com",
                None,
                PinnedMemorySource::UserRequest,
                Some("conv-1".into()),
                200,
            )
            .await
            .unwrap();

        let list = store.list().await;
        assert_eq!(list.len(), 2);
        // 降序：created_at=200 在前
        assert_eq!(list[0].id, id2);
        assert_eq!(list[1].id, id1);

        store.delete(&id1).await.unwrap();
        assert_eq!(store.list().await.len(), 1);
    }

    #[tokio::test]
    async fn update_content_and_category() {
        let store = PinnedMemoryStore::new(tmp_path()).unwrap();
        let id = store
            .add_simple("orig", Some("cat-a".into()), PinnedMemorySource::Manual, None, 1)
            .await
            .unwrap();

        store
            .update(&id, Some("updated content".into()), None)
            .await
            .unwrap();
        let m = store.get(&id).await.unwrap();
        assert_eq!(m.content, "updated content");
        assert_eq!(m.category.as_deref(), Some("cat-a"));

        // 清空 category
        store
            .update(&id, None, Some(None))
            .await
            .unwrap();
        let m = store.get(&id).await.unwrap();
        assert!(m.category.is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_is_idempotent() {
        let store = PinnedMemoryStore::new(tmp_path()).unwrap();
        store.delete("nope").await.unwrap();
    }

    #[tokio::test]
    async fn format_for_context_empty_returns_empty() {
        let store = PinnedMemoryStore::new(tmp_path()).unwrap();
        assert!(store.format_for_context().await.is_empty());
    }

    #[tokio::test]
    async fn format_for_context_orders_by_created_at_asc() {
        let store = PinnedMemoryStore::new(tmp_path()).unwrap();
        store
            .add_simple("late", None, PinnedMemorySource::Manual, None, 300)
            .await
            .unwrap();
        store
            .add_simple("early", None, PinnedMemorySource::Manual, None, 100)
            .await
            .unwrap();
        store
            .add_simple("mid", Some("fact".into()), PinnedMemorySource::Manual, None, 200)
            .await
            .unwrap();

        let formatted = store.format_for_context().await;
        // early 应排在最前（编号 1）
        let early_pos = formatted.find("early").unwrap();
        let mid_pos = formatted.find("mid").unwrap();
        let late_pos = formatted.find("late").unwrap();
        assert!(early_pos < mid_pos);
        assert!(mid_pos < late_pos);
        // 含分类标签
        assert!(formatted.contains("[fact] mid"));
    }

    #[tokio::test]
    async fn persist_and_reload_across_instances() {
        let path = tmp_path();
        let store_a = PinnedMemoryStore::new(&path).unwrap();
        store_a
            .add_simple("persistent content", None, PinnedMemorySource::Manual, None, 1)
            .await
            .unwrap();

        // 用同一文件路径再创建一个实例，应能读到已落盘数据
        let store_b = PinnedMemoryStore::new(&path).unwrap();
        let list = store_b.list().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].content, "persistent content");

        // 清理临时文件
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn clear_removes_all() {
        let store = PinnedMemoryStore::new(tmp_path()).unwrap();
        store
            .add_simple("a", None, PinnedMemorySource::Manual, None, 1)
            .await
            .unwrap();
        store
            .add_simple("b", None, PinnedMemorySource::Manual, None, 2)
            .await
            .unwrap();
        assert_eq!(store.list().await.len(), 2);
        store.clear().await.unwrap();
        assert!(store.list().await.is_empty());
        assert!(store.format_for_context().await.is_empty());
    }

    #[tokio::test]
    async fn add_with_same_id_is_idempotent() {
        let store = PinnedMemoryStore::new(tmp_path()).unwrap();
        let m = PinnedMemory {
            id: "fixed-id".to_string(),
            content: "dup".to_string(),
            category: None,
            created_at: 1,
            source: PinnedMemorySource::Manual,
            source_conversation_id: None,
        };
        store.add(m.clone()).await.unwrap();
        store.add(m).await.unwrap();
        assert_eq!(store.list().await.len(), 1);
    }
}

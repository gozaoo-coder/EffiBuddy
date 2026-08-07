//! 子 agent 会话持久化存储（独立落盘 + 可恢复 + 实时增量写盘）
//!
//! 每个子 agent 会话一个 JSON 文件，存放在 `<root>/<session_id>.json`。
//! 与主会话 `ConversationStore` 不同，子 agent 会话过去是纯内存态（重启即丢），
//! 本模块将其独立落盘，实现：
//! - **实时增量落盘**：子 agent 执行过程中反复调用 `save`，每次写完整文档
//!   （原子写：先写临时文件再 rename），崩溃/中断也能保留已生成内容。
//! - **可恢复继续**：`messages` 保存完整对话历史，重启后加载即可续聊。
//! - **独立历史入口**：`list_meta` 列出全部已落盘的子 agent 会话供前端展示。
//!
//! 设计要点：
//! - 读多写少；`save` 用**原子写**（temp + rename），读者永远看到完整文件。
//! - 无内部锁：同一会话始终由所属 `SubAgentManager` 的单一执行流写入，
//!   天然串行；`list_meta` 的并发读由原子写保证一致性。
//! - 使用 `with_capacity` 预分配 list 返回值，避免多次扩容。
//! - 所有方法返回 `Result`，错误以 `CoreError::Io`/`Serde` 上抛。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{CoreError, Message, Result, SubAgentImage, ToolCallRecord};

/// 已落盘的子 agent 会话（完整文档，含续聊所需的历史消息）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentSessionDoc {
    /// 子 agent 会话 id（`sa_xxxx`）
    pub session_id: String,
    /// 显示名
    pub name: String,
    /// 模型名
    pub model: String,
    /// 嵌套深度（1 = 主 agent 直接召唤）
    pub depth: usize,
    /// 父主会话 conversation_id（前端据此关联/过滤）
    #[serde(default)]
    pub conversation_id: String,
    /// 运行状态："running" | "done" | "error"
    pub status: String,
    /// 主 agent 最近一次交给子 agent 的任务
    pub task: String,
    /// 子 agent 最近一轮回复全文
    pub text: String,
    /// 子 agent 内部工具调用记录（最近一轮）
    #[serde(default, rename = "toolCalls")]
    pub tool_calls: Vec<ToolCallRecord>,
    /// 子 agent 生成的图片附件（path + name）
    #[serde(default)]
    pub images: Vec<SubAgentImage>,
    /// 错误信息（status=error 时）
    #[serde(default)]
    pub error: String,
    /// 完成时间（Unix 毫秒）
    #[serde(default, rename = "finishedAt")]
    pub finished_at: Option<i64>,
    /// 完整对话历史（续聊依据）：User 任务 + Assistant 文本轮回
    #[serde(default)]
    pub messages: Vec<Message>,
    /// 创建时间（Unix 毫秒）
    pub created_at: u64,
    /// 最近更新时间（Unix 毫秒）
    pub updated_at: u64,
}

impl SubAgentSessionDoc {
    /// 构造一个全新的空会话文档
    pub fn new(session_id: impl Into<String>, now: u64) -> Self {
        Self {
            session_id: session_id.into(),
            name: String::new(),
            model: String::new(),
            depth: 1,
            conversation_id: String::new(),
            status: "running".to_string(),
            task: String::new(),
            text: String::new(),
            tool_calls: Vec::new(),
            images: Vec::new(),
            error: String::new(),
            finished_at: None,
            messages: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

/// 已落盘子 agent 会话的轻量元信息（不含消息体），供历史列表展示
#[derive(Debug, Clone, Serialize)]
pub struct SubAgentSessionMeta {
    pub session_id: String,
    pub name: String,
    pub model: String,
    pub depth: usize,
    pub status: String,
    pub task: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub message_count: usize,
}

/// 子 agent 会话存储，线程安全可廉价 clone（内部 `Arc` 共享）
#[derive(Clone)]
pub struct SubAgentStore {
    root: PathBuf,
}

impl SubAgentStore {
    /// 创建存储，root 不存在时自动创建
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        Ok(Self { root })
    }

    /// 会话文件路径：`<root>/<session_id>.json`
    #[inline]
    fn path_for(&self, session_id: &str) -> PathBuf {
        // 防止 id 含路径分隔符导致越权访问：仅取文件名部分
        let safe = Path::new(session_id)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(session_id));
        self.root.join(safe).with_extension("json")
    }

    /// 保存（或覆盖）整个子 agent 会话文档。原子写：先写临时文件再 rename，
    /// 避免写一半崩溃留下损坏文件；读者要么看到旧完整版要么看到新完整版。
    pub async fn save(&self, doc: &SubAgentSessionDoc) -> Result<()> {
        let path = self.path_for(&doc.session_id);
        let bytes = serde_json::to_vec(doc).map_err(CoreError::Serde)?;
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, &bytes).await.map_err(CoreError::Io)?;
        tokio::fs::rename(&tmp, &path).await.map_err(CoreError::Io)?;
        Ok(())
    }

    /// 加载单个子 agent 会话，不存在返回 None
    pub async fn load(&self, session_id: &str) -> Result<Option<SubAgentSessionDoc>> {
        let path = self.path_for(session_id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = tokio::fs::read(&path).await.map_err(CoreError::Io)?;
        let doc: SubAgentSessionDoc =
            serde_json::from_slice(&bytes).map_err(CoreError::Serde)?;
        Ok(Some(doc))
    }

    /// 列出所有已落盘子 agent 会话元信息（不含消息体）。
    /// 排序规则：按 updated_at 降序。
    ///
    /// 兼容性：单条文件反序列化失败时跳过（不阻塞整体），并清理残留的 .tmp 文件。
    pub async fn list_meta(&self) -> Result<Vec<SubAgentSessionMeta>> {
        let mut entries = tokio::fs::read_dir(&self.root)
            .await
            .map_err(CoreError::Io)?;
        let mut out = Vec::with_capacity(16);
        while let Some(entry) = entries.next_entry().await.map_err(CoreError::Io)? {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str());
            // 清理原子写残留的临时文件
            if ext == Some("tmp") {
                let _ = tokio::fs::remove_file(&path).await;
                continue;
            }
            if ext != Some("json") {
                continue;
            }
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(_) => continue,
            };
            let doc: SubAgentSessionDoc = match serde_json::from_slice(&bytes) {
                Ok(d) => d,
                Err(_) => continue,
            };
            out.push(SubAgentSessionMeta {
                session_id: doc.session_id,
                name: doc.name,
                model: doc.model,
                depth: doc.depth,
                status: doc.status,
                task: doc.task,
                created_at: doc.created_at,
                updated_at: doc.updated_at,
                message_count: doc.messages.len(),
            });
        }
        // 按最近更新降序
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(out)
    }

    /// 删除指定子 agent 会话，不存在返回 Ok(())
    pub async fn delete(&self, session_id: &str) -> Result<()> {
        let path = self.path_for(session_id);
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
        let dir = std::env::temp_dir().join(format!("effisuite-sa-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn save_load_list_delete_roundtrip() {
        let dir = tmp_dir();
        let store = SubAgentStore::new(&dir).unwrap();

        assert!(store.load("sa_1").await.unwrap().is_none());

        let mut doc = SubAgentSessionDoc::new("sa_1", 1000);
        doc.name = "审查员".to_string();
        doc.model = "gpt-4o".to_string();
        doc.task = "审查 main.rs".to_string();
        doc.text = "无安全问题".to_string();
        doc.status = "done".to_string();
        doc.messages.push(Message::new("m1", Role::User, "审查", 1000));
        store.save(&doc).await.unwrap();

        let loaded = store.load("sa_1").await.unwrap().unwrap();
        assert_eq!(loaded.name, "审查员");
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.status, "done");

        let meta = store.list_meta().await.unwrap();
        assert_eq!(meta.len(), 1);
        assert_eq!(meta[0].session_id, "sa_1");
        assert_eq!(meta[0].message_count, 1);

        store.delete("sa_1").await.unwrap();
        assert!(store.load("sa_1").await.unwrap().is_none());

        std::fs::remove_dir_all(&dir).ok();
    }
}
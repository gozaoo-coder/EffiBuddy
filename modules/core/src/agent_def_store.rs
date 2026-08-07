//! 自定义智能体（AgentDef）持久化存储
//!
//! 用户可在「自动化 → 建立智能体」中创建自定义 agent（角色、系统提示词、模型），
//! 主 agent / 子 agent 在召唤 sub-agent 时可指定某个自定义 agent 定义，注入其
//! 系统提示词与模型，从而"召唤某个自定义智能体"。
//!
//! 基于 JSON 文件：每个定义一个文件，存放在 `appdata/agent_defs/<id>.json`。
//! 模式与 [`crate::schedule_store::ScheduledTaskStore`] 一致：`RwLock` 多读、
//! IO 在锁外、`with_capacity` 预分配。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{CoreError, Result};

/// 自定义智能体定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDef {
    /// 唯一 id（创建时生成 uuid）
    pub id: String,
    /// 显示名（如「代码审查师」）
    pub name: String,
    /// 角色描述（一句话说明其职责，用于群聊 @ 与列表展示）
    pub role: String,
    /// 系统提示词（作为该 agent 的 preamble 注入）
    pub system_prompt: String,
    /// 使用的模型 id（None 时回退到全局 active_model_id）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// 头像 emoji（群聊 / 列表展示用）
    #[serde(default = "default_avatar")]
    pub avatar: String,
    /// 是否启用工具
    #[serde(default = "default_enable_tools")]
    pub enable_tools: bool,
    pub created_at: u64,
    pub updated_at: u64,
}

fn default_avatar() -> String {
    "🤖".to_string()
}

fn default_enable_tools() -> bool {
    true
}

/// 自定义智能体存储，线程安全可廉价 clone（内部 RwLock + Arc）
#[derive(Clone)]
pub struct AgentDefStore {
    root: PathBuf,
    _lock: std::sync::Arc<RwLock<()>>,
}

impl AgentDefStore {
    /// 创建存储，root 不存在时自动创建
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        Ok(Self {
            root,
            _lock: std::sync::Arc::new(RwLock::new(())),
        })
    }

    /// 定义文件路径：`<root>/<id>.json`
    #[inline]
    fn path_for(&self, id: &str) -> PathBuf {
        let safe = Path::new(id)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(id));
        self.root.join(safe).with_extension("json")
    }

    /// 列出全部定义，按 created_at 降序
    pub async fn list(&self) -> Result<Vec<AgentDef>> {
        let mut entries = tokio::fs::read_dir(&self.root)
            .await
            .map_err(CoreError::Io)?;
        let mut out = Vec::with_capacity(8);
        while let Some(entry) = entries.next_entry().await.map_err(CoreError::Io)? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(bytes) = tokio::fs::read(&path).await else {
                continue;
            };
            let Ok(def) = serde_json::from_slice::<AgentDef>(&bytes) else {
                continue;
            };
            out.push(def);
        }
        out.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        Ok(out)
    }

    /// 加载单个定义，不存在返回 None
    pub async fn get(&self, id: &str) -> Result<Option<AgentDef>> {
        let path = self.path_for(id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = tokio::fs::read(&path).await.map_err(CoreError::Io)?;
        let def: AgentDef = serde_json::from_slice(&bytes).map_err(CoreError::Serde)?;
        Ok(Some(def))
    }

    /// 保存（或覆盖）一个定义
    pub async fn save(&self, def: &AgentDef) -> Result<()> {
        let path = self.path_for(&def.id);
        let bytes = serde_json::to_vec(def).map_err(CoreError::Serde)?;
        tokio::fs::write(&path, bytes)
            .await
            .map_err(CoreError::Io)?;
        Ok(())
    }

    /// 删除指定定义，不存在返回 Ok(())
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

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("effisuite-agentdef-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn def(id: &str) -> AgentDef {
        AgentDef {
            id: id.to_string(),
            name: "代码审查师".to_string(),
            role: "审查代码质量".to_string(),
            system_prompt: "你是专业的代码审查师".to_string(),
            model_id: None,
            avatar: "🔍".to_string(),
            enable_tools: true,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[tokio::test]
    async fn save_list_get_delete() {
        let store = AgentDefStore::new(tmp_dir()).unwrap();
        store.save(&def("a1")).await.unwrap();
        assert_eq!(store.list().await.unwrap().len(), 1);
        let got = store.get("a1").await.unwrap().unwrap();
        assert_eq!(got.name, "代码审查师");
        store.delete("a1").await.unwrap();
        assert!(store.get("a1").await.unwrap().is_none());
    }
}
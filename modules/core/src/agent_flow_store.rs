//! Agent Flow（智能体流程）持久化存储
//!
//! ComfyUI 风格的可视化节点式流程：用户把多个处理节点拖拽到画布、连线，
//! 每个节点有输入/输出类型与参数，整条流程可保存并在「运行」时按拓扑顺序执行。
//!
//! 基于 JSON 文件：每个流程一个文件，存放在 `appdata/agent_flows/<id>.json`。
//! 模式与 [`crate::schedule_store::ScheduledTaskStore`] 一致。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{CoreError, Result};

/// 端口（输入 / 输出）数据类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowDataType {
    /// 文本
    Text,
    /// 文件路径
    File,
    /// 图片
    Image,
    /// 音频
    Audio,
    /// 通用 JSON 对象
    Object,
    /// 数字
    Number,
}

impl FlowDataType {
    /// 中文标签
    pub fn label(&self) -> &'static str {
        match self {
            FlowDataType::Text => "文本",
            FlowDataType::File => "文件",
            FlowDataType::Image => "图片",
            FlowDataType::Audio => "音频",
            FlowDataType::Object => "对象",
            FlowDataType::Number => "数字",
        }
    }

    /// 端口颜色（前端连线类型着色用）
    pub fn color(&self) -> &'static str {
        match self {
            FlowDataType::Text => "#4a7eff",
            FlowDataType::File => "#f2994a",
            FlowDataType::Image => "#e5484d",
            FlowDataType::Audio => "#8e4ec6",
            FlowDataType::Object => "#30a46c",
            FlowDataType::Number => "#6e56cf",
        }
    }
}

/// 流程节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowNode {
    pub id: String,
    /// 节点类型 key（对应前端节点注册表中的实现）
    pub node_type: String,
    /// 显示标签
    pub label: String,
    /// 画布坐标
    pub x: f64,
    pub y: f64,
    /// 节点参数（JSON 对象，key=参数名）
    pub params: serde_json::Value,
    /// 输入端口数据类型
    pub input_type: FlowDataType,
    /// 输出端口数据类型
    pub output_type: FlowDataType,
}

/// 流程连线
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowEdge {
    pub id: String,
    /// 源节点 id
    pub from: String,
    /// 目标节点 id
    pub to: String,
}

/// Agent Flow 流程定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentFlow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// 智能体流程存储，线程安全可廉价 clone（内部 RwLock + Arc）
#[derive(Clone)]
pub struct AgentFlowStore {
    root: PathBuf,
    _lock: std::sync::Arc<RwLock<()>>,
}

impl AgentFlowStore {
    /// 创建存储，root 不存在时自动创建
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        Ok(Self {
            root,
            _lock: std::sync::Arc::new(RwLock::new(())),
        })
    }

    /// 流程文件路径：`<root>/<id>.json`
    #[inline]
    fn path_for(&self, id: &str) -> PathBuf {
        let safe = Path::new(id)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(id));
        self.root.join(safe).with_extension("json")
    }

    /// 列出全部流程，按 updated_at 降序
    pub async fn list(&self) -> Result<Vec<AgentFlow>> {
        let mut entries = tokio::fs::read_dir(&self.root)
            .await
            .map_err(CoreError::Io)?;
        let mut out = Vec::with_capacity(4);
        while let Some(entry) = entries.next_entry().await.map_err(CoreError::Io)? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(bytes) = tokio::fs::read(&path).await else {
                continue;
            };
            let Ok(flow) = serde_json::from_slice::<AgentFlow>(&bytes) else {
                continue;
            };
            out.push(flow);
        }
        out.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
        Ok(out)
    }

    /// 加载单个流程，不存在返回 None
    pub async fn get(&self, id: &str) -> Result<Option<AgentFlow>> {
        let path = self.path_for(id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = tokio::fs::read(&path).await.map_err(CoreError::Io)?;
        let flow: AgentFlow = serde_json::from_slice(&bytes).map_err(CoreError::Serde)?;
        Ok(Some(flow))
    }

    /// 保存（或覆盖）一个流程
    pub async fn save(&self, flow: &AgentFlow) -> Result<()> {
        let path = self.path_for(&flow.id);
        let bytes = serde_json::to_vec(flow).map_err(CoreError::Serde)?;
        tokio::fs::write(&path, bytes)
            .await
            .map_err(CoreError::Io)?;
        Ok(())
    }

    /// 删除指定流程，不存在返回 Ok(())
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
        let dir = std::env::temp_dir().join(format!("effisuite-flow-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn flow(id: &str) -> AgentFlow {
        AgentFlow {
            id: id.to_string(),
            name: "日报流程".to_string(),
            description: String::new(),
            nodes: vec![FlowNode {
                id: "n1".to_string(),
                node_type: "input".to_string(),
                label: "输入".to_string(),
                x: 0.0,
                y: 0.0,
                params: serde_json::json!({}),
                input_type: FlowDataType::Text,
                output_type: FlowDataType::Text,
            }],
            edges: Vec::new(),
            created_at: 1,
            updated_at: 1,
        }
    }

    #[tokio::test]
    async fn save_list_get_delete() {
        let store = AgentFlowStore::new(tmp_dir()).unwrap();
        store.save(&flow("f1")).await.unwrap();
        assert_eq!(store.list().await.unwrap().len(), 1);
        let got = store.get("f1").await.unwrap().unwrap();
        assert_eq!(got.name, "日报流程");
        assert_eq!(got.nodes[0].output_type.label(), "文本");
        store.delete("f1").await.unwrap();
        assert!(store.get("f1").await.unwrap().is_none());
    }
}
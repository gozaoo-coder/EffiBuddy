//! Agent 后端配置
//!
//! 用于在前端 UI 与 Rust 后端之间传递 agent 配置。
//! 持久化到 `appdata/config.json`，启动时读取以决定使用 MockAgent 还是 RigAgent。
//!
//! 字段按大小降序：String(24) 在前，bool(1) 在后，最小化 padding。

use serde::{Deserialize, Serialize};

/// Agent 后端类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    /// 离线 mock，无网络
    Mock,
    /// 通过 rig 调用 OpenAI 兼容接口
    Openai,
}

impl Default for BackendKind {
    fn default() -> Self {
        Self::Mock
    }
}

/// Agent 配置（可被前端修改并持久化）
///
/// 字段顺序：String 在前（24 字节），bool 在后（1 字节）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub api_key: String,
    pub base_url: String,
    pub model_name: String,
    pub preamble: String,
    pub backend: BackendKind,
    /// 是否启用工具调用（RAG 索引等）
    pub enable_tools: bool,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: String::new(),
            model_name: "gpt-4o-mini".to_string(),
            preamble: "你是 EffiSuite 的 AI 助手，简洁友好地回答用户问题。".to_string(),
            backend: BackendKind::Mock,
            enable_tools: true,
        }
    }
}

impl AgentConfig {
    /// 判断当前配置是否可启动 RigAgent（backend=openai 且有 api_key）
    #[inline]
    pub fn is_rig_ready(&self) -> bool {
        matches!(self.backend, BackendKind::Openai) && !self.api_key.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_is_mock() {
        let c = AgentConfig::default();
        assert!(matches!(c.backend, BackendKind::Mock));
        assert!(!c.is_rig_ready());
    }

    #[test]
    fn config_rig_ready_when_key_present() {
        let mut c = AgentConfig::default();
        c.backend = BackendKind::Openai;
        c.api_key = "sk-xxx".to_string();
        assert!(c.is_rig_ready());
    }

    #[test]
    fn config_serde_roundtrip() {
        let c = AgentConfig {
            api_key: "k".into(),
            base_url: "https://api.openai.com/v1".into(),
            model_name: "gpt-4o".into(),
            preamble: "hi".into(),
            backend: BackendKind::Openai,
            enable_tools: false,
        };
        let s = serde_json::to_string(&c).unwrap();
        let back: AgentConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c.api_key, back.api_key);
        assert_eq!(c.backend, back.backend);
        assert_eq!(c.enable_tools, back.enable_tools);
    }
}

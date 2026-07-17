//! Agent 后端配置 + Provider 预设 + 可使用模型 + 主题
//!
//! 持久化到 `appdata/config.json`，启动时读取以决定使用 MockAgent 还是 RigAgent。
//!
//! 设计要点：
//! - `AgentConfig` 同时承载"当前激活配置"与"可使用模型列表"，单文件持久化
//! - `ProviderPreset` 是内置元数据（非持久化），由 `builtin_presets()` 提供
//! - 所有 OpenAI 兼容 provider 统一用 `openai::CompletionsClient` + base_url 覆盖
//! - 字段按大小降序：String(24) 在前，bool/enum(1) 在后，最小化 padding

use serde::{Deserialize, Serialize};

/// Agent 后端类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    /// 离线 mock，无网络
    Mock,
    /// 通过 rig 调用 OpenAI 兼容接口（统一走 Chat Completions API）
    Openai,
}

impl Default for BackendKind {
    fn default() -> Self {
        Self::Mock
    }
}

/// 主题模式：系统 / 亮色 / 暗色
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// 跟随系统 prefers-color-scheme
    System,
    /// 强制亮色
    Light,
    /// 强制暗色
    Dark,
}

impl Default for ThemeMode {
    fn default() -> Self {
        Self::System
    }
}

/// Agent 配置（可被前端修改并持久化）
///
/// 同时承载：当前激活的运行时配置 + 可使用模型列表 + 主题。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    // ===== 当前激活的运行时配置 =====
    pub api_key: String,
    pub base_url: String,
    pub model_name: String,
    pub preamble: String,
    pub backend: BackendKind,
    /// 当前激活配置所属的 provider id（"openai" / "deepseek" / "custom" ...）
    pub provider_id: String,
    /// 是否启用工具调用（RAG 索引等）
    pub enable_tools: bool,

    // ===== 用户级偏好 =====
    pub theme: ThemeMode,

    // ===== 可使用模型列表（用户保存的预设） =====
    pub models: Vec<AvailableModel>,
    /// 当前激活的模型 id（指向 models 中的某一项）；None 表示使用内联配置
    pub active_model_id: Option<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: String::new(),
            model_name: "gpt-4o-mini".to_string(),
            preamble: "你是 EffiSuite 的 AI 助手，简洁友好地回答用户问题。".to_string(),
            backend: BackendKind::Mock,
            provider_id: "openai".to_string(),
            enable_tools: true,
            theme: ThemeMode::System,
            models: Vec::new(),
            active_model_id: None,
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

/// 可使用模型：用户保存的预设配置，可快速切换
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailableModel {
    /// 唯一 id（uuid）
    pub id: String,
    /// 用户自定义标签，如 "我的 GPT-4o"
    pub label: String,
    /// provider id（"openai" / "deepseek" / "custom"）
    pub provider_id: String,
    pub base_url: String,
    pub model_name: String,
    pub api_key: String,
    pub preamble: String,
    pub enable_tools: bool,
    pub created_at: u64,
}

/// Provider 预设元数据（内置，非持久化）
#[derive(Debug, Clone, Serialize)]
pub struct ProviderPreset {
    /// 唯一 id，与 AvailableModel.provider_id 对应
    pub id: String,
    /// 显示名
    pub name: String,
    /// 默认 base_url（用户可覆盖）
    pub default_base_url: String,
    /// 默认模型名（用于填充表单）
    pub default_model: String,
    /// 推荐的环境变量名（仅用于提示用户）
    pub env_var: String,
    /// 文档地址
    pub docs_url: String,
    /// 是否为 OpenAI 兼容协议（true 表示可用 CompletionsClient 统一构造）
    pub openai_compat: bool,
}

/// 内置 provider 预设列表
///
/// 仅包含 OpenAI 兼容协议的 provider（可用 `openai::CompletionsClient` 统一构造）。
/// 其他原生协议 provider（anthropic/gemini/ollama/xai）暂不支持，后续按需单独适配。
/// 另含一个 "custom" 预设，允许用户填入任意 OpenAI 兼容服务。
pub fn builtin_presets() -> Vec<ProviderPreset> {
    vec![
        ProviderPreset {
            id: "openai".into(),
            name: "OpenAI".into(),
            default_base_url: "https://api.openai.com/v1".into(),
            default_model: "gpt-4o-mini".into(),
            env_var: "OPENAI_API_KEY".into(),
            docs_url: "https://platform.openai.com/docs/api-reference".into(),
            openai_compat: true,
        },
        ProviderPreset {
            id: "deepseek".into(),
            name: "DeepSeek".into(),
            default_base_url: "https://api.deepseek.com".into(),
            default_model: "deepseek-chat".into(),
            env_var: "DEEPSEEK_API_KEY".into(),
            docs_url: "https://api-docs.deepseek.com/".into(),
            openai_compat: true,
        },
        ProviderPreset {
            id: "groq".into(),
            name: "Groq".into(),
            default_base_url: "https://api.groq.com/openai/v1".into(),
            default_model: "llama-3.3-70b-versatile".into(),
            env_var: "GROQ_API_KEY".into(),
            docs_url: "https://console.groq.com/docs".into(),
            openai_compat: true,
        },
        ProviderPreset {
            id: "moonshot".into(),
            name: "Moonshot (Kimi)".into(),
            default_base_url: "https://api.moonshot.ai/v1".into(),
            default_model: "moonshot-v1-8k".into(),
            env_var: "MOONSHOT_API_KEY".into(),
            docs_url: "https://platform.moonshot.cn/docs".into(),
            openai_compat: true,
        },
        ProviderPreset {
            id: "openrouter".into(),
            name: "OpenRouter".into(),
            default_base_url: "https://openrouter.ai/api/v1".into(),
            default_model: "openai/gpt-4o-mini".into(),
            env_var: "OPENROUTER_API_KEY".into(),
            docs_url: "https://openrouter.ai/docs".into(),
            openai_compat: true,
        },
        ProviderPreset {
            id: "together".into(),
            name: "Together AI".into(),
            default_base_url: "https://api.together.xyz/v1".into(),
            default_model: "meta-llama/Llama-3-8b-chat-hf".into(),
            env_var: "TOGETHER_API_KEY".into(),
            docs_url: "https://docs.together.ai/".into(),
            openai_compat: true,
        },
        ProviderPreset {
            id: "mistral".into(),
            name: "Mistral AI".into(),
            default_base_url: "https://api.mistral.ai/v1".into(),
            default_model: "mistral-small-latest".into(),
            env_var: "MISTRAL_API_KEY".into(),
            docs_url: "https://docs.mistral.ai/".into(),
            openai_compat: true,
        },
        ProviderPreset {
            id: "perplexity".into(),
            name: "Perplexity".into(),
            default_base_url: "https://api.perplexity.ai".into(),
            default_model: "llama-3.1-sonar-small-128k-online".into(),
            env_var: "PERPLEXITY_API_KEY".into(),
            docs_url: "https://docs.perplexity.ai/".into(),
            openai_compat: true,
        },
        ProviderPreset {
            id: "hyperbolic".into(),
            name: "Hyperbolic".into(),
            default_base_url: "https://api.hyperbolic.xyz/v1".into(),
            default_model: "meta-llama/Meta-Llama-3.1-8B-Instruct".into(),
            env_var: "HYPERBOLIC_API_KEY".into(),
            docs_url: "https://docs.hyperbolic.xyz/".into(),
            openai_compat: true,
        },
        ProviderPreset {
            id: "custom".into(),
            name: "自定义 (OpenAI 兼容)".into(),
            default_base_url: String::new(),
            default_model: String::new(),
            env_var: String::new(),
            docs_url: String::new(),
            openai_compat: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_is_mock() {
        let c = AgentConfig::default();
        assert!(matches!(c.backend, BackendKind::Mock));
        assert!(!c.is_rig_ready());
        assert!(matches!(c.theme, ThemeMode::System));
        assert!(c.models.is_empty());
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
            provider_id: "openai".into(),
            enable_tools: false,
            theme: ThemeMode::Dark,
            models: vec![AvailableModel {
                id: "m1".into(),
                label: "我的 GPT".into(),
                provider_id: "openai".into(),
                base_url: "https://api.openai.com/v1".into(),
                model_name: "gpt-4o".into(),
                api_key: "sk-x".into(),
                preamble: "p".into(),
                enable_tools: true,
                created_at: 1000,
            }],
            active_model_id: Some("m1".into()),
        };
        let s = serde_json::to_string(&c).unwrap();
        let back: AgentConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c.api_key, back.api_key);
        assert_eq!(c.backend, back.backend);
        assert_eq!(c.theme, back.theme);
        assert_eq!(c.models.len(), back.models.len());
        assert_eq!(c.models[0].label, back.models[0].label);
        assert_eq!(c.active_model_id, back.active_model_id);
    }

    #[test]
    fn builtin_presets_contains_openai_and_custom() {
        let presets = builtin_presets();
        assert!(presets.iter().any(|p| p.id == "openai"));
        assert!(presets.iter().any(|p| p.id == "custom"));
        // 所有预设都应标记为 openai_compat（本期仅支持兼容协议）
        assert!(presets.iter().all(|p| p.openai_compat));
    }

    #[test]
    fn theme_mode_serde_lowercase() {
        let s = serde_json::to_string(&ThemeMode::Light).unwrap();
        assert_eq!(s, "\"light\"");
        let d: ThemeMode = serde_json::from_str("\"dark\"").unwrap();
        assert!(matches!(d, ThemeMode::Dark));
    }
}

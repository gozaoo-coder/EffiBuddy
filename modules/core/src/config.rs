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
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    /// 离线 mock，无网络
    #[default]
    Mock,
    /// 通过 rig 调用 OpenAI 兼容接口（统一走 Chat Completions API）
    Openai,
}

/// 模型能力类型：区分 LLM 对话 / 图像生成 / 视频生成 / 音频转文字
///
/// 切换激活模型时根据 kind 决定走哪个后端：
/// - Chat：走 RigAgent（Chat Completions API）
/// - ImageGen：走图像生成工具（OpenAI 兼容 /images/generations）
/// - VideoGen：预留，暂未实现
/// - AudioTranscribe：音频转文字模型（OpenAI 兼容 /audio/transcriptions）
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    /// LLM 对话模型（默认）
    #[default]
    Chat,
    /// 图像生成模型（如 DALL-E 3、SD、Flux）
    ImageGen,
    /// 视频生成模型（预留，暂未实现）
    VideoGen,
    /// 音频转文字模型（如 whisper-1，OpenAI 兼容 /audio/transcriptions）
    AudioTranscribe,
}

/// 主题模式：系统 / 亮色 / 暗色
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// 跟随系统 prefers-color-scheme
    #[default]
    System,
    /// 强制亮色
    Light,
    /// 强制暗色
    Dark,
}

/// ASR 服务商
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrProvider {
    VolcEngine,
    Qwen,
}

/// ASR 配置：语音转写服务凭证与行为参数
///
/// 字段按大小降序排列以最小化 padding：
/// String（24B）→ Option<String>（24B）→ AsrProvider（1B）→ bool（1B）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrConfig {
    pub volc_app_id: String,
    pub volc_access_token: String,
    pub volc_cluster: String,
    pub qwen_api_key: String,
    pub qwen_base_url: String,
    pub qwen_audio_model: String,
    pub default_language: String,
    /// 摘要用 LLM 模型名；None 表示使用当前激活的对话模型
    pub summary_model: Option<String>,
    pub provider: AsrProvider,
    /// 转写完成后自动生成摘要
    pub enable_auto_summary: bool,
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            volc_app_id: String::new(),
            volc_access_token: String::new(),
            volc_cluster: "volcengine_streaming_common".to_string(),
            qwen_api_key: String::new(),
            qwen_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
            qwen_audio_model: "qwen-audio-asr".to_string(),
            default_language: "zh-CN".to_string(),
            summary_model: None,
            provider: AsrProvider::VolcEngine,
            enable_auto_summary: true,
        }
    }
}

/// Agent 配置（可被前端修改并持久化）
///
/// 同时承载：当前激活的运行时配置 + 可使用模型列表 + 主题 + ASR 配置 + 服务角色映射。
///
/// 服务角色映射（service roles）：
/// - `active_model_id`：聊天模型（主对话 agent）。向后兼容字段，等同于"聊天模型"角色。
/// - `active_image_gen_model_id`：默认生图模型。向后兼容字段。
/// - `title_model_id`：对话命名模型（auto_classify 用）。None 时回退到 active_model_id。
/// - `compression_model_id`：会话历史压缩模型。None 时回退到 active_model_id。
/// - `asr_stream_model_id`：语音实时转文字模型。None 时回退到 asr_config 原生配置。
/// - `asr_transcribe_model_id`：音频转文字模型（文件转写）。None 时回退到 asr_config 原生配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub api_key: String,
    pub base_url: String,
    pub model_name: String,
    pub preamble: String,
    /// 当前激活配置所属的 provider id（"openai" / "deepseek" / "custom" ...）
    pub provider_id: String,
    pub models: Vec<AvailableModel>,
    /// 当前激活的对话模型 id（指向 models 中 kind=Chat 的一项）；None 表示使用内联配置
    pub active_model_id: Option<String>,
    /// 当前激活的图像生成模型 id（指向 models 中 kind=ImageGen 的一项）；
    /// None 表示未配置图像生成能力，image_gen 工具调用会返回错误提示。
    /// 与 active_model_id 独立：用户可同时激活一个对话模型和一个图像生成模型。
    #[serde(default)]
    pub active_image_gen_model_id: Option<String>,
    /// 对话命名模型 id（auto_classify 用）。None 时回退到 active_model_id。
    /// 独立配置时，归类命名走此模型，避免占用主对话模型的上下文窗口。
    #[serde(default)]
    pub title_model_id: Option<String>,
    /// 会话历史压缩模型 id（compress_messages 用）。None 时回退到 active_model_id。
    /// 独立配置时，压缩走此模型，可选用擅长长文本处理的高性价比模型。
    #[serde(default)]
    pub compression_model_id: Option<String>,
    /// 语音实时转文字模型 id（指向 models 中 kind=AudioTranscribe 的一项）。
    /// None 时回退到 asr_config 原生配置（volcengine/qwen 专用协议）。
    #[serde(default)]
    pub asr_stream_model_id: Option<String>,
    /// 音频转文字模型 id（文件转写，指向 models 中 kind=AudioTranscribe 的一项）。
    /// None 时回退到 asr_config 原生配置（volcengine/qwen 专用协议）。
    #[serde(default)]
    pub asr_transcribe_model_id: Option<String>,
    /// ASR（语音转写）配置
    #[serde(default)]
    pub asr_config: AsrConfig,
    /// 会话历史压缩机制设置（阈值自动压缩 / 工具调用压缩 / 逐句压缩）
    #[serde(default)]
    pub compression_settings: CompressionSettings,
    pub backend: BackendKind,
    /// 是否启用工具调用（RAG 索引等）
    pub enable_tools: bool,
    pub theme: ThemeMode,
}

/// 会话历史压缩机制设置
///
/// 控制压缩触发的阈值、自动压缩开关，以及压缩时对工具调用 / 逐句对话的处理方式。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionSettings {
    /// 自动压缩阈值（百分比 1-100）：上下文使用达到该比例时自动触发压缩
    #[serde(default = "default_compress_threshold")]
    pub threshold_percent: u32,
    /// 是否启用自动压缩（达到阈值时在回复完成后自动压缩历史）
    #[serde(default = "default_true")]
    pub auto_compress: bool,
    /// 是否压缩工具调用 / 工具返回（长会话中工具调用占大量 token）
    #[serde(default = "default_true")]
    pub compress_tool_calls: bool,
    /// 是否逐句对话压缩（按消息粒度 Keep/Hide/Replace 精简长句）
    #[serde(default = "default_true")]
    pub compress_sentences: bool,
}

fn default_compress_threshold() -> u32 {
    80
}
fn default_true() -> bool {
    true
}

impl Default for CompressionSettings {
    fn default() -> Self {
        Self {
            threshold_percent: default_compress_threshold(),
            auto_compress: true,
            compress_tool_calls: true,
            compress_sentences: true,
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: String::new(),
            model_name: "gpt-4o-mini".to_string(),
            preamble: "你是 EffiSuite 的 AI 助手。遵守以下准则：\n【回答】简短直接，不重复用户问题，不堆砌铺垫与废话。\n【执行】调用工具前，先用 1-3 句话简述最优实现路径（含关键步骤/文件/技术选型），再发起工具调用。\n【澄清】当任务目标不明确、用户意图模糊、或修改方向过多（≥2 个互斥方向）时，必须先调用 ask_user 工具向用户提问澄清，而非猜测执行。选项要具体可执行，包含明确取舍维度。用户明确后再行动，避免连环追问。\n【原则】优先最短路径，避免试错；工具失败时给出明确下一步，不空谈。".to_string(),
            provider_id: "openai".to_string(),
            models: Vec::new(),
            active_model_id: None,
            active_image_gen_model_id: None,
            title_model_id: None,
            compression_model_id: None,
            asr_stream_model_id: None,
            asr_transcribe_model_id: None,
            asr_config: AsrConfig::default(),
            compression_settings: CompressionSettings::default(),
            backend: BackendKind::Mock,
            enable_tools: true,
            theme: ThemeMode::System,
      }
  }
}


impl AgentConfig {
    /// 判断当前配置是否可启动 RigAgent（backend=openai 且有 api_key）
    #[inline]
    pub fn is_rig_ready(&self) -> bool {
        matches!(self.backend, BackendKind::Openai) && !self.api_key.trim().is_empty()
    }

    /// 解析对话命名模型：优先 title_model_id，回退到 active_model_id，再回退到内联配置。
    /// 返回 (api_key, base_url, model_name) 三元组，供 call_auto_classify_agent 使用。
    pub fn resolve_title_model(&self) -> Option<(String, String, String)> {
        let m = self
            .title_model_id
            .as_deref()
            .and_then(|id| self.models.iter().find(|m| m.id == id))
            .or_else(|| {
                self.active_model_id
                    .as_deref()
                    .and_then(|id| self.models.iter().find(|m| m.id == id))
            });
        if let Some(m) = m {
            return Some((m.api_key.clone(), m.base_url.clone(), m.model_name.clone()));
        }
        // 无激活模型时回退到运行时内联配置
        Some((self.api_key.clone(), self.base_url.clone(), self.model_name.clone()))
    }

    /// 解析会话历史压缩模型：优先 compression_model_id，回退到 active_model_id，再回退到内联配置。
    pub fn resolve_compression_model(&self) -> Option<(String, String, String)> {
        let m = self
            .compression_model_id
            .as_deref()
            .and_then(|id| self.models.iter().find(|m| m.id == id))
            .or_else(|| {
                self.active_model_id
                    .as_deref()
                    .and_then(|id| self.models.iter().find(|m| m.id == id))
            });
        if let Some(m) = m {
            return Some((m.api_key.clone(), m.base_url.clone(), m.model_name.clone()));
        }
        Some((self.api_key.clone(), self.base_url.clone(), self.model_name.clone()))
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
    /// 图像生成专用：默认尺寸（如 "1024x1024"），仅 kind=ImageGen 时有效
    #[serde(default)]
    pub image_size: Option<String>,
    /// 图像生成专用：默认质量（如 "standard"/"hd"），仅 kind=ImageGen 时有效
    #[serde(default)]
    pub image_quality: Option<String>,
    /// 视频生成专用：默认分辨率（如 "720p"），仅 kind=VideoGen 时有效
    #[serde(default)]
    pub video_resolution: Option<String>,
    /// 视频生成专用：默认宽高比（如 "16:9"），仅 kind=VideoGen 时有效
    #[serde(default)]
    pub video_ratio: Option<String>,
    /// 音频转文字专用：默认源语言（如 "zh"/"en"/"auto"），仅 kind=AudioTranscribe 时有效
    #[serde(default)]
    pub audio_language: Option<String>,
    /// 计费单价（元/百万 tokens），None 表示未配置价格（聊天中不显示消费金额）
    #[serde(default)]
    pub pricing: Option<ModelPricing>,
    /// 模型上下文窗口大小（tokens），供前端显示剩余上下文
    #[serde(default)]
    pub context_window_tokens: Option<u32>,
    /// 视频生成专用：默认时长（秒，2..=15；None 用模型默认），仅 kind=VideoGen 时有效
    #[serde(default)]
    pub video_duration: Option<u32>,
    pub created_at: u64,
    pub enable_tools: bool,
    /// 模型能力类型：Chat（对话）/ ImageGen（图像生成）/ VideoGen（视频生成，预留）
    /// 旧配置无此字段时默认为 Chat（向后兼容）
    #[serde(default)]
    pub kind: ModelKind,
}

/// 模型计费单价（元/百万 tokens）
///
/// 对应各 provider 文档中的计费规则，用户在模型配置面板自行填写，不硬编码：
/// - 缓存命中输入（如 DeepSeek 的 prompt_cache_hit_tokens）
/// - 缓存未命中输入（如 DeepSeek 的 prompt_cache_miss_tokens）
/// - 输出
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ModelPricing {
    /// 缓存命中输入单价（元/百万 tokens）
    pub cache_hit_per_m: f64,
    /// 缓存未命中输入单价（元/百万 tokens）
    pub cache_miss_per_m: f64,
    /// 输出单价（元/百万 tokens）
    pub output_per_m: f64,
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
                kind: ModelKind::Chat,
                image_size: None,
                image_quality: None,
                video_resolution: None,
                video_ratio: None,
                audio_language: None,
                context_window_tokens: Some(128000),
                video_duration: None,
                pricing: None,
                created_at: 1000,
            }],
            active_model_id: Some("m1".into()),
            active_image_gen_model_id: None,
            title_model_id: Some("m1".into()),
            compression_model_id: None,
            asr_stream_model_id: None,
            asr_transcribe_model_id: None,
            asr_config: AsrConfig::default(),
            compression_settings: CompressionSettings::default(),
        };
        let s = serde_json::to_string(&c).unwrap();
        let back: AgentConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c.api_key, back.api_key);
        assert_eq!(c.backend, back.backend);
        assert_eq!(c.theme, back.theme);
        assert_eq!(c.models.len(), back.models.len());
        assert_eq!(c.models[0].label, back.models[0].label);
        assert_eq!(c.active_model_id, back.active_model_id);
        assert_eq!(back.title_model_id, Some("m1".to_string()));
        assert_eq!(back.compression_model_id, None);
        assert_eq!(back.asr_stream_model_id, None);
        assert_eq!(back.asr_transcribe_model_id, None);
        assert_eq!(back.asr_config.provider, AsrProvider::VolcEngine);
        assert!(back.asr_config.enable_auto_summary);
    }

    #[test]
    fn config_resolve_title_model_fallback() {
        // 无 title_model_id 时回退到 active_model_id
        let mut c = AgentConfig::default();
        c.backend = BackendKind::Openai;
        c.api_key = "inline-key".into();
        c.base_url = "https://inline/v1".into();
        c.model_name = "inline-model".into();
        // 无任何模型时回退到内联配置
        let (k, b, m) = c.resolve_title_model().unwrap();
        assert_eq!(k, "inline-key");
        assert_eq!(b, "https://inline/v1");
        assert_eq!(m, "inline-model");
        // 设置 active_model_id 后回退到该模型
        c.models.push(AvailableModel {
            id: "chat1".into(),
            label: "Chat".into(),
            provider_id: "openai".into(),
            base_url: "https://api.openai.com/v1".into(),
            model_name: "gpt-4o-mini".into(),
            api_key: "sk-active".into(),
            preamble: String::new(),
            enable_tools: true,
            kind: ModelKind::Chat,
            image_size: None,
            image_quality: None,
            video_resolution: None,
            video_ratio: None,
            audio_language: None,
            context_window_tokens: Some(128000),
            video_duration: None,
            pricing: None,
            created_at: 1000,
        });
        c.active_model_id = Some("chat1".into());
        let (k, _, m) = c.resolve_title_model().unwrap();
        assert_eq!(k, "sk-active");
        assert_eq!(m, "gpt-4o-mini");
        // 设置 title_model_id 后优先使用
        c.models.push(AvailableModel {
            id: "title1".into(),
            label: "Title".into(),
            provider_id: "deepseek".into(),
            base_url: "https://api.deepseek.com".into(),
            model_name: "deepseek-chat".into(),
            api_key: "sk-title".into(),
            preamble: String::new(),
            enable_tools: false,
            kind: ModelKind::Chat,
            image_size: None,
            image_quality: None,
            video_resolution: None,
            video_ratio: None,
            audio_language: None,
            context_window_tokens: Some(64000),
            video_duration: None,
            pricing: None,
            created_at: 1001,
        });
        c.title_model_id = Some("title1".into());
        let (k, _, m) = c.resolve_title_model().unwrap();
        assert_eq!(k, "sk-title");
        assert_eq!(m, "deepseek-chat");
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

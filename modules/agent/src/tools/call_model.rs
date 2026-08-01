//! call_model 工具：让 agent 一次性调用任意已保存模型（或当前模型）回答问题
//!
//! 典型用途：
//! - 跨模型交叉验证 / 征求意见（如"让另一个模型评审这段代码"）
//! - 调用专用小模型做分类、摘要、翻译等子任务
//! - 在当前模型不可用时，临时换一个模型完成请求
//!
//! 与 sub_agent 的区别：call_model 是**单轮无工具**调用，直接返回文本；
//! sub_agent 是有独立会话历史、可多轮、可带工具的子 agent。
//!
//! 模型解析优先级：model_id 指定的已保存模型 > 当前激活对话模型 > 运行时内联配置。

use std::sync::Arc;

use effisuite_core::{AgentConfig, ModelKind};
use rig_core::client::CompletionClient;
use rig_core::completion::Prompt;
use rig_core::providers::openai;
use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::sync::RwLock;

/// 返回文本最大长度（超出截断，避免撑爆主上下文）
const MAX_OUTPUT_CHARS: usize = 32 * 1024;

/// 工具参数
#[derive(Deserialize)]
pub struct CallModelArgs {
    /// 发送给目标模型的提示词
    pub prompt: String,
    /// 目标模型 id（模型列表中的 id）；缺省用当前激活对话模型
    #[serde(default)]
    pub model_id: Option<String>,
    /// 可选的系统提示词（不传则无 system 消息）
    #[serde(default)]
    pub system: Option<String>,
    /// 采样温度 0-2，缺省用模型默认
    #[serde(default)]
    pub temperature: Option<f64>,
    /// 最大输出 token 数，缺省用模型默认
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("call_model error: {0}")]
pub struct CallModelError(String);

/// 单次调用目标模型的解析结果
pub(crate) struct ResolvedModel {
    api_key: String,
    base_url: String,
    model_name: String,
}

/// 模型调用工具
pub struct CallModelTool {
    config: Arc<RwLock<Arc<AgentConfig>>>,
}

impl CallModelTool {
    pub fn new(config: Arc<RwLock<Arc<AgentConfig>>>) -> Self {
        Self { config }
    }
}

impl Tool for CallModelTool {
    const NAME: &'static str = "call_model";

    type Error = CallModelError;
    type Args = CallModelArgs;
    type Output = String;

    fn description(&self) -> String {
        "一次性调用一个模型回答问题（无工具、单轮），返回该模型的文本回复。\
         用于跨模型征求意见 / 让其他模型执行子任务 / 模型间交叉验证。\
         model_id 可指定模型列表中的任意对话模型；不传则用当前对话模型。\
         需要多轮对话或让子模型使用工具时，请改用 sub_agent。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "发送给目标模型的提示词"
                },
                "model_id": {
                    "type": "string",
                    "description": "目标模型 id（manage_model list 可查看）；缺省用当前激活对话模型"
                },
                "system": {
                    "type": "string",
                    "description": "可选系统提示词"
                },
                "temperature": {
                    "type": "number",
                    "description": "采样温度 0-2，缺省用模型默认"
                },
                "max_tokens": {
                    "type": "integer",
                    "description": "最大输出 token 数，缺省用模型默认"
                }
            },
            "required": ["prompt"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let resolved = self.resolve_model(args.model_id.as_deref()).await?;
        let reply = call_model_once(
            &resolved,
            args.system.as_deref(),
            &args.prompt,
            args.temperature,
            args.max_tokens,
        )
        .await?;

        let truncated = if reply.chars().count() > MAX_OUTPUT_CHARS {
            let cut: String = reply.chars().take(MAX_OUTPUT_CHARS).collect();
            format!("{cut}\n...[回复过长，已截断]")
        } else {
            reply
        };
        Ok(format!(
            "[模型 {} 的回复]\n{}",
            resolved.model_name, truncated
        ))
    }
}

impl CallModelTool {
    /// 解析目标模型配置：model_id > 当前激活对话模型 > 运行时内联配置
    async fn resolve_model(&self, model_id: Option<&str>) -> Result<ResolvedModel, CallModelError> {
        let config = self.config.read().await;
        if let Some(id) = model_id {
            let m = config
                .models
                .iter()
                .find(|m| m.id == id)
                .ok_or_else(|| {
                    let available: Vec<&str> =
                        config.models.iter().map(|m| m.id.as_str()).collect();
                    CallModelError(format!(
                        "模型 {id} 不存在。可用模型 id：{}（manage_model list 可查看）",
                        if available.is_empty() {
                            "（无）".to_string()
                        } else {
                            available.join(", ")
                        }
                    ))
                })?;
            if m.kind != ModelKind::Chat {
                return Err(CallModelError(format!(
                    "模型 {id} 是 {} 模型，不能用于对话调用，请指定 kind=chat 的模型",
                    match m.kind {
                        ModelKind::ImageGen => "image_gen（图像生成）",
                        ModelKind::VideoGen => "video_gen（视频生成）",
                        ModelKind::Chat => "chat",
                    }
                )));
            }
            if m.api_key.trim().is_empty() {
                return Err(CallModelError(format!("模型 {id} 未配置 api_key，无法调用")));
            }
            return Ok(ResolvedModel {
                api_key: m.api_key.clone(),
                base_url: m.base_url.clone(),
                model_name: m.model_name.clone(),
            });
        }

        // 缺省：当前激活对话模型 > 运行时内联配置
        if let Some(active_id) = config.active_model_id.as_ref() {
            if let Some(m) = config.models.iter().find(|m| &m.id == active_id) {
                if !m.api_key.trim().is_empty() {
                    return Ok(ResolvedModel {
                        api_key: m.api_key.clone(),
                        base_url: m.base_url.clone(),
                        model_name: m.model_name.clone(),
                    });
                }
            }
        }
        if config.api_key.trim().is_empty() {
            return Err(CallModelError(
                "未配置任何可用的对话模型（api_key 为空）。请先调用 manage_model save 添加模型。"
                    .into(),
            ));
        }
        Ok(ResolvedModel {
            api_key: config.api_key.clone(),
            base_url: config.base_url.clone(),
            model_name: config.model_name.clone(),
        })
    }
}

/// 一次性调用 OpenAI 兼容模型（无工具、单轮），供 call_model 与子 agent 复用
pub(crate) async fn call_model_once(
    model: &ResolvedModel,
    system: Option<&str>,
    prompt: &str,
    temperature: Option<f64>,
    max_tokens: Option<u32>,
) -> Result<String, CallModelError> {
    let mut builder = openai::CompletionsClient::builder().api_key(&model.api_key);
    if !model.base_url.trim().is_empty() {
        builder = builder.base_url(&model.base_url);
    }
    let client = builder
        .build()
        .map_err(|e| CallModelError(format!("模型客户端构造失败: {e}")))?;

    let mut agent_builder = client.agent(&model.model_name);
    match system.map(str::trim) {
        Some(s) if !s.is_empty() => {
            agent_builder = agent_builder.preamble(s);
        }
        _ => {
            agent_builder = agent_builder.without_preamble();
        }
    }
    if let Some(t) = temperature {
        agent_builder = agent_builder.temperature(t.clamp(0.0, 2.0));
    }
    if let Some(mt) = max_tokens {
        agent_builder = agent_builder.max_tokens(mt.clamp(1, 128_000) as u64);
    }
    let agent = agent_builder.build();
    agent
        .prompt(prompt)
        .await
        .map_err(|e| CallModelError(format!("模型 {} 调用失败: {e}", model.model_name)))
}

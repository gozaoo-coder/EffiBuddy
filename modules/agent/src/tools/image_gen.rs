//! image_gen 工具：让 LLM 调用图像生成 API 为用户生成图片
//!
//! 走 OpenAI 兼容协议：POST {base_url}/images/generations
//! 支持 base64（response_format=b64_json）与 URL 两种返回格式，
//! 统一保存到 attachments 目录，返回图片路径供前端显示。
//!
//! 工具持有图像生成模型的配置（api_key/base_url/model/size/quality），
//! 由 Tauri 命令层在 build_agent 时注入。

use std::path::PathBuf;
use std::sync::Arc;

use effisuite_core::{AgentConfig, ModelKind};
use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::sync::RwLock;

/// 图像生成模型配置快照
///
/// 字段按大小降序：String（24B）在 Option<String>（16B）前。
#[derive(Clone)]
pub struct ImageGenConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub default_size: Option<String>,
    pub default_quality: Option<String>,
}

/// 工具参数
#[derive(Deserialize)]
pub struct ImageGenArgs {
    /// 图像生成提示词（英文效果通常更好，LLM 可自行翻译）
    pub prompt: String,
    /// 图片尺寸，如 "1024x1024"/"1792x1024"/"1024x1792"。留空用模型默认
    #[serde(default)]
    pub size: Option<String>,
    /// 质量：standard / hd（部分模型支持）。留空用模型默认
    #[serde(default)]
    pub quality: Option<String>,
    /// 生成数量，默认 1
    #[serde(default)]
    pub n: Option<u32>,
    /// 指定使用的图像模型 id（模型列表中的 kind=image_gen 模型）；
    /// 缺省用当前激活的图像生成模型
    #[serde(default)]
    pub model_id: Option<String>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("image_gen error: {0}")]
pub struct ImageGenError(String);

/// 工具输出：返回保存后的附件信息，供前端渲染图片
#[derive(serde::Serialize)]
pub struct ImageGenOutput {
    /// 附件 id（同时作为文件名前缀）
    pub id: String,
    /// 相对 attachments 目录的文件名
    pub path: String,
    /// 显示用文件名
    pub name: String,
    /// 生成耗时毫秒
    pub elapsed_ms: u64,
}

/// 图像生成工具
///
/// `config` 为共享句柄：Tauri 命令层在 set_active_model 时更新，
/// 工具调用时读取最新配置。None 时工具不可用（返回错误）。
/// `models` 为可选的模型列表句柄：args.model_id 指定模型时据此解析。
pub struct ImageGenTool {
    config: Arc<RwLock<Option<ImageGenConfig>>>,
    /// 模型列表共享句柄（可选）：支持按 model_id 指定图像模型
    models: Option<Arc<RwLock<AgentConfig>>>,
    /// attachments 目录绝对路径，图片保存到此
    attachments_dir: PathBuf,
}

impl ImageGenTool {
    pub fn new(config: Arc<RwLock<Option<ImageGenConfig>>>, attachments_dir: PathBuf) -> Self {
        Self {
            config,
            models: None,
            attachments_dir,
        }
    }

    /// 附加模型列表句柄：启用 model_id 指定图像模型能力
    pub fn with_models(mut self, models: Arc<RwLock<AgentConfig>>) -> Self {
        self.models = Some(models);
        self
    }

    /// 解析本次调用的图像模型配置：model_id 指定 > 当前激活配置
    async fn resolve_config(
        &self,
        model_id: Option<&str>,
    ) -> Result<ImageGenConfig, ImageGenError> {
        if let Some(id) = model_id {
            let models = self.models.as_ref().ok_or_else(|| {
                ImageGenError("未启用模型列表句柄，无法按 model_id 指定图像模型".into())
            })?;
            let config = models.read().await;
            let m = config
                .models
                .iter()
                .find(|m| m.id == id)
                .ok_or_else(|| ImageGenError(format!("模型 {id} 不存在（manage_model list 可查看）")))?;
            if m.kind != ModelKind::ImageGen {
                return Err(ImageGenError(format!(
                    "模型 {id} 不是图像生成模型（kind 应为 image_gen）"
                )));
            }
            if m.api_key.trim().is_empty() {
                return Err(ImageGenError(format!("模型 {id} 未配置 api_key")));
            }
            return Ok(ImageGenConfig {
                api_key: m.api_key.clone(),
                base_url: m.base_url.clone(),
                model: m.model_name.clone(),
                default_size: m.image_size.clone(),
                default_quality: m.image_quality.clone(),
            });
        }
        self.config
            .read()
            .await
            .clone()
            .ok_or_else(|| {
                ImageGenError("未配置图像生成模型，请先在设置中添加 kind=image_gen 的模型并激活".into())
            })
    }

    /// 便捷生成方法：绕过 rig Tool trait，供 Tauri 命令层直接调用。
    ///
    /// 与 `Tool::call` 行为一致，但无需引入 rig_core 依赖。
    pub async fn generate(
        &self,
        prompt: String,
        size: Option<String>,
        quality: Option<String>,
    ) -> Result<ImageGenOutput, ImageGenError> {
        // 复用 Tool::call 实现，避免逻辑重复
        use rig_core::tool::Tool;
        self.call(ImageGenArgs {
            prompt,
            size,
            quality,
            n: Some(1),
            model_id: None,
        })
        .await
    }
}

/// OpenAI Images API 响应
#[derive(Deserialize)]
struct ImagesResponse {
    data: Vec<ImageData>,
}

#[derive(Deserialize)]
struct ImageData {
    /// response_format=b64_json 时返回 base64 字符串
    b64_json: Option<String>,
    /// response_format=url 时返回图片 URL
    url: Option<String>,
}

impl Tool for ImageGenTool {
    const NAME: &'static str = "image_gen";

    type Error = ImageGenError;
    type Args = ImageGenArgs;
    type Output = ImageGenOutput;

    fn description(&self) -> String {
        "调用图像生成模型为用户生成图片。支持英文/中文提示词，\
         生成后图片会自动保存并显示给用户。\
         提示词越具体效果越好，建议包含风格、构图、光影等描述。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "图像生成提示词，建议详细描述风格、构图、主体、光影等"
                },
                "size": {
                    "type": "string",
                    "description": "图片尺寸，如 1024x1024 / 1792x1024 / 1024x1792，留空用默认",
                    "enum": ["1024x1024", "1792x1024", "1024x1792"]
                },
                "quality": {
                    "type": "string",
                    "description": "质量：standard 或 hd（部分模型支持），留空用默认",
                    "enum": ["standard", "hd"]
                },
                "n": {
                    "type": "integer",
                    "description": "生成数量，默认 1",
                    "default": 1
                },
                "model_id": {
                    "type": "string",
                    "description": "指定图像模型 id（模型列表中 kind=image_gen 的模型）；缺省用当前激活图像模型"
                }
            },
            "required": ["prompt"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let started = std::time::Instant::now();
        let n = args.n.unwrap_or(1).clamp(1, 4);

        let cfg = self.resolve_config(args.model_id.as_deref()).await?;

        // 确保 attachments 目录存在
        tokio::fs::create_dir_all(&self.attachments_dir)
            .await
            .map_err(|e| ImageGenError(format!("创建 attachments 目录失败: {e}")))?;

        // 构造请求体：优先用 b64_json 以便直接落盘，避免再下载 URL
        let size = args
            .size
            .or(cfg.default_size.clone())
            .unwrap_or_else(|| "1024x1024".to_string());
        let quality = args.quality.or(cfg.default_quality.clone());
        let mut body = serde_json::json!({
            "model": cfg.model,
            "prompt": args.prompt,
            "n": n,
            "size": size,
            "response_format": "b64_json",
        });
        if let Some(q) = quality {
            body["quality"] = serde_json::Value::String(q);
        }

        // 拼接端点：base_url 可能以 /v1 结尾或不含，统一处理
        let endpoint = build_images_endpoint(&cfg.base_url);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| ImageGenError(format!("HTTP 客户端构造失败: {e}")))?;

        let resp = client
            .post(&endpoint)
            .bearer_auth(&cfg.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ImageGenError(format!("请求图像生成 API 失败: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ImageGenError(format!(
                "图像生成 API 返回错误 {}: {}",
                status,
                truncate(&text, 500)
            )));
        }

        let resp_data: ImagesResponse = resp
            .json()
            .await
            .map_err(|e| ImageGenError(format!("解析图像生成响应失败: {e}")))?;

        let first = resp_data
            .data
            .into_iter()
            .next()
            .ok_or_else(|| ImageGenError("响应无图片数据".into()))?;

        let id = uuid::Uuid::new_v4().to_string();
        let filename = format!("gen_{}.png", &id);
        let filepath = self.attachments_dir.join(&filename);

        // 优先 b64_json，其次下载 URL
        if let Some(b64) = first.b64_json {
            let bytes = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                b64.as_bytes(),
            )
            .map_err(|e| ImageGenError(format!("base64 解码失败: {e}")))?;
            tokio::fs::write(&filepath, &bytes)
                .await
                .map_err(|e| ImageGenError(format!("保存图片失败: {e}")))?;
        } else if let Some(url) = first.url {
            let bytes = client
                .get(&url)
                .send()
                .await
                .map_err(|e| ImageGenError(format!("下载图片 URL 失败: {e}")))?
                .bytes()
                .await
                .map_err(|e| ImageGenError(format!("读取图片字节失败: {e}")))?;
            tokio::fs::write(&filepath, &bytes)
                .await
                .map_err(|e| ImageGenError(format!("保存图片失败: {e}")))?;
        } else {
            return Err(ImageGenError("响应既无 b64_json 也无 url".into()));
        }

        let elapsed_ms = started.elapsed().as_millis() as u64;
        // 先借用 id 生成 name，再 move id 到结构体，避免 use after move
        let name = format!("生成图片_{}.png", &id[..8]);
        Ok(ImageGenOutput {
            id,
            path: filename,
            name,
            elapsed_ms,
        })
    }
}

/// 拼接图像生成端点：确保以 /v1/images/generations 结尾
///
/// OpenAI: https://api.openai.com/v1 → .../v1/images/generations
/// DeepSeek: https://api.deepseek.com → .../images/generations（DeepSeek 无 /v1）
/// 兼容用户填入的任意 base_url 形式
fn build_images_endpoint(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/images/generations")
    } else if base.ends_with("/v1/") {
        format!("{}/images/generations", base.trim_end_matches('/'))
    } else {
        format!("{base}/v1/images/generations")
    }
}

/// 截断字符串到 max 字符，避免错误信息过长
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{truncated}...")
    }
}

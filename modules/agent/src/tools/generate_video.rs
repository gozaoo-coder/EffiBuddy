//! generate_video 工具：让 LLM 调用视频生成 API 为用户生成视频
//!
//! 走 OpenAI 兼容协议：POST {base_url}/v1/videos/generations
//! 支持 text-to-video / image-to-video / video-to-video：
//! - image_paths / video_paths 数组中的本地文件会被读取并 base64 编码后随请求发送
//! - 响应优先取 url 字段（视频较大，URL 是主流返回方式），其次 b64_json
//! 视频下载后统一保存到 attachments 目录（或用户指定的 file_path）。
//!
//! 工具持有视频生成模型的配置（api_key/base_url/model），
//! 由 Tauri 命令层在 build_agent 时注入，set_active_model 时更新。
//!
//! 性能要点：
//! - 大文件下载用 `bytes()` 流式（reqwest 内部已流式缓冲）
//! - 视频生成耗时较长，统一 300 秒超时（用 `tokio::time::timeout` 包裹请求与下载）
//! - `Vec::with_capacity` 预分配 image/video 数组
//! - 不在锁内执行 I/O：`resolve_config` 仅持有读锁至 clone 完成
//! - `#[inline]` 标注小函数

use std::path::PathBuf;
use std::sync::Arc;

use effisuite_core::{AgentConfig, ModelKind};
use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::sync::RwLock;

/// 视频生成模型配置快照
///
/// 字段均为 String（24B），保持声明顺序即可，无 padding。
#[derive(Clone)]
pub struct VideoGenConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

/// 工具参数
///
/// 字段按大小降序：String(24) = Vec<String>(24) > Option<String>(16) > Option<i32>(8)。
#[derive(Deserialize)]
pub struct GenerateVideoArgs {
    /// 视频生成提示词（英文效果通常更好，LLM 可自行翻译）
    pub prompt: String,
    /// 目标保存路径（绝对路径）。留空则保存到 attachments 目录，文件名 video_{uuid}.mp4
    #[serde(default)]
    pub file_path: String,
    /// 参考图片路径数组（image-to-video），按顺序作为 image 1/N 引用
    #[serde(default)]
    pub image_paths: Vec<String>,
    /// 参考视频路径数组（video-to-video），按顺序作为 video 1/N 引用
    #[serde(default)]
    pub video_paths: Vec<String>,
    /// 分辨率：480p / 720p。留空用模型默认
    #[serde(default)]
    pub resolution: Option<String>,
    /// 宽高比：16:9 / 4:3 / 1:1 / 3:4 / 9:16 / 21:9 / adaptive
    #[serde(default)]
    pub ratio: Option<String>,
    /// 指定使用的视频模型 id（模型列表中 kind=video_gen 的模型）；
    /// 缺省用当前激活的视频生成模型
    #[serde(default)]
    pub model_id: Option<String>,
    /// 时长（秒）：-1=默认，或 2..=15
    #[serde(default)]
    pub duration: Option<i32>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("generate_video error: {0}")]
pub struct GenerateVideoError(String);

/// 工具输出：返回保存后的附件信息，供前端渲染视频
///
/// 字段按大小降序：String(24) > u64(8)。
#[derive(Debug, serde::Serialize)]
pub struct GenerateVideoOutput {
    /// 附件 id（同时作为文件名前缀）
    pub id: String,
    /// 相对 attachments 目录的文件名（或用户指定的绝对路径）
    pub path: String,
    /// 文件绝对路径，可直接用于 read_file/delete_file 等文件操作
    pub absolute_path: String,
    /// 显示用文件名
    pub name: String,
    /// 生成耗时毫秒
    pub elapsed_ms: u64,
}

/// 视频生成工具
///
/// `config` 为共享句柄：Tauri 命令层在 set_active_model 时更新，
/// 工具调用时读取最新配置。None 时工具不可用（返回错误）。
/// `models` 为可选的模型列表句柄：args.model_id 指定模型时据此解析。
pub struct GenerateVideoTool {
    config: Arc<RwLock<Option<VideoGenConfig>>>,
    /// 模型列表共享句柄（可选）：支持按 model_id 指定视频模型
    models: Option<Arc<RwLock<AgentConfig>>>,
    /// attachments 目录绝对路径，视频保存到此
    attachments_dir: PathBuf,
}

impl GenerateVideoTool {
    pub fn new(config: Arc<RwLock<Option<VideoGenConfig>>>, attachments_dir: PathBuf) -> Self {
        Self {
            config,
            models: None,
            attachments_dir,
        }
    }

    /// 附加模型列表句柄：启用 model_id 指定视频模型能力
    pub fn with_models(mut self, models: Arc<RwLock<AgentConfig>>) -> Self {
        self.models = Some(models);
        self
    }

    /// 解析本次调用的视频模型配置：model_id 指定 > 当前激活配置
    ///
    /// 锁临界区极短：仅持有读锁到 `clone()` 完成，立即释放。
    async fn resolve_config(
        &self,
        model_id: Option<&str>,
    ) -> Result<VideoGenConfig, GenerateVideoError> {
        if let Some(id) = model_id {
            let models = self.models.as_ref().ok_or_else(|| {
                GenerateVideoError("未启用模型列表句柄，无法按 model_id 指定视频模型".into())
            })?;
            let config = models.read().await;
            let m = config
                .models
                .iter()
                .find(|m| m.id == id)
                .ok_or_else(|| {
                    GenerateVideoError(format!("模型 {id} 不存在（manage_model list 可查看）"))
                })?;
            if m.kind != ModelKind::VideoGen {
                return Err(GenerateVideoError(format!(
                    "模型 {id} 不是视频生成模型（kind 应为 video_gen）"
                )));
            }
            if m.api_key.trim().is_empty() {
                return Err(GenerateVideoError(format!("模型 {id} 未配置 api_key")));
            }
            return Ok(VideoGenConfig {
                api_key: m.api_key.clone(),
                base_url: m.base_url.clone(),
                model: m.model_name.clone(),
            });
        }
        self.config.read().await.clone().ok_or_else(|| {
            GenerateVideoError(
                "未配置视频生成模型，请先在设置中添加 kind=video_gen 的模型并激活".into(),
            )
        })
    }

    /// 便捷生成方法：绕过 rig Tool trait，供 Tauri 命令层直接调用。
    ///
    /// 与 `Tool::call` 行为一致，但无需引入 rig_core 依赖。
    pub async fn generate(
        &self,
        prompt: String,
        file_path: String,
        image_paths: Vec<String>,
        video_paths: Vec<String>,
        resolution: Option<String>,
        ratio: Option<String>,
        duration: Option<i32>,
    ) -> Result<GenerateVideoOutput, GenerateVideoError> {
        // 复用 Tool::call 实现，避免逻辑重复
        use rig_core::tool::Tool;
        self.call(GenerateVideoArgs {
            prompt,
            file_path,
            image_paths,
            video_paths,
            resolution,
            ratio,
            model_id: None,
            duration,
        })
        .await
    }
}

/// OpenAI 兼容视频生成 API 响应
#[derive(Deserialize)]
struct VideosResponse {
    data: Vec<VideoData>,
}

#[derive(Deserialize)]
struct VideoData {
    /// 视频 URL（推荐路径，视频较大时主流返回方式）
    #[serde(default)]
    url: Option<String>,
    /// 视频 base64（部分模型支持，视频较大不推荐）
    #[serde(default)]
    b64_json: Option<String>,
}

impl Tool for GenerateVideoTool {
    const NAME: &'static str = "generate_video";

    type Error = GenerateVideoError;
    type Args = GenerateVideoArgs;
    type Output = GenerateVideoOutput;

    fn description(&self) -> String {
        "调用视频生成模型为用户生成视频。支持文本/图片/视频作为参考，\
         生成后视频会自动保存并显示给用户。\
         提示词越具体效果越好，建议包含主体动作、场景、镜头、风格等描述。\
         生成后文件保存到 attachments 目录，返回的 absolute_path 可直接用于 read_file/delete_file 等文件操作。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "视频生成提示词，建议详细描述主体动作、场景、镜头、风格等"
                },
                "file_path": {
                    "type": "string",
                    "description": "目标保存路径（绝对路径）。留空则保存到 attachments 目录，自动命名 video_{uuid}.mp4"
                },
                "image_paths": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "参考图片路径数组（image-to-video），按顺序作为 image 1/N 引用"
                },
                "video_paths": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "参考视频路径数组（video-to-video），按顺序作为 video 1/N 引用"
                },
                "resolution": {
                    "type": "string",
                    "description": "分辨率：480p 或 720p，留空用默认",
                    "enum": ["480p", "720p"]
                },
                "ratio": {
                    "type": "string",
                    "description": "宽高比，留空用默认",
                    "enum": ["16:9", "4:3", "1:1", "3:4", "9:16", "21:9", "adaptive"]
                },
                "duration": {
                    "type": "integer",
                    "description": "时长（秒）：-1=默认，或 2..=15",
                    "enum": [-1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
                },
                "model_id": {
                    "type": "string",
                    "description": "指定视频模型 id（模型列表中 kind=video_gen 的模型）；缺省用当前激活视频模型"
                }
            },
            "required": ["prompt"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let started = std::time::Instant::now();

        // 参数验证（早于配置读取，便于独立测试）
        validate_resolution(args.resolution.as_deref())?;
        validate_ratio(args.ratio.as_deref())?;
        validate_duration(args.duration)?;

        let cfg = self.resolve_config(args.model_id.as_deref()).await?;

        // 确保 attachments 目录存在
        tokio::fs::create_dir_all(&self.attachments_dir)
            .await
            .map_err(|e| GenerateVideoError(format!("创建 attachments 目录失败: {e}")))?;

        // 构造请求体
        let mut body = serde_json::json!({
            "model": cfg.model,
            "prompt": args.prompt,
        });
        if let Some(res) = args.resolution.as_deref() {
            body["resolution"] = serde_json::Value::String(res.to_string());
        }
        if let Some(r) = args.ratio.as_deref() {
            body["ratio"] = serde_json::Value::String(r.to_string());
        }
        if let Some(d) = args.duration {
            body["duration"] = serde_json::Value::from(d);
        }

        // 加载图片引用（base64 编码后随请求发送）
        if !args.image_paths.is_empty() {
            let images = encode_local_files(&args.image_paths, "图片").await?;
            body["image_paths"] = serde_json::Value::Array(images);
        }

        // 加载视频引用（base64 编码后随请求发送）
        if !args.video_paths.is_empty() {
            let videos = encode_local_files(&args.video_paths, "视频").await?;
            body["video_paths"] = serde_json::Value::Array(videos);
        }

        // 拼接端点：复用 image_gen 的 base_url 兼容模式
        let endpoint = build_videos_endpoint(&cfg.base_url);

        // 视频生成较慢：300 秒超时
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(|e| GenerateVideoError(format!("HTTP 客户端构造失败: {e}")))?;

        // 请求阶段：用 timeout 包裹，避免单次请求长时间阻塞
        let first = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            send_request(&client, &endpoint, &cfg.api_key, body),
        )
        .await
        .map_err(|_| GenerateVideoError("视频生成请求超时（300 秒）".into()))??;

        // 决定保存路径：用户指定 file_path 优先，否则 attachments_dir/video_{uuid}.mp4
        let id = uuid::Uuid::new_v4().to_string();
        let (filepath, filename, name) = resolve_save_target(
            &args.file_path,
            &self.attachments_dir,
            &id,
        );

        // 优先下载 URL（视频通常通过 URL 返回），其次 base64
        if let Some(url) = first.url {
            // 下载阶段同样 300 秒超时
            let bytes = tokio::time::timeout(
                std::time::Duration::from_secs(300),
                download_video(&client, &url),
            )
            .await
            .map_err(|_| GenerateVideoError("下载视频超时（300 秒）".into()))??;
            tokio::fs::write(&filepath, &bytes)
                .await
                .map_err(|e| GenerateVideoError(format!("保存视频失败: {e}")))?;
        } else if let Some(b64) = first.b64_json {
            let bytes = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                b64.as_bytes(),
            )
            .map_err(|e| GenerateVideoError(format!("base64 解码失败: {e}")))?;
            tokio::fs::write(&filepath, &bytes)
                .await
                .map_err(|e| GenerateVideoError(format!("保存视频失败: {e}")))?;
        } else {
            return Err(GenerateVideoError("响应既无 url 也无 b64_json".into()));
        }

        let elapsed_ms = started.elapsed().as_millis() as u64;
        // 绝对路径：保持原生分隔符（Windows 下 `\`，Unix 下 `/`），
        // read_file 等工具在 Windows 下对两种分隔符都兼容
        let absolute_path = filepath.to_string_lossy().into_owned();
        Ok(GenerateVideoOutput {
            id,
            path: filename,
            absolute_path,
            name,
            elapsed_ms,
        })
    }
}

/// 验证分辨率参数
#[inline]
fn validate_resolution(res: Option<&str>) -> Result<(), GenerateVideoError> {
    if let Some(r) = res {
        if !matches!(r, "480p" | "720p") {
            return Err(GenerateVideoError(format!(
                "不支持的分辨率 {r}（仅支持 480p / 720p）"
            )));
        }
    }
    Ok(())
}

/// 验证宽高比参数
#[inline]
fn validate_ratio(ratio: Option<&str>) -> Result<(), GenerateVideoError> {
    if let Some(r) = ratio {
        if !matches!(
            r,
            "16:9" | "4:3" | "1:1" | "3:4" | "9:16" | "21:9" | "adaptive"
        ) {
            return Err(GenerateVideoError(format!(
                "不支持的宽高比 {r}（仅支持 16:9 / 4:3 / 1:1 / 3:4 / 9:16 / 21:9 / adaptive）"
            )));
        }
    }
    Ok(())
}

/// 验证时长参数：-1=默认 或 2..=15
#[inline]
fn validate_duration(duration: Option<i32>) -> Result<(), GenerateVideoError> {
    if let Some(d) = duration {
        if d != -1 && !(2..=15).contains(&d) {
            return Err(GenerateVideoError(format!(
                "时长 {d} 越界（仅支持 -1=默认 或 2..=15）"
            )));
        }
    }
    Ok(())
}

/// 读取本地文件并 base64 编码为 JSON 数组元素
///
/// 使用 `with_capacity` 预分配，避免多次 push 触发扩容。
async fn encode_local_files(
    paths: &[String],
    kind_label: &str,
) -> Result<Vec<serde_json::Value>, GenerateVideoError> {
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let bytes = tokio::fs::read(p)
            .await
            .map_err(|e| GenerateVideoError(format!("读取{kind_label} {p} 失败: {e}")))?;
        let b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
        out.push(serde_json::json!({
            "path": p,
            "data": b64,
        }));
    }
    Ok(out)
}

/// 发送视频生成请求并解析响应
async fn send_request(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
    body: serde_json::Value,
) -> Result<VideoData, GenerateVideoError> {
    let resp = client
        .post(endpoint)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| GenerateVideoError(format!("请求视频生成 API 失败: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(GenerateVideoError(format!(
            "视频生成 API 返回错误 {}: {}",
            status,
            truncate(&text, 500)
        )));
    }

    let resp_data: VideosResponse = resp
        .json()
        .await
        .map_err(|e| GenerateVideoError(format!("解析视频生成响应失败: {e}")))?;

    resp_data
        .data
        .into_iter()
        .next()
        .ok_or_else(|| GenerateVideoError("响应无视频数据".into()))
}

/// 流式下载视频字节
///
/// reqwest 的 `bytes()` 内部已对响应体做流式缓冲，避免一次性加载到内存。
/// 返回 `Vec<u8>` 以避免引入 `bytes` crate 直接依赖（额外一次拷贝，
/// 对几 MB 的视频文件开销可忽略）。
async fn download_video(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<u8>, GenerateVideoError> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| GenerateVideoError(format!("下载视频 URL 失败: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(GenerateVideoError(format!(
            "下载视频 URL 返回错误 {}: {}",
            status,
            truncate(&text, 500)
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| GenerateVideoError(format!("读取视频字节失败: {e}")))?;
    Ok(bytes.to_vec())
}

/// 决定视频保存目标：用户指定 file_path 优先（无扩展名补 .mp4），
/// 否则保存到 attachments 目录，文件名 video_{uuid}.mp4
///
/// 返回 (绝对路径, 输出 path 字段值, 显示用 name)
fn resolve_save_target(
    file_path: &str,
    attachments_dir: &std::path::Path,
    id: &str,
) -> (PathBuf, String, String) {
    // id 前 8 字符作为显示名后缀（UUID 通常 36 字符，但测试可能用短 id）
    // 用 get(..8) 安全切片，不足 8 字符时取全部，避免 panic
    let id_prefix = id.get(..8).unwrap_or(id);
    if !file_path.is_empty() {
        let p = ensure_mp4_extension(PathBuf::from(file_path));
        let filename = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("video_{id}.mp4"));
        let name = p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("生成视频_{id_prefix}"));
        (p, filename, name)
    } else {
        let filename = format!("video_{id}.mp4");
        let p = attachments_dir.join(&filename);
        let name = format!("生成视频_{id_prefix}");
        (p, filename, name)
    }
}

/// 补全 .mp4 扩展名：路径无扩展名时追加 .mp4
#[inline]
fn ensure_mp4_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        let mut s = path.into_os_string();
        s.push(".mp4");
        PathBuf::from(s)
    } else {
        path
    }
}

/// 拼接视频生成端点：确保以 /v1/videos/generations 结尾
///
/// OpenAI: https://api.openai.com/v1 → .../v1/videos/generations
/// DeepSeek: https://api.deepseek.com → .../v1/videos/generations（DeepSeek 无 /v1）
/// 兼容用户填入的任意 base_url 形式
fn build_videos_endpoint(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/videos/generations")
    } else if base.ends_with("/v1/") {
        format!("{}/videos/generations", base.trim_end_matches('/'))
    } else {
        format!("{base}/v1/videos/generations")
    }
}

/// 截断字符串到 max 字符，避免错误信息过长
#[inline]
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{truncated}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 工具辅助：构造一个未配置模型的 GenerateVideoTool
    fn tool_without_config() -> GenerateVideoTool {
        GenerateVideoTool::new(
            Arc::new(RwLock::new(None)),
            PathBuf::from("/tmp/attachments"),
        )
    }

    fn args_with(prompt: &str) -> GenerateVideoArgs {
        GenerateVideoArgs {
            prompt: prompt.to_string(),
            file_path: String::new(),
            image_paths: Vec::new(),
            video_paths: Vec::new(),
            resolution: None,
            ratio: None,
            model_id: None,
            duration: None,
        }
    }

    #[tokio::test]
    async fn returns_error_when_config_none() {
        let tool = tool_without_config();
        let err = tool.call(args_with("test")).await.unwrap_err();
        assert!(
            err.0.contains("未配置视频生成模型"),
            "actual: {}",
            err.0
        );
    }

    #[tokio::test]
    async fn rejects_invalid_resolution() {
        let tool = tool_without_config();
        let mut args = args_with("test");
        args.resolution = Some("1080p".to_string()); // 非法
        let err = tool.call(args).await.unwrap_err();
        assert!(
            err.0.contains("不支持的分辨率 1080p"),
            "actual: {}",
            err.0
        );
    }

    #[tokio::test]
    async fn rejects_invalid_ratio() {
        let tool = tool_without_config();
        let mut args = args_with("test");
        args.ratio = Some("32:9".to_string()); // 非法
        let err = tool.call(args).await.unwrap_err();
        assert!(
            err.0.contains("不支持的宽高比 32:9"),
            "actual: {}",
            err.0
        );
    }

    #[tokio::test]
    async fn rejects_duration_out_of_range() {
        let tool = tool_without_config();
        let mut args = args_with("test");
        args.duration = Some(20); // 越界
        let err = tool.call(args).await.unwrap_err();
        assert!(err.0.contains("时长 20 越界"), "actual: {}", err.0);
    }

    #[tokio::test]
    async fn rejects_duration_zero() {
        let tool = tool_without_config();
        let mut args = args_with("test");
        args.duration = Some(0); // 0 既不是 -1 也不在 2..=15
        let err = tool.call(args).await.unwrap_err();
        assert!(err.0.contains("时长 0 越界"), "actual: {}", err.0);
    }

    #[tokio::test]
    async fn duration_minus_one_passes_validation_but_fails_on_config() {
        // -1 是合法默认值，应通过参数校验，进入配置解析阶段
        let tool = tool_without_config();
        let mut args = args_with("test");
        args.duration = Some(-1);
        let err = tool.call(args).await.unwrap_err();
        assert!(err.0.contains("未配置视频生成模型"), "actual: {}", err.0);
    }

    #[tokio::test]
    async fn valid_resolution_passes_validation_but_fails_on_config() {
        // 480p/720p 合法，进入配置解析阶段
        let tool = tool_without_config();
        let mut args = args_with("test");
        args.resolution = Some("720p".to_string());
        let err = tool.call(args).await.unwrap_err();
        assert!(err.0.contains("未配置视频生成模型"), "actual: {}", err.0);
    }

    #[test]
    fn validate_resolution_accepts_known_values() {
        assert!(validate_resolution(None).is_ok());
        assert!(validate_resolution(Some("480p")).is_ok());
        assert!(validate_resolution(Some("720p")).is_ok());
        assert!(validate_resolution(Some("1080p")).is_err());
    }

    #[test]
    fn validate_ratio_accepts_known_values() {
        for r in [
            "16:9", "4:3", "1:1", "3:4", "9:16", "21:9", "adaptive",
        ] {
            assert!(validate_ratio(Some(r)).is_ok(), "should accept {r}");
        }
        assert!(validate_ratio(None).is_ok());
        assert!(validate_ratio(Some("32:9")).is_err());
    }

    #[test]
    fn validate_duration_boundary() {
        assert!(validate_duration(None).is_ok());
        assert!(validate_duration(Some(-1)).is_ok(), "-1 是默认值");
        assert!(validate_duration(Some(2)).is_ok(), "下界");
        assert!(validate_duration(Some(15)).is_ok(), "上界");
        assert!(validate_duration(Some(1)).is_err(), "1 越界");
        assert!(validate_duration(Some(16)).is_err(), "16 越界");
        assert!(validate_duration(Some(0)).is_err(), "0 越界");
    }

    #[test]
    fn build_videos_endpoint_handles_various_base_urls() {
        // OpenAI 风格
        assert_eq!(
            build_videos_endpoint("https://api.openai.com/v1"),
            "https://api.openai.com/v1/videos/generations"
        );
        // DeepSeek 风格（无 /v1）
        assert_eq!(
            build_videos_endpoint("https://api.deepseek.com"),
            "https://api.deepseek.com/v1/videos/generations"
        );
        // 末尾带斜杠
        assert_eq!(
            build_videos_endpoint("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/videos/generations"
        );
        assert_eq!(
            build_videos_endpoint("https://api.example.com/"),
            "https://api.example.com/v1/videos/generations"
        );
        // 自定义 host
        assert_eq!(
            build_videos_endpoint("https://video.example.com"),
            "https://video.example.com/v1/videos/generations"
        );
    }

    #[test]
    fn ensure_mp4_extension_appends_when_missing() {
        let p = ensure_mp4_extension(PathBuf::from("/tmp/video_001"));
        assert_eq!(p.file_name().unwrap(), "video_001.mp4");

        // 已有扩展名：保持不变
        let p2 = ensure_mp4_extension(PathBuf::from("/tmp/video_001.mp4"));
        assert_eq!(p2.file_name().unwrap(), "video_001.mp4");

        // 已有其他扩展名：保持不变（不强制覆盖为 mp4）
        let p3 = ensure_mp4_extension(PathBuf::from("/tmp/video_001.mov"));
        assert_eq!(p3.file_name().unwrap(), "video_001.mov");
    }

    #[test]
    fn resolve_save_target_uses_attachments_when_no_file_path() {
        let attachments = PathBuf::from("/tmp/attachments");
        let (path, filename, name) =
            resolve_save_target("", &attachments, "abc123");
        assert_eq!(path, PathBuf::from("/tmp/attachments/video_abc123.mp4"));
        assert_eq!(filename, "video_abc123.mp4");
        assert_eq!(name, "生成视频_abc123");
    }

    #[test]
    fn resolve_save_target_uses_user_path_when_provided() {
        let attachments = PathBuf::from("/tmp/attachments");
        let (path, filename, name) =
            resolve_save_target("/output/my_clip", &attachments, "abc123");
        assert_eq!(path, PathBuf::from("/output/my_clip.mp4"));
        assert_eq!(filename, "my_clip.mp4");
        assert_eq!(name, "my_clip");
    }

    #[test]
    fn resolve_save_target_keeps_existing_extension() {
        let attachments = PathBuf::from("/tmp/attachments");
        let (path, filename, name) = resolve_save_target(
            "/output/my_clip.mov",
            &attachments,
            "abc123",
        );
        assert_eq!(path, PathBuf::from("/output/my_clip.mov"));
        assert_eq!(filename, "my_clip.mov");
        assert_eq!(name, "my_clip");
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("", 10), "");
    }

    #[test]
    fn truncate_long_string_gets_ellipsis() {
        let s = "a".repeat(20);
        let t = truncate(&s, 5);
        assert_eq!(t, "aaaaa...");
    }

    #[test]
    fn truncate_exact_length_no_ellipsis() {
        let s = "abcde".to_string();
        let t = truncate(&s, 5);
        assert_eq!(t, "abcde");
    }
}

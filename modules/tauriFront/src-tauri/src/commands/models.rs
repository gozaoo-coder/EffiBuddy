//! Provider 预设、可使用模型管理、远程模型列表与图像生成命令。

use std::sync::Arc;

use effisuite_agent::{ImageGenConfig, ImageGenTool};
use effisuite_core::{
    builtin_presets, Attachment, AttachmentKind, AvailableModel, BackendKind, ModelKind,
    ProviderPreset,
};
use serde::Deserialize;
use tauri::Emitter;

use crate::agent::{apply_embedding_provider, build_agent_from_state};
use crate::commands::chat::truncate_str;
use crate::commands::config::ActiveModelInfo;
use crate::config_io::save_config;
use crate::state::AppState;

/// 返回内置 provider 预设列表（openai/deepseek/groq/...）
#[tauri::command]
pub(crate) fn list_provider_presets() -> Vec<ProviderPreset> {
    builtin_presets()
}

#[tauri::command]
pub(crate) async fn get_active_model_info(
    state: tauri::State<'_, AppState>,
) -> Result<ActiveModelInfo, String> {
    // Arc 快照读（廉价，不再深拷贝 AgentConfig）
    let config = state.config.read().await.clone();
    if let Some(id) = config.active_model_id.as_ref() {
        if let Some(m) = config.models.iter().find(|m| &m.id == id) {
            return Ok(ActiveModelInfo {
                id: m.id.clone(),
                name: m.model_name.clone(),
                context_window_tokens: m.context_window_tokens,
            });
        }
    }
    // 无激活模型时回退到运行时配置
    Ok(ActiveModelInfo {
        id: String::new(),
        name: config.model_name.clone(),
        context_window_tokens: Some(128000),
    })
}

/// 保存当前 draft 配置为一个可使用模型（model.id 由前端生成）
/// 返回新模型的 id
#[tauri::command]
pub(crate) async fn save_model(
    state: tauri::State<'_, AppState>,
    model: AvailableModel,
) -> Result<String, String> {
    // COW：读快照 → clone 内部 → 修改 → 写回新 Arc
    let mut config = state.config.read().await.as_ref().clone();
    // 若 id 已存在则更新，否则新增
    let id = model.id.clone();
    if let Some(existing) = config.models.iter_mut().find(|m| m.id == id) {
        *existing = model;
    } else {
        config.models.push(model);
    }
    save_config(&config)?;
    *state.config.write().await = Arc::new(config);
    Ok(id)
}

/// 删除指定 id 的可使用模型；若它是当前激活模型则清空 active_model_id
#[tauri::command]
pub(crate) async fn delete_model(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    // COW：读快照 → clone 内部 → 修改 → 写回新 Arc
    let mut config = state.config.read().await.as_ref().clone();
    config.models.retain(|m| m.id != id);
    if config.active_model_id.as_deref() == Some(id.as_str()) {
        config.active_model_id = None;
    }
    save_config(&config)?;
    *state.config.write().await = Arc::new(config);
    Ok(())
}

/// 激活指定 id 的可使用模型：根据模型 kind 走不同后端。
///
/// - kind=Chat：把模型配置写入 AgentConfig 运行时字段，重建对话 agent。
///   用户后续对话走此模型；同时它是"默认对话模型"。
/// - kind=ImageGen：更新 image_gen_config 句柄，不重建对话 agent。
///   LLM 调用 image_gen 工具时使用此配置。
/// - kind=VideoGen：暂未实现，返回错误。
#[tauri::command]
pub(crate) async fn set_active_model(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
) -> Result<String, String> {
    // COW：读快照 → clone 内部 → 修改 → 写回新 Arc
    let mut config = state.config.read().await.as_ref().clone();
    let model = config
        .models
        .iter()
        .find(|m| m.id == id)
        .cloned()
        .ok_or_else(|| format!("模型 {} 不存在", id))?;

    match model.kind {
        ModelKind::ImageGen => {
            // 图像生成模型：只更新 image_gen_config，不重建对话 agent
            let cfg = ImageGenConfig {
                api_key: model.api_key.clone(),
                base_url: model.base_url.clone(),
                model: model.model_name.clone(),
                default_size: model.image_size.clone(),
                default_quality: model.image_quality.clone(),
            };
            config.active_image_gen_model_id = Some(id.clone());
            save_config(&config)?;
            *state.image_gen_config.write().await = Some(cfg);
            *state.config.write().await = Arc::new(config);
            // 图像模型变更不需要重建对话 agent：版本号同步跟进，避免下次消息误触发重建
            state
                .config_rev
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            state.agent_rev.store(
                state.config_rev.load(std::sync::atomic::Ordering::SeqCst),
                std::sync::atomic::Ordering::SeqCst,
            );
            let _ = app_handle.emit("agent-backend-changed", ());
            Ok(id)
        }
        ModelKind::VideoGen => Err("视频生成模型暂未实现".to_string()),
        ModelKind::Chat => {
            // 对话模型：写入运行时字段并重建 agent
            config.api_key = model.api_key.clone();
            config.base_url = model.base_url.clone();
            config.model_name = model.model_name.clone();
            config.preamble = model.preamble.clone();
            config.provider_id = model.provider_id.clone();
            config.enable_tools = model.enable_tools;
            config.backend = BackendKind::Openai;
            config.active_model_id = Some(id.clone());

            save_config(&config)?;
            // 配置版本 +1（本命令已直接重建 agent，agent_rev 同步跟进）
            state
                .config_rev
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

            // 切换模型时刷新 embedding provider（base_url/api_key 可能变化）
            apply_embedding_provider(&config, &state.memory).await;

            // 构造新 agent（复用 build_agent_from_state 消除重复参数组装）
            let new_agent = build_agent_from_state(&state, &config);
            {
                let mut agent_lock = state.agent.write().await;
                *agent_lock = new_agent;
            }
            *state.config.write().await = Arc::new(config);
            state.agent_rev.store(
                state.config_rev.load(std::sync::atomic::Ordering::SeqCst),
                std::sync::atomic::Ordering::SeqCst,
            );

            let _ = app_handle.emit("agent-backend-changed", ());
            Ok(id)
        }
    }
}

/// 激活指定 id 的图像生成模型：更新 image_gen_config 句柄，不重建对话 agent。
///
/// 与 `set_active_model`（切换对话模型）独立：用户可同时激活一个对话模型和一个图像生成模型。
/// 激活后，LLM 在对话中调用 image_gen 工具时会使用此配置。
#[tauri::command]
pub(crate) async fn set_image_gen_model(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    // COW：读快照 → clone 内部 → 修改 → 写回新 Arc
    let mut config = state.config.read().await.as_ref().clone();
    let model = config
        .models
        .iter()
        .find(|m| m.id == id)
        .cloned()
        .ok_or_else(|| format!("模型 {} 不存在", id))?;
    if model.kind != ModelKind::ImageGen {
        return Err(format!("模型 {} 不是图像生成模型（kind != image_gen）", id));
    }
    let cfg = ImageGenConfig {
        api_key: model.api_key.clone(),
        base_url: model.base_url.clone(),
        model: model.model_name.clone(),
        default_size: model.image_size.clone(),
        default_quality: model.image_quality.clone(),
    };
    config.active_image_gen_model_id = Some(id.clone());
    save_config(&config)?;
    *state.image_gen_config.write().await = Some(cfg);
    *state.config.write().await = Arc::new(config);
    // 图像模型变更不需要重建对话 agent：版本号同步跟进，避免下次消息误触发重建
    state
        .config_rev
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    state.agent_rev.store(
        state.config_rev.load(std::sync::atomic::Ordering::SeqCst),
        std::sync::atomic::Ordering::SeqCst,
    );
    Ok(id)
}

/// 直接调用图像生成 API 生成图片（绕过 LLM，供前端"立即生成"按钮使用）。
///
/// 返回附件信息（id/path/name），前端通过 read_attachment 读取图片二进制。
#[tauri::command]
pub(crate) async fn generate_image(
    state: tauri::State<'_, AppState>,
    prompt: String,
    size: Option<String>,
    quality: Option<String>,
) -> Result<Attachment, String> {
    // 确认图像生成模型已配置
    let _cfg = state.image_gen_config.read().await.clone().ok_or_else(|| {
        "未配置图像生成模型，请先在设置中激活一个 kind=image_gen 的模型".to_string()
    })?;

    let tool = ImageGenTool::new(
        Arc::clone(&state.image_gen_config),
        state.attachments_dir.clone(),
    );
    let output = tool
        .generate(prompt, size, quality)
        .await
        .map_err(|e| e.to_string())?;
    // 读取文件大小
    let filepath = state.attachments_dir.join(&output.path);
    let file_size = std::fs::metadata(&filepath).map(|m| m.len()).unwrap_or(0);

    Ok(Attachment {
        id: output.id,
        kind: AttachmentKind::Image,
        path: output.path,
        name: output.name,
        mime_type: "image/png".to_string(),
        size: file_size,
    })
}

/// 读取附件文件并返回 base64 data URL，供前端 `<img src>` 直接渲染。
///
/// 采用 base64 data URL 而非 asset 协议，避免 Tauri 2 资源协议配置复杂性，
/// 同时保证跨平台（Windows/macOS/Linux）一致行为。
/// 仅支持图片类型（image/png, image/jpeg, image/gif, image/webp）。
#[tauri::command]
pub(crate) async fn read_attachment(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let filepath = state.attachments_dir.join(&path);
    let bytes = tokio::fs::read(&filepath)
        .await
        .map_err(|e| format!("读取附件失败: {e}"))?;
    let mime = guess_mime(&path);
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    Ok(format!("data:{mime};base64,{b64}"))
}

/// 根据文件扩展名猜测 MIME 类型
fn guess_mime(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".bmp") {
        "image/bmp"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
}

// =========================================================
// 远程模型列表：调用 OpenAI 兼容 /v1/models 接口
// =========================================================

/// 远程模型条目（OpenAI /v1/models 响应中的 data 元素）
///
/// 字段对齐 OpenAI 官方 API；id 为模型标识（如 gpt-4o-mini），
/// owned_by 为归属（openai / organization-owner / ...）。
/// permission 字段对前端无用，直接丢弃避免反序列化失败。
#[derive(Debug, serde::Serialize)]
pub(crate) struct RemoteModelInfo {
    id: String,
    object: String,
    owned_by: String,
    /// 模型创建时间（Unix 秒），部分 provider 不返回，留 None
    created: Option<u64>,
}

/// /v1/models 响应
#[derive(Deserialize)]
struct RemoteModelsResponse {
    data: Vec<RemoteModelData>,
}

#[derive(Deserialize)]
struct RemoteModelData {
    id: String,
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    owned_by: Option<String>,
    #[serde(default)]
    created: Option<u64>,
}

/// /v1/models/{model} 响应：单个模型
#[derive(Deserialize)]
struct RemoteModelDetailResponse {
    id: String,
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    owned_by: Option<String>,
    #[serde(default)]
    created: Option<u64>,
}

/// 列出 API 可用模型
///
/// 调用 OpenAI 兼容 `GET {base_url}/models` 接口，返回模型列表。
/// 用于在 ModelConfigPanel 中让用户从 API 实际可用模型中选择，
/// 而不是硬编码推荐列表。
///
/// 参数：
/// - `base_url`：API 基地址（如 https://api.openai.com/v1）
/// - `api_key`：Bearer token
#[tauri::command]
pub(crate) async fn list_remote_models(
    base_url: String,
    api_key: String,
) -> Result<Vec<RemoteModelInfo>, String> {
    if api_key.trim().is_empty() {
        return Err("API Key 为空，无法拉取模型列表".into());
    }
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))?;
    let resp = client
        .get(&url)
        .bearer_auth(&api_key)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {status}: {}", truncate_str(&body, 300)));
    }
    // 手动用 serde_json 解析，避免依赖 reqwest 的 json feature
    let body_bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取响应体失败: {e}"))?;
    let parsed: RemoteModelsResponse =
        serde_json::from_slice(&body_bytes).map_err(|e| format!("解析响应失败: {e}"))?;
    // 按 id 排序，便于前端展示
    let mut list: Vec<_> = parsed
        .data
        .into_iter()
        .map(|d| RemoteModelInfo {
            id: d.id,
            object: d.object.unwrap_or_else(|| "model".to_string()),
            owned_by: d.owned_by.unwrap_or_default(),
            created: d.created,
        })
        .collect();
    list.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(list)
}

/// 检索单个模型详情
///
/// 调用 OpenAI 兼容 `GET {base_url}/models/{model}` 接口。
#[tauri::command]
pub(crate) async fn get_remote_model(
    base_url: String,
    api_key: String,
    model: String,
) -> Result<RemoteModelInfo, String> {
    if api_key.trim().is_empty() {
        return Err("API Key 为空".into());
    }
    let url = format!(
        "{}/models/{}",
        base_url.trim_end_matches('/'),
        urlencoding_encode(&model)
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))?;
    let resp = client
        .get(&url)
        .bearer_auth(&api_key)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {status}: {}", truncate_str(&body, 300)));
    }
    let body_bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取响应体失败: {e}"))?;
    let parsed: RemoteModelDetailResponse =
        serde_json::from_slice(&body_bytes).map_err(|e| format!("解析响应失败: {e}"))?;
    Ok(RemoteModelInfo {
        id: parsed.id,
        object: parsed.object.unwrap_or_else(|| "model".to_string()),
        owned_by: parsed.owned_by.unwrap_or_default(),
        created: parsed.created,
    })
}

/// URL 路径段编码（model id 含 `:` `/` 等需转义）
fn urlencoding_encode(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
                c.to_string()
            } else {
                format!("%{:02X}", c as u8)
            }
        })
        .collect()
}

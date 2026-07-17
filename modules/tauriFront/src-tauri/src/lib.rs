//! EffiSuite Tauri 应用入口：连接 agent / p2p / core 并暴露给前端
//!
//! 设计要点：
//! - `AppState` 字段按大小降序排列，最小化结构体 padding
//! - `agent` 用 `Arc<RwLock<Arc<dyn ChatAgent>>>` 包装：
//!   - 外层 `RwLock` 允许运行时热替换 agent（写少读多）
//!   - 内层 `Arc<dyn ChatAgent>` 让 async 命令可廉价 clone 后跨 await 持有
//! - 命令层薄封装：转译请求、转发事件，不持长临界区锁
//! - async 命令中先 clone 出 `Arc` 句柄再 `.await`，避免跨 await 持有
//!   `tauri::State` 的借用（Tauri 2.x async command 的 lifetime 约束）
//! - 事件转发通过 broadcast 订阅 + spawn 完成，遵循"消息传递代替共享内存"
//! - 流式命令 spawn 独立 task，逐 token emit "agent-token"，结束 emit "agent-done"

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use effisuite_agent::{
    AgentStreamItem, ChatAgent, ContextPreview, ImageGenConfig, ImageGenTool, MockAgent,
    OpenAIEmbeddingProvider, RigAgent, DEFAULT_EMBEDDING_MODEL,
};
use effisuite_core::{
    AgentConfig, Attachment, AttachmentKind, AvailableModel, BackendKind, BusEvent, Conversation,
    ConversationMeta, ConversationStore, Device, EventBus, MemoryHit, MemoryIndex, MemoryStats,
    Message, ModelKind, PinnedMemory, PinnedMemorySource, PinnedMemoryStore, ProviderPreset, Role,
    ScheduledTask, ScheduledTaskStore, SearchHit, SearchMode, Skill, SkillStore, ThemeMode,
    builtin_presets,
};
use effisuite_p2p::P2pManager;
use futures::StreamExt;
use tauri::{Emitter, Manager};
use tokio::sync::RwLock;

mod scheduler;

/// 应用全局状态，由 `tauri::Builder::manage` 注入。
///
/// 字段按大小降序：`SkillStore`/`ScheduledTaskStore`（PathBuf+Arc，4 usize）在前，
/// `Arc<RwLock<...>>` / `Arc<...>`（1 usize）居中，
/// `Option<JoinHandle>`（1 usize）在后。
pub struct AppState {
    /// 技能存储（PathBuf+Arc，4 usize）
    pub skill_store: SkillStore,
    /// 定时任务存储（PathBuf+Arc，4 usize）
    pub schedule_store: ScheduledTaskStore,
    /// 可热替换的 agent：RwLock 写少读多，内层 Arc 让 async 命令可跨 await 持有
    pub agent: Arc<RwLock<Arc<dyn ChatAgent>>>,
    pub store: Arc<ConversationStore>,
    pub config: Arc<RwLock<AgentConfig>>,
    pub p2p: Arc<P2pManager>,
    pub event_bus: EventBus,
    /// 跨会话历史记忆索引（RAG 记忆增强核心），与 agent 共享同一份 Arc
    pub memory: Arc<MemoryIndex>,
    /// 永久记忆存储（用户主动要求"记住"的内容），与 agent 共享同一份 Arc
    pub pinned_memory: Arc<PinnedMemoryStore>,
    /// 当前活跃会话 id，由 send_message 命令更新；agent 据此排除当前会话
    pub current_conversation_id: Arc<RwLock<Option<String>>>,
    /// 当前工作区路径，由 send_message 命令更新；agent 据此解析相对路径与设置 shell cwd。
    /// 优先级：会话级 working_dir > 技能级 working_dir > 进程默认 cwd。
    pub working_dir: Arc<RwLock<Option<std::path::PathBuf>>>,
    /// 图像生成模型配置句柄：set_image_gen_model 时更新，build_agent 注入到 ImageGenTool。
    /// None 表示未配置图像生成能力，image_gen 工具调用会返回错误提示。
    pub image_gen_config: Arc<RwLock<Option<ImageGenConfig>>>,
    /// 附件存储目录（绝对路径），ImageGenTool 据此落盘生成图片。
    pub attachments_dir: std::path::PathBuf,
    /// cron 调度器 task 句柄（setup 中 spawn 后写入；shutdown 时可 abort）。
    /// 用 `Mutex` 是因为句柄在 setup 阶段才产生，需运行时回填到已 manage 的 state。
    pub scheduler_handle: std::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

/// 当前 Unix 毫秒时间戳；失败时回退为 0，避免在命令路径里 panic。
#[inline]
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// appdata 根目录：`<app_data_dir>/effisuite`
fn appdata_root() -> std::path::PathBuf {
    let base = dirs::data_dir()
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("effisuite")
}

/// 配置文件路径：`<appdata>/config.json`
fn config_path() -> std::path::PathBuf {
    appdata_root().join("config.json")
}

/// 会话存储目录：`<appdata>/conversations`
fn conversations_dir() -> std::path::PathBuf {
    appdata_root().join("conversations")
}

/// 技能存储目录：`<appdata>/skills`
fn skills_dir() -> std::path::PathBuf {
    appdata_root().join("skills")
}

/// 定时任务存储目录：`<appdata>/schedules`
fn schedules_dir() -> std::path::PathBuf {
    appdata_root().join("schedules")
}

/// embedding 向量缓存文件：`<appdata>/memory_embeddings.json`
fn embeddings_cache_path() -> std::path::PathBuf {
    appdata_root().join("memory_embeddings.json")
}

/// 永久记忆存储文件：`<appdata>/pinned_memories.json`
fn pinned_memories_path() -> std::path::PathBuf {
    appdata_root().join("pinned_memories.json")
}

/// 附件存储目录：`<appdata>/attachments`
///
/// ImageGenTool 把生成图片落盘到此目录；前端通过 read_attachment 命令读取。
fn attachments_dir() -> std::path::PathBuf {
    appdata_root().join("attachments")
}

/// 加载配置；不存在时返回默认值
fn load_config_or_default() -> AgentConfig {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => AgentConfig::default(),
    }
}

/// 持久化配置到磁盘
fn save_config(config: &AgentConfig) -> std::result::Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let s = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(&path, s).map_err(|e| e.to_string())?;
    Ok(())
}

/// 根据 config 构造对应的 ChatAgent
///
/// `memory` 与 `current_conversation_id` 注入到 RigAgent 以启用 RAG 记忆增强；
/// `pinned_memory` 注入以启用永久记忆能力（每轮注入 `[永久记忆]` 段 + pin_memory 工具）。
/// `working_dir` 注入以启用工作区路径解析（read_file/list_files/shell 据此锚定相对路径）。
/// `image_gen_config` 注入以启用图像生成工具（LLM 可主动调用 image_gen 为用户作画）。
/// `attachments_dir` 为图片落盘目录。
/// MockAgent 后端忽略这些参数。
fn build_agent(
    config: &AgentConfig,
    memory: Arc<MemoryIndex>,
    pinned_memory: Arc<PinnedMemoryStore>,
    current_conversation_id: Arc<RwLock<Option<String>>>,
    working_dir: Arc<RwLock<Option<std::path::PathBuf>>>,
    image_gen_config: Arc<RwLock<Option<ImageGenConfig>>>,
    attachments_dir: std::path::PathBuf,
    store: Arc<ConversationStore>,
) -> Arc<dyn ChatAgent> {
    match config.backend {
        BackendKind::Openai if config.is_rig_ready() => {
            match RigAgent::from_key(
                &config.api_key,
                &config.base_url,
                &config.model_name,
                &config.preamble,
                config.enable_tools,
                Some(memory),
                Some(pinned_memory),
                current_conversation_id,
                working_dir,
                image_gen_config,
                attachments_dir,
                store,
            ) {
                Ok(agent) => Arc::new(agent),
                Err(e) => {
                    tracing::warn!(error = %e, "RigAgent 构造失败，回退到 MockAgent");
                    Arc::new(MockAgent::new())
                }
            }
        }
        _ => Arc::new(MockAgent::new()),
    }
}

/// 根据 config 构造 embedding provider 并注入到 memory index。
///
/// - backend=openai 且有 api_key：构造 `OpenAIEmbeddingProvider`，启用向量检索路
/// - 否则：清除 memory 的 provider，退化为纯词法检索
async fn apply_embedding_provider(config: &AgentConfig, memory: &MemoryIndex) {
    if config.is_rig_ready() {
        let provider = Arc::new(OpenAIEmbeddingProvider::new(
            config.api_key.clone(),
            config.base_url.clone(),
            DEFAULT_EMBEDDING_MODEL.to_string(),
            Some(embeddings_cache_path()),
        ));
        memory.set_embedding_provider(provider).await;
        tracing::info!(model = %DEFAULT_EMBEDDING_MODEL, "已启用向量检索路");
    } else {
        memory.clear_embedding_provider().await;
        tracing::info!("向量检索路已禁用（无 api_key 或 backend 非 openai）");
    }
}

/// 从 ConversationStore 全量重建 memory index。
///
/// 异步遍历所有 conversation 文件，把每条非系统、非空消息加入索引。
/// 在启动时 spawn 一次，避免阻塞 app 启动；后续 send_message 会增量更新。
async fn rebuild_memory_from_store(store: &ConversationStore, memory: &MemoryIndex) {
    let metas = match store.list_meta().await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "rebuild_memory: list_meta 失败");
            return;
        }
    };
    // 收集所有 (conv_id, Message) 对
    let mut pairs: Vec<(String, Message)> = Vec::with_capacity(metas.iter().map(|m| m.message_count).sum());
    for meta in &metas {
        match store.load(&meta.id).await {
            Ok(Some(conv)) => {
                for msg in conv.messages {
                    pairs.push((conv.id.clone(), msg));
                }
            }
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(error = %e, conv_id = %meta.id, "rebuild_memory: load 失败");
            }
        }
    }
    memory.rebuild_from_messages(pairs).await;
    let stats = memory.stats().await;
    tracing::info!(
        total = stats.total_entries,
        tokens = stats.unique_tokens,
        "memory index 重建完成"
    );
}

/// 后台批量计算 embedding，直到全部条目已嵌入或 provider 失效。
///
/// 每批 32 条，批间 100ms sleep 避免触发 rate limit。
async fn spawn_embedding_computation(memory: Arc<MemoryIndex>) {
    loop {
        match memory.ensure_embeddings(32).await {
            Ok(0) => {
                tracing::info!("embedding 批量计算完成（无更多待嵌入条目）");
                return;
            }
            Ok(n) => {
                tracing::info!(embedded = n, "embedding 批量完成");
                // 短暂 sleep 避免 rate limit
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "embedding 批量计算失败，停止后台任务");
                return;
            }
        }
    }
}

/// 把 `BusEvent` 转发为前端可监听的 Tauri 事件。
/// payload 直接使用 `BusEvent` 本身（已实现 Serialize，带 `kind` 标签）。
fn forward_event(handle: &tauri::AppHandle, event: &BusEvent) {
    let (name, payload) = match event {
        BusEvent::AgentStreamToken { .. } => ("agent-token", event),
        BusEvent::AgentMessage { .. } => ("agent-message", event),
        BusEvent::DeviceFound { .. } => ("device-found", event),
        BusEvent::DeviceStatusChanged { .. } => ("device-status-changed", event),
        BusEvent::PairingRequest { .. } => ("pairing-request", event),
    };
    let _ = handle.emit(name, payload);
}

// =========================================================
// 命令：通用
// =========================================================

#[tauri::command]
fn greet(name: String) -> String {
    format!("Hello, {}! EffiSuite 已就绪。", name)
}

#[tauri::command]
async fn get_agent_backend(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let agent = state.agent.read().await.clone();
    Ok(agent.backend().to_string())
}

// =========================================================
// 命令：配置管理
// =========================================================

#[tauri::command]
async fn get_config(state: tauri::State<'_, AppState>) -> Result<AgentConfig, String> {
    Ok(state.config.read().await.clone())
}

#[tauri::command]
async fn set_config(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    config: AgentConfig,
) -> Result<(), String> {
    // 持久化
    save_config(&config)?;

    // 根据新配置刷新 embedding provider（启用/禁用向量检索路）
    apply_embedding_provider(&config, &state.memory).await;

    // 同步图像生成配置：若 active_image_gen_model_id 指向有效模型则更新句柄
    if let Some(cfg) = resolve_image_gen_config(&config) {
        *state.image_gen_config.write().await = Some(cfg);
    } else {
        *state.image_gen_config.write().await = None;
    }

    // 构造新 agent：注入 memory / pinned_memory / current_conversation_id / image_gen_config / store
    let new_agent = build_agent(
        &config,
        Arc::clone(&state.memory),
        Arc::clone(&state.pinned_memory),
        Arc::clone(&state.current_conversation_id),
        Arc::clone(&state.working_dir),
        Arc::clone(&state.image_gen_config),
        state.attachments_dir.clone(),
        Arc::clone(&state.store),
    );

    // 替换 state 中的 agent 和 config
    {
        let mut agent_lock = state.agent.write().await;
        *agent_lock = new_agent;
    }
    {
        let mut config_lock = state.config.write().await;
        *config_lock = config;
    }

    // 通知前端 backend 变化
    let _ = app_handle.emit("agent-backend-changed", ());

    Ok(())
}

// =========================================================
// 命令：Provider 预设 & 可使用模型管理
// =========================================================

/// 返回内置 provider 预设列表（openai/deepseek/groq/...）
#[tauri::command]
fn list_provider_presets() -> Vec<ProviderPreset> {
    builtin_presets()
}

/// 从 AgentConfig 解析当前激活的图像生成配置。
///
/// 根据 `active_image_gen_model_id` 在 models 列表中查找 kind=ImageGen 的模型，
/// 构造 ImageGenConfig 快照。未配置时返回 None。
fn resolve_image_gen_config(config: &AgentConfig) -> Option<ImageGenConfig> {
    let id = config.active_image_gen_model_id.as_ref()?;
    let m = config.models.iter().find(|m| m.id == *id)?;
    if m.kind != ModelKind::ImageGen {
        return None;
    }
    Some(ImageGenConfig {
        api_key: m.api_key.clone(),
        base_url: m.base_url.clone(),
        model: m.model_name.clone(),
        default_size: m.image_size.clone(),
        default_quality: m.image_quality.clone(),
    })
}

/// 激活指定 id 的图像生成模型：更新 image_gen_config 句柄，不重建对话 agent。
///
/// 与 `set_active_model`（切换对话模型）独立：用户可同时激活一个对话模型和一个图像生成模型。
/// 激活后，LLM 在对话中调用 image_gen 工具时会使用此配置。
#[tauri::command]
async fn set_image_gen_model(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    let mut config = state.config.write().await.clone();
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
    *state.config.write().await = config;
    Ok(id)
}

/// 直接调用图像生成 API 生成图片（绕过 LLM，供前端"立即生成"按钮使用）。
///
/// 返回附件信息（id/path/name），前端通过 read_attachment 读取图片二进制。
#[tauri::command]
async fn generate_image(
    state: tauri::State<'_, AppState>,
    prompt: String,
    size: Option<String>,
    quality: Option<String>,
) -> Result<Attachment, String> {
    // 确认图像生成模型已配置
    let _cfg = state
        .image_gen_config
        .read()
        .await
        .clone()
        .ok_or_else(|| "未配置图像生成模型，请先在设置中激活一个 kind=image_gen 的模型".to_string())?;

    let tool = ImageGenTool::new(
        Arc::clone(&state.image_gen_config),
        state.attachments_dir.clone(),
    );
    let output = tool.generate(prompt, size, quality).await.map_err(|e| e.to_string())?;

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
async fn read_attachment(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let filepath = state.attachments_dir.join(&path);
    let bytes = tokio::fs::read(&filepath)
        .await
        .map_err(|e| format!("读取附件失败: {e}"))?;
    let mime = guess_mime(&path);
    let b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &bytes,
    );
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

/// 保存当前 draft 配置为一个可使用模型（model.id 由前端生成）
/// 返回新模型的 id
#[tauri::command]
async fn save_model(
    state: tauri::State<'_, AppState>,
    model: AvailableModel,
) -> Result<String, String> {
    let mut config = state.config.write().await.clone();
    // 若 id 已存在则更新，否则新增
    let id = model.id.clone();
    if let Some(existing) = config.models.iter_mut().find(|m| m.id == id) {
        *existing = model;
    } else {
        config.models.push(model);
    }
    save_config(&config)?;
    *state.config.write().await = config;
    Ok(id)
}

/// 删除指定 id 的可使用模型；若它是当前激活模型则清空 active_model_id
#[tauri::command]
async fn delete_model(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let mut config = state.config.write().await.clone();
    config.models.retain(|m| m.id != id);
    if config.active_model_id.as_deref() == Some(id.as_str()) {
        config.active_model_id = None;
    }
    save_config(&config)?;
    *state.config.write().await = config;
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
async fn set_active_model(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
) -> Result<String, String> {
    let mut config = state.config.write().await.clone();
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
            *state.config.write().await = config;
            let _ = app_handle.emit("agent-backend-changed", ());
            Ok(id)
        }
        ModelKind::VideoGen => {
            Err("视频生成模型暂未实现".to_string())
        }
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

            // 切换模型时刷新 embedding provider（base_url/api_key 可能变化）
            apply_embedding_provider(&config, &state.memory).await;

            let new_agent = build_agent(
                &config,
                Arc::clone(&state.memory),
                Arc::clone(&state.pinned_memory),
                Arc::clone(&state.current_conversation_id),
                Arc::clone(&state.working_dir),
                Arc::clone(&state.image_gen_config),
                state.attachments_dir.clone(),
                Arc::clone(&state.store),
            );
            {
                let mut agent_lock = state.agent.write().await;
                *agent_lock = new_agent;
            }
            *state.config.write().await = config;

            let _ = app_handle.emit("agent-backend-changed", ());
            Ok(id)
        }
    }
}

/// 设置主题模式（持久化，不重建 agent）
#[tauri::command]
async fn set_theme(
    state: tauri::State<'_, AppState>,
    theme: ThemeMode,
) -> Result<(), String> {
    let mut config = state.config.write().await.clone();
    config.theme = theme;
    save_config(&config)?;
    *state.config.write().await = config;
    Ok(())
}

// =========================================================
// 命令：会话管理
// =========================================================

#[tauri::command]
async fn list_conversations(state: tauri::State<'_, AppState>) -> Result<Vec<ConversationMeta>, String> {
    let store = state.store.clone();
    store.list_meta().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_conversation(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Option<Conversation>, String> {
    let store = state.store.clone();
    store.load(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn create_conversation(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conv = Conversation::new(id.clone(), now_ms());
    state.store.save(&conv).await.map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
async fn delete_conversation(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    state.store.delete(&id).await.map_err(|e| e.to_string())
}

/// 重命名会话标题
#[tauri::command]
async fn rename_conversation(
    state: tauri::State<'_, AppState>,
    id: String,
    title: String,
) -> Result<(), String> {
    state
        .store
        .rename(&id, title)
        .await
        .map_err(|e| e.to_string())
}

/// 置顶/取消置顶会话
#[tauri::command]
async fn toggle_pin_conversation(
    state: tauri::State<'_, AppState>,
    id: String,
    pinned: bool,
) -> Result<(), String> {
    state
        .store
        .set_pinned(&id, pinned, now_ms())
        .await
        .map_err(|e| e.to_string())
}

/// 跨会话搜索消息内容（基于存储层的简单关键词匹配）
#[tauri::command]
async fn search_conversations(
    state: tauri::State<'_, AppState>,
    query: String,
) -> Result<Vec<SearchHit>, String> {
    state
        .store
        .search(&query)
        .await
        .map_err(|e| e.to_string())
}

// =========================================================
// 命令：RAG 记忆增强检索
// =========================================================

/// 跨会话历史记忆检索（RAG：BM25 词法 + 向量 embedding + RRF 混合）
///
/// 与 `search_conversations`（存储层简单关键词匹配）不同，本命令走 memory index：
/// - `lexical`：BM25 + IDF 加权，倒排表加速
/// - `vector`：embedding 余弦相似度（需配置 OpenAI 兼容 provider）
/// - `hybrid`：RRF 融合两路（默认推荐）
///
/// 自动排除当前活跃会话（若已通过 send_message 设置）。
#[tauri::command]
async fn search_memory(
    state: tauri::State<'_, AppState>,
    query: String,
    mode: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<MemoryHit>, String> {
    let mode = parse_search_mode(mode.as_deref());
    let limit = limit.unwrap_or(5).max(1).min(50);
    // 读出当前会话 id（短暂读锁），检索时排除
    let exclude = state.current_conversation_id.read().await.clone();
    let hits = state
        .memory
        .search(&query, limit, mode, exclude.as_deref())
        .await;
    Ok(hits)
}

/// 返回 memory index 统计信息（条目数、唯一 token 数、已嵌入条目数、平均文档长度）
#[tauri::command]
async fn get_memory_stats(state: tauri::State<'_, AppState>) -> Result<MemoryStats, String> {
    Ok(state.memory.stats().await)
}

/// 解析前端传入的检索模式字符串为 SearchMode 枚举
fn parse_search_mode(s: Option<&str>) -> SearchMode {
    match s.map(str::to_ascii_lowercase).as_deref() {
        Some("lexical") | Some("bm25") | Some("keyword") => SearchMode::Lexical,
        Some("vector") | Some("embedding") | Some("semantic") => SearchMode::Vector,
        _ => SearchMode::Hybrid,
    }
}

// =========================================================
// 命令：永久记忆（Pinned Memory）管理
// =========================================================
//
// 与 `search_memory`（RAG 按相关性检索）不同，永久记忆是用户主动要求
// "始终记住"的内容：一旦加入，每轮 prompt 都会注入到 `[永久记忆]` 段。
//
// 用户可通过两条路径管理：
// 1. 对话中明确说"请记住..."→ LLM 调用 pin_memory 工具（见 agent 模块）
// 2. UI 面板手动新增/编辑/删除 → 前端调用下列命令

/// 列出全部永久记忆（按 created_at 降序，新的在前）
#[tauri::command]
async fn list_pinned_memories(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<PinnedMemory>, String> {
    Ok(state.pinned_memory.list().await)
}

/// 新增一条永久记忆（来源固定为 Manual），返回新 id
#[tauri::command]
async fn add_pinned_memory(
    state: tauri::State<'_, AppState>,
    content: String,
    category: Option<String>,
) -> Result<String, String> {
    let content = content.trim().to_string();
    if content.is_empty() {
        return Err("content 不能为空".to_string());
    }
    if content.chars().count() > 2000 {
        return Err("content 过长（>2000 字符），请精简后再添加".to_string());
    }
    state
        .pinned_memory
        .add_simple(
            content,
            category,
            PinnedMemorySource::Manual,
            None,
            now_ms(),
        )
        .await
        .map_err(|e| e.to_string())
}

/// 更新指定 id 的永久记忆内容与/或分类。
/// `category` 为 `null` 表示不变；为空字符串表示清空分类（前端约定）。
#[tauri::command]
async fn update_pinned_memory(
    state: tauri::State<'_, AppState>,
    id: String,
    content: Option<String>,
    category: Option<String>,
) -> Result<(), String> {
    // 区分"未提供 category"（None = 不变）与"清空 category"（Some("") = 清空）
    let category_opt = match category {
        None => None,
        Some(s) if s.is_empty() => Some(None),
        Some(s) => Some(Some(s)),
    };
    state
        .pinned_memory
        .update(&id, content, category_opt)
        .await
        .map_err(|e| e.to_string())
}

/// 删除指定 id 的永久记忆
#[tauri::command]
async fn delete_pinned_memory(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state
        .pinned_memory
        .delete(&id)
        .await
        .map_err(|e| e.to_string())
}

/// 清空所有永久记忆（危险操作，前端应有二次确认）
#[tauri::command]
async fn clear_pinned_memories(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state
        .pinned_memory
        .clear()
        .await
        .map_err(|e| e.to_string())
}

// =========================================================
// 命令：上下文注入预览
// =========================================================
//
// 返回当前 agent 在指定会话（或无会话）下将注入到 LLM 的完整 prompt 结构。
// 仅用于"上下文管理"面板的可视化展示，不触发实际 LLM 调用。
//
// `conversation_id` 为 None 时使用空消息列表，仅展示 preamble + 永久记忆。

/// 返回当前 agent 对指定会话的上下文注入预览。
///
/// - 加载该会话的完整消息历史
/// - 调用 `agent.context_preview(&messages)` 拿到结构化预览
/// - 返回 `Some(ContextPreview)` 或 `None`（MockAgent 后端）
#[tauri::command]
async fn get_context_preview(
    state: tauri::State<'_, AppState>,
    conversation_id: Option<String>,
) -> Result<Option<ContextPreview>, String> {
    let agent = state.agent.read().await.clone();
    let messages = if let Some(id) = conversation_id.as_deref() {
        // 加载指定会话的完整消息历史；不存在或加载失败视为空列表
        match state.store.load(id).await {
            Ok(Some(conv)) => conv.messages,
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };
    Ok(agent.context_preview(&messages).await)
}

// =========================================================
// 命令：聊天（流式 + 非流式）
// =========================================================

#[tauri::command]
async fn send_message(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    conversation_id: String,
    content: String,
) -> Result<String, String> {
    let agent = state.agent.read().await.clone();
    let store = state.store.clone();
    let bus = state.event_bus.clone();
    let memory = Arc::clone(&state.memory);
    let cur_conv = Arc::clone(&state.current_conversation_id);
    let working_dir_handle = Arc::clone(&state.working_dir);

    // 标记当前会话：agent 据此排除当前会话，避免与已注入上下文重复
    *cur_conv.write().await = Some(conversation_id.clone());

    // 同步会话级工作区到 agent 句柄：read_file/list_files/shell 据此解析相对路径
    // 优先级：会话级 working_dir > 技能级（apply_skill 已写入会话） > None
    let conv_wd = store
        .load(&conversation_id)
        .await
        .map_err(|e| e.to_string())?
        .and_then(|c| c.working_dir)
        .map(std::path::PathBuf::from);
    *working_dir_handle.write().await = conv_wd;

    // 先把用户消息持久化到 store，同时取回完整历史
    let user_msg = Message::new(
        uuid::Uuid::new_v4().to_string(),
        Role::User,
        content,
        now_ms(),
    );
    // 克隆一份用于 memory 增量索引（append_message 会 move user_msg）
    let user_msg_for_memory = user_msg.clone();
    let conv = store
        .append_message(&conversation_id, user_msg, now_ms())
        .await
        .map_err(|e| e.to_string())?;
    // 增量更新 memory index（幂等，已存在则跳过）
    memory.add(&conversation_id, user_msg_for_memory).await;

    // 调用 agent
    let history = conv.history().to_vec();
    let reply = agent.chat(&history).await.map_err(|e| e.to_string())?;

    // 持久化助手回复
    let assistant_msg = Message::new(
        uuid::Uuid::new_v4().to_string(),
        Role::Assistant,
        reply.clone(),
        now_ms(),
    );
    let assistant_msg_for_memory = assistant_msg.clone();
    store
        .append_message(&conversation_id, assistant_msg, now_ms())
        .await
        .map_err(|e| e.to_string())?;
    memory.add(&conversation_id, assistant_msg_for_memory).await;

    // 通过事件总线通知前端
    bus.publish(BusEvent::AgentMessage {
        conversation_id: conversation_id.clone(),
        content: reply.clone(),
        done: true,
    });

    let _ = app_handle.emit("agent-message", &reply);
    Ok(reply)
}

/// 流式发送消息：spawn 独立 task，逐 token emit "agent-token"，结束 emit "agent-done"
#[tauri::command]
async fn send_message_stream(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    conversation_id: String,
    content: String,
) -> Result<(), String> {
    let agent = state.agent.read().await.clone();
    let store = state.store.clone();
    let memory = Arc::clone(&state.memory);
    let cur_conv = Arc::clone(&state.current_conversation_id);
    let working_dir_handle = Arc::clone(&state.working_dir);
    let handle = app_handle.clone();

    // 标记当前会话：agent 据此排除当前会话
    *cur_conv.write().await = Some(conversation_id.clone());

    // 1. 持久化用户消息并取回完整历史
    let user_msg = Message::new(
        uuid::Uuid::new_v4().to_string(),
        Role::User,
        content,
        now_ms(),
    );
    let user_msg_for_memory = user_msg.clone();
    let conv = store
        .append_message(&conversation_id, user_msg, now_ms())
        .await
        .map_err(|e| e.to_string())?;
    // 增量更新 memory index
    memory.add(&conversation_id, user_msg_for_memory).await;

    // 同步会话级工作区到 agent 句柄：read_file/list_files/shell 据此解析相对路径
    // 优先级：会话级 working_dir > 技能级（apply_skill 已写入会话） > None
    let conv_wd = conv
        .working_dir
        .clone()
        .map(std::path::PathBuf::from);
    *working_dir_handle.write().await = conv_wd;

    let history = conv.history().to_vec();
    let conv_id = conversation_id.clone();

    // 2. spawn 独立 task 驱动流
    tauri::async_runtime::spawn(async move {
        let mut stream = agent.chat_stream(&history);
        let mut full = String::with_capacity(256);
        // 跟踪 call_id → tool_name 映射，用于在 ToolResult 时判断是否为 image_gen / set_title
        let mut tool_call_names: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        // 跟踪 call_id → arguments 映射，set_title 结果到达时解析 title 字段
        let mut tool_call_args: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        // 收集 image_gen 工具生成的图片附件，流结束后注入到助手消息
        let mut image_attachments: Vec<Attachment> = Vec::new();

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(AgentStreamItem::Text { content }) => {
                    full.push_str(&content);
                    // 仅直接 emit 给前端。
                    // 注意：不能同时 bus.publish，否则 setup() 中的总线订阅者会经
                    // forward_event 再 emit 一次 "agent-token"，导致前端收到双份
                    // token，表现为文本交错重复。
                    let _ = handle.emit(
                        "agent-token",
                        &StreamTokenPayload {
                            conversation_id: &conv_id,
                            content: &content,
                            done: false,
                        },
                    );
                }
                Ok(AgentStreamItem::Reasoning { content }) => {
                    let _ = handle.emit(
                        "agent-reasoning",
                        &AgentReasoningPayload {
                            conversation_id: &conv_id,
                            content: &content,
                        },
                    );
                }
                Ok(AgentStreamItem::ToolCallStart { call_id, tool_name, arguments }) => {
                    // 记录 call_id → tool_name，供 ToolResult 时判断是否为 image_gen / set_title
                    tool_call_names.insert(call_id.clone(), tool_name.clone());
                    let args_str = serde_json::to_string(&arguments).unwrap_or_else(|_| "null".to_string());
                    // 记录 call_id → args_str，供 set_title 结果到达时解析 title 字段
                    tool_call_args.insert(call_id.clone(), args_str.clone());
                    let _ = handle.emit(
                        "agent-tool-call",
                        &AgentToolCallPayload {
                            conversation_id: &conv_id,
                            call_id: &call_id,
                            tool_name: &tool_name,
                            arguments: &args_str,
                        },
                    );
                }
                Ok(AgentStreamItem::ToolResult { call_id, output, is_error }) => {
                    // 若为 image_gen 工具结果，解析 JSON 提取图片信息并收集为附件
                    if let Some(name) = tool_call_names.get(&call_id) {
                        if name == "image_gen" && !is_error {
                            if let Some(att) = parse_image_gen_output(&output) {
                                // 实时通知前端有新图片生成，可立即渲染
                                let _ = handle.emit(
                                    "agent-attachment",
                                    &AgentAttachmentPayload {
                                        conversation_id: &conv_id,
                                        attachment: &att,
                                    },
                                );
                                image_attachments.push(att);
                            }
                        } else if name == "set_title" && !is_error {
                            // set_title 工具已自行调用 store.rename 持久化标题。
                            // 这里从 arguments 解析 title，emit 事件让前端立即刷新 SideNav，
                            // 不必等流结束。解析失败则静默（流结束的 conversation-changed 仍会刷新）。
                            if let Some(title) = tool_call_args
                                .get(&call_id)
                                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                                .and_then(|v| v.get("title").and_then(|t| t.as_str()).map(str::to_string))
                            {
                                let _ = handle.emit(
                                    "conversation-title-updated",
                                    &ConversationTitlePayload {
                                        conversation_id: &conv_id,
                                        title: &title,
                                    },
                                );
                            }
                        }
                    }
                    let _ = handle.emit(
                        "agent-tool-result",
                        &AgentToolResultPayload {
                            conversation_id: &conv_id,
                            call_id: &call_id,
                            output: &output,
                            is_error,
                        },
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "stream error");
                    let _ = handle.emit(
                        "agent-stream-error",
                        &StreamErrorPayload {
                            conversation_id: &conv_id,
                            error: &e.to_string(),
                        },
                    );
                    return;
                }
            }
        }

        // 3. 流结束，持久化完整回复（含图片附件）
        let mut assistant_msg = Message::new(
            uuid::Uuid::new_v4().to_string(),
            Role::Assistant,
            full.clone(),
            now_ms(),
        );
        // 把 image_gen 工具生成的图片注入到消息附件，持久化后前端可历史回看
        if !image_attachments.is_empty() {
            assistant_msg.attachments = image_attachments.clone();
        }
        let assistant_msg_for_memory = assistant_msg.clone();
        if let Err(e) = store
            .append_message(&conv_id, assistant_msg, now_ms())
            .await
        {
            tracing::warn!(error = %e, "persist assistant reply failed");
        }
        // 增量更新 memory index（即使持久化失败也尝试索引，best-effort）
        memory.add(&conv_id, assistant_msg_for_memory).await;

        // 4. 通知前端流结束（仅直接 emit，避免与总线转发重复）
        let _ = handle.emit(
            "agent-done",
            &StreamTokenPayload {
                conversation_id: &conv_id,
                content: &full,
                done: true,
            },
        );
    });

    Ok(())
}

/// 流式 token payload（与前端 TS 类型对齐）
#[derive(Debug, serde::Serialize)]
struct StreamTokenPayload<'a> {
    conversation_id: &'a str,
    content: &'a str,
    done: bool,
}

#[derive(Debug, serde::Serialize)]
struct StreamErrorPayload<'a> {
    conversation_id: &'a str,
    error: &'a str,
}

/// 推理增量 payload（agent-reasoning 事件）
#[derive(Debug, serde::Serialize)]
struct AgentReasoningPayload<'a> {
    conversation_id: &'a str,
    content: &'a str,
}

/// 工具调用开始 payload（agent-tool-call 事件）
#[derive(Debug, serde::Serialize)]
struct AgentToolCallPayload<'a> {
    conversation_id: &'a str,
    call_id: &'a str,
    tool_name: &'a str,
    /// JSON 字符串形式的参数
    arguments: &'a str,
}

/// 工具执行结果 payload（agent-tool-result 事件）
#[derive(Debug, serde::Serialize)]
struct AgentToolResultPayload<'a> {
    conversation_id: &'a str,
    call_id: &'a str,
    output: &'a str,
    is_error: bool,
}

/// 图片附件生成 payload（agent-attachment 事件）
///
/// 当 image_gen 工具成功生成图片时实时 emit，前端收到后立即渲染图片，
/// 无需等待流结束。
#[derive(Debug, serde::Serialize)]
struct AgentAttachmentPayload<'a> {
    conversation_id: &'a str,
    attachment: &'a Attachment,
}

/// 会话标题更新 payload（conversation-title-updated 事件）
///
/// 当 set_title 工具成功更新标题后实时 emit，前端立即刷新 SideNav 列表，
/// 无需等流结束。title 为 SetTitleTool 返回的截断后标题。
#[derive(Debug, serde::Serialize)]
struct ConversationTitlePayload<'a> {
    conversation_id: &'a str,
    title: &'a str,
}

/// 解析 image_gen 工具输出为 Attachment。
///
/// ImageGenTool 返回的 ImageGenOutput 序列化为 JSON：
/// `{"id":"...","path":"gen_xxx.png","name":"生成图片_xxx.png","elapsed_ms":1234}`
/// rig 把它作为 ToolResultContent::Text 传回，extract_tool_output 提取为字符串。
/// 此函数尝试反序列化并构造 Attachment；失败时返回 None（静默跳过）。
fn parse_image_gen_output(output: &str) -> Option<Attachment> {
    let v: serde_json::Value = serde_json::from_str(output).ok()?;
    let id = v.get("id")?.as_str()?.to_string();
    let path = v.get("path")?.as_str()?.to_string();
    let name = v.get("name")?.as_str()?.to_string();
    Some(Attachment {
        id,
        kind: AttachmentKind::Image,
        path,
        name,
        mime_type: "image/png".to_string(),
        size: 0,
    })
}

// =========================================================
// 命令：P2P
// =========================================================

#[tauri::command]
async fn scan_devices(
    state: tauri::State<'_, AppState>,
    _app_handle: tauri::AppHandle,
) -> Result<Vec<Device>, String> {
    let p2p = state.p2p.clone();
    p2p.scan_once().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_devices(state: tauri::State<'_, AppState>) -> Result<Vec<Device>, String> {
    let p2p = state.p2p.clone();
    Ok(p2p.list_devices().await)
}

#[tauri::command]
async fn pair_device(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let p2p = state.p2p.clone();
    p2p.pair_device(&id).await.map_err(|e| e.to_string())
}

// =========================================================
// 命令：文件 / 图片选择
// =========================================================

/// 用户通过系统对话框选择的文件信息
#[derive(Debug, Clone, serde::Serialize)]
pub struct PickedFile {
    pub path: String,
    pub name: String,
    pub size: u64,
}

/// 从 `FilePath` 提取 `(path_str, name, size)`，供 pick_file / pick_image 复用。
fn picked_file_info(path_str: String) -> PickedFile {
    let name = std::path::Path::new(&path_str)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let size = std::fs::metadata(&path_str).map(|m| m.len()).unwrap_or(0);
    PickedFile {
        path: path_str,
        name,
        size,
    }
}

/// 弹出系统文件选择对话框（文档/图片/所有文件）
#[tauri::command]
async fn pick_file(app: tauri::AppHandle) -> Result<Option<PickedFile>, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app
        .dialog()
        .file()
        .add_filter("文档", &["txt", "md", "pdf", "doc", "docx", "csv", "json", "rs", "py", "ts", "js"])
        .add_filter("图片", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])
        .add_filter("所有文件", &["*"])
        .blocking_pick_file();
    Ok(path.map(|fp| picked_file_info(fp.to_string())))
}

/// 弹出系统图片选择对话框
#[tauri::command]
async fn pick_image(app: tauri::AppHandle) -> Result<Option<PickedFile>, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app
        .dialog()
        .file()
        .add_filter("图片", &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"])
        .blocking_pick_file();
    Ok(path.map(|fp| picked_file_info(fp.to_string())))
}

/// 调起系统相机应用（桌面端简化为复用图片选择器作为相机替代）
#[tauri::command]
async fn capture_photo(app: tauri::AppHandle) -> Result<Option<PickedFile>, String> {
    pick_image(app).await
}

/// 读取文件文本内容（供 agent 使用），默认最多 512KB。
/// 截断处若落在多字节字符中间，回退到最后一个有效 UTF-8 边界。
#[tauri::command]
async fn read_file_text(path: String, max_bytes: Option<u64>) -> Result<String, String> {
    let max = max_bytes.unwrap_or(512 * 1024) as usize;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let truncated: &[u8] = if bytes.len() > max { &bytes[..max] } else { &bytes[..] };
    match std::str::from_utf8(truncated) {
        Ok(s) => Ok(s.to_string()),
        Err(e) => {
            let cut = e.valid_up_to();
            if cut == 0 {
                Err("文件内容不是有效的 UTF-8 文本".to_string())
            } else {
                Ok(std::str::from_utf8(&truncated[..cut])
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| "文件内容不是有效的 UTF-8 文本".to_string()))
            }
        }
    }
}

// =========================================================
// 命令：技能（Skill）管理
// =========================================================

/// 列出全部技能：内置（agent-reach / browser-act）+ 用户自定义
#[tauri::command]
async fn list_skills(state: tauri::State<'_, AppState>) -> Result<Vec<Skill>, String> {
    state.skill_store.list_all().await.map_err(|e| e.to_string())
}

/// 创建用户技能，返回 id。空 id 自动生成；强制 builtin=false
#[tauri::command]
async fn create_skill(
    state: tauri::State<'_, AppState>,
    mut skill: Skill,
) -> Result<String, String> {
    if skill.id.is_empty() {
        skill.id = uuid::Uuid::new_v4().to_string();
    }
    if skill.created_at == 0 {
        skill.created_at = now_ms();
    }
    skill.builtin = false;
    let id = skill.id.clone();
    state
        .skill_store
        .save(&skill)
        .await
        .map_err(|e| e.to_string())?;
    Ok(id)
}

/// 更新用户技能（内置技能不可修改）。保留原 created_at，强制 builtin=false
#[tauri::command]
async fn update_skill(
    state: tauri::State<'_, AppState>,
    id: String,
    mut skill: Skill,
) -> Result<(), String> {
    let existing = state
        .skill_store
        .get(&id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("技能 {} 不存在", id))?;
    if existing.builtin {
        return Err("内置技能不可修改".to_string());
    }
    skill.id = id;
    skill.builtin = false;
    skill.created_at = existing.created_at;
    state
        .skill_store
        .save(&skill)
        .await
        .map_err(|e| e.to_string())
}

/// 删除技能；内置技能不可删除
#[tauri::command]
async fn delete_skill(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    if state
        .skill_store
        .get(&id)
        .await
        .map_err(|e| e.to_string())?
        .map(|s| s.builtin)
        .unwrap_or(false)
    {
        return Err("内置技能不可删除".to_string());
    }
    state
        .skill_store
        .delete(&id)
        .await
        .map_err(|e| e.to_string())
}

/// 在指定会话应用技能：把 preamble 作为系统消息注入会话历史，
/// 后续 send_message 会把它纳入 agent 上下文。
///
/// 工作区注入：若技能配置了 working_dir，且会话级 working_dir 未设置，
/// 则把技能的 working_dir 写入会话级（会话级优先级更高，可被用户后续覆盖）。
#[tauri::command]
async fn apply_skill(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
    skill_id: String,
) -> Result<(), String> {
    let skill = state
        .skill_store
        .get(&skill_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("技能 {} 不存在", skill_id))?;

    // 工作区注入：技能级 working_dir 在会话级未设置时写入会话
    if let Some(skill_wd) = skill.working_dir.clone() {
        let conv = state.store.load(&conversation_id).await.map_err(|e| e.to_string())?;
        let need_set = conv
            .as_ref()
            .map(|c| c.working_dir.is_none())
            .unwrap_or(true);
        if need_set {
            state
                .store
                .set_working_dir(&conversation_id, Some(skill_wd))
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    if skill.preamble.is_empty() {
        return Ok(());
    }
    let sys_msg = Message::new(
        uuid::Uuid::new_v4().to_string(),
        Role::System,
        skill.preamble,
        now_ms(),
    );
    state
        .store
        .append_message(&conversation_id, sys_msg, now_ms())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 设置/清除会话级工作区路径。
///
/// 传入 Some(path) 设置工作区，传入 None 清除（回退到技能级或进程默认）。
/// 设置后，该会话后续 send_message 时 read_file/list_files/shell 以此目录为基准。
/// 优先级：会话级 working_dir > 技能级 working_dir > 进程默认 cwd。
#[tauri::command]
async fn set_conversation_working_dir(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
    working_dir: Option<String>,
) -> Result<(), String> {
    state
        .store
        .set_working_dir(&conversation_id, working_dir)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 获取会话级工作区路径（None 表示未设置，将回退到技能级或进程默认）。
#[tauri::command]
async fn get_conversation_working_dir(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<Option<String>, String> {
    let conv = state
        .store
        .load(&conversation_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(conv.and_then(|c| c.working_dir))
}

/// 调起系统目录选择对话框，返回所选目录的绝对路径。
///
/// 供前端设置技能/会话工作区时使用，避免用户手输路径出错。
#[tauri::command]
async fn pick_directory(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app
        .dialog()
        .file()
        .set_title("选择工作区目录")
        .blocking_pick_folder();
    Ok(path.map(|p| p.to_string()))
}

// =========================================================
// 命令：定时任务（ScheduledTask）管理
// =========================================================

#[tauri::command]
async fn list_scheduled_tasks(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ScheduledTask>, String> {
    state
        .schedule_store
        .list()
        .await
        .map_err(|e| e.to_string())
}

/// 创建定时任务，返回 id。空 id 自动生成
#[tauri::command]
async fn create_scheduled_task(
    state: tauri::State<'_, AppState>,
    mut task: ScheduledTask,
) -> Result<String, String> {
    if task.id.is_empty() {
        task.id = uuid::Uuid::new_v4().to_string();
    }
    if task.created_at == 0 {
        task.created_at = now_ms();
    }
    let id = task.id.clone();
    state
        .schedule_store
        .save(&task)
        .await
        .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
async fn delete_scheduled_task(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state
        .schedule_store
        .delete(&id)
        .await
        .map_err(|e| e.to_string())
}

/// 启用/停用定时任务
#[tauri::command]
async fn toggle_scheduled_task(
    state: tauri::State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut task = state
        .schedule_store
        .get(&id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("定时任务 {} 不存在", id))?;
    task.enabled = enabled;
    state
        .schedule_store
        .save(&task)
        .await
        .map_err(|e| e.to_string())
}

// =========================================================
// 入口
// =========================================================

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // 加载配置
    let config = load_config_or_default();

    // 初始化跨会话记忆索引与当前会话 id 句柄（RAG 记忆增强核心）
    let memory: Arc<MemoryIndex> = Arc::new(MemoryIndex::new());
    let current_conversation_id: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
    // 工作区句柄：send_message 时根据会话级/技能级 working_dir 更新
    let working_dir: Arc<RwLock<Option<std::path::PathBuf>>> = Arc::new(RwLock::new(None));

    // 图像生成配置句柄：从 config 解析 active_image_gen_model_id 对应的配置
    let image_gen_config: Arc<RwLock<Option<ImageGenConfig>>> =
        Arc::new(RwLock::new(resolve_image_gen_config(&config)));
    // 附件目录：ImageGenTool 落盘生成图片到此目录
    let attachments_root = attachments_dir();

    // 初始化永久记忆存储（用户主动要求"记住"的内容）
    let pinned_memory: Arc<PinnedMemoryStore> = match PinnedMemoryStore::new(pinned_memories_path()) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!(error = %e, "PinnedMemoryStore 初始化失败，回退到临时目录");
            Arc::new(
                PinnedMemoryStore::new(std::env::temp_dir().join("effisuite-pinned-memories.json"))
                    .expect("临时目录 PinnedMemoryStore 必须成功"),
            )
        }
    };

    // 初始化会话存储：SetTitleTool 需要此句柄持久化标题，必须在 build_agent 之前完成
    let store = match ConversationStore::new(conversations_dir()) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!(error = %e, "ConversationStore 初始化失败，回退到临时目录");
            Arc::new(
                ConversationStore::new(std::env::temp_dir().join("effisuite-conversations"))
                    .expect("临时目录 ConversationStore 必须成功"),
            )
        }
    };

    // 构造 agent：注入 memory / pinned_memory / current_conversation_id / working_dir / image_gen_config / store
    let agent: Arc<dyn ChatAgent> = build_agent(
        &config,
        Arc::clone(&memory),
        Arc::clone(&pinned_memory),
        Arc::clone(&current_conversation_id),
        Arc::clone(&working_dir),
        Arc::clone(&image_gen_config),
        attachments_root.clone(),
        Arc::clone(&store),
    );

    let skill_store = match SkillStore::new(skills_dir()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "SkillStore 初始化失败，回退到临时目录");
            SkillStore::new(std::env::temp_dir().join("effisuite-skills"))
                .expect("临时目录 SkillStore 必须成功")
        }
    };
    let schedule_store = match ScheduledTaskStore::new(schedules_dir()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "ScheduledTaskStore 初始化失败，回退到临时目录");
            ScheduledTaskStore::new(std::env::temp_dir().join("effisuite-schedules"))
                .expect("临时目录 ScheduledTaskStore 必须成功")
        }
    };

    let event_bus = EventBus::new(64);
    let p2p = Arc::new(P2pManager::new(event_bus.clone()));

    tracing::info!(backend = %agent.backend(), "EffiSuite 启动");

    // 克隆一份配置用于 setup 阶段异步初始化 memory（避免在同步 setup 闭包中 .await）
    let config_for_setup = config.clone();
    let agent_lock = Arc::new(RwLock::new(agent));
    let state = AppState {
        skill_store,
        schedule_store,
        agent: Arc::clone(&agent_lock),
        store: Arc::clone(&store),
        config: Arc::new(RwLock::new(config)),
        p2p,
        event_bus,
        memory: Arc::clone(&memory),
        pinned_memory: Arc::clone(&pinned_memory),
        current_conversation_id: Arc::clone(&current_conversation_id),
        working_dir: Arc::clone(&working_dir),
        image_gen_config: Arc::clone(&image_gen_config),
        attachments_dir: attachments_root,
        scheduler_handle: std::sync::Mutex::new(None),
    };

    tauri::Builder::default()
        .manage(state)
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .setup(move |app| {
            // 订阅内部事件总线，转发为前端 Tauri 事件。
            // 使用 tauri::async_runtime::spawn 以兼容桌面与 mobile 运行时。
            let handle = app.handle().clone();
            let bus = app.state::<AppState>().event_bus.clone();
            tauri::async_runtime::spawn(async move {
                let mut rx = bus.subscribe();
                while let Ok(event) = rx.recv().await {
                    forward_event(&handle, &event);
                }
            });

            // 启动 cron 调度器：每分钟检查一次到点任务并触发技能执行。
            // 句柄回填到 AppState.scheduler_handle，便于 shutdown 时 abort。
            let state = app.state::<AppState>();
            let sched_handle = scheduler::spawn_scheduler(
                app.handle().clone(),
                state.schedule_store.clone(),
                state.skill_store.clone(),
                Arc::clone(&state.agent),
                Arc::clone(&state.store),
            );
            *state.scheduler_handle.lock().unwrap() = Some(sched_handle);

            // RAG 记忆增强启动任务：
            // 1. 全量重建 memory index（从 ConversationStore 加载所有历史消息）
            // 2. 应用 embedding provider（若配置了 api_key）
            // 3. 后台批量计算缺失 embedding（直到全部完成或 provider 失效）
            let store_clone = Arc::clone(&state.store);
            let memory_clone = Arc::clone(&state.memory);
            tauri::async_runtime::spawn(async move {
                rebuild_memory_from_store(&store_clone, &memory_clone).await;
                apply_embedding_provider(&config_for_setup, &memory_clone).await;
                spawn_embedding_computation(memory_clone).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            get_agent_backend,
            // config
            get_config,
            set_config,
            // providers & models
            list_provider_presets,
            save_model,
            delete_model,
            set_active_model,
            set_image_gen_model,
            generate_image,
            read_attachment,
            // theme
            set_theme,
            // conversations
            list_conversations,
            get_conversation,
            create_conversation,
            delete_conversation,
            rename_conversation,
            toggle_pin_conversation,
            search_conversations,
            // RAG 记忆增强检索
            search_memory,
            get_memory_stats,
            // 永久记忆管理
            list_pinned_memories,
            add_pinned_memory,
            update_pinned_memory,
            delete_pinned_memory,
            clear_pinned_memories,
            // 上下文注入预览
            get_context_preview,
            // chat
            send_message,
            send_message_stream,
            // p2p
            scan_devices,
            get_devices,
            pair_device,
            // 文件 / 图片选择
            pick_file,
            pick_image,
            capture_photo,
            read_file_text,
            pick_directory,
            // 技能
            list_skills,
            create_skill,
            update_skill,
            delete_skill,
            apply_skill,
            set_conversation_working_dir,
            get_conversation_working_dir,
            // 定时任务
            list_scheduled_tasks,
            create_scheduled_task,
            delete_scheduled_task,
            toggle_scheduled_task,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

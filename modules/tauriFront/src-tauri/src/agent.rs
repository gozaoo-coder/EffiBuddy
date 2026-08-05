//! Agent 构建与同步：构造 `ChatAgent`、懒重建、embedding provider 与 memory 重建。
//!
//! 设计要点：
//! - `build_agent` 根据 `AgentConfig.backend` 选择 `RigAgent` 或 `MockAgent`，
//!   注入 memory / pinned_memory / working_dir / image_gen_config / skills / compression
//!   等全部能力句柄。
//! - `ensure_agent_synced` 在每次 send_message 前比对 config_rev / agent_rev，
//!   不一致时用最新配置重建 agent（懒重建机制）。
//! - `apply_embedding_provider` / `rebuild_memory_from_store` / `spawn_embedding_computation`
//!   在启动时负责初始化 RAG 记忆增强（向量检索路 + 全量索引 + 后台批量 embedding）。
//! - `resolve_image_gen_config` 从配置解析当前激活的图像生成模型快照。

use std::sync::Arc;
use std::time::Duration;

use effisuite_agent::{
    AgentPoolStore, AsrService, ChatAgent, ImageGenConfig, MockAgent, OpenAIEmbeddingProvider,
    RigAgent, ShellSessionManager, DEFAULT_EMBEDDING_MODEL,
};
use effisuite_agent::todo_store::TodoStore;
use effisuite_core::clawhub::ClawHubClient;
use effisuite_core::{
    AgentConfig, BackendKind, CompressionStore, ConversationStore, MemoryIndex, Message,
    ModelKind, PinnedMemoryStore, PluginStore, RemoteTaskDispatcher, SkillStore,
};
use tauri::Emitter;
use tokio::sync::RwLock;

use crate::paths::{embeddings_cache_path, skills_dir};
use crate::state::AppState;

/// 根据 config 构造对应的 ChatAgent
///
/// `memory` 与 `current_conversation_id` 注入到 RigAgent 以启用 RAG 记忆增强；
/// `pinned_memory` 注入以启用永久记忆能力（每轮注入 `[永久记忆]` 段 + pin_memory 工具）。
/// `working_dir` 注入以启用工作区路径解析（read_file/list_files/shell 据此锚定相对路径）。
/// `image_gen_config` 注入以启用图像生成工具（LLM 可主动调用 image_gen 为用户作画）。
/// `attachments_dir` 为图片落盘目录。
/// `skill_index` / `skill_store` / `clawhub_client` / `skills_dir` 注入后启用
/// 技能 RAG 自动注入与 6 个技能管理工具（list/get/enable/uninstall + search/install ClawHub）。
/// `plugin_store` 注入后启用 uninstall_plugin 工具。
/// `compression_store` 注入后启用消息压缩（build_context_parts 对历史段应用
/// Keep/Hide/Replace 决策，当前问题不压缩）。
/// MockAgent 后端忽略这些参数。
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_agent(
    config: &AgentConfig,
    memory: Arc<MemoryIndex>,
    pinned_memory: Arc<PinnedMemoryStore>,
    current_conversation_id: Arc<RwLock<Option<String>>>,
    working_dir: Arc<RwLock<Option<std::path::PathBuf>>>,
    image_gen_config: Arc<RwLock<Option<ImageGenConfig>>>,
    attachments_dir: std::path::PathBuf,
    store: Arc<ConversationStore>,
    skill_index: Arc<effisuite_core::SkillIndex>,
    skill_store: SkillStore,
    clawhub_client: ClawHubClient,
    skills_dir: std::path::PathBuf,
    plugin_store: PluginStore,
    compression_store: CompressionStore,
    todo_store: TodoStore,
    model_manager: Arc<effisuite_agent::ModelManagerHandle>,
    sub_agents: Arc<effisuite_agent::SubAgentManager>,
    asr_service: Arc<AsrService>,
    remote_task_dispatcher: Arc<dyn RemoteTaskDispatcher>,
    shell_sessions: Arc<ShellSessionManager>,
    agent_pool: AgentPoolStore,
      pending_user_messages: Arc<effisuite_agent::PendingUserMessages>,
      reasoning_config: Arc<tokio::sync::RwLock<Option<effisuite_agent::ReasoningConfig>>>,
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
                Some(skill_index),
                Some(skill_store),
                Some(clawhub_client),
                Some(skills_dir),
                Some(plugin_store),
                Some(compression_store),
                Some(model_manager),
                Some(sub_agents),
            ) {
                Ok(agent) => Arc::new(
                    agent
                        .with_todo_store(Some(todo_store))
                        .with_asr_service(Some(asr_service))
                        .with_remote_task_dispatcher(Some(remote_task_dispatcher))
                        .with_shell_sessions(Some(shell_sessions))
                        .with_agent_pool(Some(agent_pool))
                          .with_pending_user_messages(Some(pending_user_messages))
                          .with_reasoning_config(reasoning_config),
                ),
                Err(e) => {
                    tracing::warn!(error = %e, "RigAgent 构造失败，回退到 MockAgent");
                    Arc::new(MockAgent::new())
                }
            }
        }
        _ => Arc::new(MockAgent::new()),
    }
}

/// 从 AppState 组装 build_agent 的全部参数并构造新 agent。
/// 消除 ensure_agent_synced / set_config / set_active_model 中的重复 17 参数调用。
#[inline]
pub(crate) fn build_agent_from_state(
    state: &AppState,
    config: &AgentConfig,
) -> Arc<dyn ChatAgent> {
    // P2pManager 实现了 RemoteTaskDispatcher trait，.clone() 返回 Arc<P2pManager>，
    // 在 let 绑定的类型标注处协变为 Arc<dyn RemoteTaskDispatcher>
    let dispatcher: Arc<dyn RemoteTaskDispatcher> = state.p2p.clone();
    build_agent(
        config,
        Arc::clone(&state.memory),
        Arc::clone(&state.pinned_memory),
        Arc::clone(&state.current_conversation_id),
        Arc::clone(&state.working_dir),
        Arc::clone(&state.image_gen_config),
        state.attachments_dir.clone(),
        Arc::clone(&state.store),
        Arc::clone(&state.skill_index),
        state.skill_store.clone(),
        state.clawhub.clone(),
        skills_dir(),
        state.plugin_store.clone(),
        state.compression_store.clone(),
        state.todo_store.clone(),
        Arc::clone(&state.model_manager),
        Arc::clone(&state.sub_agents),
          Arc::clone(&state.asr_service),
          dispatcher,
          Arc::clone(&state.shell_sessions),
          state.agent_pool.clone(),
            Arc::clone(&state.pending_user_messages),
            Arc::clone(&state.reasoning_config),
          )
        )
    }


/// 在 set_config / ensure_agent_synced 路径中复用。
#[inline]
pub(crate) async fn sync_image_gen_config(state: &AppState, config: &AgentConfig) {
    let cfg = resolve_image_gen_config(config);
    *state.image_gen_config.write().await = cfg;
}

/// 懒重建：若配置版本号与当前 agent 不一致（agent 工具 manage_model 修改了配置），
/// 用最新配置重建 agent 并同步 image_gen_config 句柄。
/// 在每次 send_message / send_message_stream 前调用。
pub(crate) async fn ensure_agent_synced(
    state: &tauri::State<'_, AppState>,
    app_handle: &tauri::AppHandle,
) {
    let rev = state.config_rev.load(std::sync::atomic::Ordering::SeqCst);
    if state.agent_rev.load(std::sync::atomic::Ordering::SeqCst) == rev {
        return;
    }
    // Arc 快照克隆（廉价，不再深拷贝 AgentConfig）
    let config = state.config.read().await.clone();

    // 同步图像生成配置句柄
    sync_image_gen_config(state, &config).await;

    // 重建 agent（复用 build_agent_from_state 消除重复参数组装）
    let new_agent = build_agent_from_state(state, &config);
    {
        let mut agent_lock = state.agent.write().await;
        *agent_lock = new_agent;
    }
    state
        .agent_rev
        .store(rev, std::sync::atomic::Ordering::SeqCst);
    tracing::info!(rev, "agent 已按最新配置重建（模型/工具变更生效）");
    let _ = app_handle.emit("agent-backend-changed", ());
}

/// 根据 config 构造 embedding provider 并注入到 memory index。
///
/// - backend=openai 且有 api_key：构造 `OpenAIEmbeddingProvider`，启用向量检索路
/// - 否则：清除 memory 的 provider，退化为纯词法检索
pub(crate) async fn apply_embedding_provider(config: &AgentConfig, memory: &MemoryIndex) {
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
pub(crate) async fn rebuild_memory_from_store(store: &ConversationStore, memory: &MemoryIndex) {
    let metas = match store.list_meta().await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "rebuild_memory: list_meta 失败");
            return;
        }
    };
    // 收集所有 (conv_id, Message) 对
    let mut pairs: Vec<(String, Message)> =
        Vec::with_capacity(metas.iter().map(|m| m.message_count).sum());
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
pub(crate) async fn spawn_embedding_computation(memory: Arc<MemoryIndex>) {
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

/// 从 AgentConfig 解析当前激活的图像生成配置。
///
/// 根据 `active_image_gen_model_id` 在 models 列表中查找 kind=ImageGen 的模型，
/// 构造 ImageGenConfig 快照。未配置时返回 None。
pub(crate) fn resolve_image_gen_config(config: &AgentConfig) -> Option<ImageGenConfig> {
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

//! 配置管理命令：读取、写入、主题切换与活跃模型信息。

use std::sync::Arc;

use effisuite_core::{AgentConfig, ThemeMode};
use tauri::Emitter;

use crate::agent::{apply_embedding_provider, build_agent, resolve_image_gen_config};
use crate::config_io::save_config;
use crate::paths::skills_dir;
use crate::state::AppState;

#[tauri::command]
pub(crate) async fn get_config(state: tauri::State<'_, AppState>) -> Result<AgentConfig, String> {
    Ok(state.config.read().await.clone())
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct ActiveModelInfo {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) context_window_tokens: Option<u32>,
}

#[tauri::command]
pub(crate) async fn set_config(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    config: AgentConfig,
) -> Result<(), String> {
    // 持久化
    save_config(&config)?;
    // 配置版本 +1：通知懒重建机制（本命令已直接重建，agent_rev 同步跟进）
    state
        .config_rev
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    // 根据新配置刷新 embedding provider（启用/禁用向量检索路）
    apply_embedding_provider(&config, &state.memory).await;

    // 同步图像生成配置：若 active_image_gen_model_id 指向有效模型则更新句柄
    if let Some(cfg) = resolve_image_gen_config(&config) {
        *state.image_gen_config.write().await = Some(cfg);
    } else {
        *state.image_gen_config.write().await = None;
    }

    // 构造新 agent：注入 memory / pinned_memory / current_conversation_id / image_gen_config / store
    // 同时注入 skill_index / skill_store / clawhub / skills_dir 以启用技能 RAG 自动注入
    // 与 6 个技能管理工具（list/get/enable/uninstall + search/install ClawHub）
    // plugin_store 注入以启用 uninstall_plugin 工具
    // compression_store 注入以启用消息压缩
    let new_agent = build_agent(
        &config,
        Arc::clone(&state.memory),
        Arc::clone(&state.pinned_memory),
        Arc::clone(&state.current_conversation_id),
        Arc::clone(&state.working_dir),
        Arc::clone(&state.image_gen_config),
        state.attachments_dir.clone(),
        Arc::clone(&state.store),
        Arc::clone(&state.skill_index),
        Arc::new(state.skill_store.clone()),
        Arc::new(state.clawhub.clone()),
        skills_dir(),
        Arc::new(state.plugin_store.clone()),
        Arc::new(state.compression_store.clone()),
        Arc::clone(&state.model_manager),
        Arc::clone(&state.sub_agents),
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
    state.agent_rev.store(
        state.config_rev.load(std::sync::atomic::Ordering::SeqCst),
        std::sync::atomic::Ordering::SeqCst,
    );

    // 通知前端 backend 变化
    let _ = app_handle.emit("agent-backend-changed", ());

    Ok(())
}

/// 设置主题模式（持久化，不重建 agent）
#[tauri::command]
pub(crate) async fn set_theme(
    state: tauri::State<'_, AppState>,
    theme: ThemeMode,
) -> Result<(), String> {
    let mut config = state.config.read().await.clone();
    config.theme = theme;
    save_config(&config)?;
    *state.config.write().await = config;
    Ok(())
}

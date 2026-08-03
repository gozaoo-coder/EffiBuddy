//! 配置管理命令：读取、写入、主题切换与活跃模型信息。

use std::sync::Arc;

use effisuite_core::{AgentConfig, CompressionSettings, ThemeMode};
use tauri::Emitter;

use crate::agent::{apply_embedding_provider, build_agent_from_state, sync_image_gen_config};
use crate::config_io::save_config;
use crate::state::AppState;

#[tauri::command]
pub(crate) async fn get_config(state: tauri::State<'_, AppState>) -> Result<AgentConfig, String> {
    // Arc 快照读（廉价），深拷贝仅在返回序列化时发生
    Ok(state.config.read().await.as_ref().clone())
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct ActiveModelInfo {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) context_window_tokens: Option<u32>,
    /// 当前激活模型的计费单价（元/百万 tokens）；未配置时为 None
    pub(crate) pricing: Option<effisuite_core::ModelPricing>,
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

    // 同步图像生成配置句柄
    sync_image_gen_config(&state, &config).await;

    // 构造新 agent（复用 build_agent_from_state 消除重复参数组装）
    let new_agent = build_agent_from_state(&state, &config);

    // 替换 state 中的 agent 和 config
    {
        let mut agent_lock = state.agent.write().await;
        *agent_lock = new_agent;
    }
    {
        let mut config_lock = state.config.write().await;
        *config_lock = Arc::new(config);
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
    // COW：读快照 → clone 内部 → 修改 → 写回新 Arc
    let mut config = state.config.read().await.as_ref().clone();
    config.theme = theme;
    save_config(&config)?;
    *state.config.write().await = Arc::new(config);
      Ok(())
  }

  /// 更新压缩设置（阈值 / 自动压缩开关等）。
  /// COW：读快照 → clone 内部 → 修改 → 写回新 Arc，仅持久化配置，不重建 agent。
  #[tauri::command]
pub(crate) async fn update_compression_settings(
    state: tauri::State<'_, AppState>,
    settings: CompressionSettings,
) -> Result<(), String> {
    let mut config = state.config.read().await.as_ref().clone();
    config.compression_settings = settings;
    save_config(&config)?;
    *state.config.write().await = Arc::new(config);
    Ok(())
}

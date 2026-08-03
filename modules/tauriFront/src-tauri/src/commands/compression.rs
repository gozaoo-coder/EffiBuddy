//! 消息压缩命令：非流式与流式压缩、压缩状态查询与清除、上下文阈值自动压缩。
//!
//! 压缩对用户透明：UI 仍显示原始消息，仅后续 prompt 的历史段应用压缩决策。
//! 流式命令通过 Tauri 事件实时推送进度（status/token/done/error）。
//!
//! 递进压缩模型：未压缩态 → 压缩态1 → 压缩态2 → …（无上限）。
//! 每次压缩基于「上一次压缩后的有效消息」（[`apply_compression`] 结果）进一步压缩，
//! 新决策追加到既有 actions（后者覆盖前者），等级 +1（无上限）。

use std::sync::Arc;

use effisuite_agent::{
    call_compression_agent, call_compression_agent_stream, CompressionStreamItem,
};
use effisuite_core::{
    apply_compression, build_compression_prompt_with_settings, last_reported_input_tokens,
    parse_compression_response, AgentConfig, CompressionAction, CompressionState, CompressionStore,
    ConversationStore, Message,
};
use futures::StreamExt;
use tauri::Emitter;
use tokio::sync::RwLock;

use crate::state::{now_ms, AppState};

/// 压缩输入上下文：完整历史 + 递进压缩输入 + 既有压缩状态
struct EffectiveCompressionInput {
    /// 完整历史（未应用任何压缩决策），用于计算"完全未压缩"基准 token 数
    full: Vec<Message>,
    /// 递进压缩输入：应用既有决策后的「当前压缩态」消息（无压缩时为 full）
    effective: Vec<Message>,
    /// 既有压缩状态（None = 首次压缩）
    prev: Option<CompressionState>,
}

/// 计算压缩 agent 的输入消息（递进压缩核心）：
///
/// 若存在既有压缩状态（已压缩过且含决策），返回应用既有决策后的「当前压缩态」消息
/// （压缩态N → 压缩态N+1）；否则返回原始消息（未压缩态 → 压缩态1）。
/// 同时返回完整历史与既有压缩状态，供 token 指标计算与 [`CompressionState::from_incremental`]。
async fn effective_compression_input(
    store: &ConversationStore,
    compression_store: &CompressionStore,
    conversation_id: &str,
) -> Result<EffectiveCompressionInput, String> {
    let conv = store
        .load(conversation_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("会话 {conversation_id} 不存在"))?;
    if conv.messages.is_empty() {
        return Err("会话无消息，无需压缩".to_string());
    }
    let prev = compression_store
        .load(conversation_id)
        .await
        .map_err(|e| e.to_string())?;
    let full = conv.messages;
    let effective = match &prev {
        Some(state) if !state.actions.is_empty() => apply_compression(&full, state),
        _ => full.clone(),
    };
    Ok(EffectiveCompressionInput { full, effective, prev })
}

/// 执行一次完整压缩的共享核心（非流式）：
/// 加载会话 → 计算有效输入 → 构造 prompt → 调用压缩 agent → 解析 → 合并保存。
///
/// 返回 `(本轮新增 actions, 压缩后总等级, 完全未压缩 token 数, 压缩后 token 数)`。
/// 供 [`compress_messages`] 与 [`run_auto_compress`] 复用，避免两处逻辑分叉。
async fn do_compress_core(
    store: &ConversationStore,
    compression_store: &CompressionStore,
    config: &AgentConfig,
    conversation_id: &str,
) -> Result<(Vec<CompressionAction>, u32, u64, u64), String> {
    if !config.is_rig_ready() {
        return Err("未配置 api_key 或 backend 非 openai，无法调用压缩 agent".to_string());
    }

    // 递进压缩：输入 = 既有压缩态（若有）
    let input = effective_compression_input(store, compression_store, conversation_id).await?;
    let prompt = build_compression_prompt_with_settings(&input.effective, &config.compression_settings);

    let (api_key, base_url, model_name) = config
        .resolve_compression_model()
        .ok_or_else(|| "未配置压缩模型".to_string())?;
    let reply = call_compression_agent(&api_key, &base_url, &model_name, &prompt)
        .await
        .map_err(|e| e.to_string())?;

    let actions = parse_compression_response(&reply).map_err(|e| e.to_string())?;

      // 合并到既有状态：追加 actions、等级 +1（无上限）
    let mut comp_state =
        CompressionState::from_incremental(input.prev.as_ref(), actions.clone(), now_ms());
    // 真实 token 指标：全部取自 API responses 的 usage（MessageUsage.input_tokens）。
    // base = 首次压缩时最近一次 completion 的 prompt_tokens（未压缩上下文真实占用），
    //        递进压缩时由 from_incremental 保留不重算；current = 最近一次 completion
    //        的 prompt_tokens（压缩生效后新消息上报值自然变小，得到真实节省量）。
    if comp_state.base_tokens == 0 {
        comp_state.base_tokens = last_reported_input_tokens(&input.full).unwrap_or(0);
    }
    comp_state.current_tokens = last_reported_input_tokens(&input.full).unwrap_or(0);
    compression_store
        .save(conversation_id, &comp_state)
        .await
        .map_err(|e| e.to_string())?;

    Ok((
        actions,
        comp_state.level,
        comp_state.base_tokens,
        comp_state.current_tokens,
    ))
}

/// 触发消息压缩：调用压缩 agent 分析指定会话，返回压缩操作列表并持久化
///
/// 流程：
/// 1. 从 store 加载会话 + 既有压缩状态
/// 2. 计算有效输入（既有压缩态或原始消息），构造压缩 prompt（每条消息标注 id + 角色 + 内容）
/// 3. 调用压缩 agent（复用主 agent 的 api_key/base_url/model_name + 压缩专用 preamble）
/// 4. 解析 `<act>` 块为 `Vec<CompressionAction>`
/// 5. 与既有状态递进合并（追加 actions / 等级+1）并持久化到 `<appdata>/compression/<id>.json`
/// 6. 返回本轮新增 actions（前端可展示压缩报告）
///
/// 压缩对用户透明：UI 仍显示原始消息，仅后续 prompt 的历史段应用压缩决策。
/// 当前问题（最后一条用户消息）不参与压缩。
#[tauri::command]
pub(crate) async fn compress_messages(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<CompressionAction>, String> {
    // 读取配置快照（Arc clone 廉价，不再深拷贝 AgentConfig）
    let config = state.config.read().await.clone();
    let (actions, level, _base_tokens, _current_tokens) =
        do_compress_core(&state.store, &state.compression_store, &config, &conversation_id)
            .await?;

    tracing::info!(
        conversation_id = %conversation_id,
        action_count = actions.len(),
        level,
        "消息压缩完成并已持久化"
    );
    Ok(actions)
}

/// 压缩 agent 流式事件 payload（emit "agent-compress-token" / "agent-compress-status"
/// / "agent-compress-done" / "agent-compress-error"）
///
/// 设计与 `AgentUsagePayload` 一致：扁平结构 + `serde` 透明序列化，前端 TS
/// 接口一一对应。所有 payload 都携带 `conversation_id` 用于多会话过滤。
#[derive(Debug, Clone, serde::Serialize)]
struct CompressTokenPayload<'a> {
    conversation_id: &'a str,
    /// 本次增量文本
    token: &'a str,
}

#[derive(Debug, Clone, serde::Serialize)]
struct CompressStatusPayload<'a> {
    conversation_id: &'a str,
    /// 当前阶段：loading_conv / building_prompt / streaming / parsing / persisting / done / error
    stage: &'a str,
    /// 阶段说明（人类可读）
    message: &'a str,
}

#[derive(Debug, Clone, serde::Serialize)]
struct CompressDonePayload<'a> {
    conversation_id: &'a str,
    /// 本轮新增的压缩决策列表（非累计）
    actions: &'a [CompressionAction],
    /// 流式累计的完整原始响应文本（含 `<act>` 块）
    raw_text: &'a str,
    /// 处理耗时（毫秒）
    elapsed_ms: u64,
    /// 压缩后总等级：N（无上限），供前端展示「压缩态 N」
    level: u32,
    /// 完全未压缩历史段的真实 token 数（基准）
    base_tokens: u64,
    /// 压缩后的当前有效历史真实 token 数
    current_tokens: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
struct CompressErrorPayload<'a> {
    conversation_id: &'a str,
    error: &'a str,
    /// 失败时已累计的部分文本（可能为空），便于前端展示已接收内容
    partial: &'a str,
}

/// 流式消息压缩命令
///
/// 与 [`compress_messages`] 的区别：
/// - 通过 Tauri 事件实时推送进度，前端在 BindSheet 浮窗展示
/// - 事件流：
///   1. `agent-compress-status`：阶段切换（loading_conv / building_prompt / streaming / parsing / persisting / done）
///   2. `agent-compress-token`：文本增量（仅 streaming 阶段）
///   3. `agent-compress-done`：完成，携带 actions 列表、总等级与耗时
///   4. `agent-compress-error`：失败，携带错误信息与已接收部分文本
/// - 返回值与 [`compress_messages`] 一致（`Vec<CompressionAction>`），便于不关心
///   流式进度的调用方直接使用
///
/// 命令本身在流式完成后才返回；前端若只想要结果可 `await` 命令，
/// 想看进度则监听事件。命令返回即代表 `agent-compress-done` 已 emit。
#[tauri::command]
pub(crate) async fn compress_messages_stream(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    conversation_id: String,
) -> Result<Vec<CompressionAction>, String> {
    let started = std::time::Instant::now();
    let conv_id = conversation_id.clone();
    let emit_status = |stage: &str, message: &str| {
        let _ = app_handle.emit(
            "agent-compress-status",
            &CompressStatusPayload {
                conversation_id: &conv_id,
                stage,
                message,
            },
        );
    };
    let emit_error = |error: &str, partial: &str| {
        let _ = app_handle.emit(
            "agent-compress-error",
            &CompressErrorPayload {
                conversation_id: &conv_id,
                error,
                partial,
            },
        );
    };

    // 1. 加载会话 + 既有压缩状态，计算递进压缩的有效输入
    emit_status("loading_conv", "正在加载会话与压缩状态…");
    let input = match effective_compression_input(
        &state.store,
        &state.compression_store,
        &conversation_id,
    )
    .await
    {
        Ok(v) => v,
        Err(msg) => {
            emit_error(&msg, "");
            return Err(msg);
        }
    };

    // 2. 读取配置快照（Arc clone 廉价，不再深拷贝 AgentConfig）
    let config = state.config.read().await.clone();
    if !config.is_rig_ready() {
        let msg = "未配置 api_key 或 backend 非 openai，无法调用压缩 agent".to_string();
        emit_error(&msg, "");
        return Err(msg);
    }

    // 3. 构造压缩 prompt（基于递进压缩后的有效消息 + 压缩机制设置调整指导语）
    emit_status("building_prompt", "正在构造压缩 prompt…");
    let prompt = build_compression_prompt_with_settings(
        &input.effective,
        &config.compression_settings,
    );

    // 4. 流式调用压缩 agent（优先使用 compression_model_id，回退到 active_model_id）
    emit_status("streaming", "压缩 agent 正在分析…");
    let (api_key, base_url, model_name) = config
        .resolve_compression_model()
        .ok_or_else(|| "未配置压缩模型".to_string())?;
    let mut stream = call_compression_agent_stream(&api_key, &base_url, &model_name, &prompt);

    let mut raw_text = String::with_capacity(1024);
    while let Some(item) = stream.next().await {
        match item {
            Ok(CompressionStreamItem::Token(t)) => {
                raw_text.push_str(&t);
                let _ = app_handle.emit(
                    "agent-compress-token",
                    &CompressTokenPayload {
                        conversation_id: &conv_id,
                        token: &t,
                    },
                );
            }
            Ok(CompressionStreamItem::Done(full)) => {
                // 流结束：full 已是完整文本（与 raw_text 拼接结果一致）
                raw_text = full;
            }
            Err(e) => {
                let msg = e.to_string();
                emit_error(&msg, &raw_text);
                return Err(msg);
            }
        }
    }

    // 5. 解析 <act> 块
    emit_status("parsing", "正在解析压缩决策…");
    let actions = match parse_compression_response(&raw_text) {
        Ok(a) => a,
        Err(e) => {
            let msg = e.to_string();
            emit_error(&msg, &raw_text);
            return Err(msg);
        }
    };
    let action_count = actions.len();

    // 6. 递进合并 + 持久化压缩状态
    emit_status("persisting", "正在持久化压缩状态…");
    let mut comp_state =
        CompressionState::from_incremental(input.prev.as_ref(), actions.clone(), now_ms());
    // 真实 token 指标：全部取自 API responses 的 usage（MessageUsage.input_tokens）。
    // base = 首次压缩时最近一次 completion 的 prompt_tokens（未压缩上下文真实占用），
    //        递进压缩时由 from_incremental 保留不重算；current = 最近一次 completion
    //        的 prompt_tokens（压缩生效后新消息上报值自然变小，得到真实节省量）。
    if comp_state.base_tokens == 0 {
        comp_state.base_tokens = last_reported_input_tokens(&input.full).unwrap_or(0);
    }
    comp_state.current_tokens = last_reported_input_tokens(&input.full).unwrap_or(0);
    if let Err(e) = state
        .compression_store
        .save(&conversation_id, &comp_state)
        .await
    {
        let msg = e.to_string();
        emit_error(&msg, &raw_text);
        return Err(msg);
    }

    // 7. 完成
    let elapsed_ms = started.elapsed().as_millis() as u64;
    emit_status("done", &format!("压缩完成：{action_count} 条决策"));
    let _ = app_handle.emit(
        "agent-compress-done",
        &CompressDonePayload {
            conversation_id: &conv_id,
            actions: &actions,
            raw_text: &raw_text,
            elapsed_ms,
            level: comp_state.level,
            base_tokens: comp_state.base_tokens,
            current_tokens: comp_state.current_tokens,
        },
    );

    tracing::info!(
        conversation_id = %conversation_id,
        action_count,
        elapsed_ms,
        level = comp_state.level,
        "消息压缩完成并已持久化（流式）"
    );
    Ok(actions)
}

/// 上下文阈值自动压缩入口（后台调用，不阻塞主对话流）
///
/// 由 `send_message_stream` 在每次 completion 后检查到上下文达到阈值时触发。
/// 完成时 emit `agent-compress-done`，前端复用既有事件刷新压缩状态并给出提示；
/// 失败仅记录日志（不打断主对话）。内部走与手动压缩相同的 [`do_compress_core`]，
/// 天然实现递进压缩（压缩态N → 压缩态N+1）。
pub(crate) async fn run_auto_compress(
    store: &ConversationStore,
    compression_store: &CompressionStore,
    config: &Arc<RwLock<Arc<AgentConfig>>>,
    app_handle: &tauri::AppHandle,
    conversation_id: &str,
) -> Result<(), String> {
    let started = std::time::Instant::now();
    let cfg = config.read().await.clone();
    let (actions, level, base_tokens, current_tokens) =
        do_compress_core(store, compression_store, &cfg, conversation_id).await?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let _ = app_handle.emit(
        "agent-compress-done",
        &CompressDonePayload {
            conversation_id,
            actions: &actions,
            // 自动压缩为非流式调用，不回传原始输出
            raw_text: "",
            elapsed_ms,
            level,
            base_tokens,
            current_tokens,
        },
    );

    tracing::info!(
        conversation_id = %conversation_id,
        action_count = actions.len(),
        level,
        elapsed_ms,
        "上下文达到阈值，自动压缩完成"
    );
    Ok(())
}

/// 获取指定会话的压缩状态（前端用于展示压缩报告与等级）
#[tauri::command]
pub(crate) async fn get_compression_state(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<Option<CompressionState>, String> {
    let mut comp_state = state
        .compression_store
        .load(&conversation_id)
        .await
        .map_err(|e| e.to_string())?;

    // 真实 token 指标一律取自 API responses 的 usage：从会话历史取最近一次
    // completion 的 prompt_tokens（MessageUsage.input_tokens）。
    // - 旧版本数据 base_tokens=0 时懒回填为最近一次真实上报；
    // - current_tokens 始终跟随最近一次真实上报（压缩生效后新消息上报值自然变小）。
    if let Some(s) = comp_state.as_mut() {
        if !s.actions.is_empty() {
            if let Some(conv) = state
                .store
                .load(&conversation_id)
                .await
                .map_err(|e| e.to_string())?
            {
                if let Some(t) = last_reported_input_tokens(&conv.messages) {
                    let base_was_zero = s.base_tokens == 0;
                    if base_was_zero {
                        s.base_tokens = t;
                    }
                    if base_was_zero || s.current_tokens != t {
                        s.current_tokens = t;
                        state
                            .compression_store
                            .save(&conversation_id, s)
                            .await
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
    }

    Ok(comp_state)
}

/// 清除指定会话的压缩状态（恢复全量历史注入）
#[tauri::command]
pub(crate) async fn clear_compression_state(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<(), String> {
    state
        .compression_store
        .delete(&conversation_id)
        .await
        .map_err(|e| e.to_string())
}

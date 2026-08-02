//! 消息压缩命令：非流式与流式压缩、压缩状态查询与清除。
//!
//! 压缩对用户透明：UI 仍显示原始消息，仅后续 prompt 的历史段应用压缩决策。
//! 流式命令通过 Tauri 事件实时推送进度（status/token/done/error）。

use effisuite_agent::{
    call_compression_agent, call_compression_agent_stream, CompressionStreamItem,
};
use effisuite_core::{
    build_compression_prompt, parse_compression_response, CompressionAction, CompressionState,
};
use futures::StreamExt;
use tauri::Emitter;

use crate::state::{now_ms, AppState};

/// 触发消息压缩：调用压缩 agent 分析指定会话，返回压缩操作列表并持久化
///
/// 流程：
/// 1. 从 store 加载会话（不存在则返回 Err）
/// 2. 构造压缩 prompt（每条消息标注 id + 角色 + 内容）
/// 3. 调用压缩 agent（复用主 agent 的 api_key/base_url/model_name + 压缩专用 preamble）
/// 4. 解析 `<act>` 块为 `Vec<CompressionAction>`
/// 5. 持久化 `CompressionState` 到 `<appdata>/compression/<conversation_id>.json`
/// 6. 返回 actions（前端可展示压缩报告）
///
/// 压缩对用户透明：UI 仍显示原始消息，仅后续 prompt 的历史段应用压缩决策。
/// 当前问题（最后一条用户消息）不参与压缩。
#[tauri::command]
pub(crate) async fn compress_messages(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<CompressionAction>, String> {
    // 1. 加载会话
    let conv = state
        .store
        .load(&conversation_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("会话 {conversation_id} 不存在"))?;

    if conv.messages.is_empty() {
        return Err("会话无消息，无需压缩".to_string());
    }

    // 2. 读取配置快照（Arc clone 廉价，不再深拷贝 AgentConfig）
    let config = state.config.read().await.clone();
    if !config.is_rig_ready() {
        return Err("未配置 api_key 或 backend 非 openai，无法调用压缩 agent".to_string());
    }

    // 3. 构造压缩 prompt
    let prompt = build_compression_prompt(&conv.messages);

    // 4. 调用压缩 agent（优先使用 compression_model_id，回退到 active_model_id）
    let (api_key, base_url, model_name) = config
        .resolve_compression_model()
        .ok_or_else(|| "未配置压缩模型".to_string())?;
    let reply = call_compression_agent(&api_key, &base_url, &model_name, &prompt)
        .await
        .map_err(|e| e.to_string())?;

    // 5. 解析 <act> 块
    let actions = parse_compression_response(&reply).map_err(|e| e.to_string())?;
    let action_count = actions.len();

    // 6. 持久化压缩状态
    let comp_state = CompressionState {
        actions: actions.clone(),
        updated_at: now_ms(),
    };
    state
        .compression_store
        .save(&conversation_id, &comp_state)
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!(
        conversation_id = %conversation_id,
        action_count,
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
    /// 解析得到的压缩决策列表
    actions: &'a [CompressionAction],
    /// 流式累计的完整原始响应文本（含 `<act>` 块）
    raw_text: &'a str,
    /// 处理耗时（毫秒）
    elapsed_ms: u64,
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
///   3. `agent-compress-done`：完成，携带 actions 列表与耗时
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

    // 1. 加载会话
    emit_status("loading_conv", "正在加载会话…");
    let conv = state
        .store
        .load(&conversation_id)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            let _ = app_handle.emit(
                "agent-compress-error",
                &CompressErrorPayload {
                    conversation_id: &conv_id,
                    error: &msg,
                    partial: "",
                },
            );
            msg
        })?
        .ok_or_else(|| {
            let msg = format!("会话 {conversation_id} 不存在");
            let _ = app_handle.emit(
                "agent-compress-error",
                &CompressErrorPayload {
                    conversation_id: &conv_id,
                    error: &msg,
                    partial: "",
                },
            );
            msg
        })?;

    if conv.messages.is_empty() {
        let msg = "会话无消息，无需压缩".to_string();
        let _ = app_handle.emit(
            "agent-compress-error",
            &CompressErrorPayload {
                conversation_id: &conv_id,
                error: &msg,
                partial: "",
            },
        );
        return Err(msg);
    }

    // 2. 读取配置快照（Arc clone 廉价，不再深拷贝 AgentConfig）
    let config = state.config.read().await.clone();
    if !config.is_rig_ready() {
        let msg = "未配置 api_key 或 backend 非 openai，无法调用压缩 agent".to_string();
        let _ = app_handle.emit(
            "agent-compress-error",
            &CompressErrorPayload {
                conversation_id: &conv_id,
                error: &msg,
                partial: "",
            },
        );
        return Err(msg);
    }

    // 3. 构造压缩 prompt
    emit_status("building_prompt", "正在构造压缩 prompt…");
    let prompt = build_compression_prompt(&conv.messages);

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
                let _ = app_handle.emit(
                    "agent-compress-error",
                    &CompressErrorPayload {
                        conversation_id: &conv_id,
                        error: &msg,
                        partial: &raw_text,
                    },
                );
                return Err(msg);
            }
        }
    }

    // 5. 解析 <act> 块
    emit_status("parsing", "正在解析压缩决策…");
    let actions = parse_compression_response(&raw_text).map_err(|e| {
        let msg = e.to_string();
        let _ = app_handle.emit(
            "agent-compress-error",
            &CompressErrorPayload {
                conversation_id: &conv_id,
                error: &msg,
                partial: &raw_text,
            },
        );
        msg
    })?;
    let action_count = actions.len();

    // 6. 持久化压缩状态
    emit_status("persisting", "正在持久化压缩状态…");
    let comp_state = CompressionState {
        actions: actions.clone(),
        updated_at: now_ms(),
    };
    state
        .compression_store
        .save(&conversation_id, &comp_state)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            let _ = app_handle.emit(
                "agent-compress-error",
                &CompressErrorPayload {
                    conversation_id: &conv_id,
                    error: &msg,
                    partial: &raw_text,
                },
            );
            msg
        })?;

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
        },
    );

    tracing::info!(
        conversation_id = %conversation_id,
        action_count,
        elapsed_ms,
        "消息压缩完成并已持久化（流式）"
    );
    Ok(actions)
}

/// 获取指定会话的压缩状态（前端用于展示压缩报告）
#[tauri::command]
pub(crate) async fn get_compression_state(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<Option<CompressionState>, String> {
    state
        .compression_store
        .load(&conversation_id)
        .await
        .map_err(|e| e.to_string())
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

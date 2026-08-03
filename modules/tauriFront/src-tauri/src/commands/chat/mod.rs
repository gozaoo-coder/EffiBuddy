//! 聊天命令（流式 + 非流式）与字符串截断工具。
//!
//! - `send_message`：非流式发送消息，agent.chat 返回完整回复后一次性下发。
//! - `send_message_stream`：spawn 独立 task 驱动流，逐 token emit "agent-token"，
//!   结束 emit "agent-done"；同时透传 reasoning / tool_call / tool_result / usage /
//!   billing / attachment / sub_agent 等事件，流结束时持久化完整助手消息。
//! - `truncate_str`：跨模块复用的字符串截断工具（供 models.rs 远程模型错误信息使用）。
//!
//! 流式事件 payload 定义、token 用量统计与计费汇总见 `payloads` 子模块。

mod payloads;

use std::sync::Arc;

use effisuite_agent::AgentStreamItem;

use effisuite_core::{
    Attachment, BusEvent, AgentConfig, CompressionStore, ConversationStore,
    MemoryIndex, Message, MessageUsage, ModelPricing, Role, SubAgentRecord, ToolCallRecord,
};
use futures::StreamExt;
use tauri::Emitter;
use tokio::sync::RwLock;

use crate::agent::ensure_agent_synced;
use crate::state::{now_ms, AppState};

use payloads::{
    parse_image_gen_output, AgentAttachmentPayload, AgentBillingPayload, AgentReasoningPayload,
    AgentStoppedPayload, AgentToolCallPayload, AgentToolResultPayload, AgentUsagePayload,
    BillingSummary, ConversationTitlePayload, StreamErrorPayload, StreamTokenPayload, UsageSummary,
};

/// 上下文窗口大小回退值（tokens）：激活模型未配置 `context_window_tokens` 时使用，
/// 与前端 useChatCore 的 fallbackContextTokens 保持一致。
const DEFAULT_CONTEXT_WINDOW: u32 = 128000;

/// 每次 completion 后检查上下文是否达到阈值；达到且启用自动压缩时后台触发压缩。
///
/// - `input_tokens`：该次 completion 的 prompt 输入 token 数（即上下文实际占用）
/// - 阈值判定：`input_tokens >= context_window * threshold_percent / 100`
/// - 触发后 spawn 独立 task 执行压缩（不阻塞当前流）；`inflight` 集合防止
///   同一会话并发重复触发（多 completion / 多轮次之间去重）
/// - 压缩本身复用 [`crate::commands::run_auto_compress`]，内部走递进压缩核心：
///   基于当前压缩态进一步压缩（逐级递进，无上限）
///
/// 返回 `true` 表示本次确实触发了自动压缩（调用方据此做流级去重：一条流只压缩一次），
/// 返回 `false` 表示被任一守卫拦截（未启用 / 未达阈值 / 已在压缩中）。
async fn maybe_auto_compress(
    store: Arc<ConversationStore>,
    compression_store: CompressionStore,
    config: Arc<RwLock<Arc<AgentConfig>>>,
    inflight: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    app_handle: tauri::AppHandle,
    conversation_id: String,
    input_tokens: u64,
) -> bool {
    // 1. 快速路径：读取配置快照，检查自动压缩开关与阈值
    let (window, threshold) = {
        let cfg = config.read().await;
        if !cfg.compression_settings.auto_compress {
            return false;
        }
        let window = cfg
            .active_model_id
            .as_ref()
            .and_then(|id| cfg.models.iter().find(|m| &m.id == id))
            .and_then(|m| m.context_window_tokens)
            .unwrap_or(DEFAULT_CONTEXT_WINDOW)
            .max(1) as u64;
        let threshold = cfg.compression_settings.threshold_percent.clamp(1, 100) as u64;
        (window, threshold)
    };
    if input_tokens < window.saturating_mul(threshold) / 100 {
        return false;
    }


    // 2. in-flight 去重：同一会话已有自动压缩在跑则跳过
    {
        let mut guard = inflight.lock().unwrap();
        if !guard.insert(conversation_id.clone()) {
            tracing::debug!(conversation_id = %conversation_id, "自动压缩已在进行中，跳过");
            return false;
        }
    }

    // 3. 后台执行压缩（不阻塞当前流）；完成后移除 in-flight 标记
    tauri::async_runtime::spawn(async move {
        let result = crate::commands::run_auto_compress(
            &store,
            &compression_store,
            &config,
            &app_handle,
            &conversation_id,
        )
        .await;
        inflight.lock().unwrap().remove(&conversation_id);
        if let Err(e) = result {
            tracing::warn!(conversation_id = %conversation_id, error = %e, "上下文自动压缩失败");
        }
    });
    true
}

/// 需要触发自动快照（会话版本管理）的文件写工具名集合
fn is_file_write_tool(name: &str) -> bool {
    matches!(name, "edit_file" | "edit_file_regex" | "write_file" | "delete_file")
}

/// 文件写工具名 → 快照备注文案（时间线可读性好一些）
fn file_tool_label(name: &str) -> String {
    let label = match name {
        "edit_file" => "编辑文件",
        "edit_file_regex" => "批量编辑",
        "write_file" => "写入文件",
        "delete_file" => "删除文件",
        _ => "文件操作",
    };
    format!("{label} · 自动保存")
}
#[tauri::command]
pub(crate) async fn send_message(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    conversation_id: String,
    content: String,
) -> Result<String, String> {
    // agent 工具（manage_model）可能已修改配置：版本不一致时懒重建
    ensure_agent_synced(&state, &app_handle).await;
    let agent = state.agent.read().await.clone();
    let store = state.store.clone();
    let bus = state.event_bus.clone();
    let memory = Arc::clone(&state.memory);
    let cur_conv = Arc::clone(&state.current_conversation_id);
    let working_dir_handle = Arc::clone(&state.working_dir);

    // 标记当前会话：agent 据此排除当前会话，避免与已注入上下文重复
    *cur_conv.write().await = Some(conversation_id.clone());

    // 同步会话级工作区到 agent 句柄：read_file/list_files/shell 据此解析相对路径
    // 优先级：会话级 working_dir > 技能级（enable_skill 工具写入会话） > None
    let conv_wd = store
        .load(&conversation_id)
        .await
        .map_err(|e| e.to_string())?
        .and_then(|c| c.working_dir)
        .map(std::path::PathBuf::from);
    *working_dir_handle.write().await = conv_wd;

    // 先把用户消息持久化到 store，同时取回完整历史
    let user_msg = Message::new(
        effisuite_core::gen_message_id(),
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
        effisuite_core::gen_message_id(),
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
pub(crate) async fn send_message_stream(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    conversation_id: String,
    content: String,
) -> Result<(), String> {
    // agent 工具（manage_model）可能已修改配置：版本不一致时懒重建
    ensure_agent_synced(&state, &app_handle).await;
    let agent = state.agent.read().await.clone();
    let store = state.store.clone();
    let memory = Arc::clone(&state.memory);
    let cur_conv = Arc::clone(&state.current_conversation_id);
    let working_dir_handle = Arc::clone(&state.working_dir);
    let handle = app_handle.clone();
    // 子 agent 事件累积缓冲：emitter 按 conversation_id 实时写入，流结束时取走持久化
    let sub_agent_records = Arc::clone(&state.sub_agent_records);
    // 流开始前清空该会话上一轮的缓冲（防残留；正常流程上次流结束已取走）
    sub_agent_records.lock().unwrap().remove(&conversation_id);
    // 生成期间排队消息队列：queue_user_message 命令写入，rig hook / 续接循环消费
    let pending_msgs = Arc::clone(&state.pending_user_messages);
    // Agent 运行取消注册表：流开始前注册本会话取消令牌（覆盖旧令牌，保证单流）。
    // 驱动 task 在流式循环里轮询 receiver；stop_agent 命令触发 cancel 即终止。
    let cancel_registry = Arc::clone(&state.agent_cancel);
    let mut cancel_rx = cancel_registry.register(&conversation_id);
    // 标记当前会话：agent 据此排除当前会话
    *cur_conv.write().await = Some(conversation_id.clone());

    // 1. 持久化用户消息并取回完整历史
    let user_msg = Message::new(
        effisuite_core::gen_message_id(),
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
    // 优先级：会话级 working_dir > 技能级（enable_skill 工具写入会话） > None
    let conv_wd = conv.working_dir.clone().map(std::path::PathBuf::from);
    *working_dir_handle.write().await = conv_wd;

    // 初始历史；续接轮（生成期间排队消息）会重新从 store 加载最新历史
    let mut history = conv.history().to_vec();
    let conv_id = conversation_id.clone();

    // 计费所需：模型名 + 激活模型预设的计费单价（用户配置，非硬编码）。
    // 在 spawn 前读取，避免在异步 task 内持有 RwLock。
    let model_name = agent.name().to_string();
    let pricing: Option<ModelPricing> = {
        let cfg = state.config.read().await;
        cfg.active_model_id
            .as_ref()
            .and_then(|id| cfg.models.iter().find(|m| m.id.as_str() == id.as_str()))
            .and_then(|m| m.pricing)
    };

    // 自动压缩所需句柄：压缩状态存储 + 配置快照 + in-flight 去重集合。
    // 每次 completion 后检查上下文是否达到阈值，达到则在后台触发递进压缩。
    let compression_store = state.compression_store.clone();
    let config_arc = Arc::clone(&state.config);
    let auto_compress_inflight = Arc::clone(&state.auto_compress_inflight);


    // 2. spawn 独立 task 驱动流
    // 续接循环：一轮 agent run 结束后若队列仍有排队消息（未在下一个 completion 前被
    // hook 消费），则清空队列、从 store 重建最新历史并启动新的一轮，直到队列为空。
    tauri::async_runtime::spawn(async move {
        // 全部轮次正文拼接（agent-done 事件回传用；持久化按轮次拆分，不拼接）
        let mut full = String::with_capacity(256);
        // 累计所有轮次（含续接轮）所有 completion 的 token 使用统计，最终一次计费
        let mut usage_summary = UsageSummary::default();
        // 最近一次 completion 的 prompt 输入 token 数（上下文实际占用），用于每次
        // completion 后检查是否达到自动压缩阈值。
        let mut last_input_tokens: Option<u64> = None;
        // 流级去重标志：本流（send_message_stream 一次调用，含续接轮）内最多触发一次
        // 自动压缩。置位后后续 completion 不再触发，避免一条流内连续弹出"压缩成功"。
        let mut auto_compressed = false;

        // 每次 completion 落盘后调用：若上下文达到阈值则后台触发自动压缩。
        // 复用模块级 maybe_auto_compress（阈值判定 + MAX 等级守卫 + in-flight 去重
        // + spawn 后台压缩）。触发成功后置位 auto_compressed，本流后续不再重复触发。
        macro_rules! auto_compress_check {
            () => {
                if !auto_compressed {
                    if let Some(tok) = last_input_tokens {
                        if maybe_auto_compress(
                            store.clone(),
                            compression_store.clone(),
                            config_arc.clone(),
                            auto_compress_inflight.clone(),
                            handle.clone(),
                            conv_id.clone(),
                            tok,
                        )
                        .await
                        {
                            auto_compressed = true;
                        }
                    }
                }
            };
        }

        // 回答结束时下发一次计费统计（agent-billing）。
        // 成功路径在 agent-done 前调用；错误路径在 return 前调用。
        let emit_billing = |summary: &UsageSummary| {
            if summary.completion_count == 0 {
                return;
            }
            let b = BillingSummary::compute(summary, pricing);
            let _ = handle.emit(
                "agent-billing",
                &AgentBillingPayload {
                    conversation_id: &conv_id,
                    model_name: &model_name,
                    rounds: b.rounds,
                    cache_hit_tokens: b.cache_hit_tokens,
                    cache_miss_tokens: b.cache_miss_tokens,
                    output_tokens: b.output_tokens,
                    reasoning_tokens: b.reasoning_tokens,
                    total_tokens: b.total_tokens,
                    priced: b.priced,
                    cache_hit_cost: b.cache_hit_cost,
                    cache_miss_cost: b.cache_miss_cost,
                    output_cost: b.output_cost,
                    total_cost: b.total_cost,
                },
            );
        };

        loop {
            // ---- 续接排队消息 ----
            // 队列里有消息 → 已被 queue_user_message 持久化到 store（先写 store 再入队）。
            // 清空队列并从 store 重建最新历史：最后一条用户消息成为新一轮的当前问题，
            // 模型下一轮直接回应；未消费的排队消息（run 已结束才到这里的）不会丢失。
            if pending_msgs.has_pending(&conv_id) {
                pending_msgs.clear(&conv_id);
                match store.load(&conv_id).await {
                    Ok(Some(conv)) => history = conv.history().to_vec(),
                    Ok(None) => {
                        tracing::warn!(conv_id = %conv_id, "续接轮：会话不存在，终止");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e, conv_id = %conv_id,
                            "续接轮：加载会话失败，沿用旧历史"
                        );
                    }
                }
            }

            let mut stream = agent.chat_stream(&history);

            // ---- 当前「一次 completion」轮次的累积内容 ----
            // 多次 Completions（多轮对话：模型多次调用 LLM，中间穿插工具调用）不应在持久化
            // 存储里合并成一条消息。这里按 completion 边界把每一轮的内容单独落盘为一条
            // assistant 消息：正文 / 推理 / 工具调用及结果 / 图片附件 / 本轮 usage 归一轮。
            let mut turn_text = String::with_capacity(256);
            let mut turn_reasoning: Option<String> = None;
            let mut turn_tool_calls: Vec<ToolCallRecord> = Vec::new();
            let mut turn_images: Vec<Attachment> = Vec::new();
            let mut turn_usage: Option<MessageUsage> = None;
            // 是否已收到当前轮次的 CompletionCall（本轮模型输出已结束）。
            // 置位后下一条文本/推理（或连续的下一个 Usage）即属于新的 completion 轮次，
            // 应先把当前轮次落盘再开始累积；工具调用与其结果属于同一轮次，不触发边界。
            let mut turn_complete = false;
            // 跟踪 call_id → tool_name 映射，用于在 ToolResult 时判断是否为 image_gen / set_title
            let mut tool_call_names: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            // 跟踪 call_id → arguments 映射，set_title 结果到达时解析 title 字段
            let mut tool_call_args: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();

            let mut stopped = false;

            loop {
                let chunk = tokio::select! {
                    biased;
                    // 取消信号（用户点击停止 / 旧令牌被覆盖或注销）→ 立即终止
                    _ = cancel_rx.changed() => {
                        stopped = true;
                        break;
                    }
                    c = stream.next() => c,
                };
                let Some(chunk) = chunk else { break };
                match chunk {
                    Ok(AgentStreamItem::Text { content }) => {
                        // 上一轮次已完成：本轮文本属于新 completion，先把上一轮次落盘
                        if turn_complete {
                            persist_assistant_turn(
                                &store, &memory, &conv_id, &mut turn_text, &mut turn_reasoning,
                                &mut turn_tool_calls, &mut turn_images, &mut turn_usage, None,
                            )
                            .await;
                            turn_complete = false;
                            auto_compress_check!();
                        }
                        full.push_str(&content);
                        turn_text.push_str(&content);
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
                        // 上一轮次已完成：新一轮思考属于新 completion，先落盘上一轮次
                        if turn_complete {
                            persist_assistant_turn(
                                &store, &memory, &conv_id, &mut turn_text, &mut turn_reasoning,
                                &mut turn_tool_calls, &mut turn_images, &mut turn_usage, None,
                            )
                            .await;
                            turn_complete = false;
                            auto_compress_check!();
                        }
                        turn_reasoning.get_or_insert_with(String::new).push_str(&content);
                        let _ = handle.emit(
                            "agent-reasoning",
                            &AgentReasoningPayload {
                                conversation_id: &conv_id,
                                content: &content,
                            },
                        );
                    }
                    Ok(AgentStreamItem::ToolCallStart {
                        call_id,
                        tool_name,
                        arguments,
                    }) => {
                        // 工具调用属于当前轮次（无论本轮模型输出是否已结束），不触发边界拆分
                        tool_call_names.insert(call_id.clone(), tool_name.clone());
                        let args_str =
                            serde_json::to_string(&arguments).unwrap_or_else(|_| "null".to_string());
                        // 记录 call_id → args_str，供 set_title 结果到达时解析 title 字段
                        tool_call_args.insert(call_id.clone(), args_str.clone());
                        // 记录到持久化列表（result 待 ToolResult 到达后填充）
                        turn_tool_calls.push(ToolCallRecord {
                            call_id: call_id.clone(),
                            tool_name: tool_name.clone(),
                            arguments: args_str.clone(),
                            result: String::new(),
                            is_error: false,
                        });
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
                    Ok(AgentStreamItem::ToolResult {
                        call_id,
                        output,
                        is_error,
                    }) => {
                        // 若为 image_gen / display_image 工具结果，解析 JSON 提取图片信息并收集为附件。
                        // 两者输出格式兼容（id/path/name），display_image 额外有 source 字段不影响解析。
                        if let Some(name) = tool_call_names.get(&call_id) {
                            if (name == "image_gen" || name == "display_image") && !is_error {
                                if let Some(att) = parse_image_gen_output(&output) {
                                    // 实时通知前端有新图片，可立即渲染
                                    let _ = handle.emit(
                                        "agent-attachment",
                                        &AgentAttachmentPayload {
                                            conversation_id: &conv_id,
                                            attachment: &att,
                                        },
                                    );
                                    turn_images.push(att);
                                }
                            } else if name == "set_title" && !is_error {
                                // set_title 工具已自行调用 store.rename 持久化标题。
                                // 这里从 arguments 解析 title，emit 事件让前端立即刷新 SideNav，
                                // 不必等流结束。解析失败则静默（流结束的 conversation-changed 仍会刷新）。
                                if let Some(title) = tool_call_args
                                    .get(&call_id)
                                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                                    .and_then(|v| {
                                        v.get("title").and_then(|t| t.as_str()).map(str::to_string)
                                    })
                                {
                                    let _ = handle.emit(
                                        "conversation-title-updated",
                                        &ConversationTitlePayload {
                                            conversation_id: &conv_id,
                                            title: &title,
                                        },
                                    );
                                }
                            } else if name == "install_clawhub_skill" && !is_error {
                                // agent 主动调用 install_clawhub_skill 工具成功：
                                // emit 事件让 ClawHubPanel / SkillPanel 同步刷新已安装列表。
                                // 工具内部已 rebuild SkillIndex，前端只需重新拉取 list_skills。
                                let _ = handle.emit("clawhub-skill-installed", &());
                            }
                        }
                        // 会话版本管理：文件写工具（edit_file / write_file 等）执行成功
                        // → 后台自动保存当前工作区快照，便于随时撤回/回溯。
                        // 快照由自研引擎存到应用私有目录，不触碰工作区 git 仓库。
                        if !is_error {
                            if let Some(name) = tool_call_names.get(&call_id) {
                                if is_file_write_tool(name) {
                                    if let Some(wd) = working_dir_handle.read().await.clone() {
                                        let snap_conv = conv_id.clone();
                                        let snap_msg = file_tool_label(name);
                                        let snap_handle = handle.clone();
                                        tauri::async_runtime::spawn(async move {
                                            let res =
                                                tauri::async_runtime::spawn_blocking(move || {
                                                    crate::snapshot_service::save_snapshot(
                                                        &snap_conv,
                                                        &wd,
                                                        &snap_msg,
                                                        crate::snapshot_service::
                                                            SnapshotSource::Auto,
                                                    )
                                                })
                                                .await;
                                            match res {
                                                Ok(Ok(Some(meta))) => {
                                                    let _ = snap_handle
                                                        .emit("session-snapshot-saved", &meta);
                                                }
                                                Ok(Ok(None)) => {}
                                                Ok(Err(e)) => {
                                                    tracing::warn!(error = %e, "自动快照保存失败")
                                                }
                                                Err(e) => {
                                                    tracing::warn!(error = %e, "自动快照任务失败")
                                                }
                                            }
                                        });
                                    }
                                }
                            }
                        }
                        // 填充当前轮次工具调用记录的执行结果
                        if let Some(rec) = turn_tool_calls.iter_mut().find(|r| r.call_id == call_id) {
                            rec.result = output.clone();
                            rec.is_error = is_error;
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
                    Ok(AgentStreamItem::Usage {
                        input_tokens,
                        output_tokens,
                        total_tokens,
                        reasoning_tokens,
                        cache_hit_tokens,
                        cache_miss_tokens,
                    }) => {
                        // 上一次 completion 已结束且内容尚未落盘（如连续的工具轮次）：
                        // 先持久化上一轮次，再开启新一轮次。
                        if turn_complete {
                            persist_assistant_turn(
                                &store, &memory, &conv_id, &mut turn_text, &mut turn_reasoning,
                                &mut turn_tool_calls, &mut turn_images, &mut turn_usage, None,
                            )
                            .await;
                            // （落盘后 turn_complete 会在下方重新置位，这里无需复位）
                            auto_compress_check!();
                        }
                        // 透传单次 completion 的 token 使用统计。
                        // 前端累计所有 Usage 事件得到本轮总消耗，显示在底栏。
                        // 同时累计到 usage_summary，供回答结束时计算计费（agent-billing）。
                        usage_summary.input_tokens += input_tokens;
                        usage_summary.output_tokens += output_tokens;
                        usage_summary.total_tokens += total_tokens;
                        usage_summary.reasoning_tokens += reasoning_tokens;
                        usage_summary.cache_hit_tokens += cache_hit_tokens;
                        usage_summary.cache_miss_tokens += cache_miss_tokens;
                        usage_summary.completion_count += 1;
                        // 当前轮次 usage 快照（落盘到本轮消息，rounds=1）。
                        // cache_miss 未上报时用 input - cache_hit 推导（与 BillingSummary::compute 一致）
                        let cache_miss = if cache_miss_tokens > 0 {
                            cache_miss_tokens
                        } else {
                            input_tokens.saturating_sub(cache_hit_tokens)
                        };
                        turn_usage = Some(MessageUsage {
                            input_tokens,
                            output_tokens,
                            total_tokens: cache_hit_tokens + cache_miss + output_tokens,
                            reasoning_tokens,
                            cache_hit_tokens,
                            cache_miss_tokens: cache_miss,
                            rounds: 1,
                        });
                        turn_complete = true;
                        last_input_tokens = Some(input_tokens);
                        let _ = handle.emit(
                            "agent-usage",
                            &AgentUsagePayload {
                                conversation_id: &conv_id,
                                input_tokens,
                                output_tokens,
                                total_tokens,
                                reasoning_tokens,
                                // 累计值一并下发，前端可直接用累计值显示
                                cumulative_input: usage_summary.input_tokens,
                                cumulative_output: usage_summary.output_tokens,
                                cumulative_total: usage_summary.total_tokens,
                                cumulative_reasoning: usage_summary.reasoning_tokens,
                            },
                        );
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "stream error");
                        // 异常路径也要持久化已产生的部分回复与思考：
                        // 避免推理模型流式中断后，思考过程完全不保存到会话上下文。
                        // 仅当确有内容（正文/推理/工具/图片/子 agent）时才落盘，纯错误则跳过。
                        let sub_agents = sub_agent_records.lock().unwrap().remove(&conv_id);
                        let has_partial = !turn_text.is_empty()
                            || turn_reasoning.as_ref().map_or(false, |s| !s.is_empty())
                            || !turn_tool_calls.is_empty()
                            || !turn_images.is_empty()
                            || sub_agents.as_ref().map_or(false, |v| !v.is_empty());
                        if has_partial {
                            persist_assistant_turn(
                                &store, &memory, &conv_id, &mut turn_text, &mut turn_reasoning,
                                &mut turn_tool_calls, &mut turn_images, &mut turn_usage, sub_agents,
                            )
                            .await;
                        }
                        // 回答结束（异常路径）：如有已消耗的 token，同样下发计费统计
                        emit_billing(&usage_summary);
                        let _ = handle.emit(
                            "agent-stream-error",
                            &StreamErrorPayload {
                                conversation_id: &conv_id,
                                error: &e.to_string(),
                            },
                        );
                            // 移除本会话取消令牌，避免残留
                            cancel_registry.unregister(&conv_id);
                            return;
                    }
                }
            }

            // 被用户手动停止：持久化已产生的部分内容，emit agent-stopped 后终止整个驱动 task
            if stopped {
                // 先 drop 流：取消 rig 内部在途的工具 / 子 agent future（sub_agent 工具在流内
                // await，drop 即取消其后续执行，避免后台继续烧 token / 改文件）
                drop(stream);
                let sub_agents = sub_agent_records.lock().unwrap().remove(&conv_id);
                let has_partial = !turn_text.is_empty()
                    || turn_reasoning.as_ref().map_or(false, |s| !s.is_empty())
                    || !turn_tool_calls.is_empty()
                    || !turn_images.is_empty()
                    || sub_agents.as_ref().map_or(false, |v| !v.is_empty());
                if has_partial {
                    persist_assistant_turn(
                        &store, &memory, &conv_id, &mut turn_text, &mut turn_reasoning,
                        &mut turn_tool_calls, &mut turn_images, &mut turn_usage, sub_agents,
                    )
                    .await;
                }
                // 排队消息不再消费（消息已持久化在 store，仍留在会话历史里，下次发送会并入）
                pending_msgs.clear(&conv_id);
                // 结束计费（若已产生 token 消耗）
                emit_billing(&usage_summary);
                cancel_registry.unregister(&conv_id);
                let _ = handle.emit(
                    "agent-stopped",
                    &AgentStoppedPayload {
                        conversation_id: &conv_id,
                        content: &full,
                    },
                );
                return;
            }

            // 流结束，持久化最后一轮（含子 agent 过程记录、图片附件等）
            let sub_agents = sub_agent_records.lock().unwrap().remove(&conv_id);
            persist_assistant_turn(
                &store, &memory, &conv_id, &mut turn_text, &mut turn_reasoning, &mut turn_tool_calls,
                &mut turn_images, &mut turn_usage, sub_agents,
            )
            .await;
            auto_compress_check!();

            // 若队列仍有排队消息 → 继续下一轮（把它们并入最新历史再生成）
            if !pending_msgs.has_pending(&conv_id) {
                // 回答结束（成功路径）：先下发本轮计费统计，再通知前端流结束
                emit_billing(&usage_summary);
                let _ = handle.emit(
                    "agent-done",
                    &StreamTokenPayload {
                        conversation_id: &conv_id,
                        content: &full,
                        done: true,
                    },
                );
                    // 流自然结束：移除本会话取消令牌
                    cancel_registry.unregister(&conv_id);
                    return;
            }
        }
    });


    Ok(())
}

/// 生成期间排队：AI 仍在生成时用户发送的新消息。
///
/// 区别于 `send_message_stream`（只能在没有活动流时调用，否则会产生并发流）：
/// 本命令**先**把用户消息持久化到会话存储（用户可见、可回看、可参与续接轮历史），
/// **再** push 到注入队列。消费方二选一（消息不会重复处理）：
/// 1. rig hook 在「下一个 completion」前 drain 并注入模型输入（中途打断/转向）；
/// 2. `send_message_stream` 的续接循环在当前 run 结束后把未消费消息并入新的一轮。
#[tauri::command]
pub(crate) async fn queue_user_message(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
    content: String,
) -> Result<(), String> {
    let store = state.store.clone();
    let memory = Arc::clone(&state.memory);
    let pending = Arc::clone(&state.pending_user_messages);
    let cur_conv = Arc::clone(&state.current_conversation_id);

    // 标记当前会话：hook 据此确定要注入哪个会话的排队消息
    *cur_conv.write().await = Some(conversation_id.clone());

    // 先持久化到会话存储（保证用户可见、重启后仍可回看；续接轮重建历史依赖此）
    let user_msg = Message::new(
        effisuite_core::gen_message_id(),
        Role::User,
        content.clone(),
        now_ms(),
    );
    let user_msg_for_memory = user_msg.clone();
    store
        .append_message(&conversation_id, user_msg, now_ms())
        .await
        .map_err(|e| e.to_string())?;
    memory.add(&conversation_id, user_msg_for_memory).await;

    // 再入队：先写 store 再 push，消费方（hook / 续接循环）可从 store 重建完整历史
    pending.push(&conversation_id, content);
    Ok(())
}
/// 停止指定会话正在运行的 agent（用户点击「停止」按钮）。
///
/// 触发 `send_message_stream` 驱动 task 的取消信号：流在下一个 chunk 边界
/// （或 select 立即）感知并终止，已产生的部分内容照常持久化，随后 emit
/// "agent-stopped" 事件。返回是否确实终止了活动流（无活动流时为 false）。
#[tauri::command]
pub(crate) async fn stop_agent(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<bool, String> {
    Ok(state.agent_cancel.cancel(&conversation_id))
}

/// 持久化当前模型轮次（一次 completion）的内容为一条助手消息。
///
/// 多次 Completions 的多轮对话在持久化存储中应按 completion 边界拆分为多条
/// assistant 消息，而不是合并成一条。本函数负责把「一轮」的内容（正文 / 推理 /
/// 工具调用及结果 / 图片附件 / 本轮 usage）落盘为一条消息。
///
/// 入参为各累积器的可变引用，无论是否实际落盘都会清空，以便下一轮次复用；
/// `sub_agents` 仅在最终轮次（流结束 / 异常路径）传入，归属到最后一条消息。
/// 返回是否实际写入了消息（全部为空时跳过，避免空消息污染历史）。
async fn persist_assistant_turn(
    store: &ConversationStore,
    memory: &MemoryIndex,
    conv_id: &str,
    text: &mut String,
    reasoning: &mut Option<String>,
    tool_calls: &mut Vec<ToolCallRecord>,
    attachments: &mut Vec<Attachment>,
    usage: &mut Option<MessageUsage>,
    sub_agents: Option<Vec<SubAgentRecord>>,
) -> bool {
    let has_content = !text.is_empty()
        || reasoning.as_ref().map_or(false, |s| !s.is_empty())
        || !tool_calls.is_empty()
        || !attachments.is_empty()
        || sub_agents.as_ref().map_or(false, |v| !v.is_empty());

    if !has_content {
        text.clear();
        reasoning.take();
        tool_calls.clear();
        attachments.clear();
        usage.take();
        return false;
    }

    let mut msg = Message::new(
        effisuite_core::gen_message_id(),
        Role::Assistant,
        std::mem::take(text),
        now_ms(),
    );
    if let Some(r) = reasoning.take() {
        if !r.is_empty() {
            msg.reasoning = Some(r);
        }
    }
    msg.tool_calls = std::mem::take(tool_calls);
    msg.attachments = std::mem::take(attachments);
    msg.usage = usage.take();
    if let Some(recs) = sub_agents {
        if !recs.is_empty() {
            msg.sub_agents = recs;
        }
    }
    let msg_for_memory = msg.clone();
    if let Err(e) = store.append_message(conv_id, msg, now_ms()).await {
        tracing::warn!(error = %e, "persist assistant turn failed");
    }
    memory.add(conv_id, msg_for_memory).await;
    true
}

/// 截断字符串到最大字符数（按 char 边界），附加 … 省略号
pub(crate) fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut s = s.chars().take(max_chars).collect::<String>();
    s.push('…');
    s
}

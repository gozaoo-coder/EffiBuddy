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
    Attachment, BusEvent, Message, MessageUsage, ModelPricing, Role, ToolCallRecord,
};
use futures::StreamExt;
use tauri::Emitter;

use crate::agent::ensure_agent_synced;
use crate::state::{now_ms, AppState};

use payloads::{
    parse_image_gen_output, AgentAttachmentPayload, AgentBillingPayload, AgentReasoningPayload,
    AgentToolCallPayload, AgentToolResultPayload, AgentUsagePayload, BillingSummary,
    ConversationTitlePayload, StreamErrorPayload, StreamTokenPayload, UsageSummary,
};

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
    // 优先级：会话级 working_dir > 技能级（enable_skill 工具写入会话） > None
    let conv_wd = conv.working_dir.clone().map(std::path::PathBuf::from);
    *working_dir_handle.write().await = conv_wd;

    let history = conv.history().to_vec();
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

    // 2. spawn 独立 task 驱动流
    tauri::async_runtime::spawn(async move {
        let mut stream = agent.chat_stream(&history);
        let mut full = String::with_capacity(256);
        // 累积本轮推理文本（thinking），流结束后持久化到助手消息，供历史回看
        let mut reasoning_full = String::new();
        // 累积本轮工具调用记录（call_id → 参数/结果），流结束后持久化
        let mut tool_call_records: Vec<ToolCallRecord> = Vec::new();
        // 跟踪 call_id → tool_name 映射，用于在 ToolResult 时判断是否为 image_gen / set_title
        let mut tool_call_names: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        // 跟踪 call_id → arguments 映射，set_title 结果到达时解析 title 字段
        let mut tool_call_args: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        // 收集 image_gen 工具生成的图片附件，流结束后注入到助手消息
        let mut image_attachments: Vec<Attachment> = Vec::new();
        // 累计本轮所有 completion 的 token 使用统计，agent-done 时一并下发
        let mut usage_summary = UsageSummary::default();

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
                    total_tokens: b.total_tokens,
                    priced: b.priced,
                    cache_hit_cost: b.cache_hit_cost,
                    cache_miss_cost: b.cache_miss_cost,
                    output_cost: b.output_cost,
                    total_cost: b.total_cost,
                },
            );
        };

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
                    reasoning_full.push_str(&content);
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
                    // 记录 call_id → tool_name，供 ToolResult 时判断是否为 image_gen / set_title
                    tool_call_names.insert(call_id.clone(), tool_name.clone());
                    let args_str =
                        serde_json::to_string(&arguments).unwrap_or_else(|_| "null".to_string());
                    // 记录 call_id → args_str，供 set_title 结果到达时解析 title 字段
                    tool_call_args.insert(call_id.clone(), args_str.clone());
                    // 记录到持久化列表（result 待 ToolResult 到达后填充）
                    tool_call_records.push(ToolCallRecord {
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
                                image_attachments.push(att);
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
                    // 填充持久化工具调用记录的执行结果
                    if let Some(rec) = tool_call_records.iter_mut().find(|r| r.call_id == call_id) {
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
                    // 回答结束（异常路径）：如有已消耗的 token，同样下发计费统计
                    emit_billing(&usage_summary);
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
        // 持久化推理文本 / 工具调用 / token 用量：重启后历史回看仍可见
        if !reasoning_full.is_empty() {
            assistant_msg.reasoning = Some(reasoning_full);
        }
        assistant_msg.tool_calls = tool_call_records;
        if usage_summary.completion_count > 0 {
            // cache_miss 未上报时用 input - cache_hit 推导（与 BillingSummary::compute 一致）
            let cache_miss = if usage_summary.cache_miss_tokens > 0 {
                usage_summary.cache_miss_tokens
            } else {
                usage_summary
                    .input_tokens
                    .saturating_sub(usage_summary.cache_hit_tokens)
            };
            assistant_msg.usage = Some(MessageUsage {
                input_tokens: usage_summary.input_tokens,
                output_tokens: usage_summary.output_tokens,
                total_tokens: usage_summary.cache_hit_tokens
                    + cache_miss
                    + usage_summary.output_tokens,
                reasoning_tokens: usage_summary.reasoning_tokens,
                cache_hit_tokens: usage_summary.cache_hit_tokens,
                cache_miss_tokens: cache_miss,
                rounds: usage_summary.completion_count,
            });
        }
        // 子 agent 过程记录：从累积缓冲取走并持久化，重启后历史回看可恢复卡片
        if let Some(recs) = sub_agent_records.lock().unwrap().remove(&conv_id) {
            if !recs.is_empty() {
                assistant_msg.sub_agents = recs;
            }
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

        // 4. 回答结束（成功路径）：先下发本轮计费统计，再通知前端流结束
        emit_billing(&usage_summary);
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

/// 截断字符串到最大字符数（按 char 边界），附加 … 省略号
pub(crate) fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut s = s.chars().take(max_chars).collect::<String>();
    s.push('…');
    s
}

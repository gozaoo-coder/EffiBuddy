//! 事件转发与子 agent 事件累积。
//!
//! - `forward_event`：把内部 `BusEvent` 转发为前端可监听的 Tauri 事件。
//! - `accumulate_sub_agent_event`：把子 agent 事件累积到会话缓冲，
//!   供 `send_message_stream` 流结束时持久化为 `SubAgentRecord`。

use std::sync::Arc;

use effisuite_agent::{SubAgentEvent, SubAgentEventKind};
use effisuite_core::{BusEvent, SubAgentImage, SubAgentRecord, ToolCallRecord};
use tauri::Emitter;

use crate::state::now_ms;

/// 把 `BusEvent` 转发为前端可监听的 Tauri 事件。
/// payload 直接使用 `BusEvent` 本身（已实现 Serialize，带 `kind` 标签）。
pub(crate) fn forward_event(handle: &tauri::AppHandle, event: &BusEvent) {
    let (name, payload) = match event {
        BusEvent::AgentStreamToken { .. } => ("agent-token", event),
        BusEvent::AgentMessage { .. } => ("agent-message", event),
        BusEvent::DeviceFound { .. } => ("device-found", event),
        BusEvent::DeviceStatusChanged { .. } => ("device-status-changed", event),
        BusEvent::PairingRequest { .. } => ("pairing-request", event),
        BusEvent::AskUser { .. } => ("ask-user", event),
        BusEvent::NotifyUser { .. } => ("notify-user", event),
        BusEvent::OpenPreview { .. } => ("open-preview", event),
        BusEvent::AsrStreamChunk { .. } => ("asr-stream-chunk", event),
        BusEvent::AsrSessionStatus { .. } => ("asr-session-status", event),
        BusEvent::AsrUploadProgress { .. } => ("asr-upload-progress", event),
        BusEvent::AsrRecordUpdated { .. } => ("asr-record-updated", event),
        BusEvent::TodoTreeUpdated { .. } => ("todo-tree-updated", event),
        BusEvent::AgentPoolUpdated { .. } => ("agent-pool-updated", event),
    };
    let _ = handle.emit(name, payload);
}

/// 把子 agent 事件累积到会话缓冲（供 send_message_stream 流结束时持久化）。
///
/// 聚合逻辑与前端 `onSubAgentEvent` 对齐（按 session_id 分组到同一记录）：
/// started 记任务、token 累积文本、tool_call/tool_result 记录工具调用、
/// attachment 解析图片、done/error 收尾。
pub(crate) fn accumulate_sub_agent_event(
    buf: &Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<SubAgentRecord>>>>,
    ev: &SubAgentEvent,
) {
    // 预解析 Attachment 事件的 JSON（锁外 CPU 工作），避免锁内调用 serde_json
    let attachment_image: Option<SubAgentImage> = if matches!(ev.kind, SubAgentEventKind::Attachment) {
        serde_json::from_str::<serde_json::Value>(&ev.content)
            .ok()
            .and_then(|v| {
                let path = v.get("path").and_then(|p| p.as_str())?;
                let name = v.get("name").and_then(|n| n.as_str())?;
                Some(SubAgentImage {
                    path: path.to_string(),
                    name: name.to_string(),
                })
            })
    } else {
        None
    };

    let Ok(mut map) = buf.lock() else { return };
    let recs = map.entry(ev.conversation_id.clone()).or_default();
    let rec = match recs.iter_mut().find(|r| r.session_id == ev.session_id) {
        Some(r) => r,
        None => {
            recs.push(SubAgentRecord {
                session_id: ev.session_id.clone(),
                name: ev.name.clone(),
                model: ev.model.clone(),
                depth: ev.depth,
                status: "running".to_string(),
                task: String::new(),
                text: String::new(),
                tool_calls: Vec::new(),
                images: Vec::new(),
                error: String::new(),
                finished_at: None,
            });
            recs.last_mut().unwrap()
        }
    };
    match ev.kind {
        SubAgentEventKind::Started => {
            rec.task = ev.content.clone();
            rec.status = "running".to_string();
        }
        SubAgentEventKind::Token => rec.text.push_str(&ev.content),
        SubAgentEventKind::ToolCall => rec.tool_calls.push(ToolCallRecord {
            call_id: format!("{}_{}", ev.session_id, rec.tool_calls.len()),
            tool_name: ev.tool_name.clone(),
            arguments: ev.arguments.clone(),
            result: String::new(),
            is_error: false,
        }),
        SubAgentEventKind::ToolResult => {
            // 与前端一致：按 tool_name + 未完成匹配（事件未携带 call_id）
            if let Some(tc) = rec
                .tool_calls
                .iter_mut()
                .find(|t| t.tool_name == ev.tool_name && t.result.is_empty())
            {
                tc.result = ev.content.clone();
                tc.is_error = ev.is_error;
            }
        }
        SubAgentEventKind::Attachment => {
            if let Some(img) = attachment_image {
                rec.images.push(img);
            }
        }
        SubAgentEventKind::Done => {
            rec.status = "done".to_string();
            if !ev.content.is_empty() {
                rec.text = ev.content.clone();
            }
            rec.finished_at = Some(now_ms() as i64);
        }
        SubAgentEventKind::Error => {
            rec.status = "error".to_string();
            rec.error = ev.content.clone();
            rec.finished_at = Some(now_ms() as i64);
        }
    }
}

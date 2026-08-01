//! 轻量事件总线
//!
//! 基于 `tokio::sync::broadcast`，遵循"用消息传递代替共享内存"原则。
//! 模块之间不直接持锁共享可变状态，而是向总线发布事件，由订阅者
//! （通常是 Tauri 后端）转发给前端或触发后续动作。
//!
//! 临界区保持极短：仅持锁做 `send`，不在锁内执行 IO 或重计算。

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// 跨模块流转的事件类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BusEvent {
    /// agent 流式输出的一段增量 token
    /// conversation_id 标识当前会话，content 为本次增量，done=true 表示该会话本轮结束
    AgentStreamToken {
        conversation_id: String,
        content: String,
        done: bool,
    },
    /// agent 产生的新消息（兼容旧版非流式一次性回复）
    AgentMessage {
        conversation_id: String,
        content: String,
        done: bool,
    },
    /// P2P 发现新设备
    DeviceFound { device: crate::Device },
    /// 设备状态变更
    DeviceStatusChanged {
        device_id: String,
        status: crate::DeviceStatus,
    },
    /// 配对请求
    PairingRequest { device: crate::Device },
    /// agent 向用户提问（前端展示选项卡片，用户回答经 Tauri 命令回传）
    /// questions 为序列化后的 Vec<Question>，前端按 JSON 解析
    AskUser {
        conversation_id: String,
        questions: serde_json::Value,
    },
    /// agent 通知用户审核文件（前端展示审核 UI）
    NotifyUser {
        conversation_id: String,
        explanation: String,
        file_paths: Vec<String>,
    },
    /// agent 请求前端打开预览 URL（浏览器或内嵌 webview）
    OpenPreview {
        conversation_id: String,
        preview_url: String,
        command_id: Option<String>,
    },
    /// ASR 流式转写的一段增量文本
    /// session_id 标识当前流式会话，is_final=true 表示该会话转写结束
    AsrStreamChunk {
        session_id: String,
        text: String,
        is_final: bool,
    },
    /// ASR 会话状态变更（started/transcribing/completed/failed/cancelled）
    AsrSessionStatus {
        session_id: String,
        status: String,
        error: Option<String>,
    },
    /// ASR 文件上传转写进度（0.0-1.0）
    AsrUploadProgress {
        record_id: String,
        progress: f32,
        status: String,
    },
    /// ASR 记录更新（新增/转写完成/摘要完成/编辑），前端据此刷新列表
    AsrRecordUpdated {
        record_id: String,
    },
}

/// 事件总线句柄，可被廉价 clone（broadcast 内部已是 Arc 共享）
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<BusEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity.max(8));
        Self { tx }
    }

    /// 发布事件。无订阅者时静默忽略，避免拖垮发布者。
    #[inline]
    pub fn publish(&self, event: BusEvent) {
        let _ = self.tx.send(event);
    }

    /// 订阅事件流
    #[inline]
    pub fn subscribe(&self) -> broadcast::Receiver<BusEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(64)
    }
}

//! 跨模块共享的领域模型
//!
//! 字段顺序遵循"按大小降序"原则以最小化结构体 padding，
//! 符合内存优化规则。所有结构体均 `Serialize`/`Deserialize`，
//! 以便在 Tauri 命令边界与 P2P 同步链路上无额外转换地传递。

use serde::{Deserialize, Serialize};

/// 消息角色
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// 设备状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatus {
    /// 已被发现但未配对
    Discovered,
    /// 已配对，在线
    Paired,
    /// 已配对，离线
    Offline,
    /// 正在配对握手
    Pairing,
}

/// 单条聊天消息
///
/// 字段按大小降序排列：String(24) > u64(8) > enum(1)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub content: String,
    pub timestamp: u64,
    pub role: Role,
}

impl Message {
    /// 快速构造一条消息，id 与 timestamp 由调用方提供以避免隐式 IO。
    #[inline]
    pub fn new(id: impl Into<String>, role: Role, content: impl Into<String>, timestamp: u64) -> Self {
        Self {
            id: id.into(),
            content: content.into(),
            timestamp,
            role,
        }
    }

    /// 判定是否为模型产生的内容
    #[inline]
    pub fn is_assistant(&self) -> bool {
        matches!(self.role, Role::Assistant)
    }
}

/// 可被 agent 操控的远端/本地设备
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub address: String,
    pub last_seen: u64,
    pub status: DeviceStatus,
}

impl Device {
    #[inline]
    pub fn is_paired(&self) -> bool {
        matches!(self.status, DeviceStatus::Paired)
    }
}

/// 一段对话上下文
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub messages: Vec<Message>,
    pub device_id: Option<String>,
    pub created_at: u64,
}

impl Conversation {
    pub fn new(id: impl Into<String>, created_at: u64) -> Self {
        Self {
            id: id.into(),
            messages: Vec::new(),
            device_id: None,
            created_at,
        }
    }

    /// 追加消息；若历史为空则预分配，避免反复扩容。
    pub fn push(&mut self, msg: Message) {
        if self.messages.is_empty() {
            self.messages = Vec::with_capacity(16);
        }
        self.messages.push(msg);
    }

    /// 仅在只读场景下借用历史，避免 clone
    #[inline]
    pub fn history(&self) -> &[Message] {
        &self.messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_roundtrip_serde() {
        let m = Message::new("m1", Role::User, "hi", 1);
        let s = serde_json::to_string(&m).unwrap();
        let back: Message = serde_json::from_str(&s).unwrap();
        assert_eq!(m, back);
    }
}

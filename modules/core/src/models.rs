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

/// 附件类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Image,
    File,
    Audio,
}

/// 消息附件（图片/文件/音频）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub kind: AttachmentKind,
    /// 附件存储路径（相对 attachments 目录的文件名）
    pub path: String,
    /// 原始文件名
    pub name: String,
    pub mime_type: String,
    /// 字节大小
    pub size: u64,
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
/// 字段按大小降序排列：String(24) > Vec(24) > u64(8) > enum(1)。
/// attachments 使用 #[serde(default)] 保证旧 JSON 向后兼容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub content: String,
    pub timestamp: u64,
    pub role: Role,
    /// 附件列表，旧文件无此字段时反序列化为空 Vec
    #[serde(default)]
    pub attachments: Vec<Attachment>,
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
            attachments: Vec::new(),
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
///
/// 新增字段（title/pinned/pinned_at/updated_at/working_dir）均使用 `#[serde(default)]`
/// 保证旧版 JSON 文件可无感反序列化。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub messages: Vec<Message>,
    pub device_id: Option<String>,
    pub created_at: u64,
    /// 用户自定义标题（改名），None 时前端用首条消息摘要
    #[serde(default)]
    pub title: Option<String>,
    /// 是否置顶
    #[serde(default)]
    pub pinned: bool,
    /// 置顶时间戳，用于置顶组内排序
    #[serde(default)]
    pub pinned_at: Option<u64>,
    /// 最后更新时间戳（最后一条消息的时间），用于"最近活跃"排序
    #[serde(default)]
    pub updated_at: u64,
    /// 会话级工作区路径。设置后覆盖技能级 working_dir，
    /// read_file / list_files / shell 的相对路径以此目录为基准。
    #[serde(default)]
    pub working_dir: Option<String>,
}

impl Conversation {
    pub fn new(id: impl Into<String>, created_at: u64) -> Self {
        Self {
            id: id.into(),
            messages: Vec::new(),
            device_id: None,
            created_at,
            title: None,
            pinned: false,
            pinned_at: None,
            updated_at: created_at,
            working_dir: None,
        }
    }

    /// 追加消息；若历史为空则预分配，避免反复扩容。
    /// 自动更新 updated_at 为新消息的时间戳。
    pub fn push(&mut self, msg: Message) {
        if self.messages.is_empty() {
            self.messages = Vec::with_capacity(16);
        }
        if msg.timestamp > self.updated_at {
            self.updated_at = msg.timestamp;
        }
        self.messages.push(msg);
    }

    /// 仅在只读场景下借用历史，避免 clone
    #[inline]
    pub fn history(&self) -> &[Message] {
        &self.messages
    }
}

/// 技能：preamble 前缀提示 + 工具子集的命名预设
///
/// 字段按大小降序：String/Vec（24B）→ u64（8B）→ bool（1B）。
/// 后添加字段均使用 `#[serde(default)]` 保证旧 JSON 向后兼容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 系统提示词前缀，加载技能时注入到 preamble 前
    #[serde(default)]
    pub preamble: String,
    /// 启用的工具名列表（如 ["search_history","read_file","shell"]），空表示全部
    #[serde(default)]
    pub tools: Vec<String>,
    /// 技能级工作区路径。apply_skill 时注入到会话上下文，
    /// read_file / list_files / shell 的相对路径以此目录为基准。
    /// 会话级 Conversation.working_dir 优先级更高，会覆盖此值。
    #[serde(default)]
    pub working_dir: Option<String>,
    pub created_at: u64,
    /// 是否内置（agent-reach / browser-act 等预置技能）
    #[serde(default)]
    pub builtin: bool,
}

/// 定时任务：按 cron 表达式定时执行技能
///
/// 字段按大小降序：String（24B）→ Option<u64>/u64（8B）→ bool（1B）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub name: String,
    pub skill_id: String,
    /// 5 字段 cron 表达式（分 时 日 月 周）
    pub cron: String,
    /// 上次执行时间（Unix 毫秒）
    #[serde(default)]
    pub last_run: Option<u64>,
    pub created_at: u64,
    /// 是否启用
    #[serde(default)]
    pub enabled: bool,
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

    #[test]
    fn skill_roundtrip_serde() {
        let s = Skill {
            id: "agent-reach".to_string(),
            name: "Agent Reach".to_string(),
            description: "test".to_string(),
            preamble: "preamble".to_string(),
            tools: vec!["shell".to_string()],
            working_dir: None,
            created_at: 42,
            builtin: true,
        };
        let json = serde_json::to_string(&s).unwrap();
        // 模拟旧文件：移除 builtin/tools/preamble/working_dir 字段后仍能反序列化
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut obj = v.as_object().unwrap().clone();
        obj.remove("builtin");
        obj.remove("tools");
        obj.remove("preamble");
        obj.remove("working_dir");
        let back: Skill = serde_json::from_value(serde_json::Value::Object(obj)).unwrap();
        assert_eq!(back.id, "agent-reach");
        assert!(!back.builtin);
        assert!(back.tools.is_empty());
        assert!(back.preamble.is_empty());
        assert!(back.working_dir.is_none());
    }

    #[test]
    fn scheduled_task_default_fields() {
        // 仅有必填字段时也能反序列化（enabled/last_run 缺省）
        let json = r#"{"id":"t1","name":"n","skill_id":"s","cron":"0 * * * *","created_at":1}"#;
        let t: ScheduledTask = serde_json::from_str(json).unwrap();
        assert_eq!(t.id, "t1");
        assert!(!t.enabled);
        assert!(t.last_run.is_none());
    }
}

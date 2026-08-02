//! P2P 线路协议：加密 TCP 传输层之上交换的消息类型。
//!
//! 设计要点：
//! - 所有帧均为 JSON 编码的 [`WireMessage`]，外层由 [`crate::transport`] 做
//!   长度前缀 + AES-256-GCM 整帧加密，保证机密性与完整性。
//! - `tag = "type"` + `rename_all = "snake_case"` 与前端 EventBus 风格一致，
//!   便于调试与跨语言对齐。
//! - 消息按"阶段"分组：握手 → 心跳 → 同步 → 远端任务 → 主机模式 RPC → 通用应答。
//! - 字段顺序遵循按大小降序排列原则（String/Vec 24B → [u8;32] 32B 固定 → u64 8B → bool 1B）。
//!   注意：`#[serde(tag = ...)]` 枚举的内部字段顺序由变体定义决定，这里在每个变体内
//!   尽量保持降序， minimizing padding（虽然 serde 序列化时 padding 不影响 JSON，
//!   但反序列化到内存结构时仍受益）。
//!
//! # 安全
//! - `Hello` / `HelloAck` 携带 Ed25519 签名，覆盖 `[ephemeral_pub || timestamp || device_id]`，
//!   防止握手被篡改；ephemeral_pub 经 X25519 ECDH 派生动态会话密钥（见 [`crate::crypto`]）。
//! - 所有业务消息在加密通道建立后明文 JSON 传输（信道已加密），不再单独签名。

use serde::{Deserialize, Serialize};

use effisuite_core::Message;

/// 同步数据种类（镜像模式按需同步）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncKind {
    /// 聊天会话与消息
    Conversations,
    /// 插件元数据
    Plugins,
    /// 用户缓存（永久记忆 / 跨会话记忆索引）
    UserCache,
}

/// 镜像同步：会话清单条目（轻量，不含消息体）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvManifestEntry {
    pub id: String,
    pub title: Option<String>,
    pub updated_at: u64,
    pub message_count: usize,
}

/// 镜像同步：会话清单
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncManifest {
    pub entries: Vec<ConvManifestEntry>,
}

/// 线路消息
///
/// 变体按通信阶段排序：握手 → 心跳 → 镜像同步 → 远端任务 → 主机模式 RPC → 通用应答。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireMessage {
    // ── 握手（动态会话密钥协商，紧随 TCP 连接建立） ──────────────────────
    /// 发起方 hello：携带本端设备 id、临时公钥、对 `[ephemeral_pub||ts||device_id]` 的 Ed25519 签名
    Hello {
        device_id: String,
        ephemeral_pub: [u8; 32],
        signature: Vec<u8>,
        timestamp: u64,
    },
    /// 应答方 hello_ack：对称结构，签名覆盖同字段
    HelloAck {
        device_id: String,
        ephemeral_pub: [u8; 32],
        signature: Vec<u8>,
        timestamp: u64,
    },

    // ── 心跳（在线探测，间隔默认 5s，超时 15s 判定离线） ──────────────────
    Ping { ts: u64 },
    Pong { ts: u64 },

    // ── 镜像同步 ────────────────────────────────────────────────────────
    /// 拉取指定时间点之后的指定种类数据
    SyncRequest { since: u64, kinds: Vec<SyncKind> },
    /// 响应 SyncRequest：返回会话清单（消息体按需 SyncFetch）
    SyncManifest { manifest: SyncManifest },
    /// 拉取指定会话 since_msg_ts 之后的消息
    SyncFetch {
        conversation_id: String,
        since_msg_ts: u64,
    },
    /// 响应 SyncFetch / 主动推送：一批消息（按 timestamp 升序）
    SyncMessages {
        conversation_id: String,
        messages: Vec<Message>,
    },

    // ── 远端任务派发（镜像模式：AI 跨设备指派任务） ──────────────────────
    /// 请求远端设备执行任务（远端 AI 处理后回 TaskResponse）
    TaskRequest {
        request_id: String,
        task: String,
    },
    /// 远端任务结果
    TaskResponse {
        request_id: String,
        result: String,
        is_error: bool,
    },

    // ── 主机模式 RPC（replica → host） ─────────────────────────────────
    /// 列出主机会话清单
    HostListConversations,
    /// 拉取主机指定会话消息
    HostGetConversation { conversation_id: String },
    /// 向主机指定会话发送消息（host AI 处理后流式回推，这里返回聚合结果）
    HostSendMessage {
        conversation_id: String,
        content: String,
    },
    /// 主机模式 RPC 统一响应（承载 JSON 字符串，由调用方按需解析）
    HostReply {
        ok: bool,
        payload: String,
    },

    // ── 通用应答 ────────────────────────────────────────────────────────
    Ack { ok: bool, msg: String },
}

impl WireMessage {
    /// 快速构造 Ack
    #[inline]
    pub fn ack(ok: bool, msg: impl Into<String>) -> Self {
        WireMessage::Ack {
            ok,
            msg: msg.into(),
        }
    }
}

// ── 单元测试 ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_message_roundtrip_hello() {
        let msg = WireMessage::Hello {
            device_id: "dev-1".to_string(),
            ephemeral_pub: [1u8; 32],
            signature: vec![2u8; 64],
            timestamp: 1234567890,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"hello\""));
        let back: WireMessage = serde_json::from_str(&json).unwrap();
        match back {
            WireMessage::Hello {
                device_id,
                ephemeral_pub,
                signature,
                timestamp,
            } => {
                assert_eq!(device_id, "dev-1");
                assert_eq!(ephemeral_pub, [1u8; 32]);
                assert_eq!(signature, vec![2u8; 64]);
                assert_eq!(timestamp, 1234567890);
            }
            _ => panic!("期望 Hello 变体"),
        }
    }

    #[test]
    fn wire_message_roundtrip_task_request() {
        let msg = WireMessage::TaskRequest {
            request_id: "req-1".to_string(),
            task: "检索电脑文件".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"task_request\""));
        let back: WireMessage = serde_json::from_str(&json).unwrap();
        match back {
            WireMessage::TaskRequest { request_id, task } => {
                assert_eq!(request_id, "req-1");
                assert_eq!(task, "检索电脑文件");
            }
            _ => panic!("期望 TaskRequest 变体"),
        }
    }

    #[test]
    fn wire_message_roundtrip_sync_messages() {
        let m = effisuite_core::Message::new("m1", effisuite_core::Role::User, "hi", 100);
        let msg = WireMessage::SyncMessages {
            conversation_id: "c1".to_string(),
            messages: vec![m],
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"sync_messages\""));
        let back: WireMessage = serde_json::from_str(&json).unwrap();
        match back {
            WireMessage::SyncMessages {
                conversation_id,
                messages,
            } => {
                assert_eq!(conversation_id, "c1");
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].content, "hi");
            }
            _ => panic!("期望 SyncMessages 变体"),
        }
    }

    #[test]
    fn ack_helper_constructs_correctly() {
        let msg = WireMessage::ack(true, "ok");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"ack\""));
        assert!(json.contains("\"ok\":true"));
    }

    #[test]
    fn sync_kind_serde_snake_case() {
        let json = serde_json::to_string(&SyncKind::Conversations).unwrap();
        assert_eq!(json, "\"conversations\"");
        let back: SyncKind = serde_json::from_str("\"user_cache\"").unwrap();
        assert_eq!(back, SyncKind::UserCache);
    }
}

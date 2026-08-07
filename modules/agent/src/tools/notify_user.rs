//! notify_user 工具：通知用户审核文件
//!
//! 设计要点：
//! - 发送 `NotifyUser` 事件到前端，携带说明文字与可选文件路径列表
//! - 前端收到后展示审核 UI（文件列表 + 说明）
//! - `file_paths` 为空时允许（纯文字通知）
//! - `explanation` 不能为空
//!
//! # 临界区
//! 仅短暂持有 `conversation_id` 读锁做 clone，锁内无 IO / 重计算。

use std::sync::Arc;

use effisuite_core::{BusEvent, EventBus};
use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::sync::RwLock;

/// 工具参数
///
/// 字段按大小降序：`String`（24 字节）与 `Vec<String>`（24 字节）大小相同。
#[derive(Deserialize)]
pub struct NotifyUserArgs {
    /// 通知说明
    pub explanation: String,
    /// 需要审核的文档路径列表（可为空：纯文字通知）
    #[serde(default)]
    pub file_paths: Vec<String>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("notify_user error: {0}")]
pub struct NotifyUserError(String);

/// 通知用户审核工具
///
/// 持有：
/// - `event_bus`：前端交互通道（None 时返回友好错误）
/// - `conversation_id`：当前会话 id 句柄（由 Tauri 命令层维护）
pub struct NotifyUserTool {
    event_bus: Option<Arc<EventBus>>,
    conversation_id: Arc<RwLock<Option<String>>>,
}

impl NotifyUserTool {
    pub fn new(
        event_bus: Option<Arc<EventBus>>,
        conversation_id: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            event_bus,
            conversation_id,
        }
    }
}

impl Tool for NotifyUserTool {
    const NAME: &'static str = "notify_user";

    type Error = NotifyUserError;
    type Args = NotifyUserArgs;
    type Output = String;

  fn description(&self) -> String {
      "通知用户审核文件或查看重要信息：前端弹出审核 UI 展示说明文字与文件路径列表。\
       file_paths 为空时仅展示文字通知。"
          .to_string()
  }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "explanation": {
                    "type": "string",
                      "description": "通知说明文字"
                },
                "file_paths": {
                    "type": "array",
                    "items": { "type": "string" },
                      "description": "需审核的文档路径列表（可为空）"
                }
            },
            "required": ["explanation"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // 1. 校验：explanation 不能为空
        if args.explanation.trim().is_empty() {
            return Err(NotifyUserError("explanation 不能为空".to_string()));
        }

        // 2. 检查前端交互通道
        let event_bus = self.event_bus.as_ref().ok_or_else(|| {
            NotifyUserError("前端交互通道不可用".to_string())
        })?;

        // 3. 读取会话 id（短暂持锁，仅 clone）
        let conversation_id = self
            .conversation_id
            .read()
            .await
            .clone()
            .unwrap_or_default();

        // 4. 发布事件
        let file_count = args.file_paths.len();
        event_bus.publish(BusEvent::NotifyUser {
            conversation_id,
            explanation: args.explanation,
            file_paths: args.file_paths,
        });

        // 文案区分：有文件 → 审核文件；无文件 → 纯文字通知
        // 避免"已通知用户审核 0 个文件"这种怪异表述
        let msg = if file_count == 0 {
            "已向用户发送通知".to_string()
        } else {
            format!("已通知用户审核 {file_count} 个文件")
        };
        Ok(msg)
    }
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool(
        bus: Option<Arc<EventBus>>,
        conv: Option<&str>,
    ) -> NotifyUserTool {
        NotifyUserTool::new(bus, Arc::new(RwLock::new(conv.map(|s| s.to_string()))))
    }

    #[tokio::test]
    async fn call_rejects_empty_explanation() {
        let tool = make_tool(Some(Arc::new(EventBus::new(8))), Some("c1"));
        let err = tool
            .call(NotifyUserArgs {
                explanation: "   ".to_string(),
                file_paths: vec![],
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("explanation 不能为空"));
    }

    #[tokio::test]
    async fn call_rejects_without_event_bus() {
        let tool = make_tool(None, Some("c1"));
        let err = tool
            .call(NotifyUserArgs {
                explanation: "请审核".to_string(),
                file_paths: vec![],
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("前端交互通道不可用"));
    }

    #[tokio::test]
    async fn call_allows_empty_file_paths() {
        let bus = Arc::new(EventBus::new(8));
        let mut rx = bus.subscribe();
        let tool = make_tool(Some(Arc::clone(&bus)), Some("conv-1"));

        let out = tool
            .call(NotifyUserArgs {
                explanation: "任务已完成".to_string(),
                file_paths: vec![],
            })
            .await
            .unwrap();
        assert_eq!(out, "已向用户发送通知");

        let ev = rx.recv().await.unwrap();
        match ev {
            BusEvent::NotifyUser {
                explanation,
                file_paths,
                conversation_id,
            } => {
                assert_eq!(explanation, "任务已完成");
                assert!(file_paths.is_empty());
                assert_eq!(conversation_id, "conv-1");
            }
            _ => panic!("期望 NotifyUser 事件"),
        }
    }

    #[tokio::test]
    async fn call_publishes_event_with_files() {
        let bus = Arc::new(EventBus::new(8));
        let mut rx = bus.subscribe();
        let tool = make_tool(Some(Arc::clone(&bus)), Some("conv-2"));

        let out = tool
            .call(NotifyUserArgs {
                explanation: "请审核以下报告".to_string(),
                file_paths: vec!["/a/report.md".to_string(), "/b/summary.md".to_string()],
            })
            .await
            .unwrap();
        assert_eq!(out, "已通知用户审核 2 个文件");

        let ev = rx.recv().await.unwrap();
        match ev {
            BusEvent::NotifyUser {
                explanation,
                file_paths,
                conversation_id,
            } => {
                assert_eq!(explanation, "请审核以下报告");
                assert_eq!(file_paths.len(), 2);
                assert_eq!(file_paths[0], "/a/report.md");
                assert_eq!(conversation_id, "conv-2");
            }
            _ => panic!("期望 NotifyUser 事件"),
        }
    }

    #[tokio::test]
    async fn call_validates_before_publishing() {
        let bus = Arc::new(EventBus::new(8));
        let rx = bus.subscribe();
        let tool = make_tool(Some(bus), Some("c1"));

        let err = tool
            .call(NotifyUserArgs {
                explanation: "".to_string(),
                file_paths: vec!["x".to_string()],
            })
            .await;
        assert!(err.is_err());
        assert!(rx.is_empty());
    }
}

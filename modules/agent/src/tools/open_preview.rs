//! open_preview 工具：让 LLM 请求前端打开预览 URL
//!
//! 设计要点：
//! - 发送 `OpenPreview` 事件到前端，携带 URL 与可选 command_id
//! - 前端收到后在浏览器或内嵌 webview 中打开 URL
//! - URL 必须以 `http://` 或 `https://` 开头（禁止 file/ftp 等）
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
/// 字段按大小降序：`String`（24 字节）与 `Option<String>`（24 字节）大小相同。
#[derive(Deserialize)]
pub struct OpenPreviewArgs {
    /// 预览 URL（如 http://localhost:8000/）
    pub preview_url: String,
    /// 关联的命令 ID（用于追踪启动预览的命令）
    #[serde(default)]
    pub command_id: Option<String>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("open_preview error: {0}")]
pub struct OpenPreviewError(String);

/// 打开预览 URL 工具
///
/// 持有：
/// - `event_bus`：前端交互通道（None 时返回友好错误）
/// - `conversation_id`：当前会话 id 句柄（由 Tauri 命令层维护）
pub struct OpenPreviewTool {
    event_bus: Option<Arc<EventBus>>,
    conversation_id: Arc<RwLock<Option<String>>>,
}

impl OpenPreviewTool {
    pub fn new(
        event_bus: Option<Arc<EventBus>>,
        conversation_id: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            event_bus,
            conversation_id,
        }
    }

    /// 校验 URL：非空且以 http:// 或 https:// 开头
    #[inline]
    fn validate_url(url: &str) -> Result<(), OpenPreviewError> {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return Err(OpenPreviewError("preview_url 不能为空".to_string()));
        }
        if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
            return Err(OpenPreviewError(
                "preview_url 必须以 http:// 或 https:// 开头".to_string(),
            ));
        }
        Ok(())
    }
}

impl Tool for OpenPreviewTool {
    const NAME: &'static str = "open_preview";

    type Error = OpenPreviewError;
    type Args = OpenPreviewArgs;
    type Output = String;

    fn description(&self) -> String {
        "在浏览器中打开预览 URL（如本地开发服务器 http://localhost:8000/）。\
         前端收到后会在默认浏览器或内嵌 webview 中打开。\
         URL 必须以 http:// 或 https:// 开头。\
         适用于：启动 dev server 后展示页面、打开在线文档等场景。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "preview_url": {
                    "type": "string",
                    "description": "预览 URL，必须以 http:// 或 https:// 开头",
                    "pattern": "^https?://"
                },
                "command_id": {
                    "type": "string",
                    "description": "关联的命令 ID（用于追踪启动预览的命令，可选）"
                }
            },
            "required": ["preview_url"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // 1. 校验 URL
        Self::validate_url(&args.preview_url)?;

        // 2. 检查前端交互通道
        let event_bus = self.event_bus.as_ref().ok_or_else(|| {
            OpenPreviewError("前端交互通道不可用".to_string())
        })?;

        // 3. 读取会话 id（短暂持锁，仅 clone）
        let conversation_id = self
            .conversation_id
            .read()
            .await
            .clone()
            .unwrap_or_default();

        // 4. 发布事件：trim 后的 url move 进 BusEvent，避免一次 clone
        let url = args.preview_url.trim().to_string();
        let ok_msg = format!("已打开预览：{url}");
        event_bus.publish(BusEvent::OpenPreview {
            conversation_id,
            preview_url: url,
            command_id: args.command_id,
        });

        Ok(ok_msg)
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
    ) -> OpenPreviewTool {
        OpenPreviewTool::new(bus, Arc::new(RwLock::new(conv.map(|s| s.to_string()))))
    }

    #[test]
    fn validate_url_rejects_empty() {
        let err = OpenPreviewTool::validate_url("").unwrap_err();
        assert!(err.to_string().contains("不能为空"));
    }

    #[test]
    fn validate_url_rejects_whitespace() {
        let err = OpenPreviewTool::validate_url("   ").unwrap_err();
        assert!(err.to_string().contains("不能为空"));
    }

    #[test]
    fn validate_url_rejects_non_http() {
        for bad in &["file:///etc/passwd", "ftp://x", "javascript:alert(1)", "localhost:8000"] {
            let err = OpenPreviewTool::validate_url(bad).unwrap_err();
            assert!(
                err.to_string().contains("http://"),
                "URL {bad} 应被拒绝"
            );
        }
    }

    #[test]
    fn validate_url_accepts_http() {
        assert!(OpenPreviewTool::validate_url("http://localhost:8000/").is_ok());
        assert!(OpenPreviewTool::validate_url("https://example.com").is_ok());
    }

    #[test]
    fn validate_url_accepts_with_whitespace() {
        assert!(OpenPreviewTool::validate_url("  http://localhost:3000  ").is_ok());
    }

    #[tokio::test]
    async fn call_rejects_without_event_bus() {
        let tool = make_tool(None, Some("c1"));
        let err = tool
            .call(OpenPreviewArgs {
                preview_url: "http://localhost:8000/".to_string(),
                command_id: None,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("前端交互通道不可用"));
    }

    #[tokio::test]
    async fn call_rejects_bad_url_before_bus_check() {
        // URL 校验在 event_bus 检查之前，即使 bus 为 None 也应先报 URL 错误
        let tool = make_tool(None, Some("c1"));
        let err = tool
            .call(OpenPreviewArgs {
                preview_url: "file:///x".to_string(),
                command_id: None,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("http://"));
    }

    #[tokio::test]
    async fn call_publishes_event_and_returns_url() {
        let bus = Arc::new(EventBus::new(8));
        let mut rx = bus.subscribe();
        let tool = make_tool(Some(Arc::clone(&bus)), Some("conv-9"));

        let out = tool
            .call(OpenPreviewArgs {
                preview_url: "http://localhost:8000/".to_string(),
                command_id: Some("cmd-1".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(out, "已打开预览：http://localhost:8000/");

        let ev = rx.recv().await.unwrap();
        match ev {
            BusEvent::OpenPreview {
                conversation_id,
                preview_url,
                command_id,
            } => {
                assert_eq!(conversation_id, "conv-9");
                assert_eq!(preview_url, "http://localhost:8000/");
                assert_eq!(command_id.as_deref(), Some("cmd-1"));
            }
            _ => panic!("期望 OpenPreview 事件"),
        }
    }

    #[tokio::test]
    async fn call_trims_url_whitespace() {
        let bus = Arc::new(EventBus::new(8));
        let mut rx = bus.subscribe();
        let tool = make_tool(Some(Arc::clone(&bus)), Some("c1"));

        let out = tool
            .call(OpenPreviewArgs {
                preview_url: "  https://example.com  ".to_string(),
                command_id: None,
            })
            .await
            .unwrap();
        assert_eq!(out, "已打开预览：https://example.com");

        let ev = rx.recv().await.unwrap();
        match ev {
            BusEvent::OpenPreview { preview_url, command_id, .. } => {
                assert_eq!(preview_url, "https://example.com");
                assert!(command_id.is_none());
            }
            _ => panic!("期望 OpenPreview 事件"),
        }
    }

    #[tokio::test]
    async fn call_uses_empty_string_when_no_conversation() {
        let bus = Arc::new(EventBus::new(8));
        let mut rx = bus.subscribe();
        let tool = make_tool(Some(Arc::clone(&bus)), None);

        tool.call(OpenPreviewArgs {
            preview_url: "http://x.com".to_string(),
            command_id: None,
        })
        .await
        .unwrap();

        let ev = rx.recv().await.unwrap();
        match ev {
            BusEvent::OpenPreview { conversation_id, .. } => {
                assert_eq!(conversation_id, "");
            }
            _ => panic!("期望 OpenPreview 事件"),
        }
    }
}

//! set_title 工具：让 LLM 自主为会话生成 / 更新标题
//!
//! 设计要点：
//! - 持有 `Arc<ConversationStore>` 共享存储句柄，调用时直接落盘（与 PinMemoryTool 一致）
//! - 通过 `current_conversation_id` 句柄读取当前会话 id，无需 LLM 传入
//! - 标题长度硬上限 25 字符（按 Unicode 字符边界截断，UTF-8 安全）
//! - LLM 可多次调用以覆盖旧标题（话题转变时主动更新）
//!
//! # 触发时机（在 description 中告知 LLM）
//! - 话题转变：当前标题已不概括后续对话内容时
//! - 用户明确要求改名时
//! - 注：新会话的首条消息已由 send_message 后台自动命名，无需本工具重复生成
//!
//! # 返回值
//! 成功返回 `"已更新会话标题为：xxx"`，便于 LLM 在后续回复中确认。

use std::sync::Arc;

use effisuite_core::ConversationStore;
use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::sync::RwLock;

/// 标题最大字符数（按 Unicode scalar value 计数，中文/英文均算 1）
const MAX_TITLE_CHARS: usize = 25;

/// 工具参数
#[derive(Deserialize)]
pub struct SetTitleArgs {
    /// 新标题（≤25 字符，超出会被截断）
    pub title: String,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("set title error: {0}")]
pub struct SetTitleError(String);

/// 会话标题设置工具
///
/// 持有：
/// - `store`：共享存储句柄（与 Tauri 命令层、RigAgent 共享同一份）
/// - `current_conversation_id`：当前会话 id 句柄（由 Tauri 命令层在 send_message 前更新）
pub struct SetTitleTool {
    store: Arc<ConversationStore>,
    current_conversation_id: Arc<RwLock<Option<String>>>,
}

impl SetTitleTool {
    pub fn new(
        store: Arc<ConversationStore>,
        current_conversation_id: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            store,
            current_conversation_id,
        }
    }
}

impl Tool for SetTitleTool {
    const NAME: &'static str = "set_title";

    type Error = SetTitleError;
    type Args = SetTitleArgs;
    type Output = String;

    fn description(&self) -> String {
        "为当前会话设置或更新标题。标题应简洁概括对话主题，不超过 25 个字。\n\
         注意：新会话的首条消息会自动生成标题，无需在本轮调用本工具。\n\
         调用时机：\n\
         1. 话题发生明显转变，当前标题已不能概括对话内容时；\n\
         2. 用户明确要求修改标题时。\n\
         可多次调用，新标题会覆盖旧标题。".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "新标题，简洁概括对话主题，≤25 字",
                    "maxLength": MAX_TITLE_CHARS
                }
            },
            "required": ["title"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let title = args.title.trim();
        if title.is_empty() {
            return Err(SetTitleError("title 不能为空".to_string()));
        }

        // 按 Unicode 字符截断到 MAX_TITLE_CHARS，避免从字节中间切断产生乱码
        let title: String = if title.chars().count() > MAX_TITLE_CHARS {
            title.chars().take(MAX_TITLE_CHARS).collect()
        } else {
            title.to_string()
        };

        let conv_id = self.current_conversation_id.read().await.clone();
        let conv_id = conv_id.ok_or_else(|| {
            SetTitleError("当前未选中会话，无法设置标题".to_string())
        })?;

        self.store
            .rename(&conv_id, title.clone())
            .await
            .map_err(|e| SetTitleError(e.to_string()))?;

        Ok(format!("已更新会话标题为：{title}"))
    }
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use effisuite_core::Conversation;
    use std::path::PathBuf;

    fn tmp_path() -> PathBuf {
        // ConversationStore::new 接受目录，每个 conversation 存为 <dir>/<id>.json
        std::env::temp_dir().join(format!(
            "effisuite-set-title-test-{}",
            uuid::Uuid::new_v4()
        ))
    }

    /// 创建并保存一个空会话，返回其 id
    async fn create_conv(store: &ConversationStore) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let conv = Conversation::new(id.clone(), 0);
        store.save(&conv).await.unwrap();
        id
    }

    #[tokio::test]
    async fn set_title_updates_conversation() {
        let store = Arc::new(ConversationStore::new(tmp_path()).unwrap());
        let id = create_conv(&store).await;
        let cur = Arc::new(RwLock::new(Some(id.clone())));
        let tool = SetTitleTool::new(Arc::clone(&store), cur);

        let out = tool
            .call(SetTitleArgs {
                title: "Rust 并发测试".to_string(),
            })
            .await
            .unwrap();
        assert!(out.contains("已更新会话标题为：Rust 并发测试"));

        let conv = store.load(&id).await.unwrap().unwrap();
        assert_eq!(conv.title.as_deref(), Some("Rust 并发测试"));
    }

    #[tokio::test]
    async fn set_title_truncates_overlong() {
        let store = Arc::new(ConversationStore::new(tmp_path()).unwrap());
        let id = create_conv(&store).await;
        let cur = Arc::new(RwLock::new(Some(id.clone())));
        let tool = SetTitleTool::new(Arc::clone(&store), cur);

        // 30 个中文字符，应被截断为 25
        let long_title = "一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十";
        assert_eq!(long_title.chars().count(), 30);

        let out = tool
            .call(SetTitleArgs {
                title: long_title.to_string(),
            })
            .await
            .unwrap();

        let conv = store.load(&id).await.unwrap().unwrap();
        let saved = conv.title.unwrap();
        assert_eq!(saved.chars().count(), MAX_TITLE_CHARS);
        // 截断后的内容应是前 25 个字符
        let expected: String = long_title.chars().take(MAX_TITLE_CHARS).collect();
        assert_eq!(saved, expected);
        // 返回信息中显示的是截断后的标题
        assert!(out.contains(&saved));
    }

    #[tokio::test]
    async fn set_title_rejects_empty() {
        let store = Arc::new(ConversationStore::new(tmp_path()).unwrap());
        let cur = Arc::new(RwLock::new(Some("conv-1".to_string())));
        let tool = SetTitleTool::new(store, cur);

        let res = tool
            .call(SetTitleArgs {
                title: "   ".to_string(),
            })
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn set_title_rejects_without_conversation() {
        let store = Arc::new(ConversationStore::new(tmp_path()).unwrap());
        let cur = Arc::new(RwLock::new(None));
        let tool = SetTitleTool::new(store, cur);

        let res = tool
            .call(SetTitleArgs {
                title: "无会话".to_string(),
            })
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn set_title_overwrites_previous() {
        let store = Arc::new(ConversationStore::new(tmp_path()).unwrap());
        let id = create_conv(&store).await;
        let cur = Arc::new(RwLock::new(Some(id.clone())));
        let tool = SetTitleTool::new(Arc::clone(&store), cur);

        tool.call(SetTitleArgs {
            title: "第一个标题".to_string(),
        })
        .await
        .unwrap();
        tool.call(SetTitleArgs {
            title: "第二个标题".to_string(),
        })
        .await
        .unwrap();

        let conv = store.load(&id).await.unwrap().unwrap();
        assert_eq!(conv.title.as_deref(), Some("第二个标题"));
    }
}

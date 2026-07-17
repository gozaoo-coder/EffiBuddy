//! MockAgent：无网络、无 API key 的本地回显后端
//!
//! 用于离线开发、单元测试与 UI 联调。返回内容形如 `[mock] ...`，
//! 便于在前端一眼区分真实模型回复与 mock 回复。

use async_trait::async_trait;
use effisuite_core::{Message, Result, Role};

use crate::agent::ChatAgent;

pub struct MockAgent {
    name: String,
}

impl MockAgent {
    pub fn new() -> Self {
        Self {
            name: "MockAgent".to_string(),
        }
    }
}

impl Default for MockAgent {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChatAgent for MockAgent {
    async fn chat(&self, messages: &[Message]) -> Result<String> {
        // 反向遍历找最后一条用户消息，避免显式索引循环（迭代器适配器优先）
        let last_user = messages.iter().rev().find(|m| m.role == Role::User);

        let reply = match last_user {
            Some(m) if !m.content.is_empty() => {
                format!("[mock] 已收到你的消息：{}\n（当前为 mock 后端，配置 OPENAI_API_KEY 后可切换到真实模型）", m.content)
            }
            _ => "[mock] 你好，我是 EffiSuite 的 mock agent。".to_string(),
        };

        Ok(reply)
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    fn backend(&self) -> &'static str {
        "mock"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_replies_with_echo() {
        let agent = MockAgent::new();
        let msgs = vec![Message::new("m1", Role::User, "hello", 0)];
        let r = agent.chat(&msgs).await.unwrap();
        assert!(r.contains("hello"));
    }
}

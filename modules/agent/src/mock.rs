//! MockAgent：无网络、无 API key 的本地回显后端
//!
//! 用于离线开发、单元测试与 UI 联调。返回内容形如 `[mock] ...`，
//! 便于在前端一眼区分真实模型回复与 mock 回复。
//!
//! 流式实现：把完整回复按 UTF-8 字符边界切分成多段，每段间隔 30ms yield，
//! 用于验证前端流式渲染链路。

use std::time::Duration;

use async_trait::async_trait;
use effisuite_core::{Message, Result, Role};
use futures::stream::BoxStream;
use futures::{StreamExt, stream};
use tokio::time::sleep;

use crate::agent::{AgentStreamItem, ChatAgent};

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

/// 把完整回复按字符分块（每块 3 个字符），便于流式演示
fn chunk_reply(reply: String) -> Vec<String> {
    let chars: Vec<String> = reply.chars().map(|c| c.to_string()).collect();
    chars
        .chunks(3)
        .map(|chunk| chunk.concat())
        .collect()
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

    fn chat_stream<'a>(
        &'a self,
        messages: &'a [Message],
    ) -> BoxStream<'a, Result<AgentStreamItem>> {
        let reply = match messages.iter().rev().find(|m| m.role == Role::User) {
            Some(m) if !m.content.is_empty() => {
                format!("[mock stream] 收到「{}」——这是流式回显演示。", m.content)
            }
            _ => "[mock stream] 你好，我是 mock agent。".to_string(),
        };
        let chunks = chunk_reply(reply);

        let s = stream::iter(chunks)
            .map(|chunk| Ok(AgentStreamItem::Text { content: chunk }))
            .then(|r| async move {
                // 30ms 间隔，模拟网络延迟
                sleep(Duration::from_millis(30)).await;
                r
            });
        Box::pin(s)
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
    use futures::StreamExt;

    #[tokio::test]
    async fn mock_replies_with_echo() {
        let agent = MockAgent::new();
        let msgs = vec![Message::new("m1", Role::User, "hello", 0)];
        let r = agent.chat(&msgs).await.unwrap();
        assert!(r.contains("hello"));
    }

    #[tokio::test]
    async fn mock_stream_yields_multiple_chunks() {
        let agent = MockAgent::new();
        let msgs = vec![Message::new("m1", Role::User, "hi", 0)];
        let mut stream = agent.chat_stream(&msgs);
        let mut collected = String::new();
        let mut count = 0;
        while let Some(chunk) = stream.next().await {
            match chunk.unwrap() {
                AgentStreamItem::Text { content } => collected.push_str(&content),
                _ => {}
            }
            count += 1;
        }
        assert!(count > 1, "stream should produce multiple chunks, got {count}");
        assert!(collected.contains("hi"));
    }
}

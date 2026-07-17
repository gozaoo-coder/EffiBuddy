//! ChatAgent 抽象 trait
//!
//! 业务层只依赖此 trait，不触碰 rig 具体类型，从而把 provider 切换
//! 变成"换一个实现"而非"改一串调用点"。
//!
//! 流式输出通过 [`chat_stream`] 返回 `BoxStream<Result<String>>`，
//! 每个 `Ok(String)` 是一段增量 token（可能 1 字符也可能几个字符），
//! 流自然结束表示本轮回复完成；`Err` 表示出错中断。

use async_trait::async_trait;
use effisuite_core::{Message, Result};
use futures::stream::BoxStream;

/// 对话后端的统一抽象
#[async_trait]
pub trait ChatAgent: Send + Sync {
    /// 根据完整对话历史产生一条回复（非流式，便于简单场景调用）
    async fn chat(&self, messages: &[Message]) -> Result<String>;

    /// 流式回复：返回增量 token 流
    ///
    /// `messages` 为完整历史，实现方需自行取最后一条 user 作为本轮 prompt
    /// 并把其余历史传给 model。每个 `Ok(String)` 是一段 token 增量；
    /// 流自然结束表示本轮回复完成。
    fn chat_stream<'a>(
        &'a self,
        messages: &'a [Message],
    ) -> BoxStream<'a, Result<String>>;

    /// 后端可读名称，用于 UI 展示
    fn name(&self) -> &str;

    /// 后端标识（mock / openai / ollama ...）
    fn backend(&self) -> &'static str;
}

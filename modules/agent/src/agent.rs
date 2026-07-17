//! ChatAgent 抽象 trait
//!
//! 业务层只依赖此 trait，不触碰 rig 具体类型，从而把 provider 切换
//! 变成"换一个实现"而非"改一串调用点"。

use async_trait::async_trait;
use effisuite_core::{Message, Result};

/// 对话后端的统一抽象
#[async_trait]
pub trait ChatAgent: Send + Sync {
    /// 根据完整对话历史产生一条回复
    async fn chat(&self, messages: &[Message]) -> Result<String>;

    /// 后端可读名称，用于 UI 展示
    fn name(&self) -> &str;

    /// 后端标识（mock / openai / ollama ...）
    fn backend(&self) -> &'static str;
}

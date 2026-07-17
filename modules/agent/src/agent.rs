//! ChatAgent 抽象 trait 与流式事件类型
//!
//! 业务层只依赖此 trait，不触碰 rig 具体类型，从而把 provider 切换
//! 变成"换一个实现"而非"改一串调用点"。
//!
//! 流式输出通过 [`chat_stream`] 返回 `BoxStream<Result<AgentStreamItem>>`，
//! 每个事件可能是文本增量、推理增量、工具调用开始或工具执行结果；
//! 流自然结束表示本轮回复完成；`Err` 表示出错中断。

use async_trait::async_trait;
use effisuite_core::{Message, Result};
use futures::stream::BoxStream;
use serde::Serialize;

/// 流式事件：把模型输出按语义分类透传给前端
///
/// - `Text`：模型回复文本增量（用户可见的最终答案）
/// - `Reasoning`：推理/思考链增量（DeepSeek-R1 / o1 等模型产生）
/// - `ToolCallStart`：模型决定调用工具，携带工具名与参数
/// - `ToolResult`：工具执行完成，携带返回内容
///
/// 设计要点：
/// - 用 `serde_json::Value` 携带工具参数，避免 agent 模块依赖 rig 的 ToolCall 类型
/// - 枚举本身 provider 无关，可被任何 ChatAgent 实现复用
/// - `call_id` 用于前端关联 "调用开始" 与 "执行结果"
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentStreamItem {
    /// 文本增量
    Text {
        content: String,
    },
    /// 推理增量
    Reasoning {
        content: String,
    },
    /// 工具调用开始
    ToolCallStart {
        /// Rig 生成的内部调用 ID，用于关联后续 ToolResult
        call_id: String,
        /// 工具/函数名
        tool_name: String,
        /// JSON 参数（可能是对象或 null）
        arguments: serde_json::Value,
    },
    /// 工具执行结果
    ToolResult {
        /// 关联的内部调用 ID
        call_id: String,
        /// 工具返回内容（已序列化为字符串）
        output: String,
        /// 是否为错误结果
        is_error: bool,
    },
}

/// 对话后端的统一抽象
#[async_trait]
pub trait ChatAgent: Send + Sync {
    /// 根据完整对话历史产生一条回复（非流式，便于简单场景调用）
    async fn chat(&self, messages: &[Message]) -> Result<String>;

    /// 流式回复：返回事件流
    ///
    /// `messages` 为完整历史，实现方需自行取最后一条 user 作为本轮 prompt
    /// 并把其余历史传给 model。事件按到达顺序 yield；
    /// 流自然结束表示本轮回复完成。
    fn chat_stream<'a>(
        &'a self,
        messages: &'a [Message],
    ) -> BoxStream<'a, Result<AgentStreamItem>>;

    /// 后端可读名称，用于 UI 展示
    fn name(&self) -> &str;

    /// 后端标识（mock / openai / ollama ...）
    fn backend(&self) -> &'static str;
}

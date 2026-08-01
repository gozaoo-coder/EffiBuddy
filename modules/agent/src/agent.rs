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

/// 上下文注入预览：把 `RigAgent::build_contextual_prompt` 拼装的最终 prompt
/// 按段拆开返回，供前端"上下文管理"面板分块展示。
///
/// 字段按大小降序排列（String 24B → usize 8B → bool 1B），最小化 padding。
#[derive(Debug, Clone, Serialize)]
pub struct ContextPreview {
    /// 当前激活 agent 的系统提示词（preamble），作为 system 消息注入到 rig Agent
    pub preamble: String,
    /// `[永久记忆]` 段格式化字符串（含头部说明），空表示无永久记忆
    pub pinned_section: String,
    /// `[相关历史记忆]` 段格式化字符串（含头部说明），空表示无 RAG 命中
    pub memory_section: String,
    /// `[可用技能]` 段格式化字符串（含头部说明），空表示无技能自动注入
    pub skill_section: String,
    /// `[当前对话最近]` 段格式化字符串（含头部说明），空表示无历史
    pub history_section: String,
    /// 当前用户问题文本（最后一条 user 消息）
    pub current_question: String,
    /// 拼装后的完整 prompt（与实际发给 LLM 的内容一致）
    pub full_prompt: String,
    /// 永久记忆条目数
    pub pinned_count: usize,
    /// RAG 命中条目数
    pub memory_hits_count: usize,
    /// 技能自动注入命中条目数
    pub skill_hits_count: usize,
    /// 当前对话历史保留的消息条数（与 history_total_count 相等：全量不截断）
    pub history_keep_count: usize,
    /// 当前对话总消息条数（包含当前问题）
    pub history_total_count: usize,
    /// 自动注入的相关历史记忆条数上限（MEMORY_AUTO_INJECT_LIMIT）
    pub memory_inject_limit: usize,
    /// 当前对话保留的最近消息条数上限。值 0 表示"无限制"（全量保留）。
    /// 由消息压缩系统维护 token 预算，而非在此层硬截断。
    pub recent_history_limit: usize,
    /// 单条历史消息截断字符数。值 0 表示"无限制"（保留完整内容）。
    pub history_truncate_chars: usize,
    /// 是否启用了 RAG 跨会话记忆增强
    pub memory_enabled: bool,
    /// 是否启用了技能 RAG 自动注入
    pub skill_auto_inject_enabled: bool,
}

/// 流式事件：把模型输出按语义分类透传给前端
///
/// - `Text`：模型回复文本增量（用户可见的最终答案）
/// - `Reasoning`：推理/思考链增量（DeepSeek-R1 / o1 等模型产生）
/// - `ToolCallStart`：模型决定调用工具，携带工具名与参数
/// - `ToolResult`：工具执行完成，携带返回内容
/// - `Usage`：单次 completion 请求的 token 使用统计（透传 rig 的 CompletionCall.usage）
///
/// 设计要点：
/// - 用 `serde_json::Value` 携带工具参数，避免 agent 模块依赖 rig 的 ToolCall 类型
/// - 枚举本身 provider 无关，可被任何 ChatAgent 实现复用
/// - `call_id` 用于前端关联 "调用开始" 与 "执行结果"
/// - `Usage` 累计所有 completion 调用的 token 消耗，前端据此显示实际成本
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
    /// 单次 completion 请求的 token 使用统计
    ///
    /// rig 在每次调用 LLM 时 emit `CompletionCall(usage)`，agent 透传给前端。
    /// 前端累计所有 Usage 事件得到本轮对话的总 token 消耗。
    /// provider 未返回 usage 时所有字段为 0（rig 的零值哨兵）。
    Usage {
        /// 输入（prompt）token 数
        input_tokens: u64,
        /// 输出（completion）token 数
        output_tokens: u64,
        /// 总 token 数（部分 provider 仅返回总数）
        total_tokens: u64,
        /// 推理 token 数（o1 / DeepSeek-R1 等模型的思考链）
        reasoning_tokens: u64,
        /// 缓存命中输入 token 数（DeepSeek 等 provider 在 usage 顶层返回）
        cache_hit_tokens: u64,
        /// 缓存未命中输入 token 数（DeepSeek 等 provider 在 usage 顶层返回）
        cache_miss_tokens: u64,
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

    /// 返回当前对话的上下文注入预览
    ///
    /// 默认返回 `None`（如 MockAgent），由具体实现（如 RigAgent）覆盖。
    /// 用于"上下文管理"面板可视化展示将注入到 LLM 的完整 prompt 结构，
    /// 不实际触发 LLM 调用，只读取已注入的永久记忆 / RAG 检索 / 当前对话历史。
    async fn context_preview(&self, _messages: &[Message]) -> Option<ContextPreview> {
        None
    }
}

//! RigAgent：通过 [rig](https://crates.io/crates/rig-core) 调用 OpenAI 兼容接口
//!
//! 统一使用 `openai::CompletionsClient`（Chat Completions API）+ 可覆盖的 base_url，
//! 支持所有 OpenAI 兼容 provider（openai/deepseek/groq/moonshot/openrouter/together/
//! mistral/perplexity/hyperbolic 以及任意 custom 兼容服务）。
//!
//! 同时支持：
//! - 非流式 `chat`：通过 `agent.prompt(prompt).await`
//! - 流式 `chat_stream`：通过 `agent.stream_prompt(prompt).await` 并过滤文本增量
//! - 工具调用：构造 agent 时注册 `SearchHistoryTool`、`GetTimeTool`、
//!   `ReadFileTool`、`ListFilesTool`、`ShellTool`、`WebFetchTool`，
//!   LLM 可主动调用以检索历史、获取时间、读写本地文件、执行 shell 命令
//!   （集成 agent-reach / browser-act）、抓取网页
//!
//! 设计要点（对齐 user_rules 中的 Rust 性能/并发规则）：
//! - `CompletionsClient` 内部已是 `Arc` 共享句柄，`RigAgent` 直接持有而**不**再包
//!   `Arc<Mutex<...>>`，避免双重锁开销。
//! - 每次 `chat`/`chat_stream` 构建一次 `Agent` 是 rig 推荐用法（builder 零成本），
//!   不缓存带状态的 agent，天然支持并发请求。
//! - `history` 通过 `Arc<RwLock<Vec<Message>>>` 共享给工具，读多写少用 RwLock。
//! - 流式实现：把 `StreamedAssistantContent` 按语义分类透传为
//!   `AgentStreamItem`（Text/Reasoning/ToolCallStart/ToolResult），
//!   让前端能分别渲染推理框、工具调用提示框与正文。

use std::sync::Arc;

use async_trait::async_trait;
use effisuite_core::{CoreError, Message, Result, Role};
use futures::stream::BoxStream;
use futures::StreamExt;
use rig_core::{
    agent::MultiTurnStreamItem,
    client::{CompletionClient, ProviderClient},
    completion::Prompt,
    message::ToolResultContent,
    providers::openai,
    streaming::{StreamedAssistantContent, StreamedUserContent, StreamingPrompt},
};
use tokio::sync::RwLock;

use crate::agent::{AgentStreamItem, ChatAgent};
use crate::tools::{
    GetTimeTool, ListFilesTool, ReadFileTool, SearchHistoryTool, ShellTool, WebFetchTool,
};

pub struct RigAgent {
    /// 统一用 CompletionsClient（Chat Completions API），兼容所有 OpenAI 兼容 provider
    client: openai::CompletionsClient,
    model_name: String,
    preamble: String,
    /// 共享历史快照，工具读取此数据做 RAG 检索
    history: Arc<RwLock<Vec<Message>>>,
    /// 是否启用工具调用
    enable_tools: bool,
}

impl RigAgent {
    /// 从环境变量 `OPENAI_API_KEY` 构造 OpenAI 客户端
    pub fn from_env(
        model_name: impl Into<String>,
        preamble: impl Into<String>,
        enable_tools: bool,
    ) -> Result<Self> {
        let client = openai::CompletionsClient::from_env()
            .map_err(|e| CoreError::Agent(format!("openai completions client init: {e}")))?;
        Ok(Self {
            client,
            model_name: model_name.into(),
            preamble: preamble.into(),
            history: Arc::new(RwLock::new(Vec::new())),
            enable_tools,
        })
    }

    /// 指定 API key + base_url 构造客户端（用于任意 OpenAI 兼容服务）
    ///
    /// - `api_key`：Bearer token，空串会被 rig 拒绝
    /// - `base_url`：可覆盖默认 `https://api.openai.com/v1`，留空则用默认
    /// - 走 Chat Completions API（`openai::CompletionsClient`）
    pub fn from_key(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model_name: impl Into<String>,
        preamble: impl Into<String>,
        enable_tools: bool,
    ) -> Result<Self> {
        let api_key = api_key.into();
        let base_url = base_url.into();
        let mut builder = openai::CompletionsClient::builder().api_key(&api_key);
        if !base_url.trim().is_empty() {
            builder = builder.base_url(&base_url);
        }
        let client = builder
            .build()
            .map_err(|e| CoreError::Agent(format!("openai completions client init: {e}")))?;
        Ok(Self {
            client,
            model_name: model_name.into(),
            preamble: preamble.into(),
            history: Arc::new(RwLock::new(Vec::new())),
            enable_tools,
        })
    }

    /// 共享 history 句柄，供外部更新（如新消息到达时 push 进去）
    #[inline]
    pub fn history_handle(&self) -> Arc<RwLock<Vec<Message>>> {
        Arc::clone(&self.history)
    }

    /// 构建一个带工具的 agent（每次调用重新构建，零成本）
    ///
    /// 装配工具时，所有工具共享同一份 history Arc，确保 LLM 调用
    /// search_history 时看到的是最新历史。
    ///
    /// 返回类型用关联类型 `<openai::CompletionsClient as CompletionClient>::CompletionModel`，
    /// 即 `GenericCompletionModel<OpenAICompletionsExt>`，统一所有 OpenAI 兼容 provider。
    fn build_agent(
        &self,
    ) -> rig_core::agent::Agent<<openai::CompletionsClient as CompletionClient>::CompletionModel>
    {
        let builder = self
            .client
            .agent(&self.model_name)
            .preamble(&self.preamble);

        if self.enable_tools {
            // 注册 RAG 检索工具：每次 build 都重新创建工具实例，但它们共享 history
            let search = SearchHistoryTool::new(Arc::clone(&self.history));
            let time = GetTimeTool::new(Arc::clone(&self.history));
            // 无状态本地能力工具：读文件、列目录、执行 shell（agent-reach/browser-act）、抓网页
            let read_file = ReadFileTool::new();
            let list_files = ListFilesTool::new();
            let shell = ShellTool::new();
            let web_fetch = WebFetchTool::new();
            builder
                .tool(search)
                .tool(time)
                .tool(read_file)
                .tool(list_files)
                .tool(shell)
                .tool(web_fetch)
                .default_max_turns(usize::MAX)
                .build()
        } else {
            builder.build()
        }
    }

    /// 把传入的 messages 同步到内部 history（便于工具读取最新上下文）
    async fn sync_history(&self, messages: &[Message]) {
        let mut h = self.history.write().await;
        // 简单策略：直接替换为最新快照，避免增量 diff 复杂度
        // 用 with_capacity 减少扩容
        if h.capacity() < messages.len() {
            *h = Vec::with_capacity(messages.len() + 8);
        }
        h.clear();
        h.extend_from_slice(messages);
    }

    /// 构建包含完整对话历史的上下文 prompt
    ///
    /// 将历史消息格式化为对话脚本，让 LLM 能看到所有之前的交流，
    /// 而非仅看到最后一条用户消息。这是解决 agent "一问三不知" 的关键。
    ///
    /// 格式：
    /// ```text
    /// [对话历史]
    /// 用户: 我们之前聊过 Rust 的异步编程
    /// 助手: 是的，Rust 的 async/await 基于 Future trait...
    /// 用户: 那 tokio 是怎么调度这些 future 的？
    /// 助手: tokio 使用 work-stealing 调度器...
    ///
    /// [当前问题]
    /// 用户: 能再详细解释一下 work-stealing 吗？
    /// ```
    ///
    /// 长消息会被截断到 800 字符，避免 token 爆炸。
    /// 仅有一条消息时不加历史头，直接返回。
    fn build_contextual_prompt(&self, messages: &[Message]) -> String {
        if messages.is_empty() {
            return "hello".to_string();
        }

        // 找到最后一条用户消息的位置
        let last_user_idx = messages
            .iter()
            .rposition(|m| m.role == Role::User)
            .unwrap_or(messages.len() - 1);

        // 如果只有一条消息（或只有一条用户消息），直接返回
        let history_msgs = &messages[..last_user_idx];
        let current_msg = &messages[last_user_idx];

        if history_msgs.is_empty() {
            return current_msg.content.clone();
        }

        // 构建完整上下文 prompt
        // 预估容量：每条消息 ~128 字节，减少扩容
        let mut prompt = String::with_capacity(messages.len() * 128);
        prompt.push_str("[对话历史]\n");

        for m in history_msgs {
            let role_label = match m.role {
                Role::User => "用户",
                Role::Assistant => "助手",
                Role::System => "系统",
            };
            let content = truncate_for_context(&m.content, 800);
            prompt.push_str(role_label);
            prompt.push_str(": ");
            prompt.push_str(&content);
            prompt.push('\n');
        }

        prompt.push_str("\n[当前问题]\n用户: ");
        prompt.push_str(&current_msg.content);
        prompt
    }
}

/// 截断过长消息以控制上下文 token 数
///
/// 在字符边界处截断，避免截断多字节 UTF-8 字符导致 panic。
/// 截断后追加 "…" 提示内容被省略。
fn truncate_for_context(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }
    let boundary = content.ceil_char_boundary(max_chars);
    let mut s = String::with_capacity(boundary + 3);
    s.push_str(&content[..boundary]);
    s.push('…');
    s
}

/// 从 `OneOrMany<ToolResultContent>` 提取可读文本
///
/// - Text 块：直接取 `.text`
/// - Image 块：占位为 `[image]`
/// - 多块内容用 `\n` 连接
fn extract_tool_output(content: &rig_core::OneOrMany<ToolResultContent>) -> String {
    let parts: Vec<String> = content
        .iter()
        .map(|c| match c {
            ToolResultContent::Text(t) => t.text.clone(),
            ToolResultContent::Image(_) => "[image]".to_string(),
        })
        .collect();
    parts.join("\n")
}

#[async_trait]
impl ChatAgent for RigAgent {
    async fn chat(&self, messages: &[Message]) -> Result<String> {
        // 先同步历史，让工具能看到本轮上下文
        self.sync_history(messages).await;

        let agent = self.build_agent();
        // 使用完整对话历史上下文，而非仅取最后一条用户消息
        let prompt = self.build_contextual_prompt(messages);

        let resp = agent
            .prompt(&prompt)
            .await
            .map_err(|e| CoreError::Agent(format!("rig prompt: {e}")))?;

        Ok(resp)
    }

    fn chat_stream<'a>(
        &'a self,
        messages: &'a [Message],
    ) -> BoxStream<'a, Result<AgentStreamItem>> {
        let s = async_stream::stream! {
            // 先同步历史
            self.sync_history(messages).await;

            let agent = self.build_agent();
            // 使用完整对话历史上下文，而非仅取最后一条用户消息
            let prompt = self.build_contextual_prompt(messages);

            // stream_prompt 返回 StreamingPromptRequest，await 后直接得到流
            let mut stream = agent.stream_prompt(prompt).await;

            while let Some(item) = stream.next().await {
                match item {
                    Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text))) => {
                        // 文本增量，emit 给前端
                        if !text.text.is_empty() {
                            yield Ok(AgentStreamItem::Text { content: text.text });
                        }
                    }
                    Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Reasoning(r))) => {
                        // 完整推理块：用 display_text() 合并所有 Text/Summary/Redacted 块
                        let text = r.display_text();
                        if !text.is_empty() {
                            yield Ok(AgentStreamItem::Reasoning { content: text });
                        }
                    }
                    Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::ReasoningDelta { reasoning, .. })) => {
                        // 推理增量：reasoning 字段已是 String，直接透传
                        if !reasoning.is_empty() {
                            yield Ok(AgentStreamItem::Reasoning { content: reasoning });
                        }
                    }
                    Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::ToolCall { tool_call, internal_call_id })) => {
                        // 模型决定调用工具：透传工具名与参数
                        yield Ok(AgentStreamItem::ToolCallStart {
                            call_id: internal_call_id,
                            tool_name: tool_call.function.name.clone(),
                            arguments: tool_call.function.arguments.clone(),
                        });
                    }
                    Ok(MultiTurnStreamItem::StreamUserItem(StreamedUserContent::ToolResult { tool_result, internal_call_id })) => {
                        // 工具执行结果：从 OneOrMany<ToolResultContent> 提取文本
                        let output = extract_tool_output(&tool_result.content);
                        let is_error = output.starts_with("Error:") || output.starts_with("error:");
                        yield Ok(AgentStreamItem::ToolResult {
                            call_id: internal_call_id,
                            output,
                            is_error,
                        });
                    }
                    Ok(MultiTurnStreamItem::StreamAssistantItem(_)) => {
                        // ToolCallDelta 等增量事件暂不透传，避免噪音
                        continue;
                    }
                    Ok(MultiTurnStreamItem::ToolExecutionStart { .. }) => {
                        // rig 执行工具前的事件，与 ToolCall 重复，跳过
                        continue;
                    }
                    Ok(MultiTurnStreamItem::CompletionCall(_)) => {
                        // 单次 completion 请求的 usage 统计，暂不透传
                        continue;
                    }
                    Ok(MultiTurnStreamItem::FinalResponse(_)) => {
                        // 流结束
                        return;
                    }
                    Ok(_) => {
                        // non_exhaustive 兜底：未来新增变体暂不透传
                        continue;
                    }
                    Err(e) => {
                        yield Err(CoreError::Agent(format!("rig stream item: {e}")));
                        return;
                    }
                }
            }
        };
        Box::pin(s)
    }

    #[inline]
    fn name(&self) -> &str {
        &self.model_name
    }

    #[inline]
    fn backend(&self) -> &'static str {
        "rig-openai-compat"
    }
}

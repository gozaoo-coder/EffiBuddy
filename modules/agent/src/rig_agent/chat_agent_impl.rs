//! `ChatAgent` trait 实现
//!
//! 把 [`RigAgent`] 接入统一的对话后端抽象：
//! - 非流式 [`ChatAgent::chat`]：构建 agent → prompt → 返回字符串
//! - 流式 [`ChatAgent::chat_stream`]：构建 agent → stream_prompt → 把 rig 的
//!   `StreamedAssistantContent` / `StreamedUserContent` / `MultiTurnStreamItem`
//!   按语义透传为 [`AgentStreamItem`]（Text / Reasoning / ToolCallStart / ToolResult / Usage）
//! - 上下文预览 [`ChatAgent::context_preview`]：委托给 [`RigAgent::build_context_preview`]
//!
//! 流式实现的关键点：
//! - 文本 / 推理增量分别 emit，前端能分别渲染推理框与正文
//! - 工具调用开始 + 工具执行结果配对透传，前端能渲染工具调用提示框
//! - 单次 completion 的 `Usage` 也透传，前端累计得到本轮总 token 消耗
//! - skill preamble 提到未注册工具时把 `UnknownToolCall` 错误转为可读文本，
//!   避免中断整个流

use async_trait::async_trait;
use effisuite_core::{CoreError, Message, Result};
use futures::stream::BoxStream;
use futures::StreamExt;
use rig_core::{
    agent::MultiTurnStreamItem,
    completion::Prompt,
    message::ToolResultContent,
    streaming::{StreamedAssistantContent, StreamedUserContent, StreamingPrompt},
};

use crate::agent::{AgentStreamItem, ChatAgent, ContextPreview};

use super::RigAgent;

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
        // 读取当前工作区快照注入到文件/shell 工具
        let cwd = self.working_dir.read().await.clone();

        let agent = self.build_agent(cwd);
        // 使用完整对话历史上下文，而非仅取最后一条用户消息
        let prompt = self.build_contextual_prompt(messages).await;

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
            // 读取当前工作区快照注入到文件/shell 工具
            let cwd = self.working_dir.read().await.clone();

            let agent = self.build_agent(cwd);
            // 使用完整对话历史上下文，而非仅取最后一条用户消息
            let prompt = self.build_contextual_prompt(messages).await;

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
                    Ok(MultiTurnStreamItem::CompletionCall(call)) => {
                        // 透传单次 completion 请求的 usage 统计：
                        // 前端累计所有 Usage 事件得到本轮对话的总 token 消耗。
                        // provider 未返回 usage 时所有字段为 0（rig 的零值哨兵），
                        // 前端可据此判断是否展示 token 统计。
                        // cache_hit/cache_miss 来自 DeepSeek 的 prompt_cache_hit_tokens /
                        // prompt_cache_miss_tokens（rig-core vendored patch 映射到
                        // cached_input_tokens / cache_creation_input_tokens）。
                        yield Ok(AgentStreamItem::Usage {
                            input_tokens: call.usage.input_tokens,
                            output_tokens: call.usage.output_tokens,
                            total_tokens: call.usage.total_tokens,
                            reasoning_tokens: call.usage.reasoning_tokens,
                            cache_hit_tokens: call.usage.cached_input_tokens,
                            cache_miss_tokens: call.usage.cache_creation_input_tokens,
                        });
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
                        let err_str = e.to_string();
                        // skill 的 preamble 可能提到未在 agent 中注册的工具，
                        // 直接报错会中断整个流。这里把 UnknownToolCall 转换为
                        // 一条可读的 assistant 文本，让前端正常显示并结束本轮。
                        if err_str.contains("UnknownToolCall") {
                            let tool_name = err_str
                                .split('`')
                                .nth(1)
                                .filter(|s| !s.is_empty())
                                .unwrap_or("unknown");
                            yield Ok(AgentStreamItem::Text {
                                content: format!(
                                    "⚠️ 我尝试调用工具 `{tool_name}`，但它未在当前 agent 中注册。\
                                     这通常是因为某个 skill 的说明里提到了未实现的工具。\
                                     请检查该 skill 是否完整安装，或让我改用 shell / 其他可用工具继续。"
                                ),
                            });
                            return;
                        }
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

    /// 委托给 `build_context_preview`，返回结构化上下文预览供前端展示
    async fn context_preview(&self, messages: &[Message]) -> Option<ContextPreview> {
        Some(self.build_context_preview(messages).await)
    }
}

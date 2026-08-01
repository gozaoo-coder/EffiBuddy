//! 消息压缩 agent
//!
//! 独立的 client + 流式 / 非流式调用入口，复用主对话 agent 的
//! `api_key` / `base_url` / `model_name`，但使用压缩专用 preamble
//! （[`COMPRESSION_PREAMBLE`]）。非流式调用适合后台压缩任务，
//! 流式调用适合前端实时展示压缩进度。
//!
//! 调用方负责把返回的文本交给 `effisuite_core::parse_compression_response` 解析。

use effisuite_core::{CoreError, Result};
use futures::stream::BoxStream;
use futures::StreamExt;
use rig_core::{
    agent::MultiTurnStreamItem,
    client::CompletionClient,
    completion::Prompt,
    providers::openai,
    streaming::{StreamedAssistantContent, StreamingPrompt},
};

/// 压缩 agent 专用 preamble：说明三种 method 与 XML 输出格式
pub const COMPRESSION_PREAMBLE: &str = "\
你是一个对话历史压缩助手。你的任务是分析一段聊天记录，给出压缩决策以减少 token 占用，同时保留关键信息。\n\
\n\
规则：\n\
1. 每条消息（completion）都有一个 id。你需要对消息 id 做出三种决策之一：\n\
   - 保持：内容仍然需要（后续会引用、任务追踪、代码上下文等）\n\
   - 隐藏：与当前话题无关、已完成的工具调用、用户已离开的话题\n\
   - 替换：内容冗长或语义混乱，用更简短准确的表述替代\n\
2. 一次回复可以包含多个 <act> 决策，每个 act 可以同时处理多个 completionId\n\
3. 输出格式（严格遵守）：\n\
\n\
<act>\n\
  <reason>简短理由</reason>\n\
  <method>保持|隐藏|替换</method>\n\
  <completionId>[id1,id2]</completionId>\n\
</act>\n\
\n\
<act>\n\
  <reason>简短理由</reason>\n\
  <method>替换</method>\n\
  <completionId>[id3]</completionId>\n\
  <newContent>压缩后的新内容</newContent>\n\
</act>\n\
\n\
注意：\n\
- method 必须是 保持/隐藏/替换 三者之一\n\
- completionId 是 JSON 数组格式 [id1,id2,...]\n\
- 只有 method=替换 时才需要 <newContent>\n\
- 不需要压缩的消息可以不输出 act（默认保持）";

/// 调用压缩 agent 分析对话历史，返回原始文本回复（含 `<act>` 块）
///
/// 复用主对话 agent 的 `api_key` / `base_url` / `model_name`，但使用压缩专用
/// preamble（[`COMPRESSION_PREAMBLE`]）。非流式调用，适合后台压缩任务。
///
/// 调用方负责把返回的文本交给 [`effisuite_core::parse_compression_response`] 解析。
pub async fn call_compression_agent(
    api_key: &str,
    base_url: &str,
    model_name: &str,
    prompt: &str,
) -> Result<String> {
    let mut builder = openai::CompletionsClient::builder().api_key(api_key);
    if !base_url.trim().is_empty() {
        builder = builder.base_url(base_url);
    }
    let client = builder
        .build()
        .map_err(|e| CoreError::Agent(format!("compression client init: {e}")))?;
    let agent = client
        .agent(model_name)
        .preamble(COMPRESSION_PREAMBLE)
        .build();
    let resp = agent
        .prompt(prompt)
        .await
        .map_err(|e| CoreError::Agent(format!("compression prompt: {e}")))?;
    Ok(resp)
}

/// 压缩 agent 流式事件：把流式响应分类透传给调用方
///
/// - `Token(String)`：文本增量（含 `<act>` 块的逐步输出）
/// - `Done(String)`：完整响应文本（所有 token 拼接）
///
/// 设计与 [`crate::AgentStreamItem`] 的 Text/Reasoning 透传一致，但压缩 agent 不会
/// 触发工具调用，故仅区分 Token 与 Done。
#[derive(Debug, Clone)]
pub enum CompressionStreamItem {
    /// 文本增量
    Token(String),
    /// 流结束：完整响应文本
    Done(String),
}

/// 调用压缩 agent（流式版本）
///
/// 返回 `BoxStream<'static, Result<CompressionStreamItem>>`，调用方逐项消费：
/// - `Token(text)`：实时 emit 给前端展示进度
/// - `Done(full_text)`：解析 `<act>` 块并持久化
///
/// 与 [`call_compression_agent`] 共享 client 构造逻辑，但走 `stream_prompt` 路径。
/// 内部 `await` 仅在流项到达时，无阻塞 sleep。
pub fn call_compression_agent_stream(
    api_key: &str,
    base_url: &str,
    model_name: &str,
    prompt: &str,
) -> BoxStream<'static, Result<CompressionStreamItem>> {
    let api_key = api_key.to_string();
    let base_url = base_url.to_string();
    let model_name = model_name.to_string();
    let prompt = prompt.to_string();

    let s = async_stream::stream! {
        let mut builder = openai::CompletionsClient::builder().api_key(&api_key);
        if !base_url.trim().is_empty() {
            builder = builder.base_url(&base_url);
        }
        let client = builder
            .build()
            .map_err(|e| CoreError::Agent(format!("compression client init: {e}")))?;
        let agent = client
            .agent(&model_name)
            .preamble(COMPRESSION_PREAMBLE)
            .build();

        let mut stream = agent.stream_prompt(&prompt).await;
        let mut full = String::with_capacity(1024);

        while let Some(item) = stream.next().await {
            match item {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text))) => {
                    if !text.text.is_empty() {
                        full.push_str(&text.text);
                        yield Ok(CompressionStreamItem::Token(text.text));
                    }
                }
                Ok(MultiTurnStreamItem::StreamAssistantItem(_)) => {
                    // Reasoning / ToolCallDelta 等暂不透传，压缩 agent 不应调用工具
                    continue;
                }
                Ok(_) => {
                    // ToolResult / CompletionCall 等忽略
                    continue;
                }
                Err(e) => {
                    yield Err(CoreError::Agent(format!("compression stream: {e}")));
                    return;
                }
            }
        }
        yield Ok(CompressionStreamItem::Done(full));
    };
    Box::pin(s)
}

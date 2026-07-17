//! RigAgent：通过 [rig](https://crates.io/crates/rig-core) 调用 OpenAI 兼容接口
//!
//! 统一使用 `openai::CompletionsClient`（Chat Completions API）+ 可覆盖的 base_url，
//! 支持所有 OpenAI 兼容 provider（openai/deepseek/groq/moonshot/openrouter/together/
//! mistral/perplexity/hyperbolic 以及任意 custom 兼容服务）。
//!
//! 同时支持：
//! - 非流式 `chat`：通过 `agent.prompt(prompt).await`
//! - 流式 `chat_stream`：通过 `agent.stream_prompt(prompt).await` 并过滤文本增量
//! - 工具调用：构造 agent 时注册 `SearchHistoryTool` 和 `GetTimeTool`，
//!   LLM 可主动调用以检索历史或获取时间
//!
//! 设计要点（对齐 user_rules 中的 Rust 性能/并发规则）：
//! - `CompletionsClient` 内部已是 `Arc` 共享句柄，`RigAgent` 直接持有而**不**再包
//!   `Arc<Mutex<...>>`，避免双重锁开销。
//! - 每次 `chat`/`chat_stream` 构建一次 `Agent` 是 rig 推荐用法（builder 零成本），
//!   不缓存带状态的 agent，天然支持并发请求。
//! - `history` 通过 `Arc<RwLock<Vec<Message>>>` 共享给工具，读多写少用 RwLock。
//! - 流式实现：仅过滤 `StreamedAssistantContent::Text` 作为用户可见增量，
//!   tool call / reasoning 等不直接 emit 给前端（避免噪音）。

use std::sync::Arc;

use async_trait::async_trait;
use effisuite_core::{CoreError, Message, Result, Role};
use futures::stream::BoxStream;
use futures::StreamExt;
use rig_core::{
    agent::MultiTurnStreamItem,
    client::{CompletionClient, ProviderClient},
    completion::Prompt,
    providers::openai,
    streaming::{StreamedAssistantContent, StreamingPrompt},
};
use tokio::sync::RwLock;

use crate::agent::ChatAgent;
use crate::tools::{GetTimeTool, SearchHistoryTool};

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
            builder
                .tool(search)
                .tool(time)
                .default_max_turns(10)
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

    /// 从 messages 取最后一条 user 消息作为本轮 prompt
    fn extract_prompt<'a>(&self, messages: &'a [Message]) -> &'a str {
        messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_str())
            .unwrap_or("hello")
    }
}

#[async_trait]
impl ChatAgent for RigAgent {
    async fn chat(&self, messages: &[Message]) -> Result<String> {
        // 先同步历史，让工具能看到本轮上下文
        self.sync_history(messages).await;

        let agent = self.build_agent();
        let prompt = self.extract_prompt(messages);

        let resp = agent
            .prompt(prompt)
            .await
            .map_err(|e| CoreError::Agent(format!("rig prompt: {e}")))?;

        Ok(resp)
    }

    fn chat_stream<'a>(
        &'a self,
        messages: &'a [Message],
    ) -> BoxStream<'a, Result<String>> {
        // 用 async_stream 风格：把 async block 转为 stream
        let s = async_stream::stream! {
            // 先同步历史
            self.sync_history(messages).await;

            let agent = self.build_agent();
            let prompt = self.extract_prompt(messages).to_string();

            // stream_prompt 返回 StreamingPromptRequest，await 后直接得到流
            // （StreamingResult = Pin<Box<dyn Stream<Item = Result<MultiTurnStreamItem, StreamingError>>>>
            //   不是 Result<Stream>，错误会在流的 item 中以 Err 出现）
            let mut stream = agent.stream_prompt(prompt).await;

            while let Some(item) = stream.next().await {
                match item {
                    Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text))) => {
                        // 文本增量，emit 给前端
                        if !text.text.is_empty() {
                            yield Ok(text.text);
                        }
                    }
                    Ok(MultiTurnStreamItem::FinalResponse(_)) => {
                        // 流结束
                        return;
                    }
                    Ok(_) => {
                        // tool call / reasoning 等不 emit 给前端
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

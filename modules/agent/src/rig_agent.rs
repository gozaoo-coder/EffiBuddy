//! RigAgent：通过 [rig](https://crates.io/crates/rig-core) 调用 OpenAI 兼容接口
//!
//! 统一使用 `openai::CompletionsClient`（Chat Completions API）+ 可覆盖的 base_url，
//! 支持所有 OpenAI 兼容 provider（openai/deepseek/groq/moonshot/openrouter/together/
//! mistral/perplexity/hyperbolic 以及任意 custom 兼容服务）。
//!
//! 同时支持：
//! - 非流式 `chat`：通过 `agent.prompt(prompt).await`
//! - 流式 `chat_stream`：通过 `agent.stream_prompt(prompt).await` 并过滤文本增量
//! - 工具调用：构造 agent 时注册 `SearchHistoryTool`、`SearchMemoryTool`、
//!   `GetTimeTool`、`ReadFileTool`、`ListFilesTool`、`ShellTool`、`WebFetchTool`，
//!   LLM 可主动调用以检索历史、跨会话记忆、获取时间、读写本地文件、
//!   执行 shell 命令（集成 agent-reach / browser-act）、抓取网页
//! - **RAG 记忆增强**：每次对话前自动通过 `MemoryIndex` 检索相关跨会话历史，
//!   注入到 prompt 的 `[相关历史记忆]` 区段（"自动提供上文"）
//!
//! 设计要点（对齐 user_rules 中的 Rust 性能/并发规则）：
//! - `CompletionsClient` 内部已是 `Arc` 共享句柄，`RigAgent` 直接持有而**不**再包
//!   `Arc<Mutex<...>>`，避免双重锁开销。
//! - 每次 `chat`/`chat_stream` 构建一次 `Agent` 是 rig 推荐用法（builder 零成本），
//!   不缓存带状态的 agent，天然支持并发请求。
//! - `history` 通过 `Arc<RwLock<Vec<Message>>>` 共享给工具，读多写少用 RwLock。
//! - `MemoryIndex` 与 `current_conversation_id` 句柄由 Tauri 层注入并跨请求共享，
//!   检索时排除当前会话避免与已注入上下文重复。
//! - 流式实现：把 `StreamedAssistantContent` 按语义分类透传为
//!   `AgentStreamItem`（Text/Reasoning/ToolCallStart/ToolResult），
//!   让前端能分别渲染推理框、工具调用提示框与正文。

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use effisuite_core::{
    ConversationStore, CoreError, MemoryIndex, Message, PinnedMemoryStore, Result, Role,
};
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
    DeletePinnedMemoryTool, GetTimeTool, ImageGenConfig, ImageGenTool, ListFilesTool,
    ListPinnedMemoriesTool, PinMemoryTool, ReadFileTool, SearchHistoryTool, SearchMemoryTool,
    SetTitleTool, ShellTool, WebFetchTool,
};

/// 自动注入的相关历史记忆条数上限
const MEMORY_AUTO_INJECT_LIMIT: usize = 5;
/// 当启用记忆增强时，当前对话保留在 prompt 中的最近消息条数
/// （更早的消息由 RAG 检索覆盖，避免 token 爆炸）
const RECENT_HISTORY_WITH_MEMORY: usize = 10;
/// 单条历史消息截断字符数
const HISTORY_TRUNCATE_CHARS: usize = 800;

pub struct RigAgent {
    /// 统一用 CompletionsClient（Chat Completions API），兼容所有 OpenAI 兼容 provider
    client: openai::CompletionsClient,
    model_name: String,
    preamble: String,
    /// 共享历史快照，工具读取此数据做 RAG 检索
    history: Arc<RwLock<Vec<Message>>>,
    /// 是否启用工具调用
    enable_tools: bool,
    /// 跨会话历史记忆索引（RAG 记忆增强核心）
    /// None 时退化为旧行为（包含全部当前对话历史）
    memory: Option<Arc<MemoryIndex>>,
    /// 永久记忆存储（用户主动要求"记住"的内容）
    /// None 时关闭永久记忆能力；Some 时每轮 prompt 都注入 `[永久记忆]` 段
    pinned_memory: Option<Arc<PinnedMemoryStore>>,
    /// 当前会话 id 句柄，由 Tauri 命令层在每次 send_message 前更新；
    /// search_memory 工具与自动注入都据此排除当前会话
    current_conversation_id: Arc<RwLock<Option<String>>>,
    /// 当前工作区路径句柄，由 Tauri 命令层在每次 send_message 前更新。
    /// read_file / list_files / shell 据此解析相对路径与设置子进程 cwd。
    /// 优先级：会话级 working_dir > 技能级 working_dir > 进程默认 cwd。
    working_dir: Arc<RwLock<Option<PathBuf>>>,
    /// 图像生成模型配置句柄：set_active_model 切到 kind=ImageGen 的模型时更新。
    /// build_agent 注入到 ImageGenTool；为 None 时 image_gen 工具返回错误。
    image_gen_config: Arc<RwLock<Option<ImageGenConfig>>>,
    /// 附件保存目录（绝对路径），ImageGenTool 把生成图片落盘到此目录。
    attachments_dir: PathBuf,
    /// 会话存储句柄，SetTitleTool 据此调用 store.rename 持久化标题
    store: Arc<ConversationStore>,
}

impl RigAgent {
    /// 从环境变量 `OPENAI_API_KEY` 构造 OpenAI 客户端
    ///
    /// `memory` 与 `current_conversation_id` 用于 RAG 记忆增强；传 None / 默认句柄
    /// 则关闭记忆增强能力（行为同旧版）。`pinned_memory` 用于永久记忆注入。
    /// `working_dir` 共享句柄用于工作区路径注入，传新的空句柄则默认不限制。
    /// `image_gen_config` 共享句柄供 ImageGenTool 读取，None 则 image_gen 工具不可用。
    /// `attachments_dir` 为图片落盘目录（绝对路径）。
    /// `store` 共享会话存储句柄，SetTitleTool 据此持久化标题。
    pub fn from_env(
        model_name: impl Into<String>,
        preamble: impl Into<String>,
        enable_tools: bool,
        memory: Option<Arc<MemoryIndex>>,
        pinned_memory: Option<Arc<PinnedMemoryStore>>,
        current_conversation_id: Arc<RwLock<Option<String>>>,
        working_dir: Arc<RwLock<Option<PathBuf>>>,
        image_gen_config: Arc<RwLock<Option<ImageGenConfig>>>,
        attachments_dir: PathBuf,
        store: Arc<ConversationStore>,
    ) -> Result<Self> {
        let client = openai::CompletionsClient::from_env()
            .map_err(|e| CoreError::Agent(format!("openai completions client init: {e}")))?;
        Ok(Self {
            client,
            model_name: model_name.into(),
            preamble: preamble.into(),
            history: Arc::new(RwLock::new(Vec::new())),
            enable_tools,
            memory,
            pinned_memory,
            current_conversation_id,
            working_dir,
            image_gen_config,
            attachments_dir,
            store,
        })
    }

    /// 指定 API key + base_url 构造客户端（用于任意 OpenAI 兼容服务）
    ///
    /// - `api_key`：Bearer token，空串会被 rig 拒绝
    /// - `base_url`：可覆盖默认 `https://api.openai.com/v1`，留空则用默认
    /// - 走 Chat Completions API（`openai::CompletionsClient`）
    /// - `memory` 注入后启用 RAG 记忆增强（自动上文 + search_memory 工具）
    /// - `pinned_memory` 注入后启用永久记忆（每轮注入 `[永久记忆]` 段 + pin_memory 工具）
    /// - `working_dir` 共享句柄用于工作区路径注入
    /// - `image_gen_config` 共享句柄供 ImageGenTool 读取（用户切换到 image_gen 模型时由
    ///   Tauri 命令层更新），None 时 image_gen 工具调用返回错误提示
    /// - `attachments_dir` 为图片落盘目录（绝对路径），ImageGenTool 据此保存生成结果
    /// - `store` 共享会话存储句柄，SetTitleTool 据此持久化标题
    pub fn from_key(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model_name: impl Into<String>,
        preamble: impl Into<String>,
        enable_tools: bool,
        memory: Option<Arc<MemoryIndex>>,
        pinned_memory: Option<Arc<PinnedMemoryStore>>,
        current_conversation_id: Arc<RwLock<Option<String>>>,
        working_dir: Arc<RwLock<Option<PathBuf>>>,
        image_gen_config: Arc<RwLock<Option<ImageGenConfig>>>,
        attachments_dir: PathBuf,
        store: Arc<ConversationStore>,
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
            memory,
            pinned_memory,
            current_conversation_id,
            working_dir,
            image_gen_config,
            attachments_dir,
            store,
        })
    }

    /// 共享 image_gen_config 句柄，供 Tauri 命令层在 set_active_model 时更新
    #[inline]
    pub fn image_gen_config_handle(&self) -> Arc<RwLock<Option<ImageGenConfig>>> {
        Arc::clone(&self.image_gen_config)
    }

    /// 共享 working_dir 句柄，供 Tauri 命令层在每次 send_message 前更新。
    /// 优先级：会话级 working_dir > 技能级 working_dir > 进程默认 cwd。
    #[inline]
    pub fn working_dir_handle(&self) -> Arc<RwLock<Option<PathBuf>>> {
        Arc::clone(&self.working_dir)
    }

    /// 共享 history 句柄，供外部更新（如新消息到达时 push 进去）
    #[inline]
    pub fn history_handle(&self) -> Arc<RwLock<Vec<Message>>> {
        Arc::clone(&self.history)
    }

    /// 构建一个带工具的 agent（每次调用重新构建，零成本）
    ///
    /// 装配工具时，所有工具共享同一份 history Arc 与 current_conversation_id Arc，
    /// 确保 LLM 调用 search_history / search_memory 时看到的是最新上下文。
    /// `cwd` 快照在调用前从 `working_dir` RwLock 读取，注入到文件/shell 工具。
    ///
    /// 返回类型用关联类型 `<openai::CompletionsClient as CompletionClient>::CompletionModel`，
    /// 即 `GenericCompletionModel<OpenAICompletionsExt>`，统一所有 OpenAI 兼容 provider。
    fn build_agent(
        &self,
        cwd: Option<PathBuf>,
    ) -> rig_core::agent::Agent<<openai::CompletionsClient as CompletionClient>::CompletionModel>
    {
        let builder = self
            .client
            .agent(&self.model_name)
            .preamble(&self.preamble);

        if self.enable_tools {
            // 注册会话内检索工具：每次 build 都重新创建工具实例，但它们共享 history
            let search = SearchHistoryTool::new(Arc::clone(&self.history));
            let time = GetTimeTool::new(Arc::clone(&self.history));
            // 本地能力工具：读文件、列目录、执行 shell（agent-reach/browser-act）、抓网页
            // 注入工作区 cwd（若有），相对路径以此为准
            let read_file = match &cwd {
                Some(p) => ReadFileTool::with_cwd(p.clone()),
                None => ReadFileTool::new(),
            };
            let list_files = match &cwd {
                Some(p) => ListFilesTool::with_cwd(p.clone()),
                None => ListFilesTool::new(),
            };
            let shell = match &cwd {
                Some(p) => ShellTool::with_cwd(p.clone()),
                None => ShellTool::new(),
            };
            let web_fetch = WebFetchTool::new();
            // 图像生成工具：共享 image_gen_config 句柄，调用时读取最新配置。
            // 用户切换到 kind=ImageGen 的模型时由 Tauri 命令层更新 config，
            // LLM 可主动调用此工具为用户生成图片。
            let image_gen = ImageGenTool::new(
                Arc::clone(&self.image_gen_config),
                self.attachments_dir.clone(),
            );
            // 会话标题设置工具：LLM 据此为会话生成/更新标题（≤25 字）
            // 共享 store 与 current_conversation_id 句柄，调用时直接落盘
            let set_title = SetTitleTool::new(
                Arc::clone(&self.store),
                Arc::clone(&self.current_conversation_id),
            );

            let mut b = builder
                .tool(search)
                .tool(time)
                .tool(read_file)
                .tool(list_files)
                .tool(shell)
                .tool(web_fetch)
                .tool(image_gen)
                .tool(set_title);

            // 跨会话记忆检索工具：仅在 MemoryIndex 可用时注册
            if let Some(memory) = &self.memory {
                let search_memory = SearchMemoryTool::new(
                    Arc::clone(memory),
                    Arc::clone(&self.current_conversation_id),
                );
                b = b.tool(search_memory);
            }

            // 永久记忆工具：仅在 PinnedMemoryStore 可用时注册
            // 让 LLM 能在用户说"请记住..."时主动调用 pin_memory 落盘
            if let Some(pinned) = &self.pinned_memory {
                let pin = PinMemoryTool::new(
                    Arc::clone(pinned),
                    Arc::clone(&self.current_conversation_id),
                );
                let list_pinned = ListPinnedMemoriesTool::new(Arc::clone(pinned));
                let delete_pinned = DeletePinnedMemoryTool::new(Arc::clone(pinned));
                b = b.tool(pin).tool(list_pinned).tool(delete_pinned);
            }

            b.default_max_turns(usize::MAX).build()
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
    /// 启用 RAG 记忆增强 + 永久记忆时的格式：
    /// ```text
    /// [永久记忆]（用户要求永久记住的内容，请始终遵守/参考）
    /// 1. [preference] 用户偏好深色主题
    /// 2. 我的工作邮箱是 hr@effisuite.com
    ///
    /// [相关历史记忆]（来自其他对话，供参考）
    /// 1. [会话abc123] [用户] 我们之前聊过 Rust 的异步编程
    /// 2. [会话def456] [助手] tokio 使用 work-stealing 调度器...
    ///
    /// [当前对话最近]
    /// 用户: 那 tokio 是怎么调度这些 future 的？
    /// 助手: tokio 使用 work-stealing 调度器...
    ///
    /// [当前问题]
    /// 用户: 能再详细解释一下 work-stealing 吗？
    /// ```
    ///
    /// - `[永久记忆]` 段：每轮**始终**注入（不依赖检索），来自 PinnedMemoryStore
    /// - `[相关历史记忆]` 段：按当前问题做 RAG 检索后注入
    /// - 未启用记忆增强或无相关记忆时退化为旧行为：包含全部当前对话历史
    /// - 长消息会被截断到 800 字符，避免 token 爆炸
    async fn build_contextual_prompt(&self, messages: &[Message]) -> String {
        if messages.is_empty() {
            return "hello".to_string();
        }

        // 找到最后一条用户消息的位置
        let last_user_idx = messages
            .iter()
            .rposition(|m| m.role == Role::User)
            .unwrap_or(messages.len() - 1);
        let current_msg = &messages[last_user_idx];

        // 0. 永久记忆段：始终注入（不依赖检索相关性）
        let pinned_section = if let Some(pinned) = &self.pinned_memory {
            pinned.format_for_context().await
        } else {
            String::new()
        };

        // 1. 若启用记忆增强，检索跨会话相关历史
        let memory_section = if let Some(memory) = &self.memory {
            // 跳过过短查询（如单字符）避免无意义检索
            let query = current_msg.content.trim();
            if query.len() < 2 {
                String::new()
            } else {
                let exclude = self.current_conversation_id.read().await.clone();
                let hits = memory
                    .search_hybrid(query, MEMORY_AUTO_INJECT_LIMIT, exclude.as_deref())
                    .await;
                if hits.is_empty() {
                    String::new()
                } else {
                    format_memory_section(&hits)
                }
            }
        } else {
            String::new()
        };

        // 2. 当前对话历史：启用记忆时只取最近 N 条，否则取全部（旧行为）
        let history_msgs: &[Message] = if self.memory.is_some() {
            let start = last_user_idx.saturating_sub(RECENT_HISTORY_WITH_MEMORY);
            &messages[start..last_user_idx]
        } else {
            &messages[..last_user_idx]
        };

        // 3. 若无永久记忆、无历史且无 RAG 记忆，直接返回当前问题
        if pinned_section.is_empty()
            && history_msgs.is_empty()
            && memory_section.is_empty()
        {
            return current_msg.content.clone();
        }

        // 4. 拼装 prompt
        // 预估容量：永久记忆段 + RAG 记忆段 + 历史段 + 当前问题
        let mut prompt = String::with_capacity(
            pinned_section.len()
                + memory_section.len()
                + history_msgs.len() * 128
                + current_msg.content.len()
                + 64,
        );

        if !pinned_section.is_empty() {
            prompt.push_str(&pinned_section);
            prompt.push('\n');
        }

        if !memory_section.is_empty() {
            prompt.push_str(&memory_section);
            prompt.push('\n');
        }

        if !history_msgs.is_empty() {
            prompt.push_str("[当前对话最近]\n");
            for m in history_msgs {
                let role_label = match m.role {
                    Role::User => "用户",
                    Role::Assistant => "助手",
                    Role::System => "系统",
                };
                let content = truncate_for_context(&m.content, HISTORY_TRUNCATE_CHARS);
                prompt.push_str(role_label);
                prompt.push_str(": ");
                prompt.push_str(&content);
                prompt.push('\n');
            }
            prompt.push('\n');
        }

        prompt.push_str("[当前问题]\n用户: ");
        prompt.push_str(&current_msg.content);
        prompt
    }
}

/// 格式化记忆增强的 `[相关历史记忆]` 段落
///
/// 输出格式：
/// ```text
/// [相关历史记忆]（来自其他对话，供参考）
/// 1. [会话abc12345] [用户] 我们之前聊过 Rust 的异步编程
/// 2. [会话def67890] [助手] tokio 使用 work-stealing 调度器...
/// ```
fn format_memory_section(hits: &[effisuite_core::MemoryHit]) -> String {
    let mut s = String::with_capacity(hits.len() * 128 + 32);
    s.push_str("[相关历史记忆]（来自其他对话，供参考）\n");
    for (i, hit) in hits.iter().enumerate() {
        let role = match hit.role {
            Role::User => "用户",
            Role::Assistant => "助手",
            Role::System => "系统",
        };
        s.push_str(&format!(
            "{}. [会话{}] [{}] {}\n",
            i + 1,
            short_conv_id(&hit.conversation_id),
            role,
            hit.snippet
        ));
    }
    s
}

/// 截断会话 id 用于显示（取前 8 字符，UTF-8 边界安全）
#[inline]
fn short_conv_id(id: &str) -> &str {
    if id.len() <= 8 {
        id
    } else {
        &id[..id.ceil_char_boundary(8)]
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

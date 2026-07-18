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
//!   `GetTimeTool`、`ReadFileTool`、`WriteFileTool`、`ListFilesTool`、`ShellTool`、`WebFetchTool`、
//!   `ImageGenTool`、`DisplayImageTool`，
//!   LLM 可主动调用以检索历史、跨会话记忆、获取时间、读写本地文件、
//!   执行 shell 命令（集成 agent-reach / browser-act）、抓取网页、
//!   生成图片、把已有图片推送到聊天框展示
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

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use effisuite_core::{
    ClawHubClient, CompressionStore, ConversationStore, CoreError, MemoryIndex, Message,
    PinnedMemoryStore, Result, Role, SkillIndex, SkillStore, apply_compression,
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

use crate::agent::{AgentStreamItem, ChatAgent, ContextPreview};
use crate::tools::{
    DeletePinnedMemoryTool, DisplayImageTool, GetSkillDetailTool, GetTimeTool, ImageGenConfig,
    ImageGenTool, InstallClawHubSkillTool, ListFilesTool, ListInstalledSkillsTool,
    ListPinnedMemoriesTool, PinMemoryTool, ReadFileTool, SearchClawHubSkillsTool,
    SearchHistoryTool, SearchMemoryTool, EnableSkillTool, SetTitleTool, ShellTool, WebFetchTool,
    WriteFileTool,
};

/// 自动注入的相关历史记忆条数上限
const MEMORY_AUTO_INJECT_LIMIT: usize = 5;
/// 历史段不再按条数截断：保留所有当前对话消息。
/// 此常量仅用于 ContextPreview.recent_history_limit 字段，值 0 表示"无限制"。
const RECENT_HISTORY_WITH_MEMORY: usize = 0;
/// 历史段不再按字符截断：每条消息保留完整内容。
/// 此常量仅用于 ContextPreview.history_truncate_chars 字段，值 0 表示"无限制"。
/// 长会话的 token 预算由消息压缩系统（compress_message 命令）维护，
/// 而非在 prompt 拼装层硬截断。
const HISTORY_TRUNCATE_CHARS: usize = 0;
/// 自动注入的可用技能条数上限（RAG 检索 Top-K）
/// 仅注入 name + description 摘要，agent 通过 get_skill_detail / enable_skill 深入使用
const SKILL_AUTO_INJECT_LIMIT: usize = 3;
/// 跳过过短查询的技能检索阈值（与 memory 一致），避免单字符无意义检索
const SKILL_SEARCH_MIN_QUERY_LEN: usize = 2;

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
    /// 已安装技能检索索引（RAG 技能自动注入核心）
    /// None 时关闭技能自动注入；Some 时每轮 prompt 注入 `[可用技能]` 段
    /// 并注册 list_installed_skills / get_skill_detail / enable_skill 工具
    skill_index: Option<Arc<SkillIndex>>,
    /// 技能存储句柄，GetSkillDetailTool / EnableSkillTool 据此读写
    /// None 时 skill_index 相关工具不可用
    skill_store: Option<Arc<SkillStore>>,
    /// ClawHub 客户端句柄，SearchClawHubSkillsTool / InstallClawHubSkillTool 据此访问远程市场
    /// None 时 clawhub 相关工具不可用（agent 只能用已安装技能）
    clawhub_client: Option<Arc<ClawHubClient>>,
    /// 技能解压根目录，InstallClawHubSkillTool 据此落盘 ZIP 解压结果
    skills_dir: Option<PathBuf>,
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
    /// 会话存储句柄，SetTitleTool / EnableSkillTool 据此读写会话
    store: Arc<ConversationStore>,
    /// 消息压缩状态存储句柄。
    /// Some 时 `build_context_parts` 会加载当前会话的压缩状态并对历史段
    /// （`messages[..last_user_idx]`）应用 Keep/Hide/Replace 决策。
    /// 当前问题（最后一条用户消息）不压缩。None 时退化为不压缩。
    compression_store: Option<Arc<CompressionStore>>,
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
    /// `skill_index` / `skill_store` / `clawhub_client` / `skills_dir` 注入后
    /// 启用技能 RAG 自动注入与 5 个技能管理工具；任一为 None 则相应能力降级。
    /// `compression_store` 注入后启用消息压缩（build_context_parts 对历史段应用
    /// Keep/Hide/Replace 决策）；None 时退化为不压缩。
    #[allow(clippy::too_many_arguments)]
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
        skill_index: Option<Arc<SkillIndex>>,
        skill_store: Option<Arc<SkillStore>>,
        clawhub_client: Option<Arc<ClawHubClient>>,
        skills_dir: Option<PathBuf>,
        compression_store: Option<Arc<CompressionStore>>,
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
            skill_index,
            skill_store,
            clawhub_client,
            skills_dir,
            current_conversation_id,
            working_dir,
            image_gen_config,
            attachments_dir,
            store,
            compression_store,
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
    /// - `store` 共享会话存储句柄，SetTitleTool / EnableSkillTool 据此读写会话
    /// - `skill_index` / `skill_store` / `clawhub_client` / `skills_dir` 注入后
    ///   启用技能 RAG 自动注入与 5 个技能管理工具；任一为 None 则相应能力降级
    /// - `compression_store` 注入后启用消息压缩（build_context_parts 对历史段应用
    ///   Keep/Hide/Replace 决策）；None 时退化为不压缩
    #[allow(clippy::too_many_arguments)]
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
        skill_index: Option<Arc<SkillIndex>>,
        skill_store: Option<Arc<SkillStore>>,
        clawhub_client: Option<Arc<ClawHubClient>>,
        skills_dir: Option<PathBuf>,
        compression_store: Option<Arc<CompressionStore>>,
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
            skill_index,
            skill_store,
            clawhub_client,
            skills_dir,
            current_conversation_id,
            working_dir,
            image_gen_config,
            attachments_dir,
            store,
            compression_store,
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
            let write_file = match &cwd {
                Some(p) => WriteFileTool::with_cwd(p.clone()),
                None => WriteFileTool::new(),
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
            // 图片展示工具：让 LLM 把已有图片（本地路径或 URL）推送到聊天框。
            // 与 image_gen（生成新图）互补，复用 attachments_dir 落盘。
            let display_image = match &cwd {
                Some(p) => DisplayImageTool::with_cwd(p.clone(), self.attachments_dir.clone()),
                None => DisplayImageTool::new(self.attachments_dir.clone()),
            };
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
                .tool(write_file)
                .tool(list_files)
                .tool(shell)
                .tool(web_fetch)
                .tool(image_gen)
                .tool(display_image)
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

            // 技能管理工具：仅在 skill_index + skill_store 同时可用时注册
            // 让 LLM 自主列出 / 查询 / 启用本地已安装技能（替代旧 apply_skill 命令）
            if let (Some(idx), Some(store)) = (&self.skill_index, &self.skill_store) {
                let list_skills = ListInstalledSkillsTool::new(Arc::clone(idx));
                let get_skill = GetSkillDetailTool::new(Arc::clone(store));
                let enable_skill = EnableSkillTool::new(
                    Arc::clone(store),
                    Arc::clone(&self.store),
                    Arc::clone(&self.current_conversation_id),
                );
                b = b.tool(list_skills).tool(get_skill).tool(enable_skill);
            }

            // ClawHub 工具：仅在 clawhub_client 可用时注册
            // 让 LLM 在本地无匹配技能时主动从 ClawHub 搜索 + 安装
            // install_clawhub_skill 额外依赖 skill_store / skill_index / skills_dir，
            // 任一缺失则只暴露 search_clawhub_skills（agent 可推荐 slug 但不能直接安装）
            if let Some(client) = &self.clawhub_client {
                let search_clawhub = SearchClawHubSkillsTool::new(Arc::clone(client));
                b = b.tool(search_clawhub);
                if let (Some(store), Some(idx), Some(dir)) =
                    (&self.skill_store, &self.skill_index, &self.skills_dir)
                {
                    let install_clawhub = InstallClawHubSkillTool::new(
                        Arc::clone(client),
                        Arc::clone(store),
                        Arc::clone(idx),
                        dir.clone(),
                    );
                    b = b.tool(install_clawhub);
                }
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
        // 复用 build_context_parts 的拆分逻辑，避免与预览面板出现实现分叉
        let parts = match self.build_context_parts(messages).await {
            None => return "hello".to_string(),
            Some(p) => p,
        };

        // 若无永久记忆、无历史且无 RAG 记忆，直接返回当前问题（旧行为）
        if parts.pinned_section.is_empty()
            && parts.history_section.is_empty()
            && parts.memory_section.is_empty()
        {
            return parts.current_question;
        }

        parts.assemble_prompt()
    }

    /// 构建上下文注入预览：返回结构化的各段内容 + 拼装后的完整 prompt
    ///
    /// 与 `build_contextual_prompt` 共享 `build_context_parts` 拆分逻辑，
    /// 确保预览面板展示的内容与实际发给 LLM 的 prompt 完全一致。
    /// 不实际触发 LLM 调用，只读取已注入的永久记忆 / RAG 检索 / 当前对话历史。
    pub async fn build_context_preview(&self, messages: &[Message]) -> ContextPreview {
        let preamble = self.preamble.clone();
        let memory_enabled = self.memory.is_some();
        let skill_auto_inject_enabled = self.skill_index.is_some();

        match self.build_context_parts(messages).await {
            None => ContextPreview {
                preamble,
                pinned_section: String::new(),
                memory_section: String::new(),
                skill_section: String::new(),
                history_section: String::new(),
                current_question: String::new(),
                full_prompt: String::new(),
                pinned_count: 0,
                memory_hits_count: 0,
                skill_hits_count: 0,
                history_keep_count: 0,
                history_total_count: 0,
                memory_inject_limit: MEMORY_AUTO_INJECT_LIMIT,
                recent_history_limit: RECENT_HISTORY_WITH_MEMORY,
                history_truncate_chars: HISTORY_TRUNCATE_CHARS,
                memory_enabled,
                skill_auto_inject_enabled,
            },
            Some(parts) => {
                // 退化为纯当前问题（与 build_contextual_prompt 保持一致）
                let full_prompt = if parts.pinned_section.is_empty()
                    && parts.history_section.is_empty()
                    && parts.memory_section.is_empty()
                    && parts.skill_section.is_empty()
                {
                    parts.current_question.clone()
                } else {
                    parts.assemble_prompt()
                };

                ContextPreview {
                    preamble,
                    pinned_section: parts.pinned_section,
                    memory_section: parts.memory_section,
                    skill_section: parts.skill_section,
                    history_section: parts.history_section,
                    current_question: parts.current_question,
                    full_prompt,
                    pinned_count: parts.pinned_count,
                    memory_hits_count: parts.memory_hits_count,
                    skill_hits_count: parts.skill_hits_count,
                    history_keep_count: parts.history_keep_count,
                    history_total_count: messages.len(),
                    memory_inject_limit: MEMORY_AUTO_INJECT_LIMIT,
                    recent_history_limit: RECENT_HISTORY_WITH_MEMORY,
                    history_truncate_chars: HISTORY_TRUNCATE_CHARS,
                    memory_enabled,
                    skill_auto_inject_enabled,
                }
            }
        }
    }

    /// 拆分 `build_contextual_prompt` 的内部逻辑为可复用结构
    ///
    /// 把"获取永久记忆 / RAG 检索 / 当前对话历史全量格式化"三步拆开，
    /// 既给 `build_contextual_prompt` 用，也给 `build_context_preview` 用，
    /// 避免预览面板与实际 prompt 出现实现分叉。
    ///
    /// 历史段不再做条数 / 字符截断：保留所有消息完整内容。
    /// 长会话的 token 预算由消息压缩系统（compress_message）维护。
    ///
    /// 返回 `None` 表示 `messages` 为空（与原 `build_contextual_prompt` 的早退分支一致）。
    async fn build_context_parts(&self, messages: &[Message]) -> Option<ContextParts> {
        if messages.is_empty() {
            return None;
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
        // 永久记忆条目数：解析格式化后的字符串行数（含头部说明行），减 1 得条目数
        let pinned_count = if pinned_section.is_empty() {
            0
        } else {
            pinned_section.lines().count().saturating_sub(1)
        };

        // 1. 若启用记忆增强，检索跨会话相关历史
        let (memory_section, memory_hits_count) = if let Some(memory) = &self.memory {
            // 跳过过短查询（如单字符）避免无意义检索
            let query = current_msg.content.trim();
            if query.len() < 2 {
                (String::new(), 0)
            } else {
                let exclude = self.current_conversation_id.read().await.clone();
                let hits = memory
                    .search_hybrid(query, MEMORY_AUTO_INJECT_LIMIT, exclude.as_deref())
                    .await;
                let count = hits.len();
                if hits.is_empty() {
                    (String::new(), 0)
                } else {
                    (format_memory_section(&hits), count)
                }
            }
        } else {
            (String::new(), 0)
        };

        // 1.5. RAG 技能自动注入：检索与当前问题相关的 Top-K 已安装技能
        // 仅注入 name + description 摘要让 agent 知道"我能用什么"，
        // agent 通过 list_installed_skills / get_skill_detail / enable_skill 工具深入使用
        let (skill_section, skill_hits_count) = if let Some(skill_idx) = &self.skill_index {
            let query = current_msg.content.trim();
            if query.len() < SKILL_SEARCH_MIN_QUERY_LEN {
                (String::new(), 0)
            } else {
                let hits = skill_idx.search(query, SKILL_AUTO_INJECT_LIMIT).await;
                let count = hits.len();
                if hits.is_empty() {
                    (String::new(), 0)
                } else {
                    (format_skill_section(&hits), count)
                }
            }
        } else {
            (String::new(), 0)
        };

        // 2. 当前对话历史：全量注入（不截断条数，不截断单条字符）
        // 旧逻辑：启用 RAG 时只取最近 RECENT_HISTORY_WITH_MEMORY 条，单条截断到
        //         HISTORY_TRUNCATE_CHARS 字符。新逻辑：保留所有消息完整内容，
        //         让 LLM 拥有完整的当前对话上下文，避免重要细节被截断丢失。
        //         长会话的 token 预算由消息压缩系统（compress_message）维护，
        //         而非在 build_context_parts 这一层硬截断。
        //
        // 压缩：若注入了 compression_store，加载当前会话的压缩状态并对历史段
        // （messages[..last_user_idx]）应用 Keep/Hide/Replace 决策。
        // 当前问题（最后一条用户消息）不压缩。
        // 用 Cow 避免无压缩状态时的整段克隆（零成本退化）。
        let history_slice: &[Message] = &messages[..last_user_idx];
        let compressed_history: Cow<'_, [Message]> = match &self.compression_store {
            Some(store) => {
                // 读 current_conversation_id 后立即释放锁（临界区极短）
                let conv_id = self.current_conversation_id.read().await.clone();
                match conv_id {
                    Some(id) => match store.load(&id).await {
                        Ok(Some(state)) if !state.actions.is_empty() => {
                            Cow::Owned(apply_compression(history_slice, &state))
                        }
                        Ok(_) => Cow::Borrowed(history_slice),
                        Err(e) => {
                            tracing::warn!(error = %e, "加载压缩状态失败，使用未压缩历史");
                            Cow::Borrowed(history_slice)
                        }
                    },
                    None => Cow::Borrowed(history_slice),
                }
            }
            None => Cow::Borrowed(history_slice),
        };
        let history_msgs: &[Message] = compressed_history.as_ref();

        // 3. 格式化历史段（含 `[当前对话最近]` 头部，全量不截断）
        let history_section = if history_msgs.is_empty() {
            String::new()
        } else {
            // 预估容量：每条平均 128 字节；宁多勿少，避免多次扩容
            let mut s = String::with_capacity(history_msgs.len() * 128 + 32);
            s.push_str("[当前对话最近]\n");
            for m in history_msgs {
                let role_label = match m.role {
                    Role::User => "用户",
                    Role::Assistant => "助手",
                    Role::System => "系统",
                };
                s.push_str(role_label);
                s.push_str(": ");
                s.push_str(&m.content);
                s.push('\n');
            }
            s
        };

        Some(ContextParts {
            pinned_section,
            pinned_count,
            memory_section,
            memory_hits_count,
            skill_section,
            skill_hits_count,
            history_section,
            history_keep_count: history_msgs.len(),
            current_question: current_msg.content.clone(),
        })
    }
}

/// `build_context_parts` 的中间产物：把各段拼装前的内容拆开保存
///
/// 用于 `build_contextual_prompt` 与 `build_context_preview` 共享拆分逻辑。
struct ContextParts {
    pinned_section: String,
    memory_section: String,
    skill_section: String,
    history_section: String,
    current_question: String,
    pinned_count: usize,
    memory_hits_count: usize,
    skill_hits_count: usize,
    history_keep_count: usize,
}

impl ContextParts {
    /// 把各段按
    /// `[永久记忆] → [相关历史记忆] → [可用技能] → [当前对话最近] → [当前问题]`
    /// 顺序拼装。
    ///
    /// `[可用技能]` 段位置选择在历史记忆之后、当前对话之前：
    /// - 不放最前：避免覆盖永久记忆（用户主动要求的高优先级）
    /// - 不放最后：避免与当前问题抢夺注意力，让 agent 先看到"我能用什么"再读问题
    fn assemble_prompt(&self) -> String {
        let mut prompt = String::with_capacity(
            self.pinned_section.len()
                + self.memory_section.len()
                + self.skill_section.len()
                + self.history_section.len()
                + self.current_question.len()
                + 96,
        );

        if !self.pinned_section.is_empty() {
            prompt.push_str(&self.pinned_section);
            prompt.push('\n');
        }
        if !self.memory_section.is_empty() {
            prompt.push_str(&self.memory_section);
            prompt.push('\n');
        }
        if !self.skill_section.is_empty() {
            prompt.push_str(&self.skill_section);
            prompt.push('\n');
        }
        if !self.history_section.is_empty() {
            prompt.push_str(&self.history_section);
            prompt.push('\n');
        }
        prompt.push_str("[当前问题]\n用户: ");
        prompt.push_str(&self.current_question);
        prompt
    }
}

/// 格式化技能自动注入的 `[可用技能]` 段落
///
/// 输出格式：
/// ```text
/// [可用技能]（已安装技能中与当前问题相关，调用 enable_skill(id) 启用）
/// 1. [weather] Weather — Get current weather forecast
/// 2. [translator] Translator — Translate text between languages
/// ```
fn format_skill_section(hits: &[effisuite_core::SkillHit]) -> String {
    let mut s = String::with_capacity(hits.len() * 96 + 64);
    s.push_str("[可用技能]（已安装技能中与当前问题相关，调用 enable_skill(id) 启用；\
                调用 get_skill_detail(id) 查看完整说明；\
                调用 list_installed_skills 查看全部；\
                调用 search_clawhub_skills / install_clawhub_skill 从 ClawHub 找新技能）\n");
    for (i, hit) in hits.iter().enumerate() {
        let tag = if hit.builtin { "[内置]" } else { "" };
        s.push_str(&format!(
            "{}. [{}] {}{} — {}\n",
            i + 1,
            short_skill_id(&hit.id),
            tag,
            hit.name,
            hit.description
        ));
    }
    s
}

/// 截断技能 id 用于显示（取前 12 字符，UTF-8 边界安全）。
/// 比 conversation id 略长，因为技能 id 常是 slug 风格（如 "agent-reach"），
/// 前 12 字符更易辨识
#[inline]
fn short_skill_id(id: &str) -> &str {
    if id.len() <= 12 {
        id
    } else {
        &id[..id.ceil_char_boundary(12)]
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
                        yield Ok(AgentStreamItem::Usage {
                            input_tokens: call.usage.input_tokens,
                            output_tokens: call.usage.output_tokens,
                            total_tokens: call.usage.total_tokens,
                            reasoning_tokens: call.usage.reasoning_tokens,
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

// =========================================================
// 消息压缩 agent
// =========================================================

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

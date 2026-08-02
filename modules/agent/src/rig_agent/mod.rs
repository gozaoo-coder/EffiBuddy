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
//!   `GetTimeTool`、`ReadFileTool`、`WriteFileTool`、`EditFileTool`、`EditFileRegexTool`、
//!   `EditReviseTool`、`EditUndoTool`、`SearchFileTool`、
//!   `ListFilesTool`、`ShellTool`、`WebFetchTool`、
//!   `ImageGenTool`、`DisplayImageTool`，
//!   LLM 可主动调用以检索历史、跨会话记忆、获取时间、读写本地文件、
//!   按行号编辑文件、正则替换文件内容、查看/修订/撤回编辑操作、工作区全文搜索、
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

//! 模块拆分说明：
//! - [`builder`]：`RigAgent` 的构造（`from_env` / `from_key`）与 `with_*` 配置链
//! - [`tools`]：`build_agent` 内部的工具注册逻辑
//! - [`context`]：上下文 prompt 构建（永久记忆 / RAG 记忆 / 技能注入 / 历史）
//! - [`chat_agent_impl`]：`ChatAgent` trait 实现（非流式 + 流式 + 上下文预览）
//! - [`compression`]：消息压缩 agent（独立 client + 流式 / 非流式调用）
//! - [`auto_classify`]：自动归类 agent（一次性调用，生成标题 + 文件夹归类建议）

mod auto_classify;
mod builder;
mod chat_agent_impl;
mod compression;
mod context;
mod tools;
pub use auto_classify::{
    AUTO_CLASSIFY_PREAMBLE, AutoClassifyResult, build_auto_classify_prompt,
    call_auto_classify_agent, parse_auto_classify_response,
};
pub use compression::{
    COMPRESSION_PREAMBLE, CompressionStreamItem, call_compression_agent,
    call_compression_agent_stream,
};

use std::path::PathBuf;
use std::sync::Arc;

use effisuite_core::{
    ClawHubClient, CompressionStore, ConversationStore, MemoryIndex, Message, PinnedMemoryStore,
    PluginStore, RemoteTaskDispatcher, ScheduledTaskStore, SkillIndex, SkillStore,
};
use rig_core::providers::openai;
use tokio::sync::RwLock;

use crate::tools::{
    ImageGenConfig, ModelManagerHandle, SubAgentManager, TodoItem, VideoGenConfig, WebSearchConfig,
};

/// 自动注入的相关历史记忆条数上限
pub(super) const MEMORY_AUTO_INJECT_LIMIT: usize = 5;
/// 历史段不再按条数截断：保留所有当前对话消息。
/// 此常量仅用于 ContextPreview.recent_history_limit 字段，值 0 表示"无限制"。
pub(super) const RECENT_HISTORY_WITH_MEMORY: usize = 0;
/// 历史段不再按字符截断：每条消息保留完整内容。
/// 此常量仅用于 ContextPreview.history_truncate_chars 字段，值 0 表示"无限制"。
/// 长会话的 token 预算由消息压缩系统（compress_message 命令）维护，
/// 而非在 prompt 拼装层硬截断。
pub(super) const HISTORY_TRUNCATE_CHARS: usize = 0;
/// 助手消息的思考（reasoning）注入上下文的单条长度上限（字符）。
/// 推理模型的 thinking 可能非常长，全量注入会快速耗尽上下文窗口；
/// 这里对单条 reasoning 做截断兜底，长会话的进一步预算仍由压缩系统维护。
pub(super) const REASONING_IN_CONTEXT_CHARS: usize = 2000;
/// 自动注入的可用技能条数上限（RAG 检索 Top-K）
/// 仅注入 name + description 摘要，agent 通过 get_skill_detail / enable_skill 深入使用
pub(super) const SKILL_AUTO_INJECT_LIMIT: usize = 3;
/// 跳过过短查询的技能检索阈值（与 memory 一致），避免单字符无意义检索
pub(super) const SKILL_SEARCH_MIN_QUERY_LEN: usize = 2;

/// 子 agent 默认排除的工具：set_title（避免改名主会话）、
/// display_image（避免图片推送到主会话）、image_gen（子 agent 生成的图片
/// 经 attachment 事件在子 agent 卡片内展示，不走主会话附件通道）
pub const SUB_AGENT_DEFAULT_EXCLUDED: &[&str] = &["set_title", "display_image", "image_gen"];

/// 通过 [rig](https://crates.io/crates/rig-core) 调用 OpenAI 兼容接口的 [`crate::ChatAgent`] 实现。
///
/// 字段以 `pub(super)` 暴露给 `rig_agent` 子模块的 `impl` 块使用，
/// 对 crate 外部完全不可见。
pub struct RigAgent {
    /// 统一用 CompletionsClient（Chat Completions API），兼容所有 OpenAI 兼容 provider
    pub(super) client: openai::CompletionsClient,
    pub(super) model_name: String,
    pub(super) preamble: String,
    /// 共享历史快照，工具读取此数据做 RAG 检索
    pub(super) history: Arc<RwLock<Vec<Message>>>,
    /// 是否启用工具调用
    pub(super) enable_tools: bool,
    /// 跨会话历史记忆索引（RAG 记忆增强核心）
    /// None 时退化为旧行为（包含全部当前对话历史）
    pub(super) memory: Option<Arc<MemoryIndex>>,
    /// 永久记忆存储（用户主动要求"记住"的内容）
    /// None 时关闭永久记忆能力；Some 时每轮 prompt 都注入 `[永久记忆]` 段
    pub(super) pinned_memory: Option<Arc<PinnedMemoryStore>>,
    /// 已安装技能检索索引（RAG 技能自动注入核心）
    /// None 时关闭技能自动注入；Some 时每轮 prompt 注入 `[可用技能]` 段
    /// 并注册 list_installed_skills / get_skill_detail / enable_skill 工具
    pub(super) skill_index: Option<Arc<SkillIndex>>,
    /// 技能存储句柄，GetSkillDetailTool / EnableSkillTool 据此读写
    /// None 时 skill_index 相关工具不可用
    /// `SkillStore` 已是 `Clone`（内部 Arc），无需外层 `Arc` 包装
    pub(super) skill_store: Option<SkillStore>,
    /// ClawHub 客户端句柄，SearchClawHubSkillsTool / InstallClawHubSkillTool 据此访问远程市场
    /// None 时 clawhub 相关工具不可用（agent 只能用已安装技能）
    /// `ClawHubClient` 已是 `Clone`（内部 Arc），无需外层 `Arc` 包装
    pub(super) clawhub_client: Option<ClawHubClient>,
    /// 技能解压根目录，InstallClawHubSkillTool 据此落盘 ZIP 解压结果
    pub(super) skills_dir: Option<PathBuf>,
    /// 已安装插件存储句柄，UninstallPluginTool 据此删除插件
    /// None 时 uninstall_plugin 工具不可用
    /// `PluginStore` 已是 `Clone`（内部 Arc），无需外层 `Arc` 包装
    pub(super) plugin_store: Option<PluginStore>,
    /// 当前会话 id 句柄，由 Tauri 命令层在每次 send_message 前更新；
    /// search_memory 工具与自动注入都据此排除当前会话
    pub(super) current_conversation_id: Arc<RwLock<Option<String>>>,
    /// 当前工作区路径句柄，由 Tauri 命令层在每次 send_message 前更新。
    /// read_file / list_files / shell 据此解析相对路径与设置子进程 cwd。
    /// 优先级：会话级 working_dir > 技能级 working_dir > 进程默认 cwd。
    pub(super) working_dir: Arc<RwLock<Option<PathBuf>>>,
    /// 编辑历史共享句柄：edit_file / edit_file_regex 写入，edit_revise / edit_undo 读取。
    /// 每次 build_agent 注入到 4 个 edit 工具，使它们共享同一份 op_id 历史。
    /// None 时 edit 工具退化为无历史模式（不返回 op_id，无法撤回/修订）。
    pub(super) edit_history: Option<crate::tools::EditHistoryHandle>,
    /// 图像生成模型配置句柄：set_active_model 切到 kind=ImageGen 的模型时更新。
    /// build_agent 注入到 ImageGenTool；为 None 时 image_gen 工具返回错误。
    pub(super) image_gen_config: Arc<RwLock<Option<ImageGenConfig>>>,
    /// 附件保存目录（绝对路径），ImageGenTool 把生成图片落盘到此目录。
    pub(super) attachments_dir: PathBuf,
    /// 会话存储句柄，SetTitleTool / EnableSkillTool 据此读写会话
    pub(super) store: Arc<ConversationStore>,
    /// 消息压缩状态存储句柄。
    /// Some 时 `build_context_parts` 会加载当前会话的压缩状态并对历史段
    /// （`messages[..last_user_idx]`）应用 Keep/Hide/Replace 决策。
    /// 当前问题（最后一条用户消息）不压缩。None 时退化为不压缩。
    /// `CompressionStore` 已是 `Clone`（内部 Arc），无需外层 `Arc` 包装
    pub(super) compression_store: Option<CompressionStore>,
    /// 模型管理句柄：manage_model / call_model 工具据此读写模型列表。
    /// None 时不注册这两个工具。
    pub(super) model_manager: Option<Arc<ModelManagerHandle>>,
    /// 子 agent 管理器：sub_agent 工具据此召唤子 agent（可嵌套）。
    /// None 时不注册 sub_agent 工具。
    pub(super) sub_agents: Option<Arc<SubAgentManager>>,
    /// 工具白名单：None = 注册全部工具；Some = 仅注册列表中的工具。
    pub(super) tool_allowlist: Option<Vec<String>>,
    /// 排除的工具名列表（优先级高于白名单）：子 agent 默认排除 set_title 等。
    pub(super) exclude_tools: Vec<String>,
    /// 事件总线句柄，AskUserTool / NotifyUserTool / OpenPreviewTool 据此前端通信。
    /// None 时这三个交互工具不可用。
    pub(super) event_bus: Option<Arc<effisuite_core::EventBus>>,
    /// 定时任务存储句柄，ScheduleTool 据此管理 cron 任务。
    /// None 时 schedule 工具不可用。
    pub(super) scheduled_task_store: Option<Arc<ScheduledTaskStore>>,
    /// 网络搜索配置句柄，WebSearchTool 据此调用搜索 API。
    /// None 时 web_search 工具不可用。
    pub(super) web_search_config: Arc<RwLock<Option<WebSearchConfig>>>,
    /// 视频生成配置句柄，GenerateVideoTool 据此调用视频生成 API。
    /// None 时 generate_video 工具不可用。
    pub(super) video_gen_config: Arc<RwLock<Option<VideoGenConfig>>>,
    /// 待办列表共享状态，TodoWriteTool 据此管理任务列表。
    /// None 时 todo_write 工具不可用。
    pub(super) todo_state: Option<Arc<RwLock<Vec<TodoItem>>>>,
    /// 每会话 TodoTree 存储：todo_write 工具写入后持久化到当前会话，
    /// build_context_parts 每轮注入 `[当前任务清单]` 段，任务详情常驻本会话上下文。
    pub(super) todo_store: Option<crate::todo_store::TodoStore>,
    /// ASR 语音转写服务句柄，ASR 工具集据此转写/检索/列出/获取记录。
    /// None 时不注册 ASR 工具（transcribe_audio / search_asr_records 等）。
    pub(super) asr_service: Option<Arc<crate::asr::AsrService>>,
    /// 远端任务派发器句柄（P2P 镜像模式跨设备协作）。
    /// None 时不注册 dispatch_remote_task 工具；Some 时 LLM 可列出在线设备并派发任务。
    /// 用 trait object 避免 agent crate 依赖 effisuite-p2p（依赖倒置）。
    pub(super) remote_task_dispatcher: Option<Arc<dyn RemoteTaskDispatcher>>,
    /// 后台命令会话管理器：shell_session_start / send / read / list / kill 工具据此
    /// 启用并交互常驻 cmd/sh 会话（静默后台运行，前端底栏实时展示）。
    /// None 时不注册这 5 个工具。
    pub(super) shell_sessions: Option<Arc<crate::shell_session::ShellSessionManager>>,
    /// 运行时 agent 公共会话交流池存储：跨会话协作基础设施。
    /// None 时不注册 pool_* 工具、不注入 `[Agent 交流池]` 上下文段。
    pub(super) agent_pool: Option<crate::agent_pool::AgentPoolStore>,
    /// 交流池身份：主 agent 为 None（agent_id 按 `conv:<conversation_id>` 推导）；
    /// 子 agent 为 Some(session_id)（agent_id 按 `sa:<session_id>` 推导）。
    pub(super) pool_sub_agent_id: Option<String>,
    /// 子 agent 的交流池显示名（主 agent 为 None，取会话标题）。
    pub(super) pool_sub_agent_name: Option<String>,
}

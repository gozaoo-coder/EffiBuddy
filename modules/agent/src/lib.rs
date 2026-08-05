//! EffiSuite agent 模块
//!
//! 基于 [rig](https://crates.io/crates/rig-core) 提供 LLM 对话能力。
//! 通过 `ChatAgent` trait 抽象具体后端，业务层依赖 trait 而非具体实现，
//! 便于在 mock / OpenAI / Ollama 等后端间无成本切换（零成本抽象 + 类型驱动）。
//!
//! 当前版本提供：
//! - [`MockAgent`]：纯本地回显，无网络依赖，用于离线开发与单测，支持流式
//! - [`RigAgent`]：通过 rig 调用 OpenAI 兼容接口，支持流式输出与工具调用
//! - [`tools`]：RAG 检索工具集（search_history / search_memory / get_time 等）
//! - [`embedding`]：OpenAI 兼容嵌入向量 provider，为 `MemoryIndex` 提供向量路

pub mod agent;
pub mod agent_pool;
pub mod asr;
pub mod cancel;
pub mod embedding;
pub mod mock;
pub mod rig_agent;
pub mod shell_env;
pub mod shell_session;
pub mod tools;
pub mod todo_store;
pub use agent::{AgentStreamItem, ChatAgent, ContextPreview};
pub use cancel::AgentCancelRegistry;
pub use agent_pool::{
    AgentPoolStore, AtMessage, AtStatus, PoolEntry, PoolKind, PoolStatus, format_lookup_result,
    format_pool_section,
};
pub use asr::{
    AsrError, AsrProvider, AsrService, AudioStreamConfig, FinishResult, QwenProvider,
    SessionInfo, SessionRegistry, SessionState, TranscribeResult, VolcEngineProvider,
    generate_summary,
};
pub use embedding::{DEFAULT_EMBEDDING_MODEL, OpenAIEmbeddingProvider};
pub use mock::MockAgent;
pub use rig_agent::{
    AUTO_CLASSIFY_PREAMBLE, AutoClassifyResult, COMPRESSION_PREAMBLE, CompressionStreamItem,
    PendingUserMessages, ReasoningConfig, ReasoningEffort, RigAgent, SUB_AGENT_DEFAULT_EXCLUDED, build_auto_classify_prompt,
    call_auto_classify_agent, call_compression_agent, call_compression_agent_stream,
    parse_auto_classify_response,
};
pub use todo_store::{TodoNode, TodoStore, build_todo_tree, format_todo_tree, todo_tree_stats};
pub use tools::{
    AsrRecordDetail, AsrRecordSummary, AsrTool, AskUserTool, CallModelTool, DeleteFileTool,
    DispatchAction, DispatchRemoteTaskArgs, DispatchRemoteTaskError, DispatchRemoteTaskTool,
    EditFileRegexTool, EditFileTool, EditHistoryHandle, EditReviseTool, EditUndoTool,
    GenerateVideoTool, GetAsrRecordTool, GetTimeTool, GlobTool, GrepTool, ImageGenConfig,
    ImageGenTool, ListAsrTool, ManageModelTool, ModelManagerHandle, NotifyUserTool,
    OpenPreviewTool, PoolAtArgs, PoolAtError, PoolAtTool, PoolCtx, PoolLookupArgs,
    PoolLookupError, PoolLookupTool, PoolReplyArgs, PoolReplyError, PoolReplyTool, PoolReportArgs,
    PoolReportError, PoolReportTool, ReadFileTool, ScheduleAction, ScheduleArgs, ScheduleError,
    ScheduleTool, SearchAsrTool, SearchCodebaseTool, SearchFileTool, SearchHistoryTool,
    SearchMemoryTool, SearchResult, SetTitleTool, SubAgentEvent, SubAgentEventKind, SubAgentKit,
    SubAgentManager, SubAgentTool, TodoItem, TodoPriority, TodoStatus, TodoWriteTool,
    UninstallPluginTool, UninstallSkillTool, VideoGenConfig, WebSearchConfig, WebSearchTool,
    WriteFileTool, new_shared_history,
};
pub use shell_session::{
    ShellSessionEvent, ShellSessionEventKind, ShellSessionInfo, ShellSessionKillTool,
    ShellSessionListTool, ShellSessionManager, ShellSessionReadTool, ShellSessionSendTool,
    ShellSessionStartTool, ShellSessionWaitTool,
};

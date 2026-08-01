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
pub mod embedding;
pub mod mock;
pub mod rig_agent;
pub mod tools;

pub use agent::{AgentStreamItem, ChatAgent, ContextPreview};
pub use embedding::{DEFAULT_EMBEDDING_MODEL, OpenAIEmbeddingProvider};
pub use mock::MockAgent;
pub use rig_agent::{
    AUTO_CLASSIFY_PREAMBLE, AutoClassifyResult, COMPRESSION_PREAMBLE, CompressionStreamItem,
    RigAgent, SUB_AGENT_DEFAULT_EXCLUDED, build_auto_classify_prompt, call_auto_classify_agent,
    call_compression_agent, call_compression_agent_stream, parse_auto_classify_response,
};
pub use tools::{
    AskUserTool, CallModelTool, DeleteFileTool, EditFileTool, GenerateVideoTool, GetTimeTool,
    GlobTool, GrepTool, ImageGenConfig, ImageGenTool, ManageModelTool, ModelManagerHandle,
    NotifyUserTool, OpenPreviewTool, ReadFileTool, ScheduleAction, ScheduleArgs, ScheduleError,
    ScheduleTool, SearchCodebaseTool, SearchFileTool, SearchHistoryTool, SearchMemoryTool,
    SearchResult, SetTitleTool, SubAgentEvent, SubAgentEventKind, SubAgentKit, SubAgentManager,
    SubAgentTool, TodoItem, TodoPriority, TodoStatus, TodoWriteTool, UninstallPluginTool,
    UninstallSkillTool, VideoGenConfig, WebSearchConfig, WebSearchTool, WriteFileTool,
};

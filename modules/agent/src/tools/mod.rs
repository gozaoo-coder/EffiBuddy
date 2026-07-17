//! EffiSuite agent 自定义工具集
//!
//! 这些工具通过 rig 的 `Tool` trait 注册到 Agent，让 LLM 能调用它们：
//! - [`SearchHistoryTool`]：在当前 conversation 历史中按关键词检索相关消息
//!   （保留用于会话内回溯；跨会话检索请用 `SearchMemoryTool`）
//! - [`SearchMemoryTool`]：跨所有会话检索历史记忆（BM25 / 向量 / 混合），
//!   是 RAG 记忆增强的核心入口，由 LLM 主动调用
//! - [`GetTimeTool`]：获取当前时间，便于 LLM 回答时间相关问题
//! - [`ReadFileTool`]：读取本地文件内容
//! - [`ListFilesTool`]：列出目录内容
//! - [`ShellTool`]：执行本地 shell 命令（集成 agent-reach / browser-act）
//! - [`WebFetchTool`]：抓取网页内容
//!
//! RAG 检索：`SearchMemoryTool` 通过 `MemoryIndex` 提供 BM25 / 向量 / 混合
//! 三种检索模式，自动排除当前会话避免与已注入上下文重复。

pub mod get_time;
pub mod list_files;
pub mod read_file;
pub mod search_history;
pub mod search_memory;
pub mod shell;
pub mod web_fetch;

pub use get_time::GetTimeTool;
pub use list_files::ListFilesTool;
pub use read_file::ReadFileTool;
pub use search_history::SearchHistoryTool;
pub use search_memory::SearchMemoryTool;
pub use shell::ShellTool;
pub use web_fetch::WebFetchTool;

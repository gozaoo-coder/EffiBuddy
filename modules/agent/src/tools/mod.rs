//! EffiSuite agent 自定义工具集
//!
//! 这些工具通过 rig 的 `Tool` trait 注册到 Agent，让 LLM 能调用它们：
//! - [`SearchHistoryTool`]：在当前 conversation 历史中按关键词检索相关消息
//! - [`GetTimeTool`]：获取当前时间，便于 LLM 回答时间相关问题
//! - [`ReadFileTool`]：读取本地文件内容
//! - [`ListFilesTool`]：列出目录内容
//! - [`ShellTool`]：执行本地 shell 命令（集成 agent-reach / browser-act）
//! - [`WebFetchTool`]：抓取网页内容
//!
//! RAG 索引式调用：所有工具接收查询字符串，内部使用简单词频匹配
//! （避免引入向量数据库依赖），返回相关消息摘要给 LLM 作为上下文。

pub mod get_time;
pub mod list_files;
pub mod read_file;
pub mod search_history;
pub mod shell;
pub mod web_fetch;

pub use get_time::GetTimeTool;
pub use list_files::ListFilesTool;
pub use read_file::ReadFileTool;
pub use search_history::SearchHistoryTool;
pub use shell::ShellTool;
pub use web_fetch::WebFetchTool;

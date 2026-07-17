//! EffiSuite agent 自定义工具集
//!
//! 这些工具通过 rig 的 `Tool` trait 注册到 Agent，让 LLM 能调用它们：
//! - [`SearchHistoryTool`]：在当前 conversation 历史中按关键词检索相关消息
//! - [`GetTimeTool`]：获取当前时间，便于 LLM 回答时间相关问题
//!
//! RAG 索引式调用：所有工具接收查询字符串，内部使用简单词频匹配
//! （避免引入向量数据库依赖），返回相关消息摘要给 LLM 作为上下文。

pub mod get_time;
pub mod search_history;

pub use get_time::GetTimeTool;
pub use search_history::SearchHistoryTool;

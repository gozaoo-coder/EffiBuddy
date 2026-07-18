//! EffiSuite agent 自定义工具集
//!
//! 这些工具通过 rig 的 `Tool` trait 注册到 Agent，让 LLM 能调用它们：
//! - [`SearchHistoryTool`]：在当前 conversation 历史中按关键词检索相关消息
//!   （保留用于会话内回溯；跨会话检索请用 `SearchMemoryTool`）
//! - [`SearchMemoryTool`]：跨所有会话检索历史记忆（BM25 / 向量 / 混合），
//!   是 RAG 记忆增强的核心入口，由 LLM 主动调用
//! - [`PinMemoryTool`] / [`ListPinnedMemoriesTool`] / [`DeletePinnedMemoryTool`]：
//!   永久记忆管理。用户明确要求"请记住..."时，LLM 调用 `pin_memory` 把内容
//!   永久注入到每轮 prompt 的 `[永久记忆]` 段（不依赖检索相关性）
//! - [`GetTimeTool`]：获取当前时间，便于 LLM 回答时间相关问题
//! - [`ReadFileTool`]：读取本地文件内容（支持工作区相对路径）
//! - [`ListFilesTool`]：列出目录内容（支持工作区相对路径）
//! - [`ShellTool`]：执行本地 shell 命令（集成 agent-reach / browser-act，支持工作区 cwd）
//! - [`WebFetchTool`]：抓取网页内容
//! - [`ListInstalledSkillsTool`] / [`GetSkillDetailTool`] / [`EnableSkillTool`] /
//!   [`SearchClawHubSkillsTool`] / [`InstallClawHubSkillTool`]：技能管理工具集。
//!   让 agent 自主发现 / 启用 / 搜索 / 安装技能，替代旧 apply_skill 手动应用命令
//!
//! RAG 检索：`SearchMemoryTool` 通过 `MemoryIndex` 提供 BM25 / 向量 / 混合
//! 三种检索模式，自动排除当前会话避免与已注入上下文重复。

pub mod get_time;
pub mod image_gen;
pub mod list_files;
pub mod pin_memory;
pub mod read_file;
pub mod search_history;
pub mod search_memory;
pub mod set_title;
pub mod shell;
pub mod skill_tools;
pub mod web_fetch;

pub use get_time::GetTimeTool;
pub use image_gen::{ImageGenConfig, ImageGenTool};
pub use list_files::ListFilesTool;
pub use pin_memory::{
    DeletePinnedMemoryTool, ListPinnedMemoriesTool, PinMemoryTool,
};
pub use read_file::ReadFileTool;
pub use search_history::SearchHistoryTool;
pub use search_memory::SearchMemoryTool;
pub use set_title::SetTitleTool;
pub use shell::ShellTool;
pub use skill_tools::{
    GetSkillDetailTool, InstallClawHubSkillTool, ListInstalledSkillsTool,
    SearchClawHubSkillsTool, EnableSkillTool,
};
pub use web_fetch::WebFetchTool;

use std::path::{Path, PathBuf};

/// 解析路径：绝对路径或 cwd 为 None 时按原样返回；相对路径 join 到 cwd。
///
/// 软约束：不做沙箱校验，信任本地 agent 环境。
/// 仅在工具层把相对路径锚定到工作区，避免误访问进程 cwd。
#[inline]
pub fn resolve_path(path: &str, cwd: Option<&Path>) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else if let Some(base) = cwd {
        base.join(p)
    } else {
        p.to_path_buf()
    }
}

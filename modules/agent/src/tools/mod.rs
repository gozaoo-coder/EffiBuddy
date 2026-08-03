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
//! - [`ReadFileTool`]：读取本地文件内容（支持工作区相对路径），**逐行带行号**，
//!   支持 start_line/end_line 行范围精读
//! - [`WriteFileTool`]：写入本地文件（XML/CDATA 包裹避免转义，支持工作区相对路径）
//! - [`EditFileTool`]：按行号精确编辑本地文件：替换指定行 / 指定行前插入 /
//!   无行号时追加到文件末尾新行（与 read_file/search_file 的行号配合）。
//!   启用 history 后返回 op_id，支持行数校准警告
//! - [`EditFileRegexTool`]：正则表达式匹配替换（首处 / 全部），与 edit_file 互补。
//!   支持 $1 / ${name} 捕获组引用、多行模式、大小写敏感选项
//! - [`EditReviseTool`]：查看 / 修订历史编辑操作（view / list / patch）。
//!   patch 用新 text 重做指定 op_id（先撤回再重新执行）
//! - [`EditUndoTool`]：撤回指定 op_id 的编辑操作（恢复文件到操作前状态）
//! - [`SearchFileTool`]：工作区**全文搜索**：关键词数组 + 递归遍历全部文本文件，
//!   返回命中行（path + 行号 + 行内容），行号可直接用于 edit_file
//! - [`GrepTool`]：工作区**正则搜索**（grep/ripgrep 风格）：用 `regex` 语法搜索
//!   文件内容，支持大小写敏感/多行模式、上下文行、glob 文件名过滤、
//!   content / files_with_matches / count 三种输出模式
//! - [`GlobTool`]：按文件名 glob 模式（`*` / `**` / `?` / `[abc]` / `{a,b}`）
//!   递归匹配文件，返回路径列表（按修改时间降序），跳过生成目录
//! - [`ManageModelTool`]：agent 自主管理模型列表（list/save/delete/activate）
//! - [`CallModelTool`]：一次性调用任意已保存模型（无工具单轮）
//! - [`SubAgentTool`]：召唤子 agent 多轮对话（独立会话/可嵌套/事件实时推送前端）
//! - [`DeleteFileTool`]：删除本地文件或目录（支持工作区相对路径）
//! - [`ListFilesTool`]：列出目录内容（支持工作区相对路径）
//! - [`ShellTool`]：执行本地 shell 命令（集成 agent-reach / browser-act，支持工作区 cwd）
//! - [`WebFetchTool`]：抓取网页内容
//! - [`ListInstalledSkillsTool`] / [`GetSkillDetailTool`] / [`EnableSkillTool`] /
//!   [`SearchClawHubSkillsTool`] / [`InstallClawHubSkillTool`]：技能管理工具集。
//!   让 agent 自主发现 / 启用 / 搜索 / 安装技能，替代旧 apply_skill 手动应用命令
//!
//! RAG 检索：`SearchMemoryTool` 通过 `MemoryIndex` 提供 BM25 / 向量 / 混合
//! 三种检索模式，自动排除当前会话避免与已注入上下文重复。

pub mod agent_pool;
pub mod ask_user;
pub mod asr_tools;
pub mod call_model;
pub mod delete_file;
pub mod dispatch_remote_task;
pub mod display_image;
pub mod edit_file;
pub mod generate_video;
pub mod get_time;
pub mod glob_tool;
pub mod grep_tool;
pub mod image_gen;
pub mod list_files;
pub mod model_manager;
pub mod notify_user;
pub mod open_preview;
pub mod pin_memory;
pub mod plugin_tools;
pub mod read_file;
pub mod schedule_tool;
pub mod search_codebase;
pub mod search_file;
pub mod search_history;
pub mod search_memory;
pub mod set_title;
pub mod shell;
pub mod skill_tools;
pub mod sub_agent;
pub mod text_utils;
pub mod todo_write;
pub mod web_fetch;
pub mod web_search;
pub mod write_file;
pub use agent_pool::{
    PoolAtArgs, PoolAtError, PoolAtTool, PoolCtx, PoolLookupArgs, PoolLookupError, PoolLookupTool,
    PoolReplyArgs, PoolReplyError, PoolReplyTool, PoolReportArgs, PoolReportError, PoolReportTool,
};
pub use ask_user::{AskUserArgs, AskUserError, AskUserTool, Question, QuestionOption};
pub use asr_tools::{
    AsrRecordDetail, AsrRecordSummary, AsrTool, GetAsrRecordArgs, GetAsrRecordError,
    GetAsrRecordTool, ListAsrArgs, ListAsrError, ListAsrTool, SearchAsrArgs, SearchAsrError,
    SearchAsrTool, TranscribeAudioArgs, TranscribeAudioError, TranscribeAudioOutput,
};
pub use call_model::CallModelTool;
pub use delete_file::{DeleteFileArgs, DeleteFileError, DeleteFileTool};
pub use dispatch_remote_task::{
    DispatchAction, DispatchRemoteTaskArgs, DispatchRemoteTaskError, DispatchRemoteTaskTool,
};
pub use display_image::{DisplayImageOutput, DisplayImageTool};
pub use edit_file::{
    EditFileArgs, EditFileRegexArgs, EditFileRegexTool, EditFileTool, EditHistoryHandle,
    EditOp, EditReviseArgs, EditReviseTool, EditUndoArgs, EditUndoTool, new_shared_history,
};
pub use generate_video::{
    GenerateVideoError, GenerateVideoOutput, GenerateVideoTool, VideoGenConfig,
};
pub use get_time::GetTimeTool;
pub use glob_tool::{GlobArgs, GlobError, GlobTool};
pub use grep_tool::{GrepArgs, GrepError, GrepTool};
pub use image_gen::{ImageGenConfig, ImageGenTool};
pub use list_files::ListFilesTool;
pub use model_manager::{ManageModelTool, ModelManagerHandle};
pub use notify_user::{NotifyUserArgs, NotifyUserError, NotifyUserTool};
pub use open_preview::{OpenPreviewArgs, OpenPreviewError, OpenPreviewTool};
pub use pin_memory::{
    DeletePinnedMemoryTool, ListPinnedMemoriesTool, PinMemoryTool,
};
pub use plugin_tools::{
    UninstallPluginArgs, UninstallPluginError, UninstallPluginTool,
};
pub use read_file::ReadFileTool;
pub use schedule_tool::{ScheduleAction, ScheduleArgs, ScheduleError, ScheduleTool};
pub use search_codebase::{SearchCodebaseArgs, SearchCodebaseError, SearchCodebaseTool};
pub use search_file::SearchFileTool;
pub use search_history::SearchHistoryTool;
pub use search_memory::SearchMemoryTool;
pub use set_title::SetTitleTool;
pub use shell::ShellTool;
pub use crate::shell_session::{
    ShellSessionKillTool, ShellSessionListTool, ShellSessionReadTool, ShellSessionSendTool,
    ShellSessionStartTool, ShellSessionWaitTool,
};
pub use skill_tools::{
    GetSkillDetailTool, InstallClawHubSkillTool, ListInstalledSkillsTool,
    SearchClawHubSkillsTool, EnableSkillTool, UninstallSkillTool,
};
pub use sub_agent::{
    SubAgentArgs, SubAgentEvent, SubAgentEventKind, SubAgentKit, SubAgentManager, SubAgentTool,
};
pub use todo_write::{
    TodoItem, TodoPriority, TodoStatus, TodoWriteArgs, TodoWriteError, TodoWriteTool,
};
pub use web_fetch::WebFetchTool;
pub use web_search::{SearchResult, WebSearchConfig, WebSearchTool};
pub use write_file::WriteFileTool;

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

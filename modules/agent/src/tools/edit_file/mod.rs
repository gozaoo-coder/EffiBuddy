//! edit_file 工具集：按行号精确编辑、正则替换、历史查看/修订、撤回
//!
//! 工具组成（4 个 Tool trait 实现，共享同一份 `EditHistory`）：
//! - [`EditFileTool`]：按行号精确编辑（替换/插入/追加）
//! - [`EditFileRegexTool`]：正则表达式匹配替换（首处/全部）
//! - [`EditReviseTool`]：查看 / 修订历史编辑操作（view / list / patch）
//! - [`EditUndoTool`]：撤回指定 op_id 的编辑操作
//!
//! ## 编辑历史
//! 每次成功的 non-dry_run 编辑会记录一条快照（操作前后文件完整内容）到
//! [`EditHistory`]，分配全局唯一的 `op_id`。`edit_revise` / `edit_undo`
//! 据此查看 / 修订 / 撤回操作。
//!
//! 与 read_file（带行号输出）/ search_file（带行号命中）配合使用：
//! LLM 先读取/搜索拿到行号，再用 edit_file **精确替换目标行**，
//! 或用 edit_file_regex 按 pattern 替换。
//!
//! ## edit_file 操作模式（每个 `edits` 元素一种）
//!
//! 1. **替换**：`start_line`（+ 可选 `end_line`）+ `text`
//!    - 只给 start_line：把该行替换为 text（text 可为多行；空 text 表示删除该行）
//!    - 给 start_line + end_line：把 `[start_line, end_line]` 区间整段替换为 text
//! 2. **插入**：`insert_before` + `text`：在指定行**之前**插入新行
//!    - insert_before = 文件总行数 + 1 等价于追加到末尾
//! 3. **追加**：不给任何行号字段，直接把 text 作为**新行写入文件末尾**
//!    （即"写入文本的最后一行新行"，无需知道当前行数）
//!
//! ## 规则
//! - 行号一律为 **1-based**，指**编辑前原文件**的行号
//! - 多个 edits 自动按行号排序执行，区间互不重叠（重叠会报错）
//! - 一次调用最多 50 个操作
//! - text 支持 `<content>...</content>` / CDATA 包裹避免转义（同 write_file）
//!
//! ## 报告
//! 返回**迷你 diff**（`-` 原行 / `+` 新行，均带行号）便于 LLM 自查；
//! 若插入文本的最后一行与插入点后的首行完全相同，会输出 ⚠️ 重复插入警告
//! （LLM 最常见的复制错误：把插入点后的行也抄进了 text）。
//!
//! 启用 history 后，报告头部含 `[op_id=N]` 标识，尾部含撤回提示。
//!
//! 工作区支持：构造时传入 `cwd: Option<PathBuf>`，相对路径会 join 到 cwd。
//! 信任本地 agent 环境，路径不做沙箱限制。

mod history;
mod regex_tool;
mod report;
mod revise_tool;
mod tool;
mod types;
mod undo_tool;

pub use self::history::{
    EditHistory, EditHistoryHandle, EditOpParams, EditRecord, EditRecordKind, LineEditParams,
    new_shared_history,
};
pub use self::regex_tool::{EditFileRegexError, EditFileRegexTool};
pub use self::revise_tool::{EditReviseError, EditReviseTool};
pub use self::tool::{EditFileError, EditFileTool};
pub use self::types::{EditFileArgs, EditFileRegexArgs, EditOp, EditReviseArgs, EditUndoArgs};
pub use self::undo_tool::{EditUndoError, EditUndoTool};

/// 单次调用最大操作数，防止误传超大数组
const MAX_OPS: usize = 50;
/// 每个操作的预览最大行数
const MAX_PREVIEW_LINES: usize = 40;

#[cfg(test)]
mod ops_tests;
#[cfg(test)]
mod report_tests;
#[cfg(test)]
mod xml_input_tests;
#[cfg(test)]
mod tests_common;

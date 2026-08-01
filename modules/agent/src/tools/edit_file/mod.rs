//! edit_file 工具：按行号精确编辑本地文件
//!
//! 与 read_file（带行号输出）/ search_file（带行号命中）配合使用：
//! LLM 先读取/搜索拿到行号，再用本工具**精确替换目标行**。
//!
//! 每个 `edits` 元素支持三种操作模式：
//!
//! 1. **替换**：`start_line`（+ 可选 `end_line`）+ `text`
//!    - 只给 start_line：把该行替换为 text（text 可为多行；空 text 表示删除该行）
//!    - 给 start_line + end_line：把 `[start_line, end_line]` 区间整段替换为 text
//! 2. **插入**：`insert_before` + `text`：在指定行**之前**插入新行
//!    - insert_before = 文件总行数 + 1 等价于追加到末尾
//! 3. **追加**：不给任何行号字段，直接把 text 作为**新行写入文件末尾**
//!    （即"写入文本的最后一行新行"，无需知道当前行数）
//!
//! 规则：
//! - 行号一律为 **1-based**，指**编辑前原文件**的行号
//! - 多个 edits 自动按行号排序执行，区间互不重叠（重叠会报错）
//! - 一次调用最多 50 个操作
//! - text 支持 `<content>...</content>` / CDATA 包裹避免转义（同 write_file）
//!
//! 返回**迷你 diff**（`-` 原行 / `+` 新行，均带行号）便于 LLM 自查；
//! 若插入文本的最后一行与插入点后的首行完全相同，会输出 ⚠️ 重复插入警告
//! （LLM 最常见的复制错误：把插入点后的行也抄进了 text）。
//!
//! 工作区支持：构造时传入 `cwd: Option<PathBuf>`，相对路径会 join 到 cwd。
//! 信任本地 agent 环境，路径不做沙箱限制。

mod report;
mod tool;
mod types;

pub use self::tool::{EditFileError, EditFileTool};
pub use self::types::{EditFileArgs, EditOp};

/// 单次调用最大操作数，防止误传超大数组
const MAX_OPS: usize = 50;
/// 每个操作的预览最大行数
const MAX_PREVIEW_LINES: usize = 40;

#[cfg(test)]
mod ops_tests;
#[cfg(test)]
mod report_tests;
#[cfg(test)]
mod tests_common;

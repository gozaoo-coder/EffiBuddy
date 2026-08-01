//! edit_file 工具的数据结构定义。
//!
//! 对外 API 类型（`EditFileArgs` / `EditOp`）保持 `pub`；
//! 内部解析类型（`EditKind` / `ParsedOp` / `OpResult`）仅对 `edit_file`
//! 模块及其子模块可见（`pub(super)`），字段同步开放给 `tool` / `report` 子模块。

use serde::Deserialize;

/// 工具参数
#[derive(Deserialize)]
pub struct EditFileArgs {
    /// 要编辑的文件路径（绝对或相对工作区）
    pub path: String,
    /// 编辑操作列表（按行号自动排序执行；行号指编辑前原文件）
    pub edits: Vec<EditOp>,
    /// true = 仅预览：计算并返回完整报告（含 diff 与行号变化），不写入磁盘
    #[serde(default)]
    pub dry_run: Option<bool>,
    /// 变更明细中上下文行数（每侧），默认 1；0 = 只显示变更行本身
    #[serde(default)]
    pub diff_context: Option<usize>,
}

/// 单个编辑操作
///
/// 模式判定优先级：`insert_before` 有值 → 插入；否则 `start_line` 有值 → 替换；
/// 两者都无 → 追加到文件末尾。
#[derive(Deserialize)]
pub struct EditOp {
    /// 替换模式的起始行号（1-based，含）。None = 追加模式
    #[serde(default)]
    pub start_line: Option<usize>,
    /// 替换模式的结束行号（1-based，含）。None = 仅替换 start_line 一行
    #[serde(default)]
    pub end_line: Option<usize>,
    /// 插入模式：在指定行号之前插入新文本（与 start_line/end_line 互斥，优先）
    #[serde(default)]
    pub insert_before: Option<usize>,
    /// 目标文本（替换/插入/追加的内容）。推荐 `<content>...</content>` 包裹避免转义
    pub text: String,
}

/// 内部解析后的操作（行号统一转 0-based）
pub(super) enum EditKind {
    /// 替换原文件 [start, end]（0-based，含两端）
    Replace { start: usize, end: usize },
    /// 在原文件第 at 行（0-based）之前插入
    Insert { at: usize },
}

pub(super) struct ParsedOp {
    pub(super) kind: EditKind,
    pub(super) text: String,
    /// 用于报告的原文件行号（1-based）：替换=start_line，插入/追加=插入位置
    pub(super) orig_pos: usize,
    /// 是否为追加到末尾（仅用于报告文案）
    pub(super) is_append: bool,
}

pub(super) struct OpResult {
    /// 对应 ops 中的下标
    pub(super) op_index: usize,
    /// 新文本在新文件中的起始 0-based 行号
    pub(super) new_start: usize,
    /// 操作在**原文件**中的 0-based 起始位置（替换=区间起点，插入=插入点）
    pub(super) original_start: usize,
    /// 被替换掉的原始行（插入/追加为空）
    pub(super) original_lines: Vec<String>,
    /// 写入的新行
    pub(super) new_lines: Vec<String>,
    /// 变更前上下文（执行时状态，即旧行之前的紧邻行）
    pub(super) ctx_before: Vec<String>,
    /// ctx_before 第一行在**执行时**文件中的 0-based 行号
    pub(super) ctx_before_start: usize,
    /// 变更后上下文（执行时状态，即新行之后的紧邻行）
    pub(super) ctx_after: Vec<String>,
    /// ctx_after 第一行在**执行时**文件中的 0-based 行号
    pub(super) ctx_after_start: usize,
}

//! edit_file 工具的数据结构定义。
//!
//! 对外 API 类型（`EditFileArgs` / `EditOp` / `EditFileRegexArgs` /
//! `EditReviseArgs` / `EditUndoArgs`）保持 `pub`；
//! 内部解析类型（`EditKind` / `ParsedOp` / `OpResult`）仅对 `edit_file`
//! 模块及其子模块可见（`pub(super)`），字段同步开放给 `tool` / `report` 子模块。

use serde::Deserialize;

/// 工具参数
///
/// 字段按大小降序：String（24B）= Vec（24B）> Option<bool>（1B）= Option<usize>（16B）。
/// 这里为兼容既有 JSON 调用顺序未严格排序，padding 影响可忽略（单实例）。
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

/// `edit_file_regex` 工具参数：基于正则表达式的全文替换
///
/// 字段按大小降序：String（24B）> Option<bool>（1B）。
#[derive(Deserialize)]
pub struct EditFileRegexArgs {
    /// 要编辑的文件路径（绝对或相对工作区）
    pub path: String,
    /// 正则表达式（regex crate 语法，如 `fn \w+`、`TODO\(.*\)`）
    pub pattern: String,
    /// 替换文本。支持 `$1` / `${name}` 捕获组引用；推荐 `<content>...</content>` 包裹
    pub replacement: String,
    /// true = 替换所有匹配；false = 仅替换第一处。默认 false
    #[serde(default)]
    pub global: Option<bool>,
    /// 是否多行模式（`^` / `$` 匹配行边界，`.` 不跨行）。默认 false
    #[serde(default)]
    pub multiline: Option<bool>,
    /// 是否区分大小写。默认 false（不区分）
    #[serde(default)]
    pub case_sensitive: Option<bool>,
    /// true = 仅预览：返回将替换的匹配片段与上下文，不写入磁盘
    #[serde(default)]
    pub dry_run: Option<bool>,
    /// 预览时展示的上下文行数（每侧），默认 1；0 = 只显示命中行
    #[serde(default)]
    pub diff_context: Option<usize>,
}

/// `edit_revise` 工具参数：查看 / 修订历史编辑操作
///
/// action 取值：
/// - `view`：查看指定 op_id 的详情（原内容、新内容、行号变化、操作类型）
/// - `list`：列出最近 N 条历史操作（默认 10 条）
/// - `patch`：用新 text 重做指定 op_id（先撤回该操作，再用新 text 在原位置执行）
///
/// 字段按大小降序：String（24B）> Option<u64>（16B）= Option<usize>（16B）> u8（1B）
#[derive(Deserialize)]
pub struct EditReviseArgs {
    /// 操作类型：view / list / patch
    pub action: String,
    /// 目标 op_id（view / patch 必填，list 忽略）
    #[serde(default)]
    pub op_id: Option<u64>,
    /// list 模式下列出的最大条数，默认 10
    #[serde(default)]
    pub limit: Option<usize>,
    /// patch 模式的新文本（替换原操作的目标文本）
    #[serde(default)]
    pub new_text: Option<String>,
}

/// `edit_undo` 工具参数：撤回指定 op_id 的编辑操作
///
/// 字段按大小降序：u64（8B）> Option<bool>（1B）。
#[derive(Deserialize)]
pub struct EditUndoArgs {
    /// 要撤回的 op_id（必填，由 edit_file / edit_file_regex / edit_revise 返回）
    pub op_id: u64,
    /// true = 仅预览：返回撤回后将恢复的内容，不写入磁盘
    #[serde(default)]
    pub dry_run: Option<bool>,
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

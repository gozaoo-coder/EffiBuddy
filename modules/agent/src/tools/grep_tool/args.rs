use serde::Deserialize;

/// 工具参数
///
/// 字段按大小降序排列：String / Option<String>（24B）→ Option<usize>（16B）
/// → Option<bool>（2B），最小化结构体对齐 padding。
#[derive(Deserialize)]
pub struct GrepArgs {
    /// 正则表达式（regex crate 语法，如 `fn \w+`、`TODO\(.*\)`）
    pub pattern: String,
    /// 搜索根目录（绝对或相对工作区），默认工作区根目录
    #[serde(default)]
    pub path: Option<String>,
    /// 输出模式："content" | "files_with_matches" | "count"，默认 "content"
    #[serde(default)]
    pub output_mode: Option<String>,
    /// 文件名 glob 过滤模式（如 `*.rs`），只对文件名匹配，不对路径；默认不过滤
    #[serde(default)]
    pub glob: Option<String>,
    /// 上下文行数（命中行前后各显示 N 行），默认 0；上下文行以 `·` 前缀标记
    #[serde(default)]
    pub context: Option<usize>,
    /// 最多返回命中数（content 限制显示行数，其他模式限制文件数），默认 300
    #[serde(default)]
    pub max_matches: Option<usize>,
    /// 是否区分大小写，默认 false（不区分）
    #[serde(default)]
    pub case_sensitive: Option<bool>,
    /// 多行模式：true 时正则匹配整篇文本（可跨行），`^`/`$` 匹配行边界；
    /// 默认 false（逐行匹配，标准 grep 行为）
    #[serde(default)]
    pub multiline: Option<bool>,
}

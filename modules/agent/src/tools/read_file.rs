//! read_file 工具：让 LLM 读取本地文件内容
//!
//! 返回内容**逐行带行号**（1-based、右对齐、两空格分隔），格式与 search_file
//! 完全一致，方便 LLM 精确引用行号调用 edit_file 按行编辑：
//!
//! ```text
//!    1  fn main() {
//!    2      println!("hi");
//!   12  }
//! ```
//!
//! 支持行范围读取：`start_line` / `end_line` 只返回指定区间，避免大文件
//! 一次读完撑爆上下文（配合行号可分段精读任意区域）。
//! 默认截断到 256KB；非 UTF-8 文件以 lossy 转换返回（替换非法字节为 U+FFFD）。
//!
//! 工作区支持：构造时传入 `cwd: Option<PathBuf>`，相对路径会 join 到 cwd。
//! 绝对路径或 cwd 为 None 时按原样使用（回退到进程 cwd）。
//! 信任本地 agent 运行环境，路径不做沙箱限制。

use std::path::PathBuf;

use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::fs;

use super::resolve_path;
use super::text_utils::{format_numbered_line, line_number_width};

/// 默认最大输出字节数（256 KiB）
const DEFAULT_MAX_BYTES: u64 = 256 * 1024;

/// 工具参数
///
/// 字段按大小降序：String（24B）> Option<u64>（16B）> Option<usize>（16B）。
#[derive(Deserialize)]
pub struct ReadFileArgs {
    /// 要读取的文件绝对或相对路径
    pub path: String,
    /// 最大输出字节数，默认 256 KiB，超出部分截断
    #[serde(default)]
    pub max_bytes: Option<u64>,
    /// 起始行号（1-based，含），默认 1
    #[serde(default)]
    pub start_line: Option<usize>,
    /// 结束行号（1-based，含），默认文件末尾
    #[serde(default)]
    pub end_line: Option<usize>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("read_file error: {0}")]
pub struct ReadFileError(String);

/// 文件读取工具
///
/// `cwd` 为可选工作区：设置后相对路径以此为基准，未设置则依赖进程 cwd。
pub struct ReadFileTool {
    cwd: Option<PathBuf>,
}

impl ReadFileTool {
    pub fn new() -> Self {
        Self { cwd: None }
    }

    /// 指定工作区目录，相对路径将 join 到此目录
    pub fn with_cwd(cwd: PathBuf) -> Self {
        Self { cwd: Some(cwd) }
    }
}

impl Default for ReadFileTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ReadFileTool {
    const NAME: &'static str = "read_file";

    type Error = ReadFileError;
    type Args = ReadFileArgs;
    type Output = String;

    fn description(&self) -> String {
        let cwd_hint = self
            .cwd
            .as_ref()
            .map(|p| format!("当前工作区：{}（相对路径以此为准）", p.display()))
            .unwrap_or_else(|| "未设置工作区，相对路径依赖进程工作目录".to_string());
        format!(
            "读取本地文本文件内容并返回，**每一行前面都带行号**（1-based，如 `  12  let x = 1;`），\
             格式与 search_file 一致。适用于查看配置、日志、源码等本地文本文件。\n\n\
             **行号用于精确编辑**：根据返回的行号调用 edit_file 的 start_line/end_line \
             替换目标行；不需要按行编辑时也可用 write_file 整体重写。\n\n\
             支持行范围读取：start_line / end_line 只返回指定区间（如先读全文行数，\
             再分段精读中间区域，避免大文件一次读完）。\n\
             默认最多输出 256 KiB，超出部分截断；非 UTF-8 字节以替换字符返回。\n\
             路径不做沙箱限制（信任本地 agent 环境）。\n{cwd_hint}"
        )
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "文件路径（绝对或相对工作区）"
                },
                "max_bytes": {
                    "type": "integer",
                    "description": "最大输出字节数，默认 262144（256 KiB）",
                    "default": DEFAULT_MAX_BYTES
                },
                "start_line": {
                    "type": "integer",
                    "description": "起始行号（1-based，含），默认 1",
                    "default": 1
                },
                "end_line": {
                    "type": "integer",
                    "description": "结束行号（1-based，含），默认文件末尾"
                }
            },
            "required": ["path"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let max_bytes = args.max_bytes.unwrap_or(DEFAULT_MAX_BYTES).max(1);
        let resolved = resolve_path(&args.path, self.cwd.as_deref());

        // 先用 metadata 判断文件类型
        let metadata = fs::metadata(&resolved)
            .await
            .map_err(|e| ReadFileError(format!("读取文件元数据失败 [{}]: {e}", resolved.display())))?;

        if !metadata.is_file() {
            return Err(ReadFileError(format!(
                "路径不是常规文件 [{}]",
                resolved.display()
            )));
        }

        // 以字节读取，便于按 max_bytes 截断
        let bytes = fs::read(&resolved)
            .await
            .map_err(|e| ReadFileError(format!("读取文件失败 [{}]: {e}", resolved.display())))?;

        // lossy 解码（非法字节替换为 U+FFFD），再用 lines() 切行
        let content = String::from_utf8_lossy(&bytes).into_owned();
        let total_lines = content.lines().count();

        // 行范围过滤（1-based，含两端）
        let start = args.start_line.unwrap_or(1);
        let end_requested = args.end_line.unwrap_or(total_lines);
        if start == 0 {
            return Err(ReadFileError("start_line 必须 ≥ 1".to_string()));
        }
        if start > total_lines {
            return Err(ReadFileError(format!(
                "start_line（{start}）超出文件总行数（{total_lines}）[{}]",
                resolved.display()
            )));
        }
        if end_requested < start {
            return Err(ReadFileError(format!(
                "end_line（{end_requested}）不能小于 start_line（{start}）"
            )));
        }
        // 超出总行数时钳制到末尾
        let end = end_requested.min(total_lines);

        // 行号宽度按全文总行数计算，保证与 search_file / edit_file 的行号对齐一致
        let width = line_number_width(total_lines);

        // 逐行编号输出，超 max_bytes 即截断
        let mut out = String::with_capacity(1024);
        let mut truncated = false;
        for (i, line) in content.lines().enumerate() {
            let line_no = i + 1;
            if line_no < start {
                continue;
            }
            if line_no > end {
                break;
            }
            let numbered = format_numbered_line(line_no, width, line);
            if out.len() + numbered.len() + 1 > max_bytes as usize {
                truncated = true;
                break;
            }
            out.push_str(&numbered);
            out.push('\n');
        }

        if out.is_empty() && total_lines == 0 {
            return Ok(format!("文件为空（0 行）[{}]", resolved.display()));
        }

        // 尾部标注：文件规模 + 本次显示范围 + 截断信息，避免 LLM 误把标注当正文
        let mut note = String::from("\n");
        note.push_str(&format!(
            "[文件共 {total_lines} 行，本次显示第 {start}-{end} 行]",
        ));
        if truncated {
            note.push_str(&format!(
                "（已达最大输出 {max_bytes} 字节，后续行被截断，可用 start_line/end_line 分段读取）"
            ));
        }
        if end_requested > total_lines {
            note.push_str(&format!(
                "（请求到第 {end_requested} 行，但文件只有 {total_lines} 行，已返回全部）"
            ));
        }
        out.push_str(&note);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("effisuite-read-test-{}", uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn read_returns_numbered_lines() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
        let tool = ReadFileTool::with_cwd(dir.clone());
        let out = tool
            .call(ReadFileArgs {
                path: "a.txt".to_string(),
                max_bytes: None,
                start_line: None,
                end_line: None,
            })
            .await
            .unwrap();
        // 每行带 1-based 行号（3 行 → 宽度 1，两空格分隔）
        assert!(out.starts_with("1  fn main() {\n2      println!(\"hi\");\n3  }\n"), "out: {out}");
        assert!(out.contains("[文件共 3 行，本次显示第 1-3 行]"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn read_supports_line_range() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let content = (1..=50).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
        std::fs::write(dir.join("a.txt"), &content).unwrap();
        let tool = ReadFileTool::with_cwd(dir.clone());
        let out = tool
            .call(ReadFileArgs {
                path: "a.txt".to_string(),
                max_bytes: None,
                start_line: Some(10),
                end_line: Some(12),
            })
            .await
            .unwrap();
        // 50 行 → 宽度 2，行号右对齐
        assert!(out.contains("10  line10\n11  line11\n12  line12\n"), "out: {out}");
        assert!(out.contains("[文件共 50 行，本次显示第 10-12 行]"));
        assert!(!out.contains("line9"));
        assert!(!out.contains("line13"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn read_truncates_by_max_bytes() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let content = (1..=100).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
        std::fs::write(dir.join("a.txt"), &content).unwrap();
        let tool = ReadFileTool::with_cwd(dir.clone());
        let out = tool
            .call(ReadFileArgs {
                path: "a.txt".to_string(),
                max_bytes: Some(32),
                start_line: None,
                end_line: None,
            })
            .await
            .unwrap();
        assert!(out.contains("截断"), "out: {out}");
        assert!(out.len() < 256);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn read_start_line_beyond_file_errors() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "a\nb\n").unwrap();
        let tool = ReadFileTool::with_cwd(dir.clone());
        let r = tool
            .call(ReadFileArgs {
                path: "a.txt".to_string(),
                max_bytes: None,
                start_line: Some(99),
                end_line: None,
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("超出"));
        std::fs::remove_dir_all(&dir).ok();
    }
}

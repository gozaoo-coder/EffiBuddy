//! list_files 工具：让 LLM 列出目录内容
//!
//! 非递归模式只列一层；递归模式限制深度 3 层、总条目 500，
//! 防止遍历巨大目录树导致上下文爆炸或长时间阻塞。

use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::fs;

/// 递归最大深度
const MAX_DEPTH: usize = 3;
/// 递归最大总条目数
const MAX_ENTRIES: usize = 500;

/// 工具参数
///
/// 字段按大小降序：String（24B）> Option<bool>（1B）。
#[derive(Deserialize)]
pub struct ListFilesArgs {
    /// 要列出的目录路径
    pub path: String,
    /// 是否递归列出子目录，默认 false
    #[serde(default)]
    pub recursive: Option<bool>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("list_files error: {0}")]
pub struct ListFilesError(String);

/// 目录列举工具，无状态
pub struct ListFilesTool;

impl ListFilesTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ListFilesTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ListFilesTool {
    const NAME: &'static str = "list_files";

    type Error = ListFilesError;
    type Args = ListFilesArgs;
    type Output = String;

    fn description(&self) -> String {
        "列出指定目录下的文件与子目录。非递归只列一层；\
         递归模式限制深度 3 层、总条目 500 防止爆炸。\
         返回每条目的名称、大小、类型（file/dir）。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "目录路径（绝对或相对）"
                },
                "recursive": {
                    "type": "boolean",
                    "description": "是否递归列出子目录，默认 false",
                    "default": false
                }
            },
            "required": ["path"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let recursive = args.recursive.unwrap_or(false);
        let mut entries: Vec<String> = Vec::with_capacity(64);
        let mut count: usize = 0;

        collect_entries(&args.path, recursive, 0, &mut entries, &mut count)
            .await
            .map_err(|e| ListFilesError(format!("列举目录失败 [{}]: {e}", args.path)))?;

        if entries.is_empty() {
            return Ok(format!("目录为空 [{}]", args.path));
        }

        let mut out = String::with_capacity(entries.len() * 48);
        out.push_str(&format!("目录 {} 内容（共 {} 条）：\n", args.path, entries.len()));
        for line in entries.iter() {
            out.push_str(line);
            out.push('\n');
        }
        if count >= MAX_ENTRIES {
            out.push_str(&format!(
                "\n[已达到最大条目数 {}，后续条目被截断]",
                MAX_ENTRIES
            ));
        }
        Ok(out)
    }
}

/// 递归收集目录条目
///
/// 用迭代器消费 `read_dir`，按名称 + 大小 + 类型格式化。
/// 深度超限或条目超限时停止递归。
async fn collect_entries(
    path: &str,
    recursive: bool,
    depth: usize,
    out: &mut Vec<String>,
    count: &mut usize,
) -> std::io::Result<()> {
    if *count >= MAX_ENTRIES {
        return Ok(());
    }

    let mut reader = fs::read_dir(path).await?;
    while let Some(entry) = reader.next_entry().await? {
        if *count >= MAX_ENTRIES {
            return Ok(());
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        let metadata = entry.metadata().await?;
        let is_dir = metadata.is_dir();
        let size = metadata.len();
        let kind = if is_dir { "dir" } else { "file" };

        let indent = "  ".repeat(depth);
        out.push(format!("{indent}- {name} [{kind}, {size}B]"));

        *count += 1;

        // 递归且未超深度时下钻
        // 用 Box::pin 包装递归调用，避免 async fn 递归导致 Future 大小无限增长
        if recursive && is_dir && depth + 1 < MAX_DEPTH {
            let sub_path = entry.path().to_string_lossy().into_owned();
            Box::pin(collect_entries(&sub_path, recursive, depth + 1, out, count)).await?;
        }
    }
    Ok(())
}

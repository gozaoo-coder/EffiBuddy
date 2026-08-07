//! list_files 工具：让 LLM 列出目录内容
//!
//! 非递归模式只列一层；递归模式限制深度 3 层、总条目 500，
//! 防止遍历巨大目录树导致上下文爆炸或长时间阻塞。
//!
//! 输出每行一个条目，使用**完整相对路径**（相对 path 参数指向的根目录，
//! Windows 下统一为 `/` 分隔），便于 LLM 直接回传给 read_file / edit_file。
//!
//! 工作区支持：构造时传入 `cwd: Option<PathBuf>`，相对路径会 join 到 cwd。

use std::path::{Path, PathBuf};

use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::fs;

use super::resolve_path;

/// 递归最大深度
const MAX_DEPTH: usize = 3;
/// 递归最大总条目数
const MAX_ENTRIES: usize = 500;

/// 工具参数
///
/// 字段按大小降序：`Option<String>`（24B，NonNull niche 优化）
/// > `Option<bool>`（1B）。
#[derive(Deserialize)]
pub struct ListFilesArgs {
    /// 要列出的目录路径（绝对或相对工作区），默认工作区根目录
    #[serde(default)]
    pub path: Option<String>,
    /// 是否递归列出子目录，默认 false
    #[serde(default)]
    pub recursive: Option<bool>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("list_files error: {0}")]
pub struct ListFilesError(String);

/// 目录列举工具
pub struct ListFilesTool {
    cwd: Option<PathBuf>,
}

impl ListFilesTool {
    pub fn new() -> Self {
        Self { cwd: None }
    }

    /// 指定工作区目录，相对路径将 join 到此目录
    pub fn with_cwd(cwd: PathBuf) -> Self {
        Self { cwd: Some(cwd) }
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
        "列目录结构（不按模式过滤）。按文件名模式匹配用 glob；搜文件内容用 search_file/grep。\
         非递归只列一层；递归限制深度 3 层、总条目 500 防爆炸。\
         返回每条目完整相对路径、大小、类型（file/dir），便于回传 read_file/edit_file。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "目录路径（绝对或相对工作区），默认工作区根目录" },
                "recursive": { "type": "boolean", "description": "是否递归列出子目录，默认 false", "default": false }
            }
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let recursive = args.recursive.unwrap_or(false);

        // 根目录：path 参数优先，否则工作区根（无 cwd 时回退进程 cwd）
        // 与 glob_tool 的 path 参数行为一致
        let root = match &args.path {
            Some(p) => resolve_path(p, self.cwd.as_deref()),
            None => self.cwd.clone().unwrap_or_else(|| PathBuf::from(".")),
        };
        let root_meta = fs::metadata(&root)
            .await
            .map_err(|e| ListFilesError(format!("访问目录失败 [{}]: {e}", root.display())))?;
        if !root_meta.is_dir() {
            return Err(ListFilesError(format!(
                "路径不是目录 [{}]",
                root.display()
            )));
        }

        // 统一展示路径（Windows 下 \ → /），与 glob_tool 一致
        let root_display = {
            let s = root.to_string_lossy().into_owned();
            if cfg!(windows) {
                s.replace('\\', "/")
            } else {
                s
            }
        };

        let mut entries: Vec<String> = Vec::with_capacity(64);
        let mut count: usize = 0;
        collect_entries(&root, "", recursive, 0, &mut entries, &mut count)
            .await
            .map_err(|e| ListFilesError(format!("列举目录失败 [{}]: {e}", root_display)))?;

        if entries.is_empty() {
            return Ok(format!("目录为空 [{}]", root_display));
        }

        // 输出：每行一个条目（完整相对路径 + 元信息），尾部附统计信息
        let mut out = String::with_capacity(entries.len() * 48 + 160);
        for line in entries.iter() {
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
        if count >= MAX_ENTRIES {
            out.push_str(&format!(
                "共 {} 条（已达上限 {}，后续条目被截断），根目录 {}",
                entries.len(),
                MAX_ENTRIES,
                root_display
            ));
        } else {
            out.push_str(&format!("共 {} 条，根目录 {}", entries.len(), root_display));
        }
        Ok(out)
    }
}

/// 递归收集目录条目
///
/// 用迭代器消费 `read_dir`，按 **完整相对路径** + 大小 + 类型格式化。
/// 深度超限或条目超限时停止递归。
///
/// - `dir`：当前正在遍历的目录
/// - `rel_prefix`：当前目录相对根的路径段（根目录下为空字符串，子目录为 `parent/child`
///   形式，始终使用 `/` 分隔，跨平台一致；由外层累积传入，避免对每个条目重复 strip_prefix）
async fn collect_entries(
    dir: &Path,
    rel_prefix: &str,
    recursive: bool,
    depth: usize,
    out: &mut Vec<String>,
    count: &mut usize,
) -> std::io::Result<()> {
    if *count >= MAX_ENTRIES {
        return Ok(());
    }

    let mut reader = fs::read_dir(dir).await?;
    while let Some(entry) = reader.next_entry().await? {
        if *count >= MAX_ENTRIES {
            return Ok(());
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        let metadata = entry.metadata().await?;
        let is_dir = metadata.is_dir();
        let size = metadata.len();
        let kind = if is_dir { "dir" } else { "file" };

        // 完整相对路径：rel_prefix 为空时直接用 name（move，避免 clone）；
        // 否则用 format! 拼接。rel_prefix 始终用 `/` 分隔，保证跨平台一致
        let full_rel = if rel_prefix.is_empty() {
            name
        } else {
            format!("{rel_prefix}/{name}")
        };

        out.push(format!("{full_rel} [{kind}, {size}B]"));
        *count += 1;

        // 递归且未超深度时下钻
        // 用 Box::pin 包装递归调用，避免 async fn 递归导致 Future 大小无限增长
        if recursive && is_dir && depth + 1 < MAX_DEPTH {
            let sub_dir = entry.path();
            Box::pin(collect_entries(&sub_dir, &full_rel, recursive, depth + 1, out, count))
                .await?;
        }
    }
    Ok(())
}

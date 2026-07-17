//! read_file 工具：让 LLM 读取本地文件内容
//!
//! 信任本地 agent 运行环境，路径不做沙箱限制。
//! 默认截断到 256KB，避免一次性读取超大文件导致上下文爆炸。
//! 非 UTF-8 文件会以 lossy 转换返回（替换非法字节为 U+FFFD）。
//!
//! 工作区支持：构造时传入 `cwd: Option<PathBuf>`，相对路径会 join 到 cwd。
//! 绝对路径或 cwd 为 None 时按原样使用（回退到进程 cwd）。

use std::path::PathBuf;

use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::fs;

use super::resolve_path;

/// 默认最大读取字节数（256 KiB）
const DEFAULT_MAX_BYTES: u64 = 256 * 1024;

/// 工具参数
///
/// 字段按大小降序：String（24B）> Option<u64>（16B）。
#[derive(Deserialize)]
pub struct ReadFileArgs {
    /// 要读取的文件绝对或相对路径
    pub path: String,
    /// 最大读取字节数，默认 256 KiB，超出部分截断
    #[serde(default)]
    pub max_bytes: Option<u64>,
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
            "读取本地文件内容并返回文本。路径不做沙箱限制（信任本地 agent 环境）。\
             默认最多读取 256 KiB，超出部分截断；非 UTF-8 字节以替换字符返回。\
             适用于查看配置、日志、源码等本地文本文件。\n{cwd_hint}"
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
                    "description": "最大读取字节数，默认 262144（256 KiB）",
                    "default": DEFAULT_MAX_BYTES
                }
            },
            "required": ["path"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let max_bytes = args.max_bytes.unwrap_or(DEFAULT_MAX_BYTES).max(1);
        let resolved = resolve_path(&args.path, self.cwd.as_deref());

        // 先用 metadata 判断文件大小，决定是否需要分段读取
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

        let take = if bytes.len() as u64 > max_bytes {
            // 在 UTF-8 字符边界处截断，避免切坏多字节字符
            let mut end = max_bytes as usize;
            if end > bytes.len() {
                end = bytes.len();
            }
            // 向前回退到字符边界
            while end > 0 && (bytes[end] & 0xC0) == 0x80 {
                end -= 1;
            }
            end
        } else {
            bytes.len()
        };

        let content = String::from_utf8_lossy(&bytes[..take]).into_owned();
        let truncated = bytes.len() as u64 > max_bytes;
        let mut out = String::with_capacity(content.len() + 64);
        out.push_str(&content);
        if truncated {
            out.push_str(&format!(
                "\n\n[已截断：文件总大小 {} 字节，仅返回前 {} 字节]",
                bytes.len(),
                take
            ));
        }
        Ok(out)
    }
}

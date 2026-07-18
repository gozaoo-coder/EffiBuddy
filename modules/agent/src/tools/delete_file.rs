//! delete_file 工具：让 LLM 删除本地文件或目录
//!
//! 与 [`ReadFileTool`] / [`WriteFileTool`] 对称，提供"删"能力。
//! 默认仅删除文件或空目录；设置 recursive=true 时可删除非空目录。
//!
//! 工作区支持：构造时传入 `cwd: Option<PathBuf>`，相对路径会 join 到 cwd。
//! 信任本地 agent 运行环境，路径不做沙箱限制。

use std::path::PathBuf;

use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::fs;

use super::resolve_path;

/// 工具参数
///
/// 字段按大小降序：String（24B）> Option<bool>（1B）。
#[derive(Deserialize)]
pub struct DeleteFileArgs {
    /// 要删除的文件或目录路径（绝对或相对工作区）
    pub path: String,
    /// 删除目录时是否递归删除非空目录，默认 false
    #[serde(default)]
    pub recursive: Option<bool>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("delete_file error: {0}")]
pub struct DeleteFileError(String);

/// 文件/目录删除工具
///
/// `cwd` 为可选工作区：设置后相对路径以此为基准，未设置则依赖进程 cwd。
pub struct DeleteFileTool {
    cwd: Option<PathBuf>,
}

impl DeleteFileTool {
    pub fn new() -> Self {
        Self { cwd: None }
    }

    /// 指定工作区目录，相对路径将 join 到此目录
    pub fn with_cwd(cwd: PathBuf) -> Self {
        Self { cwd: Some(cwd) }
    }
}

impl Default for DeleteFileTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for DeleteFileTool {
    const NAME: &'static str = "delete_file";

    type Error = DeleteFileError;
    type Args = DeleteFileArgs;
    type Output = String;

    fn description(&self) -> String {
        let cwd_hint = self
            .cwd
            .as_ref()
            .map(|p| format!("当前工作区：{}（相对路径以此为准）", p.display()))
            .unwrap_or_else(|| "未设置工作区，相对路径依赖进程工作目录".to_string());
        format!(
            "删除本地文件或空目录。路径不做沙箱限制（信任本地 agent 环境）。\
             默认仅删除文件或空目录；设置 recursive=true 可删除非空目录。\
             删除前请确认路径正确，删除后不可恢复。\n{cwd_hint}"
        )
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "文件或目录路径（绝对或相对工作区）"
                },
                "recursive": {
                    "type": "boolean",
                    "description": "删除目录时是否递归删除非空目录，默认 false",
                    "default": false
                }
            },
            "required": ["path"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let recursive = args.recursive.unwrap_or(false);
        let resolved = resolve_path(&args.path, self.cwd.as_deref());
        let path_display = resolved.display().to_string();

        // 先检查路径是否存在
        let metadata = match fs::metadata(&resolved).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(DeleteFileError(format!(
                    "路径不存在 [{}]",
                    path_display
                )));
            }
            Err(e) => {
                return Err(DeleteFileError(format!(
                    "获取路径元数据失败 [{}]: {e}",
                    path_display
                )));
            }
        };

        if metadata.is_file() || metadata.is_symlink() {
            fs::remove_file(&resolved)
                .await
                .map_err(|e| DeleteFileError(format!("删除文件失败 [{}]: {e}", path_display)))?;
            Ok(format!("已删除文件 [{}]", path_display))
        } else if metadata.is_dir() {
            if recursive {
                fs::remove_dir_all(&resolved)
                    .await
                    .map_err(|e| DeleteFileError(format!("递归删除目录失败 [{}]: {e}", path_display)))?;
                Ok(format!("已递归删除目录 [{}]", path_display))
            } else {
                fs::remove_dir(&resolved)
                    .await
                    .map_err(|e| DeleteFileError(format!("删除目录失败 [{}]: {e}。若目录非空，请设置 recursive=true", path_display)))?;
                Ok(format!("已删除空目录 [{}]", path_display))
            }
        } else {
            Err(DeleteFileError(format!(
                "未知路径类型 [{}]",
                path_display
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("effisuite-delete-test-{}", uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn delete_file_ok() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join("test.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let tool = DeleteFileTool::with_cwd(dir.clone());
        let result = tool
            .call(DeleteFileArgs {
                path: "test.txt".to_string(),
                recursive: None,
            })
            .await
            .unwrap();
        assert!(result.contains("已删除文件"));
        assert!(!file_path.exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn delete_empty_dir_ok() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let empty_dir = dir.join("empty");
        std::fs::create_dir(&empty_dir).unwrap();

        let tool = DeleteFileTool::with_cwd(dir.clone());
        let result = tool
            .call(DeleteFileArgs {
                path: "empty".to_string(),
                recursive: None,
            })
            .await
            .unwrap();
        assert!(result.contains("已删除空目录"));
        assert!(!empty_dir.exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn delete_non_empty_dir_without_recursive_fails() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let nested = dir.join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("file.txt"), "content").unwrap();

        let tool = DeleteFileTool::with_cwd(dir.clone());
        let result = tool
            .call(DeleteFileArgs {
                path: "nested".to_string(),
                recursive: Some(false),
            })
            .await;
        assert!(result.is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn delete_non_empty_dir_with_recursive_ok() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let nested = dir.join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("file.txt"), "content").unwrap();
        std::fs::write(nested.join("file2.txt"), "content2").unwrap();

        let tool = DeleteFileTool::with_cwd(dir.clone());
        let result = tool
            .call(DeleteFileArgs {
                path: "nested".to_string(),
                recursive: Some(true),
            })
            .await
            .unwrap();
        assert!(result.contains("已递归删除目录"));
        assert!(!nested.exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn delete_missing_path_fails() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();

        let tool = DeleteFileTool::with_cwd(dir.clone());
        let result = tool
            .call(DeleteFileArgs {
                path: "not-exist.txt".to_string(),
                recursive: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("不存在"));

        std::fs::remove_dir_all(&dir).ok();
    }
}

//! shell 工具：让 LLM 执行本地 shell 命令
//!
//! 这是集成 agent-reach 和 browser-act 的关键入口：
//! - agent-reach：LLM 可调用 `agent-reach doctor`、`agent-reach install --env=auto --safe`、
//!   `opencli twitter search "query"` 等
//! - browser-act：LLM 可调用 `browser-act browser list`、`browser-act fetch "url"` 等
//!
//! 跨平台：Windows 用 `cmd /c`，Unix 用 `sh -c`。
//! 捕获 stdout + stderr，截断到 8 KiB 返回，避免上下文爆炸。
//! 默认超时 30s，用 tokio::time::timeout 防止挂死。
//!
//! 工作区支持：构造时传入 `cwd: Option<PathBuf>`，命令的子进程工作目录设为此目录。

use std::path::PathBuf;

use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::process::Command;

/// 默认命令超时（30 秒）
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// 输出最大字节数（8 KiB）
const MAX_OUTPUT_BYTES: usize = 8 * 1024;

/// 工具参数
///
/// 字段按大小降序：String（24B）> Option<u64>（16B）。
#[derive(Deserialize)]
pub struct ShellArgs {
    /// 要执行的 shell 命令字符串
    pub command: String,
    /// 命令超时毫秒数，默认 30000（30s）
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("shell error: {0}")]
pub struct ShellError(String);

/// Shell 命令执行工具
pub struct ShellTool {
    cwd: Option<PathBuf>,
}

impl ShellTool {
    pub fn new() -> Self {
        Self { cwd: None }
    }

    /// 指定工作区目录，子进程 cwd 设为此目录
    pub fn with_cwd(cwd: PathBuf) -> Self {
        Self { cwd: Some(cwd) }
    }
}

impl Default for ShellTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ShellTool {
    const NAME: &'static str = "shell";

    type Error = ShellError;
    type Args = ShellArgs;
    type Output = String;

    fn description(&self) -> String {
        let cwd_hint = self
            .cwd
            .as_ref()
            .map(|p| format!("当前工作区：{}（命令在此目录执行）", p.display()))
            .unwrap_or_else(|| "未设置工作区，命令在进程工作目录执行".to_string());
        format!(
            "在本地执行 shell 命令并返回 stdout+stderr。跨平台：Windows 用 cmd /c，Unix 用 sh -c。\
             默认超时 30 秒，输出截断到 8 KiB。\
             可用于调用已安装的 CLI 工具，例如：\n\
             - agent-reach: `agent-reach doctor`、`agent-reach install --env=auto --safe`、`opencli twitter search \"query\"`\n\
             - browser-act: `browser-act browser list`、`browser-act fetch \"url\"`\n\
             注意：这是本地命令执行，请谨慎调用可能修改系统的命令。\n{cwd_hint}"
        )
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "要执行的 shell 命令（如 `agent-reach doctor`、`browser-act browser list`）"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "命令超时毫秒数，默认 30000",
                    "default": DEFAULT_TIMEOUT_MS
                }
            },
            "required": ["command"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let timeout_ms = args.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).max(1);

        // 跨平台选择 shell
        let mut cmd = if cfg!(target_os = "windows") {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(&args.command);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg(&args.command);
            c
        };

        // 设置工作区目录（若配置）
        if let Some(cwd) = &self.cwd {
            cmd.current_dir(cwd);
        }

        // 不继承父进程的 stdin，避免阻塞
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            // Windows 上关闭窗口，避免在控制台进程组里产生额外输出
            ;

        let child = cmd
            .spawn()
            .map_err(|e| ShellError(format!("启动命令失败 [{}]: {e}", args.command)))?;

        // 用 tokio::time::timeout 包装等待，超时则返回错误
        let wait = async {
            let output = child.wait_with_output().await?;
            std::io::Result::Ok(output)
        };

        let output = match tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            wait,
        )
        .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Err(ShellError(format!(
                    "等待命令输出失败 [{}]: {e}",
                    args.command
                )));
            }
            Err(_) => {
                return Err(ShellError(format!(
                    "命令超时（{}ms），可能已挂死 [{}]",
                    timeout_ms, args.command
                )));
            }
        };

        // 合并 stdout + stderr，截断到 MAX_OUTPUT_BYTES
        let mut combined = Vec::with_capacity(output.stdout.len() + output.stderr.len());
        combined.extend_from_slice(&output.stdout);
        combined.extend_from_slice(&output.stderr);

        let truncated = combined.len() > MAX_OUTPUT_BYTES;
        let take = if truncated {
            // 在 UTF-8 字符边界处截断
            let mut end = MAX_OUTPUT_BYTES;
            if end > combined.len() {
                end = combined.len();
            }
            while end > 0 && (combined[end] & 0xC0) == 0x80 {
                end -= 1;
            }
            end
        } else {
            combined.len()
        };

        let body = String::from_utf8_lossy(&combined[..take]).into_owned();
        let mut out = String::with_capacity(body.len() + 64);
        out.push_str(&format!("exit code: {}\n", output.status.code().unwrap_or(-1)));
        out.push_str(&body);
        if truncated {
            out.push_str(&format!(
                "\n\n[输出已截断：总 {} 字节，仅返回前 {} 字节]",
                combined.len(),
                take
            ));
        }
        Ok(out)
    }
}

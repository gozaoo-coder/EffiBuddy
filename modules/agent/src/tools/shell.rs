//! shell 工具：让 LLM 执行本地 shell 命令
//! 这是集成 agent-reach 和 browser-act 的关键入口：
//! - agent-reach：LLM 可调用 `agent-reach doctor`、`agent-reach install --env=auto --safe`、
//!   `opencli twitter search "query"` 等
//! - browser-act：LLM 可调用 `browser-act browser list`、`browser-act fetch "url"` 等
//!
//! 跨平台且可自选 shell：默认 Windows 上 bash → powershell → cmd，Unix 用 `sh`；
//! 也可在 `shell` 参数里显式指定 bash / cmd / powershell / sh / auto。
//! 具体 shell 选择、编码与窗口隐藏策略见 [`crate::shell_env`]。
//! 捕获 stdout + stderr，截断到 8 KiB 返回，避免上下文爆炸。

use std::path::PathBuf;

use crate::shell_env::{self, ShellKind};
use rig_core::tool::Tool;
use serde::Deserialize;

/// 默认命令超时（30 秒）
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// 输出最大字节数（8 KiB）
const MAX_OUTPUT_BYTES: usize = 8 * 1024;

/// 工具参数
///
/// 字段按大小降序：String（24B）> Option<String>（24B）> Option<u64>（16B）。
#[derive(Deserialize)]
pub struct ShellArgs {
    /// 要执行的 shell 命令字符串
    pub command: String,
    /// 使用的命令行工具：auto / bash / cmd / powershell / sh；默认 auto（自动选择）
    #[serde(default)]
    pub shell: Option<String>,
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
        let shell_line = {
            let avail = shell_env::available_shells()
                .iter()
                .map(|k| k.label())
                .collect::<Vec<_>>()
                .join(" / ");
            format!(
                "当前可用 shell：{avail}，不指定时默认用 {}。\
                 可用 `shell` 参数显式选择（bash / cmd / powershell / sh / auto）",
                shell_env::shell_kind().label()
            )
        };
        format!(
            "在本地执行 shell 命令并返回 stdout+stderr。{shell_line}。\
                默认超时 30 秒，输出截断到 8 KiB；Windows 上静默运行，不弹出控制台窗口。\
                可用于调用已安装的 CLI 工具，例如：\n\
                 - agent-reach: `agent-reach doctor`、`agent-reach install --env=auto --safe`、`opencli twitter search \"query\"`\n\
                 - browser-act: `browser-act browser list`、`browser-act fetch \"url\"`\n\
                 **注意**：本工具一次性执行（每次新进程，不保留状态）。\
                 如需多步操作、保持工作目录、长任务或交互式命令，改用 shell_session_start +\
                 shell_session_send + shell_session_read（后台常驻会话，前端底栏可见）。\
                 **Windows 环境提示**：bash 下 ls/grep/cat 等 Unix 工具开箱即用；\
                 powershell 适合脚本/管道对象，中文输出乱码可先 `chcp 65001`。\n\
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
                "shell": {
                    "type": "string",
                    "enum": ["auto", "bash", "cmd", "powershell", "sh"],
                    "description": "要使用的命令行工具，默认 auto（自动选择）"
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

        // 解析并校验 AI 指定的 shell；未指定用默认策略（auto）
        let kind = shell_env::resolve(args.shell.as_deref()).map_err(ShellError)?;
        let mut cmd = shell_env::run_command_for(kind, &args.command);

        // 设置工作区目录（若配置）
        if let Some(cwd) = &self.cwd {
            cmd.current_dir(cwd);
        }

        // 不继承父进程的 stdin，避免阻塞
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        // Windows 上关闭控制台窗口（CREATE_NO_WINDOW），避免弹出 cmd/bash 黑窗
        shell_env::apply_no_window(&mut cmd);

        let child = cmd
            .spawn()
            .map_err(|e| {
                let hint = match kind {
                    ShellKind::Bash => "（未找到可用的 bash，可安装 Git for Windows 后重试）",
                    ShellKind::Cmd => "（cmd 启动失败，请检查系统 shell）",
                    ShellKind::PowerShell => "（未找到可用的 PowerShell）",
                    ShellKind::Sh => "",
                };
                ShellError(format!("启动命令失败 [{}]: {e}{hint}", args.command))
            })?;

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

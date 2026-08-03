//! shell 环境：选择并构造命令执行器（bash / cmd / powershell / sh），统一隐藏窗口策略
//!
//! 单一职责：把「用哪个 shell 跑命令」集中在这里，`shell`（一次性）与
//! `shell_session`（常驻会话）两个工具共用，避免各自维护一套分支。
//!
//! AI 可在工具参数里显式指定 shell（`shell` 字段），本模块负责解析与可用性校验：
//! - `bash`：Windows 用 Git Bash / MSYS2 bash，Unix 用原生 bash
//! - `cmd`：Windows cmd.exe
//! - `powershell`：Windows PowerShell（优先 pwsh / v7，回退系统自带 powershell.exe）
//! - `sh`：Unix POSIX sh（Windows 上等同 bash，Git Bash 提供）
//! - `auto`（或不填）：按默认策略选 —— Windows 上 bash → powershell → cmd；Unix 上 sh
//!
//! 所有子进程在 Windows 上统一加 `CREATE_NO_WINDOW`（见 [`apply_no_window`]），
//! 保证不弹出控制台窗口。探测结果用 `OnceLock` 缓存一次，避免每次命令都查磁盘。

use std::path::PathBuf;
use std::sync::OnceLock;

use tokio::process::Command;

/// 可用 shell 的种类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    /// Git Bash / MSYS2 bash（Windows）或 Unix 原生 bash
    Bash,
    /// cmd.exe（Windows）
    Cmd,
    /// PowerShell：优先 pwsh（v7），回退系统自带 powershell.exe（v5.1）
    PowerShell,
    /// POSIX sh（Unix）
    Sh,
}

impl ShellKind {
    /// 显示名（会话便签 / 日志 / 描述用）
    pub fn label(self) -> &'static str {
        match self {
            ShellKind::Bash => "bash",
            ShellKind::Cmd => "cmd",
            ShellKind::PowerShell => "powershell",
            ShellKind::Sh => "sh",
        }
    }

    /// 写入 stdin 命令时使用的换行符：bash/sh/powershell 用 LF，cmd 需 CRLF
    pub fn line_ending(self) -> &'static str {
        match self {
            ShellKind::Bash | ShellKind::Sh | ShellKind::PowerShell => "\n",
            ShellKind::Cmd => "\r\n",
        }
    }

    /// 给 AI 看的简要说明（description 拼装用）
    pub fn describe(self) -> &'static str {
        match self {
            ShellKind::Bash => "bash（Git Bash，支持 ls/grep/cat 等 Unix 工具）",
            ShellKind::Cmd => "cmd（Windows 原生命令提示符）",
            ShellKind::PowerShell => "powershell（Windows PowerShell，脚本/管道对象能力更强）",
            ShellKind::Sh => "sh（POSIX）",
        }
    }
}

/// 解析 AI 请求的 shell 名称 → ShellKind（`auto`/`default`/空 返回默认策略结果）
pub fn parse_shell(name: &str) -> Result<ShellKind, String> {
    match name.trim().to_ascii_lowercase().as_str() {
        "bash" | "bash.exe" => Ok(ShellKind::Bash),
        // Windows 上用 Git Bash 兼容 sh 语法；Unix 上用真 POSIX sh
        "sh" | "sh.exe" => {
            if cfg!(target_os = "windows") {
                Ok(ShellKind::Bash)
            } else {
                Ok(ShellKind::Sh)
            }
        }
        "cmd" | "cmd.exe" => Ok(ShellKind::Cmd),
        "powershell" | "pwsh" | "powershell.exe" | "pwsh.exe" | "ps" => {
            Ok(ShellKind::PowerShell)
        }
        "auto" | "default" | "" => Ok(shell_kind()),
        other => Err(format!(
            "不支持的 shell `{other}`（可用：bash / cmd / powershell / sh / auto）"
        )),
    }
}

/// 当前系统上该 shell 是否可用（Windows 探测安装情况；Unix 原生 bash/sh 恒可用）
pub fn is_available(kind: ShellKind) -> bool {
    match kind {
        ShellKind::Bash => {
            if cfg!(target_os = "windows") {
                find_bash().is_some()
            } else {
                true
            }
        }
        ShellKind::Cmd => cfg!(target_os = "windows"),
        ShellKind::PowerShell => find_powershell().is_some(),
        ShellKind::Sh => !cfg!(target_os = "windows"),
    }
}

/// 当前系统可用的 shell 列表（供 description / 错误提示展示）
pub fn available_shells() -> Vec<ShellKind> {
    [
        ShellKind::Bash,
        ShellKind::Cmd,
        ShellKind::PowerShell,
        ShellKind::Sh,
    ]
    .into_iter()
    .filter(|k| is_available(*k))
    .collect()
}

/// 默认 shell（不指定时）：Windows 上 bash → powershell → cmd；Unix 上 sh
pub fn shell_kind() -> ShellKind {
    if cfg!(target_os = "windows") {
        if find_bash().is_some() {
            ShellKind::Bash
        } else if find_powershell().is_some() {
            ShellKind::PowerShell
        } else {
            ShellKind::Cmd
        }
    } else {
        ShellKind::Sh
    }
}

/// 解析并校验 AI 请求的 shell；不填 / `auto` 用默认策略。
/// 校验失败（如在 Windows 上请求不可用的 shell）返回带可用列表的错误。
pub fn resolve(request: Option<&str>) -> Result<ShellKind, String> {
    let kind = match request {
        Some(name) if !name.trim().is_empty() => parse_shell(name)?,
        _ => return Ok(shell_kind()),
    };
    if is_available(kind) {
        Ok(kind)
    } else {
        let avail = available_shells()
            .iter()
            .map(|k| k.label())
            .collect::<Vec<_>>()
            .join(" / ");
        Err(format!(
            "shell `{}` 当前不可用，可用：{}（可用 shell 工具的 shell 字段 / shell_session_start 的 shell 字段指定）",
            kind.label(),
            avail
        ))
    }
}

/// 构造「一次性执行命令」的子进程命令（默认 shell，跑完即退出）。
pub fn run_command(command: &str) -> Command {
    run_command_for(shell_kind(), command)
}

/// 构造「一次性执行命令」的子进程命令（指定 shell，需先经 [`resolve`] 校验可用）。
pub fn run_command_for(kind: ShellKind, command: &str) -> Command {
    match kind {
        ShellKind::Bash => {
            let mut c = Command::new(bash_program());
            c.arg("-c").arg(command);
            c
        }
        ShellKind::Cmd => {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(command);
            c
        }
        ShellKind::PowerShell => {
            // -EncodedCommand：Base64(UTF-16LE) 编码，规避中文/引号/特殊字符的编码坑
            let mut c = Command::new(powershell_program());
            c.arg("-NoProfile")
                .arg("-NonInteractive")
                .arg("-EncodedCommand")
                .arg(powershell_encode(command));
            c
        }
        ShellKind::Sh => {
            let mut c = Command::new("sh");
            c.arg("-c").arg(command);
            c
        }
    }
}

/// 构造「常驻交互会话」的子进程命令（默认 shell，保持运行、从 stdin 逐条读命令）。
pub fn session_command() -> (ShellKind, Command) {
    session_command_for(shell_kind())
}

/// 构造「常驻交互会话」的子进程命令（指定 shell，需先经 [`resolve`] 校验可用）。
pub fn session_command_for(kind: ShellKind) -> (ShellKind, Command) {
    let c = match kind {
        ShellKind::Bash => {
            // --noprofile --norc：不读用户配置，启动快、无横幅噪音
            let mut c = Command::new(bash_program());
            c.arg("--noprofile").arg("--norc");
            c
        }
        ShellKind::Cmd => {
            // /Q 关闭命令回显；/K 执行后保持交互；prompt $G 把提示符设为 `>`
            let mut c = Command::new("cmd");
            c.arg("/Q").arg("/K").arg("prompt $G");
            c
        }
        ShellKind::PowerShell => {
            // -NoLogo 无横幅；-NoProfile 不读用户配置；-Command - 从 stdin 读命令（REPL）
            let mut c = Command::new(powershell_program());
            c.arg("-NoLogo").arg("-NoProfile").arg("-Command").arg("-");
            c
        }
        ShellKind::Sh => Command::new("sh"),
    };
    (kind, c)
}

/// Windows 上隐藏子进程控制台窗口（CREATE_NO_WINDOW）；其他平台无操作。
#[cfg(windows)]
pub fn apply_no_window(cmd: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// 非 Windows：无需隐藏窗口
#[cfg(not(windows))]
pub fn apply_no_window(_cmd: &mut Command) {}

// ---------------------------------------------------------------
// 内部：bash / powershell 探测与程序路径
// ---------------------------------------------------------------

/// bash 程序路径：Windows 用探测到的绝对路径（Git Bash / MSYS2 / PATH）；
/// Unix 直接用 PATH 里的 `bash`。
fn bash_program() -> &'static PathBuf {
    #[cfg(windows)]
    {
        bash_path()
    }
    #[cfg(not(windows))]
    {
        static CACHE: OnceLock<PathBuf> = OnceLock::new();
        CACHE.get_or_init(|| PathBuf::from("bash"))
    }
}

/// PowerShell 程序路径（调用前需保证 [`is_available`] 判定 PowerShell 可用；
/// 非 Windows 平台不可达，因为 resolve 会先拦截）。
fn powershell_program() -> &'static PathBuf {
    static CACHE: OnceLock<Option<PathBuf>> = OnceLock::new();
    CACHE
        .get_or_init(find_powershell)
        .as_ref()
        .expect("PowerShell 已探测存在（is_available 判定时必然命中缓存）")
}

/// 把命令编码为 PowerShell `-EncodedCommand` 需要的 Base64(UTF-16LE)。
fn powershell_encode(command: &str) -> String {
    let utf16le: Vec<u8> = command
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(&utf16le)
}

/// Windows 上返回探测到的 bash 绝对路径（调用前需保证 [`is_available`] 判定 Bash 可用）。
#[cfg(windows)]
fn bash_path() -> &'static PathBuf {
    static CACHE: OnceLock<Option<PathBuf>> = OnceLock::new();
    CACHE
        .get_or_init(find_bash)
        .as_ref()
        .expect("bash 已探测存在（is_available 判定为 Bash 时必然命中缓存）")
}

/// Windows 上查找 bash：先查常见安装路径，再解析 PATH；结果缓存一次。
#[cfg(windows)]
fn find_bash() -> Option<PathBuf> {
    const CANDIDATES: &[&str] = &[
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files\Git\usr\bin\bash.exe",
        r"C:\Program Files (x86)\Git\bin\bash.exe",
        r"C:\msys64\usr\bin\bash.exe",
    ];
    for p in CANDIDATES {
        let b = PathBuf::from(p);
        if b.is_file() {
            return Some(b);
        }
    }
    // 解析 PATH：跳过 C:\Windows\System32（那是 WSL 的 bash 启动器，不是原生 bash）
    if let Some(env) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&env) {
            let lower = dir.to_string_lossy().to_ascii_lowercase();
            if lower.starts_with(r"c:\windows\system32") {
                continue;
            }
            let b = dir.join("bash.exe");
            if b.is_file() {
                return Some(b);
            }
        }
    }
    None
}

/// Windows 上查找 PowerShell：优先 pwsh（v7），回退系统自带 powershell.exe（v5.1）。
#[cfg(windows)]
fn find_powershell() -> Option<PathBuf> {
    // 1. PowerShell 7（pwsh）常见安装路径
    const CANDIDATES: &[&str] = &[
        r"C:\Program Files\PowerShell\7\pwsh.exe",
        r"C:\Program Files\PowerShell\7-preview\pwsh.exe",
    ];
    for p in CANDIDATES {
        let b = PathBuf::from(p);
        if b.is_file() {
            return Some(b);
        }
    }
    // 2. PATH 里的 pwsh.exe
    if let Some(env) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&env) {
            let b = dir.join("pwsh.exe");
            if b.is_file() {
                return Some(b);
            }
        }
    }
    // 3. 系统自带 Windows PowerShell（v5.1，Windows 10+ 恒有）
    let sys = PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
    if sys.is_file() {
        return Some(sys);
    }
    None
}

/// 非 Windows：无 PowerShell
#[cfg(not(windows))]
fn find_powershell() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn parse_known_shells() {
        assert_eq!(parse_shell("bash").unwrap(), ShellKind::Bash);
        assert_eq!(parse_shell("CMD").unwrap(), ShellKind::Cmd);
        assert_eq!(parse_shell("powershell").unwrap(), ShellKind::PowerShell);
        assert_eq!(parse_shell("pwsh").unwrap(), ShellKind::PowerShell);
        assert!(parse_shell("fish").is_err());
    }

    #[test]
    fn default_shell_is_available() {
        let kind = shell_kind();
        assert!(is_available(kind), "默认 shell {kind:?} 应可用");
    }

    #[test]
    fn powershell_encode_roundtrip() {
        // Base64(UTF-16LE) 应能被 PowerShell 解析回原命令
        let cmd = "Write-Output 'hi'; $x = 1 + 2";
        let enc = powershell_encode(cmd);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&enc)
            .unwrap();
        let utf16: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let decoded = String::from_utf16(&utf16).unwrap();
        assert_eq!(decoded, cmd);
    }
}

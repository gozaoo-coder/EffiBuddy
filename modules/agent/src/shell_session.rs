//! 后台命令会话（shell session）：让 AI 启用一个持久化 shell 会话并持续交互。
//!
//! 设计目标（对齐用户需求）：
//! 1. **AI 启用命令会话后可进一步输入**：`start` 开启一个常驻的 `cmd`/`sh`
//!    子进程，`send` 向其中追加命令（写 stdin），`read` 读取增量输出，
//!    支持交互式命令（如 `y/n` 确认、长任务进度）。
//! 2. **不闪出到用户界面，后台静默运行**：Windows 用 `CREATE_NO_WINDOW`
//!    （0x08000000）创建进程，不分配控制台窗口；stdin/stdout/stderr 全部
//!    走管道，不继承父进程控制台。
//! 3. **前端实时查看工作状态**：每次输出/命令/退出都通过事件回调
//!    （`ShellSessionEvent`，Tauri 层转发为 `shell-session-event`）推送，
//!    前端在 main-content 底栏以「便签」形式展示每个会话。
//! 4. **短 ID 标记**：每个会话分配 4 位十六进制短 ID（如 `#a1b2`），
//!    agent 与前端都用它识别具体会话。
//!
//! 结构：
//! - [`ShellSessionManager`]：会话注册表 + 事件推送（agent 工具与 Tauri 命令共用）
//! - [`ShellSession`]：单个会话（子进程 + 输出缓冲 + 增量游标）
//! - 5 个 rig 工具：`shell_session_start` / `shell_session_send` /
//!   `shell_session_read` / `shell_session_list` / `shell_session_kill`
//!
//! 输出缓冲：stdout/stderr 行读取器持续把输出追加到 `SessionBuf`，
//! 同时 emit `Output` 事件供前端实时显示；`send`/`read` 用「安静期」启发式
//! （输出停止增长 N 毫秒视为命令已到提示符/结束）等待命令产出后返回增量。

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rig_core::tool::Tool;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::RwLock;

/// 命令发送默认超时（30s）
pub const DEFAULT_SEND_TIMEOUT_MS: u64 = 30_000;
/// `read` 默认等待时间（10s）
pub const DEFAULT_READ_TIMEOUT_MS: u64 = 10_000;
/// 安静期：输出停止增长 N 毫秒视为命令产出完毕
pub const SETTLE_MS: u64 = 300;
/// 输出缓冲上限（256 KiB），超出丢弃最早内容，防止长任务撑爆内存
const MAX_BUFFER_BYTES: usize = 256 * 1024;
/// 会话数上限，超出淘汰最久未活跃的会话
const MAX_SESSIONS: usize = 32;

/// Windows 隐藏窗口标志（CREATE_NO_WINDOW）：子进程不分配控制台窗口
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 会话事件：经 Tauri 层转发为前端 `shell-session-event`
#[derive(Debug, Clone, Serialize)]
pub struct ShellSessionEvent {
    /// 当前对话 conversation_id（前端据此过滤）
    pub conversation_id: String,
    /// 会话短 ID（如 `a1b2`）
    pub session_id: String,
    /// 事件类型
    pub kind: ShellSessionEventKind,
    /// 输出行 / 命令文本 / 错误信息 / 退出码
    pub content: String,
    pub is_error: bool,
}

/// 会话事件类型
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellSessionEventKind {
    /// 会话启动（content = 会话摘要）
    Started,
    /// AI 向会话发送了一条命令（content = 命令文本）
    Command,
    /// 进程输出一行（content = 该行）
    Output,
    /// 进程退出（content = 退出码）
    Exited,
    /// 错误（content = 错误信息）
    Error,
}

/// 会话列表条目（list 命令 / 前端恢复用）
#[derive(Debug, Clone, Serialize)]
pub struct ShellSessionInfo {
    pub id: String,
    pub name: String,
    pub shell: String,
    pub cwd: String,
    pub running: bool,
    pub last_command: String,
    pub last_active: u64,
}

/// 会话输出缓冲：累积输出 + 增量游标
struct SessionBuf {
    data: String,
    /// 已消费游标（字节偏移，`data[cursor..]` 为未读增量）
    cursor: usize,
}

impl SessionBuf {
    fn new() -> Self {
        Self {
            data: String::new(),
            cursor: 0,
        }
    }

    fn append(&mut self, text: &str) {
        self.data.push_str(text);
        if self.data.len() > MAX_BUFFER_BYTES {
            let drop = self.data.len() - MAX_BUFFER_BYTES;
            self.cursor = self.cursor.saturating_sub(drop);
            self.data.drain(..drop);
            if self.cursor > self.data.len() {
                self.cursor = 0;
            }
        }
    }

    /// 返回自游标起的全部增量并推进游标（不丢旧输出：旧输出保留在 data 中）
    fn take_delta(&mut self) -> String {
        let s = self.data[self.cursor..].to_string();
        self.cursor = self.data.len();
        s
    }

    fn len(&self) -> usize {
        self.data.len()
    }
}

/// 单个命令会话
pub struct ShellSession {
    pub id: String,
    pub name: String,
    pub cwd: PathBuf,
    /// "cmd"（Windows）或 "sh"（Unix）
    pub shell: &'static str,
    /// 进程句柄（waiter 取出 wait；kill 调用 kill()）
    child: tokio::sync::Mutex<Option<Child>>,
    /// stdin 管道（send 写入命令）
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    /// 输出缓冲（stdout/stderr 行读取器写入）
    buf: Arc<Mutex<SessionBuf>>,
    /// 进程是否仍在运行
    running: AtomicBool,
    /// 最近一条命令（前端便签摘要）
    last_command: Mutex<String>,
    /// 最近活跃时间（Unix 毫秒）
    last_active: AtomicU64,
}

/// 当前 Unix 毫秒时间戳
fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

impl ShellSession {
    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn touch(&self) {
        self.last_active.store(now_ms(), Ordering::SeqCst);
    }

    fn set_last_command(&self, cmd: &str) {
        *self.last_command.lock().unwrap() = cmd.to_string();
    }

    fn last_command(&self) -> String {
        self.last_command.lock().unwrap().clone()
    }

    fn info(&self) -> ShellSessionInfo {
        ShellSessionInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            shell: self.shell.to_string(),
            cwd: self.cwd.display().to_string(),
            running: self.is_running(),
            last_command: self.last_command(),
            last_active: self.last_active.load(Ordering::SeqCst),
        }
    }
}

/// 命令会话管理器：会话注册表 + 事件推送。
/// agent 工具与 Tauri 命令层共享同一份 Arc 实例。
pub struct ShellSessionManager {
    sessions: RwLock<HashMap<String, Arc<ShellSession>>>,
    /// 事件回调（Tauri 层：app_handle.emit("shell-session-event", ev)）
    emitter: Box<dyn Fn(&ShellSessionEvent) + Send + Sync>,
    /// 当前对话 conversation_id 句柄（事件过滤用）
    current_conversation_id: Arc<RwLock<Option<String>>>,
}

impl ShellSessionManager {
    pub fn new(
        emitter: Box<dyn Fn(&ShellSessionEvent) + Send + Sync>,
        current_conversation_id: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            emitter,
            current_conversation_id,
        }
    }

    /// 推送会话事件（conversation_id 取自共享句柄，前端据此过滤）
    async fn emit(
        &self,
        session_id: &str,
        kind: ShellSessionEventKind,
        content: String,
        is_error: bool,
    ) {
        let conversation_id = self
            .current_conversation_id
            .read()
            .await
            .clone()
            .unwrap_or_default();
        let ev = ShellSessionEvent {
            conversation_id,
            session_id: session_id.to_string(),
            kind,
            content,
            is_error,
        };
        (self.emitter)(&ev);
    }

    /// 生成不重复的 4 位十六进制短 ID
    async fn gen_id(&self) -> String {
        loop {
            let id: String = uuid::Uuid::new_v4().to_string().chars().take(4).collect();
            if !self.sessions.read().await.contains_key(&id) {
                return id;
            }
        }
    }

    /// 启动一个新命令会话，返回会话摘要（含短 ID）。
    pub async fn start(
        self: &Arc<Self>,
        name: Option<&str>,
        cwd: Option<&PathBuf>,
    ) -> Result<String, String> {
        // 会话数上限：淘汰最久未活跃
        {
            let mut sessions = self.sessions.write().await;
            if sessions.len() >= MAX_SESSIONS {
                let oldest = sessions
                    .iter()
                    .min_by_key(|(_, s)| s.last_active.load(Ordering::SeqCst))
                    .map(|(k, _)| k.clone());
                if let Some(k) = oldest {
                    if let Some(s) = sessions.remove(&k) {
                        s.running.store(false, Ordering::SeqCst);
                        self.emit(
                            &k,
                            ShellSessionEventKind::Exited,
                            "被清理（会话数达上限）".to_string(),
                            false,
                        )
                        .await;
                    }
                }
            }
        }

        let id = self.gen_id().await;
        let (shell, cmd_builder) = if cfg!(target_os = "windows") {
            let mut c = Command::new("cmd");
            // /Q 关闭命令回显；/K 执行后保持交互；prompt $G 把提示符设为 `>`
            c.arg("/Q").arg("/K").arg("prompt $G");
            ("cmd", c)
        } else {
            let c = Command::new("sh");
            ("sh", c)
        };
        let mut cmd = cmd_builder;

        // 工作区目录（若指定）
        let effective_cwd = match cwd {
            Some(p) if !p.as_os_str().is_empty() => p.clone(),
            _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        };
        cmd.current_dir(&effective_cwd);

        // 后台静默运行：管道化 stdio，不继承父进程控制台
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动 {shell} 会话失败 [{}]: {e}", effective_cwd.display()))?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdin = child.stdin.take();

        let default_name = format!("{shell} · {}", &effective_cwd.display().to_string().replace('\\', "/"));
        let session_name = name.map(str::trim).filter(|s| !s.is_empty()).unwrap_or(&default_name).to_string();

        let session = Arc::new(ShellSession {
            id: id.clone(),
            name: session_name.clone(),
            cwd: effective_cwd.clone(),
            shell,
            child: tokio::sync::Mutex::new(Some(child)),
            stdin: tokio::sync::Mutex::new(stdin),
            buf: Arc::new(Mutex::new(SessionBuf::new())),
            running: AtomicBool::new(true),
            last_command: Mutex::new(String::new()),
            last_active: AtomicU64::new(now_ms()),
        });

        // 启动 stdout / stderr 行读取器：追加缓冲 + 推送 Output 事件
        if let Some(out) = stdout {
            let s = Arc::clone(&session);
            let mgr = Arc::clone(self);
            let sid = id.clone();
            tokio::spawn(async move {
                read_pipe_lines(out, s.name.clone(), Arc::clone(&s.buf), mgr, sid).await;
            });
          }
          if let Some(err) = stderr {
            let s = Arc::clone(&session);
            let mgr = Arc::clone(self);
            let sid = id.clone();
            tokio::spawn(async move {
                read_pipe_lines(err, s.name.clone(), Arc::clone(&s.buf), mgr, sid).await;
            });
          }

          // 等待进程退出：更新 running 标记 + 推送 Exited 事件
        {
            let s = Arc::clone(&session);
            let mgr = Arc::clone(self);
            let sid = id.clone();
            tokio::spawn(async move {
                let child = s.child.lock().await.take();
                let Some(mut child) = child else { return };
                let status = child.wait().await;
                let code = status.ok().and_then(|st| st.code()).map(|c| c.to_string());
                s.running.store(false, Ordering::SeqCst);
                // 把剩余缓冲作为最终输出推送一次
                let tail = s.buf.lock().unwrap().take_delta();
                if !tail.trim().is_empty() {
                    mgr.emit(&sid, ShellSessionEventKind::Output, tail, false).await;
                }
                mgr.emit(
                    &sid,
                    ShellSessionEventKind::Exited,
                    code.unwrap_or_else(|| "unknown".to_string()),
                    false,
                )
                .await;
            });
          }

          // 注册到表 + 推送 Started 事件
        self.sessions.write().await.insert(id.clone(), Arc::clone(&session));
        self.emit(
            &id,
            ShellSessionEventKind::Started,
            format!("{shell} · {session_name} · {}({} 位)", effective_cwd.display(), id.len()),
            false,
        )
        .await;

        Ok(format!(
            "已启动后台命令会话 #{id}（{shell}，静默运行不会弹出窗口）\n\
             - 名称：{session_name}\n\
             - 工作区：{}\n\
             - 后续：向 #{id} 发送命令用 shell_session_send（session_id=\"{id}\", command=\"...\"）\n\
             - 读取输出用 shell_session_read（session_id=\"{id}\"）\n\
             - 查看全部会话用 shell_session_list，结束会话用 shell_session_kill（session_id=\"{id}\"）",
            effective_cwd.display()
        ))
    }

    /// 向指定会话发送命令（写 stdin），等待产出后返回增量输出。
    pub async fn send(
        &self,
        session_id: &str,
        command: &str,
        timeout_ms: Option<u64>,
    ) -> Result<String, String> {
        let session = self
            .sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| format!("会话 #{session_id} 不存在（shell_session_list 可查看）"))?;
        if !session.is_running() {
            return Err(format!("会话 #{session_id} 已退出，无法再发送命令（可用 shell_session_start 新建）"));
        }
        let command = command.trim();
        if command.is_empty() {
            return Err("命令不能为空".to_string());
        }

        // 1. 写入命令（平台对应换行：cmd 需 \r\n，sh 用 \n）
        {
            let mut stdin = session.stdin.lock().await;
            let s = stdin
                .as_mut()
                .ok_or_else(|| format!("会话 #{session_id} 的 stdin 已关闭"))?;
            let line_ending = if cfg!(target_os = "windows") { "\r\n" } else { "\n" };
                s.write_all(command.as_bytes())
                    .await
                    .map_err(|e| format!("写入命令到 #{session_id} 失败: {e}"))?;
                s.write_all(line_ending.as_bytes())
                    .await
                    .map_err(|e| format!("写入换行到 #{session_id} 失败: {e}"))?;
                s.flush()
                    .await
                    .map_err(|e| format!("刷新 #{session_id} stdin 失败: {e}"))?;
        }
        session.set_last_command(command);
        session.touch();
        // 推送 Command 事件（前端显示「AI 运行了 …」）
        self.emit(
            session_id,
            ShellSessionEventKind::Command,
            command.to_string(),
            false,
        )
        .await;
        session.touch();
        // 2. 等待产出（安静期启发式：输出停止增长视为命令已到提示符/结束）
        wait_for_settle(&session.buf, timeout_ms.unwrap_or(DEFAULT_SEND_TIMEOUT_MS)).await;
        let delta = session.buf.lock().unwrap().take_delta();
        let status_note = if session.is_running() {
            "（会话仍在后台运行，可继续 shell_session_send / shell_session_read）"
        } else {
            "（会话已退出）"
        };
        Ok(format!(
            "=== 会话 #{session_id} 命令输出 ===\n{delta}\n{status_note}"
        ))
    }

    /// 读取指定会话的增量输出（不发送命令），等待新输出或超时。
    /// 适合检查长任务进度 / 等待交互提示符。
    pub async fn read(
        &self,
        session_id: &str,
        timeout_ms: Option<u64>,
    ) -> Result<String, String> {
        let session = self
            .sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| format!("会话 #{session_id} 不存在（shell_session_list 可查看）"))?;
        if !session.is_running() {
            // 已退出：返回剩余缓冲
            let tail = session.buf.lock().unwrap().take_delta();
            return Ok(format!(
                "=== 会话 #{session_id}（已退出）剩余输出 ===\n{tail}"
            ));
        }
        wait_for_settle(&session.buf, timeout_ms.unwrap_or(DEFAULT_READ_TIMEOUT_MS)).await;
        session.touch();
        let delta = session.buf.lock().unwrap().take_delta();
        if delta.trim().is_empty() {
            Ok(format!("（会话 #{session_id} 暂无新输出，进程仍在运行）"))
        } else {
            Ok(format!(
                "=== 会话 #{session_id} 新输出 ===\n{delta}"
            ))
        }
    }

    /// 列出全部会话（按最近活跃倒序）
    pub async fn list(&self) -> Vec<ShellSessionInfo> {
        let sessions = self.sessions.read().await;
        let mut items: Vec<ShellSessionInfo> = sessions
            .values()
            .map(|s| s.info())
            .collect();
        items.sort_by_key(|i| std::cmp::Reverse(i.last_active));
        items
    }

    /// 结束指定会话（终止进程）
    pub async fn kill(&self, session_id: &str) -> Result<String, String> {
        let session = self
            .sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| format!("会话 #{session_id} 不存在"))?;
        let mut guard = session.child.lock().await;
        match guard.as_mut() {
            Some(child) => {
                let _ = child.kill().await;
                // kill 后由 waiter 统一推送 Exited
                Ok(format!("已请求结束会话 #{session_id}（{}）", session.name))
            }
            None => Ok(format!(
                "会话 #{session_id} 已结束（{}）",
                session.name
            )),
        }
    }
}

/// 读取管道字节流，按行分割（UTF-8 lossy，兼容 Windows GBK 输出），
/// 追加到缓冲并推送 Output 事件。
async fn read_pipe_lines<R: AsyncRead + Unpin>(
    reader: R,
    session_name: String,
    buf: Arc<Mutex<SessionBuf>>,
    mgr: Arc<ShellSessionManager>,
    emit_sid: String,
) {
    let mut reader = BufReader::new(reader);
    let mut line: Vec<u8> = Vec::with_capacity(256);
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                for &b in &chunk[..n] {
                    if b == b'\n' {
                        let text = String::from_utf8_lossy(&line).into_owned();
                        line.clear();
                        if !text.is_empty() {
                            buf.lock().unwrap().append(&text);
                            mgr.emit(&emit_sid, ShellSessionEventKind::Output, text, false)
                                .await;
                        }
                    } else {
                        line.push(b);
                    }
                }
            }
            Err(e) => {
                mgr.emit(
                    &emit_sid,
                    ShellSessionEventKind::Error,
                    format!("读取 {session_name} 输出失败: {e}"),
                    true,
                )
                .await;
                break;
            }
        }
    }
    // 末尾无换行的残行
    if !line.is_empty() {
        let text = String::from_utf8_lossy(&line).into_owned();
        buf.lock().unwrap().append(&text);
        mgr.emit(&emit_sid, ShellSessionEventKind::Output, text, false).await;
    }
}

/// 安静期等待：输出停止增长 `SETTLE_MS` 毫秒视为命令产出完毕；否则等满超时。
async fn wait_for_settle(buf: &Arc<Mutex<SessionBuf>>, timeout_ms: u64) {
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms.max(1));
    let settle = tokio::time::Duration::from_millis(SETTLE_MS);
    let mut last_len = buf.lock().unwrap().len();
    let mut last_change = tokio::time::Instant::now();
    loop {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let len = buf.lock().unwrap().len();
        if len > last_len {
            last_len = len;
            last_change = tokio::time::Instant::now();
        } else if tokio::time::Instant::now().duration_since(last_change) >= settle {
            break;
        }
    }
}

// =========================================================
// rig 工具
// =========================================================

/// shell_session_start 参数
#[derive(Deserialize)]
pub struct ShellSessionStartArgs {
    /// 会话显示名（便签标题），缺省用 shell 类型 + 工作区
    #[serde(default)]
    pub name: Option<String>,
    /// 工作区目录（绝对路径），缺省用进程当前目录
    #[serde(default)]
    pub cwd: Option<String>,
}

/// shell_session_send 参数
#[derive(Deserialize)]
pub struct ShellSessionSendArgs {
    /// 会话短 ID（如 `a1b2`）
    pub session_id: String,
    /// 要执行的命令（会追加到会话，可携带交互输入如 `y`）
    pub command: String,
    /// 等待输出的超时毫秒数，默认 30000
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// shell_session_read 参数
#[derive(Deserialize)]
pub struct ShellSessionReadArgs {
    /// 会话短 ID（如 `a1b2`）
    pub session_id: String,
    /// 等待新输出的毫秒数，默认 10000
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// shell_session_kill 参数
#[derive(Deserialize)]
pub struct ShellSessionKillArgs {
    /// 会话短 ID（如 `a1b2`）
    pub session_id: String,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("shell session error: {0}")]
pub struct ShellSessionError(String);

/// 启动后台命令会话
pub struct ShellSessionStartTool {
    manager: Arc<ShellSessionManager>,
}

impl ShellSessionStartTool {
    pub fn new(manager: Arc<ShellSessionManager>) -> Self {
        Self { manager }
    }
}

impl Tool for ShellSessionStartTool {
    const NAME: &'static str = "shell_session_start";

    type Error = ShellSessionError;
    type Args = ShellSessionStartArgs;
    type Output = String;

    fn description(&self) -> String {
        "启用一个后台命令会话（常驻 cmd/sh 子进程，静默运行不会弹出窗口）。\
         返回带短 ID（如 #a1b2）的会话。之后用 shell_session_send 向该会话追加命令、\
         用 shell_session_read 读取输出，支持多步 / 交互式命令。\
         适合需要多次操作、保持工作目录或长任务场景；一次性简单命令仍可用 shell 工具。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "会话显示名（便签标题），缺省用 shell 类型 + 工作区"
                },
                "cwd": {
                    "type": "string",
                    "description": "工作区目录（绝对路径），缺省用进程当前目录"
                }
            }
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let cwd = args.cwd.as_deref().map(PathBuf::from);
        self.manager
            .start(args.name.as_deref(), cwd.as_ref())
            .await
            .map_err(ShellSessionError)
    }
}

/// 向已启动的命令会话发送命令（可追加输入）
pub struct ShellSessionSendTool {
    manager: Arc<ShellSessionManager>,
}

impl ShellSessionSendTool {
    pub fn new(manager: Arc<ShellSessionManager>) -> Self {
        Self { manager }
    }
}

impl Tool for ShellSessionSendTool {
    const NAME: &'static str = "shell_session_send";

    type Error = ShellSessionError;
    type Args = ShellSessionSendArgs;
    type Output = String;

    fn description(&self) -> String {
        "向已启动的命令会话发送一条命令或交互输入（写 stdin），等待产出后返回增量输出。\
         session_id 取 shell_session_start 返回的短 ID。\
         支持在会话中继续执行（保持工作目录）、向交互式提示符输入（如 y/n）、\
         以及多步操作。命令默认超时 30s；长任务可缩短 timeout_ms 快速返回，\
         再用 shell_session_read 轮询进度。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "会话短 ID（如 a1b2，来自 shell_session_start）"
                },
                "command": {
                    "type": "string",
                    "description": "要执行的命令或交互输入（如 `cd src`、`npm run build`、`y`）"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "等待输出的超时毫秒数，默认 30000",
                    "default": DEFAULT_SEND_TIMEOUT_MS
                }
            },
            "required": ["session_id", "command"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.manager
            .send(&args.session_id, &args.command, args.timeout_ms)
            .await
            .map_err(ShellSessionError)
    }
}

/// 读取命令会话的增量输出（不发送命令）
pub struct ShellSessionReadTool {
    manager: Arc<ShellSessionManager>,
}

impl ShellSessionReadTool {
    pub fn new(manager: Arc<ShellSessionManager>) -> Self {
        Self { manager }
    }
}

impl Tool for ShellSessionReadTool {
    const NAME: &'static str = "shell_session_read";

    type Error = ShellSessionError;
    type Args = ShellSessionReadArgs;
    type Output = String;

    fn description(&self) -> String {
        "读取命令会话自上次读取以来的新输出（不发送命令）。\
         用于检查长任务的进度、等待交互式提示符，或命令在后台运行时轮询结果。\
         返回空表示暂无新输出（进程仍在运行）。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "会话短 ID（如 a1b2）"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "等待新输出的毫秒数，默认 10000",
                    "default": DEFAULT_READ_TIMEOUT_MS
                }
            },
            "required": ["session_id"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.manager
            .read(&args.session_id, args.timeout_ms)
            .await
            .map_err(ShellSessionError)
    }
}

/// 列出全部命令会话
pub struct ShellSessionListTool {
    manager: Arc<ShellSessionManager>,
}

impl ShellSessionListTool {
    pub fn new(manager: Arc<ShellSessionManager>) -> Self {
        Self { manager }
    }
}

impl Tool for ShellSessionListTool {
    const NAME: &'static str = "shell_session_list";

    type Error = ShellSessionError;
    type Args = (); // 无参数
    type Output = String;

    fn description(&self) -> String {
        "列出当前全部命令会话（短 ID、名称、shell 类型、工作区、是否运行、最近命令）。\
         用于回顾有哪些会话、确认某会话 ID 是否仍存活，或找会话来发送命令。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let list = self.manager.list().await;
        if list.is_empty() {
            return Ok("（当前没有命令会话，可用 shell_session_start 启用一个）".to_string());
        }
        let mut out = String::from("当前命令会话：\n");
        for s in &list {
            out.push_str(&format!(
                "- #{} · {} · {}({}) · {}{}\n",
                s.id,
                s.name,
                s.shell,
                s.cwd,
                if s.running { "● 运行中" } else { "○ 已退出" },
                if s.last_command.is_empty() {
                    String::new()
                } else {
                    format!(" · 最近: {}", s.last_command)
                }
            ));
        }
        Ok(out)
    }
}

/// 结束指定命令会话
pub struct ShellSessionKillTool {
    manager: Arc<ShellSessionManager>,
}

impl ShellSessionKillTool {
    pub fn new(manager: Arc<ShellSessionManager>) -> Self {
        Self { manager }
    }
}

impl Tool for ShellSessionKillTool {
    const NAME: &'static str = "shell_session_kill";

    type Error = ShellSessionError;
    type Args = ShellSessionKillArgs;
    type Output = String;

    fn description(&self) -> String {
        "结束（终止）一个命令会话。参数为 shell_session_start 返回的短 ID。\
         结束后该会话不再接收命令；其剩余输出会先推送一次。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "会话短 ID（如 a1b2）"
                }
            },
            "required": ["session_id"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.manager
            .kill(&args.session_id)
            .await
            .map_err(ShellSessionError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 会话管理器：无操作 emitter + 固定 conversation_id
    fn test_manager() -> Arc<ShellSessionManager> {
        let conv: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(Some("test-conv".into())));
        Arc::new(ShellSessionManager::new(Box::new(|_| {}), conv))
    }

    /// 启动 → 发命令 → 读到输出 → kill 全链路（Windows cmd / Unix sh 都覆盖）
    #[tokio::test]
    async fn session_full_flow() {
        let mgr = test_manager();
        let summary = mgr.start(Some("flow-test"), None).await.expect("start");
        // 从摘要解析短 ID（#a1b2 的 4 位十六进制）
        let sid = summary
            .split('#')
            .nth(1)
            .and_then(|s| s.get(..4).map(|x| x.to_string()))
            .expect("session id");

        // 等待 shell 提示符就绪（首次启动有启动横幅输出）
        let _ = mgr.read(&sid, Some(1500)).await;

        let probe = if cfg!(target_os = "windows") { "echo effisuite-probe" } else { "echo effisuite-probe" };
        let out = mgr.send(&sid, probe, Some(10_000)).await.expect("send");
        assert!(
            out.contains("effisuite-probe"),
            "send 输出应包含命令回显/结果，实际: {out:?}"
        );

        // 再发一条验证会话可继续（多步输入）
        let out2 = mgr.send(&sid, "echo second-probe", Some(10_000)).await.expect("send2");
        assert!(out2.contains("second-probe"), "第二次 send 应成功，实际: {out2:?}");

        // list 应包含该会话
        let list = mgr.list().await;
        assert!(list.iter().any(|i| i.id == sid), "list 应包含会话 {sid}");

        // kill 结束会话
        mgr.kill(&sid).await.expect("kill");
    }

    /// 不存在的会话应返回友好错误
    #[tokio::test]
    async fn unknown_session_errors() {
        let mgr = test_manager();
        let e = mgr.send("zzzz", "echo hi", None).await.unwrap_err();
        assert!(e.contains("不存在"), "err: {e}");
    }
}

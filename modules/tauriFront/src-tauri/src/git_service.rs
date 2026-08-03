//! git 上下文版本管理核心服务（调用系统 git CLI，不引入 git2 crate）
//!
//! 设计要点：
//! - 全部 git 操作通过 `std::process::Command` 调用系统 git CLI
//!   （依赖外部 git，≥2.23 以支持 `git restore` / `git branch --show-current`）
//! - 仓库范围：
//!   - `chat`：`<appdata>/effisuite/conversations`（聊天记录目录本身）
//!   - `workspace`：会话级 working_dir（未设置时由命令层报错）
//! - 安全边界（硬性约束，不因调用方而放宽）：
//!   1. **绝不 `reset --hard`**：撤回最近提交用 `reset --soft`（撤销提交但改动保留
//!      在暂存区，零丢失）；撤回到某提交用 `git restore --source=<commit> --staged
//!      --worktree`（把工作区+暂存区文件恢复到该提交状态，不动分支指针、不丢历史）。
//!   2. **回溯**用 `git checkout <commit>`（detached HEAD），前置校验无未提交改动；
//!      若目标仓库是 EffiSuite 项目自身仓库（`is_effisuite_project`）则直接拒绝。
//!   3. **工作区复用已有仓库**时，`add -A .` 的 pathspec 限定在当前目录，
//!      不会误暂存仓库其他目录的文件（满足「只 add/commit 指定范围内的文件」）。
//!   4. 提交身份：优先使用仓库/全局已有配置；未配置时仅对本提交注入
//!      `-c user.name=EffiSuite -c user.email=effisuite@local`，不写任何 config 文件。
//! - 单元测试用临时目录跑真实 git 流程（init/add/commit/branch/checkout），
//!   不污染 EffiSuite 仓库。

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

/// 单个历史提交信息（history 列表项）
#[derive(Debug, Clone, Serialize)]
pub struct GitCommitInfo {
    pub hash: String,
    pub message: String,
    /// 提交时间（Unix 秒）
    pub timestamp: i64,
}

/// 仓库状态快照（status / history / init / branch / save / revert / checkout 统一返回）
#[derive(Debug, Clone, Serialize)]
pub struct GitRepoInfo {
    /// 目标目录（聊天记录目录或工作区目录）
    pub path: String,
    /// 实际 git 仓库根目录（未初始化时为空字符串）
    pub repo_root: String,
    pub is_repo: bool,
    /// 当前 HEAD 短哈希（未初始化时为空字符串）
    pub head_hash: String,
    /// 当前分支（detached HEAD 时为 None）
    pub branch: Option<String>,
    pub detached: bool,
    /// 目标仓库是否为 EffiSuite 项目自身仓库（该仓库禁止撤回/回溯等危险操作）
    pub is_effisuite_project: bool,
    /// 是否有未提交改动
    pub dirty: bool,
    /// `git status --porcelain` 短状态行（如 ` M file` / `?? new`）
    pub changed: Vec<String>,
    /// 历史提交（最新在前，最多 50 条）
    pub commits: Vec<GitCommitInfo>,
}

/// 保存（commit）结果
#[derive(Debug, Clone, Serialize)]
pub struct GitSaveResult {
    pub committed: bool,
    pub hash: Option<String>,
    pub message: String,
}

// ==================== 基础工具 ====================

/// 执行 git 命令：成功返回 stdout（trim），失败返回友好错误（含 stderr）
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| {
            format!("无法调用 git：{e}（请确认系统已安装 git 并加入 PATH）")
        })?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(if detail.is_empty() {
            format!("git 命令失败：{}", args.first().copied().unwrap_or(""))
        } else {
            detail
        });
    }
    Ok(stdout)
}

/// EffiSuite 项目根目录（由编译时 `CARGO_MANIFEST_DIR` 上溯三级得到：
/// `.../modules/tauriFront/src-tauri` → 项目根）
pub fn effisuite_project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 目录（或其祖先）是否已是 git 仓库
fn is_repo(dir: &Path) -> bool {
    git(dir, &["rev-parse", "--git-dir"]).is_ok()
}

/// 目标目录是否落在 EffiSuite 项目自身仓库内（含 working_dir 为项目子目录的情况）
fn is_effisuite_project(dir: &Path) -> bool {
    let Ok(root) = git(dir, &["rev-parse", "--show-toplevel"]) else {
        return false;
    };
    let root = PathBuf::from(root);
    let proj = effisuite_project_root();
    match (std::fs::canonicalize(&root), std::fs::canonicalize(&proj)) {
        (Ok(a), Ok(b)) => a == b,
        _ => root == proj,
    }
}

/// 提交身份覆盖参数：仓库/全局已配置 user.name + user.email 时返回空列表，
/// 否则返回 `-c user.name=EffiSuite -c user.email=effisuite@local`
/// （仅作用于本次 commit 调用，不持久化到任何 config 文件）。
fn identity_args(dir: &Path) -> Vec<String> {
    let name = git(dir, &["config", "user.name"]).unwrap_or_default();
    let email = git(dir, &["config", "user.email"]).unwrap_or_default();
    if name.is_empty() || email.is_empty() {
        vec![
            "-c".into(),
            "user.name=EffiSuite".into(),
            "-c".into(),
            "user.email=effisuite@local".into(),
        ]
    } else {
        vec![]
    }
}

/// 执行一次带身份注入的 commit
fn commit(dir: &Path, message: &str) -> Result<(), String> {
    let mut args = identity_args(dir);
    args.push("commit".into());
    args.push("-m".into());
    args.push(message.to_string());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    git(dir, &refs)?;
    Ok(())
}

// ==================== 对外操作 ====================

/// 确保目录是 git 仓库：
/// 1. 未初始化则 `git init`（不写全局配置）；
/// 2. 仓库尚无提交且目录内有内容时创建初始快照「初始化上下文快照」。
///
/// 对已处于其他 git 仓库内的目录（如工作区在用户项目里）不会重复 init。
pub fn ensure_repo(dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(format!("目录不存在：{}", dir.display()));
    }
    if !is_repo(dir) {
        git(dir, &["init"])?;
    }
    // 无提交时创建初始快照（仅当有内容可提交）
    if git(dir, &["rev-parse", "--verify", "HEAD"]).is_err() {
        git(dir, &["add", "-A", "."]).ok();
        let has_changes =
            !git(dir, &["status", "--porcelain"]).unwrap_or_default().is_empty();
        if has_changes {
            commit(dir, "初始化上下文快照")?;
        }
    }
    Ok(())
}

/// 读取历史提交列表（最新在前，最多 50 条）
pub fn history(dir: &Path) -> Result<Vec<GitCommitInfo>, String> {
    if !is_repo(dir) {
        return Ok(Vec::new());
    }
    let out = git(
        dir,
        &["log", "-n", "50", "--pretty=format:%h%x1f%s%x1f%ct"],
    )?;
    if out.is_empty() {
        return Ok(Vec::new());
    }
    let mut commits = Vec::with_capacity(16);
    for line in out.lines() {
        let mut parts = line.splitn(3, '\x1f');
        let hash = parts.next().unwrap_or("").to_string();
        let message = parts.next().unwrap_or("").to_string();
        let timestamp = parts
            .next()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        commits.push(GitCommitInfo {
            hash,
            message,
            timestamp,
        });
    }
    Ok(commits)
}

/// 仓库状态快照（含历史；未初始化时返回 `is_repo=false`，不自动 init）
pub fn status(dir: &Path) -> Result<GitRepoInfo, String> {
    let repo = is_repo(dir);
    let repo_root = if repo {
        git(dir, &["rev-parse", "--show-toplevel"]).unwrap_or_default()
    } else {
        String::new()
    };
    let head_hash = if repo {
        git(dir, &["rev-parse", "--short", "HEAD"]).unwrap_or_default()
    } else {
        String::new()
    };
    let (branch, detached) = if repo {
        let b = git(dir, &["branch", "--show-current"]).unwrap_or_default();
        if b.is_empty() {
            (None, true)
        } else {
            (Some(b), false)
        }
    } else {
        (None, false)
    };
    let changed = if repo {
        git(dir, &["status", "--porcelain"])
            .unwrap_or_default()
            .lines()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let dirty = !changed.is_empty();
    let commits = if repo { history(dir).unwrap_or_default() } else { Vec::new() };
    Ok(GitRepoInfo {
        path: dir.display().to_string(),
        repo_root,
        is_repo: repo,
        head_hash,
        branch,
        detached,
        is_effisuite_project: repo && is_effisuite_project(dir),
        dirty,
        changed,
        commits,
    })
}

/// 创建并切换到新分支（`git checkout -b`；分支名合法性由 git 校验并透传错误）
pub fn create_branch(dir: &Path, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("分支名不能为空".to_string());
    }
    git(dir, &["checkout", "-b", name])?;
    Ok(())
}

/// 保存当前状态：`git add -A .`（paths 限定在当前目录）+ commit。
/// 无任何改动时返回 `committed=false`，不产生空提交。
pub fn save(dir: &Path, message: &str) -> Result<GitSaveResult, String> {
    let message = message.trim();
    let msg = if message.is_empty() {
        "保存上下文快照".to_string()
    } else {
        message.to_string()
    };
    git(dir, &["add", "-A", "."])?;
    let changed = git(dir, &["status", "--porcelain"]).unwrap_or_default();
    if changed.is_empty() {
        return Ok(GitSaveResult {
            committed: false,
            hash: None,
            message: "没有可提交的更改".to_string(),
        });
    }
    commit(dir, &msg)?;
    let hash = git(dir, &["rev-parse", "--short", "HEAD"]).ok();
    Ok(GitSaveResult {
        committed: true,
        hash,
        message: format!("已提交：{msg}"),
    })
}

/// 撤回：
/// - `commit=None`：撤销最近一次提交（`reset --soft HEAD~1`，改动保留在暂存区，零丢失）；
/// - `commit=Some(hash)`：把工作区 + 暂存区文件恢复到该提交的状态
///   （`git restore --source=<hash> --staged --worktree`，不动分支指针、不丢历史，
///   不影响未跟踪文件）。
///
/// EffiSuite 项目自身仓库直接拒绝，保护项目代码。
pub fn revert(dir: &Path, commit: Option<String>) -> Result<String, String> {
    if is_effisuite_project(dir) {
        return Err("目标目录是 EffiSuite 项目自身仓库，为保护项目代码禁止撤回操作".to_string());
    }
    match commit {
        None => {
            let head = git(dir, &["rev-parse", "--short", "HEAD"])
                .map_err(|_| "仓库尚无提交，无法撤回".to_string())?;
            git(dir, &["reset", "--soft", "HEAD~1"])?;
            Ok(format!(
                "已撤销最近一次提交（{head}），改动保留在暂存区，可重新编辑后再次「保存」"
            ))
        }
        Some(hash) => {
            let hash = hash.trim();
            if hash.is_empty() {
                return Err("请提供要撤回到的提交".to_string());
            }
            let current = git(dir, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();
            if current == hash {
                return Ok("当前已在该提交，无需撤回".to_string());
            }
            git(dir, &["rev-parse", "--verify", &format!("{hash}^{{commit}}")])
                .map_err(|_| format!("提交 {hash} 不存在"))?;
            git(
                dir,
                &["restore", "--source", hash, "--staged", "--worktree", "--", "."],
            )?;
            Ok(format!(
                "已将文件恢复到提交 {hash} 的状态（未改动分支指针），请确认后点击「保存」固化"
            ))
        }
    }
}

/// 回溯：检出到指定提交（detached HEAD）。
/// 前置校验无未提交改动（避免覆盖当前工作），且拒绝 EffiSuite 项目自身仓库。
pub fn checkout(dir: &Path, commit: &str) -> Result<String, String> {
    let commit = commit.trim();
    if commit.is_empty() {
        return Err("请提供要回溯到的提交".to_string());
    }
    if is_effisuite_project(dir) {
        return Err("目标目录是 EffiSuite 项目自身仓库，禁止回溯检出操作".to_string());
    }
    let info = status(dir)?;
    if info.dirty {
        return Err("有未提交的修改，请先「保存」再回溯（避免丢失当前工作）".to_string());
    }
    git(dir, &["rev-parse", "--verify", &format!("{commit}^{{commit}}")])
        .map_err(|_| format!("提交 {commit} 不存在"))?;
    git(dir, &["checkout", commit])?;
    Ok(format!(
        "已回溯到提交 {commit}（当前处于 detached HEAD，可「开分支」从该点继续）"
    ))
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "effisuite-git-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn init_save_branch_history_checkout_flow() {
        let dir = tmp_dir();

        // 空目录 init → 仓库就绪但无提交
        ensure_repo(&dir).unwrap();
        let info = status(&dir).unwrap();
        assert!(info.is_repo);
        assert!(info.commits.is_empty());

        // 写文件并保存 → 产生 1 条提交
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        let r = save(&dir, "第一次保存").unwrap();
        assert!(r.committed);
        assert!(r.hash.is_some());
        let info = status(&dir).unwrap();
        assert_eq!(info.commits.len(), 1);
        assert!(!info.dirty);

        // 开分支并切换
        create_branch(&dir, "feature").unwrap();
        let info = status(&dir).unwrap();
        assert_eq!(info.branch.as_deref(), Some("feature"));

        // 修改 + 保存 → 2 条提交
        std::fs::write(dir.join("a.txt"), "world").unwrap();
        save(&dir, "第二次保存").unwrap();
        let info = status(&dir).unwrap();
        assert_eq!(info.commits.len(), 2);

        // 撤回最近一次提交（soft）→ 改动保留在暂存区，提交数回到 1
        let msg = revert(&dir, None).unwrap();
        assert!(msg.contains("撤销"));
        let info = status(&dir).unwrap();
        assert_eq!(info.commits.len(), 1);
        assert!(info.dirty);

        // 重新保存后再回溯到第一条提交 → detached HEAD
        save(&dir, "重新保存").unwrap();
        let first = info.commits[0].hash.clone();
        let m = checkout(&dir, &first).unwrap();
        assert!(m.contains("回溯"));
        let info = status(&dir).unwrap();
        assert!(info.detached);
        assert_eq!(info.head_hash, first);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn revert_to_commit_restores_files_without_moving_branch() {
        let dir = tmp_dir();
        ensure_repo(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "v1").unwrap();
        save(&dir, "v1").unwrap();
        let first = status(&dir).unwrap().commits[0].hash.clone();

        std::fs::write(dir.join("a.txt"), "v2").unwrap();
        save(&dir, "v2").unwrap();

        // 撤回到 v1：文件内容恢复为 v1，分支指针不变（仍在原分支）
        let branch_before = status(&dir).unwrap().branch.unwrap();
        let msg = revert(&dir, Some(first.clone())).unwrap();
        assert!(msg.contains(&first));
        let content = std::fs::read_to_string(dir.join("a.txt")).unwrap();
        assert_eq!(content, "v1");
        let info = status(&dir).unwrap();
        assert_eq!(info.branch.as_deref(), Some(branch_before.as_str()));
        assert!(!info.detached);
        // 历史仍完整（2 条）
        assert_eq!(info.commits.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reject_dangerous_ops_on_effisuite_project() {
        // 用 EffiSuite 项目根（本仓库自身）构造临时场景：
        // 仅验证 is_effisuite_project 判定与 revert 拒绝，不做任何写入。
        let proj = effisuite_project_root();
        if is_repo(&proj) {
            assert!(is_effisuite_project(&proj));
            let err = revert(&proj, None).unwrap_err();
            assert!(err.contains("禁止撤回"));
            let err = checkout(&proj, "HEAD").unwrap_err();
            assert!(err.contains("禁止回溯"));
        }
    }
}

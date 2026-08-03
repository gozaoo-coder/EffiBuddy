//! git 风格会话版本控制 —— 版本仓库存储
//!
//! 每会话一个 `repo.json`（位于 `<conversations_root>/.versions/<id>.json`），
//! 内部为内容寻址的提交对象库 + 消息池 + 引用表 + HEAD。与
//! [`crate::storage::ConversationStore`] 的并发模型一致：每会话独立锁，
//! 读-改-写操作仅阻塞同一会话。
//!
//! 关键约定：
//! - **工作区 = HEAD 提交快照**：会话 JSON 文件的 `messages` 始终等于当前
//!   HEAD 指向提交的消息快照；`append_message` 每次落盘后自动追加一个
//!   `Append` 提交，保证每个消息点都有可回溯的版本。
//! - **不可变提交**：`commits` 永不删除；破坏性操作（回溯/撤回/检出）前先写
//!   `chkpt-*` 检查点引用，任何历史状态都可随时找回（reflog 语义）。
//! - **内容寻址去重**：消息本体在 `messages` 池中按 id 存一份，提交只保存
//!   有序 id 快照，避免每个版本全量复制（git 的 blob/tree 思路）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex as StdMutex;

use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::{CoreError, Message, Result};

use super::types::{
    Commit, CommitKind, CommitSummary, RefKind, RefSummary, VersionList, VersionOpResult,
    VersionRepo,
};

/// 版本仓库存储：管理全部会话的 git 风格历史。
///
/// `root` 为 `<conversations_root>/.versions`；每会话一个 JSON 文件。
/// 可廉价 clone（内部 Arc 共享）。
#[derive(Clone)]
pub struct VersionStore {
    root: PathBuf,
    /// 每会话独立锁表（与 ConversationStore 一致：外层 StdMutex 短暂持表，
    /// 内层 tokio::Mutex 跨 await）
    locks: std::sync::Arc<StdMutex<HashMap<String, std::sync::Arc<Mutex<()>>>>>,
}

impl VersionStore {
    /// 创建版本存储，root 不存在时自动创建
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        Ok(Self {
            root,
            locks: std::sync::Arc::new(StdMutex::new(HashMap::new())),
        })
    }

    /// 指定会话的独立锁
    #[inline]
    fn conv_lock(&self, id: &str) -> std::sync::Arc<Mutex<()>> {
        let mut map = self.locks.lock().unwrap();
        map.entry(id.to_string())
            .or_insert_with(|| std::sync::Arc::new(Mutex::new(())))
            .clone()
    }

    /// 会话版本仓库文件路径：`<root>/<safe_id>.json`
    #[inline]
    fn repo_path(&self, id: &str) -> PathBuf {
        let safe = Path::new(id)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(id));
        self.root.join(safe).with_extension("json")
    }

    /// 加载仓库；不存在或空文件时返回默认空仓库
    async fn load_repo(&self, id: &str) -> Result<VersionRepo> {
        let path = self.repo_path(id);
        if !path.exists() {
            return Ok(VersionRepo::default());
        }
        let bytes = tokio::fs::read(&path).await.map_err(CoreError::Io)?;
        if bytes.is_empty() {
            return Ok(VersionRepo::default());
        }
        serde_json::from_slice(&bytes).map_err(CoreError::Serde)
    }

    /// 原子写入仓库（tmp + rename）
    async fn save_repo(&self, id: &str, repo: &VersionRepo) -> Result<()> {
        let path = self.repo_path(id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(CoreError::Io)?;
        }
        let bytes = serde_json::to_vec_pretty(repo).map_err(CoreError::Serde)?;
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, &bytes).await.map_err(CoreError::Io)?;
        tokio::fs::rename(&tmp, &path).await.map_err(CoreError::Io)?;
        Ok(())
    }

    // ---------- 提交对象 ----------

    /// 计算提交内容寻址 hash（SHA-256 hex 前缀）。
    /// 输入为「父提交 + 类型 + 快照消息 id 序列」的确定性编码：同一父提交下
    /// 相同快照幂等（重试不产生重复提交）；追加新消息因 id 唯一必然产生新 hash。
    fn hash_of(parent: Option<&str>, kind: CommitKind, message_ids: &[String]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(parent.unwrap_or("").as_bytes());
        hasher.update([0u8]);
        hasher.update(
            match kind {
                CommitKind::Append => "append",
                CommitKind::Branch => "branch",
                CommitKind::TempSave => "temp",
                CommitKind::Rollback => "rollback",
                CommitKind::Undo => "undo",
            }
            .as_bytes(),
        );
        hasher.update([0u8]);
        for id in message_ids {
            hasher.update(id.as_bytes());
            hasher.update([0u8]);
        }
        let digest = hasher.finalize();
        format!("{:x}", digest)
    }

    /// 从消息列表派生快照 id 列表（仅追加模式）
    fn ids_of(messages: &[Message]) -> Vec<String> {
        messages.iter().map(|m| m.id.clone()).collect()
    }

    /// 把消息写入池（按 id 去重，幂等）
    fn upsert_pool(repo: &mut VersionRepo, messages: &[Message]) {
        for m in messages {
            repo.messages.entry(m.id.clone()).or_insert_with(|| m.clone());
        }
    }

    /// 生成一个不冲突的引用名：`<prefix>-<ts>`，同毫秒冲突时追加序号
    fn unique_ref_name(repo: &VersionRepo, prefix: &str, now: u64) -> String {
        let base = format!("{prefix}-{now}");
        if !repo.refs.contains_key(&base) {
            return base;
        }
        let mut i = 1u64;
        loop {
            let cand = format!("{base}-{i}");
            if !repo.refs.contains_key(&cand) {
                return cand;
            }
            i += 1;
        }
    }

    /// 核心：创建提交并移动 HEAD 引用（幂等：同一快照 + 父提交 → 复用已有提交）。
    /// `repo.head` 为空时初始化默认分支 `main`。
    fn commit_locked(
        repo: &mut VersionRepo,
        messages: &[Message],
        kind: CommitKind,
        note: String,
        now: u64,
    ) -> Commit {
        if repo.head.is_empty() {
            repo.head = "main".to_string();
        }
        let head = repo.head.clone();
        let parent = repo.refs.get(&head).cloned();
        let message_ids = Self::ids_of(messages);
        let head_message_id = message_ids.last().cloned().unwrap_or_default();
        let hash = Self::hash_of(parent.as_deref(), kind, &message_ids);
        // 同一快照已提交过（幂等）则直接复用，避免重复提交
        if let Some(existing) = repo.commits.get(&hash) {
            return existing.clone();
        }
        let commit = Commit {
            hash: hash.clone(),
            parent,
            kind,
            note,
            created_at: now,
            head_message_id,
            message_ids,
            message_count: messages.len(),
        };
        Self::upsert_pool(repo, messages);
        repo.commits.insert(hash.clone(), commit.clone());
        repo.refs.insert(head, hash.clone());
        commit
    }

    /// 追加消息后提交（由 ConversationStore::append_message 自动调用）
    pub async fn commit_append(
        &self,
        conv_id: &str,
        messages: &[Message],
        now: u64,
    ) -> Result<Commit> {
        let lock = self.conv_lock(conv_id);
        let _guard = lock.lock().await;
        let mut repo = self.load_repo(conv_id).await?;
        let commit = Self::commit_locked(&mut repo, messages, CommitKind::Append, "新消息".into(), now);
        self.save_repo(conv_id, &repo).await?;
        Ok(commit)
    }

    // ---------- 按消息定位提交 ----------

    /// 在当前 HEAD 链上查找"以该消息为最后一条"的提交（即该消息追加时生成的提交）。
    /// 按父链向上回溯；找不到返回 None。
    fn find_commit_by_message<'a>(
        repo: &'a VersionRepo,
        message_id: &str,
    ) -> Option<&'a Commit> {
        let head = repo.refs.get(&repo.head)?.clone();
        let mut cur = repo.commits.get(&head);
        while let Some(c) = cur {
            if c.head_message_id == message_id {
                return Some(c);
            }
            cur = c.parent.as_ref().and_then(|p| repo.commits.get(p));
        }
        None
    }

    /// 解析提交快照为消息列表
    fn snapshot(repo: &VersionRepo, commit: &Commit) -> Vec<Message> {
        commit.resolve(&repo.messages)
    }

    // ---------- 版本操作 ----------

    /// 开启分支：从包含 `message_id` 的提交创建新分支并切换 HEAD，
    /// 工作区由调用方同步为分支点快照（`git checkout -b <branch> <commit>`）。
    pub async fn create_branch(
        &self,
        conv_id: &str,
        message_id: &str,
        now: u64,
    ) -> Result<VersionOpResult> {
        let lock = self.conv_lock(conv_id);
        let _guard = lock.lock().await;
        let mut repo = self.load_repo(conv_id).await?;
        if repo.commits.is_empty() {
            return Err(CoreError::Msg("会话还没有可分支的历史".into()));
        }
        let commit = Self::find_commit_by_message(&repo, message_id)
            .ok_or_else(|| CoreError::Msg("未找到该消息对应的版本点（可能在其它分支）".into()))?
            .clone();
        let branch = Self::unique_ref_name(&repo, "branch", now);
        repo.refs.insert(branch.clone(), commit.hash.clone());
        repo.head = branch.clone();
        self.save_repo(conv_id, &repo).await?;
        let messages = Self::snapshot(&repo, &commit);
        Ok(VersionOpResult {
            head_hash: commit.hash.clone(),
            kind: CommitKind::Branch,
            branch: branch.clone(),
            note: format!(
                "分支 {branch}：从消息 {short_id} 开始",
                short_id = short(&commit.head_message_id)
            ),
            messages,
        })
    }

    /// 保存临时版本：在包含 `message_id` 的提交处打 `temp-*` 书签（不移动 HEAD）
    pub async fn save_temp_version(
        &self,
        conv_id: &str,
        message_id: &str,
        note: String,
        now: u64,
    ) -> Result<RefSummary> {
        let lock = self.conv_lock(conv_id);
        let _guard = lock.lock().await;
        let mut repo = self.load_repo(conv_id).await?;
        if repo.commits.is_empty() {
            return Err(CoreError::Msg("会话还没有可保存的历史".into()));
        }
        let commit = Self::find_commit_by_message(&repo, message_id)
            .ok_or_else(|| CoreError::Msg("未找到该消息对应的版本点（可能在其它分支）".into()))?
            .clone();
        let name = Self::unique_ref_name(&repo, "temp", now);
        repo.refs.insert(name.clone(), commit.hash.clone());
        repo.temp_notes.insert(name.clone(), note);
        self.save_repo(conv_id, &repo).await?;
        Ok(ref_summary(&repo, &name, RefKind::Temp, commit))
    }

    /// 回溯版本：重置 HEAD 到包含 `message_id` 的提交（丢弃其后消息）。
    /// 破坏性操作前先保存 `chkpt-*` 检查点（reflog 语义，任何状态可找回）。
    pub async fn rollback_to_message(
        &self,
        conv_id: &str,
        message_id: &str,
        now: u64,
    ) -> Result<VersionOpResult> {
        let lock = self.conv_lock(conv_id);
        let _guard = lock.lock().await;
        let mut repo = self.load_repo(conv_id).await?;
        if repo.commits.is_empty() {
            return Err(CoreError::Msg("会话还没有可回溯的历史".into()));
        }
        let commit = Self::find_commit_by_message(&repo, message_id)
            .ok_or_else(|| CoreError::Msg("未找到该消息对应的版本点（可能在其它分支）".into()))?
            .clone();
        Self::save_checkpoint(&mut repo, now);
        let head = repo.head.clone();
        repo.refs.insert(head.clone(), commit.hash.clone());
        self.save_repo(conv_id, &repo).await?;
        let messages = Self::snapshot(&repo, &commit);
        Ok(VersionOpResult {
            head_hash: commit.hash.clone(),
            kind: CommitKind::Rollback,
            branch: head,
            note: format!("回溯到消息 {short_id}", short_id = short(&commit.head_message_id)),
            messages,
        })
    }

    /// 撤回至此消息前：重置 HEAD 到该消息提交的父提交（丢弃该消息及其后全部）。
    pub async fn undo_before_message(
        &self,
        conv_id: &str,
        message_id: &str,
        now: u64,
    ) -> Result<VersionOpResult> {
        let lock = self.conv_lock(conv_id);
        let _guard = lock.lock().await;
        let mut repo = self.load_repo(conv_id).await?;
        if repo.commits.is_empty() {
            return Err(CoreError::Msg("会话还没有可撤回的历史".into()));
        }
        let commit = Self::find_commit_by_message(&repo, message_id)
            .ok_or_else(|| CoreError::Msg("未找到该消息对应的版本点（可能在其它分支）".into()))?
            .clone();
        let target = commit
            .parent
            .as_ref()
            .and_then(|p| repo.commits.get(p).cloned())
            .ok_or_else(|| CoreError::Msg("这是首条消息，之前没有内容可撤回".into()))?;
        Self::save_checkpoint(&mut repo, now);
        let head = repo.head.clone();
        repo.refs.insert(head.clone(), target.hash.clone());
        self.save_repo(conv_id, &repo).await?;
        let messages = Self::snapshot(&repo, &target);
        Ok(VersionOpResult {
            head_hash: target.hash.clone(),
            kind: CommitKind::Undo,
            branch: head,
            note: format!("撤回至消息 {short_id} 前", short_id = short(&commit.head_message_id)),
            messages,
        })
    }

    /// 检出到指定引用（`git checkout <ref>`）：
    /// - 分支/main → 切换分支；
    /// - temp/checkpoint → 从该提交新建分支并切换（避免 detached HEAD 丢失后续提交）。
    pub async fn checkout_ref(
        &self,
        conv_id: &str,
        ref_name: &str,
        now: u64,
    ) -> Result<VersionOpResult> {
        let lock = self.conv_lock(conv_id);
        let _guard = lock.lock().await;
        let mut repo = self.load_repo(conv_id).await?;
        let hash = repo
            .refs
            .get(ref_name)
            .cloned()
            .ok_or_else(|| CoreError::Msg(format!("引用 {ref_name} 不存在")))?;
        let commit = repo
            .commits
            .get(&hash)
            .cloned()
            .ok_or_else(|| CoreError::Msg(format!("提交 {h} 不存在", h = short(&hash))))?;
        Self::save_checkpoint(&mut repo, now);
        let (head, branch) = match RefKind::of(ref_name) {
            RefKind::Main | RefKind::Branch => {
                // 直接切换分支
                (ref_name.to_string(), ref_name.to_string())
            }
            RefKind::Temp | RefKind::Checkpoint => {
                // 从书签/检查点新建分支继续
                let b = Self::unique_ref_name(&repo, "branch", now);
                repo.refs.insert(b.clone(), hash.clone());
                (b.clone(), b.clone())
            }
        };
        repo.head = head;
        self.save_repo(conv_id, &repo).await?;
        let messages = Self::snapshot(&repo, &commit);
        Ok(VersionOpResult {
            head_hash: commit.hash.clone(),
            kind: CommitKind::Branch,
            branch,
            note: format!("检出 {ref_name}"),
            messages,
        })
    }

    /// 删除引用（临时版本/检查点/分支；main 不允许删除）
    pub async fn delete_ref(&self, conv_id: &str, ref_name: &str) -> Result<()> {
        let lock = self.conv_lock(conv_id);
        let _guard = lock.lock().await;
        let mut repo = self.load_repo(conv_id).await?;
        if !repo.refs.contains_key(ref_name) {
            return Err(CoreError::Msg(format!("引用 {ref_name} 不存在")));
        }
        if RefKind::of(ref_name) == RefKind::Main {
            return Err(CoreError::Msg("不能删除默认分支 main".into()));
        }
        if repo.head == ref_name {
            return Err(CoreError::Msg("不能删除当前检出的分支，请先切到其它分支".into()));
        }
        repo.refs.remove(ref_name);
        repo.temp_notes.remove(ref_name);
        self.save_repo(conv_id, &repo).await?;
        Ok(())
    }

    /// 列出会话版本（当前分支提交链 + 全部引用）
    pub async fn list_versions(&self, conv_id: &str) -> Result<VersionList> {
        let lock = self.conv_lock(conv_id);
        let _guard = lock.lock().await;
        let repo = self.load_repo(conv_id).await?;
        Ok(Self::list_locked(&repo))
    }

    /// 由仓库快照构建版本列表（不加锁版本，供内部复用）
    fn list_locked(repo: &VersionRepo) -> VersionList {
        let head = if repo.head.is_empty() {
            "main".to_string()
        } else {
            repo.head.clone()
        };
        // 引用摘要（含分支 / 临时版本 / 检查点），按时间新→旧排序
        let mut refs: Vec<RefSummary> = repo
            .refs
            .iter()
            .filter_map(|(name, hash)| {
                let commit = repo.commits.get(hash)?;
                let kind = RefKind::of(name);
                Some(ref_summary(repo, name, kind, commit.clone()))
            })
            .collect();
        refs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        // 当前分支提交链（新 → 旧）
        let mut commits: Vec<CommitSummary> = Vec::new();
        let mut cur = repo.refs.get(&head).cloned();
        while let Some(hash) = cur {
            let Some(c) = repo.commits.get(&hash) else { break };
            let is_head = commits.is_empty();
            commits.push(CommitSummary {
                hash: c.hash.clone(),
                kind: c.kind,
                note: c.note.clone(),
                created_at: c.created_at,
                head_message_id: c.head_message_id.clone(),
                message_count: c.message_count,
                is_head,
            });
            cur = c.parent.clone();
        }
        VersionList { head, refs, commits }
    }

    /// 破坏性操作前保存检查点：当前 HEAD 提交处打 `chkpt-*` 引用，
    /// 使任何历史状态都可通过 `checkout` 找回（reflog 语义）。
    fn save_checkpoint(repo: &mut VersionRepo, now: u64) {
        let head = repo.head.clone();
        if let Some(hash) = repo.refs.get(&head).cloned() {
            let name = Self::unique_ref_name(repo, "chkpt", now);
            repo.refs.insert(name, hash);
        }
    }

    /// 删除会话的版本仓库（会话删除时联动清理）
    pub async fn clear(&self, conv_id: &str) -> Result<()> {
        let lock = self.conv_lock(conv_id);
        let _guard = lock.lock().await;
        let path = self.repo_path(conv_id);
        if path.exists() {
            tokio::fs::remove_file(&path).await.map_err(CoreError::Io)?;
        }
        Ok(())
    }
}

/// 生成引用摘要（临时版本的备注优先取用户备注，否则用提交说明）
fn ref_summary(repo: &VersionRepo, name: &str, kind: RefKind, commit: Commit) -> RefSummary {
    let note = repo
        .temp_notes
        .get(name)
        .cloned()
        .unwrap_or_else(|| commit.note.clone());
    RefSummary {
        name: name.to_string(),
        kind: kind.as_str().to_string(),
        hash: commit.hash.clone(),
        created_at: commit.created_at,
        message_count: commit.message_count,
        head_message_id: commit.head_message_id.clone(),
        note,
    }
}

/// 消息 id 缩短展示（取后 8 位）
fn short(id: &str) -> String {
    if id.len() <= 8 {
        id.to_string()
    } else {
        id[id.len() - 8..].to_string()
    }
}

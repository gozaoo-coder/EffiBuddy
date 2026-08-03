//! 会话版本管理——自研内容寻址快照引擎（不依赖 git、不与工作区原有 git 仓库冲突）
//!
//! # 设计目标
//! 用户要求：每次 edit / write 等操作后自动保存当前工作区文件状态，可随时撤回 / 回溯，
//! 且**不能**干扰工作区里已有的 git 仓库（不做 `git add` / `git commit`，不写 `.git`）。
//! 因此这里用纯文件快照方案：内容寻址 + 增量去重，全部数据存放在应用私有目录：
//!
//! ```text
//! <appdata>/effisuite/session_versions/<conversation_id>/
//!   objects/             # 内容寻址对象（文件名 = 内容指纹 hex + 长度，去重）
//!   snapshots/<id>.json  # 每个快照的 manifest（路径 → 对象引用 + 大小）
//!   index.json           # 有序索引（最新在前），最多保留 MAX_SNAPSHOTS 条
//! ```
//!
//! # 关键语义
//! - **保存**：扫描工作区 → 计算每个文件的内容指纹 → 只写入未出现过的对象 → 生成 manifest。
//!   与最新快照完全一致时返回 `None`（无改动，不产生空快照）。
//! - **恢复**：按目标 manifest 把对象写回工作区对应路径；并删除「当前存在、被快照跟踪、
//!   但目标快照不含」的文件（即撤销该快照之后的新增文件）。恢复前自动保存
//!   「恢复前自动快照」，保证任何恢复都可再撤回。
//! - **忽略规则**：`.git` / `node_modules` / `target` / `dist` / `__pycache__` 等生成目录
//!   一律不纳入快照，避免把构建产物与依赖目录做成大快照。
//! - **内容指纹**：标准库 `DefaultHasher`（确定性 SipHash-1-3）+ 文件长度，足以用于
//!   增量去重与差异比对（非安全场景，不引入外部哈希依赖）。
//!
//! # 并发
//! 快照写操作（保存 / 删除 / 恢复）通过进程内静态互斥锁串行化，避免并发写 index.json
//! 相互覆盖；扫描与哈希在锁外进行以降低占用时间。

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

/// 快照保留上限（超出时淘汰最旧 manifest；对象文件靠内容寻址去重，不重复占用）
pub const MAX_SNAPSHOTS: usize = 200;

/// 快照来源：auto = agent 工具自动保存；manual = 用户手动保存；pre_restore = 恢复前保护
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotSource {
    Auto,
    Manual,
    PreRestore,
}

/// 单个文件条目（路径相对于工作区根，统一 `/` 分隔）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEntry {
    pub path: String,
    /// 内容指纹（hex，来自 DefaultHasher + 长度）
    pub hash: String,
    pub size: u64,
}

/// 快照摘要（列表 / 索引用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMeta {
    /// 快照 id（时间戳毫秒字符串）
    pub id: String,
    /// 创建时间（Unix 毫秒）
    pub created_at: u64,
    /// 备注（手动保存时的消息；自动保存为工具名 + 摘要）
    pub message: String,
    pub source: SnapshotSource,
    /// 文件数
    pub files: usize,
    /// 总字节数（去重后该快照实际引用的对象体积）
    pub bytes: u64,
}

/// 工作区状态（status 命令返回）
#[derive(Debug, Clone, Serialize)]
pub struct SnapshotStatus {
    /// 工作区路径
    pub working_dir: String,
    /// 工作区目录是否存在
    pub dir_exists: bool,
    /// 是否已有快照
    pub has_snapshot: bool,
    /// 最新快照 id
    pub latest_id: Option<String>,
    /// 最新快照时间（Unix 毫秒）
    pub latest_at: Option<u64>,
    /// 当前工作区与最新快照是否有差异（新增 / 修改 / 删除）
    pub dirty: bool,
    /// 差异明细
    pub changes: Vec<ChangeInfo>,
    /// 快照总数
    pub total: usize,
}

/// 单个差异项
#[derive(Debug, Clone, Serialize)]
pub struct ChangeInfo {
    pub path: String,
    /// added / modified / deleted
    pub kind: String,
}

/// 恢复结果
#[derive(Debug, Clone, Serialize)]
pub struct RestoreResult {
    /// 覆盖写回的文件数
    pub restored: usize,
    /// 删除的额外文件数（目标快照不含、但被快照跟踪的）
    pub removed: usize,
    /// 跳过（无变化）的文件数
    pub skipped: usize,
    /// 若为 true 表示这是干跑预览
    pub dry_run: bool,
    pub message: String,
}

// ==================== 路径 / 锁 ====================

/// 会话版本存储根：`<appdata>/effisuite/session_versions/<conversation_id>`
pub fn session_root(conversation_id: &str) -> PathBuf {
    crate::paths::appdata_root()
        .join("session_versions")
        .join(sanitize_conv_id(conversation_id))
}

/// 会话 id 可能含 `/` `\` 等字符，清洗为安全目录名
fn sanitize_conv_id(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    if cleaned.is_empty() {
        "default".to_string()
    } else {
        cleaned
    }
}

/// 进程内写锁：快照写操作串行化，避免并发写 index.json 相互覆盖
fn write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

// ==================== 忽略规则 ====================

/// 相对路径组件是否应忽略（生成目录 / 版本控制目录等）
fn is_ignored(rel: &str) -> bool {
    let mut parts = rel.split('/');
    let last = parts.next_back().unwrap_or(rel);
    if matches!(last, ".DS_Store" | "Thumbs.db" | ".gitignore_keep") {
        return true;
    }
    parts.any(|seg| {
        matches!(
            seg,
            ".git" | "node_modules" | "target" | "dist" | "build" | "__pycache__" | ".idea"
        )
    }) || matches!(
        last,
        ".git" | "node_modules" | "target" | "dist" | "build" | "__pycache__" | ".idea"
    )
}

// ==================== 内容指纹 ====================

/// 计算文件内容指纹：`<hex16>-<len>`（DefaultHasher 确定性 SipHash + 长度）
fn content_hash(data: &[u8]) -> String {
    let mut h = DefaultHasher::new();
    data.hash(&mut h);
    format!("{:016x}-{}", h.finish(), data.len())
}

// ==================== 扫描 ====================

/// 递归扫描目录，返回路径 → (hash, size)。跳过忽略项与符号链接。
fn scan_dir(dir: &Path) -> Result<Vec<SnapshotEntry>, String> {
    let mut entries = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(cur) = stack.pop() {
        let rd = fs::read_dir(&cur).map_err(|e| format!("读取目录失败 {}：{e}", cur.display()))?;
        for item in rd.flatten() {
            let ft = item
                .file_type()
                .map_err(|e| format!("读取文件类型失败 {}：{e}", item.path().display()))?;
            if ft.is_symlink() {
                continue;
            }
            let full = item.path();
            let rel = full
                .strip_prefix(dir)
                .unwrap_or(&full)
                .to_string_lossy()
                .replace('\\', "/");
            if ft.is_dir() {
                if is_ignored(&rel) {
                    continue;
                }
                stack.push(full);
            } else if ft.is_file() {
                if is_ignored(&rel) {
                    continue;
                }
                let data = fs::read(&full).map_err(|e| format!("读取文件失败 {}：{e}", full.display()))?;
                let hash = content_hash(&data);
                entries.push(SnapshotEntry {
                    path: rel,
                    hash,
                    size: data.len() as u64,
                });
            }
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

// ==================== 持久化：对象 / manifest / index ====================

fn objects_dir(root: &Path) -> PathBuf {
    root.join("objects")
}

fn snapshots_dir(root: &Path) -> PathBuf {
    root.join("snapshots")
}

fn index_path(root: &Path) -> PathBuf {
    root.join("index.json")
}

fn ensure_dirs(root: &Path) -> Result<(), String> {
    fs::create_dir_all(objects_dir(root)).map_err(|e| e.to_string())?;
    fs::create_dir_all(snapshots_dir(root)).map_err(|e| e.to_string())?;
    Ok(())
}

fn object_exists(root: &Path, hash: &str) -> bool {
    objects_dir(root).join(hash).exists()
}

fn write_object(root: &Path, hash: &str, data: &[u8]) -> Result<(), String> {
    let obj = objects_dir(root).join(hash);
    if obj.exists() {
        return Ok(());
    }
    fs::write(&obj, data).map_err(|e| format!("写入对象失败 {}：{e}", obj.display()))
}

fn read_object(root: &Path, hash: &str) -> Result<Vec<u8>, String> {
    let obj = objects_dir(root).join(hash);
    fs::read(&obj).map_err(|e| format!("读取对象失败 {}：{e}", obj.display()))
}

/// 读取索引（不存在时返回空列表）
fn read_index(root: &Path) -> Vec<SnapshotMeta> {
    let raw = fs::read_to_string(index_path(root)).unwrap_or_default();
    serde_json::from_str::<Vec<SnapshotMeta>>(&raw).unwrap_or_default()
}

fn write_index(root: &Path, metas: &[SnapshotMeta]) -> Result<(), String> {
    let tmp = index_path(root).with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(metas).unwrap_or_default())
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, index_path(root)).map_err(|e| e.to_string())
}

/// 读取某个快照的 manifest（路径 → entry）
fn read_manifest(root: &Path, id: &str) -> Result<HashMap<String, SnapshotEntry>, String> {
    let p = snapshots_dir(root).join(format!("{id}.json"));
    let raw = fs::read_to_string(&p).map_err(|e| format!("快照 {id} 不存在：{e}"))?;
    let entries: Vec<SnapshotEntry> =
        serde_json::from_str(&raw).map_err(|e| format!("快照 {id} 解析失败：{e}"))?;
    Ok(entries.into_iter().map(|e| (e.path.clone(), e)).collect())
}

fn write_manifest(root: &Path, id: &str, entries: &[SnapshotEntry]) -> Result<(), String> {
    let p = snapshots_dir(root).join(format!("{id}.json"));
    let tmp = p.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(entries).unwrap_or_default())
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

fn delete_manifest(root: &Path, id: &str) {
    let _ = fs::remove_file(snapshots_dir(root).join(format!("{id}.json")));
}

/// 快照 id：时间戳毫秒（同一毫秒两次保存会覆盖，故冲突时追加序号）
fn gen_id(existing: &[String]) -> String {
    let base = crate::state::now_ms().to_string();
    if !existing.iter().any(|e| e == &base) {
        return base;
    }
    let mut n = 1u32;
    loop {
        let cand = format!("{base}-{n}");
        if !existing.iter().any(|e| e == &cand) {
            return cand;
        }
        n += 1;
    }
}

// ==================== 对外操作 ====================

/// 保存快照：扫描工作区 → 去重写对象 → 生成 manifest → 更新索引。
/// 与最新快照完全一致时返回 `Ok(None)`（无改动）。
pub fn save_snapshot(
    conversation_id: &str,
    dir: &Path,
    message: &str,
    source: SnapshotSource,
) -> Result<Option<SnapshotMeta>, String> {
    if !dir.is_dir() {
        return Err(format!("工作区目录不存在：{}", dir.display()));
    }
    let root = session_root(conversation_id);
    ensure_dirs(&root)?;
    let entries = scan_dir(dir)?;

    let _guard = write_lock()
        .lock()
        .map_err(|_| "快照写锁获取失败".to_string())?;
    let mut metas = read_index(&root);
    let existing_ids: Vec<String> = metas.iter().map(|m| m.id.clone()).collect();

    // 与最新快照对比，无差异则跳过
    if let Some(latest) = metas.first() {
        if let Ok(latest_map) = read_manifest(&root, &latest.id) {
            let same_len = latest_map.len() == entries.len();
            let same = same_len
                && entries.iter().all(|e| {
                    latest_map
                        .get(&e.path)
                        .map(|o| o.hash == e.hash && o.size == e.size)
                        .unwrap_or(false)
                });
            if same {
                return Ok(None);
            }
        }
    }

    // 写对象（只写不存在的）
    for e in &entries {
        if object_exists(&root, &e.hash) {
            continue;
        }
        let data = fs::read(dir.join(&e.path))
            .map_err(|err| format!("读取文件失败 {}：{err}", dir.join(&e.path).display()))?;
        write_object(&root, &e.hash, &data)?;
    }

    let id = gen_id(&existing_ids);
    write_manifest(&root, &id, &entries)?;

    let bytes = entries.iter().map(|e| e.size).sum();
    let meta = SnapshotMeta {
        id: id.clone(),
        created_at: crate::state::now_ms(),
        message: if message.trim().is_empty() {
            match source {
                SnapshotSource::Auto => "自动快照".to_string(),
                SnapshotSource::Manual => "手动快照".to_string(),
                SnapshotSource::PreRestore => "恢复前保护".to_string(),
            }
        } else {
            message.trim().to_string()
        },
        source,
        files: entries.len(),
        bytes,
    };
    metas.insert(0, meta.clone());
    // 截断到上限：淘汰最旧 manifest
    if metas.len() > MAX_SNAPSHOTS {
        for old in metas.drain(MAX_SNAPSHOTS..) {
            delete_manifest(&root, &old.id);
        }
    }
    write_index(&root, &metas)?;
    Ok(Some(meta))
}

/// 快照列表（最新在前）
pub fn list_snapshots(conversation_id: &str) -> Vec<SnapshotMeta> {
    read_index(&session_root(conversation_id))
}

/// 工作区状态：与最新快照对比得出差异
pub fn snapshot_status(conversation_id: &str, dir: &Path) -> SnapshotStatus {
    let root = session_root(conversation_id);
    let metas = read_index(&root);
    let dir_exists = dir.is_dir();
    let latest = metas.first();

    // 无快照或目录不存在 → 直接返回基础状态
    if latest.is_none() || !dir_exists {
        return SnapshotStatus {
            working_dir: dir.display().to_string(),
            dir_exists,
            has_snapshot: latest.is_some(),
            latest_id: latest.map(|m| m.id.clone()),
            latest_at: latest.map(|m| m.created_at),
            dirty: false,
            changes: Vec::new(),
            total: metas.len(),
        };
    }

    let latest_meta = latest.unwrap();
    let latest_map = read_manifest(&root, &latest_meta.id).unwrap_or_default();
    let current = scan_dir(dir).unwrap_or_default();
    let cur_map: HashMap<String, &SnapshotEntry> =
        current.iter().map(|e| (e.path.clone(), e)).collect();

    let mut changes: Vec<ChangeInfo> = Vec::new();
    // 当前 vs 最新：修改 / 新增
    for e in &current {
        match latest_map.get(&e.path) {
            Some(o) if o.hash == e.hash && o.size == e.size => {}
            Some(_) => changes.push(ChangeInfo {
                path: e.path.clone(),
                kind: "modified".into(),
            }),
            None => changes.push(ChangeInfo {
                path: e.path.clone(),
                kind: "added".into(),
            }),
        }
    }
    // 最新快照有、当前没有 → 删除
    for (path, _) in &latest_map {
        if !cur_map.contains_key(path) {
            changes.push(ChangeInfo {
                path: path.clone(),
                kind: "deleted".into(),
            });
        }
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));

    SnapshotStatus {
        working_dir: dir.display().to_string(),
        dir_exists,
        has_snapshot: true,
        latest_id: Some(latest_meta.id.clone()),
        latest_at: Some(latest_meta.created_at),
        dirty: !changes.is_empty(),
        changes,
        total: metas.len(),
    }
}

/// 恢复快照：把工作区还原到目标快照的文件状态。
/// 恢复前自动保存「恢复前保护」快照（保证可再撤回）。
/// `dry_run=true` 只返回将要发生的操作，不写工作区。
pub fn restore_snapshot(
    conversation_id: &str,
    snapshot_id: &str,
    dir: &Path,
    dry_run: bool,
) -> Result<RestoreResult, String> {
    if !dir.is_dir() {
        return Err(format!("工作区目录不存在：{}", dir.display()));
    }
    let root = session_root(conversation_id);
    ensure_dirs(&root)?;
    let metas = read_index(&root);
    let exists = metas.iter().any(|m| m.id == snapshot_id);
    if !exists {
        return Err(format!("快照 {snapshot_id} 不存在"));
    }
    let target = read_manifest(&root, snapshot_id)?;

    // 恢复前保护快照：仅实际恢复时保存（dry run 不写盘）。
    // 保存后再读最新索引，使「待删除基准」覆盖到恢复那一刻的完整工作区：
    // 这样在目标快照之后新增的文件（如 agent 后续新建的 c.txt）也会被判定为
    // 「多余文件」一并清除，真正做到把工作区回溯到目标快照的文件状态。
    let latest_map = if dry_run {
        metas
            .first()
            .and_then(|m| read_manifest(&root, &m.id).ok())
            .unwrap_or_default()
    } else {
        if let Err(e) = save_snapshot(
            conversation_id,
            dir,
            &format!("恢复至 {snapshot_id} 前"),
            SnapshotSource::PreRestore,
        ) {
            tracing::warn!(error = %e, "恢复前自动快照失败");
        }
        read_index(&root)
            .first()
            .and_then(|m| read_manifest(&root, &m.id).ok())
            .unwrap_or_default()
    };

    // 待删除集合：当前存在 + 出现在「恢复前基准快照」 + 目标快照不含
    let current = scan_dir(dir)?;
    let cur_map: HashMap<String, &SnapshotEntry> =
        current.iter().map(|e| (e.path.clone(), e)).collect();
    let mut to_remove: Vec<&str> = Vec::new();
    for (path, _) in &latest_map {
        if cur_map.contains_key(path) && !target.contains_key(path) {
            to_remove.push(path.as_str());
        }
    }
    to_remove.sort_unstable();

    if !dry_run {
        // 覆盖写回目标文件
        for (path, entry) in &target {
            let data = read_object(&root, &entry.hash)?;
            let full = dir.join(path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&full, &data).map_err(|e| format!("恢复文件失败 {}：{e}", full.display()))?;
        }
        // 删除多余文件
        for path in &to_remove {
            let full = dir.join(path);
            if full.is_file() {
                let _ = fs::remove_file(&full);
            }
        }
    }

    let skipped = target
        .iter()
        .filter(|(p, e)| {
            cur_map
                .get(p.as_str())
                .map(|c| c.hash == e.hash && c.size == e.size)
                .unwrap_or(false)
        })
        .count();

    let restored = target.len() - skipped;
    let removed = to_remove.len();
    Ok(RestoreResult {
        restored,
        removed,
        skipped,
        dry_run,
        message: format!(
            "{}：覆盖 {restored} 个文件，删除 {removed} 个额外文件，跳过 {skipped} 个无变化文件",
            if dry_run { "预览" } else { "已恢复" }
        ),
    })
}

/// 删除快照（保护：至少保留一条且不可删除最新一条）
pub fn delete_snapshot(conversation_id: &str, snapshot_id: &str) -> Result<(), String> {
    let root = session_root(conversation_id);
    let _guard = write_lock()
        .lock()
        .map_err(|_| "快照写锁获取失败".to_string())?;
    let mut metas = read_index(&root);
    let idx = metas.iter().position(|m| m.id == snapshot_id);
    let Some(idx) = idx else {
        return Err(format!("快照 {snapshot_id} 不存在"));
    };
    if idx == 0 {
        return Err("不能删除最新一条快照（可先保存新快照再删）".to_string());
    }
    let meta = metas.remove(idx);
    delete_manifest(&root, &meta.id);
    write_index(&root, &metas)?;
    Ok(())
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "effisuite-snapshot-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn save_scan_restore_delete_flow() {
        let dir = tmp_dir();
        // 会话 id 唯一：避免跨测试残留快照（appdata 目录共享）
        let conv = format!("test-{}", uuid::Uuid::new_v4());
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/b.txt"), "world").unwrap();
        // 忽略项不应入快照
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::write(dir.join("node_modules/x.js"), "big").unwrap();

        let s1 = save_snapshot(&conv, &dir, "v1", SnapshotSource::Manual).unwrap().unwrap();
        assert_eq!(s1.files, 2);

        // 再次保存无改动 → None
        assert!(save_snapshot(&conv, &dir, "again", SnapshotSource::Auto)
            .unwrap()
            .is_none());

        // 修改 a.txt → v2
        std::fs::write(dir.join("a.txt"), "hello2").unwrap();
        let s2 = save_snapshot(&conv, &dir, "v2", SnapshotSource::Auto).unwrap().unwrap();
        assert_eq!(s2.files, 2);
        assert_eq!(list_snapshots(&conv).len(), 2);

        // status：dirty（相对最新 v2，当前无改动）→ 先改再查
        std::fs::write(dir.join("c.txt"), "new").unwrap();
        let st = snapshot_status(&conv, &dir);
        assert!(st.dirty);
        assert!(st.changes.iter().any(|c| c.kind == "added"));

          // 恢复 v1（dry run 预览）
        let preview = restore_snapshot(&conv, &s1.id, &dir, true).unwrap();
        assert!(preview.dry_run);
        // 实际恢复
        let r = restore_snapshot(&conv, &s1.id, &dir, false).unwrap();
        assert!(r.restored >= 1);
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "hello");
        assert!(!dir.join("c.txt").exists());
        assert!(!dir.join("node_modules").exists() || dir.join("node_modules/x.js").exists());

          // 恢复前内部自动保存「保护快照」（当前状态与 v2 不同，必然新增一条），
          // 保证任何恢复都能再撤回；随后 list = [保护快照, v2, v1]
          assert_eq!(list_snapshots(&conv).len(), 3);

        // 删除最新 → 拒绝
        let latest = list_snapshots(&conv);
        assert!(delete_snapshot(&conv, &latest[0].id).is_err());
        // 删除非最新 → 成功
        assert!(delete_snapshot(&conv, &latest[1].id).is_ok());

        // 清理：临时工作区 + 会话快照存储
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(session_root(&conv)).ok();
    }
}

//! 常用工作区（Favorite Workspace）：用户收藏的会话工作区路径
//!
//! 在「会话工作区」设置面板里，用户可把常用目录收藏起来以便快速切换，
//! 也可删除不再需要的收藏。典型场景：
//!
//! - 用户在某会话设置工作区后，点「收藏为常用」→ 写入此存储
//! - 打开新会话时，从常用列表一步切换过去，省去反复弹系统目录选择框
//!
//! # 设计要点（对齐 pinned_memory）
//!
//! - 线程安全 + 廉价 clone：内部 `Arc<RwLock<..>>`，读多写少
//! - 锁内零 IO：`persist` 在锁外做 `tokio::fs::write`
//! - 落盘为单个 JSON 数组文件 `favorite_workspaces.json`，便于备份与 P2P 同步

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{CoreError, Result};

/// 一个常用工作区收藏项
///
/// 字段按大小降序排列以最小化 padding：
/// `id` / `path`（String, 24B）→ `created_at`（u64, 8B）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FavoriteWorkspace {
    pub id: String,
    /// 收藏的目录绝对路径
    pub path: String,
    pub created_at: u64,
}

/// 常用工作区存储，线程安全且可廉价 clone（内部 `Arc<RwLock<..>>`）
#[derive(Clone)]
pub struct FavoriteWorkspaceStore {
    inner: Arc<RwLock<FavoriteWorkspaceState>>,
    path: PathBuf,
}

struct FavoriteWorkspaceState {
    workspaces: Vec<FavoriteWorkspace>,
}

impl FavoriteWorkspaceStore {
    /// 创建存储并加载磁盘上已存在的数据。
    ///
    /// `path` 为持久化文件路径（通常是 `<appdata>/favorite_workspaces.json`）。
    /// 父目录不存在时自动创建；文件不存在或解析失败时返回空列表（best-effort）。
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(CoreError::Io)?;
        }
        let workspaces = if path.exists() {
            match std::fs::read(&path) {
                Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };
        Ok(Self {
            inner: Arc::new(RwLock::new(FavoriteWorkspaceState { workspaces })),
            path,
        })
    }

    /// 列出全部常用工作区（按 `created_at` 降序，新的在前）
    pub async fn list(&self) -> Vec<FavoriteWorkspace> {
        let s = self.inner.read().await;
        let mut out: Vec<FavoriteWorkspace> = s.workspaces.to_vec();
        out.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        out
    }

    /// 收藏一个工作区，返回 id。
    ///
    /// `id` 为空则自动生成；若已收藏过相同 `path`，则幂等返回既有条目的 id（不重复收藏）。
    pub async fn add(&self, path: impl Into<String>) -> Result<String> {
        let path = path.into().trim().to_string();
        if path.is_empty() {
            return Err(CoreError::Msg("收藏的工作区路径不能为空".into()));
        }
        let id = uuid::Uuid::new_v4().to_string();
        // created_at 严格递增：取当前时间与既有最大值+1 的较大者，保证「新的在前」排序稳定
        let created_at = {
            let s = self.inner.read().await;
            let max = s.workspaces.iter().map(|w| w.created_at).max().unwrap_or(0);
            max.max(now_ms()) + 1
        };
        {
            let mut s = self.inner.write().await;
            // 幂等：相同路径已存在则返回其既有 id，不新增
            if let Some(existing) = s.workspaces.iter().find(|w| w.path == path) {
                return Ok(existing.id.clone());
            }
            s.workspaces.push(FavoriteWorkspace {
                id: id.clone(),
                path,
                created_at,
            });
        }
        self.persist().await?;
        Ok(id)
    }

    /// 删除指定 id 的收藏；不存在视为成功（幂等）
    pub async fn delete(&self, id: &str) -> Result<()> {
        let changed = {
            let mut s = self.inner.write().await;
            let before = s.workspaces.len();
            s.workspaces.retain(|w| w.id != id);
            before != s.workspaces.len()
        };
        if changed {
            self.persist().await?;
        }
        Ok(())
    }

    /// 按 id 查询单个收藏
    pub async fn get(&self, id: &str) -> Option<FavoriteWorkspace> {
        let s = self.inner.read().await;
        s.workspaces.iter().find(|w| w.id == id).cloned()
    }

    /// 持久化到磁盘（锁外 IO）
    async fn persist(&self) -> Result<()> {
        let bytes = {
            let s = self.inner.read().await;
            serde_json::to_vec_pretty(&s.workspaces).map_err(CoreError::Serde)?
        };
        tokio::fs::write(&self.path, bytes)
            .await
            .map_err(CoreError::Io)
    }
}

/// 当前 Unix 时间戳（毫秒）
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "effisuite-fav-ws-test-{}.json",
            uuid::Uuid::new_v4()
        ))
    }

    #[tokio::test]
    async fn add_list_delete_roundtrip() {
        let store = FavoriteWorkspaceStore::new(tmp_path()).unwrap();
        assert!(store.list().await.is_empty());

        let id1 = store.add("/path/a").await.unwrap();
        let _id2 = store.add("/path/b").await.unwrap();
        let id3 = store.add("/path/c").await.unwrap();

        let list = store.list().await;
        assert_eq!(list.len(), 3);
        // 降序：先 add 的排后面
        assert_eq!(list[0].id, id3);
        assert_eq!(list[2].id, id1);

        store.delete(&id1).await.unwrap();
        assert_eq!(store.list().await.len(), 2);
    }

    #[tokio::test]
    async fn add_same_path_is_idempotent() {
        let store = FavoriteWorkspaceStore::new(tmp_path()).unwrap();
        let id1 = store.add("/path/dup").await.unwrap();
        let id2 = store.add("/path/dup").await.unwrap();
        assert_eq!(id1, id2, "相同路径应幂等返回既有 id");
        assert_eq!(store.list().await.len(), 1);
    }

    #[tokio::test]
    async fn add_empty_path_errors() {
        let store = FavoriteWorkspaceStore::new(tmp_path()).unwrap();
        assert!(store.add("   ").await.is_err());
    }

    #[tokio::test]
    async fn delete_nonexistent_is_idempotent() {
        let store = FavoriteWorkspaceStore::new(tmp_path()).unwrap();
        store.delete("nope").await.unwrap();
    }

    #[tokio::test]
    async fn persist_and_reload_across_instances() {
        let path = tmp_path();
        let store_a = FavoriteWorkspaceStore::new(&path).unwrap();
        let id = store_a.add("/persist/path").await.unwrap();

        // 用同一文件路径再创建一个实例，应能读到已落盘数据
        let store_b = FavoriteWorkspaceStore::new(&path).unwrap();
        let list = store_b.list().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].path, "/persist/path");

        // 清理临时文件
        std::fs::remove_file(&path).ok();
    }
}
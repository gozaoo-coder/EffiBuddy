//! 定时任务（ScheduledTask）持久化存储
//!
//! 基于 JSON 文件：每个任务一个文件，存放在 `appdata/schedules/<id>.json`。
//! 模式与 [`crate::storage::ConversationStore`] / [`crate::skill_store::SkillStore`]
//! 一致：`RwLock` 多读、IO 在锁外、`with_capacity` 预分配。

use std::path::{Path, PathBuf};

use tokio::sync::RwLock;

use crate::{CoreError, Result, ScheduledTask};

/// 定时任务存储，线程安全可廉价 clone（内部 RwLock + Arc）
#[derive(Clone)]
pub struct ScheduledTaskStore {
    root: PathBuf,
    _lock: std::sync::Arc<RwLock<()>>,
}

impl ScheduledTaskStore {
    /// 创建存储，root 不存在时自动创建
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        Ok(Self {
            root,
            _lock: std::sync::Arc::new(RwLock::new(())),
        })
    }

    /// 任务文件路径：`<root>/<id>.json`
    #[inline]
    fn path_for(&self, id: &str) -> PathBuf {
        let safe = Path::new(id)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(id));
        self.root.join(safe).with_extension("json")
    }

    /// 列出全部定时任务，按 created_at 降序。
    pub async fn list(&self) -> Result<Vec<ScheduledTask>> {
        let mut entries = tokio::fs::read_dir(&self.root).await.map_err(CoreError::Io)?;
        let mut out = Vec::with_capacity(8);
        while let Some(entry) = entries.next_entry().await.map_err(CoreError::Io)? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(_) => continue,
            };
            let task: ScheduledTask = match serde_json::from_slice(&bytes) {
                Ok(t) => t,
                Err(_) => continue,
            };
            out.push(task);
        }
        out.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        Ok(out)
    }

    /// 加载单个任务，不存在返回 None
    pub async fn get(&self, id: &str) -> Result<Option<ScheduledTask>> {
        let path = self.path_for(id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = tokio::fs::read(&path).await.map_err(CoreError::Io)?;
        let task: ScheduledTask = serde_json::from_slice(&bytes).map_err(CoreError::Serde)?;
        Ok(Some(task))
    }

    /// 保存（或覆盖）一个任务
    pub async fn save(&self, task: &ScheduledTask) -> Result<()> {
        let path = self.path_for(&task.id);
        let bytes = serde_json::to_vec(task).map_err(CoreError::Serde)?;
        tokio::fs::write(&path, bytes).await.map_err(CoreError::Io)?;
        Ok(())
    }

    /// 删除指定任务，不存在返回 Ok(())
    pub async fn delete(&self, id: &str) -> Result<()> {
        let path = self.path_for(id);
        if path.exists() {
            tokio::fs::remove_file(&path)
                .await
                .map_err(CoreError::Io)?;
        }
        Ok(())
    }

    /// 更新上次执行时间；任务不存在时返回 NotFound
    pub async fn update_last_run(&self, id: &str, time: u64) -> Result<()> {
        let _guard = self._lock.write().await;
        let mut task = self
            .get(id)
            .await?
            .ok_or_else(|| CoreError::NotFound(format!("scheduled task {} not found", id)))?;
        task.last_run = Some(time);
        self.save(&task).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("effisuite-sched-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn save_list_update_delete() {
        let store = ScheduledTaskStore::new(tmp_dir()).unwrap();
        let task = ScheduledTask {
            id: "t1".to_string(),
            name: "n".to_string(),
            skill_id: "agent-reach".to_string(),
            cron: "0 * * * *".to_string(),
            last_run: None,
            created_at: 1,
            enabled: true,
        };
        store.save(&task).await.unwrap();
        assert_eq!(store.list().await.unwrap().len(), 1);

        store.update_last_run("t1", 999).await.unwrap();
        let got = store.get("t1").await.unwrap().unwrap();
        assert_eq!(got.last_run, Some(999));

        store.delete("t1").await.unwrap();
        assert!(store.get("t1").await.unwrap().is_none());
    }
}

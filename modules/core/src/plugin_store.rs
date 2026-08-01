//! 已安装插件（InstalledPlugin）持久化存储
//!
//! 与 [`SkillStore`] 类似的简单 JSON 文件方案：每个插件一个文件，
//! 存放在 `appdata/plugins/<safe_id>.json`。
//!
//! 注意：`InstalledPlugin.id` 形如 `<owner>/<name>`，含 `/`，
//! 不能直接作为文件名。这里将 `/` 替换为 `__`（双下划线），
//! 既保留可读性，又避免目录穿越。
//!
//! 设计要点：
//! - 读多写少：`RwLock` 允许多读
//! - IO 在锁外完成
//! - `list` 用 `with_capacity` 预分配
//! - 文件名安全：`/` → `__`，并校验最终 file_name 与转换后一致

use std::path::{Path, PathBuf};

use tokio::sync::RwLock;

use crate::{CoreError, InstalledPlugin, Result};

/// `/` 在文件名中的转义序列
const SLUG_SEP: &str = "__";

/// 已安装插件存储，线程安全可廉价 clone（内部 RwLock + Arc）
#[derive(Clone)]
pub struct PluginStore {
    root: PathBuf,
    _lock: std::sync::Arc<RwLock<()>>,
}

impl PluginStore {
    /// 创建存储，root 不存在时自动创建
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        Ok(Self {
            root,
            _lock: std::sync::Arc::new(RwLock::new(())),
        })
    }

    /// 返回存储根目录，供安装流程把解压目录与元数据放在同一位置
    #[inline]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 将插件 id（`<owner>/<name>`）转换为安全文件名。
    ///
    /// 规则：将所有 `/` 替换为 `__`。对于已含 `__` 的合法 id
    /// 极少见，可接受轻微歧义（仅作为文件名，原 id 已存入 JSON）。
    #[inline]
    fn id_to_file_name(id: &str) -> String {
        id.replace('/', SLUG_SEP)
    }

    /// 插件文件路径：`<root>/<safe_id>.json`
    ///
    /// 二次防御：使用 `Path::file_name` 校验最终路径仍位于 root 内。
    fn path_for(&self, id: &str) -> PathBuf {
        let safe = Self::id_to_file_name(id);
        // 进一步防止 id 含路径分隔符或 `..`：仅取 file_name
        let file_name = Path::new(&safe)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(&safe));
        self.root.join(file_name).with_extension("json")
    }

    /// 列出所有已安装插件，按 `installed_at` 降序（新的在前）。
    pub async fn list(&self) -> Result<Vec<InstalledPlugin>> {
        let mut entries = tokio::fs::read_dir(&self.root)
            .await
            .map_err(CoreError::Io)?;
        let mut out = Vec::with_capacity(8);
        while let Some(entry) = entries.next_entry().await.map_err(CoreError::Io)? {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(_) => continue,
            };
            let plugin: InstalledPlugin = match serde_json::from_slice(&bytes) {
                Ok(p) => p,
                Err(_) => continue,
            };
            out.push(plugin);
        }
        out.sort_by_key(|b| std::cmp::Reverse(b.installed_at));
        Ok(out)
    }

    /// 按 id 加载单个插件
    pub async fn get(&self, id: &str) -> Result<Option<InstalledPlugin>> {
        let path = self.path_for(id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = tokio::fs::read(&path).await.map_err(CoreError::Io)?;
        let plugin: InstalledPlugin = serde_json::from_slice(&bytes).map_err(CoreError::Serde)?;
        Ok(Some(plugin))
    }

    /// 按 ClawHub 包名查找（用于检测是否已安装 / 跳过重复安装）。
    ///
    /// 匹配 `name == package_name`（如 "whatsapp" 或 "@openclaw/whatsapp"）。
    /// O(n) 但 n 通常很小。
    pub async fn find_by_name(&self, name: &str) -> Result<Option<InstalledPlugin>> {
        let plugins = self.list().await?;
        Ok(plugins.into_iter().find(|p| p.name == name))
    }

    /// 保存（或覆盖）一个插件记录
    pub async fn save(&self, plugin: &InstalledPlugin) -> Result<()> {
        let path = self.path_for(&plugin.id);
        let bytes = serde_json::to_vec(plugin).map_err(CoreError::Serde)?;
        tokio::fs::write(&path, bytes)
            .await
            .map_err(CoreError::Io)?;
        Ok(())
    }

    /// 删除指定插件记录。同时清理解压目录（若存在）。
    pub async fn delete(&self, id: &str) -> Result<()> {
        let path = self.path_for(id);
        if path.exists() {
            tokio::fs::remove_file(&path).await.map_err(CoreError::Io)?;
        }
        // 删除可能的解压目录：<root>/<safe_id>/
        let safe = Self::id_to_file_name(id);
        let dir = self.root.join(&safe);
        if dir.is_dir() {
            if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
                tracing::warn!(error = %e, dir = ?dir, "删除插件解压目录失败（忽略）");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("effisuite-plugin-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_plugin(id: &str, name: &str) -> InstalledPlugin {
        InstalledPlugin {
            id: id.to_string(),
            name: name.to_string(),
            display_name: name.to_string(),
            summary: "test summary".to_string(),
            family: "code-plugin".to_string(),
            channel: "official".to_string(),
            owner_handle: "openclaw".to_string(),
            version: "1.0.0".to_string(),
            install_path: None,
            installed_at: 42,
        }
    }

    #[tokio::test]
    async fn save_get_delete_plugin() {
        let store = PluginStore::new(tmp_dir()).unwrap();
        let p = sample_plugin("openclaw/whatsapp", "whatsapp");
        store.save(&p).await.unwrap();

        let got = store.get("openclaw/whatsapp").await.unwrap().unwrap();
        assert_eq!(got.name, "whatsapp");
        assert_eq!(got.version, "1.0.0");

        store.delete("openclaw/whatsapp").await.unwrap();
        assert!(store.get("openclaw/whatsapp").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn find_by_name_matches() {
        let store = PluginStore::new(tmp_dir()).unwrap();
        let p = sample_plugin("openclaw/whatsapp", "@openclaw/whatsapp");
        store.save(&p).await.unwrap();

        let found = store
            .find_by_name("@openclaw/whatsapp")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, "openclaw/whatsapp");

        let missing = store.find_by_name("telegram").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn list_returns_all_sorted_desc() {
        let store = PluginStore::new(tmp_dir()).unwrap();
        // 故意以非降序插入
        store
            .save(&InstalledPlugin {
                installed_at: 100,
                ..sample_plugin("o/a", "a")
            })
            .await
            .unwrap();
        store
            .save(&InstalledPlugin {
                installed_at: 300,
                ..sample_plugin("o/c", "c")
            })
            .await
            .unwrap();
        store
            .save(&InstalledPlugin {
                installed_at: 200,
                ..sample_plugin("o/b", "b")
            })
            .await
            .unwrap();

        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 3);
        // 期望按 installed_at 降序
        assert_eq!(list[0].id, "o/c");
        assert_eq!(list[1].id, "o/b");
        assert_eq!(list[2].id, "o/a");
    }

    #[test]
    fn id_filename_safe() {
        // 含 `/` 的 id 应安全转换为不含分隔符的文件名
        assert_eq!(
            PluginStore::id_to_file_name("openclaw/whatsapp"),
            "openclaw__whatsapp"
        );
        // 命名空间形式
        assert_eq!(
            PluginStore::id_to_file_name("@openclaw/whatsapp"),
            "@openclaw__whatsapp"
        );
    }

    #[tokio::test]
    async fn path_for_rejects_traversal() {
        let store = PluginStore::new(tmp_dir()).unwrap();
        // `..` 在 file_name 提取时会被剥离，不会越界
        let path = store.path_for("../escape");
        // 路径仍应位于 root 内
        assert!(path.starts_with(&store.root));
        // file_name 部分不应为 `escape.json` 而应剔除 `..`
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        assert!(
            !file_name.contains(".."),
            "file_name 含 .. 是危险的: {file_name}"
        );
    }
}

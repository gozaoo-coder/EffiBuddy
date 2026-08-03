//! 插件配置存储（按插件命名空间隔离）
//!
//! 每个插件拥有独立的配置命名空间：`<appdata>/plugin_configs/<safe_id>.json`，
//! 文件内容为 `{ key: value }` 的 JSON 映射。
//!
//! 设计要点：
//! - 命名空间隔离：插件只能读写自己 id 对应的配置，互不干扰
//! - 目录穿越防护：插件 id → 文件名时 `/` 替换为 `__`，并取 file_name 兜底
//! - 读多写少：`RwLock` 允许多读
//! - 全量读写单文件：配置文件小，整文件读写即可，避免部分写损坏
//!
//! 安全边界：插件 id 必须先经 plugin_store 校验（已安装插件）再访问本存储，
//! 由命令层保证（get_plugin_config / set_plugin_config 会先查 plugin_store）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tokio::sync::RwLock;

use crate::{CoreError, Result};

/// 插件配置存储，线程安全可廉价 clone（内部 RwLock + Arc）
#[derive(Clone)]
pub struct PluginConfigStore {
    root: PathBuf,
    _lock: std::sync::Arc<RwLock<()>>,
}

impl PluginConfigStore {
    /// 创建存储，root 不存在时自动创建
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        Ok(Self {
            root,
            _lock: std::sync::Arc::new(RwLock::new(())),
        })
    }

    /// 插件配置文件路径：`<root>/<safe_id>.json`
    fn path_for(&self, plugin_id: &str) -> PathBuf {
        let safe = plugin_id.replace('/', "__");
        let file_name = Path::new(&safe)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(&safe));
        self.root.join(file_name).with_extension("json")
    }

    /// 读取插件全部配置（不存在返回空对象）
    pub async fn get_all(&self, plugin_id: &str) -> Result<Value> {
        let path = self.path_for(plugin_id);
        if !path.exists() {
            return Ok(Value::Object(Default::default()));
        }
        let bytes = tokio::fs::read(&path).await.map_err(CoreError::Io)?;
        let v: Value = serde_json::from_slice(&bytes).map_err(CoreError::Serde)?;
        Ok(v)
    }

    /// 读取单个配置项；不存在返回 None
    pub async fn get(&self, plugin_id: &str, key: &str) -> Result<Option<Value>> {
        let all = self.get_all(plugin_id).await?;
        Ok(all.get(key).cloned())
    }

    /// 写入单个配置项
    pub async fn set(&self, plugin_id: &str, key: &str, value: Value) -> Result<()> {
        let mut map: BTreeMap<String, Value> = match self.get_all(plugin_id).await? {
            Value::Object(m) => m.into_iter().collect(),
            _ => BTreeMap::new(),
        };
        map.insert(key.to_string(), value);
        self.write(plugin_id, map).await
    }

    /// 删除单个配置项
    pub async fn remove(&self, plugin_id: &str, key: &str) -> Result<()> {
        let mut map: BTreeMap<String, Value> = match self.get_all(plugin_id).await? {
            Value::Object(m) => m.into_iter().collect(),
            _ => BTreeMap::new(),
        };
        map.remove(key);
        self.write(plugin_id, map).await
    }

    /// 删除插件全部配置（卸载时调用）
    pub async fn delete_all(&self, plugin_id: &str) -> Result<()> {
        let path = self.path_for(plugin_id);
        if path.exists() {
            tokio::fs::remove_file(&path).await.map_err(CoreError::Io)?;
        }
        Ok(())
    }

    async fn write(&self, plugin_id: &str, map: BTreeMap<String, Value>) -> Result<()> {
        let path = self.path_for(plugin_id);
        let obj: serde_json::Map<String, Value> = map.into_iter().collect();
        let bytes = serde_json::to_vec_pretty(&Value::Object(obj)).map_err(CoreError::Serde)?;
        tokio::fs::write(&path, bytes).await.map_err(CoreError::Io)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("effisuite-plugin-cfg-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn set_get_remove() {
        let store = PluginConfigStore::new(tmp_dir()).unwrap();
        assert!(store.get("owner/demo", "k").await.unwrap().is_none());

        store
            .set("owner/demo", "k", Value::from(true))
            .await
            .unwrap();
        assert_eq!(
            store.get("owner/demo", "k").await.unwrap(),
            Some(Value::from(true))
        );

        store.remove("owner/demo", "k").await.unwrap();
        assert!(store.get("owner/demo", "k").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn namespace_isolated() {
        let store = PluginConfigStore::new(tmp_dir()).unwrap();
        store.set("a", "x", Value::from(1)).await.unwrap();
        assert!(store.get("b", "x").await.unwrap().is_none());
        assert_eq!(store.get("a", "x").await.unwrap(), Some(Value::from(1)));
    }

    #[tokio::test]
    async fn delete_all_clears() {
        let store = PluginConfigStore::new(tmp_dir()).unwrap();
        store.set("owner/demo", "k", Value::from(1)).await.unwrap();
        store.delete_all("owner/demo").await.unwrap();
        assert!(store.get("owner/demo", "k").await.unwrap().is_none());
    }
}

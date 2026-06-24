//! Config persistence. Single JSON file under the app data dir.
//! Schema is a free-form map; plugins read/write their own keys.

use anyhow::{Context, Result};
use parking_lot::RwLock;
use serde_json::{Map, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

pub struct ConfigStore {
    path: PathBuf,
    data: Arc<RwLock<Map<String, Value>>>,
}

impl ConfigStore {
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        let data = if path.exists() {
            let raw = fs::read_to_string(&path).context("read config")?;
            serde_json::from_str::<Map<String, Value>>(&raw).context("parse config")?
        } else {
            Map::new()
        };
        Ok(Self {
            path,
            data: Arc::new(RwLock::new(data)),
        })
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        self.data.read().get(key).cloned()
    }

    pub fn set(&self, key: impl Into<String>, value: Value) -> Result<()> {
        {
            let mut guard = self.data.write();
            guard.insert(key.into(), value);
        }
        self.flush()
    }

    pub fn all(&self) -> Value {
        Value::Object(self.data.read().clone())
    }

    fn flush(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let raw = serde_json::to_string_pretty(&*self.data.read())?;
        fs::write(&self.path, raw).context("write config")?;
        Ok(())
    }
}

fn config_path() -> Result<PathBuf> {
    let dir = dirs::data_dir().context("no data dir")?;
    Ok(dir.join("desktop-suite").join("config.json"))
}

/// Directory holding installed packages.
pub fn packages_dir() -> Result<PathBuf> {
    let dir = dirs::data_dir().context("no data dir")?;
    let p = dir.join("desktop-suite").join("packages");
    fs::create_dir_all(&p).ok();
    Ok(p)
}

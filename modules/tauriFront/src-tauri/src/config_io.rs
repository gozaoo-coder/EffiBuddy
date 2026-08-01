//! 配置文件持久化：加载与保存。
//!
//! 配置以 JSON 形式落盘到 `<appdata>/config.json`，
//! 加载失败（文件不存在或解析失败）时回退到默认配置。

use effisuite_core::AgentConfig;

use crate::paths::config_path;

/// 加载配置；不存在时返回默认值
pub(crate) fn load_config_or_default() -> AgentConfig {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => AgentConfig::default(),
    }
}

/// 持久化配置到磁盘
pub(crate) fn save_config(config: &AgentConfig) -> std::result::Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let s = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(&path, s).map_err(|e| e.to_string())?;
    Ok(())
}

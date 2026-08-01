//! 插件管理工具集：让 LLM 自主卸载已安装插件
//!
//! 当前仅提供卸载能力（安装走 ClawHub 前端命令）。
//! 插件 id 形如 `<owner>/<name>`，卸载时支持精确匹配，未命中时按 name 回退匹配。

use effisuite_core::PluginStore;
use rig_core::tool::Tool;
use serde::Deserialize;

/// 卸载插件参数
#[derive(Deserialize)]
pub struct UninstallPluginArgs {
    /// 插件 id（如 `openclaw/whatsapp`）或 name（如 `whatsapp`）
    pub id: String,
}

/// 卸载插件错误
#[derive(Debug, thiserror::Error)]
#[error("uninstall plugin error: {0}")]
pub struct UninstallPluginError(String);

/// 卸载插件工具
///
/// 持有 `PluginStore` 共享句柄。先按 id 精确匹配，未命中则按 name 匹配。
/// 删除后返回结果文本。
pub struct UninstallPluginTool {
    store: PluginStore,
}

impl UninstallPluginTool {
    pub fn new(store: PluginStore) -> Self {
        Self { store }
    }
}

impl Tool for UninstallPluginTool {
    const NAME: &'static str = "uninstall_plugin";

    type Error = UninstallPluginError;
    type Args = UninstallPluginArgs;
    type Output = String;

    fn description(&self) -> String {
        "卸载已安装的插件。id 优先精确匹配（如 `openclaw/whatsapp`）；\
         若未命中，会尝试按插件 name 匹配。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "插件 id（如 openclaw/whatsapp）或 name（如 whatsapp）"
                }
            },
            "required": ["id"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let target = args.id.trim();
        if target.is_empty() {
            return Err(UninstallPluginError("id 不能为空".to_string()));
        }

        // 1. 精确 id 匹配
        let plugin = match self.store.get(target).await {
            Ok(Some(p)) => Some(p),
            _ => {
                // 2. 按 name 回退匹配
                let plugins = self
                    .store
                    .list()
                    .await
                    .map_err(|e| UninstallPluginError(e.to_string()))?;
                plugins
                    .into_iter()
                    .find(|p| p.name == target || p.display_name == target)
            }
        };

        let Some(plugin) = plugin else {
            return Ok(format!(
                "未找到 id 或 name 包含「{}」的插件。可调用 list_installed_plugins 查看全部已安装插件。",
                target
            ));
        };

        let name = plugin.display_name.clone();
        let id = plugin.id.clone();

        self.store
            .delete(&id)
            .await
            .map_err(|e| UninstallPluginError(e.to_string()))?;

        Ok(format!(
            "已卸载插件「{}」（id={}）。",
            name, id
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use effisuite_core::InstalledPlugin;

    fn tmp_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "effisuite-plugin-tool-test-{}",
            uuid::Uuid::new_v4()
        ))
    }

    fn sample_plugin(id: &str, name: &str, display_name: &str) -> InstalledPlugin {
        InstalledPlugin {
            id: id.to_string(),
            name: name.to_string(),
            display_name: display_name.to_string(),
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
    async fn uninstall_plugin_by_id() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let store = PluginStore::new(&dir).unwrap();
        let plugin = sample_plugin("openclaw/whatsapp", "whatsapp", "WhatsApp");
        store.save(&plugin).await.unwrap();

        let tool = UninstallPluginTool::new(store.clone());
        let out = tool
            .call(UninstallPluginArgs {
                id: "openclaw/whatsapp".to_string(),
            })
            .await
            .unwrap();
        assert!(out.contains("已卸载插件"));
        assert!(out.contains("WhatsApp"));
        assert!(store.get("openclaw/whatsapp").await.unwrap().is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn uninstall_plugin_by_name_fallback() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let store = PluginStore::new(&dir).unwrap();
        let plugin = sample_plugin("openclaw/telegram", "telegram", "Telegram");
        store.save(&plugin).await.unwrap();

        let tool = UninstallPluginTool::new(store.clone());
        let out = tool
            .call(UninstallPluginArgs {
                id: "telegram".to_string(),
            })
            .await
            .unwrap();
        assert!(out.contains("已卸载插件"));
        assert!(store.get("openclaw/telegram").await.unwrap().is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn uninstall_plugin_not_found() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let store = PluginStore::new(&dir).unwrap();

        let tool = UninstallPluginTool::new(store.clone());
        let out = tool
            .call(UninstallPluginArgs {
                id: "missing".to_string(),
            })
            .await
            .unwrap();
        assert!(out.contains("未找到"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn uninstall_plugin_rejects_empty_id() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let store = PluginStore::new(&dir).unwrap();

        let tool = UninstallPluginTool::new(store.clone());
        let res = tool
            .call(UninstallPluginArgs {
                id: "   ".to_string(),
            })
            .await;
        assert!(res.is_err());

        std::fs::remove_dir_all(&dir).ok();
    }
}

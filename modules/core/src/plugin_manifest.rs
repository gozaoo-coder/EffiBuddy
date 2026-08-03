//! 插件 Manifest：声明式贡献清单（插件不执行代码）
//!
//! EffiSuite 的插件体系采用「声明式 manifest」驱动：插件包内可携带
//! `plugin.json`（或 `manifest.json` / `effisuite.json`），声明其贡献——
//! 左栏按钮 / 页面 / 命令。插件本身不执行任意代码，安全性由本模块的
//! 校验（id 匹配 / 权限白名单 / 路径防护）+ 后端只读解析保证。
//!
//! 生命周期（详见 docs/plugin-architecture.md）：
//! - 安装：ClawHub 下载解压 → 落盘 plugin_store → 读取 manifest 校验
//! - 激活：`list_plugin_contributions` 汇总全部已安装插件贡献供前端消费
//! - 卸载：删除 plugin_store 记录 + 解压目录，贡献随之消失
//!
//! 内置示例：`builtin_contributions()` 返回 EffiSuite 自带示例页面
//! （UserTodoPage）的贡献，作为「插件注册页签/页面/组件」的参考实例。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{CoreError, Result};

/// manifest 文件名候选（按优先级依次探测）
pub const MANIFEST_NAMES: &[&str] = &["plugin.json", "manifest.json", "effisuite.json"];

/// 当前支持的 manifest API 版本（主版本一致即可）
pub const MANIFEST_API_VERSION: &str = "1.0";

/// 声明式权限白名单：插件只能请求以下能力，超出即拒绝
pub const KNOWN_PERMISSIONS: &[&str] = &[
    "config.read",
    "config.write",
    "config.delete",
    "agent.command",
    "agent.skill",
    "ui.rail",
    "ui.page",
    "files.read",
    "shell",
];

/// 插件左栏按钮动作
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRailAction {
    /// 动作类型：`open-page` | `command`
    #[serde(rename = "type")]
    pub kind: PluginRailActionKind,
    /// open-page 时：目标页面 id
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    /// command 时：要触发的命令 id
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// 动作类型枚举（序列化为 kebab-case 标签）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginRailActionKind {
    OpenPage,
    Command,
}

/// 插件贡献到左栏一的按钮
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRailContribution {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub icon: String,
    /// 所在分组：main / bottom，默认 main
    #[serde(default = "default_section")]
    pub section: String,
    pub action: PluginRailAction,
}

/// 插件注册的页面（页签 / 路由 / 组件）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPageContribution {
    /// 页面 id（`<pluginId>/<pageId>`，由后端组装）
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub route: String,
    /// 入口：builtin（前端注册表）| file（插件包内文件）
    #[serde(default = "default_entry")]
    pub entry: String,
}

/// 插件注册给 agent 的命令（命令 / skill）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCommandContribution {
    /// 命令 id（`<pluginId>/<cmdId>`）
    pub id: String,
    pub name: String,
    pub description: String,
}

/// 单个插件的贡献集合
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginContributions {
    #[serde(default)]
    pub rail: Vec<PluginRailContribution>,
    #[serde(default)]
    pub pages: Vec<PluginPageContribution>,
    #[serde(default)]
    pub commands: Vec<PluginCommandContribution>,
}

  /// 插件 manifest（从插件包内文件解析）
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct PluginManifest {
    #[serde(default = "default_api_version")]
    pub api_version: String,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// 声明式权限白名单（仅允许 KNOWN_PERMISSIONS 内的能力）
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub contributions: PluginContributions,
}

/// 单个插件的贡献汇总（list_plugin_contributions 响应单元）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginContributionSet {
    pub plugin_id: String,
    pub plugin_name: String,
    pub display_name: String,
    pub version: String,
    pub rail: Vec<PluginRailContribution>,
    pub pages: Vec<PluginPageContribution>,
    pub commands: Vec<PluginCommandContribution>,
}

fn default_section() -> String {
    "main".to_string()
}

fn default_entry() -> String {
    "builtin".to_string()
}

fn default_api_version() -> String {
    MANIFEST_API_VERSION.to_string()
}

impl PluginManifest {
    /// 校验 manifest 的合法性与安全性。
    ///
    /// - API 版本主版本必须兼容
    /// - 插件 id 必须匹配预期（防目录穿越 / 越权）
    /// - 贡献 id 非空且唯一
    /// - 权限必须在白名单内
    pub fn validate(&self, expected_id: &str) -> Result<()> {
        if self.api_version.split('.').next() != MANIFEST_API_VERSION.split('.').next() {
            return Err(CoreError::Config(format!(
                "插件 manifest API 版本不兼容：{}（需要 {}.*）",
                self.api_version, MANIFEST_API_VERSION
            )));
        }
        if self.id.is_empty() {
            return Err(CoreError::Config("插件 manifest 缺少 id".into()));
        }
        if !expected_id.is_empty() && self.id != expected_id {
            return Err(CoreError::Config(format!(
                "插件 manifest id 不匹配：声明 {}，期望 {}",
                self.id, expected_id
            )));
        }

        // 贡献 id 唯一性
        let mut seen = std::collections::HashSet::new();
        for r in &self.contributions.rail {
            if r.id.is_empty() || !seen.insert(format!("rail:{}", r.id)) {
                return Err(CoreError::Config(format!(
                    "插件贡献 rail id 非法或重复：{}",
                    r.id
                )));
            }
        }
        for p in &self.contributions.pages {
            if p.id.is_empty() || !seen.insert(format!("page:{}", p.id)) {
                return Err(CoreError::Config(format!(
                    "插件贡献 page id 非法或重复：{}",
                    p.id
                )));
            }
        }
        for c in &self.contributions.commands {
            if c.id.is_empty() || !seen.insert(format!("cmd:{}", c.id)) {
                return Err(CoreError::Config(format!(
                    "插件贡献 command id 非法或重复：{}",
                    c.id
                )));
            }
        }

        // 权限白名单
        for p in &self.permissions {
            if !KNOWN_PERMISSIONS.contains(&p.as_str()) {
                return Err(CoreError::Config(format!(
                    "插件请求了未授权能力：{p}（允许：{}）",
                    KNOWN_PERMISSIONS.join(", ")
                )));
            }
        }
        Ok(())
    }
}

/// 在插件解压目录中探测并解析 manifest（依次尝试候选文件名）。
pub fn load_manifest(install_dir: &Path) -> Result<Option<PluginManifest>> {
    for name in MANIFEST_NAMES {
        let candidate = install_dir.join(name);
        if !candidate.is_file() {
            continue;
        }
        let bytes = std::fs::read(&candidate).map_err(CoreError::Io)?;
        let manifest: PluginManifest = serde_json::from_slice(&bytes).map_err(|e| {
            CoreError::Config(format!("解析插件 manifest {} 失败：{e}", name))
        })?;
        return Ok(Some(manifest));
    }
    Ok(None)
}

/// 把 manifest 的贡献组装为带插件上下文的贡献集合（页面/命令 id 加插件前缀）。
pub fn build_contribution_set(
    plugin_id: &str,
    plugin_name: &str,
    display_name: &str,
    version: &str,
    manifest: &PluginManifest,
) -> PluginContributionSet {
    let c = &manifest.contributions;
    PluginContributionSet {
        plugin_id: plugin_id.to_string(),
        plugin_name: plugin_name.to_string(),
        display_name: display_name.to_string(),
        version: version.to_string(),
        rail: c.rail.clone(),
        pages: c
            .pages
            .iter()
            .map(|p| PluginPageContribution {
                id: format!("{plugin_id}/{}", p.id),
                title: p.title.clone(),
                icon: p.icon.clone(),
                route: p.route.clone(),
                entry: p.entry.clone(),
            })
            .collect(),
        commands: c
            .commands
            .iter()
            .map(|cmd| PluginCommandContribution {
                id: format!("{plugin_id}/{}", cmd.id),
                name: cmd.name.clone(),
                description: cmd.description.clone(),
            })
            .collect(),
    }
}

/// 内置示例插件贡献：EffiSuite「我的待办」页面。
///
/// 作为「插件注册页签/页面/组件」体系的可运行实例，不依赖任何 ClawHub 插件，
/// 前端页面注册表（usePluginPages）按 `effisuite/user-todo` 解析到 UserTodoPage。
pub fn builtin_contributions() -> PluginContributionSet {
    PluginContributionSet {
        plugin_id: "effisuite".to_string(),
        plugin_name: "effisuite-builtin".to_string(),
        display_name: "EffiSuite 内置".to_string(),
        version: "1.0.0".to_string(),
        rail: vec![PluginRailContribution {
            id: "user-todo".to_string(),
            label: "我的待办".to_string(),
            icon: "book".to_string(),
            section: "main".to_string(),
            action: PluginRailAction {
                kind: PluginRailActionKind::OpenPage,
                page_id: Some("effisuite/user-todo".to_string()),
                command: None,
            },
        }],
        pages: vec![PluginPageContribution {
            id: "effisuite/user-todo".to_string(),
            title: "我的待办".to_string(),
            icon: "book".to_string(),
            route: "/todo".to_string(),
            entry: "builtin".to_string(),
        }],
        commands: vec![PluginCommandContribution {
            id: "effisuite/user-todo/list".to_string(),
            name: "todo_list".to_string(),
            description: "列出「我的待办」页面中的待办事项".to_string(),
        }],
    }
}

/// 校验 `install_dir` 可安全用于 manifest 探测（防止目录穿越）。
pub fn safe_install_dir(install_dir: &Path, plugins_root: &Path) -> bool {
    install_dir.starts_with(plugins_root)
}

/// 将插件 id（`<owner>/<name>` 或 `<name>`）转换为安全路径段（`/` → `__`）。
pub fn safe_plugin_path_segment(plugin_id: &str) -> String {
    PathBuf::from(plugin_id.replace('/', "__"))
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| plugin_id.replace('/', "__"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_manifest(id: &str) -> PluginManifest {
        serde_json::from_str(&format!(
            r#"{{
                "api_version": "1.0",
                "id": "{id}",
                "name": "demo",
                "version": "1.0.0",
                "contributions": {{
                    "rail": [{{"id":"b1","label":"按钮","icon":"chat","section":"main","action":{{"type":"open-page","pageId":"demo/p1"}}}}],
                    "pages": [{{"id":"p1","title":"页面","icon":"book","entry":"builtin"}}],
                    "commands": [{{"id":"c1","name":"cmd","description":"desc"}}]
                }}
            }}"#
        ))
        .unwrap()
    }

    #[test]
    fn validate_ok() {
        let m = valid_manifest("owner/demo");
        m.validate("owner/demo").unwrap();
    }

    #[test]
    fn validate_rejects_id_mismatch() {
        let m = valid_manifest("owner/demo");
        assert!(m.validate("other/x").is_err());
    }

    #[test]
    fn validate_rejects_unknown_permission() {
        let mut m = valid_manifest("owner/demo");
        m.permissions = vec!["sudo.rm".to_string()];
        assert!(m.validate("owner/demo").is_err());
    }

    #[test]
    fn validate_rejects_api_version() {
        let mut m = valid_manifest("owner/demo");
        m.api_version = "2.0".to_string();
        assert!(m.validate("owner/demo").is_err());
    }

    #[test]
    fn build_contribution_set_prefixes_ids() {
        let m = valid_manifest("owner/demo");
        let set = build_contribution_set("owner/demo", "demo", "Demo", "1.0.0", &m);
        assert_eq!(set.plugin_id, "owner/demo");
        assert_eq!(set.pages[0].id, "owner/demo/p1");
        assert_eq!(set.commands[0].id, "owner/demo/c1");
    }

    #[test]
    fn safe_segment_replaces_slash() {
        assert_eq!(safe_plugin_path_segment("owner/demo"), "owner__demo");
        assert_eq!(safe_plugin_path_segment("demo"), "demo");
    }
}

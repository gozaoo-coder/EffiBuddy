//! 插件贡献注册与插件配置命令
//!
//! - `list_plugin_contributions`：汇总「内置示例 + 全部已安装插件」的声明式贡献
//! - `get_plugin_config` / `set_plugin_config` / `delete_plugin_config` / `get_plugin_config_all`：
//!   插件配置命名空间读写（持久化到 appdata/plugin_configs/<safe_id>.json）
//! - `sync_plugin_skills`：把插件 manifest 声明的命令注册为 agent 技能
//!   （SkillStore source="plugin"），使 agent 在 `list_installed_skills` 与
//!   RAG 技能注入中「看到」插件命令
//!
//! 安全边界：
//! - manifest 只读解析 + 校验（id 匹配 / 权限白名单），插件不执行代码
//! - 配置读写前先校验插件已安装（plugin_store 命中），拒绝未安装插件的访问

use std::path::PathBuf;
use std::sync::Arc;

use effisuite_core::{
    build_contribution_set, builtin_contributions, load_manifest, safe_plugin_path_segment, Skill,
    PluginContributionSet, PluginManifest, PluginStore, SkillIndex, SkillStore,
};
use serde_json::Value;
use tauri::State;

use crate::commands::skills::rebuild_skill_index_from;
use crate::state::{now_ms, AppState};

/// 汇总全部插件贡献：内置示例 + 已安装插件 manifest。
///
/// 对每个已安装插件：
/// 1. 定位其解压目录（install_path）
/// 2. 探测并解析 manifest（plugin.json / manifest.json / effisuite.json）
/// 3. 校验（id 匹配 / 权限白名单）
/// 4. 组装带插件前缀的贡献集合（页面/命令 id 加 `<pluginId>/`）
#[tauri::command]
pub(crate) async fn list_plugin_contributions(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let mut plugins: Vec<PluginContributionSet> = Vec::new();

    // 1. 内置示例（EffiSuite「我的待办」）
    plugins.push(builtin_contributions());

    // 2. 已安装插件
    let installed = state
        .plugin_store
        .list()
        .await
        .map_err(|e| e.to_string())?;
    let plugins_root = state.plugin_store.root().to_path_buf();

    for plugin in installed {
        let Some(install_path) = plugin.install_path.clone() else {
            continue;
        };
        let dir = PathBuf::from(&install_path);
        // 路径防护：解压目录必须在 plugins 根下
        if !dir.starts_with(&plugins_root) {
            tracing::warn!(plugin_id = %plugin.id, "插件安装路径越界，跳过贡献注册");
            continue;
        }
        let manifest = match load_manifest(&dir) {
            Ok(Some(m)) => m,
            Ok(None) => continue, // 无 manifest 的插件不贡献 UI/命令
            Err(e) => {
                tracing::warn!(plugin_id = %plugin.id, error = %e, "插件 manifest 解析失败，跳过");
                continue;
            }
        };
        if let Err(e) = manifest.validate(&plugin.id) {
            tracing::warn!(plugin_id = %plugin.id, error = %e, "插件 manifest 校验失败，跳过");
            continue;
        }
        plugins.push(build_contribution_set(
            &plugin.id,
            &plugin.name,
            &plugin.display_name,
            &plugin.version,
            &manifest,
        ));
    }

    Ok(serde_json::json!({ "plugins": plugins }))
}

/// 获取单个已安装插件的 manifest（供前端查看/调试）。
#[tauri::command]
pub(crate) async fn get_plugin_manifest(
    state: State<'_, AppState>,
    plugin_id: String,
) -> Result<Option<PluginManifest>, String> {
    let plugin = state
        .plugin_store
        .get(&plugin_id)
        .await
        .map_err(|e| e.to_string())?;
    let Some(plugin) = plugin else {
        return Ok(None);
    };
    let Some(install_path) = plugin.install_path else {
        return Ok(None);
    };
    let dir = PathBuf::from(&install_path);
    if !dir.starts_with(state.plugin_store.root()) {
        return Ok(None);
    }
    load_manifest(&dir).map_err(|e| e.to_string())
}

/// 前置校验：插件 id 必须对应一个已安装插件（防未授权访问配置）
async fn ensure_plugin_installed(state: &AppState, plugin_id: &str) -> Result<(), String> {
    if plugin_id.is_empty() {
        return Err("插件 id 不能为空".to_string());
    }
    match state.plugin_store.get(plugin_id).await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(format!("插件未安装：{plugin_id}")),
        Err(e) => Err(e.to_string()),
    }
}

/// 读取插件配置项（命名空间隔离，仅已安装插件可访问）
#[tauri::command]
pub(crate) async fn get_plugin_config(
    state: State<'_, AppState>,
    plugin_id: String,
    key: String,
) -> Result<Option<Value>, String> {
    ensure_plugin_installed(&state, &plugin_id).await?;
    state
        .plugin_config
        .get(&plugin_id, &key)
        .await
        .map_err(|e| e.to_string())
}

/// 写入插件配置项
#[tauri::command]
pub(crate) async fn set_plugin_config(
    state: State<'_, AppState>,
    plugin_id: String,
    key: String,
    value: Value,
) -> Result<(), String> {
    ensure_plugin_installed(&state, &plugin_id).await?;
    if key.is_empty() {
        return Err("配置键不能为空".to_string());
    }
    state
        .plugin_config
        .set(&plugin_id, &key, value)
        .await
        .map_err(|e| e.to_string())
}

/// 删除插件配置项
#[tauri::command]
pub(crate) async fn delete_plugin_config(
    state: State<'_, AppState>,
    plugin_id: String,
    key: String,
) -> Result<(), String> {
    ensure_plugin_installed(&state, &plugin_id).await?;
    state
        .plugin_config
        .remove(&plugin_id, &key)
        .await
        .map_err(|e| e.to_string())
}

/// 读取插件全部配置（对象）
#[tauri::command]
pub(crate) async fn get_plugin_config_all(
    state: State<'_, AppState>,
    plugin_id: String,
) -> Result<Value, String> {
    ensure_plugin_installed(&state, &plugin_id).await?;
    state
        .plugin_config
        .get_all(&plugin_id)
        .await
        .map_err(|e| e.to_string())
}

/// 卸载插件时清理其配置（由 clawhub_uninstall_plugin 调用）
pub(crate) async fn cleanup_plugin_config(state: &AppState, plugin_id: &str) {
    if let Err(e) = state.plugin_config.delete_all(plugin_id).await {
        tracing::warn!(plugin_id = %plugin_id, error = %e, "清理插件配置失败（忽略）");
    }
}

/// 收集全部已安装插件的 manifest 命令贡献（供技能同步）。
///
/// 只依赖 `&PluginStore`（可 Clone 的句柄），不依赖完整 AppState，
/// 因此可在 `'static` 后台任务（启动同步）中调用。
async fn collect_plugin_commands(plugin_store: &PluginStore) -> Vec<(String, PluginManifest)> {
    let mut out = Vec::new();
    let installed = match plugin_store.list().await {
        Ok(v) => v,
        Err(_) => return out,
    };
    let plugins_root = plugin_store.root().to_path_buf();
    for plugin in installed {
        let Some(install_path) = plugin.install_path.clone() else {
            continue;
        };
        let dir = PathBuf::from(&install_path);
        if !dir.starts_with(&plugins_root) {
            continue;
        }
        let manifest = match load_manifest(&dir) {
            Ok(Some(m)) => m,
            _ => continue,
        };
        if manifest.validate(&plugin.id).is_err() {
            continue;
        }
        if !manifest.contributions.commands.is_empty() {
            out.push((plugin.id, manifest));
        }
    }
    out
}

/// 把插件 manifest 声明的命令注册为 agent 技能（SkillStore source="plugin"）。
///
/// - 技能 id：`plugin:<pluginId>:<cmdId>`（`/` → `:`，保证文件安全且唯一）
/// - preamble：描述该命令的用途，agent 启用后可理解并调用
/// - 同步策略：幂等 upsert；已不存在的插件命令会被清除
/// - 调用时机：应用启动后 / 插件安装后 / 插件卸载后
/// 把插件 manifest 声明的命令注册为 agent 技能（SkillStore source="plugin"）。
///
/// - 技能 id：`plugin:<pluginId>:<cmdId>`（`/` → `:`，保证文件安全且唯一）
/// - preamble：描述该命令的用途，agent 启用后可理解并调用
/// - 同步策略：幂等 upsert；已不存在的插件命令会被清除
/// - 调用时机：应用启动后 / 插件安装后 / 插件卸载后
///
/// 只接收可 Clone 的存储句柄（plugin_store / skill_store / skill_index），
/// 便于在 `'static` 后台任务（启动）与 async 命令（安装/卸载）中调用。
pub(crate) async fn sync_plugin_skills(
    plugin_store: PluginStore,
    skill_store: SkillStore,
    skill_index: Arc<SkillIndex>,
) {
    let commands = collect_plugin_commands(&plugin_store).await;

    // 组装新技能集合
    let mut skills: Vec<Skill> = Vec::new();
    for (plugin_id, manifest) in &commands {
        for cmd in &manifest.contributions.commands {
            let skill_id = format!("plugin:{}", cmd.id.replace('/', ":"));
            let preamble = format!(
                "这是一个由插件「{}」注册的命令。\n命令名：{}\n用途：{}\n插件 id：{}",
                manifest.display_name, cmd.name, cmd.description, plugin_id
            );
            skills.push(Skill {
                id: skill_id,
                name: format!("{}（插件）", cmd.name),
                description: cmd.description.clone(),
                preamble,
                tools: Vec::new(),
                working_dir: None,
                created_at: now_ms(),
                builtin: false,
                source: Some("plugin".to_string()),
                source_slug: None,
                source_owner: Some(plugin_id.clone()),
                source_version: Some(manifest.version.clone()),
            });
        }
    }

    // 1. upsert / 清除过期的插件技能（source == "plugin"）
    let existing = match skill_store.list_user().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "同步插件技能：读取技能列表失败");
            return;
        }
    };
    let current_ids: std::collections::HashSet<String> =
        skills.iter().map(|s| s.id.clone()).collect();
    for old in &existing {
        if old.source.as_deref() != Some("plugin") {
            continue;
        }
        if !current_ids.contains(&old.id) {
            let _ = skill_store.delete(&old.id).await;
        }
    }
    for skill in &skills {
        if let Err(e) = skill_store.save(skill).await {
            tracing::warn!(skill_id = %skill.id, error = %e, "同步插件技能失败");
        }
    }

    // 2. 重建技能 RAG 索引，使插件命令进入 agent 的「可用技能」注入
    rebuild_skill_index_from(&skill_index, &skill_store).await;
}

/// 供调试/文档使用的路径段工具（保持与 core 一致）
#[allow(dead_code)]
fn _path_segment(plugin_id: &str) -> String {
    safe_plugin_path_segment(plugin_id)
}

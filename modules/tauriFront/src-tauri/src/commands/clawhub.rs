//! ClawHub 浏览 / 安装命令（Skills & Plugins）。
//!
//! 与本地 skill_store 不同，ClawHub 命令直接走 HTTP API：
//! - 浏览 / 搜索 / 详情：透传到 clawhub.ai，前端按需懒加载
//! - 安装 skill：下载 ZIP → spawn_blocking 解压到 `<skills_dir>/<slug>/`
//!   → 解析 SKILL.md → 写入 skill_store（source="clawhub"）
//! - 安装 plugin：下载 → 解压到 `<plugins_dir>/<safe_id>/`
//!   → 写入 plugin_store 元数据
//! - 卸载：删除文件 + 解压目录
//!
//! ClawHub 限速 3000/min/IP（读）与 1200/min/IP（下载），429 时返回 Retry-After。
//! 这里只做单次请求，重试退避由前端控制（避免在命令层阻塞）。

use effisuite_core::clawhub::{
    extract_zip_to, parse_skill_md, PackageListResponse, PackageResponse, PackageSearchResponse,
    SearchResponse, SkillListResponse, SkillResponse,
};
use effisuite_core::{InstalledPlugin, Skill};
use std::sync::Arc;
use tauri::Emitter;

use crate::commands::plugins::sync_plugin_skills;
use crate::commands::skills::rebuild_skill_index;
use crate::paths::skills_dir;
use crate::state::{now_ms, AppState};

/// `GET /api/v1/skills` - 列出 ClawHub 技能
#[tauri::command]
pub(crate) async fn clawhub_list_skills(
    state: tauri::State<'_, AppState>,
    limit: Option<u32>,
    sort: Option<String>,
    cursor: Option<String>,
) -> Result<SkillListResponse, String> {
    state
        .clawhub
        .list_skills(limit, sort.as_deref(), cursor.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// `GET /api/v1/search?q=...` - 搜索 ClawHub 技能
#[tauri::command]
pub(crate) async fn clawhub_search_skills(
    state: tauri::State<'_, AppState>,
    q: String,
    limit: Option<u32>,
) -> Result<SearchResponse, String> {
    state
        .clawhub
        .search_skills(&q, limit)
        .await
        .map_err(|e| e.to_string())
}

/// `GET /api/v1/skills/{slug}` - 获取 ClawHub 技能详情
#[tauri::command]
pub(crate) async fn clawhub_get_skill(
    state: tauri::State<'_, AppState>,
    slug: String,
) -> Result<SkillResponse, String> {
    state
        .clawhub
        .get_skill(&slug)
        .await
        .map_err(|e| e.to_string())
}

/// 安装 ClawHub 技能：下载 ZIP → 解压到 `<skills_dir>/<slug>/` → 解析 SKILL.md → 落盘 skill_store
///
/// 流程：
/// 1. 检查是否已安装（find_by_clawhub_slug），已安装则返回 existing id（幂等）
/// 2. 拉取 skill 详情获取 owner/version 元数据
/// 3. 下载 ZIP（5min 超时）
/// 4. spawn_blocking 中解压到 `<skills_dir>/<slug>/`，带 zip-slip 防护
/// 5. 解析 SKILL.md frontmatter 提取 name/description/version，
///    同时把正文（去除 frontmatter）写入 `preamble`，
///    使 enable_skill 工具能透明注入到 agent 上下文，agent 据此"看到"并"使用"技能
/// 6. 构造 Skill 记录，写入 skill_store（source="clawhub"）
///
/// 返回新（或已存在）技能 id。
#[tauri::command]
pub(crate) async fn clawhub_install_skill(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    slug: String,
) -> Result<String, String> {
    // 幂等：若已安装则直接返回
    if let Some(existing) = state
        .skill_store
        .find_by_clawhub_slug(&slug)
        .await
        .map_err(|e| e.to_string())?
    {
        // 即使是幂等命中也通知前端刷新（用户可能在前端等待状态变化）
        let _ = app_handle.emit("clawhub-skill-installed", &existing.id);
        return Ok(existing.id);
    }

    let client = state.clawhub.clone();
    let skill_store = state.skill_store.clone();
    let skills_root = skills_dir();
    let slug_clone = slug.clone();

    // 1. 拉详情获取 owner / latest_version
    let detail = client
        .get_skill(&slug)
        .await
        .map_err(|e| format!("获取技能详情失败: {e}"))?;
    let owner_handle = detail
        .owner
        .as_ref()
        .and_then(|o| o.handle.clone())
        .unwrap_or_default();
    let version = detail
        .latest_version
        .as_ref()
        .map(|v| v.version.clone())
        .unwrap_or_default();

    // 2. 下载 ZIP
    let zip_bytes = client
        .download_skill_zip(&slug, None, None)
        .await
        .map_err(|e| format!("下载技能包失败: {e}"))?;

    // 3. 解压到 <skills_dir>/<slug>/
    let dest_dir = skills_root.join(&slug);
    let dest_for_blocking = dest_dir.clone();
    tokio::task::spawn_blocking(move || extract_zip_to(&dest_for_blocking, &zip_bytes))
        .await
        .map_err(|e| format!("解压任务调度失败: {e}"))?
        .map_err(|e| format!("解压失败: {e}"))?;

    // 4. 解析 SKILL.md：提取 frontmatter 字段 + 正文作为 preamble
    // preamble 写入 Skill.preamble，enable_skill 工具注入为 System 消息，
    // agent 据此看到技能指令；working_dir 已指向解压目录，agent 可通过
    // read_file/list_files/shell 访问技能携带的脚本与资源文件。
    let skill_md_path = dest_dir.join("SKILL.md");
    let (name, description, parsed_version, body) =
        match tokio::fs::read_to_string(&skill_md_path).await {
            Ok(content) => {
                let p = parse_skill_md(&content);
                (
                    if p.name.is_empty() {
                        slug.clone()
                    } else {
                        p.name
                    },
                    if p.description.is_empty() {
                        format!("ClawHub 技能: {}", slug)
                    } else {
                        p.description
                    },
                    if p.version.is_empty() {
                        version.clone()
                    } else {
                        p.version
                    },
                    p.body,
                )
            }
            Err(_) => (
                slug.clone(),
                format!("ClawHub 技能: {}", slug),
                version.clone(),
                String::new(),
            ),
        };

    // 5. 落盘 skill_store：preamble 为 SKILL.md 正文（无 frontmatter 时为整个文件）
    let skill = Skill {
        id: slug_clone.clone(),
        name,
        description,
        preamble: body,
        tools: Vec::new(),
        working_dir: Some(dest_dir.to_string_lossy().into_owned()),
        created_at: now_ms(),
        builtin: false,
        source: Some("clawhub".to_string()),
        source_slug: Some(slug_clone.clone()),
        source_owner: if owner_handle.is_empty() {
            None
        } else {
            Some(owner_handle)
        },
        source_version: if parsed_version.is_empty() {
            None
        } else {
            Some(parsed_version)
        },
    };
    skill_store.save(&skill).await.map_err(|e| e.to_string())?;
    rebuild_skill_index(&state).await;
    // 通知前端：技能已安装，ClawHubPanel / SkillPanel 据此刷新
    let _ = app_handle.emit("clawhub-skill-installed", &skill.id);
    Ok(skill.id)
}

/// 卸载 ClawHub 技能：删除 skill_store 记录 + 解压目录
#[tauri::command]
pub(crate) async fn clawhub_uninstall_skill(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    state
        .skill_store
        .delete(&id)
        .await
        .map_err(|e| e.to_string())?;
    rebuild_skill_index(&state).await;
    // 通知前端：技能已卸载，ClawHubPanel / SkillPanel 据此刷新
    let _ = app_handle.emit("clawhub-skill-uninstalled", &id);
    Ok(())
}

/// `GET /api/v1/plugins` - 列出 ClawHub 插件
#[tauri::command]
pub(crate) async fn clawhub_list_plugins(
    state: tauri::State<'_, AppState>,
    limit: Option<u32>,
    sort: Option<String>,
    cursor: Option<String>,
) -> Result<PackageListResponse, String> {
    state
        .clawhub
        .list_plugins(limit, sort.as_deref(), cursor.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// `GET /api/v1/plugins/search?q=...` - 搜索 ClawHub 插件
#[tauri::command]
pub(crate) async fn clawhub_search_plugins(
    state: tauri::State<'_, AppState>,
    q: String,
    limit: Option<u32>,
) -> Result<PackageSearchResponse, String> {
    state
        .clawhub
        .search_plugins(&q, limit)
        .await
        .map_err(|e| e.to_string())
}

/// `GET /api/v1/packages/{name}` - 获取 ClawHub 包详情
#[tauri::command]
pub(crate) async fn clawhub_get_package(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<PackageResponse, String> {
    state
        .clawhub
        .get_package(&name)
        .await
        .map_err(|e| e.to_string())
}

/// 安装 ClawHub 插件：下载 → 解压到 `<plugins_dir>/<safe_id>/` → 落盘 plugin_store
///
/// EffiSuite 不执行插件代码（OpenClaw 运行时不同），仅记录元信息并提供卸载入口。
#[tauri::command]
pub(crate) async fn clawhub_install_plugin(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    name: String,
) -> Result<String, String> {
    // 幂等：若已安装则直接返回
    if let Some(existing) = state
        .plugin_store
        .find_by_name(&name)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(existing.id);
    }

    let client = state.clawhub.clone();
    let plugin_store = state.plugin_store.clone();
    // 使用 plugin_store 自身的 root，避免初始化回退到临时目录时
    // 元数据 JSON 与实际解压目录分离。
    let plugins_root = plugin_store.root().to_path_buf();

    // 1. 拉取包详情
    let detail = client
        .get_package(&name)
        .await
        .map_err(|e| format!("获取插件详情失败: {e}"))?;
    let pkg = detail
        .package
        .ok_or_else(|| format!("ClawHub 包 {} 不存在", name))?;
    let owner_handle = detail
        .owner
        .as_ref()
        .and_then(|o| o.handle.clone())
        .unwrap_or_else(|| pkg.owner_handle.clone().unwrap_or_default());

    // 2. 下载
    let zip_bytes = client
        .download_package(&name)
        .await
        .map_err(|e| format!("下载插件包失败: {e}"))?;

    // 3. 解压到 <plugins_dir>/<safe_id>/
    // safe_id 用 owner_handle/name 形式，与 InstalledPlugin.id 一致
    let safe_id = if owner_handle.is_empty() {
        pkg.name.clone()
    } else {
        format!("{}/{}", owner_handle, pkg.name)
    };
    let dest_dir = plugins_root.join(safe_id.replace('/', "__"));
    let dest_for_blocking = dest_dir.clone();
    tokio::task::spawn_blocking(move || extract_zip_to(&dest_for_blocking, &zip_bytes))
        .await
        .map_err(|e| format!("解压任务调度失败: {e}"))?
        .map_err(|e| format!("解压失败: {e}"))?;

    // 4. 落盘 plugin_store
    let plugin = InstalledPlugin {
        id: safe_id.clone(),
        name: pkg.name.clone(),
        display_name: pkg.display_name.clone(),
        summary: pkg.summary.clone().unwrap_or_default(),
        family: pkg.family.clone(),
        channel: pkg.channel.clone(),
        owner_handle,
        version: pkg.latest_version.clone().unwrap_or_default(),
        install_path: Some(dest_dir.to_string_lossy().into_owned()),
        installed_at: now_ms(),
    };
    plugin_store
        .save(&plugin)
        .await
        .map_err(|e| e.to_string())?;
    // 安装后同步插件命令为 agent 技能（幂等 upsert + 重建技能索引），
    // 使 agent 在 list_installed_skills 与 RAG 技能注入中看到新插件命令。
    sync_plugin_skills(
        state.plugin_store.clone(),
        state.skill_store.clone(),
        Arc::clone(&state.skill_index),
    )
    .await;
    let _ = app_handle.emit("plugins-changed", ());
    Ok(plugin.id)
}

/// 卸载 ClawHub 插件：删除 plugin_store 记录 + 解压目录 + 插件配置
#[tauri::command]
pub(crate) async fn clawhub_uninstall_plugin(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    state
        .plugin_store
        .delete(&id)
        .await
        .map_err(|e| e.to_string())?;
    // 清理插件配置（appdata/plugin_configs/<safe_id>.json）
    crate::commands::plugins::cleanup_plugin_config(&*state, &id).await;
    // 卸载后同步插件技能（清除该插件注册的技能并重建索引）
    sync_plugin_skills(
        state.plugin_store.clone(),
        state.skill_store.clone(),
        Arc::clone(&state.skill_index),
    )
    .await;
    let _ = app_handle.emit("plugins-changed", ());
    Ok(())
}

/// 列出本地已安装插件（按 installed_at 降序）

/// 列出本地已安装插件（按 installed_at 降序）
#[tauri::command]
pub(crate) async fn list_installed_plugins(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<InstalledPlugin>, String> {
    state.plugin_store.list().await.map_err(|e| e.to_string())
}

//! 技能（Skill）管理与会话工作区命令。
//!
//! 技能增删后通过 `rebuild_skill_index` 刷新 RAG 索引，
//! 确保下一轮 RAG 自动注入与 list_installed_skills 工具看到最新数据。

use effisuite_core::Skill;

use crate::state::{now_ms, AppState};

/// 列出全部技能：内置（agent-reach / browser-act）+ 用户自定义
#[tauri::command]
pub(crate) async fn list_skills(state: tauri::State<'_, AppState>) -> Result<Vec<Skill>, String> {
    state
        .skill_store
        .list_all()
        .await
        .map_err(|e| e.to_string())
}

/// 创建用户技能，返回 id。空 id 自动生成；强制 builtin=false
#[tauri::command]
pub(crate) async fn create_skill(
    state: tauri::State<'_, AppState>,
    mut skill: Skill,
) -> Result<String, String> {
    if skill.id.is_empty() {
        skill.id = uuid::Uuid::new_v4().to_string();
    }
    if skill.created_at == 0 {
        skill.created_at = now_ms();
    }
    skill.builtin = false;
    let id = skill.id.clone();
    state
        .skill_store
        .save(&skill)
        .await
        .map_err(|e| e.to_string())?;
    rebuild_skill_index(&state).await;
    Ok(id)
}

/// 更新用户技能（内置技能不可修改）。保留原 created_at，强制 builtin=false
#[tauri::command]
pub(crate) async fn update_skill(
    state: tauri::State<'_, AppState>,
    id: String,
    mut skill: Skill,
) -> Result<(), String> {
    let existing = state
        .skill_store
        .get(&id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("技能 {} 不存在", id))?;
    if existing.builtin {
        return Err("内置技能不可修改".to_string());
    }
    skill.id = id;
    skill.builtin = false;
    skill.created_at = existing.created_at;
    state
        .skill_store
        .save(&skill)
        .await
        .map_err(|e| e.to_string())?;
    rebuild_skill_index(&state).await;
    Ok(())
}

/// 删除技能；内置技能不可删除
#[tauri::command]
pub(crate) async fn delete_skill(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    if state
        .skill_store
        .get(&id)
        .await
        .map_err(|e| e.to_string())?
        .map(|s| s.builtin)
        .unwrap_or(false)
    {
        return Err("内置技能不可删除".to_string());
    }
    state
        .skill_store
        .delete(&id)
        .await
        .map_err(|e| e.to_string())?;
    rebuild_skill_index(&state).await;
    Ok(())
}

/// 重建技能 RAG 索引：从 SkillStore 全量加载并刷新 SkillIndex。
///
/// 在技能增删（create_skill / update_skill / delete_skill /
/// clawhub_install_skill / clawhub_uninstall_skill）后调用，确保下一轮
/// RAG 自动注入与 list_installed_skills 工具看到最新数据。
///
/// 失败仅记录日志不阻断主流程：索引短暂过期不影响已有技能可用性，
/// 下次增删或重启时会再次 rebuild 自愈。
pub(crate) async fn rebuild_skill_index(state: &AppState) {
    rebuild_skill_index_from(&state.skill_index, &state.skill_store).await;
}

/// 重建技能 RAG 索引：从 SkillStore 全量加载并刷新 SkillIndex。
///
/// 与 [`rebuild_skill_index`] 等价，但只依赖两个可 Clone 的存储句柄，
/// 可在 `'static` 后台任务（如应用启动时的插件技能同步）中调用，
/// 无需持有 `&AppState` 跨 await。
pub(crate) async fn rebuild_skill_index_from(
    skill_index: &effisuite_core::SkillIndex,
    skill_store: &effisuite_core::SkillStore,
) {
    if let Err(e) = skill_index.rebuild_from_store(skill_store).await {
        tracing::warn!(error = %e, "技能索引 rebuild 失败，将在下次增删或重启时重试");
    }
}

/// 设置/清除会话级工作区路径。
///
/// 传入 Some(path) 设置工作区，传入 None 清除（回退到技能级或进程默认）。
/// 设置后，该会话后续 send_message 时 read_file/list_files/shell 以此目录为基准。
/// 优先级：会话级 working_dir > 技能级 working_dir > 进程默认 cwd。
#[tauri::command]
pub(crate) async fn set_conversation_working_dir(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
    working_dir: Option<String>,
) -> Result<(), String> {
    state
        .store
        .set_working_dir(&conversation_id, working_dir)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 获取会话级工作区路径（None 表示未设置，将回退到技能级或进程默认）。
#[tauri::command]
pub(crate) async fn get_conversation_working_dir(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<Option<String>, String> {
    let conv = state
        .store
        .load(&conversation_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(conv.and_then(|c| c.working_dir))
}

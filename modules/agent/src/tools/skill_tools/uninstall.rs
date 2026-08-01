use std::sync::Arc;

use effisuite_core::{Skill, SkillIndex, SkillStore};
use rig_core::tool::Tool;
use serde::Deserialize;

use super::short_id;

// =========================================================
// UninstallSkillTool：卸载已安装技能
// =========================================================

#[derive(Deserialize)]
pub struct UninstallSkillArgs {
    /// 技能 id（完整或前 8 字符前缀）
    pub id: String,
}

#[derive(Debug, thiserror::Error)]
#[error("uninstall skill error: {0}")]
pub struct UninstallSkillError(String);

/// 卸载技能工具
///
/// 持有 `SkillStore` 与 `SkillIndex` 共享句柄。先定位技能，禁止删除内置技能，
/// 删除后重建索引，让下一轮 RAG 自动注入与 list_installed_skills 工具看到最新数据。
pub struct UninstallSkillTool {
    store: SkillStore,
    index: Arc<SkillIndex>,
}

impl UninstallSkillTool {
    pub fn new(store: SkillStore, index: Arc<SkillIndex>) -> Self {
        Self { store, index }
    }
}

impl Tool for UninstallSkillTool {
    const NAME: &'static str = "uninstall_skill";

    type Error = UninstallSkillError;
    type Args = UninstallSkillArgs;
    type Output = String;

    fn description(&self) -> String {
        "卸载已安装的技能。内置技能不可卸载；id 支持前 8 字符前缀匹配。\
         卸载后索引会自动重建，agent 将不再看到此技能。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "要卸载的技能 id（完整或前 8 字符前缀）"
                }
            },
            "required": ["id"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let target = args.id.trim();
        if target.is_empty() {
            return Err(UninstallSkillError("id 不能为空".to_string()));
        }

        // 定位技能：先精确匹配，再前缀匹配
        let skill = match self.store.get(target).await {
            Ok(Some(s)) => Some(s),
            _ => {
                let all = self
                    .store
                    .list_all()
                    .await
                    .map_err(|e| UninstallSkillError(e.to_string()))?;
                let prefix_match: Vec<&Skill> =
                    all.iter().filter(|s| s.id.starts_with(target)).collect();
                match prefix_match.len() {
                    1 => Some(prefix_match[0].clone()),
                    0 => all.into_iter().find(|s| s.id == target),
                    _ => None,
                }
            }
        };

        let Some(skill) = skill else {
            return Ok(format!(
                "未找到 id 包含「{}」的技能。可调用 list_installed_skills 查看全部已安装技能。",
                target
            ));
        };

        if skill.builtin {
            return Err(UninstallSkillError(format!(
                "技能「{}」（id={}）是内置技能，不可卸载",
                skill.name,
                short_id(&skill.id)
            )));
        }

        let name = skill.name.clone();
        let short = short_id(&skill.id);

        self.store
            .delete(&skill.id)
            .await
            .map_err(|e| UninstallSkillError(e.to_string()))?;
        self.index
            .rebuild_from_store(&self.store)
            .await
            .map_err(|e| UninstallSkillError(e.to_string()))?;

        Ok(format!(
            "已卸载技能「{}」（id={}）。索引已重建，agent 将不再看到此技能。",
            name, short
        ))
    }
}

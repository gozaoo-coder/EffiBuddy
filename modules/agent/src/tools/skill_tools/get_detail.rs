use effisuite_core::{Skill, SkillStore};
use rig_core::tool::Tool;
use serde::Deserialize;

use super::short_id;

// =========================================================
// GetSkillDetailTool：按 id 获取技能完整 preamble + working_dir
// =========================================================

#[derive(Deserialize)]
pub struct GetSkillDetailArgs {
    /// 技能 id（完整或前 8 字符前缀）
    pub id: String,
}

#[derive(Debug, thiserror::Error)]
#[error("get skill detail error: {0}")]
pub struct GetSkillDetailError(String);

/// 获取技能详情工具
///
/// 持有 `SkillStore` 共享句柄，按 id 读取完整 Skill（含 preamble / working_dir）。
/// agent 据此判断"如何使用这个技能"，并可用 read_file / list_files /
/// shell 工具访问 working_dir 下的技能资源（脚本、配置等）。
pub struct GetSkillDetailTool {
    store: SkillStore,
}

impl GetSkillDetailTool {
    pub fn new(store: SkillStore) -> Self {
        Self { store }
    }
}

impl Tool for GetSkillDetailTool {
    const NAME: &'static str = "get_skill_detail";

    type Error = GetSkillDetailError;
    type Args = GetSkillDetailArgs;
    type Output = String;

  fn description(&self) -> String {
      "按 id 获取已安装技能的完整说明（preamble 全文 + 工作目录）。\
       准备使用某技能或需了解完整指令时调用；id 支持前 8 字符前缀匹配。"
          .to_string()
  }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "技能 id（完整或前 8 字符前缀）"
                }
            },
            "required": ["id"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let target = args.id.trim();
        if target.is_empty() {
            return Err(GetSkillDetailError("id 不能为空".to_string()));
        }

        // 先精确匹配；不中再前缀匹配（与 DeletePinnedMemoryTool 一致）
        let skill = match self.store.get(target).await {
            Ok(Some(s)) => Some(s),
            _ => {
                let all = self
                    .store
                    .list_all()
                    .await
                    .map_err(|e| GetSkillDetailError(e.to_string()))?;
                // 前缀匹配优先；若前缀匹配多条则退化为精确匹配
                let prefix_match: Vec<&Skill> =
                    all.iter().filter(|s| s.id.starts_with(target)).collect();
                match prefix_match.len() {
                    1 => Some(prefix_match[0].clone()),
                    0 => all.into_iter().find(|s| s.id == target),
                    _ => {
                        // 多条前缀匹配：返回 None 让上层走"未找到"分支，
                        // 引导用户提供更长 id
                        None
                    }
                }
            }
        };

        let Some(skill) = skill else {
            return Ok(format!(
                "未找到 id 包含「{}」的技能。可调用 list_installed_skills 查看全部已安装技能。",
                target
            ));
        };

        let mut out = String::with_capacity(skill.preamble.len() + 256);
        out.push_str(&format!("技能：{}（id={}）\n", skill.name, short_id(&skill.id)));
        out.push_str(&format!("简介：{}\n", skill.description));
        if skill.builtin {
            out.push_str("类型：内置\n");
        } else if skill.source.as_deref() == Some("clawhub") {
            out.push_str("来源：ClawHub\n");
        }
        if let Some(wd) = skill.working_dir.as_deref() {
            out.push_str(&format!("工作目录：{}\n", wd));
            out.push_str("（可用 read_file / list_files / shell 工具访问此目录下的资源）\n");
        }
        out.push_str("\n--- 技能指令（preamble）---\n");
        if skill.preamble.is_empty() {
            out.push_str("（preamble 为空，技能无显式指令；可能仅提供工作目录资源）\n");
        } else {
            out.push_str(&skill.preamble);
            if !skill.preamble.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push_str("\n--- 结束 ---\n");
        out.push_str("如需把此技能指令注入会话上下文以便后续对话遵循，调用 enable_skill(id)。");
        Ok(out)
    }
}

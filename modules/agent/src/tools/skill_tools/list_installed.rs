use std::sync::Arc;

use effisuite_core::SkillIndex;
use rig_core::tool::Tool;
use serde::Deserialize;

use super::short_id;

// =========================================================
// ListInstalledSkillsTool：列出所有已安装技能
// =========================================================

/// 无参数（rig 要求 Args 类型，用空 struct + 自定义 deserialize）
#[derive(Deserialize, Default)]
pub struct ListInstalledSkillsArgs {}

/// 列表工具错误
#[derive(Debug, thiserror::Error)]
#[error("list installed skills error: {0}")]
pub struct ListInstalledSkillsError(String);

/// 列出所有已安装技能工具
///
/// 持有 `SkillIndex` 共享句柄（与 RigAgent 共享同一份索引快照）。
/// 走索引读路径，零 IO；技能增删后由 Tauri 命令层 rebuild 索引。
pub struct ListInstalledSkillsTool {
    index: Arc<SkillIndex>,
}

impl ListInstalledSkillsTool {
    pub fn new(index: Arc<SkillIndex>) -> Self {
        Self { index }
    }
}

impl Tool for ListInstalledSkillsTool {
    const NAME: &'static str = "list_installed_skills";

    type Error = ListInstalledSkillsError;
    type Args = ListInstalledSkillsArgs;
    type Output = String;

    fn description(&self) -> String {
        "列出当前已安装的全部技能（id/名称/简介/是否内置）。\
         在需要了解本地有哪些可用能力、或准备深入使用某个技能前调用。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let list = self.index.list_all().await;
        if list.is_empty() {
            return Ok("当前没有任何已安装技能。可调用 search_clawhub_skills 从 ClawHub 查找并安装。".to_string());
        }
        let mut out = String::with_capacity(list.len() * 96);
        out.push_str(&format!("当前共 {} 个已安装技能：\n", list.len()));
        for (i, s) in list.iter().enumerate() {
            let tag = if s.builtin { "[内置]" } else { "" };
            out.push_str(&format!(
                "{}. (id={}) {}{} — {}\n",
                i + 1,
                short_id(&s.id),
                tag,
                s.name,
                s.description
            ));
        }
        out.push_str("\n提示：调用 get_skill_detail(id) 可获取技能完整说明；调用 enable_skill(id) 把技能指令注入会话上下文。");
        Ok(out)
    }
}

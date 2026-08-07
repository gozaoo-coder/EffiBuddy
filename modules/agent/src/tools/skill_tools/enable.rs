use std::sync::Arc;

use effisuite_core::{ConversationStore, Message, Role, Skill, SkillStore};
use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::sync::RwLock;

use super::{now_ms, short_id};

// =========================================================
// EnableSkillTool：把技能 preamble 注入当前会话上下文
// =========================================================

#[derive(Deserialize)]
pub struct EnableSkillArgs {
    /// 技能 id（完整或前 8 字符前缀）
    pub id: String,
}

#[derive(Debug, thiserror::Error)]
#[error("enable skill error: {0}")]
pub struct EnableSkillError(String);

/// 启用技能工具
///
/// 持有：
/// - `SkillStore`：读取技能 preamble（ClawHub 技能 preamble 为 SKILL.md 正文）
/// - `ConversationStore`：把 preamble 作为 System 消息追加到当前会话历史
/// - `current_conversation_id`：当前会话 id 句柄
///
/// 替代旧 apply_skill 命令；用户不再需要手动点击应用，由 agent 主动调用。
/// 注入后，后续对话的 prompt 会包含此技能指令，agent 据此"使用"技能。
pub struct EnableSkillTool {
    store: SkillStore,
    conversation_store: Arc<ConversationStore>,
    current_conversation_id: Arc<RwLock<Option<String>>>,
}

impl EnableSkillTool {
    pub fn new(
        store: SkillStore,
        conversation_store: Arc<ConversationStore>,
        current_conversation_id: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            store,
            conversation_store,
            current_conversation_id,
        }
    }
}

impl Tool for EnableSkillTool {
    const NAME: &'static str = "enable_skill";

    type Error = EnableSkillError;
    type Args = EnableSkillArgs;
    type Output = String;

  fn description(&self) -> String {
      "启用指定技能：把技能指令（preamble）注入当前会话上下文，后续对话遵循其行事。\
       需要使用某技能或用户要求启用时调用；id 支持前 8 字符前缀匹配。"
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
            return Err(EnableSkillError("id 不能为空".to_string()));
        }

        // 解析当前会话 id
        let conv_id = self.current_conversation_id.read().await.clone();
        let Some(conv_id) = conv_id else {
            return Err(EnableSkillError("当前没有活动会话，无法启用技能".to_string()));
        };

        // 加载技能（先精确，后前缀）
        let skill = match self.store.get(target).await {
            Ok(Some(s)) => Some(s),
            _ => {
                let all = self
                    .store
                    .list_all()
                    .await
                    .map_err(|e| EnableSkillError(e.to_string()))?;
                all.into_iter().find(|s| s.id.starts_with(target))
            }
        };
        let Some(skill) = skill else {
            return Ok(format!(
                "未找到 id 包含「{}」的技能。可调用 list_installed_skills 查看全部已安装技能。",
                target
            ));
        };

        // 工作目录注入：若技能配置了 working_dir 且会话级 working_dir 未设置，写入会话级
        // （与旧 apply_skill 行为一致，agent 据此访问技能资源）
        if let Some(skill_wd) = skill.working_dir.clone() {
            let conv = self
                .conversation_store
                .load(&conv_id)
                .await
                .map_err(|e| EnableSkillError(e.to_string()))?;
            let need_set = conv
                .as_ref()
                .map(|c| c.working_dir.is_none())
                .unwrap_or(true);
            if need_set {
                self.conversation_store
                    .set_working_dir(&conv_id, Some(skill_wd))
                    .await
                    .map_err(|e| EnableSkillError(e.to_string()))?;
            }
        }

        // 解析 preamble：优先持久化值；为空且 working_dir 含 SKILL.md 时回读
        // （兼容此修复前安装的 ClawHub 技能 + 支持外部编辑 SKILL.md 热更新）
        let preamble = resolve_preamble(&skill).await;
        if preamble.is_empty() {
            return Ok(format!(
                "技能「{}」无可用指令（preamble 为空），仅工作目录已注入会话。",
                skill.name
            ));
        }

        // 把 preamble 作为 System 消息追加到会话历史
        let sys_msg = Message::new(
            effisuite_core::gen_message_id(),
            Role::System,
            preamble,
            now_ms(),
        );
        self.conversation_store
            .append_message(&conv_id, sys_msg, now_ms())
            .await
            .map_err(|e| EnableSkillError(e.to_string()))?;

        Ok(format!(
            "已启用技能「{}」（id={}）。技能指令已注入会话上下文，后续对话将遵循此指令。{}",
            skill.name,
            short_id(&skill.id),
            if skill.working_dir.is_some() {
                "\n工作目录也已注入，可用 read_file / list_files / shell 访问技能资源。"
            } else {
                ""
            }
        ))
    }
}

/// 解析技能最终要注入到会话的 preamble 文本。
///
/// 优先级：
/// 1. `skill.preamble` 非空 → 直接返回
/// 2. `skill.working_dir` 下存在 `SKILL.md` → 现读并返回其正文
/// 3. 否则返回空串
pub(super) async fn resolve_preamble(skill: &Skill) -> String {
    if !skill.preamble.is_empty() {
        return skill.preamble.clone();
    }
    let Some(wd) = skill.working_dir.as_deref() else {
        return String::new();
    };
    let skill_md = std::path::Path::new(wd).join("SKILL.md");
    let content = match tokio::fs::read_to_string(&skill_md).await {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    effisuite_core::clawhub::parse_skill_md(&content).body
}

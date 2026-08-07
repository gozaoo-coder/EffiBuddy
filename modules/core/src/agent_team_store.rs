//! Agent Team（智能体群组）持久化存储
//!
//! 用户可创建多个「群聊」（像微信一样），群内成员 = 用户 + 自定义智能体
//! （AgentDef）或主 agent。管理员（owner/admin）可移除成员、颁布任务、
//! 接收并监督各 agent 状态；群内消息支持 @ 提及某个 agent，被提及 / 被拉入
//! 的 agent 会收到消息并选择是否回复。
//!
//! 基于 JSON 文件：每个群一个文件，存放在 `appdata/agent_teams/<id>.json`。
//! 模式与 [`crate::schedule_store::ScheduledTaskStore`] 一致。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{CoreError, Result};

/// 群成员角色
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamRole {
    /// 群主（拥有移除群 / 颁任务等全部权限）
    Owner,
    /// 管理员（可移除成员、颁任务、监督状态）
    Admin,
    /// 普通成员
    Member,
}

impl TeamRole {
    /// 是否具备管理员权限（owner 或 admin）
    pub fn is_admin(&self) -> bool {
        matches!(self, TeamRole::Owner | TeamRole::Admin)
    }
}

/// 群成员类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamMemberKind {
    /// 用户本人
    User,
    /// 自定义智能体（agent_def_id 指向 AgentDef）
    Agent,
    /// 主 agent
    MainAgent,
}

/// 群成员
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    /// 成员唯一 id（用户 = `user:me`；智能体 = `def:<id>`；主 agent = `main`）
    pub id: String,
    /// 显示名
    pub name: String,
    /// 头像 emoji
    #[serde(default = "default_member_avatar")]
    pub avatar: String,
    /// 成员类型
    pub kind: TeamMemberKind,
    /// 角色
    pub role: TeamRole,
    /// 仅 kind=Agent 时：关联的自定义智能体定义 id
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_def_id: Option<String>,
    pub joined_at: u64,
}

fn default_member_avatar() -> String {
    "🙂".to_string()
}

/// 群消息类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamMessageKind {
    /// 普通聊天文本
    Text,
    /// 系统消息（成员加入/移除等）
    System,
    /// 管理员颁布的任务
    Task,
}

/// 群消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMessage {
    pub id: String,
    /// 发送方成员 id
    pub sender_id: String,
    /// 发送方显示名
    pub sender_name: String,
    /// 发送方头像 emoji
    #[serde(default)]
    pub sender_avatar: String,
    /// 消息类型
    pub kind: TeamMessageKind,
    /// 消息内容
    pub content: String,
    /// 被 @ 的成员 id 列表（用于把消息推送给这些 agent）
    #[serde(default)]
    pub mentions: Vec<String>,
    /// 任务状态（仅 kind=Task）：受理 agent 是否已回复
    #[serde(default)]
    pub task_handled: bool,
    /// 任务受理 agent 的回复（已回复时有值）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    pub created_at: u64,
}

/// 智能体群组（Agent Team）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTeam {
    /// 群唯一 id
    pub id: String,
    /// 群名
    pub name: String,
    /// 群描述
    pub description: String,
    /// 群主成员 id
    pub owner_id: String,
    /// 群成员列表（含用户 + 智能体）
    pub members: Vec<TeamMember>,
    /// 群消息（按时间戳升序）
    pub messages: Vec<TeamMessage>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl AgentTeam {
    /// 查询成员（按 id）
    pub fn member(&self, id: &str) -> Option<&TeamMember> {
        self.members.iter().find(|m| m.id == id)
    }

    /// 该成员是否具备管理员权限
    pub fn is_admin(&self, member_id: &str) -> bool {
        self.member(member_id).map(|m| m.role.is_admin()).unwrap_or(false)
    }
}

/// 智能体群组存储，线程安全可廉价 clone（内部 RwLock + Arc）
#[derive(Clone)]
pub struct AgentTeamStore {
    root: PathBuf,
    _lock: std::sync::Arc<RwLock<()>>,
}

impl AgentTeamStore {
    /// 创建存储，root 不存在时自动创建
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        Ok(Self {
            root,
            _lock: std::sync::Arc::new(RwLock::new(())),
        })
    }

    /// 群文件路径：`<root>/<id>.json`
    #[inline]
    fn path_for(&self, id: &str) -> PathBuf {
        let safe = Path::new(id)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(id));
        self.root.join(safe).with_extension("json")
    }

    /// 列出全部群，按 updated_at 降序
    pub async fn list(&self) -> Result<Vec<AgentTeam>> {
        let mut entries = tokio::fs::read_dir(&self.root)
            .await
            .map_err(CoreError::Io)?;
        let mut out = Vec::with_capacity(4);
        while let Some(entry) = entries.next_entry().await.map_err(CoreError::Io)? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(bytes) = tokio::fs::read(&path).await else {
                continue;
            };
            let Ok(team) = serde_json::from_slice::<AgentTeam>(&bytes) else {
                continue;
            };
            out.push(team);
        }
        out.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
        Ok(out)
    }

    /// 加载单个群，不存在返回 None
    pub async fn get(&self, id: &str) -> Result<Option<AgentTeam>> {
        let path = self.path_for(id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = tokio::fs::read(&path).await.map_err(CoreError::Io)?;
        let team: AgentTeam = serde_json::from_slice(&bytes).map_err(CoreError::Serde)?;
        Ok(Some(team))
    }

    /// 保存（或覆盖）一个群
    pub async fn save(&self, team: &AgentTeam) -> Result<()> {
        let path = self.path_for(&team.id);
        let bytes = serde_json::to_vec(team).map_err(CoreError::Serde)?;
        tokio::fs::write(&path, bytes)
            .await
            .map_err(CoreError::Io)?;
        Ok(())
    }

    /// 删除指定群，不存在返回 Ok(())
    pub async fn delete(&self, id: &str) -> Result<()> {
        let path = self.path_for(id);
        if path.exists() {
            tokio::fs::remove_file(&path).await.map_err(CoreError::Io)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("effisuite-team-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn team(id: &str) -> AgentTeam {
        AgentTeam {
            id: id.to_string(),
            name: "研发群".to_string(),
            description: String::new(),
            owner_id: "user:me".to_string(),
            members: vec![TeamMember {
                id: "user:me".to_string(),
                name: "我".to_string(),
                avatar: "🙂".to_string(),
                kind: TeamMemberKind::User,
                role: TeamRole::Owner,
                agent_def_id: None,
                joined_at: 1,
            }],
            messages: Vec::new(),
            created_at: 1,
            updated_at: 1,
        }
    }

    #[tokio::test]
    async fn save_list_get_delete() {
        let store = AgentTeamStore::new(tmp_dir()).unwrap();
        store.save(&team("t1")).await.unwrap();
        assert_eq!(store.list().await.unwrap().len(), 1);
        let got = store.get("t1").await.unwrap().unwrap();
        assert_eq!(got.name, "研发群");
        assert!(got.is_admin("user:me"));
        store.delete("t1").await.unwrap();
        assert!(store.get("t1").await.unwrap().is_none());
    }
}
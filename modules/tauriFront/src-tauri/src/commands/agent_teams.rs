//! Agent Team（智能体群组）命令：群聊、成员管理、任务颁布、agent 自动回复
//!
//! 支持：
//! - 群 CRUD（创建 / 列表 / 详情 / 删除），像微信一样可建多个群聊
//! - 成员管理：把自定义智能体（AgentDef）或主 agent 拉入群、移除成员
//! - 群聊：用户发消息（支持 @ 提及），被提及/被拉入的 agent 收到后选择是否回复
//! - 管理员（owner/admin）：颁布任务、移除成员、监督各 agent 状态
//! - agent 自动回复：后台异步调用模型（call_model_by_id），注入自定义智能体的
//!   系统提示词与模型，回复以新消息追加进群，并 emit `agent-team-event` 到前端
//!
//! 本模块不持有长临界区锁：读改写均在 store（内部 Arc）上短锁完成，模型调用
//! 在锁外的 spawn 任务中执行，遵循"消息传递代替共享内存"。

use std::sync::Arc;

use effisuite_agent::tools::call_model::call_model_by_id;
use effisuite_core::{
    AgentConfig, AgentDefStore, AgentTeam, AgentTeamStore, TeamMember, TeamMemberKind,
    TeamMessage, TeamMessageKind, TeamRole,
};
use tauri::{Emitter, State};
use tokio::sync::RwLock;

use crate::state::{now_ms, AppState};

/// 群内"用户本人"的固定成员 id
pub(crate) const USER_MEMBER_ID: &str = "user:me";
/// 主 agent 的固定成员 id
const MAIN_AGENT_ID: &str = "main";

/// 创建（或更新）一个群。members 传入要拉入的智能体成员描述。
#[tauri::command]
pub(crate) async fn save_agent_team(
    state: State<'_, AppState>,
    team: AgentTeam,
) -> Result<AgentTeam, String> {
    let now = now_ms();
    let mut team = team;
    if team.id.is_empty() {
        team.id = uuid::Uuid::new_v4().to_string();
        team.created_at = now;
        // 新群默认包含用户本人（owner）
        if team.owner_id.is_empty() {
            team.owner_id = USER_MEMBER_ID.to_string();
        }
        if !team
            .members
            .iter()
            .any(|m| m.id == USER_MEMBER_ID)
        {
            team.members.insert(
                0,
                TeamMember {
                    id: USER_MEMBER_ID.to_string(),
                    name: "我".to_string(),
                    avatar: "🙂".to_string(),
                    kind: TeamMemberKind::User,
                    role: TeamRole::Owner,
                    agent_def_id: None,
                    joined_at: now,
                },
            );
        }
    }
    team.updated_at = now;
    state
        .agent_team_store
        .save(&team)
        .await
        .map_err(|e| e.to_string())?;
    Ok(team)
}

/// 列出全部群（按 updated_at 降序）
#[tauri::command]
pub(crate) async fn list_agent_teams(
    state: State<'_, AppState>,
) -> Result<Vec<AgentTeam>, String> {
    state
        .agent_team_store
        .list()
        .await
        .map_err(|e| e.to_string())
}

/// 获取单个群详情
#[tauri::command]
pub(crate) async fn get_agent_team(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<AgentTeam>, String> {
    state.agent_team_store.get(&id).await.map_err(|e| e.to_string())
}

/// 删除一个群（管理员可删；非 owner 亦可由调用方校验）
#[tauri::command]
pub(crate) async fn delete_agent_team(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state
        .agent_team_store
        .delete(&id)
        .await
        .map_err(|e| e.to_string())
}

/// 添加成员到群。member_id 取值：`main`（主 agent）或 `def:<agent_def_id>`。
/// role：owner / admin / member。
#[tauri::command]
pub(crate) async fn add_team_member(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    team_id: String,
    member_id: String,
    name: String,
    avatar: String,
    role: TeamRole,
) -> Result<AgentTeam, String> {
    let now = now_ms();
    let mut team = state
        .agent_team_store
        .get(&team_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("群 {team_id} 不存在"))?;

    if team.members.iter().any(|m| m.id == member_id) {
        return Err(format!("成员 {name} 已在群中"));
    }
    let kind = if member_id == MAIN_AGENT_ID {
        TeamMemberKind::MainAgent
    } else if let Some(def_id) = member_id.strip_prefix("def:") {
        // 校验自定义智能体存在
        if state
            .agent_def_store
            .get(def_id)
            .await
            .map_err(|e| e.to_string())?
            .is_none()
        {
            return Err(format!("自定义智能体 {def_id} 不存在"));
        }
        TeamMemberKind::Agent
    } else {
        return Err(format!("member_id 非法：{member_id}（应为 main 或 def:<id>）"));
    };

    team.members.push(TeamMember {
        id: member_id.clone(),
        name: name.clone(),
        avatar,
        kind,
        role,
        agent_def_id: member_id.strip_prefix("def:").map(str::to_string),
        joined_at: now,
    });
    team.updated_at = now;
    append_system_message(&mut team, &format!("成员 {name} 已加入群聊"));
    state
        .agent_team_store
        .save(&team)
        .await
        .map_err(|e| e.to_string())?;
    let _ = app.emit("agent-team-event", build_event(&team, team_id, "updated"));
    Ok(team)
}

/// 从群移除成员（管理员可移除普通成员；不允许移除 owner 本人）
#[tauri::command]
pub(crate) async fn remove_team_member(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    team_id: String,
    member_id: String,
) -> Result<AgentTeam, String> {
    let mut team = state
        .agent_team_store
        .get(&team_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("群 {team_id} 不存在"))?;

    if member_id == team.owner_id {
        return Err("不能移除群主".to_string());
    }
    let removed = team.members.iter().find(|m| m.id == member_id).cloned();
    team.members.retain(|m| m.id != member_id);
    if let Some(m) = removed {
        append_system_message(&mut team, &format!("成员 {} 已移出群聊", m.name));
    }
    team.updated_at = now_ms();
    state
        .agent_team_store
        .save(&team)
        .await
        .map_err(|e| e.to_string())?;
    let _ = app.emit(
        "agent-team-event",
        build_event(&team, team_id.clone(), "updated"),
    );
    Ok(team)
}

/// 用户发送群聊消息（支持 @ 提及）。returns 收到该消息的群。
/// 若消息 @ 了某个 agent，或 kind=Task，后台会触发这些 agent 自动回复。
#[tauri::command]
pub(crate) async fn send_team_message(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    team_id: String,
    content: String,
    mentions: Vec<String>,
    kind: Option<TeamMessageKind>,
) -> Result<AgentTeam, String> {
    let content = content.trim().to_string();
    if content.is_empty() {
        return Err("消息内容不能为空".to_string());
    }
    let now = now_ms();
    let mut team = state
        .agent_team_store
        .get(&team_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("群 {team_id} 不存在"))?;

    let user = team
        .member(USER_MEMBER_ID)
        .cloned()
        .unwrap_or_else(|| TeamMember {
            id: USER_MEMBER_ID.to_string(),
            name: "我".to_string(),
            avatar: "🙂".to_string(),
            kind: TeamMemberKind::User,
            role: TeamRole::Owner,
            agent_def_id: None,
            joined_at: now,
        });
    let msg_kind = kind.unwrap_or(TeamMessageKind::Text);
    let mut mentions = mentions;
    mentions.retain(|m| m != USER_MEMBER_ID && team.members.iter().any(|x| x.id == *m));
    let msg = TeamMessage {
        id: uuid::Uuid::new_v4().to_string(),
        sender_id: user.id.clone(),
        sender_name: user.name.clone(),
        sender_avatar: user.avatar.clone(),
        kind: msg_kind,
        content: content.clone(),
        mentions: mentions.clone(),
        task_handled: false,
        reply: None,
        created_at: now,
    };
    team.messages.push(msg);
    team.updated_at = now;
    state
        .agent_team_store
        .save(&team)
        .await
        .map_err(|e| e.to_string())?;
    let _ = app.emit(
        "agent-team-event",
        build_event(&team, team_id.clone(), "message"),
    );

    // 触发相关 agent 回复（锁外异步，不阻塞消息返回）
    spawn_agent_replies(
        app,
        state.agent_team_store.clone(),
        state.agent_def_store.clone(),
        Arc::clone(&state.config),
        team_id,
        content,
        mentions,
        msg_kind,
    );

    Ok(team)
}

/// 管理员颁布任务：给指定成员（或 @ 全体 agent）下达任务。
/// 任务以 Task 消息入群，被指定的 agent 自动受理并回复。
#[tauri::command]
pub(crate) async fn assign_team_task(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    team_id: String,
    content: String,
    assignees: Vec<String>,
) -> Result<AgentTeam, String> {
    // 校验调用方是管理员（owner 或 admin）
    let mut team = state
        .agent_team_store
        .get(&team_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("群 {team_id} 不存在"))?;
    if !team.is_admin(USER_MEMBER_ID) {
        return Err("仅管理员可颁布任务".to_string());
    }
    let content = content.trim().to_string();
    if content.is_empty() {
        return Err("任务内容不能为空".to_string());
    }
    let now = now_ms();
    let user = team
        .member(USER_MEMBER_ID)
        .cloned()
        .unwrap_or_else(|| TeamMember {
            id: USER_MEMBER_ID.to_string(),
            name: "我".to_string(),
            avatar: "🙂".to_string(),
            kind: TeamMemberKind::User,
            role: TeamRole::Owner,
            agent_def_id: None,
            joined_at: now,
        });
    // assignees 为空则视为 @ 给全体 agent 成员
    let assignees: Vec<String> = if assignees.is_empty() {
        team.members
            .iter()
            .filter(|m| m.kind != TeamMemberKind::User)
            .map(|m| m.id.clone())
            .collect()
    } else {
        let valid: Vec<String> = assignees
            .into_iter()
            .filter(|a| team.members.iter().any(|m| m.id == *a))
            .collect();
        if valid.is_empty() {
            return Err("未指定任何有效成员".to_string());
        }
        valid
    };
    let msg = TeamMessage {
        id: uuid::Uuid::new_v4().to_string(),
        sender_id: user.id.clone(),
        sender_name: user.name.clone(),
        sender_avatar: user.avatar.clone(),
        kind: TeamMessageKind::Task,
        content: content.clone(),
        mentions: assignees.clone(),
        task_handled: false,
        reply: None,
        created_at: now,
    };
    team.messages.push(msg);
    team.updated_at = now;
    state
        .agent_team_store
        .save(&team)
        .await
        .map_err(|e| e.to_string())?;
    let _ = app.emit(
        "agent-team-event",
        build_event(&team, team_id.clone(), "task"),
    );

    spawn_agent_replies(
        app,
        state.agent_team_store.clone(),
        state.agent_def_store.clone(),
        Arc::clone(&state.config),
        team_id,
        content,
        assignees,
        TeamMessageKind::Task,
    );
    Ok(team)
}

/// 查询群内各 agent 状态快照（成员列表 + 最近活跃）
#[tauri::command]
pub(crate) async fn get_team_status(
    state: State<'_, AppState>,
    team_id: String,
) -> Result<serde_json::Value, String> {
    let team = state
        .agent_team_store
        .get(&team_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("群 {team_id} 不存在"))?;
    let last_msg_at = team.messages.last().map(|m| m.created_at).unwrap_or(0);
    let members: Vec<serde_json::Value> = team
        .members
        .iter()
        .map(|m| {
            let last_by = team
                .messages
                .iter()
                .rev()
                .find(|x| x.sender_id == m.id)
                .map(|x| x.created_at)
                .unwrap_or(0);
            serde_json::json!({
                "id": m.id,
                "name": m.name,
                "avatar": m.avatar,
                "kind": format!("{:?}", m.kind),
                "role": format!("{:?}", m.role),
                "last_active": last_by,
            })
        })
        .collect();
    Ok(serde_json::json!({
        "id": team.id,
        "name": team.name,
        "last_msg_at": last_msg_at,
        "members": members,
    }))
}

/// 群消息前面的系统消息（成员加入/移除）
fn append_system_message(team: &mut AgentTeam, content: &str) {
    team.messages.push(TeamMessage {
        id: uuid::Uuid::new_v4().to_string(),
        sender_id: "system".to_string(),
        sender_name: "系统".to_string(),
        sender_avatar: "🛠️".to_string(),
        kind: TeamMessageKind::System,
        content: content.to_string(),
        mentions: Vec::new(),
        task_handled: false,
        reply: None,
        created_at: now_ms(),
    });
}

/// 触发被 @ 的 agent 自动回复（异步，锁外执行模型调用）。
/// 对每个被指定的 agent 成员：
/// 1. 从 agent_def_store 取自定义智能体定义（含系统提示词 + 模型）
/// 2. 用 call_model_by_id 单轮调用生成回复
/// 3. 把回复作为新消息追加进群，并 emit 事件
fn spawn_agent_replies(
    app: tauri::AppHandle,
    team_store: AgentTeamStore,
    def_store: AgentDefStore,
    config: Arc<RwLock<Arc<AgentConfig>>>,
    team_id: String,
    content: String,
    mentions: Vec<String>,
    kind: TeamMessageKind,
) {
    tauri::async_runtime::spawn(async move {
        // 读到群并收集被指定的 agent 成员（含模型信息）
        let targets: Vec<TeamMember> = {
            let Ok(Some(team)) = team_store.get(&team_id).await else {
                return;
            };
            team.members
                .into_iter()
                .filter(|m| m.kind != TeamMemberKind::User && mentions.iter().any(|id| id == &m.id))
                .collect()
        };
        if targets.is_empty() {
            return;
        }
        for m in targets {
            let def_id = match &m.agent_def_id {
                Some(d) => d.clone(),
                None => continue, // 主 agent 或未知，跳过自动回复
            };
            let Ok(Some(def)) = def_store.get(&def_id).await else {
                continue;
            };
            // 组装提示词：任务/提及上下文 → 该 agent
            let prompt = if kind == TeamMessageKind::Task {
                format!(
                    "管理员向你在群「{}」颁布了任务，请受理并执行：\n{content}\n\
                     完成后给出简洁的最终答复（作为你的任务回复）。",
                    team_id
                )
            } else {
                format!(
                    "你在群「{}」中收到一条消息，成员 @ 了你：\n{content}\n\
                     请判断是否需要回复：若与你职责相关请回复；若无关可选择简短回应或说明暂不参与。",
                    team_id
                )
            };
            let reply = call_model_by_id(&config, def.model_id.as_deref(), &def.system_prompt, &prompt)
                .await;

            // 追加回复到群
            let mut team = match team_store.get(&team_id).await {
                Ok(Some(t)) => t,
                _ => continue,
            };
            let reply_text = match reply {
                Ok(t) => t,
                Err(e) => format!("（{name} 回复失败：{e}）", name = m.name),
            };
            let now = now_ms();
            team.messages.push(TeamMessage {
                id: uuid::Uuid::new_v4().to_string(),
                sender_id: m.id.clone(),
                sender_name: m.name.clone(),
                sender_avatar: m.avatar.clone(),
                kind: TeamMessageKind::Text,
                content: reply_text.clone(),
                mentions: Vec::new(),
                task_handled: kind == TeamMessageKind::Task,
                reply: if kind == TeamMessageKind::Task {
                    Some(reply_text)
                } else {
                    None
                },
                created_at: now,
            });
            team.updated_at = now;
            if team_store.save(&team).await.is_ok() {
                let _ = app.emit(
                    "agent-team-event",
                    build_event(&team, team_id.clone(), "reply"),
                );
            }
        }
    });
}

/// 构造前端事件负载
fn build_event(
    team: &AgentTeam,
    _team_id: String,
    event_type: &str,
) -> serde_json::Value {
    serde_json::json!({
        "type": event_type,
        "team": team,
    })
}
//! 运行时 agent 公共会话交流池工具集
//!
//! 让 agent 在多会话并行运行时具备跨会话协作能力：
//! - [`PoolReportTool`]（`pool_report`）：长任务登记 / 上报（任务 + 研究报告 +
//!   todoTree 摘要 + 状态：进行中 / 等待中 / 已完成）
//! - [`PoolLookupTool`]（`pool_lookup`）：查询是否有其他 agent 正在处理同一事项，
//!   仅返回进行中 / 等待中的活跃条目（已完成对新查询不可见）
//! - [`PoolAtTool`]（`pool_at`）：`@` 某个长任务 agent 询问状态 / 冲突。
//!   `mode=async` 投递后继续干自己的；`mode=await` 轮询等待对方回复后再决策。
//! - [`PoolReplyTool`]（`pool_reply`）：目标 agent 回复收件箱中的 @ 消息
//!
//! 工具的身份解析：
//! - 主 agent：`agent_id = conv:<conversation_id>`，显示名取会话标题
//! - 子 agent：`agent_id = sa:<session_id>`，显示名由 SubAgentManager 传入
//!
//! 所有变更会经事件总线发布 `agent-pool-updated`，前端据此刷新会话列表状态。

use std::sync::Arc;
use std::time::Duration;

use effisuite_core::{BusEvent, ConversationStore, EventBus};
use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::agent_pool::{
    AgentPoolStore, AtStatus, PoolEntry, PoolKind, PoolStatus, format_lookup_result,
    format_pool_section,
};

/// 工具共享上下文：身份 + 存储 + 事件总线
#[derive(Clone)]
pub struct PoolCtx {
    pub pool: AgentPoolStore,
    pub conv_id: Arc<RwLock<Option<String>>>,
    /// 子 agent 时 Some(session_id)；主 agent 为 None（按会话 id 推导）
    pub sub_agent_id: Option<String>,
    /// 子 agent 显示名；主 agent 为 None（按会话标题加载）
    pub sub_agent_name: Option<String>,
    pub store: Option<Arc<ConversationStore>>,
    pub event_bus: Option<Arc<EventBus>>,
}

impl PoolCtx {
    /// 当前 agent 的唯一 id（主 / 子）
    pub async fn self_agent_id(&self) -> Option<String> {
        if let Some(sid) = &self.sub_agent_id {
            return Some(PoolEntry::sub_agent_id(sid));
        }
        self.conv_id
            .read()
            .await
            .clone()
            .map(|id| PoolEntry::main_agent_id(&id))
    }

    /// 当前 agent 的条目类型
    fn kind(&self) -> PoolKind {
        if self.sub_agent_id.is_some() {
            PoolKind::SubAgent
        } else {
            PoolKind::Main
        }
    }

    /// 当前 agent 的显示名（主：会话标题；子：传入名；均缺失回退到 agent_id 后缀）
    pub async fn self_name(&self, agent_id: &str) -> String {
        if let Some(n) = &self.sub_agent_name {
            return n.clone();
        }
        if let Some(store) = &self.store {
            if let Some(conv_id) = self.conv_id.read().await.as_deref() {
                if let Ok(Some(conv)) = store.load(conv_id).await {
                    if let Some(title) = conv.title {
                        return title;
                    }
                }
            }
        }
        agent_id.replacen("conv:", "", 1)
    }

    /// 读取自身条目（未登记返回 None）
    pub async fn own_entry(&self) -> Option<PoolEntry> {
        let id = self.self_agent_id().await?;
        self.pool.find_by_agent_id(&id).await
    }

    /// 变更后通知前端刷新（best-effort）
    async fn notify_changed(&self) {
        if let Some(bus) = &self.event_bus {
            let conv_id = self.conv_id.read().await.clone().unwrap_or_default();
            bus.publish(BusEvent::AgentPoolUpdated { conversation_id: conv_id });
        }
    }
}

// =========================================================
// pool_report：长任务登记 / 上报
// =========================================================

/// 工具参数
#[derive(Deserialize)]
pub struct PoolReportArgs {
    /// 当前长任务描述（如「改造登录接口 v2」）
    pub task: String,
    /// 工作状态：in_progress（正在干）/ waiting（等待中）/ completed（干完了）
    pub status: String,
    /// 研究报告（可选，长任务开始时撰写，供其他 agent 了解进展）
    #[serde(default)]
    pub research_report: Option<String>,
    /// todoTree 摘要（可选，与 todo_write 维护的任务树对齐）
    #[serde(default)]
    pub todo_summary: Option<String>,
    /// 最近一次状态上报文本（可选）
    #[serde(default)]
    pub last_report: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("pool_report error: {0}")]
pub struct PoolReportError(String);

/// 工具
pub struct PoolReportTool {
    ctx: PoolCtx,
}

impl PoolReportTool {
    pub fn new(ctx: PoolCtx) -> Self {
        Self { ctx }
    }
}

impl Tool for PoolReportTool {
    const NAME: &'static str = "pool_report";

    type Error = PoolReportError;
    type Args = PoolReportArgs;
    type Output = String;

    fn description(&self) -> String {
        "在运行时 agent 公共会话交流池登记 / 上报当前长任务状态（进行中/等待中/已完成），\
         附研究报告与 todoTree 摘要。多会话并行时，其他 agent 通过 pool_lookup 看到你的任务、\
         通过 pool_at @ 你询问状态，避免重复劳动与文件操作冲突。\
         调用时机：开始长任务时登记（status=in_progress）；中途等待依赖时上报（status=waiting）；\
         干完后上报（status=completed）。".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "description": "当前长任务描述（必填）" },
                "status": {
                    "type": "string",
                    "enum": ["in_progress", "waiting", "completed"],
                    "description": "工作状态：in_progress（正在干）/ waiting（等待中）/ completed（干完了）"
                },
                "research_report": { "type": "string", "description": "研究报告（可选）：任务背景、调研结论、实施计划" },
                "todo_summary": { "type": "string", "description": "todoTree 摘要（可选）：当前任务树的关键进度" },
                "last_report": { "type": "string", "description": "最近一次状态上报文本（可选）：本次干了什么 / 卡在哪" }
            },
            "required": ["task", "status"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let agent_id = self
            .ctx
            .self_agent_id()
            .await
            .ok_or_else(|| PoolReportError("无法确定当前会话 id，pool_report 不可用".into()))?;
        let status = parse_status(&args.status)?;
        if args.task.trim().is_empty() {
            return Err(PoolReportError("task 不能为空".into()));
        }

        let name = self.ctx.self_name(&agent_id).await;
        let conversation_id = self
            .ctx
            .conv_id
            .read()
            .await
            .clone()
            .unwrap_or_else(|| agent_id.clone());
        let last_report = args
            .last_report
            .unwrap_or_default();
        let entry = PoolEntry {
            agent_id: agent_id.clone(),
            conversation_id,
            name,
            kind: self.ctx.kind(),
            task: args.task.clone(),
            research_report: args.research_report.unwrap_or_default(),
            todo_summary: args.todo_summary.unwrap_or_default(),
            status,
            last_report,
            created_at: 0,
            updated_at: 0,
            inbox: Vec::new(),
        };
        self.ctx
            .pool
            .register(entry)
            .await
            .map_err(|e| PoolReportError(e.to_string()))?;
        self.ctx.notify_changed().await;

        // 返回给 agent 的确认（含自身收件箱，提醒回复 @ 消息）
        let mut out = format!(
            "已登记到交流池（agent_id={agent_id}）：任务「{}」，状态 {}\n",
            args.task,
            status.label()
        );
        let own = self.ctx.own_entry().await;
        let inbox: Vec<_> = own
            .as_ref()
            .map(|e| e.inbox.clone())
            .unwrap_or_default();
        out.push_str(&format_pool_section(own.as_ref(), &inbox));
        Ok(out)
    }
}

// =========================================================
// pool_lookup：查询活跃长任务
// =========================================================

/// 工具参数
#[derive(Deserialize)]
pub struct PoolLookupArgs {
    /// 查询关键词（可选）：文件名 / 接口名 / 任务主题等；空 = 列出全部活跃条目
    #[serde(default)]
    pub query: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("pool_lookup error: {0}")]
pub struct PoolLookupError(String);

/// 工具
pub struct PoolLookupTool {
    ctx: PoolCtx,
}

impl PoolLookupTool {
    pub fn new(ctx: PoolCtx) -> Self {
        Self { ctx }
    }
}

impl Tool for PoolLookupTool {
    const NAME: &'static str = "pool_lookup";

    type Error = PoolLookupError;
    type Args = PoolLookupArgs;
    type Output = String;

    fn description(&self) -> String {
        "查询运行时 agent 公共会话交流池：是否有其他 agent 正在处理同一事项\
         （同文件 / 同接口 / 同任务）。仅返回进行中 / 等待中的活跃长任务；\
         已完成的任务对新查询不可见（不会与已完成的工作冲突）。\
         若发现相关任务，用 pool_at 指定其 agent_id 或 conversation_id 联系询问。\
         若你的工作与其他 agent 无关，则无需协助。".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "查询关键词（可选）：文件名/接口名/任务主题；空 = 列出全部活跃条目"
                }
            }
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let exclude = self.ctx.self_agent_id().await;
        let entries = self
            .ctx
            .pool
            .lookup(args.query.as_deref().unwrap_or(""), exclude.as_deref())
            .await;
        Ok(format_lookup_result(&entries))
    }
}

// =========================================================
// pool_at：@ 某个长任务 agent 询问状态 / 冲突
// =========================================================

/// 工具参数
#[derive(Deserialize)]
pub struct PoolAtArgs {
    /// 目标 agent_id 或 conversation_id（来自 pool_lookup 结果）
    pub target: String,
    /// 提问内容（如「接口状态如何？干完没有？我操作文件是否会冲突？」）
    pub question: String,
    /// 调用模式：async（投递后继续干自己的）/ await（等待回复后再决策）
    #[serde(default)]
    pub mode: Option<String>,
    /// await 模式的等待上限（毫秒，默认 60000）
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
#[error("pool_at error: {0}")]
pub struct PoolAtError(String);

/// 工具
pub struct PoolAtTool {
    ctx: PoolCtx,
}

impl PoolAtTool {
    pub fn new(ctx: PoolCtx) -> Self {
        Self { ctx }
    }
}

impl Tool for PoolAtTool {
    const NAME: &'static str = "pool_at";

    type Error = PoolAtError;
    type Args = PoolAtArgs;
    type Output = String;

    fn description(&self) -> String {
        "在运行时 agent 公共会话交流池中 @ 一个长任务 agent，定向询问其状态（\
         接口状态如何？干完没有？我操作文件是否会冲突？）。提问会插入到目标 agent 的\
         下一次 completion 中，由它调用 pool_reply 回复。\n\
         mode=async：投递后不等待回复，先干自己的事（默认）；\n\
         mode=await：投递后轮询等待对方回复（默认超时 60 秒），拿到回复后再决策。\n\
         若目标已完成，会立即返回其完成结论，无需等待。".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "目标 agent_id 或 conversation_id（来自 pool_lookup 结果）"
                },
                "question": {
                    "type": "string",
                    "description": "提问内容，如「接口状态如何？干完没有？我操作文件是否会冲突？」"
                },
                "mode": {
                    "type": "string",
                    "enum": ["async", "await"],
                    "description": "async=投递后继续干自己的；await=等待回复后再决策（默认 async）"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "await 模式等待上限（毫秒，默认 60000）"
                }
            },
            "required": ["target", "question"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let mode = args.mode.as_deref().unwrap_or("async");
        let from = self
            .ctx
            .self_agent_id()
            .await
            .ok_or_else(|| PoolAtError("无法确定当前会话 id，pool_at 不可用".into()))?;
        let from_name = self.ctx.self_name(&from).await;

        // 目标存在性 + 完成态短路
        let target_entry = self
            .ctx
            .pool
            .find(&args.target)
            .await
            .ok_or_else(|| {
                PoolAtError(format!(
                    "交流池中没有匹配 {target} 的条目，请先用 pool_lookup 查询确认 agent_id 或 conversation_id",
                    target = args.target
                ))
            })?;
        if target_entry.status == PoolStatus::Completed {
            let mut out = format!(
                "目标「{}」的长任务【已完成】：{}\n",
                target_entry.name,
                if target_entry.last_report.is_empty() {
                    target_entry.task
                } else {
                    target_entry.last_report.clone()
                }
            );
            if !target_entry.todo_summary.trim().is_empty() {
                out.push_str(&format!("其 todoTree 摘要：{}\n", target_entry.todo_summary));
            }
            out.push_str("你无需等待，也不会与该任务冲突。");
            return Ok(out);
        }

        let at_id = self
            .ctx
            .pool
            .at(&args.target, &from, &from_name, &args.question)
            .await
            .map_err(|e| PoolAtError(e.to_string()))?;
        self.ctx.notify_changed().await;

        if mode == "await" {
            // await：轮询等待对方回复
            let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(60_000).min(600_000));
            let poll = Duration::from_millis(1500);
            let deadline = tokio::time::Instant::now() + timeout;
            loop {
                if tokio::time::Instant::now() >= deadline {
                    return Err(PoolAtError(format!(
                        "已等待 {} 秒未收到「{}」的回复（at_id={at_id}）。\
                         对方可能在执行其他长任务，可稍后再次 pool_at，或用 async 模式继续自己的工作。",
                        timeout.as_secs(),
                        target_entry.name
                    )));
                }
                tokio::time::sleep(poll).await;
                if let Some(msg) = self.ctx.pool.at_status(&at_id).await {
                    if msg.status == AtStatus::Answered {
                        return Ok(format!(
                            "「{}」回复（at_id={at_id}）：{}\n你可以据此决策（继续 / 等待 / 调整操作路径）。",
                            target_entry.name,
                            msg.reply.unwrap_or_default()
                        ));
                    }
                }
            }
        }

        // async：投递即返回
        Ok(format!(
            "已 @ 「{}」（at_id={at_id}）：{question}\n\
             提问已插入对方收件箱，将在其下一次 completion 送达并由其回复。\
             你继续干自己的事，无需等待。",
            target_entry.name,
            question = args.question
        ))
    }
}

// =========================================================
// pool_reply：回复收件箱 @ 消息
// =========================================================

/// 工具参数
#[derive(Deserialize)]
pub struct PoolReplyArgs {
    /// 目标 @ 消息 id（来自 `[Agent 交流池]` 上下文段的收件箱）
    pub at_id: String,
    /// 你的答复
    pub reply: String,
}

#[derive(Debug, thiserror::Error)]
#[error("pool_reply error: {0}")]
pub struct PoolReplyError(String);

/// 工具
pub struct PoolReplyTool {
    ctx: PoolCtx,
}

impl PoolReplyTool {
    pub fn new(ctx: PoolCtx) -> Self {
        Self { ctx }
    }
}

impl Tool for PoolReplyTool {
    const NAME: &'static str = "pool_reply";

    type Error = PoolReplyError;
    type Args = PoolReplyArgs;
    type Output = String;

    fn description(&self) -> String {
        "回复交流池收件箱中的 @ 消息。你的收件箱消息会出现在上下文段的\
         [Agent 交流池] 里（形如 [at_id=xxx] 来自「xx」）。调用本工具把答复回传给\
         对方（对方若用 pool_at mode=await 会立刻收到）。".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "at_id": { "type": "string", "description": "@ 消息 id（来自上下文收件箱的 at_id=xxx）" },
                "reply": { "type": "string", "description": "你的答复，如实说明状态/是否干完/是否会冲突" }
            },
            "required": ["at_id", "reply"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if args.reply.trim().is_empty() {
            return Err(PoolReplyError("reply 不能为空".into()));
        }
        let found = self
            .ctx
            .pool
            .reply(&args.at_id, &args.reply)
            .await
            .map_err(|e| PoolReplyError(e.to_string()))?;
        if !found {
            return Err(PoolReplyError(format!(
                "未找到 at_id={} 的 @ 消息，可能已被删除",
                args.at_id
            )));
        }
        self.ctx.notify_changed().await;
        Ok("已回复对方。".to_string())
    }
}

/// 解析状态字符串为 PoolStatus（兼容 snake_case 与中文）
fn parse_status(s: &str) -> Result<PoolStatus, PoolReportError> {
    match s.trim().to_lowercase().as_str() {
        "in_progress" | "inprogress" | "进行中" | "干" => Ok(PoolStatus::InProgress),
        "waiting" | "等待中" => Ok(PoolStatus::Waiting),
        "completed" | "done" | "已完成" | "干完了" => Ok(PoolStatus::Completed),
        other => Err(PoolReportError(format!(
            "无效状态 {other}，应为 in_progress / waiting / completed"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_accepts_variants() {
        assert_eq!(parse_status("in_progress").unwrap(), PoolStatus::InProgress);
        assert_eq!(parse_status("进行中").unwrap(), PoolStatus::InProgress);
        assert_eq!(parse_status("waiting").unwrap(), PoolStatus::Waiting);
        assert_eq!(parse_status("completed").unwrap(), PoolStatus::Completed);
        assert_eq!(parse_status("干完了").unwrap(), PoolStatus::Completed);
        assert!(parse_status("bogus").is_err());
    }

    #[tokio::test]
    async fn report_lookup_at_reply_end_to_end() {
        let dir = std::env::temp_dir().join(format!("effisuite-pooltool-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = AgentPoolStore::new(&dir).unwrap();
        let conv_id: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(Some("1".to_string())));
        let ctx = PoolCtx {
            pool: pool.clone(),
            conv_id: Arc::clone(&conv_id),
            sub_agent_id: None,
            sub_agent_name: None,
            store: None,
            event_bus: None,
        };

        // A 登记
        let report = PoolReportTool::new(ctx.clone());
        let out = report
            .call(PoolReportArgs {
                task: "改造登录接口 v2".to_string(),
                status: "in_progress".to_string(),
                research_report: Some("调研完成".to_string()),
                todo_summary: Some("1/3".to_string()),
                last_report: Some("正在改 auth.rs".to_string()),
            })
            .await
            .unwrap();
        assert!(out.contains("agent_id=conv:1"));

        // B（会话 2）查询
        let conv2: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(Some("2".to_string())));
        let ctx2 = PoolCtx {
            pool: pool.clone(),
            conv_id: Arc::clone(&conv2),
            sub_agent_id: None,
            sub_agent_name: None,
            store: None,
            event_bus: None,
        };
        let lookup = PoolLookupTool::new(ctx2.clone());
        let out = lookup
            .call(PoolLookupArgs { query: Some("登录".to_string()) })
            .await
            .unwrap();
        assert!(out.contains("conv:1"));
        assert!(out.contains("改造登录接口 v2"));

        // B @ A（async）
        let at = PoolAtTool::new(ctx2.clone());
        let out = at
            .call(PoolAtArgs {
                target: "1".to_string(),
                question: "接口状态如何？我操作 auth.rs 会冲突吗？".to_string(),
                mode: Some("async".to_string()),
                timeout_ms: None,
            })
            .await
            .unwrap();
        assert!(out.contains("at_id="));

        // A 的收件箱有消息，回复
        let reply = PoolReplyTool::new(ctx.clone());
        // 找到 at_id
        let entry = pool.find_by_agent_id("conv:1").await.unwrap();
        let at_id = entry.inbox[0].at_id.clone();
        let out = reply
            .call(PoolReplyArgs { at_id: at_id.clone(), reply: "已干完，auth.rs 我改完了，你可以直接操作".to_string() })
            .await
            .unwrap();
        assert!(out.contains("已回复"));

        // B await 轮询应立刻拿到回复
        let at2 = PoolAtTool::new(ctx2.clone());
        // 重新 @ 并 await（目标已完成登记但仍投递）
        let _ = pool.set_status("conv:2", PoolStatus::InProgress, String::new()).await.unwrap();
        let _ = pool.at("1", "conv:2", "会话B", "再确认下").await.unwrap();
        let e = pool.find_by_agent_id("conv:1").await.unwrap();
        let at_id2 = e.inbox.last().unwrap().at_id.clone();
        let _ = pool.reply(&at_id2, "可以").await.unwrap();
        // 直接构造 await 调用路径（目标已完成 @ 会短路返回完成态，这里用进行中目标验证轮询）
        // 简化：验证 completed 短路
        let _ = pool.set_status("conv:1", PoolStatus::Completed, "登录接口已上线".to_string()).await.unwrap();
        let out = at2
            .call(PoolAtArgs {
                target: "1".to_string(),
                question: "现在可以操作了吗".to_string(),
                mode: Some("async".to_string()),
                timeout_ms: None,
            })
            .await
            .unwrap();
        assert!(out.contains("已完成"));
        assert!(out.contains("登录接口已上线"));

        std::fs::remove_dir_all(&dir).ok();
    }
}

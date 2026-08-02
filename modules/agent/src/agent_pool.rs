//! 运行时 agent 公共会话交流池（跨会话协作基础设施）
//!
//! # 背景
//!
//! 程序可能同时运行多个会话（每个会话一个 agent），子 agent 也会被并发召唤。
//! 它们可能操作同一个工作区 / 同一批文件、执行重叠的任务。为避免互相踩踏并
//! 提供跨会话协作，引入一个**特殊的会话池**：每个长任务在池中登记自己的
//! 工作状态（进行中 / 等待中 / 已完成）、研究报告与 todoTree 摘要；其他 agent
//! 遇到问题时先在池中查询是否已有 agent 在处理，再用 `@` 机制定向询问。
//!
//! # 核心能力
//!
//! - **登记 / 上报**：`pool_report` 工具把长任务（研究报告 + todoTree 摘要 + 状态）
//!   写入池。状态机：`in_progress`（正在干）→ `waiting`（等待中）→ `completed`（干完了）。
//! - **查询边界**：`pool_lookup` 只返回**进行中 / 等待中**的条目——已经干完的长任务
//!   对"之后才来查的 agent"不可见（避免 @ 一个已经结束的任务）；而**并行运行**的
//!   agent 可通过 `@` 一个已完成目标立刻拿到"干完了"的答复。
//! - **@ 机制**：`pool_at` 把一条提问插入目标 agent 的**收件箱**（持久化），
//!   目标在**下一次 completion**（`[Agent 交流池]` 段注入）中看到并调用
//!   `pool_reply` 回复。调用方可选 `async`（投递后继续干自己的）或
//!   `await`（轮询等待回复后再决策）。
//! - **子 agent**：由会话 agent 创建的子 agent 同样加入池（`kind=sub_agent`），
//!   按长任务周期自动上报（开始→in_progress，完成→completed）。
//! - **持久化**：池整体写入单个 JSON 文件（`<root>/pool.json`），每次变更落盘，
//!   程序崩溃 / 异常退出后重启可恢复全部状态与收件箱。
//!
//! # 数据归属
//!
//! - 主 agent 的 `agent_id = "conv:<conversation_id>"`
//! - 子 agent 的 `agent_id = "sa:<session_id>"`
//! - `conversation_id` 统一指向主会话，方便会话列表按会话聚合展示运行状态。

use std::path::PathBuf;
use std::sync::Arc;

use effisuite_core::{CoreError, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// 交流池条目状态（会话列表据此展示「进行中 / 等待中 / 已完成」）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolStatus {
    /// 进行中：正在执行长任务
    InProgress,
    /// 等待中：任务挂起（等待外部依赖 / 等待 @ 回复 / 等待用户输入）
    Waiting,
    /// 已完成：长任务干完了
    Completed,
}

impl PoolStatus {
    /// 中文标签（前端 / 提示词展示用）
    pub fn label(&self) -> &'static str {
        match self {
            PoolStatus::InProgress => "进行中",
            PoolStatus::Waiting => "等待中",
            PoolStatus::Completed => "已完成",
        }
    }

    /// 是否为"活跃"状态（可被其他 agent 查询到、可被 @ 的目标）
    pub fn is_active(&self) -> bool {
        !matches!(self, PoolStatus::Completed)
    }
}

/// @ 消息回复状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtStatus {
    /// 已投递，等待目标回复
    Pending,
    /// 目标已回复
    Answered,
}

/// 交流池条目类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolKind {
    /// 会话主 agent
    Main,
    /// 由会话 agent 创建的子 agent
    SubAgent,
}

/// 一条 @ 消息（目标 agent 的收件箱条目）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtMessage {
    /// @ 消息唯一 id（pool_reply 的定位键）
    pub at_id: String,
    /// 发送方 agent_id
    pub from: String,
    /// 发送方显示名
    pub from_name: String,
    /// 提问内容（如「接口状态如何？干完没有？我操作文件是否会冲突？」）
    pub question: String,
    /// 回复状态
    pub status: AtStatus,
    /// 目标回复内容（answered 时有值）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<u64>,
}

/// 交流池条目：一个长任务的完整登记信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolEntry {
    /// agent 唯一 id：主 agent = `conv:<conversation_id>`；子 agent = `sa:<session_id>`
    pub agent_id: String,
    /// 所属会话 conversation_id（主 agent 即自身；子 agent 为主会话 id）
    pub conversation_id: String,
    /// 显示名（会话标题 / 子 agent 名）
    pub name: String,
    /// 条目类型
    pub kind: PoolKind,
    /// 当前长任务描述
    pub task: String,
    /// 研究报告（长任务开始时撰写，供其他 agent 了解进展）
    pub research_report: String,
    /// todoTree 摘要文本（与 todo_write 工具维护的任务树对齐）
    pub todo_summary: String,
    /// 工作状态：进行中 / 等待中 / 已完成
    pub status: PoolStatus,
    /// 最近一次状态上报文本
    pub last_report: String,
    pub created_at: u64,
    pub updated_at: u64,
    /// 收件箱 @ 消息（目标下一次 completion 注入）
    #[serde(default)]
    pub inbox: Vec<AtMessage>,
}

impl PoolEntry {
    /// 主 agent 的默认 agent_id
    pub fn main_agent_id(conversation_id: &str) -> String {
        format!("conv:{conversation_id}")
    }

    /// 子 agent 的默认 agent_id
    pub fn sub_agent_id(session_id: &str) -> String {
        format!("sa:{session_id}")
    }
}

/// 池文件的磁盘表示
#[derive(Debug, Default, Serialize, Deserialize)]
struct PoolFile {
    entries: Vec<PoolEntry>,
}

/// 运行时 agent 公共会话交流池存储，线程安全可廉价 clone（内部 Arc 共享）。
///
/// - 全局单一 JSON 文件，内存缓存 + 每次变更异步落盘
/// - 所有读/写经 `RwLock` 串行化（池规模很小，微秒级，无性能压力）
/// - 崩溃 / 异常退出后重启，`new` 会从磁盘恢复全部条目与收件箱
#[derive(Clone)]
pub struct AgentPoolStore {
    path: PathBuf,
    state: Arc<RwLock<PoolFile>>,
}

impl AgentPoolStore {
    /// 创建交流池存储，root 不存在时自动创建；存在 pool.json 则恢复历史状态
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        let path = root.join("pool.json");
        let state = if path.exists() {
            match std::fs::read(&path) {
                Ok(bytes) => serde_json::from_slice::<PoolFile>(&bytes)
                    .unwrap_or_else(|e| {
                        tracing::warn!(error = %e, path = %path.display(), "agent pool 反序列化失败，使用空池");
                        PoolFile::default()
                    }),
                Err(e) => {
                    tracing::warn!(error = %e, path = %path.display(), "agent pool 读取失败，使用空池");
                    PoolFile::default()
                }
            }
        } else {
            PoolFile::default()
        };
        Ok(Self {
            path,
            state: Arc::new(RwLock::new(state)),
        })
    }

    /// 落盘（每次变更后调用）。失败仅记录日志，不影响内存态（best-effort）。
    async fn persist(&self, file: &PoolFile) {
        let bytes = match serde_json::to_vec(file) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "agent pool 序列化失败");
                return;
            }
        };
        if let Err(e) = tokio::fs::write(&self.path, bytes).await {
            tracing::warn!(error = %e, path = %self.path.display(), "agent pool 持久化失败");
        }
    }

    // =========================================================
    // 登记 / 上报
    // =========================================================

    /// 登记或更新一个交流池条目（按 agent_id 覆盖）。不存在则新建。
    pub async fn register(&self, entry: PoolEntry) -> Result<()> {
        let mut file = self.state.write().await;
        let now = now_ms();
        if let Some(ex) = file.entries.iter_mut().find(|e| e.agent_id == entry.agent_id) {
            // 合并：保留现有收件箱，其余字段以新值覆盖
            ex.conversation_id = entry.conversation_id;
            ex.name = entry.name;
            ex.kind = entry.kind;
            ex.task = entry.task;
            ex.research_report = entry.research_report;
            ex.todo_summary = entry.todo_summary;
            ex.status = entry.status;
            ex.last_report = entry.last_report;
            ex.updated_at = now;
        } else {
            let mut e = entry;
            e.created_at = now;
            e.updated_at = now;
            file.entries.push(e);
        }
        self.persist(&file).await;
        Ok(())
    }

    /// 更新条目的任务/报告/摘要/状态/上报文本；agent_id 不存在时返回 false（不新建）
    pub async fn update(
        &self,
        agent_id: &str,
        task: String,
        research_report: String,
        todo_summary: String,
        status: PoolStatus,
        last_report: String,
    ) -> Result<bool> {
        let mut file = self.state.write().await;
        let now = now_ms();
        let Some(ex) = file.entries.iter_mut().find(|e| e.agent_id == agent_id) else {
            return Ok(false);
        };
        ex.task = task;
        ex.research_report = research_report;
        ex.todo_summary = todo_summary;
        ex.status = status;
        ex.last_report = last_report;
        ex.updated_at = now;
        self.persist(&file).await;
        Ok(true)
    }

    /// 仅更新状态（用于子 agent 自动上报 / 会话删除时清理等），不存在返回 false
    pub async fn set_status(&self, agent_id: &str, status: PoolStatus, last_report: String) -> Result<bool> {
        let mut file = self.state.write().await;
        let now = now_ms();
        let Some(ex) = file.entries.iter_mut().find(|e| e.agent_id == agent_id) else {
            return Ok(false);
        };
        ex.status = status;
        ex.last_report = last_report;
        ex.updated_at = now;
        self.persist(&file).await;
        Ok(true)
    }

    // =========================================================
    // 查询
    // =========================================================

    /// 按 agent_id 或 conversation_id 精确查找条目
    pub async fn find(&self, agent_id_or_conv: &str) -> Option<PoolEntry> {
        let file = self.state.read().await;
        file.entries
            .iter()
            .find(|e| e.agent_id == agent_id_or_conv || e.conversation_id == agent_id_or_conv)
            .cloned()
    }

    /// 按 agent_id 精确查找
    pub async fn find_by_agent_id(&self, agent_id: &str) -> Option<PoolEntry> {
        let file = self.state.read().await;
        file.entries.iter().find(|e| e.agent_id == agent_id).cloned()
    }

    /// 查询交流池：仅返回**进行中 / 等待中**的活跃条目（已完成对后续新查询不可见）。
    ///
    /// `query` 为空返回全部活跃条目；否则对 名称 / 任务 / 研究报告 / todoTree 摘要 /
    /// 状态上报 做不区分大小写的子串匹配。`exclude_agent_id` 用于排除自身。
    pub async fn lookup(&self, query: &str, exclude_agent_id: Option<&str>) -> Vec<PoolEntry> {
        let file = self.state.read().await;
        let q = query.trim().to_lowercase();
        let mut out: Vec<PoolEntry> = file
            .entries
            .iter()
            .filter(|e| {
                e.status.is_active()
                    && exclude_agent_id.map(|id| e.agent_id != id).unwrap_or(true)
            })
            .filter(|e| {
                if q.is_empty() {
                    return true;
                }
                [&e.name, &e.task, &e.research_report, &e.todo_summary, &e.last_report]
                    .iter()
                    .any(|s| s.to_lowercase().contains(&q))
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        out
    }

    /// 列出全部条目（含已完成；供会话列表 / 管理面板展示）
    pub async fn list(&self) -> Vec<PoolEntry> {
        let file = self.state.read().await;
        let mut out = file.entries.clone();
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        out
    }

    // =========================================================
    // @ 机制
    // =========================================================

    /// 给目标 agent 投递一条 @ 消息到其收件箱，返回 at_id。
    ///
    /// 目标不存在时返回 `Err`。目标已处于 `completed` 时仍投递（并行 agent
    /// 应看到"干完了"的答复；若目标不会再运行，`pool_at` 工具层会给出提示）。
    pub async fn at(
        &self,
        target: &str,
        from: &str,
        from_name: &str,
        question: &str,
    ) -> Result<String> {
        let mut file = self.state.write().await;
        let now = now_ms();
        let Some(ex) = file
            .entries
            .iter_mut()
            .find(|e| e.agent_id == target || e.conversation_id == target)
        else {
            return Err(CoreError::Agent(format!(
                "交流池中没有匹配 agent_id/conversation_id = {target} 的活跃条目，请先用 pool_lookup 查询"
            )));
        };
        let at_id = uuid::Uuid::new_v4().to_string();
        ex.inbox.push(AtMessage {
            at_id: at_id.clone(),
            from: from.to_string(),
            from_name: from_name.to_string(),
            question: question.to_string(),
            status: AtStatus::Pending,
            reply: None,
            created_at: now,
            answered_at: None,
        });
        ex.updated_at = now;
        self.persist(&file).await;
        Ok(at_id)
    }

    /// 目标 agent 回复某条 @ 消息（按 at_id 全局定位）。返回是否找到。
    pub async fn reply(&self, at_id: &str, reply: &str) -> Result<bool> {
        let mut file = self.state.write().await;
        let now = now_ms();
        let mut found = false;
        for ex in file.entries.iter_mut() {
            if let Some(msg) = ex.inbox.iter_mut().find(|m| m.at_id == at_id) {
                msg.status = AtStatus::Answered;
                msg.reply = Some(reply.to_string());
                msg.answered_at = Some(now);
                found = true;
                ex.updated_at = now;
                break;
            }
        }
        if found {
            self.persist(&file).await;
        }
        Ok(found)
    }

    /// 读取某条 @ 消息当前状态（await 轮询用）
    pub async fn at_status(&self, at_id: &str) -> Option<AtMessage> {
        let file = self.state.read().await;
        file.entries.iter().find_map(|e| {
            e.inbox.iter().find(|m| m.at_id == at_id).cloned()
        })
    }

    // =========================================================
    // 清理
    // =========================================================

    /// 删除某会话的全部交流池条目（含其子 agent）；会话被删除时调用
    pub async fn remove_by_conversation(&self, conversation_id: &str) -> Result<()> {
        let mut file = self.state.write().await;
        let before = file.entries.len();
        file.entries
            .retain(|e| e.conversation_id != conversation_id);
        if file.entries.len() != before {
            self.persist(&file).await;
        }
        Ok(())
    }

    /// 批量删除多个会话的全部交流池条目（含其子 agent）。
    ///
    /// 相比循环调用 `remove_by_conversation`，此方法仅获取一次写锁、
    /// 最多持久化一次，显著降低批量删除时的锁竞争与 IO 开销。
    ///
    /// 使用 HashSet 做 O(1) 查找，避免 `contains` 的 O(N*M) 扫描。
    pub async fn remove_by_conversations(&self, conversation_ids: &[String]) -> Result<()> {
        if conversation_ids.is_empty() {
            return Ok(());
        }
        let id_set: std::collections::HashSet<&str> = conversation_ids
            .iter()
            .map(|s| s.as_str())
            .collect();
        let mut file = self.state.write().await;
        let before = file.entries.len();
        file.entries
            .retain(|e| !id_set.contains(e.conversation_id.as_str()));
        if file.entries.len() != before {
            self.persist(&file).await;
        }
        Ok(())
    }

    /// 删除指定 agent 条目
    pub async fn remove(&self, agent_id: &str) -> Result<()> {
        let mut file = self.state.write().await;
        let before = file.entries.len();
        file.entries.retain(|e| e.agent_id != agent_id);
        if file.entries.len() != before {
            self.persist(&file).await;
        }
        Ok(())
    }
}

// =========================================================
// 格式化辅助（上下文注入 / 工具输出共用）
// =========================================================

/// 构建 `[Agent 交流池]` 上下文段：注入给当前 agent 的协作信息。
///
/// 包含三块：
/// 1. 自身交流池状态（任务 / 状态），提醒 agent 记得上报
/// 2. 收件箱 @ 消息（"访问信息插入下一次 completion"），要求用 pool_reply 回复
/// 3. 协作协议说明（何时 pool_report / pool_lookup / pool_at）
///
/// `own` 为当前 agent 的条目（None 表示尚未登记）；`inbox` 为收件箱 @ 消息
/// （取自 own 或单独传入，保证即使未登记也能看到 @ 消息）。
pub fn format_pool_section(own: Option<&PoolEntry>, inbox: &[AtMessage]) -> String {
    let mut s = String::with_capacity(256);
    s.push_str("[Agent 交流池]（多会话协作：长任务在此登记状态，其他 agent 可 @ 你）\n");

    // 1. 自身状态
    match own {
        Some(e) => {
            s.push_str("你的交流池状态：");
            s.push_str(e.status.label());
            s.push_str(" | 任务：");
            s.push_str(&e.task);
            if !e.todo_summary.trim().is_empty() {
                s.push_str(" | todoTree：");
                s.push_str(&e.todo_summary);
            }
            s.push('\n');
        }
        None => {
            s.push_str("你尚未在交流池登记。若本次是长任务，请先调用 pool_report 登记状态。\n");
        }
    }

    // 2. 收件箱 @ 消息
    if !inbox.is_empty() {
        s.push_str("收件箱 @ 消息（请在下一次执行中回复）：\n");
        for (i, m) in inbox.iter().enumerate() {
            if m.status == AtStatus::Answered {
                continue;
            }
            s.push_str(&format!(
                "{}. [at_id={}] 来自「{}」：{}\n",
                i + 1,
                m.at_id,
                m.from_name,
                m.question
            ));
            s.push_str(&format!(
                "   → 请调用 pool_reply 工具回复（参数 at_id={}，reply=你的答复）\n",
                m.at_id
            ));
        }
    }

    // 3. 协作协议
    s.push_str("协作协议：\n");
    s.push_str("- 长任务开始 / 推进 / 完成时调用 pool_report 登记状态（含研究报告与 todoTree 摘要）；\n");
    s.push_str("- 遇到问题（如操作文件、接口）先调用 pool_lookup 查看是否有其他 agent 正在处理同一事项，避免冲突；\n");
    s.push_str("- 需要协作时用 pool_at 联系对方：mode=async 不等待回复先干自己的；mode=await 等待回复后再决策。\n");
    s
}

/// 把查询到的活跃条目格式化为 `pool_lookup` 的输出文本
pub fn format_lookup_result(entries: &[PoolEntry]) -> String {
    if entries.is_empty() {
        return "交流池中暂无可协作的活跃长任务（进行中/等待中）。\n\
                 已完成的任务不在此列出（不会与新的工作冲突）。".to_string();
    }
    let mut s = String::with_capacity(entries.len() * 160);
    s.push_str("交流池活跃长任务（进行中/等待中，共 ");
    s.push_str(&entries.len().to_string());
    s.push_str(" 个）：\n");
    for (i, e) in entries.iter().enumerate() {
        let kind = match e.kind {
            PoolKind::Main => "会话",
            PoolKind::SubAgent => "子agent",
        };
        s.push_str(&format!(
            "{}. [{}] 「{}」（{}，agent_id={}，conversation_id={}）\n",
            i + 1,
            kind,
            e.name,
            e.status.label(),
            e.agent_id,
            e.conversation_id
        ));
        s.push_str(&format!("   任务：{}\n", e.task));
        if !e.todo_summary.trim().is_empty() {
            s.push_str(&format!("   todoTree：{}\n", e.todo_summary));
        }
        if !e.last_report.trim().is_empty() && e.last_report != e.task {
            s.push_str(&format!("   最近上报：{}\n", e.last_report));
        }
        if !e.research_report.trim().is_empty() {
            s.push_str(&format!("   研究报告：{}\n", e.research_report));
        }
    }
    s.push_str("\n如需询问某个 agent 的状态/冲突，用 pool_at 指定其 agent_id 或 conversation_id。");
    s
}

/// 当前 Unix 毫秒时间戳
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("effisuite-pool-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(agent_id: &str, conv: &str, name: &str, task: &str, status: PoolStatus) -> PoolEntry {
        PoolEntry {
            agent_id: agent_id.to_string(),
            conversation_id: conv.to_string(),
            name: name.to_string(),
            kind: PoolKind::Main,
            task: task.to_string(),
            research_report: String::new(),
            todo_summary: String::new(),
            status,
            last_report: String::new(),
            created_at: 0,
            updated_at: 0,
            inbox: Vec::new(),
        }
    }

    #[tokio::test]
    async fn register_and_persist_roundtrip() {
        let dir = tmp_dir();
        let store = AgentPoolStore::new(&dir).unwrap();
        store
            .register(entry("conv:1", "1", "会话A", "改造登录接口", PoolStatus::InProgress))
            .await
            .unwrap();
        assert!(store.find("1").await.is_some());

        // 重新加载（模拟崩溃重启）应恢复
        let store2 = AgentPoolStore::new(&dir).unwrap();
        let e = store2.find("1").await.unwrap();
        assert_eq!(e.task, "改造登录接口");
        assert_eq!(e.status, PoolStatus::InProgress);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn lookup_excludes_completed_and_self() {
        let dir = tmp_dir();
        let store = AgentPoolStore::new(&dir).unwrap();
        store
            .register(entry("conv:1", "1", "A", "改造登录接口", PoolStatus::InProgress))
            .await
            .unwrap();
        store
            .register(entry("conv:2", "2", "B", "重构首页", PoolStatus::Waiting))
            .await
            .unwrap();
        store
            .register(entry("conv:3", "3", "C", "写测试文档", PoolStatus::Completed))
            .await
            .unwrap();

        // 全部活跃（不含已完成）
        let all = store.lookup("", None).await;
        assert_eq!(all.len(), 2);

        // 排除自身
        let others = store.lookup("", Some("conv:1")).await;
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].agent_id, "conv:2");

        // 关键词匹配
        let hits = store.lookup("登录", None).await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].agent_id, "conv:1");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn at_and_reply_flow() {
        let dir = tmp_dir();
        let store = AgentPoolStore::new(&dir).unwrap();
        store
            .register(entry("conv:1", "1", "A", "改造登录接口", PoolStatus::InProgress))
            .await
            .unwrap();

        let at_id = store
            .at("1", "conv:2", "会话B", "接口状态如何？干完没有？我操作文件是否会冲突？")
            .await
            .unwrap();
        let pending = store.at_status(&at_id).await.unwrap();
        assert_eq!(pending.status, AtStatus::Pending);

        // 目标回复
        assert!(store.reply(&at_id, "已干完，接口 v2 已上线，文件 src/api.rs 我在改，建议等 5 分钟").await.unwrap());
        let answered = store.at_status(&at_id).await.unwrap();
        assert_eq!(answered.status, AtStatus::Answered);
        assert!(answered.reply.unwrap().contains("已干完"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn at_unknown_target_errors() {
        let dir = tmp_dir();
        let store = AgentPoolStore::new(&dir).unwrap();
        let r = store.at("missing", "conv:1", "A", "hi").await;
        assert!(r.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn remove_by_conversation_cleans_subagents() {
        let dir = tmp_dir();
        let store = AgentPoolStore::new(&dir).unwrap();
        store
            .register(entry("conv:1", "1", "A", "任务", PoolStatus::InProgress))
            .await
            .unwrap();
        store
            .register(entry("sa:abc", "1", "子", "子任务", PoolStatus::InProgress))
            .await
            .unwrap();
        store.remove_by_conversation("1").await.unwrap();
        assert!(store.list().await.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pool_section_contains_protocol_and_inbox() {
        let e = entry("conv:1", "1", "A", "改造登录接口", PoolStatus::InProgress);
        let msg = AtMessage {
            at_id: "at-1".to_string(),
            from: "conv:2".to_string(),
            from_name: "会话B".to_string(),
            question: "接口状态如何？".to_string(),
            status: AtStatus::Pending,
            reply: None,
            created_at: 0,
            answered_at: None,
        };
        let s = format_pool_section(Some(&e), &[msg]);
        assert!(s.contains("进行中"));
        assert!(s.contains("改造登录接口"));
        assert!(s.contains("at-1"));
        assert!(s.contains("pool_reply"));
        assert!(s.contains("pool_lookup"));
        assert!(s.contains("pool_at"));
    }
}

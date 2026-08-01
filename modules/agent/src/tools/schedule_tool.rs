//! 定时任务管理工具：让 LLM 通过 cron 表达式管理定时任务
//!
//! 单工具多 action 设计（rig `Tool` trait），底层使用 `ScheduledTaskStore`：
//! - create：创建定时任务（生成 UUID id，name/cron/skill_id 必填）
//! - update：按 id 更新任务字段（name/cron/skill_id）
//! - pause / resume：切换 enabled 状态
//! - delete：删除任务
//! - list：列出全部任务（表格格式，含启用/暂停计数）
//! - get：查询单个任务详情
//! - trigger：标记 last_run 为当前时间（实际触发由外部调度器执行）
//!
//! # 设计要点（对齐 user_rules）
//!
//! - 工具无状态，数据在共享 `Arc<ScheduledTaskStore>` 中
//! - IO 全异步，工具内不持锁（store 内部 RwLock 临界区极短，IO 在锁外）
//! - `with_capacity` 预分配输出缓冲，避免多次 realloc
//! - cron 验证为简单 5 字段格式校验；next_run 计算失败时优雅降级
//! - 返回纯文本，流式友好；错误以 `ScheduleError` 结构化返回

use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local, TimeZone};
use cron::Schedule;
use effisuite_core::{ScheduledTask, ScheduledTaskStore};
use rig_core::tool::Tool;
use serde::Deserialize;

/// 操作类型
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleAction {
    Create,
    Update,
    Pause,
    Resume,
    Delete,
    List,
    Get,
    Trigger,
}

/// 工具参数
#[derive(Deserialize)]
pub struct ScheduleArgs {
    /// 操作类型
    pub action: ScheduleAction,
    /// 任务 ID（update/pause/resume/delete/get/trigger 需要）
    #[serde(default)]
    pub scheduled_task_id: Option<String>,
    /// 任务名称（create/update 需要）
    #[serde(default)]
    pub name: Option<String>,
    /// 任务内容/消息（create/update 需要，描述任务要做什么）
    #[serde(default)]
    pub message: Option<String>,
    /// cron 表达式（create/update 需要，5字段：分 时 日 月 周）
    #[serde(default)]
    pub cron_expression: Option<String>,
    /// 时区（IANA 标识，如 "Asia/Shanghai"）
    #[serde(default)]
    pub timezone: Option<String>,
    /// 关联的技能 ID（create 需要）
    #[serde(default)]
    pub skill_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("schedule error: {0}")]
pub struct ScheduleError(String);

/// 定时任务管理工具
///
/// 持有共享 `Arc<ScheduledTaskStore>`（与 Tauri 命令层、调度器共享同一份）。
/// 工具本身无状态，所有持久化由 store 完成。
pub struct ScheduleTool {
    store: Arc<ScheduledTaskStore>,
}

impl ScheduleTool {
    pub fn new(store: Arc<ScheduledTaskStore>) -> Self {
        Self { store }
    }
}

// =========================================================
// Tool trait 实现
// =========================================================

impl Tool for ScheduleTool {
    const NAME: &'static str = "schedule";

    type Error = ScheduleError;
    type Args = ScheduleArgs;
    type Output = String;

    fn description(&self) -> String {
        "管理 cron 定时任务。支持创建/更新/暂停/恢复/删除/列表/查询/触发定时任务。\
         定时任务通过 cron 表达式（5字段：分 时 日 月 周）按计划自动执行关联的技能。\
         例如：每天 9 点执行报告技能、每周一 10 点执行周报技能等。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "update", "pause", "resume", "delete", "list", "get", "trigger"],
                    "description": "操作类型"
                },
                "scheduled_task_id": {
                    "type": "string",
                    "description": "任务 ID（update/pause/resume/delete/get/trigger 需要）"
                },
                "name": {
                    "type": "string",
                    "description": "任务名称（create/update 需要）"
                },
                "message": {
                    "type": "string",
                    "description": "任务内容/消息，描述任务要做什么（create/update 需要）"
                },
                "cron_expression": {
                    "type": "string",
                    "description": "cron 表达式，5字段：分 时 日 月 周（如 \"0 9 * * *\" 表示每天 9 点）"
                },
                "timezone": {
                    "type": "string",
                    "description": "时区（IANA 标识，如 \"Asia/Shanghai\"）"
                },
                "skill_id": {
                    "type": "string",
                    "description": "关联的技能 ID（create 需要）"
                }
            },
            "required": ["action"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        match args.action {
            ScheduleAction::Create => self.action_create(args).await,
            ScheduleAction::Update => self.action_update(args).await,
            ScheduleAction::Pause => self.action_pause(args).await,
            ScheduleAction::Resume => self.action_resume(args).await,
            ScheduleAction::Delete => self.action_delete(args).await,
            ScheduleAction::List => self.action_list().await,
            ScheduleAction::Get => self.action_get(args).await,
            ScheduleAction::Trigger => self.action_trigger(args).await,
        }
    }
}

// =========================================================
// Action 实现
// =========================================================

impl ScheduleTool {
    async fn action_create(&self, args: ScheduleArgs) -> Result<String, ScheduleError> {
        let name = args.name.as_deref().map(str::trim).unwrap_or("");
        let cron = args.cron_expression.as_deref().map(str::trim).unwrap_or("");
        let skill_id = args.skill_id.as_deref().map(str::trim).unwrap_or("");

        if name.is_empty() {
            return Err(ScheduleError(
                "create 操作需要 name 参数".to_string(),
            ));
        }
        if cron.is_empty() {
            return Err(ScheduleError(
                "create 操作需要 cron_expression 参数".to_string(),
            ));
        }
        if skill_id.is_empty() {
            return Err(ScheduleError(
                "create 操作需要 skill_id 参数".to_string(),
            ));
        }
        validate_cron(cron)?;

        let id = uuid::Uuid::new_v4().to_string();
        let task = ScheduledTask {
            id: id.clone(),
            name: name.to_string(),
            skill_id: skill_id.to_string(),
            cron: cron.to_string(),
            last_run: None,
            created_at: now_ms(),
            enabled: true,
        };
        self.store
            .save(&task)
            .await
            .map_err(|e| ScheduleError(e.to_string()))?;

        let mut out = String::with_capacity(192);
        out.push_str(&format!(
            "已创建定时任务「{}」(id={})。\ncron: {}  | 技能: {}  | 状态: 启用",
            name, id, cron, skill_id
        ));
        if let Some(msg) = args.message.as_deref() {
            let msg = msg.trim();
            if !msg.is_empty() {
                out.push_str(&format!("\n备注: {}", msg));
            }
        }
        Ok(out)
    }

    async fn action_update(&self, args: ScheduleArgs) -> Result<String, ScheduleError> {
        let id = require_id(&args)?;
        let mut task = self
            .store
            .get(id)
            .await
            .map_err(|e| ScheduleError(e.to_string()))?
            .ok_or_else(|| ScheduleError(format!("任务 {} 不存在", id)))?;

        if let Some(name) = args.name.as_deref() {
            let name = name.trim();
            if !name.is_empty() {
                task.name = name.to_string();
            }
        }
        if let Some(cron) = args.cron_expression.as_deref() {
            let cron = cron.trim();
            if !cron.is_empty() {
                validate_cron(cron)?;
                task.cron = cron.to_string();
            }
        }
        if let Some(sid) = args.skill_id.as_deref() {
            let sid = sid.trim();
            if !sid.is_empty() {
                task.skill_id = sid.to_string();
            }
        }

        self.store
            .save(&task)
            .await
            .map_err(|e| ScheduleError(e.to_string()))?;
        Ok(format!(
            "已更新定时任务「{}」(id={})。",
            task.name,
            short_id(&task.id)
        ))
    }

    async fn action_pause(&self, args: ScheduleArgs) -> Result<String, ScheduleError> {
        self.set_enabled(args, false).await
    }

    async fn action_resume(&self, args: ScheduleArgs) -> Result<String, ScheduleError> {
        self.set_enabled(args, true).await
    }

    /// pause / resume 共用逻辑：按 id 查找后设置 enabled 状态
    async fn set_enabled(
        &self,
        args: ScheduleArgs,
        enabled: bool,
    ) -> Result<String, ScheduleError> {
        let id = require_id(&args)?;
        let mut task = self
            .store
            .get(id)
            .await
            .map_err(|e| ScheduleError(e.to_string()))?
            .ok_or_else(|| ScheduleError(format!("任务 {} 不存在", id)))?;
        task.enabled = enabled;
        self.store
            .save(&task)
            .await
            .map_err(|e| ScheduleError(e.to_string()))?;
        let verb = if enabled { "已恢复" } else { "已暂停" };
        Ok(format!(
            "{}定时任务「{}」(id={})。",
            verb,
            task.name,
            short_id(&task.id)
        ))
    }

    async fn action_delete(&self, args: ScheduleArgs) -> Result<String, ScheduleError> {
        let id = require_id(&args)?;
        // 先读取用于回显任务名，不存在也允许 delete（幂等）
        let task = self
            .store
            .get(id)
            .await
            .map_err(|e| ScheduleError(e.to_string()))?;
        self.store
            .delete(id)
            .await
            .map_err(|e| ScheduleError(e.to_string()))?;
        let name_part = task
            .as_ref()
            .map(|t| format!("「{}」", t.name))
            .unwrap_or_default();
        Ok(format!(
            "已删除定时任务{}(id={})。",
            name_part,
            short_id(id)
        ))
    }

    async fn action_list(&self) -> Result<String, ScheduleError> {
        let tasks = self
            .store
            .list()
            .await
            .map_err(|e| ScheduleError(e.to_string()))?;
        if tasks.is_empty() {
            return Ok("当前没有任何定时任务。".to_string());
        }

        let total = tasks.len();
        let enabled_count = tasks.iter().filter(|t| t.enabled).count();
        let paused_count = total - enabled_count;

        let mut out = String::with_capacity(64 + total * 128);
        out.push_str(&format!(
            "定时任务（共 {} 个，{} 启用，{} 暂停）：\n\n",
            total, enabled_count, paused_count
        ));

        for (i, task) in tasks.iter().enumerate() {
            let marker = if task.enabled {
                "▸ [启用]"
            } else {
                "  [暂停]"
            };
            out.push_str(&format!("{} {} ({})\n", marker, task.name, task.id));
            let last_run_str = task
                .last_run
                .map(format_time)
                .unwrap_or_else(|| "从未执行".to_string());
            out.push_str(&format!(
                "  cron: {}  | 技能: {}  | 上次执行: {}",
                task.cron, task.skill_id, last_run_str
            ));
            // 仅启用任务显示下次执行
            if task.enabled {
                if let Some(next) = compute_next_run(&task.cron, task.last_run) {
                    out.push_str(&format!("\n  下次执行: {}", next.format("%Y-%m-%d %H:%M")));
                }
            }
            if i + 1 < total {
                out.push('\n');
            }
        }
        Ok(out)
    }

    async fn action_get(&self, args: ScheduleArgs) -> Result<String, ScheduleError> {
        let id = require_id(&args)?;
        let task = self
            .store
            .get(id)
            .await
            .map_err(|e| ScheduleError(e.to_string()))?
            .ok_or_else(|| ScheduleError(format!("任务 {} 不存在", id)))?;
        Ok(format_task_detail(&task))
    }

    async fn action_trigger(&self, args: ScheduleArgs) -> Result<String, ScheduleError> {
        let id = require_id(&args)?;
        let task = self
            .store
            .get(id)
            .await
            .map_err(|e| ScheduleError(e.to_string()))?
            .ok_or_else(|| ScheduleError(format!("任务 {} 不存在", id)))?;
        let now = now_ms();
        self.store
            .update_last_run(id, now)
            .await
            .map_err(|e| ScheduleError(e.to_string()))?;
        Ok(format!(
            "已触发定时任务「{}」(id={})，last_run 已更新为当前时间（{}）。\n实际执行由外部调度器在下一 tick 完成。",
            task.name,
            short_id(&task.id),
            format_time(now)
        ))
    }
}

// =========================================================
// 辅助函数
// =========================================================

/// 当前 Unix 毫秒时间戳；失败回退为 0
#[inline]
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 简单 cron 格式校验：5 字段（空格分割为 5 部分）
fn validate_cron(expr: &str) -> Result<(), ScheduleError> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Err(ScheduleError("cron 表达式不能为空".to_string()));
    }
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(ScheduleError(format!(
            "cron 表达式必须是 5 个字段（分 时 日 月 周），当前 {} 个字段: \"{}\"",
            parts.len(),
            trimmed
        )));
    }
    Ok(())
}

/// 格式化 Unix 毫秒为本地时间字符串 "YYYY-MM-DD HH:MM"
fn format_time(ms: u64) -> String {
    Local
        .timestamp_millis_opt(ms as i64)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| format!("unix:{}ms", ms))
}

/// 计算下次执行时刻（本地时间），cron 无效或无下次触发时返回 None
///
/// 与 `tauriFront/src-tauri/src/scheduler.rs` 保持一致：
/// 在 5 字段 cron 前补秒 "0 " 适配 `cron` crate 的 6 字段格式。
fn compute_next_run(cron_expr: &str, last_run_ms: Option<u64>) -> Option<DateTime<Local>> {
    let expr = format!("0 {}", cron_expr.trim());
    let sched = Schedule::from_str(&expr).ok()?;
    let now = Local::now();
    let from: DateTime<Local> = match last_run_ms {
        Some(t) => Local
            .timestamp_millis_opt(t as i64)
            .single()
            .unwrap_or(now),
        None => now,
    };
    sched.after(&from).next().map(|dt| dt.with_timezone(&Local))
}

/// 状态标签
#[inline]
fn status_label(enabled: bool) -> &'static str {
    if enabled {
        "启用"
    } else {
        "暂停"
    }
}

/// 从参数中提取必填的 scheduled_task_id
fn require_id(args: &ScheduleArgs) -> Result<&str, ScheduleError> {
    args.scheduled_task_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ScheduleError("缺少必填参数 scheduled_task_id".to_string()))
}

/// 截断 id 用于显示（取前 8 字符，UTF-8 边界安全）
#[inline]
fn short_id(id: &str) -> String {
    if id.len() <= 8 {
        id.to_string()
    } else {
        id[..id.ceil_char_boundary(8)].to_string()
    }
}

/// 格式化单个任务详情
fn format_task_detail(task: &ScheduledTask) -> String {
    let last_run_str = task
        .last_run
        .map(format_time)
        .unwrap_or_else(|| "从未执行".to_string());
    let next_run_str = if task.enabled {
        compute_next_run(&task.cron, task.last_run)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "无法计算".to_string())
    } else {
        "已暂停".to_string()
    };
    format!(
        "定时任务详情：\n\n  ID: {}\n  名称: {}\n  cron: {}\n  技能: {}\n  状态: {}\n  创建时间: {}\n  上次执行: {}\n  下次执行: {}",
        task.id,
        task.name,
        task.cron,
        task.skill_id,
        status_label(task.enabled),
        format_time(task.created_at),
        last_run_str,
        next_run_str
    )
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("effisuite-sched-tool-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_store() -> Arc<ScheduledTaskStore> {
        Arc::new(ScheduledTaskStore::new(tmp_dir()).unwrap())
    }

    /// 构造仅含 action 的默认参数（其余字段为 None）
    fn args_with(action: ScheduleAction) -> ScheduleArgs {
        ScheduleArgs {
            action,
            scheduled_task_id: None,
            name: None,
            message: None,
            cron_expression: None,
            timezone: None,
            skill_id: None,
        }
    }

    // create + get 验证
    #[tokio::test]
    async fn create_then_get() {
        let store = make_store();
        let tool = ScheduleTool::new(Arc::clone(&store));

        let create_out = tool
            .call(ScheduleArgs {
                action: ScheduleAction::Create,
                name: Some("每日报告".to_string()),
                message: Some("生成日报".to_string()),
                cron_expression: Some("0 9 * * *".to_string()),
                timezone: Some("Asia/Shanghai".to_string()),
                skill_id: Some("agent-reach".to_string()),
                ..args_with(ScheduleAction::Create)
            })
            .await
            .expect("create should succeed");
        assert!(create_out.contains("已创建"));
        assert!(create_out.contains("每日报告"));

        let tasks = store.list().await.unwrap();
        assert_eq!(tasks.len(), 1);
        let id = tasks[0].id.clone();

        let get_out = tool
            .call(ScheduleArgs {
                action: ScheduleAction::Get,
                scheduled_task_id: Some(id),
                ..args_with(ScheduleAction::Get)
            })
            .await
            .expect("get should succeed");
        assert!(get_out.contains("每日报告"));
        assert!(get_out.contains("0 9 * * *"));
        assert!(get_out.contains("agent-reach"));
    }

    // list 空列表
    #[tokio::test]
    async fn list_empty() {
        let store = make_store();
        let tool = ScheduleTool::new(store);
        let out = tool
            .call(args_with(ScheduleAction::List))
            .await
            .unwrap();
        assert!(out.contains("没有任何定时任务"));
    }

    // pause/resume 切换
    #[tokio::test]
    async fn pause_resume_toggle() {
        let store = make_store();
        let tool = ScheduleTool::new(Arc::clone(&store));

        tool.call(ScheduleArgs {
            action: ScheduleAction::Create,
            name: Some("测试任务".to_string()),
            cron_expression: Some("0 9 * * *".to_string()),
            skill_id: Some("agent-reach".to_string()),
            ..args_with(ScheduleAction::Create)
        })
        .await
        .unwrap();

        let id = store.list().await.unwrap()[0].id.clone();

        // pause
        let pause_out = tool
            .call(ScheduleArgs {
                action: ScheduleAction::Pause,
                scheduled_task_id: Some(id.clone()),
                ..args_with(ScheduleAction::Pause)
            })
            .await
            .unwrap();
        assert!(pause_out.contains("已暂停"));
        assert!(!store.get(&id).await.unwrap().unwrap().enabled);

        // resume
        let resume_out = tool
            .call(ScheduleArgs {
                action: ScheduleAction::Resume,
                scheduled_task_id: Some(id.clone()),
                ..args_with(ScheduleAction::Resume)
            })
            .await
            .unwrap();
        assert!(resume_out.contains("已恢复"));
        assert!(store.get(&id).await.unwrap().unwrap().enabled);
    }

    // delete 后 get 返回 None（错误）
    #[tokio::test]
    async fn delete_then_get() {
        let store = make_store();
        let tool = ScheduleTool::new(Arc::clone(&store));

        tool.call(ScheduleArgs {
            action: ScheduleAction::Create,
            name: Some("待删除".to_string()),
            cron_expression: Some("0 9 * * *".to_string()),
            skill_id: Some("agent-reach".to_string()),
            ..args_with(ScheduleAction::Create)
        })
        .await
        .unwrap();

        let id = store.list().await.unwrap()[0].id.clone();

        let del_out = tool
            .call(ScheduleArgs {
                action: ScheduleAction::Delete,
                scheduled_task_id: Some(id.clone()),
                ..args_with(ScheduleAction::Delete)
            })
            .await
            .unwrap();
        assert!(del_out.contains("已删除"));

        // store 层确认已删除
        assert!(store.get(&id).await.unwrap().is_none());

        // 工具层 get 返回错误
        let result = tool
            .call(ScheduleArgs {
                action: ScheduleAction::Get,
                scheduled_task_id: Some(id),
                ..args_with(ScheduleAction::Get)
            })
            .await;
        assert!(result.is_err());
    }

    // 缺少必填字段报错
    #[tokio::test]
    async fn create_missing_required_fields() {
        let store = make_store();
        let tool = ScheduleTool::new(store);

        // 缺 name
        let err = tool
            .call(ScheduleArgs {
                action: ScheduleAction::Create,
                cron_expression: Some("0 9 * * *".to_string()),
                skill_id: Some("agent-reach".to_string()),
                ..args_with(ScheduleAction::Create)
            })
            .await;
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("name"));

        // 缺 cron_expression
        let err = tool
            .call(ScheduleArgs {
                action: ScheduleAction::Create,
                name: Some("x".to_string()),
                skill_id: Some("agent-reach".to_string()),
                ..args_with(ScheduleAction::Create)
            })
            .await;
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("cron"));

        // 缺 skill_id
        let err = tool
            .call(ScheduleArgs {
                action: ScheduleAction::Create,
                name: Some("x".to_string()),
                cron_expression: Some("0 9 * * *".to_string()),
                ..args_with(ScheduleAction::Create)
            })
            .await;
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("skill_id"));
    }

    // 无效 cron 格式报错
    #[tokio::test]
    async fn create_invalid_cron() {
        let store = make_store();
        let tool = ScheduleTool::new(store);

        // 4 字段
        let err = tool
            .call(ScheduleArgs {
                action: ScheduleAction::Create,
                name: Some("x".to_string()),
                cron_expression: Some("0 9 * *".to_string()),
                skill_id: Some("agent-reach".to_string()),
                ..args_with(ScheduleAction::Create)
            })
            .await;
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("5 个字段"));

        // 6 字段
        let err = tool
            .call(ScheduleArgs {
                action: ScheduleAction::Create,
                name: Some("x".to_string()),
                cron_expression: Some("0 9 * * * *".to_string()),
                skill_id: Some("agent-reach".to_string()),
                ..args_with(ScheduleAction::Create)
            })
            .await;
        assert!(err.is_err());
    }

    // update 验证：仅更新提供的字段
    #[tokio::test]
    async fn update_task_fields() {
        let store = make_store();
        let tool = ScheduleTool::new(Arc::clone(&store));

        tool.call(ScheduleArgs {
            action: ScheduleAction::Create,
            name: Some("旧名称".to_string()),
            cron_expression: Some("0 9 * * *".to_string()),
            skill_id: Some("agent-reach".to_string()),
            ..args_with(ScheduleAction::Create)
        })
        .await
        .unwrap();

        let id = store.list().await.unwrap()[0].id.clone();

        tool.call(ScheduleArgs {
            action: ScheduleAction::Update,
            scheduled_task_id: Some(id.clone()),
            name: Some("新名称".to_string()),
            cron_expression: Some("0 10 * * *".to_string()),
            ..args_with(ScheduleAction::Update)
        })
        .await
        .unwrap();

        let task = store.get(&id).await.unwrap().unwrap();
        assert_eq!(task.name, "新名称");
        assert_eq!(task.cron, "0 10 * * *");
        assert_eq!(task.skill_id, "agent-reach"); // 未改
    }

    // trigger 验证：last_run 被更新
    #[tokio::test]
    async fn trigger_updates_last_run() {
        let store = make_store();
        let tool = ScheduleTool::new(Arc::clone(&store));

        tool.call(ScheduleArgs {
            action: ScheduleAction::Create,
            name: Some("触发测试".to_string()),
            cron_expression: Some("0 9 * * *".to_string()),
            skill_id: Some("agent-reach".to_string()),
            ..args_with(ScheduleAction::Create)
        })
        .await
        .unwrap();

        let id = store.list().await.unwrap()[0].id.clone();
        assert!(store.get(&id).await.unwrap().unwrap().last_run.is_none());

        let out = tool
            .call(ScheduleArgs {
                action: ScheduleAction::Trigger,
                scheduled_task_id: Some(id.clone()),
                ..args_with(ScheduleAction::Trigger)
            })
            .await
            .unwrap();
        assert!(out.contains("已触发"));

        assert!(store.get(&id).await.unwrap().unwrap().last_run.is_some());
    }

    // list 格式化输出验证
    #[tokio::test]
    async fn list_formats_output() {
        let store = make_store();
        let tool = ScheduleTool::new(Arc::clone(&store));

        // 创建任务 A（启用）
        tool.call(ScheduleArgs {
            action: ScheduleAction::Create,
            name: Some("任务A".to_string()),
            cron_expression: Some("0 9 * * *".to_string()),
            skill_id: Some("skill-a".to_string()),
            ..args_with(ScheduleAction::Create)
        })
        .await
        .unwrap();

        // 创建任务 B（启用）
        tool.call(ScheduleArgs {
            action: ScheduleAction::Create,
            name: Some("任务B".to_string()),
            cron_expression: Some("0 10 * * 1".to_string()),
            skill_id: Some("skill-b".to_string()),
            ..args_with(ScheduleAction::Create)
        })
        .await
        .unwrap();

        // 暂停任务 B
        let tasks = store.list().await.unwrap();
        let task_b_id = tasks
            .iter()
            .find(|t| t.name == "任务B")
            .map(|t| t.id.clone())
            .unwrap();
        tool.call(ScheduleArgs {
            action: ScheduleAction::Pause,
            scheduled_task_id: Some(task_b_id),
            ..args_with(ScheduleAction::Pause)
        })
        .await
        .unwrap();

        let out = tool
            .call(args_with(ScheduleAction::List))
            .await
            .unwrap();
        assert!(out.contains("共 2 个"));
        assert!(out.contains("1 启用"));
        assert!(out.contains("1 暂停"));
        assert!(out.contains("任务A"));
        assert!(out.contains("任务B"));
        assert!(out.contains("[启用]"));
        assert!(out.contains("[暂停]"));
    }
}

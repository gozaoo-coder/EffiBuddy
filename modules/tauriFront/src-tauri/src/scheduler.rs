//! Cron 调度器：每分钟检查一次，到点触发技能执行
//!
//! 设计要点：
//! - `tokio::time::interval` 每 60s tick 一次（极轻量）
//! - 5 字段 cron（分 时 日 月 周）通过在前面补 "0 "（秒）适配 `cron` crate 的 6 字段格式
//! - 命中判定：计算 last_run 之后的下一次触发时刻，若 <= now 则触发
//! - 触发即更新 last_run，避免同一分钟内重复触发
//! - 实际 agent.chat 执行 spawn 到独立 task，不阻塞调度循环
//! - 锁临界区极短：调度循环只做 store.list 与 update_last_run，agent 读锁仅在 clone 句柄瞬间持有

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::TimeZone;
use cron::Schedule;
use effisuite_agent::ChatAgent;
use effisuite_core::{ConversationStore, Message, Role, ScheduledTaskStore, SkillStore};
use tauri::Emitter;
use tokio::sync::RwLock;

use crate::now_ms;

/// 调度器触发结果，emit 给前端 "scheduled-task-result" 事件
#[derive(Debug, serde::Serialize)]
struct ScheduledTaskResult<'a> {
    task_id: &'a str,
    task_name: &'a str,
    conversation_id: &'a str,
    content: &'a str,
    success: bool,
}

/// 检查任务是否应在此刻触发。
///
/// 返回 `Some(触发时刻 unix 秒)` 表示应触发，`None` 表示未到点或 cron 非法。
/// 算法：在 last_run（或 90s 前）之后寻找下一次触发时刻，若 <= 当前时刻则触发。
fn check_due(cron_expr: &str, last_run_unix_sec: Option<i64>) -> Option<i64> {
    // cron crate 需 6+ 字段：在 5 字段前补秒 "0 "
    let expr = format!("0 {}", cron_expr);
    let sched = Schedule::from_str(&expr).ok()?;
    let now = chrono::Local::now();
    let now_sec = now.timestamp();
    // 搜索起点：有 last_run 则从 last_run 起；否则回退 90s 以覆盖首次启动漏触发
    let from = match last_run_unix_sec {
        Some(t) => chrono::Local.timestamp_opt(t, 0).single().unwrap_or(now),
        None => now - chrono::Duration::seconds(90),
    };
    let next = sched.after(&from).next()?;
    let next_sec = next.timestamp();
    if next_sec <= now_sec {
        Some(next_sec)
    } else {
        None
    }
}

/// 启动调度器，返回 JoinHandle 供 AppState 持有（便于 shutdown 时 abort）。
#[allow(clippy::too_many_arguments)]
pub fn spawn_scheduler(
    app_handle: tauri::AppHandle,
    schedule_store: ScheduledTaskStore,
    skill_store: SkillStore,
    agent_lock: Arc<RwLock<Arc<dyn ChatAgent>>>,
    store: Arc<ConversationStore>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        // 60s 间隔；tokio::time::interval 首次 tick 立即返回，吞掉以避免启动瞬间触发
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(e) = run_tick(
                &app_handle,
                &schedule_store,
                &skill_store,
                &agent_lock,
                &store,
            )
            .await
            {
                tracing::warn!(error = %e, "scheduler tick failed");
            }
        }
    })
}

/// 单次 tick：枚举所有任务，对到点的任务触发执行。
async fn run_tick(
    app_handle: &tauri::AppHandle,
    schedule_store: &ScheduledTaskStore,
    skill_store: &SkillStore,
    agent_lock: &Arc<RwLock<Arc<dyn ChatAgent>>>,
    store: &Arc<ConversationStore>,
) -> Result<(), String> {
    let tasks = schedule_store.list().await.map_err(|e| e.to_string())?;
    let now = now_ms();

    for task in tasks {
        if !task.enabled {
            continue;
        }
        let last_run_sec = task.last_run.map(|t| (t / 1000) as i64);
        if check_due(&task.cron, last_run_sec).is_none() {
            continue;
        }

        // 先更新 last_run，避免同一分钟内重复触发（短临界区）
        if let Err(e) = schedule_store.update_last_run(&task.id, now).await {
            tracing::warn!(error = %e, task_id = %task.id, "update last_run failed, skip");
            continue;
        }

        // 加载技能 preamble
        let skill = match skill_store.get(&task.skill_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::warn!(skill_id = %task.skill_id, "skill not found for scheduled task");
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "load skill failed");
                continue;
            }
        };

        // 执行 spawn 到独立 task，不阻塞调度循环
        let app_handle = app_handle.clone();
        let agent_lock = Arc::clone(agent_lock);
        let store = Arc::clone(store);
        let task_id = task.id.clone();
        let task_name = task.name.clone();
        let preamble = skill.preamble.clone();
        tauri::async_runtime::spawn(async move {
            execute_skill(app_handle, agent_lock, store, task_id, task_name, preamble).await;
        });
    }
    Ok(())
}

/// 在临时会话中执行技能 preamble，emit "scheduled-task-result" 结果。
///
/// 会话 id 形如 `schedule-<task_id>-<ts>`，便于在前端按前缀筛选调度产生的历史。
async fn execute_skill(
    app_handle: tauri::AppHandle,
    agent_lock: Arc<RwLock<Arc<dyn ChatAgent>>>,
    store: Arc<ConversationStore>,
    task_id: String,
    task_name: String,
    preamble: String,
) {
    let now = now_ms();
    let conv_id = format!("schedule-{}-{}", task_id, now);

    // 把 preamble 作为 user 消息注入并取回历史
    let user_msg = Message::new(effisuite_core::gen_message_id(), Role::User, preamble, now);
    let conv = match store.append_message(&conv_id, user_msg, now).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "persist skill preamble failed");
            emit_result(
                &app_handle,
                &task_id,
                &task_name,
                &conv_id,
                &e.to_string(),
                false,
            );
            return;
        }
    };

    let history = conv.history().to_vec();
    // 短读锁：clone 出 Arc<dyn ChatAgent> 后立即释放
    let agent = agent_lock.read().await.clone();
    match agent.chat(&history).await {
        Ok(reply) => {
            let assistant_msg = Message::new(
                effisuite_core::gen_message_id(),
                Role::Assistant,
                reply.clone(),
                now_ms(),
            );
            if let Err(e) = store
                .append_message(&conv_id, assistant_msg, now_ms())
                .await
            {
                tracing::warn!(error = %e, "persist scheduled reply failed");
            }
            emit_result(&app_handle, &task_id, &task_name, &conv_id, &reply, true);
        }
        Err(e) => {
            let msg = e.to_string();
            emit_result(&app_handle, &task_id, &task_name, &conv_id, &msg, false);
        }
    }
}

#[inline]
fn emit_result(
    app_handle: &tauri::AppHandle,
    task_id: &str,
    task_name: &str,
    conversation_id: &str,
    content: &str,
    success: bool,
) {
    let _ = app_handle.emit(
        "scheduled-task-result",
        &ScheduledTaskResult {
            task_id,
            task_name,
            conversation_id,
            content,
            success,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_due_invalid_cron_returns_none() {
        assert!(check_due("not a cron", None).is_none());
    }

    #[test]
    fn check_due_future_returns_none() {
        // 每分钟触发，但 last_run 设为当前时刻 → 下一次在 1 分钟后，不应触发
        let now_sec = chrono::Local::now().timestamp();
        assert!(check_due("* * * * *", Some(now_sec)).is_none());
    }
}

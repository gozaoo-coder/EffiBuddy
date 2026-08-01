//! 流式会话管理：跟踪活跃 ASR 会话的元数据与生命周期。
//!
//! 独立于具体 [`AsrProvider`](super::provider::AsrProvider) 实现：
//! provider 负责 WebSocket/HTTP 通信，session 模块负责"业务侧"的会话状态——
//! 创建时间、语言、关联的 record_id、状态机迁移等。
//!
//! # 设计要点（对齐 user_rules）
//!
//! - `SessionRegistry` 用 `Arc<StdMutex<HashMap>>`：临界区极短（仅查表/插入/移除）
//! - 状态变更通过 [`EventBus`] 通知前端，遵循"消息传递代替共享内存"
//! - 不持有 provider 句柄：上层 [`AsrService`](super::AsrService) 协调两者
//! - `SessionInfo` 字段按大小降序，最小化 padding

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use effisuite_core::{BusEvent, EventBus};

use super::error::AsrError;

/// 会话状态（业务侧视角，与 provider 内部连接状态互补）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// 已启动，正在接收音频
    Active,
    /// 调用方主动结束，等待最终转写
    Finishing,
    /// 已完成（finish 返回了转写文本）
    Completed,
    /// 已取消
    Cancelled,
    /// 出错终止
    Failed,
}

impl SessionState {
    /// 是否处于终态（不可再操作）
    #[inline]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            SessionState::Completed | SessionState::Cancelled | SessionState::Failed
        )
    }
}

/// 单个流式会话的业务元数据
///
/// 字段按大小降序：String(24) > Instant(8) > SessionState(1)。
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub session_id: String,
    /// 关联的 ASR 记录 id（finish 后用于持久化）
    pub record_id: Option<String>,
    pub language: String,
    pub started_at: Instant,
    pub state: SessionState,
}

/// 会话注册表：跟踪活跃流式会话
///
/// 所有方法 `&self`：内部用 `Arc<StdMutex<HashMap>>`，临界区极短（仅 HashMap 操作）。
/// 不执行 IO，不在锁内 await。
#[derive(Clone)]
pub struct SessionRegistry {
    sessions: Arc<StdMutex<HashMap<String, SessionInfo>>>,
    event_bus: Option<Arc<EventBus>>,
}

impl SessionRegistry {
    /// 构造空注册表。`event_bus` 用于发布 `AsrSessionStatus` 事件。
    pub fn new(event_bus: Option<Arc<EventBus>>) -> Self {
        Self {
            sessions: Arc::new(StdMutex::new(HashMap::new())),
            event_bus,
        }
    }

    /// 注册新会话。若 session_id 已存在返回 `Protocol` 错误。
    /// 发布 `started` 状态事件。
    pub fn register(&self, session_id: String, language: &str) -> Result<(), AsrError> {
        let info = SessionInfo {
            session_id: session_id.clone(),
            record_id: None,
            language: language.to_string(),
            started_at: Instant::now(),
            state: SessionState::Active,
        };
        {
            let mut map = self.sessions.lock().unwrap();
            if map.contains_key(&session_id) {
                return Err(AsrError::Protocol(format!(
                    "会话 {session_id} 已存在"
                )));
            }
            map.insert(session_id.clone(), info);
        }
        self.publish_status(&session_id, "started", None);
        Ok(())
    }

    /// 更新会话状态。会话不存在时静默返回（provider 可能已自行清理）。
    /// 终态状态下再变更视为错误（避免 finish 后再 cancel 误触发事件）。
    pub fn transition(&self, session_id: &str, new_state: SessionState) -> Result<(), AsrError> {
        let mut map = self.sessions.lock().unwrap();
        let Some(info) = map.get_mut(session_id) else {
            // 会话不在注册表中（可能已被清理）：仅发布事件，不报错
            drop(map);
            self.publish_status(
                session_id,
                state_to_status_str(new_state),
                None,
            );
            return Ok(());
        };
        if info.state.is_terminal() {
            return Err(AsrError::SessionFinished(format!(
                "会话 {session_id} 已处于终态 {:?}，无法迁移到 {:?}",
                info.state, new_state
            )));
        }
        info.state = new_state;
        let status_str = state_to_status_str(new_state);
        drop(map);
        self.publish_status(session_id, status_str, None);
        Ok(())
    }

    /// 关联 record_id（finish 成功后由 AsrService 调用）
    pub fn set_record_id(&self, session_id: &str, record_id: String) -> Result<(), AsrError> {
        let mut map = self.sessions.lock().unwrap();
        let Some(info) = map.get_mut(session_id) else {
            return Err(AsrError::SessionNotFound(session_id.to_string()));
        };
        info.record_id = Some(record_id);
        Ok(())
    }

    /// 标记会话失败并记录错误信息
    pub fn mark_failed(&self, session_id: &str, error: String) {
        let _ = self.sessions.lock().unwrap().get_mut(session_id).map(|info| {
            info.state = SessionState::Failed;
        });
        self.publish_status(session_id, "failed", Some(error));
    }

    /// 获取会话信息快照（clone）
    pub fn get(&self, session_id: &str) -> Option<SessionInfo> {
        self.sessions.lock().unwrap().get(session_id).cloned()
    }

    /// 移除会话（finish/cancel 后由 AsrService 调用，避免注册表无限增长）
    pub fn remove(&self, session_id: &str) -> Option<SessionInfo> {
        self.sessions.lock().unwrap().remove(session_id)
    }

    /// 列出所有活跃会话的快照（供调试/UI 列表）
    pub fn list_active(&self) -> Vec<SessionInfo> {
        let map = self.sessions.lock().unwrap();
        map.values()
            .filter(|i| !i.state.is_terminal())
            .cloned()
            .collect()
    }

    /// 当前活跃会话数（不含终态）
    #[inline]
    pub fn active_count(&self) -> usize {
        let map = self.sessions.lock().unwrap();
        map.values().filter(|i| !i.state.is_terminal()).count()
    }

    /// 发布会话状态事件（锁外执行，避免锁内 IO）
    #[inline]
    fn publish_status(&self, session_id: &str, status: &str, error: Option<String>) {
        if let Some(bus) = &self.event_bus {
            bus.publish(BusEvent::AsrSessionStatus {
                session_id: session_id.to_string(),
                status: status.to_string(),
                error,
            });
        }
    }

    /// 内部访问 event_bus 句柄（供 AsrService 发布 AsrRecordUpdated 等事件）
    #[inline]
    pub(super) fn event_bus_ref(&self) -> Option<Arc<EventBus>> {
        self.event_bus.clone()
    }
}

/// 把 `SessionState` 映射为事件总线使用的状态字符串
#[inline]
fn state_to_status_str(state: SessionState) -> &'static str {
    match state {
        SessionState::Active => "transcribing",
        SessionState::Finishing => "finishing",
        SessionState::Completed => "completed",
        SessionState::Cancelled => "cancelled",
        SessionState::Failed => "failed",
    }
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_get() {
        let reg = SessionRegistry::new(None);
        reg.register("s1".into(), "zh-CN").unwrap();
        let info = reg.get("s1").unwrap();
        assert_eq!(info.session_id, "s1");
        assert_eq!(info.language, "zh-CN");
        assert_eq!(info.state, SessionState::Active);
        assert!(info.record_id.is_none());
    }

    #[test]
    fn register_duplicate_returns_error() {
        let reg = SessionRegistry::new(None);
        reg.register("s1".into(), "zh-CN").unwrap();
        let err = reg.register("s1".into(), "zh-CN").unwrap_err();
        assert!(matches!(err, AsrError::Protocol(_)));
    }

    #[test]
    fn transition_to_finishing_then_completed() {
        let reg = SessionRegistry::new(None);
        reg.register("s1".into(), "zh-CN").unwrap();
        reg.transition("s1", SessionState::Finishing).unwrap();
        assert_eq!(reg.get("s1").unwrap().state, SessionState::Finishing);
        reg.transition("s1", SessionState::Completed).unwrap();
        assert_eq!(reg.get("s1").unwrap().state, SessionState::Completed);
    }

    #[test]
    fn transition_from_terminal_fails() {
        let reg = SessionRegistry::new(None);
        reg.register("s1".into(), "zh-CN").unwrap();
        reg.transition("s1", SessionState::Completed).unwrap();
        let err = reg.transition("s1", SessionState::Cancelled).unwrap_err();
        assert!(matches!(err, AsrError::SessionFinished(_)));
    }

    #[test]
    fn transition_nonexistent_succeeds_silently() {
        let reg = SessionRegistry::new(None);
        // 不存在的会话不报错（provider 可能已清理）
        reg.transition("ghost", SessionState::Completed).unwrap();
    }

    #[test]
    fn set_record_id_updates_info() {
        let reg = SessionRegistry::new(None);
        reg.register("s1".into(), "zh-CN").unwrap();
        reg.set_record_id("s1", "rec-123".into()).unwrap();
        assert_eq!(reg.get("s1").unwrap().record_id.as_deref(), Some("rec-123"));
    }

    #[test]
    fn set_record_id_nonexistent_fails() {
        let reg = SessionRegistry::new(None);
        let err = reg.set_record_id("ghost", "rec".into()).unwrap_err();
        assert!(matches!(err, AsrError::SessionNotFound(_)));
    }

    #[test]
    fn mark_failed_sets_state() {
        let reg = SessionRegistry::new(None);
        reg.register("s1".into(), "zh-CN").unwrap();
        reg.mark_failed("s1", "网络错误".into());
        assert_eq!(reg.get("s1").unwrap().state, SessionState::Failed);
    }

    #[test]
    fn remove_returns_info() {
        let reg = SessionRegistry::new(None);
        reg.register("s1".into(), "zh-CN").unwrap();
        let removed = reg.remove("s1").unwrap();
        assert_eq!(removed.session_id, "s1");
        assert!(reg.get("s1").is_none());
    }

    #[test]
    fn list_active_excludes_terminal() {
        let reg = SessionRegistry::new(None);
        reg.register("s1".into(), "zh-CN").unwrap();
        reg.register("s2".into(), "en-US").unwrap();
        reg.transition("s2", SessionState::Completed).unwrap();
        let active = reg.list_active();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].session_id, "s1");
        assert_eq!(reg.active_count(), 1);
    }

    #[test]
    fn state_to_status_str_mapping() {
        assert_eq!(state_to_status_str(SessionState::Active), "transcribing");
        assert_eq!(state_to_status_str(SessionState::Finishing), "finishing");
        assert_eq!(state_to_status_str(SessionState::Completed), "completed");
        assert_eq!(state_to_status_str(SessionState::Cancelled), "cancelled");
        assert_eq!(state_to_status_str(SessionState::Failed), "failed");
    }

    #[test]
    fn is_terminal_correctness() {
        assert!(!SessionState::Active.is_terminal());
        assert!(!SessionState::Finishing.is_terminal());
        assert!(SessionState::Completed.is_terminal());
        assert!(SessionState::Cancelled.is_terminal());
        assert!(SessionState::Failed.is_terminal());
    }
}

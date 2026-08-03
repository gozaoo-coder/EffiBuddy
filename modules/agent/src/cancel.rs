//! Agent 运行取消注册表：按会话 conversation_id 维护「取消令牌」。
//!
//! 语义：`send_message_stream` spawn 的驱动 task 在流开始前 `register`，
//! 拿到一个 `watch::Receiver<bool>`（初始 `false`）。用户点击「停止」→
//! Tauri 命令 `stop_agent` → `cancel(conv_id)` 把值置为 `true`。驱动 task
//! 在流式循环的每个 chunk 边界（`tokio::select!`）轮询 receiver 感知取消，
//! 立即终止并持久化已产生的部分内容。
//!
//! 生命周期：`register`（流开始）→ `cancel`（用户停止）或 `unregister`
//! （流自然结束 / 出错）。`register` 会覆盖同一会话的旧令牌：旧 sender 被
//! drop 后旧 receiver 的 `changed()` 返回 `Err`，旧驱动 task 同样按「已取消」
//! 处理退出——保证任何时刻只有一个活动流，且旧流必然收敛。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

/// 会话级 agent 运行取消注册表。
///
/// 内部用 `Mutex<HashMap<String, watch::Sender<bool>>>` 持有每个会话的
/// 取消发送端；命令层 / 驱动 task 共享同一个 `Arc`。读多写少、临界区极短，
/// 用标准库 `Mutex` 即可（持锁期间仅做 HashMap 查插，无 await）。
#[derive(Debug, Clone, Default)]
pub struct AgentCancelRegistry {
    inner: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
}

impl AgentCancelRegistry {
    /// 创建空的取消注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册（或覆盖）某会话的取消令牌，返回对应的接收端。
    ///
    /// 调用方（`send_message_stream` 的驱动 task）持有返回的 receiver，
    /// 在流式循环里轮询。覆盖旧 sender：旧 receiver 的 `changed()` 随即返回
    /// `Err`，按「已取消」收敛，防止并发双流。
    pub fn register(&self, conv_id: &str) -> watch::Receiver<bool> {
        let (tx, rx) = watch::channel(false);
        self.inner.lock().unwrap().insert(conv_id.to_string(), tx);
        rx
    }

    /// 触发取消：把某会话的取消标志置为 `true`。
    ///
    /// 返回是否确实存在活动令牌（无活动流时返回 `false`，调用方可据此判断
    /// 停止操作是否命中正在运行的 agent）。
    pub fn cancel(&self, conv_id: &str) -> bool {
        let tx = self.inner.lock().unwrap().get(conv_id).cloned();
        match tx {
            Some(tx) => tx.send(true).is_ok(),
            None => false,
        }
    }

    /// 查询某会话是否已处于取消状态（无活动令牌时视为未取消）。
    pub fn is_cancelled(&self, conv_id: &str) -> bool {
        let tx = self.inner.lock().unwrap().get(conv_id).cloned();
        match tx {
            Some(tx) => *tx.borrow(),
            None => false,
        }
    }

    /// 移除某会话的取消令牌。
    ///
    /// 流自然结束 / 出错 / 已处理停止后调用，避免 HashMap 随会话数无限膨胀。
    /// 移除后 `is_cancelled` 返回 `false`，新一轮 `register` 重新建令牌。
    pub fn unregister(&self, conv_id: &str) {
        self.inner.lock().unwrap().remove(conv_id);
    }

    /// 当前登记的活动会话数（调试 / 健康检查用）。
    pub fn active_count(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancel_triggers_receiver() {
        let reg = AgentCancelRegistry::new();
        let mut rx = reg.register("conv-1");
        assert!(!*rx.borrow());
        assert!(reg.cancel("conv-1"));
        // changed() 应在取消后被唤醒
        rx.changed().await.unwrap();
        assert!(*rx.borrow());
    }

    #[tokio::test]
    async fn cancel_missing_conversation_returns_false() {
        let reg = AgentCancelRegistry::new();
        assert!(!reg.cancel("nope"));
        assert!(!reg.is_cancelled("nope"));
    }

    #[tokio::test]
    async fn unregister_closes_channel() {
        let reg = AgentCancelRegistry::new();
        let mut rx = reg.register("conv-2");
        reg.unregister("conv-2");
        // sender 被 drop 后 changed() 返回 Err，驱动 task 按取消处理
        assert!(rx.changed().await.is_err());
    }

    #[tokio::test]
    async fn re_register_overrides_old_sender() {
        let reg = AgentCancelRegistry::new();
        let mut rx_old = reg.register("conv-3");
        let _rx_new = reg.register("conv-3");
        // 旧 receiver 的 sender 已被覆盖 drop → 视为取消
        assert!(rx_old.changed().await.is_err());
        assert!(reg.is_cancelled("conv-3") == false);
    }
}

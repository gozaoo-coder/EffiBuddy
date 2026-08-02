//! 远端任务派发抽象（依赖倒置：agent 依赖此 trait，p2p 实现此 trait）。
//!
//! agent crate 不依赖 `effisuite-p2p`，仅依赖本 trait；P2pManager 在 p2p crate
//! 中实现此 trait，由 Tauri 命令层注入到 agent 工具。这样 agent 编译不引入
//! 加密 / 网络依赖，符合 core "零业务逻辑、零外部 IO 依赖" 的边界约束。
//!
//! # 临界区
//! 实现方应避免在派发路径上持有长锁；远端 RTT 通常百毫秒级，锁内等待会阻塞其他派发。

use async_trait::async_trait;

use crate::{Device, Result};

/// 远端任务派发器
///
/// 由 [`crate::P2pManager`]（实现在 `effisuite-p2p` crate）实现，
/// 供 agent 的 `dispatch_remote_task` 工具调用，实现 AI 跨设备指派任务。
#[async_trait]
pub trait RemoteTaskDispatcher: Send + Sync {
    /// 列出当前在线且已配对的设备（AI 据此选择派发目标）
    async fn list_online_devices(&self) -> Vec<Device>;

    /// 向指定设备派发任务，返回远端 AI 处理后的结果文本
    ///
    /// - `device_id`：目标设备 id（必须在信任库中且在线）
    /// - `task`：任务描述（自然语言，远端 AI 处理）
    /// - 返回：远端 AI 回复文本；设备离线 / 不信任 / 超时返回 `Err`
    async fn dispatch_remote_task(&self, device_id: &str, task: &str) -> Result<String>;
}

/// 当前 Unix 秒时间戳（用于 `last_seen` 等场景）。
///
/// 放在 core 而非 p2p 模块，便于 manager 在不引入时间依赖的情况下复用。
/// 失败时返回 0，绝不 panic。
#[inline]
pub fn remote_task_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

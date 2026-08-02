//! 镜像同步：按时间顺序同步会话/插件/用户缓存。
//!
//! **本文件为占位实现**，正式实现待后续完善。当前仅满足 lib.rs / manager.rs 编译。

use std::sync::Arc;

use effisuite_core::{EventBus, Result};

use crate::protocol::SyncKind;
use crate::transport::Transport;
use crate::protocol::WireMessage;

/// 镜像同步器（占位实现）
pub struct Sync {
    _transport: Arc<Transport>,
    _event_bus: EventBus,
}

impl Sync {
    pub fn new(transport: Arc<Transport>, event_bus: EventBus) -> Self {
        Self {
            _transport: transport,
            _event_bus: event_bus,
        }
    }

    pub async fn pull(&self, _device_id: &str, _since: u64, _kinds: &[SyncKind]) -> Result<()> {
        Ok(())
    }

    pub async fn push(&self, _device_id: &str, _kinds: &[SyncKind]) -> Result<()> {
        Ok(())
    }

    pub async fn sync_cursor(&self, _device_id: &str) -> u64 {
        0
    }

    pub async fn handle_request(
        &self,
        _device_id: &str,
        _since: u64,
        _kinds: &[SyncKind],
    ) -> Result<()> {
        Ok(())
    }

    pub async fn handle_fetch(
        &self,
        _device_id: &str,
        _conversation_id: &str,
        _since_msg_ts: u64,
    ) -> Result<()> {
        Ok(())
    }

    pub async fn handle_incoming(&self, _device_id: &str, _msg: WireMessage) -> Result<()> {
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        Ok(())
    }
}

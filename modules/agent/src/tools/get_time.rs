//! get_time 工具：返回当前时间字符串
//!
//! 最简单的工具示例，用于验证 rig tool 调用链路是否打通。
//! LLM 可在用户问"现在几点"时调用此工具。

use effisuite_core::Message;
use rig_core::tool::Tool;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 工具参数（无参数，但 rig 要求 Args 实现 Deserialize）
#[derive(Deserialize)]
pub struct GetTimeArgs;

/// 工具错误（thiserror 派生，满足 Tool::Error bound）
#[derive(Debug, thiserror::Error)]
#[error("time tool error: {0}")]
pub struct TimeToolError(String);

/// 获取当前时间的工具
///
/// `history` 字段保留以便未来扩展（如带上时区偏好等），当前未使用。
/// 字段顺序：Arc（1 usize）在前。
pub struct GetTimeTool {
    _history: Arc<RwLock<Vec<Message>>>,
}

impl GetTimeTool {
    pub fn new(history: Arc<RwLock<Vec<Message>>>) -> Self {
        Self { _history: history }
    }
}

impl Tool for GetTimeTool {
    const NAME: &'static str = "get_time";

    type Error = TimeToolError;
    type Args = GetTimeArgs;
    type Output = String;

    fn description(&self) -> String {
        "获取当前本地时间，格式为 ISO 8601。当用户询问当前时间、日期时调用。".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "description": "无参数"
        })
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let now = chrono::Local::now();
        Ok(now.to_rfc3339())
    }
}

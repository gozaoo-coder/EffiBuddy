//! 统一错误类型
//!
//! 各子模块以字符串形式向上传递错误，core 在此提供 `CoreError` 作为
//! 跨模块的通用错误枚举，避免上层被迫依赖下层具体错误类型。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    /// 业务逻辑错误（版本控制等通用错误信息）
    #[error("{0}")]
    Msg(String),

    #[error("agent error: {0}")]
    Agent(String),

    #[error("p2p error: {0}")]
    P2p(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;

impl From<CoreError> for String {
    /// 便于 Tauri 命令把错误直接以字符串返回前端
    #[inline]
    fn from(e: CoreError) -> Self {
        e.to_string()
    }
}

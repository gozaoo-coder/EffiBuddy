use crate::CoreError;

/// ClawHub 专用错误类型。
///
/// `RateLimited` 携带 `retry_after`（秒），便于上层实现抖动退避。
#[derive(Debug, thiserror::Error)]
pub enum ClawHubError {
    #[error("clawhub request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("clawhub rate limited; retry after {retry_after:?} seconds")]
    RateLimited { retry_after: Option<u64> },

    #[error("clawhub api error: status={status}, body={body}")]
    Api { status: u16, body: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("zip extract error: {0}")]
    Zip(String),

    #[error("invalid response: {0}")]
    Decode(String),

    #[error("skill slug not found: {0}")]
    SlugNotFound(String),

    #[error("package not found: {0}")]
    PackageNotFound(String),
}

impl From<ClawHubError> for CoreError {
    #[inline]
    fn from(e: ClawHubError) -> Self {
        CoreError::Config(e.to_string())
    }
}

impl From<ClawHubError> for String {
    /// 便于 Tauri 命令把错误直接以字符串返回前端
    #[inline]
    fn from(e: ClawHubError) -> Self {
        e.to_string()
    }
}

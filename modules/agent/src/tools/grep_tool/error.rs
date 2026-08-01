/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("grep error: {0}")]
pub struct GrepError(pub(super) String);

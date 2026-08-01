//! ASR 模块统一错误类型
//!
//! 独立文件以避免 provider.rs 成为"上帝文件"。所有 ASR 子模块共享此错误类型。

use thiserror::Error;

/// ASR 转写错误
#[derive(Debug, Error)]
pub enum AsrError {
    #[error("网络错误: {0}")]
    Network(String),

    #[error("协议错误: {0}")]
    Protocol(String),

    #[error("鉴权失败: {0}")]
    Auth(String),

    #[error("音频格式错误: {0}")]
    AudioFormat(String),

    #[error("ASR 未配置: {0}")]
    NotConfigured(String),

    #[error("会话不存在: {0}")]
    SessionNotFound(String),

    #[error("会话已结束: {0}")]
    SessionFinished(String),

    #[error("转写失败: {0}")]
    Transcribe(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),
}

impl AsrError {
    /// 从 reqwest 错误构造 Network 变体（截断过长的错误信息）
    #[inline]
    pub fn from_reqwest(e: reqwest::Error) -> Self {
        Self::Network(e.to_string())
    }
}

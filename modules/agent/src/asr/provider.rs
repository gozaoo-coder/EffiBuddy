//! ASR Provider 统一抽象 trait 与共享数据结构。
//!
//! 把"火山引擎"与"千问 Qwen-Omni"两种转写后端收敛到同一个 [`AsrProvider`] trait，
//! 上层 [`AsrService`](super::AsrService) 只依赖 trait，可在 provider 间零成本切换。
//!
//! # 设计要点（对齐 user_rules）
//!
//! - trait 方法全部 `&self`：provider 内部状态（活跃 WebSocket 会话表）用
//!   `Arc<StdMutex<HashMap<...>>>` 包裹，临界区极短（仅查表 clone channel sender，
//!   WebSocket 写入在锁外完成）
//! - 流式音频推送走 mpsc channel，不共享 `Arc<Mutex<Vec<u8>>>`
//! - `AudioStreamConfig` / `TranscribeResult` 字段按大小降序

use std::path::Path;

use async_trait::async_trait;

use super::error::AsrError;

/// 流式音频参数要求：provider 期望前端推送的 PCM 格式
///
/// 字段按大小降序：u32(4) = u32(4) > u16(2)。
#[derive(Debug, Clone, Copy)]
pub struct AudioStreamConfig {
    pub sample_rate: u32,
    pub frame_ms: u32,
    pub bits: u16,
    pub channels: u16,
}

impl AudioStreamConfig {
    /// 火山引擎/千问统一默认：PCM 16kHz/16bit/单声道/200ms 一帧
    pub const DEFAULT: Self = Self {
        sample_rate: 16000,
        frame_ms: 200,
        bits: 16,
        channels: 1,
    };

    /// 单帧字节数 = sample_rate * channels * (bits/8) * frame_ms / 1000
    ///
    /// 注意：先乘后除，避免 `frame_ms / 1000` 整数截断为 0（如 200 / 1000 = 0）。
    #[inline]
    pub fn frame_bytes(&self) -> usize {
        (self.sample_rate as usize)
            * (self.channels as usize)
            * (self.bits as usize / 8)
            * (self.frame_ms as usize)
            / 1000
    }
}

/// 文件转写结果
///
/// 字段按大小降序：String(24) > u64(8)。
#[derive(Debug, Clone)]
pub struct TranscribeResult {
    pub text: String,
    pub duration_ms: u64,
}

/// ASR 转写后端统一抽象
///
/// 实现方负责：
/// - 流式：管理 WebSocket 会话生命周期，内部用 channel 收发音频帧
/// - 文件：一次性 HTTP 请求转写整段音频
///
/// 所有方法 `&self`：内部状态用 `Arc<StdMutex<...>>` 包裹，临界区极短。
#[async_trait]
pub trait AsrProvider: Send + Sync {
    /// 启动流式会话，返回音频参数要求。
    /// `session_id` 由调用方生成（uuid），provider 据此建立连接并跟踪会话。
    async fn start_streaming(
        &self,
        session_id: String,
        lang: &str,
    ) -> Result<AudioStreamConfig, AsrError>;

    /// 推送一帧 PCM 音频（格式需匹配 start_streaming 返回的 config）。
    /// 非阻塞：内部通过 channel 转发到 session task，立即返回。
    async fn push_audio_chunk(&self, session_id: &str, pcm: &[u8]) -> Result<(), AsrError>;

    /// 结束流式，返回完整转写文本。
    /// 发送结束信号后等待 provider 返回最终结果。
    async fn finish_streaming(&self, session_id: &str) -> Result<String, AsrError>;

    /// 取消流式会话（幂等，不存在的 session 视为成功）。
    async fn cancel_streaming(&self, session_id: &str) -> Result<(), AsrError>;

    /// 上传音频文件做一次性转写（同步返回完整文本）。
    async fn transcribe_file(
        &self,
        audio_path: &Path,
        lang: &str,
    ) -> Result<TranscribeResult, AsrError>;
}

#[cfg(test)]
pub(crate) mod mock {
    //! 供 session / service 层单测使用的 mock provider

    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;

    use super::{AsrProvider, AudioStreamConfig, TranscribeResult};
    use crate::asr::error::AsrError;

    /// 记录调用次数的 mock provider，用于验证 session 生命周期
    pub struct MockAsrProvider {
        pub start_calls: AtomicUsize,
        pub push_calls: AtomicUsize,
        pub finish_calls: AtomicUsize,
        pub cancel_calls: AtomicUsize,
        pub file_calls: AtomicUsize,
        pub transcript: Arc<std::sync::Mutex<String>>,
    }

    impl MockAsrProvider {
        pub fn new(transcript: &str) -> Self {
            Self {
                start_calls: AtomicUsize::new(0),
                push_calls: AtomicUsize::new(0),
                finish_calls: AtomicUsize::new(0),
                cancel_calls: AtomicUsize::new(0),
                file_calls: AtomicUsize::new(0),
                transcript: Arc::new(std::sync::Mutex::new(transcript.to_string())),
            }
        }
    }

    #[async_trait]
    impl AsrProvider for MockAsrProvider {
        async fn start_streaming(
            &self,
            _session_id: String,
            _lang: &str,
        ) -> Result<AudioStreamConfig, AsrError> {
            self.start_calls.fetch_add(1, Ordering::SeqCst);
            Ok(AudioStreamConfig::DEFAULT)
        }

        async fn push_audio_chunk(&self, _session_id: &str, _pcm: &[u8]) -> Result<(), AsrError> {
            self.push_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn finish_streaming(&self, _session_id: &str) -> Result<String, AsrError> {
            self.finish_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.transcript.lock().unwrap().clone())
        }

        async fn cancel_streaming(&self, _session_id: &str) -> Result<(), AsrError> {
            self.cancel_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn transcribe_file(
            &self,
            _audio_path: &Path,
            _lang: &str,
        ) -> Result<TranscribeResult, AsrError> {
            self.file_calls.fetch_add(1, Ordering::SeqCst);
            Ok(TranscribeResult {
                text: self.transcript.lock().unwrap().clone(),
                duration_ms: 1000,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_bytes_default() {
        // 16000 * 1 * 2 * 0.2 = 6400
        assert_eq!(AudioStreamConfig::DEFAULT.frame_bytes(), 6400);
    }
}

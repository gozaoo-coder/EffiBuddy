//! 千问 Qwen-Omni ASR provider（OpenAI 兼容协议）
//!
//! # 工作方式
//!
//! Qwen-Omni 是"整段输入"模型，不支持真正的流式音频推送。因此：
//! - **流式**：前端 push_audio_chunk 时在前端累积完整音频，finish_streaming 时一次性
//!   调用 Qwen 转写。流式 chunk 事件不发（或发"处理中"状态）。
//!   实现上 provider 在 start_streaming 时建立内部状态，push_audio_chunk 把音频帧
//!   累积到 session buffer，finish_streaming 把累积的 PCM 整体发送给 Qwen。
//! - **文件转写**：base64 编码整个音频，system prompt 引导纯转写，SSE 流式解析。
//!
//! # API
//!
//! - POST `{base_url}/chat/completions`
//! - Authorization: Bearer {api_key}
//! - model = qwen_audio_model（默认 qwen-audio-asr）
//! - messages 含 input_audio ContentPart（data:audio/{format};base64,{b64}）
//! - modalities: ["text"]，stream: true（SSE 格式响应）
//!
//! # 并发与性能（对齐 user_rules）
//!
//! - session buffer 用 mpsc channel 传递，不共享 `Arc<Mutex<Vec<u8>>>`
//! - SSE 解析为纯函数，可独立单测
//! - HTTP 请求在锁外完成

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use base64::Engine;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use effisuite_core::{BusEvent, EventBus};

use super::error::AsrError;
use super::provider::{AsrProvider, AudioStreamConfig, TranscribeResult};

/// HTTP 请求超时（Qwen-Omni 转写可能较慢）
const REQUEST_TIMEOUT_SECS: u64 = 120;
/// session task 音频帧 channel 容量
const AUDIO_CHANNEL_CAPACITY: usize = 256;
/// 默认 base_url
const DEFAULT_BASE_URL: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";
/// 错误信息最大字符数
const MAX_ERR_CHARS: usize = 500;

/// 发送给 session task 的命令
enum QwenCmd {
    /// 累积一帧音频（PCM）
    Audio(Vec<u8>),
    /// 结束流式，携带结果回传通道
    Finish(oneshot::Sender<Result<String, AsrError>>),
    /// 取消流式
    Cancel,
}

/// 活跃流式会话条目
struct QwenSession {
    cmd_tx: mpsc::Sender<QwenCmd>,
}

/// 千问 Qwen-Omni ASR provider
///
/// 字段按大小降序：String(24) > Arc<...>(1 usize)
pub struct QwenProvider {
    api_key: String,
    base_url: String,
    audio_model: String,
    /// 活跃流式会话表：临界区极短（仅查表 clone sender）
    sessions: Arc<StdMutex<HashMap<String, QwenSession>>>,
    /// 事件总线（None 时不推送状态事件）
    event_bus: Option<Arc<EventBus>>,
    /// HTTP 客户端（连接池化）
    http_client: reqwest::Client,
}

impl QwenProvider {
    /// 构造 provider
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        audio_model: impl Into<String>,
        event_bus: Option<Arc<EventBus>>,
    ) -> Self {
        let base_url = {
            let b = base_url.into();
            if b.is_empty() {
                DEFAULT_BASE_URL.to_string()
            } else {
                b
            }
        };
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            api_key: api_key.into(),
            base_url,
            audio_model: audio_model.into(),
            sessions: Arc::new(StdMutex::new(HashMap::new())),
            event_bus,
            http_client,
        }
    }

    /// 从会话表中移除并返回 session
    fn take_session(&self, session_id: &str) -> Option<QwenSession> {
        self.sessions.lock().unwrap().remove(session_id)
    }

    /// spawn 流式 session task：累积音频，finish 时一次性调用 Qwen 转写
    fn spawn_session_task(
        &self,
        session_id: String,
        lang: String,
        cmd_rx: mpsc::Receiver<QwenCmd>,
    ) {
        let api_key = self.api_key.clone();
        let base_url = self.base_url.clone();
        let audio_model = self.audio_model.clone();
        let sessions = Arc::clone(&self.sessions);
        let event_bus = self.event_bus.clone();
        let session_id_for_task = session_id.clone();

        tokio::spawn(async move {
            let result = run_qwen_session(
                &api_key,
                &base_url,
                &audio_model,
                &session_id_for_task,
                &lang,
                cmd_rx,
                event_bus.as_deref(),
            )
            .await;
            sessions.lock().unwrap().remove(&session_id_for_task);
            if let Err(e) = &result {
                warn!(session_id = %session_id_for_task, error = %e, "Qwen ASR 会话异常结束");
            }
        });
    }
}

/// 运行 Qwen 流式会话：累积音频帧，finish 时一次性转写
async fn run_qwen_session(
    api_key: &str,
    base_url: &str,
    audio_model: &str,
    session_id: &str,
    _lang: &str,
    mut cmd_rx: mpsc::Receiver<QwenCmd>,
    event_bus: Option<&EventBus>,
) -> Result<(), AsrError> {
    let mut audio_buffer: Vec<u8> = Vec::with_capacity(16000 * 2 * 30); // 预分配 30s PCM

    if let Some(bus) = &event_bus {
        bus.publish(BusEvent::AsrSessionStatus {
            session_id: session_id.to_string(),
            status: "transcribing".to_string(),
            error: None,
        });
    }

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            QwenCmd::Audio(pcm) => {
                audio_buffer.extend_from_slice(&pcm);
            }
            QwenCmd::Finish(result_tx) => {
                // 一次性转写累积的 PCM
                let b64 = base64::engine::general_purpose::STANDARD.encode(&audio_buffer);
                let result = transcribe_pcm_chunk(
                    api_key,
                    base_url,
                    audio_model,
                    &b64,
                    "raw",
                    "请将这段语音转写为纯文本，只输出转写结果，不要添加任何解释或标点以外的内容。",
                )
                .await;
                let _ = result_tx.send(result);
                break;
            }
            QwenCmd::Cancel => {
                if let Some(bus) = &event_bus {
                    bus.publish(BusEvent::AsrSessionStatus {
                        session_id: session_id.to_string(),
                        status: "cancelled".to_string(),
                        error: None,
                    });
                }
                return Ok(());
            }
        }
    }
    Ok(())
}

/// 调用 Qwen-Omni 转写 base64 编码的 PCM 音频
async fn transcribe_pcm_chunk(
    api_key: &str,
    base_url: &str,
    audio_model: &str,
    audio_b64: &str,
    audio_format: &str,
    prompt: &str,
) -> Result<String, AsrError> {
    if api_key.is_empty() {
        return Err(AsrError::NotConfigured("千问 api_key 未配置".into()));
    }
    let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let data_url = format!("data:audio/{audio_format};base64,{audio_b64}");

    let body = serde_json::json!({
        "model": audio_model,
        "messages": [
            { "role": "system", "content": prompt },
            {
                "role": "user",
                "content": [
                    {
                        "type": "input_audio",
                        "input_audio": { "data": data_url }
                    }
                ]
            }
        ],
        "modalities": ["text"],
        "stream": true
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(AsrError::from_reqwest)?;

    let resp = client
        .post(&endpoint)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(AsrError::from_reqwest)?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(AsrError::Transcribe(format!(
            "Qwen HTTP {}: {}",
            status,
            truncate(&text, MAX_ERR_CHARS)
        )));
    }

    let text = resp.text().await.map_err(AsrError::from_reqwest)?;
    let transcript = parse_sse_stream(&text);
    if transcript.trim().is_empty() {
        return Err(AsrError::Transcribe("Qwen 返回空转写".into()));
    }
    Ok(transcript)
}

/// 解析 SSE 流式响应，累积 content 增量
///
/// SSE 格式：每行 `data: {json}`，`data: [DONE]` 表示结束。
/// JSON 中 `choices[0].delta.content` 为文本增量。
fn parse_sse_stream(text: &str) -> String {
    let mut result = String::with_capacity(text.len() / 4);
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("data:") {
            continue;
        }
        let data = line["data:".len()..].trim();
        if data == "[DONE]" || data.is_empty() {
            continue;
        }
        if let Ok(chunk) = serde_json::from_str::<SseChunk>(data) {
            if let Some(choice) = chunk.choices.first() {
                if let Some(content) = &choice.delta.content {
                    result.push_str(content);
                }
            }
        }
    }
    result
}

/// SSE 单块响应
#[derive(Deserialize)]
struct SseChunk {
    #[serde(default)]
    choices: Vec<SseChoice>,
}

#[derive(Deserialize)]
struct SseChoice {
    #[serde(default)]
    delta: SseDelta,
}

#[derive(Deserialize, Default)]
struct SseDelta {
    #[serde(default)]
    content: Option<String>,
}

/// 文件转写用 Qwen-Omni 请求结构（用于显式序列化，保持字段顺序清晰）
#[derive(Serialize)]
struct QwenTranscribeRequest {
    model: String,
    messages: Vec<QwenMessage>,
    modalities: Vec<String>,
    stream: bool,
}

#[derive(Serialize)]
struct QwenMessage {
    role: String,
    content: serde_json::Value,
}

/// 截断字符串到 max 字符
#[inline]
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push_str("...");
        out
    }
}

#[async_trait]
impl AsrProvider for QwenProvider {
    async fn start_streaming(
        &self,
        session_id: String,
        _lang: &str,
    ) -> Result<AudioStreamConfig, AsrError> {
        if self.api_key.is_empty() {
            return Err(AsrError::NotConfigured("千问 api_key 未配置".into()));
        }
        let (cmd_tx, cmd_rx) = mpsc::channel(AUDIO_CHANNEL_CAPACITY);
        {
            let mut sessions = self.sessions.lock().unwrap();
            if sessions.contains_key(&session_id) {
                return Err(AsrError::Protocol(format!("会话 {session_id} 已存在")));
            }
            sessions.insert(session_id.clone(), QwenSession { cmd_tx });
        }
        self.spawn_session_task(session_id, String::new(), cmd_rx);
        Ok(AudioStreamConfig::DEFAULT)
    }

    async fn push_audio_chunk(&self, session_id: &str, pcm: &[u8]) -> Result<(), AsrError> {
        let cmd_tx = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .get(session_id)
                .map(|s| s.cmd_tx.clone())
                .ok_or_else(|| AsrError::SessionNotFound(session_id.to_string()))?
        };
        cmd_tx
            .send(QwenCmd::Audio(pcm.to_vec()))
            .await
            .map_err(|_| AsrError::SessionFinished(session_id.to_string()))
    }

    async fn finish_streaming(&self, session_id: &str) -> Result<String, AsrError> {
        let session = self
            .take_session(session_id)
            .ok_or_else(|| AsrError::SessionNotFound(session_id.to_string()))?;
        let (tx, rx) = oneshot::channel();
        session
            .cmd_tx
            .send(QwenCmd::Finish(tx))
            .await
            .map_err(|_| AsrError::SessionFinished(session_id.to_string()))?;
        let result = rx
            .await
            .map_err(|_| AsrError::SessionFinished(session_id.to_string()))?;
        if let Some(bus) = &self.event_bus {
            bus.publish(BusEvent::AsrSessionStatus {
                session_id: session_id.to_string(),
                status: "completed".to_string(),
                error: None,
            });
        }
        result
    }

    async fn cancel_streaming(&self, session_id: &str) -> Result<(), AsrError> {
        if let Some(session) = self.take_session(session_id) {
            let _ = session.cmd_tx.send(QwenCmd::Cancel).await;
        }
        Ok(())
    }

    async fn transcribe_file(
        &self,
        audio_path: &Path,
        _lang: &str,
    ) -> Result<TranscribeResult, AsrError> {
        if self.api_key.is_empty() {
            return Err(AsrError::NotConfigured("千问 api_key 未配置".into()));
        }
        let audio_bytes = tokio::fs::read(audio_path)
            .await
            .map_err(AsrError::Io)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&audio_bytes);
        // 根据文件扩展名推断格式
        let format = audio_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("wav");

        let endpoint = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let data_url = format!("data:audio/{format};base64,{b64}");
        let body = QwenTranscribeRequest {
            model: self.audio_model.clone(),
            messages: vec![
                QwenMessage {
                    role: "system".into(),
                    content: serde_json::Value::String(
                        "请将这段语音转写为纯文本，只输出转写结果，不要添加任何解释。".into(),
                    ),
                },
                QwenMessage {
                    role: "user".into(),
                    content: serde_json::json!([{
                        "type": "input_audio",
                        "input_audio": { "data": data_url }
                    }]),
                },
            ],
            modalities: vec!["text".into()],
            stream: true,
        };

        let resp = self
            .http_client
            .post(&endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(AsrError::from_reqwest)?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AsrError::Transcribe(format!(
                "Qwen HTTP {}: {}",
                status,
                truncate(&text, MAX_ERR_CHARS)
            )));
        }

        let text = resp.text().await.map_err(AsrError::from_reqwest)?;
        let transcript = parse_sse_stream(&text);
        if transcript.trim().is_empty() {
            return Err(AsrError::Transcribe("Qwen 返回空转写".into()));
        }
        Ok(TranscribeResult {
            text: transcript,
            duration_ms: 0,
        })
    }
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sse_single_chunk() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\ndata: [DONE]\n";
        let result = parse_sse_stream(sse);
        assert_eq!(result, "你好");
    }

    #[test]
    fn parse_sse_multiple_chunks() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\", \"}}]}\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\"world!\"}}]}\n\
                   data: [DONE]\n";
        let result = parse_sse_stream(sse);
        assert_eq!(result, "Hello, world!");
    }

    #[test]
    fn parse_sse_skips_non_data_lines() {
        let sse = ": comment\n\
                   event: message\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\
                   data: [DONE]\n";
        let result = parse_sse_stream(sse);
        assert_eq!(result, "ok");
    }

    #[test]
    fn parse_sse_empty() {
        let result = parse_sse_stream("");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_sse_only_done() {
        let result = parse_sse_stream("data: [DONE]\n");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_sse_skips_invalid_json() {
        let sse = "data: not json\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\"valid\"}}]}\n";
        let result = parse_sse_stream(sse);
        assert_eq!(result, "valid");
    }

    #[test]
    fn parse_sse_no_content_field() {
        // delta 无 content 字段（如 role 块或 finish_reason 块）
        let sse = "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\"text\"},\"finish_reason\":null}]}\n";
        let result = parse_sse_stream(sse);
        assert_eq!(result, "text");
    }

    #[test]
    fn truncate_long_string() {
        let s = "x".repeat(10);
        assert_eq!(truncate(&s, 5), "xxxxx...");
    }

    #[test]
    fn sse_chunk_deserializes() {
        let json = r#"{"choices":[{"delta":{"content":"hi"},"finish_reason":null}]}"#;
        let chunk: SseChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.choices.len(), 1);
        assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hi"));
    }

    #[test]
    fn sse_chunk_empty_choices() {
        let json = r#"{"choices":[]}"#;
        let chunk: SseChunk = serde_json::from_str(json).unwrap();
        assert!(chunk.choices.is_empty());
    }
}

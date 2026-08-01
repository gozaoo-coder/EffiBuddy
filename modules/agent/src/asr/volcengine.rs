//! 火山引擎 ASR provider：流式 WebSocket + 文件极速版 HTTP
//!
//! # 流式（bigmodel_async）
//!
//! - WebSocket `wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async`
//! - 鉴权在 HTTP Header：X-Api-App-Key / X-Api-Access-Key / X-Api-Resource-Id /
//!   X-Api-Connect-Id(UUID)
//! - **二进制帧协议**：Header(4B) + PayloadSize(4B BE) + Payload(gzip)
//! - 音频 PCM 16k/16bit/mono，单帧 200ms = 6400 字节
//! - 结束信号 = 最后一包音频帧 flags = `END_OF_AUDIO`
//! - 每个 session 独立 task 持有 WebSocket sink，用 mpsc channel 接收音频帧
//!
//! # 文件极速版（auc_turbo）
//!
//! - HTTP POST `https://openspeech.bytedance.com/api/v3/auc/bigmodel/recognize/flash`
//! - Header 同上 + X-Api-Resource-Id: volc.bigasr.auc_turbo + X-Api-Request-Id(UUID)
//!   + X-Api-Sequence: -1
//! - Body JSON: { user, audio: { data: base64 }, request: { model_name: "bigmodel" } }
//! - 响应 Header X-Api-Status-Code: 20000000 = 成功
//!
//! # 并发与性能（对齐 user_rules）
//!
//! - `sessions` 用 `Arc<StdMutex<HashMap>>`：临界区极短（仅查表 clone sender）
//! - WebSocket 写入在独立 session task 中完成，锁外执行
//! - 用 mpsc channel 传递音频帧，不共享 `Arc<Mutex<Vec<u8>>>`
//! - 帧编解码为纯函数，无锁无 IO，可独立单测

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use base64::Engine;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, Mutex as TokioMutex};
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::http::Request;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

use effisuite_core::{EventBus, BusEvent};

use super::error::AsrError;
use super::provider::{AsrProvider, AudioStreamConfig, TranscribeResult};

// =========================================================
// 常量
// =========================================================

/// 流式 ASR WebSocket 端点
const STREAMING_ENDPOINT: &str = "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async";
/// 文件极速版 HTTP 端点
const FILE_ENDPOINT: &str = "https://openspeech.bytedance.com/api/v3/auc/bigmodel/recognize/flash";
/// 流式 resource id
const STREAMING_RESOURCE_ID: &str = "volc.bigasr.sauc.duration";
/// 文件极速版 resource id
const FILE_RESOURCE_ID: &str = "volc.bigasr.auc_turbo";
/// 文件转写成功状态码
const FILE_SUCCESS_CODE: u32 = 20000000;
/// HTTP 请求超时（文件转写，音频较大）
const FILE_TIMEOUT_SECS: u64 = 120;
/// session task 音频帧 channel 容量
const AUDIO_CHANNEL_CAPACITY: usize = 64;
/// 错误信息最大字符数
const MAX_ERR_CHARS: usize = 500;

// =========================================================
// 二进制帧协议（纯函数，可单测）
// =========================================================

/// 帧头第 0 字节高 4 位：message_type
///
/// 协议规范常量：部分常量（如 `FULL_SERVER_RESPONSE`）仅用于文档化完整协议，
/// 当前解析逻辑通过 `SERVER_ERROR_RESPONSE` 判定错误帧，非错误帧隐式为成功响应。
#[allow(dead_code)]
mod message_type {
    pub const FULL_CLIENT_REQUEST: u8 = 0b0001_0000;
    pub const AUDIO_ONLY_REQUEST: u8 = 0b0010_0000;
    pub const FULL_SERVER_RESPONSE: u8 = 0b1001_0000;
    pub const SERVER_ERROR_RESPONSE: u8 = 0b1011_0000;
    pub const MASK: u8 = 0xF0;
}

/// 帧头第 0 字节低 4 位：flags
///
/// 协议规范常量：`MASK` 用于文档化 flags 字段的位掩码，
/// 当前解析端不需要提取 flags（flags 仅用于客户端→服务器方向）。
#[allow(dead_code)]
mod flags {
    pub const NONE: u8 = 0b0000;
    /// 最后一包音频帧（结束信号）
    pub const END_OF_AUDIO: u8 = 0b0010;
    pub const MASK: u8 = 0x0F;
}

/// 帧头第 1 字节高 4 位：serialization
///
/// 协议规范常量：`MASK` 用于文档化 serialization 字段的位掩码，
/// 当前解析端不检查 serialization 类型（仅按 compression 决定是否解压）。
#[allow(dead_code)]
mod serialization {
    pub const JSON: u8 = 0b0001_0000;
    pub const NONE: u8 = 0b0000;
    pub const MASK: u8 = 0xF0;
}

/// 帧头第 1 字节低 4 位：compression
///
/// 协议规范常量：`NONE` 用于文档化无压缩选项，
/// 当前所有帧均使用 GZIP 压缩。
#[allow(dead_code)]
mod compression {
    pub const GZIP: u8 = 0b0001;
    pub const NONE: u8 = 0b0000;
    pub const MASK: u8 = 0x0F;
}

/// 帧头长度（4B）+ payload size 长度（4B BE）
const HEADER_LEN: usize = 4;
const PAYLOAD_SIZE_LEN: usize = 4;
const PREAMBLE_LEN: usize = HEADER_LEN + PAYLOAD_SIZE_LEN;

/// gzip 压缩字节
#[inline]
fn gzip_compress(data: &[u8]) -> Result<Vec<u8>, AsrError> {
    let mut encoder = GzEncoder::new(Vec::with_capacity(data.len() / 2), Compression::default());
    encoder.write_all(data).map_err(AsrError::Io)?;
    encoder.finish().map_err(AsrError::Io)
}

/// gzip 解压字节
#[inline]
fn gzip_decompress(data: &[u8]) -> Result<Vec<u8>, AsrError> {
    let mut decoder = GzDecoder::new(data);
    let mut out = Vec::with_capacity(data.len() * 4);
    decoder.read_to_end(&mut out).map_err(AsrError::Io)?;
    Ok(out)
}

/// 构造完整客户端请求帧（JSON config，gzip 压缩）
///
/// message_type = FULL_CLIENT_REQUEST, serialization = JSON, compression = GZIP
fn encode_full_client_request(payload: &[u8]) -> Result<Vec<u8>, AsrError> {
    let compressed = gzip_compress(payload)?;
    encode_frame(message_type::FULL_CLIENT_REQUEST, flags::NONE, serialization::JSON, compression::GZIP, &compressed)
}

/// 构造音频帧（原始 PCM，gzip 压缩）
///
/// `is_last` = true 时 flags = END_OF_AUDIO（结束信号）
fn encode_audio_frame(pcm: &[u8], is_last: bool) -> Result<Vec<u8>, AsrError> {
    let compressed = gzip_compress(pcm)?;
    let flags = if is_last { flags::END_OF_AUDIO } else { flags::NONE };
    // 音频帧无 serialization（raw bytes），但 gzip 压缩
    encode_frame(message_type::AUDIO_ONLY_REQUEST, flags, serialization::NONE, compression::GZIP, &compressed)
}

/// 拼接二进制帧：Header(4B) + PayloadSize(4B BE) + Payload
fn encode_frame(
    msg_type: u8,
    flags: u8,
    serialization: u8,
    compression: u8,
    payload: &[u8],
) -> Result<Vec<u8>, AsrError> {
    let payload_size = u32::try_from(payload.len())
        .map_err(|_| AsrError::Protocol("payload 超过 u32 范围".into()))?;
    let mut out = Vec::with_capacity(PREAMBLE_LEN + payload.len());
    out.push(msg_type | flags);
    out.push(serialization | compression);
    out.push(0); // reserved
    out.push(0); // reserved
    out.extend_from_slice(&payload_size.to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// 解析服务端响应帧
#[derive(Debug)]
struct ServerFrame {
    is_error: bool,
    payload: Vec<u8>,
}

/// 从二进制帧解析服务端响应
fn parse_response_frame(data: &[u8]) -> Result<ServerFrame, AsrError> {
    if data.len() < PREAMBLE_LEN {
        return Err(AsrError::Protocol(format!(
            "帧长度 {} < 最小 {}",
            data.len(),
            PREAMBLE_LEN
        )));
    }
    let msg_type = data[0] & message_type::MASK;
    let compression = data[1] & compression::MASK;
    let payload_size = u32::from_be_bytes([
        data[4],
        data[5],
        data[6],
        data[7],
    ]) as usize;
    if data.len() < PREAMBLE_LEN + payload_size {
        return Err(AsrError::Protocol(format!(
            "payload 截断：声明 {} 字节，实际 {} 字节",
            payload_size,
            data.len() - PREAMBLE_LEN
        )));
    }
    let raw_payload = &data[PREAMBLE_LEN..PREAMBLE_LEN + payload_size];
    // 解压（若 gzip）
    let payload = if compression == compression::GZIP && !raw_payload.is_empty() {
        gzip_decompress(raw_payload)?
    } else {
        raw_payload.to_vec()
    };
    Ok(ServerFrame {
        is_error: msg_type == message_type::SERVER_ERROR_RESPONSE,
        payload,
    })
}

// =========================================================
// JSON 请求/响应结构
// =========================================================

/// 流式客户端初始配置请求
#[derive(Serialize)]
struct FullClientRequest {
    user: UserPayload,
    audio: AudioPayload,
    request: StreamingRequestPayload,
}

#[derive(Serialize)]
struct UserPayload {
    uid: String,
}

#[derive(Serialize)]
struct AudioPayload {
    format: String,
    rate: u32,
    bits: u16,
    channel: u16,
    language: String,
}

#[derive(Serialize)]
struct StreamingRequestPayload {
    model_name: String,
    #[serde(rename = "enable_punc")]
    enable_punc: bool,
    #[serde(rename = "result_format")]
    result_format: String,
}

/// 流式服务端响应
#[derive(Deserialize)]
struct StreamingResponse {
    #[serde(default)]
    result: Option<StreamingResult>,
    #[serde(default)]
    code: Option<u32>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
struct StreamingResult {
    #[serde(default)]
    text: String,
    #[serde(default, rename = "utterances")]
    utterances: Vec<Utterance>,
    #[serde(default, rename = "definite")]
    definite: bool,
}

#[derive(Deserialize)]
struct Utterance {
    #[serde(default)]
    text: String,
}

/// 文件转写请求
#[derive(Serialize)]
struct FileTranscribeRequest {
    user: UserPayload,
    audio: FileAudioPayload,
    request: FileRequestPayload,
}

#[derive(Serialize)]
struct FileAudioPayload {
    data: String,
}

#[derive(Serialize)]
struct FileRequestPayload {
    model_name: String,
}

/// 文件转写响应
#[derive(Deserialize)]
struct FileTranscribeResponse {
    #[serde(default, rename = "result")]
    result: Option<FileResult>,
    /// API 返回的状态码（20000000 表示成功）。
    /// 当前逻辑通过 `result` 字段判定成功，`code` 仅反序列化保留供调试。
    #[serde(default, rename = "code")]
    #[allow(dead_code)]
    code: Option<u32>,
    #[serde(default, rename = "message")]
    message: Option<String>,
}

#[derive(Deserialize)]
struct FileResult {
    #[serde(default)]
    text: String,
    #[serde(default, rename = "duration_ms")]
    duration_ms: Option<u64>,
}

// =========================================================
// 流式 session 内部状态
// =========================================================

/// 发送给 session task 的命令
enum StreamingCmd {
    /// 推送一帧音频（PCM）
    Audio(Vec<u8>),
    /// 结束流式，携带结果回传通道
    Finish(oneshot::Sender<Result<String, AsrError>>),
    /// 取消流式
    Cancel,
}

/// 活跃流式会话条目
struct StreamingSession {
    cmd_tx: mpsc::Sender<StreamingCmd>,
}

// =========================================================
// VolcEngineProvider
// =========================================================

/// 火山引擎 ASR provider
///
/// 字段按大小降序：String(24) > Arc<...>(1 usize)
pub struct VolcEngineProvider {
    app_key: String,
    access_key: String,
    /// 流式模型名（bigmodel）
    streaming_model: String,
    /// 文件模型名（bigmodel）
    file_model: String,
    /// 活跃流式会话表：临界区极短（仅查表 clone sender）
    sessions: Arc<StdMutex<HashMap<String, StreamingSession>>>,
    /// 事件总线（None 时不推送流式 chunk 事件）
    event_bus: Option<Arc<EventBus>>,
    /// HTTP 客户端（文件转写，连接池化）
    http_client: reqwest::Client,
}

impl VolcEngineProvider {
    /// 构造 provider。app_key / access_key 来自 AsrConfig.volc_app_id / volc_access_token
    pub fn new(
        app_key: impl Into<String>,
        access_key: impl Into<String>,
        event_bus: Option<Arc<EventBus>>,
    ) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(FILE_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            app_key: app_key.into(),
            access_key: access_key.into(),
            streaming_model: "bigmodel".to_string(),
            file_model: "bigmodel".to_string(),
            sessions: Arc::new(StdMutex::new(HashMap::new())),
            event_bus,
            http_client,
        }
    }

    /// 从会话表中移除并返回 session（锁内仅 remove，无 IO）
    fn take_session(&self, session_id: &str) -> Option<StreamingSession> {
        self.sessions.lock().unwrap().remove(session_id)
    }

    /// 发布会话状态事件
    #[inline]
    fn publish_status(&self, session_id: &str, status: &str, error: Option<String>) {
        if let Some(bus) = &self.event_bus {
            bus.publish(BusEvent::AsrSessionStatus {
                session_id: session_id.to_string(),
                status: status.to_string(),
                error,
            });
        }
    }

    /// spawn 流式 session task：持有 WebSocket，接收音频帧并转发
    fn spawn_session_task(
        &self,
        session_id: String,
        lang: String,
        cmd_rx: mpsc::Receiver<StreamingCmd>,
    ) {
        let app_key = self.app_key.clone();
        let access_key = self.access_key.clone();
        let streaming_model = self.streaming_model.clone();
        let sessions = Arc::clone(&self.sessions);
        let event_bus = self.event_bus.clone();
        let session_id_for_task = session_id.clone();

        tokio::spawn(async move {
            let result = run_streaming_session(
                &app_key,
                &access_key,
                &streaming_model,
                &session_id_for_task,
                &lang,
                cmd_rx,
                event_bus.clone(),
            )
            .await;

            // 会话结束，从表中移除
            sessions.lock().unwrap().remove(&session_id_for_task);

            match &result {
                Ok(_) => debug!(session_id = %session_id_for_task, "ASR 流式会话正常结束"),
                Err(e) => {
                    warn!(session_id = %session_id_for_task, error = %e, "ASR 流式会话异常结束");
                    if let Some(bus) = event_bus.as_deref() {
                        bus.publish(BusEvent::AsrSessionStatus {
                            session_id: session_id_for_task.clone(),
                            status: "failed".to_string(),
                            error: Some(e.to_string()),
                        });
                    }
                }
            }
        });
    }
}

/// 运行单个流式会话：建立 WebSocket → 发初始配置 → 收音频帧转发 → 收响应累积转写
///
/// 独立 async 函数（非方法），避免 `&self` 借用跨 await 阻碍 task spawn。
async fn run_streaming_session(
    app_key: &str,
    access_key: &str,
    streaming_model: &str,
    session_id: &str,
    lang: &str,
    mut cmd_rx: mpsc::Receiver<StreamingCmd>,
    event_bus: Option<Arc<EventBus>>,
) -> Result<(), AsrError> {
    // 1. 构造握手请求
    let connect_id = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("GET")
        .uri(STREAMING_ENDPOINT)
        .header("Host", "openspeech.bytedance.com")
        .header("Upgrade", "websocket")
        .header("Connection", "Upgrade")
        .header("Sec-WebSocket-Key", generate_key())
        .header("Sec-WebSocket-Version", "13")
        .header("X-Api-App-Key", app_key)
        .header("X-Api-Access-Key", access_key)
        .header("X-Api-Resource-Id", STREAMING_RESOURCE_ID)
        .header("X-Api-Connect-Id", &connect_id)
        .body(())
        .map_err(|e| AsrError::Protocol(format!("构造 WebSocket 请求失败: {e}")))?;

    // 2. 建立 WebSocket 连接（native-tls）
    let (ws_stream, _response) =
        tokio_tungstenite::connect_async_tls_with_config(req, None, false, None)
            .await
            .map_err(|e| AsrError::Network(format!("WebSocket 连接失败: {e}")))?;

    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    // 3. 发送初始配置（full client request）
    let config_payload = serde_json::to_vec(&FullClientRequest {
        user: UserPayload {
            uid: session_id.to_string(),
        },
        audio: AudioPayload {
            format: "raw".to_string(),
            rate: 16000,
            bits: 16,
            channel: 1,
            language: lang.to_string(),
        },
        request: StreamingRequestPayload {
            model_name: streaming_model.to_string(),
            enable_punc: true,
            result_format: "full".to_string(),
        },
    })
    .map_err(AsrError::Json)?;
    let frame = encode_full_client_request(&config_payload)?;
    ws_sink
        .send(Message::Binary(frame))
        .await
        .map_err(|e| AsrError::Network(format!("发送初始配置失败: {e}")))?;

    if let Some(bus) = &event_bus {
        bus.publish(BusEvent::AsrSessionStatus {
            session_id: session_id.to_string(),
            status: "transcribing".to_string(),
            error: None,
        });
    }

    // 4. 累积转写文本（Arc<TokioMutex> 跨 task 共享）
    let transcript: Arc<TokioMutex<String>> = Arc::new(TokioMutex::new(String::new()));

    // spawn 响应读取 task：持续读 WebSocket 响应，解析文本，累积到 transcript
    // event_bus 需克隆 Arc 移入 spawned task（task 要求 'static）
    let transcript_clone = Arc::clone(&transcript);
    let session_id_clone = session_id.to_string();
    let resp_event_bus = event_bus.clone();
    let mut resp_task = Some(tokio::spawn(async move {
        let mut last_text = String::new();
        while let Some(msg) = ws_stream.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(e) => {
                    warn!(session_id = %session_id_clone, error = %e, "读取 WS 响应失败");
                    break;
                }
            };
            let data = match msg {
                Message::Binary(b) => b,
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Close(_) => break,
                _ => continue,
            };
            let frame = match parse_response_frame(&data) {
                Ok(f) => f,
                Err(e) => {
                    warn!(session_id = %session_id_clone, error = %e, "解析响应帧失败");
                    continue;
                }
            };
            if frame.is_error {
                if let Ok(resp) = serde_json::from_slice::<StreamingResponse>(&frame.payload) {
                    warn!(
                        session_id = %session_id_clone,
                        code = ?resp.code,
                        msg = ?resp.message,
                        "服务端返回错误"
                    );
                }
                break;
            }
            // 解析响应 payload
            if let Ok(resp) = serde_json::from_slice::<StreamingResponse>(&frame.payload) {
                if let Some(result) = &resp.result {
                    // 优先用 utterances 累积，否则用顶层 text
                    let current_text: String = if result.utterances.is_empty() {
                        result.text.clone()
                    } else {
                        result
                            .utterances
                            .iter()
                            .map(|u| u.text.as_str())
                            .collect::<Vec<_>>()
                            .join("")
                    };
                    // 增量推送（只发新增部分）
                    if current_text.len() > last_text.len() {
                        let delta = &current_text[last_text.len()..];
                        if let Some(bus) = &resp_event_bus {
                            bus.publish(BusEvent::AsrStreamChunk {
                                session_id: session_id_clone.clone(),
                                text: delta.to_string(),
                                is_final: result.definite,
                            });
                        }
                    }
                    if result.definite {
                        // definite=true 表示确定结果，更新累积文本
                        last_text = current_text.clone();
                        let mut t = transcript_clone.lock().await;
                        t.clear();
                        t.push_str(&last_text);
                    }
                }
            }
        }
    }));

    // 5. 主循环：接收音频帧命令，转发到 WebSocket
    let mut finished = false;
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            StreamingCmd::Audio(pcm) => {
                let frame = encode_audio_frame(&pcm, false)?;
                if ws_sink
                    .send(Message::Binary(frame))
                    .await
                    .is_err()
                {
                    // WebSocket 已断开，终止会话
                    break;
                }
            }
            StreamingCmd::Finish(result_tx) => {
                // 发送结束信号帧（空音频 + END_OF_AUDIO flag）
                let end_frame = encode_audio_frame(&[], true)?;
                let _ = ws_sink.send(Message::Binary(end_frame)).await;
                let _ = ws_sink.close().await;
                // 等待响应读取 task 完成（确保最终文本已累积）
                if let Some(task) = resp_task.take() {
                    let _ = task.await;
                }
                let transcript_val = transcript.lock().await.clone();
                // 转移所有权到 oneshot，无需 clone（AsrError 未实现 Clone）
                let _ = result_tx.send(Ok(transcript_val));
                finished = true;
                break;
            }
            StreamingCmd::Cancel => {
                let _ = ws_sink.close().await;
                if let Some(task) = resp_task.take() {
                    task.abort();
                }
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

    // 若循环自然结束（channel 关闭）且未发 finish，终止响应读取 task
    if !finished {
        if let Some(task) = resp_task.take() {
            task.abort();
        }
    }

    Ok(())
}

#[async_trait]
impl AsrProvider for VolcEngineProvider {
    async fn start_streaming(
        &self,
        session_id: String,
        lang: &str,
    ) -> Result<AudioStreamConfig, AsrError> {
        if self.app_key.is_empty() || self.access_key.is_empty() {
            return Err(AsrError::NotConfigured(
                "火山引擎 app_key / access_key 未配置".into(),
            ));
        }
        let (cmd_tx, cmd_rx) = mpsc::channel(AUDIO_CHANNEL_CAPACITY);
        // 先注册 session（锁内仅 insert，无 IO）
        {
            let mut sessions = self.sessions.lock().unwrap();
            if sessions.contains_key(&session_id) {
                return Err(AsrError::Protocol(format!(
                    "会话 {session_id} 已存在"
                )));
            }
            sessions.insert(session_id.clone(), StreamingSession { cmd_tx });
        }
        self.spawn_session_task(session_id.clone(), lang.to_string(), cmd_rx);
        Ok(AudioStreamConfig::DEFAULT)
    }

    async fn push_audio_chunk(&self, session_id: &str, pcm: &[u8]) -> Result<(), AsrError> {
        // 锁内仅查表 clone sender（极短临界区），IO 在锁外
        let cmd_tx = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .get(session_id)
                .map(|s| s.cmd_tx.clone())
                .ok_or_else(|| AsrError::SessionNotFound(session_id.to_string()))?
        };
        cmd_tx
            .send(StreamingCmd::Audio(pcm.to_vec()))
            .await
            .map_err(|_| AsrError::SessionFinished(session_id.to_string()))
    }

    async fn finish_streaming(&self, session_id: &str) -> Result<String, AsrError> {
        // 锁内取 session（移除），锁外等待结果
        let session = self
            .take_session(session_id)
            .ok_or_else(|| AsrError::SessionNotFound(session_id.to_string()))?;
        let (tx, rx) = oneshot::channel();
        session
            .cmd_tx
            .send(StreamingCmd::Finish(tx))
            .await
            .map_err(|_| AsrError::SessionFinished(session_id.to_string()))?;
        let result = rx
            .await
            .map_err(|_| AsrError::SessionFinished(session_id.to_string()))?;
        self.publish_status(session_id, "completed", None);
        result
    }

    async fn cancel_streaming(&self, session_id: &str) -> Result<(), AsrError> {
        if let Some(session) = self.take_session(session_id) {
            let _ = session.cmd_tx.send(StreamingCmd::Cancel).await;
        }
        // 幂等：不存在的 session 视为成功
        Ok(())
    }

    async fn transcribe_file(
        &self,
        audio_path: &Path,
        lang: &str,
    ) -> Result<TranscribeResult, AsrError> {
        if self.app_key.is_empty() || self.access_key.is_empty() {
            return Err(AsrError::NotConfigured(
                "火山引擎 app_key / access_key 未配置".into(),
            ));
        }

        // 读取音频文件并 base64 编码
        let audio_bytes = tokio::fs::read(audio_path)
            .await
            .map_err(|e| AsrError::Io(e))?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&audio_bytes);

        let request_id = uuid::Uuid::new_v4().to_string();
        let body = FileTranscribeRequest {
            user: UserPayload {
                uid: "effisuite".to_string(),
            },
            audio: FileAudioPayload { data: b64 },
            request: FileRequestPayload {
                model_name: self.file_model.clone(),
            },
        };

        let resp = self
            .http_client
            .post(FILE_ENDPOINT)
            .header("X-Api-App-Key", &self.app_key)
            .header("X-Api-Access-Key", &self.access_key)
            .header("X-Api-Resource-Id", FILE_RESOURCE_ID)
            .header("X-Api-Request-Id", &request_id)
            .header("X-Api-Sequence", "-1")
            .json(&body)
            .send()
            .await
            .map_err(AsrError::from_reqwest)?;

        // 检查响应 Header 中的状态码
        let status_code = resp
            .headers()
            .get("X-Api-Status-Code")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AsrError::Transcribe(format!(
                "HTTP {}: {}",
                status,
                truncate(&text, MAX_ERR_CHARS)
            )));
        }

        let resp_data: FileTranscribeResponse = resp
            .json()
            .await
            .map_err(AsrError::from_reqwest)?;

        if status_code != 0 && status_code != FILE_SUCCESS_CODE {
            return Err(AsrError::Transcribe(format!(
                "火山引擎返回错误码 {}：{}",
                status_code,
                resp_data.message.unwrap_or_default()
            )));
        }

        let result = resp_data
            .result
            .ok_or_else(|| AsrError::Transcribe("响应无 result 字段".into()))?;
        let _ = lang; // 文件转写语言由 model 决定，此处忽略
        Ok(TranscribeResult {
            text: result.text,
            duration_ms: result.duration_ms.unwrap_or(0),
        })
    }
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

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gzip_roundtrip() {
        let data = b"hello world, ASR streaming audio frame payload";
        let compressed = gzip_compress(data).unwrap();
        let decompressed = gzip_decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn encode_full_client_request_has_correct_header() {
        let payload = br#"{"user":{"uid":"test"}}"#;
        let frame = encode_full_client_request(payload).unwrap();
        // 帧总长度 == preamble + payload_size 字段声明的字节数
        let payload_size = u32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]) as usize;
        assert_eq!(frame.len(), PREAMBLE_LEN + payload_size);
        // message_type = FULL_CLIENT_REQUEST, flags = NONE
        assert_eq!(frame[0] & message_type::MASK, message_type::FULL_CLIENT_REQUEST);
        assert_eq!(frame[0] & flags::MASK, flags::NONE);
        // serialization = JSON, compression = GZIP
        assert_eq!(frame[1] & serialization::MASK, serialization::JSON);
        assert_eq!(frame[1] & compression::MASK, compression::GZIP);
    }

    #[test]
    fn encode_audio_frame_intermediate_flags() {
        let pcm = vec![0u8; 6400];
        let frame = encode_audio_frame(&pcm, false).unwrap();
        assert_eq!(frame[0] & message_type::MASK, message_type::AUDIO_ONLY_REQUEST);
        assert_eq!(frame[0] & flags::MASK, flags::NONE);
        assert_eq!(frame[1] & compression::MASK, compression::GZIP);
    }

    #[test]
    fn encode_audio_frame_end_flags() {
        let pcm = vec![0u8; 6400];
        let frame = encode_audio_frame(&pcm, true).unwrap();
        assert_eq!(frame[0] & message_type::MASK, message_type::AUDIO_ONLY_REQUEST);
        assert_eq!(frame[0] & flags::MASK, flags::END_OF_AUDIO);
    }

    #[test]
    fn encode_frame_payload_size_big_endian() {
        let payload = b"test payload data";
        let frame = encode_frame(
            message_type::AUDIO_ONLY_REQUEST,
            flags::NONE,
            serialization::NONE,
            compression::GZIP,
            payload,
        )
        .unwrap();
        let size = u32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]) as usize;
        assert_eq!(size, payload.len());
    }

    #[test]
    fn parse_response_frame_full_server_response() {
        // 构造一个 FULL_SERVER_RESPONSE 帧（gzip 压缩 JSON payload）
        // 注意：byte string literal 不允许非 ASCII，用 .as_bytes() 代替 br#"..."#
        let payload = r#"{"result":{"text":"你好","definite":true}}"#.as_bytes();
        let compressed = gzip_compress(payload).unwrap();
        let frame = encode_frame(
            message_type::FULL_SERVER_RESPONSE,
            flags::NONE,
            serialization::JSON,
            compression::GZIP,
            &compressed,
        )
        .unwrap();
        let parsed = parse_response_frame(&frame).unwrap();
        assert!(!parsed.is_error);
        assert_eq!(parsed.payload, payload);
    }

    #[test]
    fn parse_response_frame_server_error() {
        let payload = br#"{"code":1001,"message":"auth failed"}"#;
        let compressed = gzip_compress(payload).unwrap();
        let frame = encode_frame(
            message_type::SERVER_ERROR_RESPONSE,
            flags::NONE,
            serialization::JSON,
            compression::GZIP,
            &compressed,
        )
        .unwrap();
        let parsed = parse_response_frame(&frame).unwrap();
        assert!(parsed.is_error);
        assert_eq!(parsed.payload, payload);
    }

    #[test]
    fn parse_response_frame_too_short() {
        let result = parse_response_frame(&[0, 0, 0]);
        assert!(matches!(result, Err(AsrError::Protocol(_))));
    }

    #[test]
    fn parse_response_frame_truncated_payload() {
        // 声明 payload_size=100 但实际只有 10 字节
        let mut frame = vec![0u8; PREAMBLE_LEN + 10];
        frame[0] = message_type::FULL_SERVER_RESPONSE;
        frame[4..8].copy_from_slice(&100u32.to_be_bytes());
        let result = parse_response_frame(&frame);
        assert!(matches!(result, Err(AsrError::Protocol(_))));
    }

    #[test]
    fn parse_response_frame_uncompressed() {
        let payload = br#"{"result":{"text":"hi"}}"#;
        let frame = encode_frame(
            message_type::FULL_SERVER_RESPONSE,
            flags::NONE,
            serialization::JSON,
            compression::NONE,
            payload,
        )
        .unwrap();
        let parsed = parse_response_frame(&frame).unwrap();
        assert_eq!(parsed.payload, payload);
    }

    #[test]
    fn streaming_response_parses() {
        let json = r#"{"result":{"text":"你好世界","utterances":[{"text":"你好","definite":true}],"definite":true}}"#;
        let resp: StreamingResponse = serde_json::from_str(json).unwrap();
        assert!(resp.result.is_some());
        let result = resp.result.unwrap();
        assert_eq!(result.text, "你好世界");
        assert_eq!(result.utterances.len(), 1);
        assert!(result.definite);
    }

    #[test]
    fn streaming_response_empty() {
        let json = r#"{}"#;
        let resp: StreamingResponse = serde_json::from_str(json).unwrap();
        assert!(resp.result.is_none());
        assert!(resp.code.is_none());
    }

    #[test]
    fn file_transcribe_response_parses() {
        let json = r#"{"result":{"text":"转写文本","duration_ms":5000},"code":20000000,"message":"success"}"#;
        let resp: FileTranscribeResponse = serde_json::from_str(json).unwrap();
        assert!(resp.result.is_some());
        let result = resp.result.unwrap();
        assert_eq!(result.text, "转写文本");
        assert_eq!(result.duration_ms, Some(5000));
        assert_eq!(resp.code, Some(FILE_SUCCESS_CODE));
    }

    #[test]
    fn file_transcribe_response_error() {
        let json = r#"{"result":null,"code":1001,"message":"audio too long"}"#;
        let resp: FileTranscribeResponse = serde_json::from_str(json).unwrap();
        assert!(resp.result.is_none());
        assert_eq!(resp.code, Some(1001));
    }

    #[test]
    fn truncate_long_string() {
        let s = "x".repeat(10);
        assert_eq!(truncate(&s, 5), "xxxxx...");
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }
}

//! ASR（语音转写）Tauri 命令模块。
//!
//! 暴露给前端的 ASR 能力：
//! - 流式录音：start / push_audio / finish / cancel
//! - 文件转写：transcribe_file
//! - 记录管理：list / get / search / delete / update
//! - 摘要 RAG：search_summaries
//! - 会话管理：list_sessions
//! - 配置管理：get_config / update_config
//! - 摘要生成：generate_summary（对已转写记录补充摘要）
//!
//! # 设计要点
//!
//! - 命令层薄封装：clone Arc 句柄后 await，不跨 await 持有 State 借用
//! - 错误统一转 `String`（Tauri 命令要求 `Result<T, String>`）
//! - 流式 push_audio 接收 base64 编码的 PCM 帧，解码后转发给 AsrService

use std::sync::Arc;

use base64::Engine;
use effisuite_agent::{generate_summary, SessionInfo};
use effisuite_core::{
    AsrConfig, AsrRecord, AsrSearchQuery, AsrSource, AsrStatus, AsrSummaryHit,
};

use crate::state::AppState;

// =========================================================
// 可序列化响应类型（AsrService 内部类型不全部实现 Serialize）
// =========================================================

/// finish_streaming / transcribe_file 的返回结果
#[derive(Debug, serde::Serialize)]
pub struct AsrFinishResult {
    pub transcript: String,
    pub record_id: Option<String>,
    pub summary: Option<String>,
}

/// 活跃流式会话信息（SessionInfo 的可序列化投影）
#[derive(Debug, serde::Serialize)]
pub struct AsrSessionInfo {
    pub session_id: String,
    pub record_id: Option<String>,
    pub language: String,
    pub state: String,
}

impl From<SessionInfo> for AsrSessionInfo {
    #[inline]
    fn from(info: SessionInfo) -> Self {
        Self {
            session_id: info.session_id,
            record_id: info.record_id,
            language: info.language,
            state: format!("{:?}", info.state).to_lowercase(),
        }
    }
}

// =========================================================
// 流式录音命令
// =========================================================

/// 启动流式 ASR 会话，返回 session_id
#[tauri::command]
pub(crate) async fn asr_start_streaming(
    state: tauri::State<'_, AppState>,
    lang: Option<String>,
) -> Result<String, String> {
    let svc = Arc::clone(&state.asr_service);
    let lang = lang
        .as_deref()
        .unwrap_or("")
        .trim();
    let lang = if lang.is_empty() {
        svc.config().await.default_language
    } else {
        lang.to_string()
    };
    svc.start_streaming(&lang)
        .await
        .map_err(|e| e.to_string())
}

/// 推送一帧 base64 编码的 PCM 音频到活跃会话
#[tauri::command]
pub(crate) async fn asr_push_audio(
    state: tauri::State<'_, AppState>,
    session_id: String,
    audio_base64: String,
) -> Result<(), String> {
    let svc = Arc::clone(&state.asr_service);
    let pcm = base64::engine::general_purpose::STANDARD
        .decode(audio_base64.as_bytes())
        .map_err(|e| format!("base64 解码失败: {e}"))?;
    svc.push_audio_chunk(&session_id, &pcm)
        .await
        .map_err(|e| e.to_string())
}

/// 结束流式转写，返回完整转写文本与摘要（若启用自动摘要）
#[tauri::command]
pub(crate) async fn asr_finish_streaming(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<AsrFinishResult, String> {
    let svc = Arc::clone(&state.asr_service);
    // 读出 agent 句柄（短暂读锁），传入 ASR 服务做自动摘要
    let agent = state.agent.read().await.clone();
    let result = svc
        .finish_streaming(&session_id, Some(agent.as_ref()))
        .await
        .map_err(|e| e.to_string())?;
    Ok(AsrFinishResult {
        transcript: result.transcript,
        record_id: result.record_id,
        summary: result.summary,
    })
}

/// 取消流式会话（幂等）
#[tauri::command]
pub(crate) async fn asr_cancel_streaming(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    let svc = Arc::clone(&state.asr_service);
    svc.cancel_streaming(&session_id)
        .await
        .map_err(|e| e.to_string())
}

// =========================================================
// 文件转写命令
// =========================================================

/// 转写本地音频文件（一次性，非流式）
#[tauri::command]
pub(crate) async fn asr_transcribe_file(
    state: tauri::State<'_, AppState>,
    audio_path: String,
    lang: Option<String>,
) -> Result<AsrFinishResult, String> {
    let svc = Arc::clone(&state.asr_service);
    let path = std::path::Path::new(&audio_path);
    if !path.exists() {
        return Err(format!("音频文件不存在: {audio_path}"));
    }
    let lang = lang.as_deref().unwrap_or("").trim();
    let lang = if lang.is_empty() {
        svc.config().await.default_language
    } else {
        lang.to_string()
    };
    let agent = state.agent.read().await.clone();
    let result = svc
        .transcribe_file(path, &lang, Some(agent.as_ref()))
        .await
        .map_err(|e| e.to_string())?;
    Ok(AsrFinishResult {
        transcript: result.transcript,
        record_id: result.record_id,
        summary: result.summary,
    })
}

// =========================================================
// 记录管理命令
// =========================================================

/// 列出所有 ASR 记录元数据（不含 transcript）
#[tauri::command]
pub(crate) async fn asr_list_records(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AsrRecord>, String> {
    let svc = Arc::clone(&state.asr_service);
    svc.list_records().await.map_err(|e| e.to_string())
}

/// 获取单条 ASR 记录（含完整 transcript）
#[tauri::command]
pub(crate) async fn asr_get_record(
    state: tauri::State<'_, AppState>,
    record_id: String,
) -> Result<Option<AsrRecord>, String> {
    let svc = Arc::clone(&state.asr_service);
    svc.get_record(&record_id).await.map_err(|e| e.to_string())
}

/// 搜索 ASR 记录（按关键词/来源/状态/时间范围过滤）
#[tauri::command]
pub(crate) async fn asr_search_records(
    state: tauri::State<'_, AppState>,
    keyword: Option<String>,
    limit: Option<usize>,
    source: Option<String>,
    status: Option<String>,
) -> Result<Vec<AsrRecord>, String> {
    let svc = Arc::clone(&state.asr_service);
    let query = AsrSearchQuery {
        keyword,
        limit: limit.unwrap_or(50).min(200),
        source: source.as_deref().and_then(parse_source),
        status: status.as_deref().and_then(parse_status),
        ..Default::default()
    };
    svc.search_records(&query)
        .await
        .map_err(|e| e.to_string())
}

/// 删除 ASR 记录（同时从 RAG 索引移除）
#[tauri::command]
pub(crate) async fn asr_delete_record(
    state: tauri::State<'_, AppState>,
    record_id: String,
) -> Result<(), String> {
    let svc = Arc::clone(&state.asr_service);
    svc.delete_record(&record_id)
        .await
        .map_err(|e| e.to_string())
}

/// 更新 ASR 记录（标题/标签/摘要），返回更新后的记录
#[tauri::command]
pub(crate) async fn asr_update_record(
    state: tauri::State<'_, AppState>,
    record_id: String,
    title: Option<String>,
    tags: Option<Vec<String>>,
    summary: Option<String>,
) -> Result<AsrRecord, String> {
    let svc = Arc::clone(&state.asr_service);
    let mut record = svc
        .get_record(&record_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("记录 {record_id} 不存在"))?;
    if let Some(t) = title {
        record.title = t;
    }
    if let Some(tg) = tags {
        record.tags = tg;
    }
    if let Some(s) = summary {
        record.summary = Some(s);
        record.status = AsrStatus::Completed;
    }
    record.updated_at = chrono::Utc::now();
    let updated = record.clone();
    svc.update_record(record)
        .await
        .map_err(|e| e.to_string())?;
    Ok(updated)
}

// =========================================================
// 摘要 RAG 检索命令
// =========================================================

/// 搜索 ASR 摘要 RAG 索引（BM25 词法检索）
#[tauri::command]
pub(crate) async fn asr_search_summaries(
    state: tauri::State<'_, AppState>,
    keyword: String,
    limit: Option<usize>,
) -> Result<Vec<AsrSummaryHit>, String> {
    let svc = Arc::clone(&state.asr_service);
    let limit = limit.unwrap_or(10).min(50);
    svc.search_summaries(&keyword, limit)
        .await
        .map_err(|e| e.to_string())
}

// =========================================================
// 会话管理命令
// =========================================================

/// 列出当前活跃的流式 ASR 会话
#[tauri::command]
pub(crate) async fn asr_list_sessions(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AsrSessionInfo>, String> {
    let svc = Arc::clone(&state.asr_service);
    Ok(svc.list_active_sessions().into_iter().map(Into::into).collect())
}

// =========================================================
// 配置管理命令
// =========================================================

/// 获取当前 ASR 配置快照
#[tauri::command]
pub(crate) async fn asr_get_config(
    state: tauri::State<'_, AppState>,
) -> Result<AsrConfig, String> {
    let svc = Arc::clone(&state.asr_service);
    Ok(svc.config().await)
}

/// 更新 ASR 配置（热更新自动摘要开关；provider 切换需重启生效）
#[tauri::command]
pub(crate) async fn asr_update_config(
    state: tauri::State<'_, AppState>,
    config: AsrConfig,
) -> Result<(), String> {
    let svc = Arc::clone(&state.asr_service);
    svc.update_config(config).await;
    Ok(())
}

// =========================================================
// 摘要生成命令
// =========================================================

/// 对已转写的记录生成结构化摘要（调用当前激活的对话模型）
#[tauri::command]
pub(crate) async fn asr_generate_summary(
    state: tauri::State<'_, AppState>,
    record_id: String,
) -> Result<Option<String>, String> {
    let svc = Arc::clone(&state.asr_service);
    let record = svc
        .get_record(&record_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("记录 {record_id} 不存在"))?;
    if record.transcript.trim().is_empty() {
        return Err("转写文本为空，无法生成摘要".into());
    }
    let agent = state.agent.read().await.clone();
    let summary = generate_summary(agent.as_ref(), &record.transcript, None)
        .await
        .map_err(|e| e.to_string())?;
    // 更新记录的摘要与状态
    let mut updated = record;
    updated.summary = Some(summary.clone());
    updated.status = AsrStatus::Completed;
    updated.updated_at = chrono::Utc::now();
    svc.update_record(updated)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some(summary))
}

// =========================================================
// 辅助函数
// =========================================================

/// 解析来源字符串为 AsrSource 枚举
#[inline]
fn parse_source(s: &str) -> Option<AsrSource> {
    match s.to_ascii_lowercase().as_str() {
        "streaming" => Some(AsrSource::Streaming),
        "upload" => Some(AsrSource::Upload),
        _ => None,
    }
}

/// 解析状态字符串为 AsrStatus 枚举
#[inline]
fn parse_status(s: &str) -> Option<AsrStatus> {
    match s.to_ascii_lowercase().as_str() {
        "pending" => Some(AsrStatus::Pending),
        "transcribing" => Some(AsrStatus::Transcribing),
        "transcribed" => Some(AsrStatus::Transcribed),
        "summarizing" => Some(AsrStatus::Summarizing),
        "completed" => Some(AsrStatus::Completed),
        "failed" => Some(AsrStatus::Failed),
        _ => None,
    }
}

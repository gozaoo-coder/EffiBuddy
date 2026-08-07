//! asr 工具：让 LLM 调用 ASR 能力
//!
//! 暴露给 LLM 的能力：
//! - `transcribe_audio`：转写本地音频文件，返回完整转写文本与摘要
//! - `search_asr_records`：按关键词搜索已转写的 ASR 记录
//! - `list_asr_records`：列出最近的 ASR 转写记录
//! - `get_asr_record`：获取指定 ASR 记录的完整转写文本
//!
//! 流式录音 API（start_streaming / push_audio_chunk / finish_streaming）
//! 不作为 LLM 工具暴露（LLM 无法实时推送音频帧），由 Tauri 命令层直接暴露给前端。
//!
//! # 性能要点
//!
//! - 工具持有 `Arc<AsrService>`，clone 廉价
//! - 转写调用走 `AsrService::transcribe_file`，内部异步 IO
//! - 搜索结果用 `with_capacity` 预分配
//! - 不在锁内执行 IO

use std::borrow::Cow;
use std::sync::Arc;

use effisuite_core::{AsrSearchQuery, AsrSource, AsrStatus};
use rig_core::tool::Tool;
use serde::Deserialize;
use serde_json::json;

use crate::asr::{AsrError, AsrService};

// =========================================================
// transcribe_audio 工具
// =========================================================

/// transcribe_audio 工具参数
///
/// 字段按大小降序：String(24) > Option<String>(24)。
#[derive(Deserialize)]
pub struct TranscribeAudioArgs {
    /// 音频文件绝对路径（支持 wav/mp3/m4a/flac 等格式，取决于 provider）
    pub audio_path: String,
    /// 语言代码（如 zh-CN / en-US）。留空用服务默认语言
    #[serde(default)]
    pub lang: Option<String>,
}

/// transcribe_audio 工具错误
#[derive(Debug, thiserror::Error)]
#[error("transcribe_audio error: {0}")]
pub struct TranscribeAudioError(String);

impl From<AsrError> for TranscribeAudioError {
    #[inline]
    fn from(e: AsrError) -> Self {
        Self(e.to_string())
    }
}

/// transcribe_audio 工具输出
///
/// 字段按大小降序：String(24) > Option<String>(24)。
#[derive(Debug, serde::Serialize)]
pub struct TranscribeAudioOutput {
    /// 完整转写文本
    pub transcript: String,
    /// AI 生成的结构化摘要（Markdown 格式）
    pub summary: Option<String>,
    /// 持久化后的记录 id（可用于后续 search_asr_records / get_asr_record）
    pub record_id: Option<String>,
    /// 音频时长（毫秒）
    pub duration_ms: u64,
}

/// 转写音频文件工具
///
/// 让 LLM 在用户要求"转写这段录音"、"听一下这个音频"时调用。
/// 工具调用 AsrService 完成转写 + 自动摘要 + 持久化。
pub struct AsrTool {
    service: Arc<AsrService>,
}

impl AsrTool {
    pub fn new(service: Arc<AsrService>) -> Self {
        Self { service }
    }
}

impl Tool for AsrTool {
    const NAME: &'static str = "transcribe_audio";

    type Error = TranscribeAudioError;
    type Args = TranscribeAudioArgs;
    type Output = TranscribeAudioOutput;

  fn description(&self) -> String {
      "转写本地音频文件为文本并生成结构化摘要（支持 wav/mp3/m4a/flac 等）。\
       结果持久化，可用 search_asr_records 检索。"
          .to_string()
  }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "audio_path": {
                    "type": "string",
                      "description": "音频文件绝对路径（wav/mp3/m4a/flac 等）"
                },
                "lang": {
                    "type": "string",
                      "description": "语言代码（zh-CN/en-US 等）",
                    "default": ""
                }
            },
            "required": ["audio_path"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let path = std::path::Path::new(&args.audio_path);
        if !path.exists() {
            return Err(TranscribeAudioError(format!(
                "音频文件不存在: {}",
                args.audio_path
            )));
        }
        let lang = args.lang.as_deref().unwrap_or("").trim();
        // Cow 避免无谓分配：显式指定语言时借用，用默认时才拥有
        let lang: Cow<'_, str> = if lang.is_empty() {
            Cow::Owned(self.service.config().await.default_language)
        } else {
            Cow::Borrowed(lang)
        };
        // 工具调用时不传 agent（避免循环调用），摘要由 Tauri 命令层单独触发
        let result = self.service.transcribe_file(path, &lang, None).await?;
        Ok(TranscribeAudioOutput {
            transcript: result.transcript,
            summary: result.summary,
            record_id: result.record_id,
            duration_ms: 0,
        })
    }
}

// =========================================================
// search_asr_records 工具
// =========================================================

/// search_asr_records 工具参数
#[derive(Deserialize)]
pub struct SearchAsrArgs {
    /// 搜索关键词（匹配标题/转写文本/摘要/标签）
    #[serde(default)]
    pub keyword: Option<String>,
    /// 返回条数上限，默认 10
    #[serde(default)]
    pub limit: Option<usize>,
}

/// search_asr_records 工具错误
#[derive(Debug, thiserror::Error)]
#[error("search_asr_records error: {0}")]
pub struct SearchAsrError(String);

impl From<AsrError> for SearchAsrError {
    #[inline]
    fn from(e: AsrError) -> Self {
        Self(e.to_string())
    }
}

/// 单条 ASR 记录摘要（给 LLM 看的精简版）
#[derive(Debug, serde::Serialize)]
pub struct AsrRecordSummary {
    pub id: String,
    pub title: String,
    /// 转写前 200 字预览
    pub transcript_preview: String,
    /// 摘要（若有）
    pub summary: Option<String>,
    pub created_at: String,
    pub duration_ms: u64,
    pub source: String,
    pub status: String,
}

/// 搜索 ASR 记录工具
pub struct SearchAsrTool {
    service: Arc<AsrService>,
}

impl SearchAsrTool {
    pub fn new(service: Arc<AsrService>) -> Self {
        Self { service }
    }
}

impl Tool for SearchAsrTool {
    const NAME: &'static str = "search_asr_records";

    type Error = SearchAsrError;
    type Args = SearchAsrArgs;
    type Output = Vec<AsrRecordSummary>;

  fn description(&self) -> String {
      "搜索已转写的 ASR 语音记录，关键词匹配标题/转写文本/摘要/标签，返回最近匹配列表（含预览）。"
          .to_string()
  }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "keyword": {
                    "type": "string",
                      "description": "搜索关键词（标题/转写/摘要/标签）"
                },
                "limit": {
                    "type": "integer",
                      "description": "返回条数上限",
                    "default": 10
                }
            }
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let limit = args.limit.unwrap_or(10).min(50);
        let query = AsrSearchQuery {
            keyword: args.keyword,
            limit,
            ..Default::default()
        };
        let records = self.service.search_records(&query).await?;
        let out: Vec<AsrRecordSummary> = records
            .into_iter()
            .map(|r| AsrRecordSummary {
                id: r.id,
                title: r.title,
                transcript_preview: r.transcript.chars().take(200).collect(),
                summary: r.summary,
                created_at: r.created_at.to_rfc3339(),
                duration_ms: r.duration_ms,
                source: format!("{:?}", r.source).to_lowercase(),
                status: format!("{:?}", r.status).to_lowercase(),
            })
            .collect();
        Ok(out)
    }
}

// =========================================================
// list_asr_records 工具
// =========================================================

/// list_asr_records 工具参数
#[derive(Deserialize)]
pub struct ListAsrArgs {
    /// 返回条数上限，默认 10
    #[serde(default)]
    pub limit: Option<usize>,
    /// 仅返回指定来源（streaming / upload），留空不限制
    #[serde(default)]
    pub source: Option<String>,
}

/// list_asr_records 工具错误
#[derive(Debug, thiserror::Error)]
#[error("list_asr_records error: {0}")]
pub struct ListAsrError(String);

impl From<AsrError> for ListAsrError {
    #[inline]
    fn from(e: AsrError) -> Self {
        Self(e.to_string())
    }
}

/// 列出最近 ASR 记录工具
pub struct ListAsrTool {
    service: Arc<AsrService>,
}

impl ListAsrTool {
    pub fn new(service: Arc<AsrService>) -> Self {
        Self { service }
    }
}

impl Tool for ListAsrTool {
    const NAME: &'static str = "list_asr_records";

    type Error = ListAsrError;
    type Args = ListAsrArgs;
    type Output = Vec<AsrRecordSummary>;

  fn description(&self) -> String {
      "列出最近的 ASR 语音转写记录（按创建时间倒序）。"
          .to_string()
  }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                      "description": "返回条数上限",
                    "default": 10
                },
                "source": {
                    "type": "string",
                      "description": "来源过滤：streaming 或 upload",
                    "enum": ["streaming", "upload"]
                }
            }
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let limit = args.limit.unwrap_or(10).min(50);
        let source_filter = args
            .source
            .as_deref()
            .and_then(|s| match s.to_lowercase().as_str() {
                "streaming" => Some(AsrSource::Streaming),
                "upload" => Some(AsrSource::Upload),
                _ => None,
            });
        let query = AsrSearchQuery {
            limit,
            source: source_filter,
            status: Some(AsrStatus::Completed),
            ..Default::default()
        };
        let records = self.service.search_records(&query).await?;
        let out: Vec<AsrRecordSummary> = records
            .into_iter()
            .map(|r| AsrRecordSummary {
                id: r.id,
                title: r.title,
                transcript_preview: r.transcript.chars().take(200).collect(),
                summary: r.summary,
                created_at: r.created_at.to_rfc3339(),
                duration_ms: r.duration_ms,
                source: format!("{:?}", r.source).to_lowercase(),
                status: format!("{:?}", r.status).to_lowercase(),
            })
            .collect();
        Ok(out)
    }
}

// =========================================================
// get_asr_record 工具
// =========================================================

/// get_asr_record 工具参数
#[derive(Deserialize)]
pub struct GetAsrRecordArgs {
    /// ASR 记录 id
    pub record_id: String,
}

/// get_asr_record 工具错误
#[derive(Debug, thiserror::Error)]
#[error("get_asr_record error: {0}")]
pub struct GetAsrRecordError(String);

impl From<AsrError> for GetAsrRecordError {
    #[inline]
    fn from(e: AsrError) -> Self {
        Self(e.to_string())
    }
}

/// 完整 ASR 记录详情（含完整转写文本）
#[derive(Debug, serde::Serialize)]
pub struct AsrRecordDetail {
    pub id: String,
    pub title: String,
    pub transcript: String,
    pub summary: Option<String>,
    pub created_at: String,
    pub duration_ms: u64,
    pub language: String,
    pub tags: Vec<String>,
    pub source: String,
    pub status: String,
}

/// 获取 ASR 记录详情工具
pub struct GetAsrRecordTool {
    service: Arc<AsrService>,
}

impl GetAsrRecordTool {
    pub fn new(service: Arc<AsrService>) -> Self {
        Self { service }
    }
}

impl Tool for GetAsrRecordTool {
    const NAME: &'static str = "get_asr_record";

    type Error = GetAsrRecordError;
    type Args = GetAsrRecordArgs;
    type Output = AsrRecordDetail;

  fn description(&self) -> String {
      "获取指定 ASR 记录的完整转写文本与摘要。"
          .to_string()
  }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "record_id": {
                    "type": "string",
                      "description": "ASR 记录 id（可从 list/search 获取）"
                }
            },
            "required": ["record_id"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let record = self
            .service
            .get_record(&args.record_id)
            .await?
            .ok_or_else(|| GetAsrRecordError(format!("记录 {} 不存在", args.record_id)))?;
        Ok(AsrRecordDetail {
            id: record.id,
            title: record.title,
            transcript: record.transcript,
            summary: record.summary,
            created_at: record.created_at.to_rfc3339(),
            duration_ms: record.duration_ms,
            language: record.language,
            tags: record.tags,
            source: format!("{:?}", record.source).to_lowercase(),
            status: format!("{:?}", record.status).to_lowercase(),
        })
    }
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::provider::mock::MockAsrProvider;
    use crate::asr::AsrService;
    use effisuite_core::AsrConfig;

    fn mock_service() -> Arc<AsrService> {
        let provider: Arc<dyn crate::asr::AsrProvider> = Arc::new(MockAsrProvider::new("测试转写"));
        Arc::new(AsrService::new(provider, None, None, None, AsrConfig::default()))
    }

    #[tokio::test]
    async fn transcribe_audio_missing_file() {
        let svc = mock_service();
        let tool = AsrTool::new(Arc::clone(&svc));
        let args = TranscribeAudioArgs {
            audio_path: "/nonexistent/file.wav".to_string(),
            lang: None,
        };
        let err = tool.call(args).await.unwrap_err();
        assert!(err.0.contains("音频文件不存在"));
    }

    #[tokio::test]
    async fn search_asr_returns_empty_without_store() {
        let svc = mock_service();
        let tool = SearchAsrTool::new(Arc::clone(&svc));
        let args = SearchAsrArgs {
            keyword: Some("test".to_string()),
            limit: Some(5),
        };
        let result = tool.call(args).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn list_asr_returns_empty_without_store() {
        let svc = mock_service();
        let tool = ListAsrTool::new(Arc::clone(&svc));
        let args = ListAsrArgs {
            limit: Some(10),
            source: None,
        };
        let result = tool.call(args).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn get_asr_record_nonexistent_returns_error() {
        let svc = mock_service();
        let tool = GetAsrRecordTool::new(Arc::clone(&svc));
        let args = GetAsrRecordArgs {
            record_id: "nonexistent".to_string(),
        };
        let err = tool.call(args).await.unwrap_err();
        assert!(err.0.contains("不存在"));
    }

    #[test]
    fn tool_names_are_unique() {
        assert_ne!(AsrTool::NAME, SearchAsrTool::NAME);
        assert_ne!(AsrTool::NAME, ListAsrTool::NAME);
        assert_ne!(AsrTool::NAME, GetAsrRecordTool::NAME);
        assert_ne!(SearchAsrTool::NAME, ListAsrTool::NAME);
        assert_ne!(SearchAsrTool::NAME, GetAsrRecordTool::NAME);
        assert_ne!(ListAsrTool::NAME, GetAsrRecordTool::NAME);
    }
}

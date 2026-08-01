//! ASR 数据结构定义
//!
//! 集中存放 [`AsrRecord`] / [`AsrStatus`] / [`AsrSource`] 等公开类型。
//! 将数据结构与持久化（store.rs）和索引（index.rs）逻辑解耦，便于独立演进。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::AsrProvider;

/// ASR 转写状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrStatus {
    /// 等待转写
    Pending,
    /// 正在转写
    Transcribing,
    /// 转写完成，待生成摘要
    Transcribed,
    /// 正在生成摘要
    Summarizing,
    /// 全部完成（转写 + 摘要）
    Completed,
    /// 失败
    Failed,
}

/// ASR 音频来源
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrSource {
    /// 流式录音
    Streaming,
    /// 文件上传
    Upload,
}

/// 一条 ASR 转写记录
///
/// 字段按大小降序排列以最小化 padding：
/// String（24B）→ Option<String>（24B）→ Vec<String>（24B）
/// → DateTime<Utc>（8B）→ u64（8B）→ u32（4B）→ enum（1B）
///
/// `transcript` 字段在 records.json 索引中跳过序列化（`skip_serializing_if`），
/// 完整转写文本存储在单独的 `<id>.txt` 文件中，避免索引文件膨胀。
/// 反序列化时缺省为空字符串（`#[serde(default)]`），由 [`AsrStore`](super::store::AsrStore)
/// 的 `get`/`load_all_full` 从 transcript 文件填充。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrRecord {
    pub id: String,
    pub audio_path: String,
    /// 完整转写文本。records.json 中跳过序列化，单独存文件。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub transcript: String,
    pub title: String,
    pub language: String,
    /// AI 生成的摘要
    pub summary: Option<String>,
    pub error_message: Option<String>,
    /// 用户/自动打的标签
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 音频时长毫秒
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub provider: AsrProvider,
    pub status: AsrStatus,
    pub source: AsrSource,
}

impl AsrRecord {
    /// 快速构造一条新记录：id 自动生成 uuid，title 默认从 transcript 前 30 字截取
    #[inline]
    pub fn new(audio_path: impl Into<String>, transcript: &str, source: AsrSource) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            audio_path: audio_path.into(),
            transcript: transcript.to_string(),
            title: Self::default_title(transcript),
            language: "zh-CN".to_string(),
            summary: None,
            error_message: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            duration_ms: 0,
            sample_rate: 0,
            provider: AsrProvider::VolcEngine,
            status: AsrStatus::Pending,
            source,
        }
    }

    /// 从 transcript 前 30 字截取默认标题
    #[inline]
    pub fn default_title(transcript: &str) -> String {
        transcript.chars().take(30).collect()
    }
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_serde_roundtrip() {
        let now = Utc::now();
        let record = AsrRecord {
            id: "test-id".to_string(),
            audio_path: "test.wav".to_string(),
            transcript: "这是一段转写文本".to_string(),
            title: "测试记录".to_string(),
            language: "zh-CN".to_string(),
            summary: Some("摘要内容".to_string()),
            error_message: None,
            tags: vec!["tag1".to_string(), "tag2".to_string()],
            created_at: now,
            updated_at: now,
            duration_ms: 5000,
            sample_rate: 16000,
            provider: AsrProvider::Qwen,
            status: AsrStatus::Completed,
            source: AsrSource::Upload,
        };
        let json = serde_json::to_string(&record).unwrap();
        let back: AsrRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, record.id);
        assert_eq!(back.transcript, record.transcript);
        assert_eq!(back.title, record.title);
        assert_eq!(back.summary, record.summary);
        assert_eq!(back.tags, record.tags);
        assert_eq!(back.provider, AsrProvider::Qwen);
        assert_eq!(back.status, AsrStatus::Completed);
        assert_eq!(back.source, AsrSource::Upload);
    }

    #[test]
    fn record_serde_skips_empty_transcript() {
        let record = AsrRecord {
            id: "test-id".to_string(),
            audio_path: "test.wav".to_string(),
            transcript: String::new(), // empty
            title: "测试".to_string(),
            language: "zh-CN".to_string(),
            summary: None,
            error_message: None,
            tags: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            duration_ms: 0,
            sample_rate: 0,
            provider: AsrProvider::VolcEngine,
            status: AsrStatus::Pending,
            source: AsrSource::Streaming,
        };
        let json = serde_json::to_string(&record).unwrap();
        // transcript 字段应被跳过
        assert!(!json.contains("transcript"));
        // 反序列化后 transcript 应为空字符串
        let back: AsrRecord = serde_json::from_str(&json).unwrap();
        assert!(back.transcript.is_empty());
    }

    #[test]
    fn record_old_json_without_transcript_deserializes() {
        // 模拟 records.json 中的元数据条目（无 transcript 字段）
        let json = r#"{
            "id": "r1",
            "audio_path": "a.wav",
            "title": "标题",
            "language": "zh-CN",
            "summary": null,
            "error_message": null,
            "tags": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "duration_ms": 100,
            "sample_rate": 16000,
            "provider": "volc_engine",
            "status": "completed",
            "source": "upload"
        }"#;
        let record: AsrRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.id, "r1");
        assert!(record.transcript.is_empty()); // 缺省为空
        assert_eq!(record.provider, AsrProvider::VolcEngine);
        assert_eq!(record.status, AsrStatus::Completed);
    }

    #[test]
    fn status_serde_snake_case() {
        let s = serde_json::to_string(&AsrStatus::Transcribing).unwrap();
        assert_eq!(s, "\"transcribing\"");
        let d: AsrStatus = serde_json::from_str("\"completed\"").unwrap();
        assert!(matches!(d, AsrStatus::Completed));
    }

    #[test]
    fn source_serde_snake_case() {
        let s = serde_json::to_string(&AsrSource::Streaming).unwrap();
        assert_eq!(s, "\"streaming\"");
        let d: AsrSource = serde_json::from_str("\"upload\"").unwrap();
        assert!(matches!(d, AsrSource::Upload));
    }

    #[test]
    fn default_title_takes_30_chars() {
        let short = "短文本";
        assert_eq!(AsrRecord::default_title(short), "短文本");

        let long = "这是一段很长的转写文本用于测试标题截取功能应该只取前三十个字符剩下的不取";
        let title = AsrRecord::default_title(long);
        assert_eq!(title.chars().count(), 30);
    }

    #[test]
    fn new_generates_uuid_id() {
        let record = AsrRecord::new("audio.wav", "转写内容", AsrSource::Upload);
        assert!(!record.id.is_empty());
        assert_eq!(record.transcript, "转写内容");
        assert_eq!(record.title, "转写内容"); // 前 30 字 = 全文
        assert!(matches!(record.status, AsrStatus::Pending));
        assert!(matches!(record.source, AsrSource::Upload));
    }
}

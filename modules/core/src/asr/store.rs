//! ASR 记录持久化存储
//!
//! 基于 JSON 文件的持久化方案：所有记录的元数据（不含 transcript）存放在
//! `records.json` 单文件索引中，每条记录的完整转写文本单独存放在
//! `transcripts/<id>.txt` 文件中。
//!
//! # 设计要点（对齐 user_rules 与 ConversationStore 模式）
//!
//! - **每记录独立锁**：`records_locks` 仅阻塞同一记录的并发操作，不同记录互不阻塞
//! - **全局文件锁**：`file_lock` 序列化 records.json 的读-改-写，临界区极短
//!   （仅 records.json 读写，transcript 文件 IO 在 file_lock 外完成）
//! - **锁外搜索**：`search` 先 `load_all` 释放锁，再用迭代器链过滤
//! - **transcript 分离存储**：records.json 仅存元数据（transcript 跳过序列化），
//!   避免 JSON 索引文件随转写文本膨胀；`get`/`load_all_full` 按需从 txt 文件加载
//! - **异步 IO**：全部使用 `tokio::fs`
//! - **Send/Sync 安全**：`Arc<tokio::sync::Mutex<()>>` + `Arc<StdMutex<HashMap>>`

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex as StdMutex;

use chrono::{DateTime, Utc};
use tokio::sync::Mutex;

use crate::{CoreError, Result};

use super::types::{AsrRecord, AsrSource, AsrStatus};

/// ASR 记录搜索查询
///
/// 所有字段可选，None 表示不限制。`limit` 默认 50。
pub struct AsrSearchQuery {
    pub keyword: Option<String>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub source: Option<AsrSource>,
    pub status: Option<AsrStatus>,
    pub limit: usize,
}

impl Default for AsrSearchQuery {
    fn default() -> Self {
        Self {
            keyword: None,
            start: None,
            end: None,
            source: None,
            status: None,
            limit: 50,
        }
    }
}

/// ASR 记录存储，线程安全且可廉价 clone（内部 `Arc` 共享）
///
/// 读-改-写操作（save/delete）使用每记录独立锁 + 全局文件锁。
/// 纯读操作（load_all_meta/get/search）仅短暂持有文件锁读取 records.json，
/// 搜索/过滤在锁外完成。
#[derive(Clone)]
pub struct AsrStore {
    /// ASR 根目录（`<appdata>/effisuite/asr/`）
    root: PathBuf,
    /// records.json 全局文件锁：序列化读-改-写操作
    file_lock: std::sync::Arc<Mutex<()>>,
    /// 每记录独立锁表：`record_id → Arc<Mutex<()>>`。
    /// 外层 `StdMutex` 仅短暂持有（无 IO/await），内层 `tokio::Mutex` 跨 await 持有。
    records_locks: std::sync::Arc<StdMutex<HashMap<String, std::sync::Arc<Mutex<()>>>>>,
}

impl AsrStore {
    /// 创建存储，root 不存在时自动创建（含 transcripts/audio 子目录）
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        std::fs::create_dir_all(root.join("transcripts")).map_err(CoreError::Io)?;
        std::fs::create_dir_all(root.join("audio")).map_err(CoreError::Io)?;
        Ok(Self {
            root,
            file_lock: std::sync::Arc::new(Mutex::new(())),
            records_locks: std::sync::Arc::new(StdMutex::new(HashMap::new())),
        })
    }

    /// 获取指定记录的独立锁（不存在则创建）。
    /// 外层 `StdMutex` 仅短暂持有以查表，无 IO/await。
    #[inline]
    fn record_lock(&self, id: &str) -> std::sync::Arc<Mutex<()>> {
        let mut map = self.records_locks.lock().unwrap();
        map.entry(id.to_string())
            .or_insert_with(|| std::sync::Arc::new(Mutex::new(())))
            .clone()
    }

    /// transcript 文件路径：`<root>/transcripts/<id>.txt`
    /// 防止 id 含路径分隔符：仅取 file_name 部分
    #[inline]
    fn transcript_path(&self, id: &str) -> PathBuf {
        let safe = Path::new(id)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(id));
        self.root.join("transcripts").join(safe).with_extension("txt")
    }

    /// 从相对音频路径构造绝对路径：`<root>/audio/<rel>`
    #[inline]
    fn audio_abs_path(&self, rel: &str) -> PathBuf {
        self.root.join("audio").join(rel)
    }

    /// records.json 路径
    #[inline]
    fn records_path(&self) -> PathBuf {
        self.root.join("records.json")
    }

    /// 读取 records.json 全部记录（transcript 字段为空，需用 get/load_all_full 填充）
    async fn read_records(&self) -> Result<Vec<AsrRecord>> {
        let path = self.records_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let bytes = tokio::fs::read(&path).await.map_err(CoreError::Io)?;
        let records: Vec<AsrRecord> = serde_json::from_slice(&bytes).map_err(CoreError::Serde)?;
        Ok(records)
    }

    /// 写入 records.json（全部记录，transcript 为空被跳过）
    async fn write_records(&self, records: &[AsrRecord]) -> Result<()> {
        let path = self.records_path();
        let bytes = serde_json::to_vec_pretty(records).map_err(CoreError::Serde)?;
        tokio::fs::write(&path, bytes)
            .await
            .map_err(CoreError::Io)?;
        Ok(())
    }

    /// 加载全部记录元数据（不含 transcript）。
    ///
    /// 临界区极短：仅持有 file_lock 读取 records.json，立即释放。
    /// 适合列表展示等不需要转写文本的场景。
    pub async fn load_all_meta(&self) -> Result<Vec<AsrRecord>> {
        let _guard = self.file_lock.lock().await;
        self.read_records().await
    }

    /// 加载全部记录（含 transcript）。
    ///
    /// 先读 records.json（短临界区），再在锁外逐条读取 transcript 文件。
    pub async fn load_all_full(&self) -> Result<Vec<AsrRecord>> {
        let records = {
            let _guard = self.file_lock.lock().await;
            self.read_records().await?
        };
        // 锁外读取 transcript 文件
        let count = records.len();
        let mut out = Vec::with_capacity(count);
        for mut record in records {
            let transcript_path = self.transcript_path(&record.id);
            if let Ok(bytes) = tokio::fs::read(&transcript_path).await {
                record.transcript = String::from_utf8_lossy(&bytes).into_owned();
            }
            out.push(record);
        }
        Ok(out)
    }

    /// 获取单条记录（含 transcript）。不存在返回 None。
    pub async fn get(&self, id: &str) -> Result<Option<AsrRecord>> {
        // 短临界区：读 records.json 找记录
        let record = {
            let _guard = self.file_lock.lock().await;
            let records = self.read_records().await?;
            records.into_iter().find(|r| r.id == id)
        };
        // 锁外读 transcript 文件
        match record {
            Some(mut r) => {
                let transcript_path = self.transcript_path(id);
                if let Ok(bytes) = tokio::fs::read(&transcript_path).await {
                    r.transcript = String::from_utf8_lossy(&bytes).into_owned();
                }
                Ok(Some(r))
            }
            None => Ok(None),
        }
    }

    /// 保存（或覆盖）一条记录。
    ///
    /// - id 为空时自动生成 uuid
    /// - transcript 写入单独的 `<id>.txt` 文件
    /// - 元数据（transcript 清空）写入 records.json 索引
    pub async fn save(&self, mut record: AsrRecord) -> Result<()> {
        if record.id.is_empty() {
            record.id = uuid::Uuid::new_v4().to_string();
        }
        let id = record.id.clone();

        // 每记录锁：防止对同一记录的并发操作
        let lock = self.record_lock(&id);
        let _record_guard = lock.lock().await;

        // 写 transcript 到单独文件（file_lock 外，不影响其他记录的索引操作）
        let transcript_path = self.transcript_path(&id);
        if record.transcript.is_empty() {
            let _ = tokio::fs::remove_file(&transcript_path).await;
        } else {
            tokio::fs::write(&transcript_path, record.transcript.as_bytes())
                .await
                .map_err(CoreError::Io)?;
        }

        // 准备元数据（transcript 清空，被 skip_serializing_if 跳过）
        record.transcript.clear();
        let meta = record;

        // 读-改-写 records.json（短临界区）
        {
            let _file_guard = self.file_lock.lock().await;
            let mut records = self.read_records().await?;
            match records.iter_mut().find(|r| r.id == id) {
                Some(existing) => *existing = meta,
                None => records.push(meta),
            }
            self.write_records(&records).await?;
        }

        Ok(())
    }

    /// 删除指定记录及其 transcript 文件与音频文件。不存在视为成功（幂等）。
    pub async fn delete(&self, id: &str) -> Result<()> {
        let lock = self.record_lock(id);
        let _record_guard = lock.lock().await;

        // 读-改-写 records.json，捕获 audio_path 用于文件清理
        let audio_rel = {
            let _file_guard = self.file_lock.lock().await;
            let mut records = self.read_records().await?;
            match records.iter().position(|r| r.id == id) {
                Some(idx) => {
                    let removed = records.remove(idx);
                    self.write_records(&records).await?;
                    Some(removed.audio_path)
                }
                None => None,
            }
        };

        // 锁外删除 transcript 与音频文件
        if let Some(rel) = audio_rel {
            let transcript_path = self.transcript_path(id);
            let _ = tokio::fs::remove_file(&transcript_path).await;
            if !rel.is_empty() {
                let audio_abs = self.audio_abs_path(&rel);
                let _ = tokio::fs::remove_file(&audio_abs).await;
            }
        }

        Ok(())
    }

    /// 搜索记录：按时间范围 + 关键词（标题/transcript/summary/tags 子串匹配）
    /// + source/status 过滤，用迭代器链，在锁外执行。
    ///
    /// 有关键词时调用 `load_all_full`（需 transcript 参与匹配），
    /// 无关键词时调用 `load_all_meta`（更快，跳过 transcript 文件读取）。
    pub async fn search(&self, query: &AsrSearchQuery) -> Result<Vec<AsrRecord>> {
        let records = if query.keyword.is_some() {
            self.load_all_full().await?
        } else {
            self.load_all_meta().await?
        };

        let keyword_lower = query.keyword.as_ref().map(|k| k.to_lowercase());
        let start = query.start;
        let end = query.end;
        let source_filter = query.source;
        let status_filter = query.status;
        let limit = if query.limit == 0 { 50 } else { query.limit };

        let results: Vec<AsrRecord> = records
            .into_iter()
            .filter(|r| {
                keyword_lower.as_ref().map_or(true, |kw| {
                    r.title.to_lowercase().contains(kw.as_str())
                        || r.transcript.to_lowercase().contains(kw.as_str())
                        || r.summary
                            .as_deref()
                            .map_or(false, |s| s.to_lowercase().contains(kw.as_str()))
                        || r.tags.iter().any(|t| t.to_lowercase().contains(kw.as_str()))
                })
            })
            .filter(|r| start.map_or(true, |s| r.created_at >= s))
            .filter(|r| end.map_or(true, |e| r.created_at <= e))
            .filter(|r| source_filter.map_or(true, |s| r.source == s))
            .filter(|r| status_filter.map_or(true, |s| r.status == s))
            .take(limit)
            .collect();

        Ok(results)
    }
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AsrProvider;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "effisuite-asr-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_record(id: &str, title: &str, transcript: &str) -> AsrRecord {
        let now = Utc::now();
        AsrRecord {
            id: id.to_string(),
            audio_path: format!("{}.wav", id),
            transcript: transcript.to_string(),
            title: title.to_string(),
            language: "zh-CN".to_string(),
            summary: Some("摘要内容".to_string()),
            error_message: None,
            tags: vec!["test".to_string()],
            created_at: now,
            updated_at: now,
            duration_ms: 5000,
            sample_rate: 16000,
            provider: AsrProvider::VolcEngine,
            status: AsrStatus::Completed,
            source: AsrSource::Upload,
        }
    }

    #[tokio::test]
    async fn save_get_delete_roundtrip() {
        let dir = tmp_dir();
        let store = AsrStore::new(&dir).unwrap();

        // 不存在时 get 返回 None
        assert!(store.get("r1").await.unwrap().is_none());

        // save
        let record = make_record("r1", "测试记录", "这是一段转写文本");
        store.save(record).await.unwrap();

        // get 应返回完整记录（含 transcript）
        let loaded = store.get("r1").await.unwrap().unwrap();
        assert_eq!(loaded.id, "r1");
        assert_eq!(loaded.title, "测试记录");
        assert_eq!(loaded.transcript, "这是一段转写文本");

        // delete
        store.delete("r1").await.unwrap();
        assert!(store.get("r1").await.unwrap().is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn save_generates_uuid_for_empty_id() {
        let dir = tmp_dir();
        let store = AsrStore::new(&dir).unwrap();

        let record = make_record("", "无ID", "内容");
        store.save(record).await.unwrap();

        // record.id 被修改了（save 取所有权），但 store 内部应有记录
        // 验证 load_all_meta 返回一条记录
        let metas = store.load_all_meta().await.unwrap();
        assert_eq!(metas.len(), 1);
        assert!(!metas[0].id.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn load_all_meta_excludes_transcript() {
        let dir = tmp_dir();
        let store = AsrStore::new(&dir).unwrap();

        store
            .save(make_record("r1", "记录", "转写内容"))
            .await
            .unwrap();

        let metas = store.load_all_meta().await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, "r1");
        assert!(metas[0].transcript.is_empty()); // 元数据不含 transcript

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn load_all_full_includes_transcript() {
        let dir = tmp_dir();
        let store = AsrStore::new(&dir).unwrap();

        store
            .save(make_record("r1", "记录", "完整转写文本"))
            .await
            .unwrap();

        let full = store.load_all_full().await.unwrap();
        assert_eq!(full.len(), 1);
        assert_eq!(full[0].transcript, "完整转写文本");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn save_updates_existing_record() {
        let dir = tmp_dir();
        let store = AsrStore::new(&dir).unwrap();

        store
            .save(make_record("r1", "原标题", "原转写"))
            .await
            .unwrap();

        // 覆盖保存
        store
            .save(make_record("r1", "新标题", "新转写"))
            .await
            .unwrap();

        let loaded = store.get("r1").await.unwrap().unwrap();
        assert_eq!(loaded.title, "新标题");
        assert_eq!(loaded.transcript, "新转写");

        // records.json 应只有一条记录
        let metas = store.load_all_meta().await.unwrap();
        assert_eq!(metas.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_by_keyword_in_title() {
        let dir = tmp_dir();
        let store = AsrStore::new(&dir).unwrap();

        store
            .save(make_record("r1", "Rust 编程", "内容A"))
            .await
            .unwrap();
        store
            .save(make_record("r2", "Python 入门", "内容B"))
            .await
            .unwrap();

        let results = store
            .search(&AsrSearchQuery {
                keyword: Some("rust".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "r1");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_by_keyword_in_transcript() {
        let dir = tmp_dir();
        let store = AsrStore::new(&dir).unwrap();

        store
            .save(make_record("r1", "记录A", "讨论了异步编程模型"))
            .await
            .unwrap();
        store
            .save(make_record("r2", "记录B", "关于数据库设计"))
            .await
            .unwrap();

        let results = store
            .search(&AsrSearchQuery {
                keyword: Some("异步".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "r1");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_by_status_and_source() {
        let dir = tmp_dir();
        let store = AsrStore::new(&dir).unwrap();

        store.save(make_record("r1", "A", "内容A")).await.unwrap();
        store.save(make_record("r2", "B", "内容B")).await.unwrap();

        let results = store
            .search(&AsrSearchQuery {
                status: Some(AsrStatus::Completed),
                source: Some(AsrSource::Upload),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(results.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn search_with_limit() {
        let dir = tmp_dir();
        let store = AsrStore::new(&dir).unwrap();

        for i in 0..10 {
            store
                .save(make_record(&format!("r{}", i), "标题", "内容"))
                .await
                .unwrap();
        }

        let results = store
            .search(&AsrSearchQuery {
                limit: 3,
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(results.len(), 3);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let dir = tmp_dir();
        let store = AsrStore::new(&dir).unwrap();

        // 删除不存在的记录不报错
        store.delete("nonexistent").await.unwrap();
    }

    #[tokio::test]
    async fn delete_removes_transcript_file() {
        let dir = tmp_dir();
        let store = AsrStore::new(&dir).unwrap();

        store
            .save(make_record("r1", "记录", "转写内容"))
            .await
            .unwrap();

        // transcript 文件应存在
        let transcript_path = store.transcript_path("r1");
        assert!(transcript_path.exists());

        store.delete("r1").await.unwrap();

        // transcript 文件应被删除
        assert!(!transcript_path.exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn persist_and_reload_across_instances() {
        let dir = tmp_dir();
        let store_a = AsrStore::new(&dir).unwrap();
        store_a
            .save(make_record("r1", "持久化", "持久化的转写"))
            .await
            .unwrap();

        // 用同一目录再创建一个实例，应能读到已落盘数据
        let store_b = AsrStore::new(&dir).unwrap();
        let loaded = store_b.get("r1").await.unwrap().unwrap();
        assert_eq!(loaded.title, "持久化");
        assert_eq!(loaded.transcript, "持久化的转写");

        std::fs::remove_dir_all(&dir).ok();
    }
}

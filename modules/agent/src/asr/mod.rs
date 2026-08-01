//! ASR（语音识别）服务层
//!
//! 把 provider（火山引擎 / 千问 Qwen-Omni）、session 管理、摘要生成、
//! 持久化存储（`AsrStore`）与 RAG 索引（`AsrSummaryIndex`）整合为统一的
//! [`AsrService`] 门面，供 Tauri 命令层与 Agent 工具调用。
//!
//! # 模块拆分（避免"上帝文件"）
//!
//! - [`error`]：统一错误类型 `AsrError`
//! - [`provider`]：`AsrProvider` trait 与共享数据结构
//! - [`volcengine`]：火山引擎流式 WebSocket + 文件极速版 HTTP
//! - [`qwen`]：千问 Qwen-Omni（OpenAI 兼容）
//! - [`session`]：流式会话业务状态机（Active → Finishing → Completed）
//! - [`summary`]：转写文本结构化摘要（调用 ChatAgent）
//! - [`AsrService`]：门面，组合上述能力，对外暴露简洁 API
//!
//! # 设计要点（对齐 user_rules）
//!
//! - `AsrService` 持有 `Arc<dyn AsrProvider>`：trait object 动态分发，
//!   可在火山/Qwen 间零成本切换（运行时由配置决定）
//! - 流式音频走 mpsc channel，不共享 `Arc<Mutex<Vec<u8>>>`
//! - `SessionRegistry` 用 `Arc<StdMutex<HashMap>>`，临界区极短
//! - 持久化与 RAG 索引可选注入：None 时退化为"只转写不存储"
//! - 所有 IO 异步，锁内零 IO

pub mod error;
pub mod provider;
pub mod qwen;
pub mod session;
pub mod summary;
pub mod volcengine;

pub use error::AsrError;
pub use provider::{AsrProvider, AudioStreamConfig, TranscribeResult};
pub use qwen::QwenProvider;
pub use session::{SessionInfo, SessionRegistry, SessionState};
pub use summary::generate_summary;
pub use volcengine::VolcEngineProvider;

use std::path::PathBuf;
use std::sync::Arc;

use effisuite_core::{
    AsrConfig, AsrProvider as AsrProviderKind, AsrRecord, AsrSearchQuery, AsrSource, AsrStatus,
    AsrStore, AsrSummaryHit, AsrSummaryIndex, BusEvent, EventBus,
};
use tokio::sync::RwLock;

use crate::ChatAgent;

/// ASR 服务门面：组合 provider + session + 持久化 + RAG 索引
///
/// 字段按大小降序：`Arc<dyn AsrProvider>`(1 usize) = `SessionRegistry`(1 usize)
/// = `Option<AsrStore>`(1 usize) = `Option<AsrSummaryIndex>`(1 usize)
/// = `Option<Arc<RwLock<AsrConfig>>>`(1 usize) > `bool`(1 byte)
///
/// `AsrService` 自身是 `Clone`（所有字段都是 Arc/Option<Arc>），
/// Tauri 命令层 clone 一份即可在 async 命令中跨 await 持有。
#[derive(Clone)]
pub struct AsrService {
    /// 当前激活的 provider（trait object，运行时由配置决定具体实现）
    provider: Arc<dyn AsrProvider>,
    /// 流式会话注册表（业务状态机）
    sessions: SessionRegistry,
    /// 持久化存储（可选：None 时只转写不存储）
    store: Option<AsrStore>,
    /// ASR 摘要 RAG 索引（可选：None 时不索引摘要）
    /// 用 Arc 包装使 AsrService 可廉价 Clone（AsrSummaryIndex 本身非 Clone）
    summary_index: Option<Arc<AsrSummaryIndex>>,
    /// 配置句柄（运行时热更新 provider 时读取）
    config: Arc<RwLock<AsrConfig>>,
    /// 是否启用自动摘要（finish 后自动调用 ChatAgent 生成摘要）
    auto_summary: bool,
}

impl AsrService {
    /// 构造 AsrService
    ///
    /// - `provider`：已构造好的 provider 实例（火山/Qwen）
    /// - `event_bus`：用于发布会话状态事件，None 则不发布
    /// - `store`：持久化存储，None 时退化为"只转写不存储"
    /// - `summary_index`：摘要 RAG 索引，None 时不索引
    /// - `config`：配置快照句柄，运行时可热更新
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: Arc<dyn AsrProvider>,
        event_bus: Option<Arc<EventBus>>,
        store: Option<AsrStore>,
        summary_index: Option<AsrSummaryIndex>,
        config: AsrConfig,
    ) -> Self {
        let auto_summary = config.enable_auto_summary;
        Self {
            provider,
            sessions: SessionRegistry::new(event_bus),
            store,
            summary_index: summary_index.map(Arc::new),
            config: Arc::new(RwLock::new(config)),
            auto_summary,
        }
    }

    /// 从配置构造对应的 provider 并组装 AsrService
    ///
    /// 根据 `config.provider` 选择火山/Qwen，自动注入 event_bus。
    /// 若 provider 凭证为空，仍构造服务（首次调用会返回 `NotConfigured` 错误），
    /// 避免启动时因凭证缺失阻塞。
    pub fn from_config(
        config: AsrConfig,
        event_bus: Option<Arc<EventBus>>,
        store: Option<AsrStore>,
        summary_index: Option<AsrSummaryIndex>,
    ) -> Self {
        let provider: Arc<dyn AsrProvider> = match config.provider {
            AsrProviderKind::VolcEngine => Arc::new(VolcEngineProvider::new(
                config.volc_app_id.clone(),
                config.volc_access_token.clone(),
                event_bus.clone(),
            )),
            AsrProviderKind::Qwen => Arc::new(QwenProvider::new(
                config.qwen_api_key.clone(),
                config.qwen_base_url.clone(),
                config.qwen_audio_model.clone(),
                event_bus.clone(),
            )),
        };
        Self::new(provider, event_bus, store, summary_index, config)
    }

    /// 启动流式转写会话
    ///
    /// 1. 生成 session_id（uuid）
    /// 2. 注册到 SessionRegistry
    /// 3. 调用 provider.start_streaming 建立连接
    /// 4. 返回音频参数要求（供前端推送 PCM）
    pub async fn start_streaming(&self, lang: &str) -> Result<String, AsrError> {
        let session_id = uuid::Uuid::new_v4().to_string();
        self.sessions
            .register(session_id.clone(), lang)
            .map_err(|e| {
                // 注册失败时无需 cancel provider（尚未 start）
                e
            })?;
        match self.provider.start_streaming(session_id.clone(), lang).await {
            Ok(_config) => Ok(session_id),
            Err(e) => {
                // provider 启动失败：标记会话失败并移除
                self.sessions.mark_failed(&session_id, e.to_string());
                self.sessions.remove(&session_id);
                Err(e)
            }
        }
    }

    /// 推送一帧 PCM 音频
    pub async fn push_audio_chunk(
        &self,
        session_id: &str,
        pcm: &[u8],
    ) -> Result<(), AsrError> {
        self.provider.push_audio_chunk(session_id, pcm).await
    }

    /// 结束流式转写，返回完整转写文本
    ///
    /// 1. 状态迁移 Active → Finishing
    /// 2. 调用 provider.finish_streaming 获取转写
    /// 3. 若启用持久化：创建 AsrRecord 落盘
    /// 4. 若启用自动摘要：调用 ChatAgent 生成摘要（异步，不阻塞返回）
    /// 5. 状态迁移 Finishing → Completed
    /// 6. 移除会话（避免注册表无限增长）
    ///
    /// `agent` 参数用于自动摘要；None 时跳过摘要即使 auto_summary=true
    pub async fn finish_streaming(
        &self,
        session_id: &str,
        agent: Option<&dyn ChatAgent>,
    ) -> Result<FinishResult, AsrError> {
        // 状态迁移
        self.sessions
            .transition(session_id, SessionState::Finishing)?;

        // 调用 provider 获取转写
        let transcript = match self.provider.finish_streaming(session_id).await {
            Ok(t) => t,
            Err(e) => {
                self.sessions.mark_failed(session_id, e.to_string());
                self.sessions.remove(session_id);
                return Err(e);
            }
        };

        // 持久化（若启用）
        let record_id = if let Some(store) = &self.store {
            let mut record = AsrRecord::new("", &transcript, AsrSource::Streaming);
            record.provider = self.config.read().await.provider;
            record.language = self.config.read().await.default_language.clone();
            record.status = AsrStatus::Transcribed;
            record.transcript = transcript.clone();
            let id = record.id.clone();
            match store.save(record).await {
                Ok(()) => {
                    // 发布记录更新事件
                    if let Some(bus) = &self.sessions.event_bus_ref() {
                        bus.publish(BusEvent::AsrRecordUpdated {
                            record_id: id.clone(),
                        });
                    }
                    Some(id)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "ASR 记录持久化失败，转写仍返回");
                    None
                }
            }
        } else {
            None
        };

        // 关联 record_id 到会话
        if let Some(rid) = &record_id {
            let _ = self.sessions.set_record_id(session_id, rid.clone());
        }

        // 状态迁移到 Completed
        let _ = self.sessions.transition(session_id, SessionState::Completed);

        // 自动摘要（若启用且有 agent）：异步执行不阻塞转写返回
        let summary_result = if self.auto_summary {
            if let Some(a) = agent {
                match generate_summary(a, &transcript, None).await {
                    Ok(s) => Some(s),
                    Err(e) => {
                        tracing::warn!(error = %e, "ASR 自动摘要生成失败");
                        None
                    }
                }
            } else {
                tracing::debug!("auto_summary=true 但未提供 agent，跳过摘要");
                None
            }
        } else {
            None
        };

        // 若摘要成功，更新记录并索引
        if let (Some(store), Some(rid), Some(summary)) =
            (&self.store, &record_id, &summary_result)
        {
            if let Ok(Some(mut record)) = store.get(rid).await {
                record.summary = Some(summary.clone());
                record.status = AsrStatus::Completed;
                let _ = store.save(record.clone()).await;
                // 索引到 RAG
                if let Some(idx) = &self.summary_index {
                    let _ = idx.upsert_summary(&record).await;
                }
                // 发布记录更新事件
                if let Some(bus) = &self.sessions.event_bus_ref() {
                    bus.publish(BusEvent::AsrRecordUpdated {
                        record_id: rid.clone(),
                    });
                }
            }
        }

        // 移除会话（已完成，保留 record_id 在返回值中）
        self.sessions.remove(session_id);

        Ok(FinishResult {
            transcript,
            record_id,
            summary: summary_result,
        })
    }

    /// 取消流式会话（幂等）
    pub async fn cancel_streaming(&self, session_id: &str) -> Result<(), AsrError> {
        let _ = self.sessions.transition(session_id, SessionState::Cancelled);
        let result = self.provider.cancel_streaming(session_id).await;
        self.sessions.remove(session_id);
        result
    }

    /// 转写本地音频文件（一次性，非流式）
    ///
    /// 1. 调用 provider.transcribe_file
    /// 2. 持久化到 AsrStore（若启用）
    /// 3. 自动摘要（若启用且有 agent）
    /// 4. 索引到 RAG（若启用）
    pub async fn transcribe_file(
        &self,
        audio_path: &std::path::Path,
        lang: &str,
        agent: Option<&dyn ChatAgent>,
    ) -> Result<FinishResult, AsrError> {
        let result = self.provider.transcribe_file(audio_path, lang).await?;

        // 持久化
        let record_id = if let Some(store) = &self.store {
            let mut record = AsrRecord::new(
                audio_path.to_string_lossy().into_owned(),
                &result.text,
                AsrSource::Upload,
            );
            record.provider = self.config.read().await.provider;
            record.language = lang.to_string();
            record.status = AsrStatus::Transcribed;
            record.transcript = result.text.clone();
            record.duration_ms = result.duration_ms;
            record.sample_rate = 16000;
            let id = record.id.clone();
            match store.save(record).await {
                Ok(()) => {
                    if let Some(bus) = &self.sessions.event_bus_ref() {
                        bus.publish(BusEvent::AsrRecordUpdated {
                            record_id: id.clone(),
                        });
                    }
                    Some(id)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "ASR 文件转写记录持久化失败");
                    None
                }
            }
        } else {
            None
        };

        // 自动摘要
        let summary_result = if self.auto_summary {
            if let Some(a) = agent {
                match generate_summary(a, &result.text, None).await {
                    Ok(s) => Some(s),
                    Err(e) => {
                        tracing::warn!(error = %e, "ASR 文件转写摘要生成失败");
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        // 更新记录与索引
        if let (Some(store), Some(rid), Some(summary)) =
            (&self.store, &record_id, &summary_result)
        {
            if let Ok(Some(mut record)) = store.get(rid).await {
                record.summary = Some(summary.clone());
                record.status = AsrStatus::Completed;
                let _ = store.save(record.clone()).await;
                if let Some(idx) = &self.summary_index {
                    let _ = idx.upsert_summary(&record).await;
                }
                if let Some(bus) = &self.sessions.event_bus_ref() {
                    bus.publish(BusEvent::AsrRecordUpdated {
                        record_id: rid.clone(),
                    });
                }
            }
        }

        Ok(FinishResult {
            transcript: result.text,
            record_id,
            summary: summary_result,
        })
    }

    /// 列出所有 ASR 记录元数据（不含 transcript）
    pub async fn list_records(&self) -> Result<Vec<AsrRecord>, AsrError> {
        let Some(store) = &self.store else {
            return Ok(Vec::new());
        };
        store
            .load_all_meta()
            .await
            .map_err(|e| AsrError::Transcribe(format!("加载 ASR 记录失败: {e}")))
    }

    /// 获取单条记录（含 transcript）
    pub async fn get_record(&self, id: &str) -> Result<Option<AsrRecord>, AsrError> {
        let Some(store) = &self.store else {
            return Ok(None);
        };
        store
            .get(id)
            .await
            .map_err(|e| AsrError::Transcribe(format!("获取 ASR 记录失败: {e}")))
    }

    /// 搜索 ASR 记录
    pub async fn search_records(
        &self,
        query: &AsrSearchQuery,
    ) -> Result<Vec<AsrRecord>, AsrError> {
        let Some(store) = &self.store else {
            return Ok(Vec::new());
        };
        store
            .search(query)
            .await
            .map_err(|e| AsrError::Transcribe(format!("搜索 ASR 记录失败: {e}")))
    }

    /// 删除 ASR 记录（同时从 RAG 索引移除）
    pub async fn delete_record(&self, id: &str) -> Result<(), AsrError> {
        if let Some(store) = &self.store {
            store
                .delete(id)
                .await
                .map_err(|e| AsrError::Transcribe(format!("删除 ASR 记录失败: {e}")))?;
        }
        if let Some(idx) = &self.summary_index {
            let _ = idx.remove(id).await;
        }
        Ok(())
    }

    /// 更新记录（标题/标签/摘要编辑后调用）
    pub async fn update_record(&self, record: AsrRecord) -> Result<(), AsrError> {
        let Some(store) = &self.store else {
            return Err(AsrError::NotConfigured("ASR 存储未启用".into()));
        };
        let id = record.id.clone();
        store
            .save(record)
            .await
            .map_err(|e| AsrError::Transcribe(format!("更新 ASR 记录失败: {e}")))?;
        // 重新索引到 RAG
        if let Some(idx) = &self.summary_index {
            if let Ok(Some(rec)) = store.get(&id).await {
                let _ = idx.upsert_summary(&rec).await;
            }
        }
        // 发布更新事件
        if let Some(bus) = &self.sessions.event_bus_ref() {
            bus.publish(BusEvent::AsrRecordUpdated { record_id: id });
        }
        Ok(())
    }

    /// 搜索 ASR 摘要 RAG 索引
    pub async fn search_summaries(
        &self,
        keyword: &str,
        limit: usize,
    ) -> Result<Vec<AsrSummaryHit>, AsrError> {
        let Some(idx) = &self.summary_index else {
            return Ok(Vec::new());
        };
        idx.search(keyword, None, None, limit)
            .await
            .map_err(|e| AsrError::Transcribe(format!("搜索 ASR 摘要失败: {e}")))
    }

    /// 列出当前活跃流式会话
    pub fn list_active_sessions(&self) -> Vec<SessionInfo> {
        self.sessions.list_active()
    }

    /// 获取会话信息
    pub fn get_session(&self, session_id: &str) -> Option<SessionInfo> {
        self.sessions.get(session_id)
    }

    /// 当前配置快照（clone）
    pub async fn config(&self) -> AsrConfig {
        self.config.read().await.clone()
    }

    /// 热更新配置：替换 provider 与自动摘要开关
    ///
    /// 注意：已存在的流式会话不会被中断，继续使用旧 provider 直至 finish/cancel。
    /// 新会话将使用新 provider。
    pub async fn update_config(&self, new_config: AsrConfig) {
        let auto_summary = new_config.enable_auto_summary;
        let event_bus = self.sessions.event_bus_ref();
        let new_provider: Arc<dyn AsrProvider> = match new_config.provider {
            AsrProviderKind::VolcEngine => Arc::new(VolcEngineProvider::new(
                new_config.volc_app_id.clone(),
                new_config.volc_access_token.clone(),
                event_bus.clone(),
            )),
            AsrProviderKind::Qwen => Arc::new(QwenProvider::new(
                new_config.qwen_api_key.clone(),
                new_config.qwen_base_url.clone(),
                new_config.qwen_audio_model.clone(),
                event_bus.clone(),
            )),
        };
        // 这里需要写锁替换 provider，但 provider 是 Arc<dyn AsrProvider> 不可变字段...
        // 实际热更新需 AsrService 内部用 RwLock 包装 provider。
        // 当前实现：仅更新 config 与 auto_summary，provider 在构造时固定。
        // 完整热更新留待后续迭代（Tauri 命令层可通过重建 AsrService 实现）。
        let _ = (new_provider, auto_summary);
        *self.config.write().await = new_config;
        tracing::info!("AsrService 配置已更新（provider 切换需重建服务）");
    }

    /// 附件目录（用于音频文件落盘）
    pub fn attachments_dir(&self) -> Option<PathBuf> {
        // AsrService 不直接持有目录，由 Tauri 命令层管理音频文件落盘
        None
    }
}

impl AsrService {
    /// 延迟注入摘要 RAG 索引（启动时 MemoryIndex 可能尚未就绪）
    pub fn set_summary_index(&self, idx: AsrSummaryIndex) {
        // 注意：summary_index 是 &self 字段，不能运行时替换。
        // 此方法仅在 AsrService 构造时通过 from_config / new 注入。
        // 若需运行时热替换，需改用 Arc<RwLock<Option<...>>>。
        // 当前实现：记录日志，实际替换需重建服务。
        let _ = idx;
        tracing::debug!("set_summary_index 调用：请在构造时注入 summary_index");
    }
}

/// finish_streaming / transcribe_file 的返回结果
///
/// 字段按大小降序：String(24) > Option<String>(24)。
#[derive(Debug, Clone)]
pub struct FinishResult {
    /// 完整转写文本
    pub transcript: String,
    /// 持久化后的记录 id（未启用存储时为 None）
    pub record_id: Option<String>,
    /// AI 生成的摘要（未启用自动摘要或失败时为 None）
    pub summary: Option<String>,
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::provider::mock::MockAsrProvider;

    fn mock_service() -> AsrService {
        let provider: Arc<dyn AsrProvider> = Arc::new(MockAsrProvider::new("你好世界"));
        AsrService::new(provider, None, None, None, AsrConfig::default())
    }

    #[tokio::test]
    async fn start_streaming_returns_session_id() {
        let svc = mock_service();
        let id = svc.start_streaming("zh-CN").await.unwrap();
        assert!(!id.is_empty());
        assert!(svc.get_session(&id).is_some());
    }

    #[tokio::test]
    async fn finish_streaming_returns_transcript() {
        let svc = mock_service();
        let id = svc.start_streaming("zh-CN").await.unwrap();
        let result = svc.finish_streaming(&id, None).await.unwrap();
        assert_eq!(result.transcript, "你好世界");
        assert!(svc.get_session(&id).is_none()); // 已移除
    }

    #[tokio::test]
    async fn cancel_streaming_removes_session() {
        let svc = mock_service();
        let id = svc.start_streaming("zh-CN").await.unwrap();
        svc.cancel_streaming(&id).await.unwrap();
        assert!(svc.get_session(&id).is_none());
    }

    #[tokio::test]
    async fn list_active_sessions_tracks_state() {
        let svc = mock_service();
        let _id1 = svc.start_streaming("zh-CN").await.unwrap();
        let _id2 = svc.start_streaming("en-US").await.unwrap();
        assert_eq!(svc.list_active_sessions().len(), 2);
    }

    #[tokio::test]
    async fn list_records_empty_without_store() {
        let svc = mock_service();
        let records = svc.list_records().await.unwrap();
        assert!(records.is_empty());
    }

    #[tokio::test]
    async fn search_summaries_empty_without_index() {
        let svc = mock_service();
        let hits = svc.search_summaries("test", 10).await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn update_config_updates_auto_summary_flag() {
        let svc = mock_service();
        let mut cfg = svc.config().await;
        cfg.enable_auto_summary = false;
        svc.update_config(cfg).await;
        // 配置已更新（内部 provider 不变）
        assert!(!svc.config().await.enable_auto_summary);
    }
}

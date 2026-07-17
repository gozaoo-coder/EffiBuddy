//! OpenAI 兼容嵌入向量提供者 + 磁盘缓存
//!
//! 实现 [`effisuite_core::EmbeddingProvider`] trait，通过 POST `/embeddings`
//! 接口（OpenAI / DeepSeek / Groq 等所有 OpenAI 兼容服务均支持）批量计算向量。
//!
//! # 缓存策略
//!
//! - 内存缓存：`HashMap<String, Vec<f32>>`，key = `<model>:<fnv1a_64(content)>`
//!   用 FNV-1a 64 位哈希避免长文本作为 key 的内存浪费，且 FNV 跨版本稳定。
//! - 磁盘缓存：`<path>.json`，构造时加载，每批新向量计算后异步 spawn 保存
//!   （非阻塞，best-effort；失败仅记录日志）。
//!
//! # 并发与性能（对齐 user_rules）
//!
//! - 内存缓存用 `std::sync::RwLock`：读多写少，查询路径锁内仅 HashMap 查找。
//! - HTTP 调用在锁外完成，批量发送减少 RTT。
//! - `reqwest::Client` 内部已是连接池化的 Arc 句柄，clone 廉价。
//! - 使用 `with_capacity` 预分配批量请求 payload。
//!
//! # 错误处理
//!
//! - 网络错误 / 非法响应：以 `CoreError::Agent` 上抛，调用方可降级到纯词法检索
//! - 磁盘缓存加载失败：警告日志后继续（不影响功能，仅丢失缓存）

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use effisuite_core::{CoreError, EmbeddingProvider, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// 默认嵌入模型（OpenAI text-embedding-3-small，性价比高、维度 1536）
pub const DEFAULT_EMBEDDING_MODEL: &str = "text-embedding-3-small";

/// OpenAI 兼容嵌入向量提供者
///
/// 持有：
/// - HTTP client（连接池化）
/// - API key / base_url / model_name
/// - 内存 + 磁盘缓存
pub struct OpenAIEmbeddingProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model_name: String,
    /// 内存缓存；用 RwLock 允许多读
    cache: Arc<RwLock<HashMap<String, Vec<f32>>>>,
    /// 磁盘缓存路径；None 则不持久化
    cache_path: Option<PathBuf>,
}

impl OpenAIEmbeddingProvider {
    /// 构造 provider 并加载磁盘缓存（若 path 存在）
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model_name: impl Into<String>,
        cache_path: Option<PathBuf>,
    ) -> Self {
        let api_key = api_key.into();
        let base_url = base_url.into();
        let model_name = model_name.into();
        let cache_path_cloned = cache_path.clone();
        let cache = load_cache_blocking(cache_path_cloned.as_deref(), &model_name);

        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url,
            model_name,
            cache: Arc::new(RwLock::new(cache)),
            cache_path,
        }
    }

    /// 是否启用磁盘持久化
    pub fn has_disk_cache(&self) -> bool {
        self.cache_path.is_some()
    }

    /// 异步保存内存缓存到磁盘（spawn 独立 task，best-effort）
    fn spawn_save_cache(&self) {
        let cache = Arc::clone(&self.cache);
        let path = match self.cache_path.clone() {
            Some(p) => p,
            None => return,
        };
        let model = self.model_name.clone();
        tokio::spawn(async move {
            let snapshot = cache.read().await.clone();
            if let Err(e) = save_cache_async(&path, &model, &snapshot).await {
                tracing::warn!(error = %e, "保存 embedding 缓存失败");
            }
        });
    }

    /// 计算 cache key：`<model>:<fnv1a_64(content)>`
    fn cache_key(model: &str, content: &str) -> String {
        format!("{}:{:x}", model, fnv1a_64(content))
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAIEmbeddingProvider {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // 1. 读缓存，找出未命中的下标
        let mut cache = self.cache.read().await;
        let mut results: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
        let mut misses: Vec<(usize, String)> = Vec::with_capacity(texts.len());
        for (i, t) in texts.iter().enumerate() {
            let key = Self::cache_key(&self.model_name, t);
            if let Some(v) = cache.get(&key) {
                results[i] = Some(v.clone());
            } else {
                misses.push((i, (*t).to_string()));
            }
        }
        drop(cache);

        // 2. 全部命中则直接返回
        if misses.is_empty() {
            return Ok(results.into_iter().map(|o| o.unwrap_or_default()).collect());
        }

        // 3. 批量调用 /embeddings 接口
        let batch_texts: Vec<&str> = misses.iter().map(|(_, t)| t.as_str()).collect();
        let new_embeddings = call_embeddings_api(
            &self.client,
            &self.api_key,
            &self.base_url,
            &self.model_name,
            &batch_texts,
        )
        .await?;

        if new_embeddings.len() != misses.len() {
            return Err(CoreError::Agent(format!(
                "embeddings 数量不匹配：期望 {}，实际 {}",
                misses.len(),
                new_embeddings.len()
            )));
        }

        // 4. 写回内存缓存并触发异步落盘
        let mut cache = self.cache.write().await;
        for ((_, text), emb) in misses.iter().zip(new_embeddings.iter()) {
            let key = Self::cache_key(&self.model_name, text);
            cache.insert(key, emb.clone());
        }
        drop(cache);
        self.spawn_save_cache();

        // 5. 合并结果（保持输入顺序）
        let mut emb_iter = new_embeddings.into_iter();
        for (i, _) in misses {
            results[i] = Some(emb_iter.next().unwrap_or_default());
        }
        Ok(results.into_iter().map(|o| o.unwrap_or_default()).collect())
    }
}

// =========================================================
// HTTP 调用
// =========================================================

/// OpenAI /embeddings 请求体
#[derive(Serialize)]
struct EmbeddingsRequest<'a> {
    model: &'a str,
    input: &'a [&'a str],
}

/// OpenAI /embeddings 响应体（仅取所需字段）
#[derive(Deserialize)]
struct EmbeddingsResponse {
    data: Vec<EmbeddingItem>,
}

#[derive(Deserialize)]
struct EmbeddingItem {
    embedding: Vec<f32>,
    #[allow(dead_code)]
    index: usize,
}

/// 调用 OpenAI 兼容 /embeddings 接口
async fn call_embeddings_api(
    client: &reqwest::Client,
    api_key: &str,
    base_url: &str,
    model: &str,
    texts: &[&str],
) -> Result<Vec<Vec<f32>>> {
    let url = format!(
        "{}/embeddings",
        base_url.trim_end_matches('/')
    );
    let req = EmbeddingsRequest { model, input: texts };
    let resp = client
        .post(&url)
        .bearer_auth(api_key)
        .json(&req)
        .send()
        .await
        .map_err(|e| CoreError::Agent(format!("embeddings request: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(CoreError::Agent(format!(
            "embeddings API 返回 {status}: {}",
            truncate_body(&body, 200)
        )));
    }

    let parsed: EmbeddingsResponse = resp
        .json()
        .await
        .map_err(|e| CoreError::Agent(format!("embeddings response parse: {e}")))?;

    // 按 index 排序确保顺序与输入一致
    let mut items = parsed.data;
    items.sort_by_key(|i| i.index);
    Ok(items.into_iter().map(|i| i.embedding).collect())
}

/// 截断错误响应体用于日志
fn truncate_body(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let boundary = s.ceil_char_boundary(max);
    format!("{}…", &s[..boundary])
}

// =========================================================
// 缓存 IO
// =========================================================

/// 磁盘缓存文件结构：包裹一层 model 字段，便于校验/未来迁移
#[derive(Serialize, Deserialize, Default)]
struct CacheFile {
    /// 缓存生成时所用的 embedding 模型名
    model: String,
    /// 嵌入向量映射表：key = `<model>:<fnv1a_64(content)>`
    entries: HashMap<String, Vec<f32>>,
}

/// 同步加载磁盘缓存（在构造时调用，避免 async 构造函数）
///
/// 若文件不存在或解析失败，返回空 HashMap 并记录警告。
/// 若文件中的 model 与当前 model 不匹配，也返回空（避免维度错配）。
fn load_cache_blocking(path: Option<&Path>, expected_model: &str) -> HashMap<String, Vec<f32>> {
    let path = match path {
        Some(p) => p,
        None => return HashMap::new(),
    };
    if !path.exists() {
        return HashMap::new();
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "读取 embedding 缓存失败");
            return HashMap::new();
        }
    };
    let parsed: CacheFile = match serde_json::from_slice(&bytes) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "解析 embedding 缓存失败，将重建");
            return HashMap::new();
        }
    };
    if parsed.model != expected_model {
        tracing::info!(
            cached_model = %parsed.model,
            current_model = %expected_model,
            "embedding 缓存 model 不匹配，丢弃旧缓存"
        );
        return HashMap::new();
    }
    parsed.entries
}

/// 异步保存缓存到磁盘
async fn save_cache_async(path: &Path, model: &str, entries: &HashMap<String, Vec<f32>>) -> Result<()> {
    let file = CacheFile {
        model: model.to_string(),
        entries: entries.clone(),
    };
    let bytes = serde_json::to_vec(&file).map_err(CoreError::Serde)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(CoreError::Io)?;
    }
    tokio::fs::write(path, bytes).await.map_err(CoreError::Io)?;
    Ok(())
}

// =========================================================
// FNV-1a 64 位哈希（稳定跨 Rust 版本，用于缓存键）
// =========================================================

#[inline]
fn fnv1a_64(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_is_stable() {
        // 锁定 FNV-1a 已知值，确保跨版本稳定
        assert_eq!(fnv1a_64(""), 0xcbf29ce484222325);
        assert_eq!(fnv1a_64("a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a_64("hello"), 0xa430d84680aabd0b);
    }

    #[test]
    fn cache_key_includes_model() {
        let k1 = OpenAIEmbeddingProvider::cache_key("model-a", "hello");
        let k2 = OpenAIEmbeddingProvider::cache_key("model-b", "hello");
        assert_ne!(k1, k2);
    }

    #[tokio::test]
    async fn embed_empty_input_returns_empty() {
        let provider = OpenAIEmbeddingProvider::new("k", "https://example.com", "m", None);
        let result = provider.embed(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn load_cache_returns_empty_when_model_mismatch() {
        let dir = std::env::temp_dir().join(format!("emb-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cache.json");
        let file = CacheFile {
            model: "old-model".to_string(),
            entries: {
                let mut m = HashMap::new();
                m.insert("old-model:abc".to_string(), vec![0.1, 0.2]);
                m
            },
        };
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        let loaded = load_cache_blocking(Some(&path), "new-model");
        assert!(loaded.is_empty(), "model 不匹配时应返回空");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_cache_returns_entries_when_model_matches() {
        let dir = std::env::temp_dir().join(format!("emb-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cache.json");
        let mut entries = HashMap::new();
        entries.insert("model-x:abc".to_string(), vec![0.1, 0.2, 0.3]);
        let file = CacheFile {
            model: "model-x".to_string(),
            entries,
        };
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        let loaded = load_cache_blocking(Some(&path), "model-x");
        assert_eq!(loaded.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_cache_missing_file_returns_empty() {
        let path = std::path::PathBuf::from("/nonexistent/path/cache.json");
        let loaded = load_cache_blocking(Some(&path), "any");
        assert!(loaded.is_empty());
    }

    #[test]
    fn truncate_body_short_strings_unchanged() {
        assert_eq!(truncate_body("hi", 200), "hi");
        let long = "a".repeat(300);
        let t = truncate_body(&long, 10);
        assert!(t.ends_with('…'));
        assert!(t.len() <= 12);
    }
}

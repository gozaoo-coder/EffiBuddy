//! RigAgent：通过 [rig](https://crates.io/crates/rig-core) 调用 OpenAI 兼容接口
//!
//! 真正接入 rig 的实现。当 `OPENAI_API_KEY` 环境变量存在时，由
//! [`crate::ChatAgent`] 的调用方择机构造 `RigAgent` 替换 `MockAgent`。
//!
//! 设计要点（对齐 user_rules 中的 Rust 性能/并发规则）：
//! - `openai::Client` 内部已是 `Arc` 共享句柄，`RigAgent` 直接持有而**不**再包
//!   `Arc<Mutex<...>>`，避免双重锁开销。
//! - 每次 `chat` 构建一次 `Agent` 是 rig 推荐用法（builder 零成本），不缓存
//!   带状态的 agent，天然支持并发请求。
//! - 错误以字符串向上传递，符合 core 的 `CoreError::Agent` 约定。

use async_trait::async_trait;
use effisuite_core::{CoreError, Message, Result, Role};
use rig_core::{
    client::{CompletionClient, ProviderClient},
    completion::Prompt,
    providers::openai,
};

use crate::agent::ChatAgent;

pub struct RigAgent {
    client: openai::Client,
    model_name: String,
    preamble: String,
}

impl RigAgent {
    /// 从环境变量 `OPENAI_API_KEY` 构造 OpenAI 客户端。
    pub fn from_env(model_name: impl Into<String>, preamble: impl Into<String>) -> Result<Self> {
        let client = openai::Client::from_env()
            .map_err(|e| CoreError::Agent(format!("openai client init: {e}")))?;
        Ok(Self {
            client,
            model_name: model_name.into(),
            preamble: preamble.into(),
        })
    }

    /// 指定 API key 构造客户端（用于 OpenAI 兼容服务）
    ///
    /// rig 0.40 的 `Client::new` 返回 `Result`，需处理错误。
    pub fn from_key(
        api_key: impl Into<String>,
        model_name: impl Into<String>,
        preamble: impl Into<String>,
    ) -> Result<Self> {
        let client = openai::Client::new(api_key.into())
            .map_err(|e| CoreError::Agent(format!("openai client init: {e}")))?;
        Ok(Self {
            client,
            model_name: model_name.into(),
            preamble: preamble.into(),
        })
    }
}

#[async_trait]
impl ChatAgent for RigAgent {
    async fn chat(&self, messages: &[Message]) -> Result<String> {
        let agent = self
            .client
            .agent(&self.model_name)
            .preamble(&self.preamble)
            .build();

        // 取最后一条用户消息作为本轮 prompt；用迭代器而非索引循环
        let prompt = messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_str())
            .unwrap_or("hello");

        let resp = agent
            .prompt(prompt)
            .await
            .map_err(|e| CoreError::Agent(format!("rig prompt: {e}")))?;

        Ok(resp)
    }

    #[inline]
    fn name(&self) -> &str {
        &self.model_name
    }

    #[inline]
    fn backend(&self) -> &'static str {
        "rig-openai"
    }
}

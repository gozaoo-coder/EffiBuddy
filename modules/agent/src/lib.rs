//! EffiSuite agent 模块
//!
//! 基于 [rig](https://crates.io/crates/rig-core) 提供 LLM 对话能力。
//! 通过 `ChatAgent` trait 抽象具体后端，业务层依赖 trait 而非具体实现，
//! 便于在 mock / OpenAI / Ollama 等后端间无成本切换（零成本抽象 + 类型驱动）。
//!
//! 当前最小版本提供：
//! - [`MockAgent`]：纯本地回显，无网络依赖，用于离线开发与单测
//! - [`RigAgent`]：通过 rig 调用 OpenAI 兼容接口，需要 `OPENAI_API_KEY`

pub mod agent;
pub mod mock;
pub mod rig_agent;

pub use agent::ChatAgent;
pub use mock::MockAgent;
pub use rig_agent::RigAgent;

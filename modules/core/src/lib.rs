//! EffiSuite 核心模块
//!
//! 提供跨模块共享的数据结构与错误类型，是 agent / p2pConnection / tauriFront
//! 之间的"通用语言"。本模块刻意保持零业务逻辑、零外部 IO 依赖（除 tokio），
//! 以确保编译迅速、可被任何模块以最低成本引入。

pub mod config;
pub mod error;
pub mod events;
pub mod models;
pub mod storage;

pub use config::{
    AgentConfig, AvailableModel, BackendKind, ProviderPreset, ThemeMode, builtin_presets,
};
pub use error::{CoreError, Result};
pub use events::{BusEvent, EventBus};
pub use models::{Conversation, Device, DeviceStatus, Message, Role};
pub use storage::ConversationStore;

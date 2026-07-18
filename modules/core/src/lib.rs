//! EffiSuite 核心模块
//!
//! 提供跨模块共享的数据结构与错误类型，是 agent / p2pConnection / tauriFront
//! 之间的"通用语言"。本模块刻意保持零业务逻辑、零外部 IO 依赖（除 tokio），
//! 以确保编译迅速、可被任何模块以最低成本引入。
//!
//! ClawHub 客户端（`clawhub`）是该规则的例外：它需要 reqwest + zip 来访问
//! ClawHub HTTP API 并解压安装包。把它放在 core 是为了让 tauriFront 命令层
//! 直接复用，避免在 agent crate 中引入额外耦合。

pub mod clawhub;
pub mod config;
pub mod error;
pub mod events;
pub mod memory;
pub mod models;
pub mod pinned_memory;
pub mod plugin_store;
pub mod schedule_store;
pub mod skill_index;
pub mod skill_store;
pub mod storage;

pub use config::{
    AgentConfig, AvailableModel, BackendKind, ModelKind, ProviderPreset, ThemeMode,
    builtin_presets,
};
pub use error::{CoreError, Result};
pub use events::{BusEvent, EventBus};
pub use clawhub::ClawHubClient;
pub use memory::{
    EmbeddingProvider, MemoryEntry, MemoryHit, MemoryIndex, MemoryStats, SearchMode, tokenize,
};
pub use models::{
    Attachment, AttachmentKind, Conversation, Device, DeviceStatus, InstalledPlugin, Message,
    Role, ScheduledTask, Skill,
};
pub use pinned_memory::{PinnedMemory, PinnedMemorySource, PinnedMemoryStore};
pub use plugin_store::PluginStore;
pub use schedule_store::ScheduledTaskStore;
pub use skill_index::{SkillEntry, SkillHit, SkillIndex};
pub use skill_store::SkillStore;
pub use storage::{ConversationMeta, ConversationStore, SearchHit};

//! EffiSuite 核心模块
//!
//! 提供跨模块共享的数据结构与错误类型，是 agent / p2pConnection / tauriFront
//! 之间的"通用语言"。本模块刻意保持零业务逻辑、零外部 IO 依赖（除 tokio），
//! 以确保编译迅速、可被任何模块以最低成本引入。

pub mod config;
pub mod error;
pub mod events;
pub mod memory;
pub mod models;
pub mod pinned_memory;
pub mod schedule_store;
pub mod skill_store;
pub mod storage;

pub use config::{
    AgentConfig, AvailableModel, BackendKind, ProviderPreset, ThemeMode, builtin_presets,
};
pub use error::{CoreError, Result};
pub use events::{BusEvent, EventBus};
pub use memory::{
    EmbeddingProvider, MemoryEntry, MemoryHit, MemoryIndex, MemoryStats, SearchMode, tokenize,
};
pub use models::{
    Attachment, AttachmentKind, Conversation, Device, DeviceStatus, Message, Role, ScheduledTask,
    Skill,
};
pub use pinned_memory::{PinnedMemory, PinnedMemorySource, PinnedMemoryStore};
pub use schedule_store::ScheduledTaskStore;
pub use skill_store::SkillStore;
pub use storage::{ConversationMeta, ConversationStore, SearchHit};

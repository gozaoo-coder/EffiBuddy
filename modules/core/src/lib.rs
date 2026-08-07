//! EffiSuite 核心模块
//!
//! 提供跨模块共享的数据结构与错误类型，是 agent / p2pConnection / tauriFront
//! 之间的"通用语言"。本模块刻意保持零业务逻辑、零外部 IO 依赖（除 tokio），
//! 以确保编译迅速、可被任何模块以最低成本引入。
//!
//! ClawHub 客户端（`clawhub`）是该规则的例外：它需要 reqwest + zip 来访问
//! ClawHub HTTP API 并解压安装包。把它放在 core 是为了让 tauriFront 命令层
//! 直接复用，避免在 agent crate 中引入额外耦合。

pub mod asr;
pub mod clawhub;
pub mod compression;
pub mod config;
pub mod error;
pub mod events;
pub mod external_skills;
pub mod favorite_workspace;
pub mod memory;
pub mod models;
pub mod paths;
pub mod pinned_memory;
pub mod plugin_config;
pub mod plugin_manifest;
pub mod plugin_store;
pub mod remote_task;
pub mod schedule_store;
pub mod skill_index;
pub mod skill_store;
pub mod storage;
pub mod tokens;
pub mod versions;

pub use asr::{
    AsrRecord, AsrSearchQuery, AsrSource, AsrStatus, AsrStore, AsrSummaryHit, AsrSummaryIndex,
};
pub use clawhub::ClawHubClient;
pub use compression::{
    apply_compression, build_compression_prompt, build_compression_prompt_with_settings,
    parse_compression_response, CompressionAction, CompressionState, CompressionStore,
};
pub use config::{
    builtin_presets, AgentConfig, AsrConfig, AsrProvider, AvailableModel, BackendKind,
    CompressionSettings, ModelKind, ModelPricing, ProviderPreset, ThemeMode,
};
pub use error::{CoreError, Result};
pub use events::{BusEvent, EventBus};
pub use external_skills::scan_external_skills;
pub use favorite_workspace::{FavoriteWorkspace, FavoriteWorkspaceStore};
pub use memory::{
    make_snippet, tokenize, EmbeddingProvider, MemoryEntry, MemoryHit, MemoryIndex, MemoryStats,
    SearchMode, SNIPPET_MAX_CHARS,
};
pub use models::{
    gen_message_id, Attachment, AttachmentKind, Conversation, Device, DeviceStatus, InstalledPlugin,
    Message, MessageUsage, Role, ScheduledTask, Skill, SubAgentImage, SubAgentRecord,
    ToolCallRecord,
};
pub use pinned_memory::{PinnedMemory, PinnedMemorySource, PinnedMemoryStore};
pub use plugin_config::PluginConfigStore;
pub use plugin_manifest::{
    build_contribution_set, builtin_contributions, load_manifest, safe_plugin_path_segment,
    safe_install_dir, PluginCommandContribution, PluginContributionSet, PluginContributions,
    PluginManifest, PluginPageContribution, PluginRailAction, PluginRailActionKind,
    PluginRailContribution, KNOWN_PERMISSIONS, MANIFEST_API_VERSION, MANIFEST_NAMES,
};
pub use plugin_store::PluginStore;
pub use remote_task::{remote_task_now, RemoteTaskDispatcher};
pub use schedule_store::ScheduledTaskStore;
pub use skill_index::{SkillEntry, SkillHit, SkillIndex};
pub use skill_store::SkillStore;
pub use storage::{ConversationMeta, ConversationStore, SearchHit};
pub use tokens::last_reported_input_tokens;
pub use versions::{
    Commit, CommitKind, CommitSummary, RefKind, RefSummary, VersionList, VersionOpResult,
    VersionRepo, VersionStore,
};

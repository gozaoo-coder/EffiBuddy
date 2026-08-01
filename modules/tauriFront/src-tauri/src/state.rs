//! 应用全局状态定义与全局工具函数。
//!
//! `AppState` 字段按大小降序排列，最小化结构体 padding；
//! `now_ms` 是全局 Unix 毫秒时间戳工具函数，被多个模块复用。

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use effisuite_agent::{ChatAgent, AsrService, ImageGenConfig, ModelManagerHandle, SubAgentManager};
use effisuite_core::clawhub::ClawHubClient;
use effisuite_core::{
    AgentConfig, CompressionStore, ConversationStore, EventBus, MemoryIndex, PinnedMemoryStore,
    PluginStore, ScheduledTaskStore, SkillStore, SubAgentRecord,
};
use effisuite_p2p::P2pManager;
use tokio::sync::RwLock;

/// 应用全局状态，由 `tauri::Builder::manage` 注入。
///
/// 字段按大小降序：`SkillStore`/`ScheduledTaskStore`（PathBuf+Arc，4 usize）在前，
/// `Arc<RwLock<...>>` / `Arc<...>`（1 usize）居中，
/// `Option<JoinHandle>`（1 usize）在后。
pub struct AppState {
    /// 技能存储（PathBuf+Arc，4 usize）
    pub skill_store: SkillStore,
    /// 已安装技能 RAG 检索索引（BM25），与 agent 共享同一份 Arc
    /// 启动时从 skill_store 全量重建；技能增删后由对应命令 rebuild
    pub skill_index: Arc<effisuite_core::SkillIndex>,
    /// 定时任务存储（PathBuf+Arc，4 usize）
    pub schedule_store: ScheduledTaskStore,
    /// 已安装插件存储（PathBuf+Arc，4 usize）
    pub plugin_store: PluginStore,
    /// 消息压缩状态存储（PathBuf+Arc，4 usize），与 agent 共享同一份 Arc
    /// 存放每会话的 Keep/Hide/Replace 决策，build_context_parts 据此压缩历史段
    pub compression_store: CompressionStore,
    /// ClawHub 客户端（Clone 廉价，内部 Arc<reqwest::Client>）
    pub clawhub: ClawHubClient,
    /// 可热替换的 agent：RwLock 写少读多，内层 Arc 让 async 命令可跨 await 持有
    pub agent: Arc<RwLock<Arc<dyn ChatAgent>>>,
    pub store: Arc<ConversationStore>,
    /// 配置快照：`Arc<RwLock<Arc<AgentConfig>>>` 让读操作 clone Arc（廉价），
    /// 写操作 clone 内部 AgentConfig → 修改 → 写回新 Arc（COW 语义）。
    /// 消除 read().await.clone() 的深拷贝（5 处读路径受益）。
    pub config: Arc<RwLock<Arc<AgentConfig>>>,
    pub p2p: Arc<P2pManager>,
    pub event_bus: EventBus,
    /// 跨会话历史记忆索引（RAG 记忆增强核心），与 agent 共享同一份 Arc
    pub memory: Arc<MemoryIndex>,
    /// 永久记忆存储（用户主动要求"记住"的内容），与 agent 共享同一份 Arc
    pub pinned_memory: Arc<PinnedMemoryStore>,
    /// 当前活跃会话 id，由 send_message 命令更新；agent 据此排除当前会话
    pub current_conversation_id: Arc<RwLock<Option<String>>>,
    /// 当前工作区路径，由 send_message 命令更新；agent 据此解析相对路径与设置 shell cwd。
    /// 优先级：会话级 working_dir > 技能级 working_dir > 进程默认 cwd。
    pub working_dir: Arc<RwLock<Option<std::path::PathBuf>>>,
    /// 图像生成模型配置句柄：set_image_gen_model 时更新，build_agent 注入到 ImageGenTool。
    /// None 表示未配置图像生成能力，image_gen 工具调用会返回错误提示。
    pub image_gen_config: Arc<RwLock<Option<ImageGenConfig>>>,
    /// 附件存储目录（绝对路径），ImageGenTool 据此落盘生成图片。
    pub attachments_dir: std::path::PathBuf,
    /// cron 调度器 task 句柄（setup 中 spawn 后写入；shutdown 时可 abort）。
    /// 用 `Mutex` 是因为句柄在 setup 阶段才产生，需运行时回填到已 manage 的 state。
    pub scheduler_handle: std::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// 配置版本号：agent 工具（manage_model）每次修改配置 +1；
    /// 与 agent_rev 比对，send_message 时懒重建 agent。
    pub config_rev: Arc<std::sync::atomic::AtomicU64>,
    /// 当前 agent 所基于的配置版本号。config_rev != agent_rev 时触发重建。
    pub agent_rev: Arc<std::sync::atomic::AtomicU64>,
    /// 模型管理句柄：注入 agent，manage_model / call_model / sub_agent 工具据此读写模型列表
    pub model_manager: Arc<ModelManagerHandle>,
    /// 子 agent 管理器：注入 agent，sub_agent 工具据此召唤子 agent；事件经回调 emit 到前端
    pub sub_agents: Arc<SubAgentManager>,
    /// 子 agent 事件累积缓冲：key = 主会话 conversation_id，value = 该会话当前
    /// 流式回复中子 agent 的过程记录（emitter 实时累积，流结束时持久化到消息）。
    pub sub_agent_records:
        Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<SubAgentRecord>>>>,
    /// ASR 语音转写服务句柄（火山引擎/千问），注入 agent 启用 ASR 工具，
    /// 亦供 Tauri 命令层直接调用流式录音 API。
    pub asr_service: Arc<AsrService>,
}

/// 当前 Unix 毫秒时间戳；失败时回退为 0，避免在命令路径里 panic。
#[inline]
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

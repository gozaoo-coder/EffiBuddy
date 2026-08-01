//! `RigAgent` 的构造与配置链
//!
//! 包含两个构造入口：
//! - [`RigAgent::from_env`]：从环境变量 `OPENAI_API_KEY` 构造客户端
//! - [`RigAgent::from_key`]：显式传入 `api_key` / `base_url`，覆盖环境变量
//!
//! 以及一组 `with_*` 链式配置方法，按需注入可选能力（事件总线、定时任务、
//! 网络 / 视频生成配置、待办列表状态等）和 `_handle` 句柄访问器。

use std::path::PathBuf;
use std::sync::Arc;

use effisuite_core::{
    ClawHubClient, CompressionStore, ConversationStore, CoreError, EventBus, MemoryIndex,
    PinnedMemoryStore, PluginStore, Result, ScheduledTaskStore, SkillIndex, SkillStore,
};
use rig_core::client::ProviderClient;
use rig_core::providers::openai;
use tokio::sync::RwLock;

use crate::tools::{
    ImageGenConfig, ModelManagerHandle, SubAgentManager, TodoItem, VideoGenConfig, WebSearchConfig,
};

use super::RigAgent;

impl RigAgent {
    /// 从环境变量 `OPENAI_API_KEY` 构造 OpenAI 客户端
    ///
    /// `memory` 与 `current_conversation_id` 用于 RAG 记忆增强；传 None / 默认句柄
    /// 则关闭记忆增强能力（行为同旧版）。`pinned_memory` 用于永久记忆注入。
    /// `working_dir` 共享句柄用于工作区路径注入，传新的空句柄则默认不限制。
    /// `image_gen_config` 共享句柄供 ImageGenTool 读取，None 则 image_gen 工具不可用。
    /// `attachments_dir` 为图片落盘目录（绝对路径）。
    /// `store` 共享会话存储句柄，SetTitleTool 据此持久化标题。
    /// `skill_index` / `skill_store` / `clawhub_client` / `skills_dir` 注入后
    /// 启用技能 RAG 自动注入与 5 个技能管理工具；任一为 None 则相应能力降级。
    /// `plugin_store` 注入后启用 uninstall_plugin 工具。
    /// `compression_store` 注入后启用消息压缩（build_context_parts 对历史段应用
    /// Keep/Hide/Replace 决策）；None 时退化为不压缩。
    #[allow(clippy::too_many_arguments)]
    pub fn from_env(
        model_name: impl Into<String>,
        preamble: impl Into<String>,
        enable_tools: bool,
        memory: Option<Arc<MemoryIndex>>,
        pinned_memory: Option<Arc<PinnedMemoryStore>>,
        current_conversation_id: Arc<RwLock<Option<String>>>,
        working_dir: Arc<RwLock<Option<PathBuf>>>,
        image_gen_config: Arc<RwLock<Option<ImageGenConfig>>>,
        attachments_dir: PathBuf,
        store: Arc<ConversationStore>,
        skill_index: Option<Arc<SkillIndex>>,
        skill_store: Option<SkillStore>,
        clawhub_client: Option<ClawHubClient>,
        skills_dir: Option<PathBuf>,
        plugin_store: Option<PluginStore>,
        compression_store: Option<CompressionStore>,
        model_manager: Option<Arc<ModelManagerHandle>>,
        sub_agents: Option<Arc<SubAgentManager>>,
    ) -> Result<Self> {
        let client = openai::CompletionsClient::from_env()
            .map_err(|e| CoreError::Agent(format!("openai completions client init: {e}")))?;
        Ok(Self {
            client,
            model_name: model_name.into(),
            preamble: preamble.into(),
            history: Arc::new(RwLock::new(Vec::new())),
            enable_tools,
            memory,
            pinned_memory,
            skill_index,
            skill_store,
            clawhub_client,
            skills_dir,
            plugin_store,
            current_conversation_id,
            working_dir,
            image_gen_config,
            attachments_dir,
            store,
            compression_store,
            model_manager,
            sub_agents,
            tool_allowlist: None,
            exclude_tools: Vec::new(),
            event_bus: None,
            scheduled_task_store: None,
            web_search_config: Arc::new(RwLock::new(None)),
            video_gen_config: Arc::new(RwLock::new(None)),
            todo_state: None,
            asr_service: None,
        })
    }

    /// 指定工具白名单：None = 全部工具；Some = 仅注册列表中的工具
    pub fn with_tool_allowlist(mut self, tools: Option<Vec<String>>) -> Self {
        self.tool_allowlist = tools;
        self
    }

    /// 排除指定工具（优先级高于白名单）
    pub fn with_excluded_tools(mut self, tools: Vec<String>) -> Self {
        self.exclude_tools = tools;
        self
    }

    /// 注入事件总线句柄，启用 ask_user / notify_user / open_preview 交互工具
    pub fn with_event_bus(mut self, bus: Option<Arc<EventBus>>) -> Self {
        self.event_bus = bus;
        self
    }

    /// 注入定时任务存储句柄，启用 schedule 工具
    pub fn with_scheduled_task_store(mut self, store: Option<Arc<ScheduledTaskStore>>) -> Self {
        self.scheduled_task_store = store;
        self
    }

    /// 注入网络搜索配置句柄，启用 web_search 工具
    pub fn with_web_search_config(
        mut self,
        config: Option<Arc<RwLock<Option<WebSearchConfig>>>>,
    ) -> Self {
        if let Some(c) = config {
            self.web_search_config = c;
        }
        self
    }

    /// 注入视频生成配置句柄，启用 generate_video 工具
    pub fn with_video_gen_config(
        mut self,
        config: Option<Arc<RwLock<Option<VideoGenConfig>>>>,
    ) -> Self {
        if let Some(c) = config {
            self.video_gen_config = c;
        }
        self
    }

    /// 注入待办列表共享状态，启用 todo_write 工具
    pub fn with_todo_state(mut self, state: Option<Arc<RwLock<Vec<TodoItem>>>>) -> Self {
        self.todo_state = state;
        self
    }

    /// 注入 ASR 语音转写服务句柄，启用 transcribe_audio / search_asr_records /
    /// list_asr_records / get_asr_record 工具
    pub fn with_asr_service(mut self, service: Option<Arc<crate::asr::AsrService>>) -> Self {
        self.asr_service = service;
        self
    }

    /// 共享 web_search_config 句柄
    #[inline]
    pub fn web_search_config_handle(&self) -> Arc<RwLock<Option<WebSearchConfig>>> {
        Arc::clone(&self.web_search_config)
    }

    /// 共享 video_gen_config 句柄
    #[inline]
    pub fn video_gen_config_handle(&self) -> Arc<RwLock<Option<VideoGenConfig>>> {
        Arc::clone(&self.video_gen_config)
    }
    ///
    /// - `api_key`：Bearer token，空串会被 rig 拒绝
    /// - `base_url`：可覆盖默认 `https://api.openai.com/v1`，留空则用默认
    /// - 走 Chat Completions API（`openai::CompletionsClient`）
    /// - `memory` 注入后启用 RAG 记忆增强（自动上文 + search_memory 工具）
    /// - `pinned_memory` 注入后启用永久记忆（每轮注入 `[永久记忆]` 段 + pin_memory 工具）
    /// - `working_dir` 共享句柄用于工作区路径注入
    /// - `image_gen_config` 共享句柄供 ImageGenTool 读取（用户切换到 image_gen 模型时由
    ///   Tauri 命令层更新），None 时 image_gen 工具调用返回错误提示
    /// - `attachments_dir` 为图片落盘目录（绝对路径），ImageGenTool 据此保存生成结果
    /// - `store` 共享会话存储句柄，SetTitleTool / EnableSkillTool 据此读写会话
    /// - `skill_index` / `skill_store` / `clawhub_client` / `skills_dir` 注入后
    ///   启用技能 RAG 自动注入与 5 个技能管理工具；任一为 None 则相应能力降级
    /// - `plugin_store` 注入后启用 uninstall_plugin 工具
    /// - `compression_store` 注入后启用消息压缩（build_context_parts 对历史段应用
    ///   Keep/Hide/Replace 决策）；None 时退化为不压缩
    #[allow(clippy::too_many_arguments)]
    pub fn from_key(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model_name: impl Into<String>,
        preamble: impl Into<String>,
        enable_tools: bool,
        memory: Option<Arc<MemoryIndex>>,
        pinned_memory: Option<Arc<PinnedMemoryStore>>,
        current_conversation_id: Arc<RwLock<Option<String>>>,
        working_dir: Arc<RwLock<Option<PathBuf>>>,
        image_gen_config: Arc<RwLock<Option<ImageGenConfig>>>,
        attachments_dir: PathBuf,
        store: Arc<ConversationStore>,
        skill_index: Option<Arc<SkillIndex>>,
        skill_store: Option<SkillStore>,
        clawhub_client: Option<ClawHubClient>,
        skills_dir: Option<PathBuf>,
        plugin_store: Option<PluginStore>,
        compression_store: Option<CompressionStore>,
        model_manager: Option<Arc<ModelManagerHandle>>,
        sub_agents: Option<Arc<SubAgentManager>>,
    ) -> Result<Self> {
        let api_key = api_key.into();
        let base_url = base_url.into();
        let mut builder = openai::CompletionsClient::builder().api_key(&api_key);
        if !base_url.trim().is_empty() {
            builder = builder.base_url(&base_url);
        }
        let client = builder
            .build()
            .map_err(|e| CoreError::Agent(format!("openai completions client init: {e}")))?;
        Ok(Self {
            client,
            model_name: model_name.into(),
            preamble: preamble.into(),
            history: Arc::new(RwLock::new(Vec::new())),
            enable_tools,
            memory,
            pinned_memory,
            skill_index,
            skill_store,
            clawhub_client,
            skills_dir,
            plugin_store,
            current_conversation_id,
            working_dir,
            image_gen_config,
            attachments_dir,
            store,
            compression_store,
            model_manager,
            sub_agents,
            tool_allowlist: None,
            exclude_tools: Vec::new(),
            event_bus: None,
            scheduled_task_store: None,
            web_search_config: Arc::new(RwLock::new(None)),
            video_gen_config: Arc::new(RwLock::new(None)),
            todo_state: None,
            asr_service: None,
        })
    }

    /// 共享 image_gen_config 句柄，供 Tauri 命令层在 set_active_model 时更新
    #[inline]
    pub fn image_gen_config_handle(&self) -> Arc<RwLock<Option<ImageGenConfig>>> {
        Arc::clone(&self.image_gen_config)
    }

    /// 共享 working_dir 句柄，供 Tauri 命令层在每次 send_message 前更新。
    /// 优先级：会话级 working_dir > 技能级 working_dir > 进程默认 cwd。
    #[inline]
    pub fn working_dir_handle(&self) -> Arc<RwLock<Option<PathBuf>>> {
        Arc::clone(&self.working_dir)
    }

    /// 共享 history 句柄，供外部更新（如新消息到达时 push 进去）
    #[inline]
    pub fn history_handle(&self) -> Arc<RwLock<Vec<effisuite_core::Message>>> {
        Arc::clone(&self.history)
    }
}

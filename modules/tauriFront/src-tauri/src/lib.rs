//! EffiSuite Tauri 应用入口：连接 agent / p2p / core 并暴露给前端
//!
//! 设计要点：
//! - `AppState` 字段按大小降序排列，最小化结构体 padding
//! - `agent` 用 `Arc<RwLock<Arc<dyn ChatAgent>>>` 包装：
//!   - 外层 `RwLock` 允许运行时热替换 agent（写少读多）
//!   - 内层 `Arc<dyn ChatAgent>` 让 async 命令可廉价 clone 后跨 await 持有
//! - 命令层薄封装：转译请求、转发事件，不持长临界区锁
//! - async 命令中先 clone 出 `Arc` 句柄再 `.await`，避免跨 await 持有
//!   `tauri::State` 的借用（Tauri 2.x async command 的 lifetime 约束）
//! - 事件转发通过 broadcast 订阅 + spawn 完成，遵循"消息传递代替共享内存"
//! - 流式命令 spawn 独立 task，逐 token emit "agent-token"，结束 emit "agent-done"
//!
//! 模块结构（详见各模块文档）：
//! - `state`：`AppState` 全局状态定义与 `now_ms` 工具函数
//! - `paths`：appdata 目录与文件路径工具函数
//! - `agent`：agent 构建与同步（懒重建、embedding、memory 重建）
//! - `events`：事件转发与子 agent 事件累积
//! - `config_io`：配置文件持久化
//! - `commands`：全部 Tauri 命令（按功能域拆分为 12 个子模块）
//! - `scheduler`：cron 定时调度器

use std::sync::Arc;

use effisuite_agent::{
    AsrService, ChatAgent, ImageGenConfig, ModelManagerHandle, ShellSessionEvent,
    ShellSessionManager, SubAgentEvent, SubAgentKit, SubAgentManager,
};
use effisuite_agent::todo_store::TodoStore;
use effisuite_core::clawhub::ClawHubClient;
  use effisuite_core::{
      AgentConfig, AsrStore, AsrSummaryIndex, CompressionStore, ConversationStore, EventBus,
      FavoriteWorkspaceStore, MemoryIndex, PinnedMemoryStore, PluginConfigStore, PluginStore,
      ScheduledTaskStore, SkillStore, SubAgentRecord,
  };
use effisuite_p2p::P2pManager;
use tauri::{Emitter, Manager};
use tokio::sync::RwLock;

mod agent;
mod commands;
mod config_io;
mod events;
mod git_service;
mod paths;
mod scheduler;
mod snapshot_service;
mod state;
mod sync_store;

pub use state::AppState;
pub(crate) use state::now_ms;

use agent::{
    apply_embedding_provider, build_agent, rebuild_memory_from_store, resolve_image_gen_config,
    spawn_embedding_computation,
};
use commands::*;
use config_io::{load_config_or_default, save_config};
use events::{accumulate_sub_agent_event, forward_event};
use paths::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // 加载配置
    let config = load_config_or_default();

    // 初始化跨会话记忆索引与当前会话 id 句柄（RAG 记忆增强核心）
    let memory: Arc<MemoryIndex> = Arc::new(MemoryIndex::new());
    let current_conversation_id: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
    // 工作区句柄：send_message 时根据会话级/技能级 working_dir 更新
    let working_dir: Arc<RwLock<Option<std::path::PathBuf>>> = Arc::new(RwLock::new(None));

    // 图像生成配置句柄：从 config 解析 active_image_gen_model_id 对应的配置
    let image_gen_config: Arc<RwLock<Option<ImageGenConfig>>> =
        Arc::new(RwLock::new(resolve_image_gen_config(&config)));
    // 附件目录：ImageGenTool 落盘生成图片到此目录
    let attachments_root = attachments_dir();

    // 初始化永久记忆存储（用户主动要求"记住"的内容）
    let pinned_memory: Arc<PinnedMemoryStore> = match PinnedMemoryStore::new(pinned_memories_path())
    {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!(error = %e, "PinnedMemoryStore 初始化失败，回退到临时目录");
            Arc::new(
                PinnedMemoryStore::new(std::env::temp_dir().join("effisuite-pinned-memories.json"))
                    .expect("临时目录 PinnedMemoryStore 必须成功"),
            )
        }
    };

  // 初始化常用工作区存储（用户在「会话工作区」面板收藏的常用目录）
  let favorite_workspaces: Arc<FavoriteWorkspaceStore> =
      match FavoriteWorkspaceStore::new(favorite_workspaces_path()) {
          Ok(s) => Arc::new(s),
          Err(e) => {
              tracing::error!(error = %e, "FavoriteWorkspaceStore 初始化失败，回退到临时目录");
              Arc::new(
                  FavoriteWorkspaceStore::new(
                      std::env::temp_dir().join("effisuite-favorite-workspaces.json"),
                  )
                  .expect("临时目录 FavoriteWorkspaceStore 必须成功"),
              )
          }
      };
    // 初始化会话存储：SetTitleTool 需要此句柄持久化标题，必须在 build_agent 之前完成
    let store = match ConversationStore::with_versions(conversations_dir()) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!(error = %e, "ConversationStore 初始化失败，回退到临时目录");
            Arc::new(
                ConversationStore::new(std::env::temp_dir().join("effisuite-conversations"))
                    .expect("临时目录 ConversationStore 必须成功"),
            )
        }
    };

    let skill_store = match SkillStore::new(skills_dir()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "SkillStore 初始化失败，回退到临时目录");
            SkillStore::new(std::env::temp_dir().join("effisuite-skills"))
                .expect("临时目录 SkillStore 必须成功")
        }
    };
      let plugin_store = match PluginStore::new(plugins_dir()) {
          Ok(s) => s,
          Err(e) => {
              tracing::error!(error = %e, "PluginStore 初始化失败，回退到临时目录");
              PluginStore::new(std::env::temp_dir().join("effisuite-plugins"))
                  .expect("临时目录 PluginStore 必须成功")
          }
      };
      // 插件配置存储：命名空间隔离，插件请求配置时落盘到 appdata/plugin_configs
      let plugin_config = match PluginConfigStore::new(plugin_configs_dir()) {
          Ok(s) => s,
          Err(e) => {
              tracing::error!(error = %e, "PluginConfigStore 初始化失败，回退到临时目录");
              PluginConfigStore::new(std::env::temp_dir().join("effisuite-plugin-configs"))
                  .expect("临时目录 PluginConfigStore 必须成功")
          }
      };
    // ClawHub 客户端：共享单个 reqwest::Client 连接池
    let clawhub_client = ClawHubClient::new();

    // 消息压缩状态存储：与 agent 共享同一份 Arc，build_context_parts 据此压缩历史段
    let compression_store = match CompressionStore::new(compression_dir()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "CompressionStore 初始化失败，回退到临时目录");
            CompressionStore::new(std::env::temp_dir().join("effisuite-compression"))
                .expect("临时目录 CompressionStore 必须成功")
        }
    };

    // 每会话 todoTree 存储：与 agent 共享同一份 Arc，build_context_parts 每轮注入任务清单
    let todo_store = match TodoStore::new(todo_dir()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "TodoStore 初始化失败，回退到临时目录");
            TodoStore::new(std::env::temp_dir().join("effisuite-todos"))
                .expect("临时目录 TodoStore 必须成功")
        }
    };
    // 技能 RAG 索引：启动时从 SkillStore 全量重建，技能增删后由对应命令 rebuild
    let skill_index = Arc::new(effisuite_core::SkillIndex::new());
    let skills_root = skills_dir();

    // 运行时 agent 公共会话交流池存储：跨会话长任务登记 / 状态上报 / 收件箱 @ 消息。
    // 与主 agent、子 agent 共享同一份 Arc；pool.json 持久化，崩溃重启可恢复。
    let agent_pool = match effisuite_agent::AgentPoolStore::new(pool_dir()) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "AgentPoolStore 初始化失败，回退到临时目录");
            effisuite_agent::AgentPoolStore::new(std::env::temp_dir().join("effisuite-pool"))
                .expect("临时目录 AgentPoolStore 必须成功")
        }
    };


    // ===== 模型管理句柄 + 子 agent 管理器（注入 agent，供新工具使用） =====
    // 配置版本号：manage_model 工具修改配置后 bump，send_message 时懒重建 agent
    let config_rev = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let agent_rev = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let config_lock: Arc<RwLock<Arc<AgentConfig>>> =
        Arc::new(RwLock::new(Arc::new(config.clone())));
    let model_manager = Arc::new(ModelManagerHandle {
        config: Arc::clone(&config_lock),
        save: Box::new(save_config),
        bump: Arc::clone(&config_rev),
    });
    // 子 agent 事件发射器：setup 阶段回填 AppHandle 后转发为前端 sub-agent-event；
    // 同时按 conversation_id 累积到 sub_agent_records 缓冲，供 send_message_stream
    // 流结束时把子 agent 过程记录持久化到助手消息（重启后历史回看可恢复卡片）。
    let emitter_slot: Arc<std::sync::Mutex<Option<tauri::AppHandle>>> =
        Arc::new(std::sync::Mutex::new(None));
    let sub_agent_records: Arc<
        std::sync::Mutex<std::collections::HashMap<String, Vec<SubAgentRecord>>>,
    > = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let emitter = {
        let slot = Arc::clone(&emitter_slot);
        let buf = Arc::clone(&sub_agent_records);
        Box::new(move |ev: &SubAgentEvent| {
            if let Some(handle) = slot.lock().unwrap().as_ref() {
                let _ = handle.emit("sub-agent-event", ev);
            }
            accumulate_sub_agent_event(&buf, ev);
        })
    };
    let sub_agents = Arc::new(SubAgentManager::new(
        SubAgentKit {
            memory: Some(Arc::clone(&memory)),
            pinned_memory: Some(Arc::clone(&pinned_memory)),
            current_conversation_id: Arc::clone(&current_conversation_id),
            working_dir: Arc::clone(&working_dir),
            image_gen_config: Arc::clone(&image_gen_config),
            attachments_dir: attachments_root.clone(),
            store: Arc::clone(&store),
            skill_index: Some(Arc::clone(&skill_index)),
            skill_store: Some(skill_store.clone()),
            clawhub_client: Some(clawhub_client.clone()),
            skills_dir: Some(skills_root.clone()),
            plugin_store: Some(plugin_store.clone()),
            compression_store: Some(compression_store.clone()),
              model_config: Arc::clone(&config_lock),
              model_manager: Some(Arc::clone(&model_manager)),
              agent_pool: Some(agent_pool.clone()),
          },
          emitter,
      ));

    // 后台命令会话管理器：agent 的 shell_session_* 工具启用/交互常驻 cmd/sh 会话。
    // 会话事件（started/command/output/exited/error）经 emitter 转发为前端
    // shell-session-event，前端在 main-content 底栏以便签展示 AI 工作状态。
    let shell_emitter_slot: Arc<std::sync::Mutex<Option<tauri::AppHandle>>> =
        Arc::new(std::sync::Mutex::new(None));
    let shell_emitter = {
        let slot = Arc::clone(&shell_emitter_slot);
        Box::new(move |ev: &ShellSessionEvent| {
            if let Some(handle) = slot.lock().unwrap().as_ref() {
                let _ = handle.emit("shell-session-event", ev);
            }
        })
    };
    let shell_sessions = Arc::new(ShellSessionManager::new(
        shell_emitter,
        Arc::clone(&current_conversation_id),
    ));
    // 事件总线：ASR 服务与 P2P 均需注入，须在 build_agent 之前构造
    let event_bus = EventBus::new(64);

    // ASR 服务构造：AsrStore 持久化转写记录，AsrSummaryIndex 接入独立 MemoryIndex
    // 做摘要 RAG 检索（namespace="asr" 隔离，不污染主记忆索引）
    let asr_store = match AsrStore::new(asr_dir()) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(error = %e, "AsrStore 初始化失败，ASR 记录不会持久化");
            None
        }
    };
    let asr_summary_index = AsrSummaryIndex::new(Arc::new(RwLock::new(MemoryIndex::new())));
    let asr_service = Arc::new(AsrService::from_config(
        config.asr_config.clone(),
        Some(Arc::new(event_bus.clone())),
        asr_store,
        Some(asr_summary_index),
    ));

    // P2P 管理器（先于 agent 构造：agent 需要其 RemoteTaskDispatcher 能力做跨设备任务派发）
    let p2p = Arc::new(P2pManager::new(event_bus.clone()));
    // 用户中断注入队列：AI 生成期间用户排队消息（先写 store 再入队），
    // agent hook / send_message_stream 续接循环据此消费。
      let pending_user_messages = Arc::new(effisuite_agent::PendingUserMessages::new());
      // 推理设置共享句柄：send_message / send_message_stream 命令在发送前写入，
      // agent 每回合读取并注入请求体（thinking + reasoning_effort）。默认 None = 关闭。
      let reasoning_config: Arc<
          tokio::sync::RwLock<Option<effisuite_agent::ReasoningConfig>>,
      > = Arc::new(tokio::sync::RwLock::new(None));
    // 构造 agent：注入 memory / pinned_memory / current_conversation_id / working_dir /
    // image_gen_config / store / skill_index / skill_store / clawhub / skills_dir /
    // plugin_store / compression_store / model_manager / sub_agents / asr_service /
    // remote_task_dispatcher（P2pManager 实现 RemoteTaskDispatcher trait）
    // skill_store / clawhub / plugin_store / compression_store 已是 Clone（内部 Arc），
    // 直接传值，无需 Arc::new 包装
    let agent: Arc<dyn ChatAgent> = build_agent(
        &config,
        Arc::clone(&memory),
        Arc::clone(&pinned_memory),
        Arc::clone(&current_conversation_id),
        Arc::clone(&working_dir),
        Arc::clone(&image_gen_config),
        attachments_root.clone(),
        Arc::clone(&store),
        Arc::clone(&skill_index),
        skill_store.clone(),
        clawhub_client.clone(),
        skills_root.clone(),
        plugin_store.clone(),
        compression_store.clone(),
        todo_store.clone(),
        Arc::clone(&model_manager),
        Arc::clone(&sub_agents),
        Arc::clone(&asr_service),
            p2p.clone(),
            Arc::clone(&shell_sessions),
            agent_pool.clone(),
              Arc::clone(&pending_user_messages),
              Arc::clone(&reasoning_config),
          );
    let schedule_store = match ScheduledTaskStore::new(schedules_dir()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "ScheduledTaskStore 初始化失败，回退到临时目录");
            ScheduledTaskStore::new(std::env::temp_dir().join("effisuite-schedules"))
                .expect("临时目录 ScheduledTaskStore 必须成功")
        }
    };

    tracing::info!(backend = %agent.backend(), "EffiSuite 启动");

    // 克隆一份配置用于 setup 阶段异步初始化 memory（避免在同步 setup 闭包中 .await）
    let config_for_setup = config.clone();
    let agent_lock = Arc::new(RwLock::new(agent));
    let state = AppState {
        skill_store,
        skill_index,
        schedule_store,
          plugin_store,
          plugin_config,
          compression_store,
        todo_store,
        clawhub: clawhub_client,
        agent: Arc::clone(&agent_lock),
        store: Arc::clone(&store),
        config: Arc::clone(&config_lock),
        p2p,
        event_bus,
        memory: Arc::clone(&memory),
        pinned_memory: Arc::clone(&pinned_memory),
          favorite_workspaces: Arc::clone(&favorite_workspaces),
        current_conversation_id: Arc::clone(&current_conversation_id),
        working_dir: Arc::clone(&working_dir),
        image_gen_config: Arc::clone(&image_gen_config),
        attachments_dir: attachments_root,
        scheduler_handle: std::sync::Mutex::new(None),
        config_rev,
        agent_rev,
        model_manager,
        sub_agents,
                sub_agent_records,
                asr_service,
                shell_sessions,
                agent_pool,
                  pending_user_messages,
                  reasoning_config,

                agent_cancel: Arc::new(effisuite_agent::AgentCancelRegistry::new()),
                auto_compress_inflight: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            };

    tauri::Builder::default()
        .manage(state)
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .setup(move |app| {
            // 回填子 agent 事件发射器所需的 AppHandle（setup 阶段才可用）
            *emitter_slot.lock().unwrap() = Some(app.handle().clone());
            // 回填命令会话事件发射器所需的 AppHandle
            *shell_emitter_slot.lock().unwrap() = Some(app.handle().clone());

            // 订阅内部事件总线，转发为前端 Tauri 事件。
            // 使用 tauri::async_runtime::spawn 以兼容桌面与 mobile 运行时。
            let handle = app.handle().clone();
            let bus = app.state::<AppState>().event_bus.clone();
            tauri::async_runtime::spawn(async move {
                let mut rx = bus.subscribe();
                while let Ok(event) = rx.recv().await {
                    forward_event(&handle, &event);
                }
            });

            // 启动 cron 调度器：每分钟检查一次到点任务并触发技能执行。
            // 句柄回填到 AppState.scheduler_handle，便于 shutdown 时 abort。
            let state = app.state::<AppState>();
            let sched_handle = scheduler::spawn_scheduler(
                app.handle().clone(),
                state.schedule_store.clone(),
                state.skill_store.clone(),
                Arc::clone(&state.agent),
                Arc::clone(&state.store),
            );
            *state.scheduler_handle.lock().unwrap() = Some(sched_handle);

            // RAG 记忆增强启动任务：
            // 1. 全量重建 memory index（从 ConversationStore 加载所有历史消息）
            // 2. 应用 embedding provider（若配置了 api_key）
            // 3. 后台批量计算缺失 embedding（直到全部完成或 provider 失效）
            // 4. 全量重建 skill index（从 SkillStore 加载所有已安装技能，
            //    用于本轮起 RAG 自动注入 [可用技能] 段与 list_installed_skills 工具）
            let store_clone = Arc::clone(&state.store);
            let memory_clone = Arc::clone(&state.memory);
            let skill_index_clone = Arc::clone(&state.skill_index);
                let skill_store_clone = state.skill_store.clone();
                let plugin_store_clone = state.plugin_store.clone();
                tauri::async_runtime::spawn(async move {
                    rebuild_memory_from_store(&store_clone, &memory_clone).await;
                    apply_embedding_provider(&config_for_setup, &memory_clone).await;
                    spawn_embedding_computation(memory_clone).await;
                    if let Err(e) = skill_index_clone
                        .rebuild_from_store(&skill_store_clone)
                        .await
                    {
                        tracing::warn!(error = %e, "启动时技能索引 rebuild 失败，将在下次技能增删或重启时重试");
                    } else {
                        tracing::info!("启动时技能索引 rebuild 完成");
                    }
                    // 启动时把已安装插件的 manifest 命令同步为 agent 技能（source="plugin"），
                    // 使 agent 在 list_installed_skills 与 RAG 技能注入中看到插件命令。
                    // 放在技能索引 rebuild 之后；sync 内部会幂等 upsert 并再次 rebuild（一次性成本）。
                    commands::plugins::sync_plugin_skills(
                        plugin_store_clone,
                        skill_store_clone,
                        skill_index_clone,
                    )
                    .await;
                });

            // ===== P2P 服务启动 =====
            // 程序启动 → P2P 广播启动 → 持续扫描可信设备是否在线。
            // 1. 确保 p2p 目录存在（信任库 trust.json 的父目录）
            // 2. 加载或新建 TrustStore（首次启动生成 Ed25519 身份并持久化）
            // 3. 从信任库读取本机身份密钥
            // 4. 调 start_with_trust 启动 transport（TCP 加密通道）+ discovery（UDP 广播）
            //    + pairing（配对协议）+ sync（镜像同步），并自动收集 PairingRequest 事件
            let p2p_manager = Arc::clone(&state.p2p);
            let p2p_trust_path = p2p_trust_path();
            // 镜像同步数据源：基于本地会话 / 插件 / 永久记忆存储
            let sync_store = sync_store::P2pSyncStore::new(
                Arc::clone(&state.store),
                state.plugin_store.clone(),
                Arc::clone(&state.pinned_memory),
            );
            tauri::async_runtime::spawn(async move {
                // 确保目录存在（load_or_create 仅写文件，不创建父目录）
                if let Some(parent) = p2p_trust_path.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        tracing::error!(error = %e, "P2P 信任库目录创建失败，P2P 服务未启动");
                        return;
                    }
                }
                let trust = match effisuite_p2p::TrustStore::load_or_create(p2p_trust_path).await {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::error!(error = %e, "P2P 信任库加载失败，P2P 服务未启动");
                        return;
                    }
                };
                let identity = match trust.self_identity().await {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::error!(error = %e, "P2P 身份密钥读取失败，P2P 服务未启动");
                        return;
                    }
                };
                // 绑定 0.0.0.0:DEFAULT_P2P_PORT（TCP/UDP 同端口号不冲突）；
                // 端口被占用时由 OS 分配（bind 0.0.0.0:0），discovery 广播实际端口
                let bind_addr: std::net::SocketAddr =
                    format!("0.0.0.0:{}", effisuite_p2p::DEFAULT_P2P_PORT)
                        .parse()
                        .expect("valid bind addr");
                if let Err(e) = p2p_manager
                    .start_with_trust(trust, identity, bind_addr)
                    .await
                {
                    tracing::error!(error = %e, "P2P 服务启动失败");
                    return;
                }
                // 注入镜像同步数据源（同步器读写会话/插件/永久记忆）
                p2p_manager
                    .set_sync_data_store(Arc::new(sync_store))
                    .await;
                tracing::info!("P2P 服务已启动（transport + discovery + pairing + sync）");
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            get_agent_backend,
            // config
            get_config,
            set_config,
            // providers & models
            list_provider_presets,
            get_active_model_info,
            save_model,
            delete_model,
            set_active_model,
            set_image_gen_model,
            set_service_model_role,
            list_remote_models,
            get_remote_model,
            generate_image,
            read_attachment,
            // theme
            set_theme,
            // conversations
            list_conversations,
            get_conversation,
            create_conversation,
            delete_conversation,
            delete_conversations,
            rename_conversation,
            toggle_pin_conversation,
            search_conversations,
            auto_classify_conversation,
            // RAG 记忆增强检索
            search_memory,
            get_memory_stats,
            // 永久记忆管理
            list_pinned_memories,
            add_pinned_memory,
            update_pinned_memory,
            delete_pinned_memory,
            clear_pinned_memories,
            // 上下文注入预览
            get_context_preview,
            // 消息压缩
            compress_messages,
            compress_messages_stream,
            get_compression_state,
            clear_compression_state,
            update_compression_settings,
            // 每会话 todoTree
            get_todo_tree,
            save_todo_tree,
            clear_todo_tree,
            // chat
            send_message,
            send_message_stream,
            queue_user_message,
            stop_agent,
            // p2p
            scan_devices,
            get_devices,
            get_online_devices,
            start_discovery,
            stop_discovery,
            pair_by_address,
            pair_device,
            reject_pair,
            unpair,
            pending_pairing_requests,
            sync_pull,
            sync_push,
            sync_cursor,
            get_p2p_status,
            stop_p2p,
            // 文件 / 图片选择
            pick_file,
            pick_image,
            capture_photo,
            read_file_text,
            pick_directory,
            // 技能
            list_skills,
            create_skill,
            update_skill,
            delete_skill,
            set_conversation_working_dir,
            get_conversation_working_dir,
          // 常用工作区（会话工作区收藏）
          list_favorite_workspaces,
          add_favorite_workspace,
          delete_favorite_workspace,
            // git 上下文版本管理（开分支/保存/撤回/回溯）
            git_context_status,
            git_context_init,
            git_context_branch,
            git_context_save,
            git_context_revert,
            git_context_history,
            // 会话版本管理（自研快照：每次 edit 等操作自动保存工作区状态，可撤回/回溯）
            snapshot_save,
            snapshot_list,
            snapshot_status,
            snapshot_restore,
            snapshot_delete,
            // 会话版本控制（git 风格：分支/临时版本/回溯/撤回/检出）
            version_list,
            version_create_branch,
            version_save_temp,
            version_rollback,
            version_undo_before,
            version_checkout,
            version_delete_ref,
            // ClawHub 浏览 / 安装
            clawhub_list_skills,
            clawhub_search_skills,
            clawhub_get_skill,
            clawhub_install_skill,
            clawhub_uninstall_skill,
            clawhub_list_plugins,
            clawhub_search_plugins,
            clawhub_get_package,
            clawhub_install_plugin,
            clawhub_uninstall_plugin,
            list_installed_plugins,
            // 插件贡献注册与插件配置（manifest / 生命周期 / appdata 配置）
            list_plugin_contributions,
            get_plugin_manifest,
            get_plugin_config,
            set_plugin_config,
            delete_plugin_config,
            get_plugin_config_all,
            // 定时任务
            list_scheduled_tasks,
            create_scheduled_task,
            delete_scheduled_task,
            toggle_scheduled_task,
            // ASR 语音转写
            asr_start_streaming,
            asr_push_audio,
            asr_finish_streaming,
            asr_cancel_streaming,
            asr_transcribe_file,
            asr_list_records,
            asr_get_record,
            asr_search_records,
            asr_delete_record,
            asr_update_record,
            asr_search_summaries,
            asr_list_sessions,
            asr_get_config,
            asr_update_config,
            asr_generate_summary,
            // 后台命令会话（前端底栏便签）
            list_shell_sessions,
            kill_shell_session,
            // 运行时 agent 公共会话交流池（会话列表运行状态）
            list_pool,
            get_pool_entry,
            clear_pool,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

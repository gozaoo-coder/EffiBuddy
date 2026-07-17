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

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use effisuite_agent::{ChatAgent, MockAgent, RigAgent};
use effisuite_core::{
    AgentConfig, BackendKind, BusEvent, Conversation, ConversationStore, Device, EventBus, Message,
    Role,
};
use effisuite_p2p::P2pManager;
use futures::StreamExt;
use tauri::{Emitter, Manager};
use tokio::sync::RwLock;

/// 应用全局状态，由 `tauri::Builder::manage` 注入。
///
/// 字段按大小降序：`Arc<RwLock<...>>` (fat pointer, 2 usize) 在前，
/// `Arc<P2pManager>` (1 usize) 与 `EventBus` (1 usize) 在后。
pub struct AppState {
    /// 可热替换的 agent：RwLock 写少读多，内层 Arc 让 async 命令可跨 await 持有
    pub agent: Arc<RwLock<Arc<dyn ChatAgent>>>,
    pub store: Arc<ConversationStore>,
    pub p2p: Arc<P2pManager>,
    pub event_bus: EventBus,
    pub config: Arc<RwLock<AgentConfig>>,
}

/// 当前 Unix 毫秒时间戳；失败时回退为 0，避免在命令路径里 panic。
#[inline]
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// appdata 根目录：`<app_data_dir>/effisuite`
fn appdata_root() -> std::path::PathBuf {
    let base = dirs::data_dir()
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("effisuite")
}

/// 配置文件路径：`<appdata>/config.json`
fn config_path() -> std::path::PathBuf {
    appdata_root().join("config.json")
}

/// 会话存储目录：`<appdata>/conversations`
fn conversations_dir() -> std::path::PathBuf {
    appdata_root().join("conversations")
}

/// 加载配置；不存在时返回默认值
fn load_config_or_default() -> AgentConfig {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => AgentConfig::default(),
    }
}

/// 持久化配置到磁盘
fn save_config(config: &AgentConfig) -> std::result::Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let s = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(&path, s).map_err(|e| e.to_string())?;
    Ok(())
}

/// 根据 config 构造对应的 ChatAgent
fn build_agent(config: &AgentConfig) -> Arc<dyn ChatAgent> {
    match config.backend {
        BackendKind::Openai if config.is_rig_ready() => {
            match RigAgent::from_key(
                &config.api_key,
                &config.model_name,
                &config.preamble,
                config.enable_tools,
            ) {
                Ok(agent) => Arc::new(agent),
                Err(e) => {
                    tracing::warn!(error = %e, "RigAgent 构造失败，回退到 MockAgent");
                    Arc::new(MockAgent::new())
                }
            }
        }
        _ => Arc::new(MockAgent::new()),
    }
}

/// 把 `BusEvent` 转发为前端可监听的 Tauri 事件。
/// payload 直接使用 `BusEvent` 本身（已实现 Serialize，带 `kind` 标签）。
fn forward_event(handle: &tauri::AppHandle, event: &BusEvent) {
    let (name, payload) = match event {
        BusEvent::AgentStreamToken { .. } => ("agent-token", event),
        BusEvent::AgentMessage { .. } => ("agent-message", event),
        BusEvent::DeviceFound { .. } => ("device-found", event),
        BusEvent::DeviceStatusChanged { .. } => ("device-status-changed", event),
        BusEvent::PairingRequest { .. } => ("pairing-request", event),
    };
    let _ = handle.emit(name, payload);
}

// =========================================================
// 命令：通用
// =========================================================

#[tauri::command]
fn greet(name: String) -> String {
    format!("Hello, {}! EffiSuite 已就绪。", name)
}

#[tauri::command]
async fn get_agent_backend(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let agent = state.agent.read().await.clone();
    Ok(agent.backend().to_string())
}

// =========================================================
// 命令：配置管理
// =========================================================

#[tauri::command]
async fn get_config(state: tauri::State<'_, AppState>) -> Result<AgentConfig, String> {
    Ok(state.config.read().await.clone())
}

#[tauri::command]
async fn set_config(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    config: AgentConfig,
) -> Result<(), String> {
    // 持久化
    save_config(&config)?;

    // 构造新 agent
    let new_agent = build_agent(&config);

    // 替换 state 中的 agent 和 config
    {
        let mut agent_lock = state.agent.write().await;
        *agent_lock = new_agent;
    }
    {
        let mut config_lock = state.config.write().await;
        *config_lock = config;
    }

    // 通知前端 backend 变化
    let _ = app_handle.emit("agent-backend-changed", ());

    Ok(())
}

// =========================================================
// 命令：会话管理
// =========================================================

#[tauri::command]
async fn list_conversations(state: tauri::State<'_, AppState>) -> Result<Vec<ConversationMeta>, String> {
    let store = state.store.clone();
    let list = store.list().await.map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .map(|(id, created_at, message_count)| ConversationMeta {
            id,
            created_at,
            message_count,
        })
        .collect())
}

/// 会话元信息（轻量，不含消息体）
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConversationMeta {
    pub id: String,
    pub created_at: u64,
    pub message_count: usize,
}

#[tauri::command]
async fn get_conversation(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Option<Conversation>, String> {
    let store = state.store.clone();
    store.load(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn create_conversation(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conv = Conversation::new(id.clone(), now_ms());
    state.store.save(&conv).await.map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
async fn delete_conversation(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    state.store.delete(&id).await.map_err(|e| e.to_string())
}

// =========================================================
// 命令：聊天（流式 + 非流式）
// =========================================================

#[tauri::command]
async fn send_message(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    conversation_id: String,
    content: String,
) -> Result<String, String> {
    let agent = state.agent.read().await.clone();
    let store = state.store.clone();
    let bus = state.event_bus.clone();

    // 先把用户消息持久化到 store，同时取回完整历史
    let user_msg = Message::new(
        uuid::Uuid::new_v4().to_string(),
        Role::User,
        content,
        now_ms(),
    );
    let conv = store
        .append_message(&conversation_id, user_msg, now_ms())
        .await
        .map_err(|e| e.to_string())?;

    // 调用 agent
    let history = conv.history().to_vec();
    let reply = agent.chat(&history).await.map_err(|e| e.to_string())?;

    // 持久化助手回复
    let assistant_msg = Message::new(
        uuid::Uuid::new_v4().to_string(),
        Role::Assistant,
        reply.clone(),
        now_ms(),
    );
    store
        .append_message(&conversation_id, assistant_msg, now_ms())
        .await
        .map_err(|e| e.to_string())?;

    // 通过事件总线通知前端
    bus.publish(BusEvent::AgentMessage {
        conversation_id: conversation_id.clone(),
        content: reply.clone(),
        done: true,
    });

    let _ = app_handle.emit("agent-message", &reply);
    Ok(reply)
}

/// 流式发送消息：spawn 独立 task，逐 token emit "agent-token"，结束 emit "agent-done"
#[tauri::command]
async fn send_message_stream(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    conversation_id: String,
    content: String,
) -> Result<(), String> {
    let agent = state.agent.read().await.clone();
    let store = state.store.clone();
    let bus = state.event_bus.clone();
    let handle = app_handle.clone();

    // 1. 持久化用户消息并取回完整历史
    let user_msg = Message::new(
        uuid::Uuid::new_v4().to_string(),
        Role::User,
        content,
        now_ms(),
    );
    let conv = store
        .append_message(&conversation_id, user_msg, now_ms())
        .await
        .map_err(|e| e.to_string())?;
    let history = conv.history().to_vec();
    let conv_id = conversation_id.clone();

    // 2. spawn 独立 task 驱动流
    tauri::async_runtime::spawn(async move {
        let mut stream = agent.chat_stream(&history);
        let mut full = String::with_capacity(256);

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(token) => {
                    full.push_str(&token);
                    // 通过事件总线 + Tauri emit 双通道推送
                    bus.publish(BusEvent::AgentStreamToken {
                        conversation_id: conv_id.clone(),
                        content: token.clone(),
                        done: false,
                    });
                    let _ = handle.emit(
                        "agent-token",
                        &StreamTokenPayload {
                            conversation_id: &conv_id,
                            content: &token,
                            done: false,
                        },
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "stream error");
                    let _ = handle.emit(
                        "agent-stream-error",
                        &StreamErrorPayload {
                            conversation_id: &conv_id,
                            error: &e.to_string(),
                        },
                    );
                    return;
                }
            }
        }

        // 3. 流结束，持久化完整回复
        let assistant_msg = Message::new(
            uuid::Uuid::new_v4().to_string(),
            Role::Assistant,
            full.clone(),
            now_ms(),
        );
        if let Err(e) = store
            .append_message(&conv_id, assistant_msg, now_ms())
            .await
        {
            tracing::warn!(error = %e, "persist assistant reply failed");
        }

        // 4. 通知前端流结束
        bus.publish(BusEvent::AgentStreamToken {
            conversation_id: conv_id.clone(),
            content: String::new(),
            done: true,
        });
        let _ = handle.emit(
            "agent-done",
            &StreamTokenPayload {
                conversation_id: &conv_id,
                content: &full,
                done: true,
            },
        );
    });

    Ok(())
}

/// 流式 token payload（与前端 TS 类型对齐）
#[derive(Debug, serde::Serialize)]
struct StreamTokenPayload<'a> {
    conversation_id: &'a str,
    content: &'a str,
    done: bool,
}

#[derive(Debug, serde::Serialize)]
struct StreamErrorPayload<'a> {
    conversation_id: &'a str,
    error: &'a str,
}

// =========================================================
// 命令：P2P
// =========================================================

#[tauri::command]
async fn scan_devices(
    state: tauri::State<'_, AppState>,
    _app_handle: tauri::AppHandle,
) -> Result<Vec<Device>, String> {
    let p2p = state.p2p.clone();
    p2p.scan_once().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_devices(state: tauri::State<'_, AppState>) -> Result<Vec<Device>, String> {
    let p2p = state.p2p.clone();
    Ok(p2p.list_devices().await)
}

#[tauri::command]
async fn pair_device(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let p2p = state.p2p.clone();
    p2p.pair_device(&id).await.map_err(|e| e.to_string())
}

// =========================================================
// 入口
// =========================================================

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // 加载配置
    let config = load_config_or_default();
    let agent: Arc<dyn ChatAgent> = build_agent(&config);

    // 初始化存储
    let store = match ConversationStore::new(conversations_dir()) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!(error = %e, "ConversationStore 初始化失败，回退到临时目录");
            Arc::new(
                ConversationStore::new(std::env::temp_dir().join("effisuite-conversations"))
                    .expect("临时目录 ConversationStore 必须成功"),
            )
        }
    };

    let event_bus = EventBus::new(64);
    let p2p = Arc::new(P2pManager::new(event_bus.clone()));

    tracing::info!(backend = %agent.backend(), "EffiSuite 启动");

    let state = AppState {
        agent: Arc::new(RwLock::new(agent)),
        store,
        p2p,
        event_bus,
        config: Arc::new(RwLock::new(config)),
    };

    tauri::Builder::default()
        .manage(state)
        .setup(|app| {
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            get_agent_backend,
            // config
            get_config,
            set_config,
            // conversations
            list_conversations,
            get_conversation,
            create_conversation,
            delete_conversation,
            // chat
            send_message,
            send_message_stream,
            // p2p
            scan_devices,
            get_devices,
            pair_device,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

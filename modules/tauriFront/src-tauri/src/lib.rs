//! EffiSuite Tauri 应用入口：连接 agent / p2p / core 并暴露给前端
//!
//! 设计要点：
//! - `AppState` 字段按大小降序排列，最小化结构体 padding
//! - 命令层薄封装：转译请求、转发事件，不持长临界区锁
//! - async 命令中先 clone 出 `Arc` 句柄再 `.await`，避免跨 await 持有
//!   `tauri::State` 的借用（Tauri 2.x async command 的 lifetime 约束）
//! - 事件转发通过 broadcast 订阅 + spawn 完成，遵循"消息传递代替共享内存"

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use effisuite_agent::{ChatAgent, MockAgent};
use effisuite_core::{BusEvent, Device, EventBus, Message, Role};
use effisuite_p2p::P2pManager;
use tauri::{Emitter, Manager};

/// 应用全局状态，由 `tauri::Builder::manage` 注入。
///
/// 字段按大小降序：`agent`（`Arc<dyn ChatAgent>` = fat pointer，2 usize）
/// 在前，`p2p`（`Arc<P2pManager>`，1 usize）与 `event_bus`（1 usize）在后。
///
/// 使用 `Arc` 包装以便在 async 命令中廉价 clone 出句柄、跨 await 持有，
/// 避免 `tauri::State` 借用的 lifetime 冲突。
pub struct AppState {
    pub agent: Arc<dyn ChatAgent>,
    pub p2p: Arc<P2pManager>,
    pub event_bus: EventBus,
}

/// 当前 Unix 毫秒时间戳；失败时回退为 0，避免在命令路径里 panic。
#[inline]
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 把 `BusEvent` 转发为前端可监听的 Tauri 事件。
/// payload 直接使用 `BusEvent` 本身（已实现 Serialize，带 `kind` 标签）。
fn forward_event(handle: &tauri::AppHandle, event: &BusEvent) {
    let (name, payload) = match event {
        BusEvent::AgentMessage { .. } => ("agent-message", event),
        BusEvent::DeviceFound { .. } => ("device-found", event),
        BusEvent::DeviceStatusChanged { .. } => ("device-status-changed", event),
        BusEvent::PairingRequest { .. } => ("pairing-request", event),
    };
    // emit 失败（如无窗口监听）不影响后端逻辑，静默忽略。
    let _ = handle.emit(name, payload);
}

#[tauri::command]
fn greet(name: String) -> String {
    format!("Hello, {}! EffiSuite 已就绪。", name)
}

#[tauri::command]
fn get_agent_backend(state: tauri::State<'_, AppState>) -> String {
    state.agent.backend().to_string()
}

#[tauri::command]
async fn send_message(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    content: String,
) -> Result<String, String> {
    // 先 clone 出 Arc 句柄，避免跨 await 持有 State 借用
    let agent = state.agent.clone();
    // content 之后不再使用，直接 move 进 Message（零分配）
    let msg = Message::new(uuid::Uuid::new_v4().to_string(), Role::User, content, now_ms());
    let reply = agent.chat(&[msg]).await.map_err(|e| e.to_string())?;
    // 前端也可直接使用返回值；emit 失败不影响主流程。
    let _ = app_handle.emit("agent-reply", &reply);
    Ok(reply)
}

#[tauri::command]
async fn scan_devices(
    state: tauri::State<'_, AppState>,
    _app_handle: tauri::AppHandle,
) -> Result<Vec<Device>, String> {
    let p2p = state.p2p.clone();
    p2p.scan_once().await.map_err(|e| e.to_string())
}

// Tauri 2.x：async command 必须返回 Result
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let event_bus = EventBus::new(64);
    let agent: Arc<dyn ChatAgent> = Arc::new(MockAgent::new());
    let p2p = Arc::new(P2pManager::new(event_bus.clone()));

    tracing::info!(backend = %agent.backend(), "EffiSuite 启动");

    let state = AppState {
        agent,
        p2p,
        event_bus,
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
            send_message,
            scan_devices,
            get_devices,
            pair_device,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

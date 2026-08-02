//! 远端任务派发工具：让 LLM 跨已配对设备指派任务
//!
//! 单工具多 action 设计（rig `Tool` trait），底层依赖 `RemoteTaskDispatcher` trait
//! （定义在 `effisuite-core`，由 `P2pManager` 实现），实现依赖倒置：
//! agent crate 不依赖 `effisuite-p2p`，仅依赖 trait object。
//!
//! - list：列出当前在线且已配对的设备（AI 据此选择派发目标）
//! - dispatch：向指定设备派发自然语言任务，返回远端 AI 处理结果
//!
//! # 设计要点（对齐 user_rules）
//!
//! - 工具无状态，所有状态在 `RemoteTaskDispatcher` 实现侧
//! - IO 全异步，工具内不持锁（dispatcher 实现侧保证临界区极短）
//! - `with_capacity` 预分配输出缓冲，避免多次 realloc
//! - 用 `&str` 做参数（`RemoteTaskDispatcher::dispatch_remote_task(&self, device_id: &str, task: &str)` 已符合）
//! - 错误用 `thiserror` newtype，不引入 `anyhow`
//! - 仅在 P2P 在线且 dispatcher 注入时注册；未注入时工具不暴露给 LLM（由 `build_agent` 控制）

use std::sync::Arc;

use effisuite_core::RemoteTaskDispatcher;
use rig_core::tool::Tool;
use serde::Deserialize;

/// 操作类型
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DispatchAction {
    /// 列出当前在线且已配对的设备
    List,
    /// 向指定设备派发任务
    Dispatch,
}

/// 工具参数
///
/// 字段按大小降序：`String`（24 字节）相同，`DispatchAction`（1 字节，Copy）在后。
/// 实际声明顺序受 `#[serde]` 解析逻辑约束（按字段名出现顺序），此处保持可读性。
#[derive(Deserialize)]
pub struct DispatchRemoteTaskArgs {
    /// 操作类型
    pub action: DispatchAction,
    /// 目标设备 id（dispatch 必填，list 忽略）
    #[serde(default)]
    pub device_id: Option<String>,
    /// 任务描述（dispatch 必填，自然语言描述要远端设备做的事）
    #[serde(default)]
    pub task: Option<String>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("dispatch_remote_task error: {0}")]
pub struct DispatchRemoteTaskError(String);

/// 远端任务派发工具
///
/// 持有 `Arc<dyn RemoteTaskDispatcher>`（trait object），由 Tauri 命令层在
/// `build_agent` 时注入 `P2pManager` 的 clone（trait upcasting）。
/// 工具本身无状态，所有连接 / 派发逻辑由 dispatcher 实现侧处理。
pub struct DispatchRemoteTaskTool {
    dispatcher: Arc<dyn RemoteTaskDispatcher>,
}

impl DispatchRemoteTaskTool {
    pub fn new(dispatcher: Arc<dyn RemoteTaskDispatcher>) -> Self {
        Self { dispatcher }
    }
}

// =========================================================
// Tool trait 实现
// =========================================================

impl Tool for DispatchRemoteTaskTool {
    const NAME: &'static str = "dispatch_remote_task";

    type Error = DispatchRemoteTaskError;
    type Args = DispatchRemoteTaskArgs;
    type Output = String;

    fn description(&self) -> String {
        "跨已配对设备派发任务（P2P 镜像模式）。支持：\n\
         - list：列出当前在线且已配对的设备（id / 名称 / 地址 / 最近在线时间），AI 据此选择派发目标\n\
         - dispatch：向指定设备派发自然语言任务，远端 AI 处理后返回结果文本\n\n\
         典型场景：用户在多设备间协作（如本机为「电脑」负责编程，向「手机」派发检索用户信息的任务），\n\
         仅当目标设备在线时才可派发；派发会阻塞至远端 AI 返回结果（百毫秒到分钟级）。\n\n\
         各 action 所需参数：\n\
         - list: 无需额外参数\n\
         - dispatch: device_id, task"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "dispatch"],
                    "description": "操作类型：list=列出在线设备；dispatch=派发任务"
                },
                "device_id": {
                    "type": "string",
                    "description": "目标设备 id（dispatch 必填，可先用 list 查询在线设备获取）"
                },
                "task": {
                    "type": "string",
                    "description": "任务描述（dispatch 必填，自然语言描述要远端设备做的事；远端 AI 处理后返回结果）"
                }
            },
            "required": ["action"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        match args.action {
            DispatchAction::List => self.action_list().await,
            DispatchAction::Dispatch => self.action_dispatch(args).await,
        }
    }
}

// =========================================================
// Action 实现
// =========================================================

impl DispatchRemoteTaskTool {
    /// 列出当前在线且已配对的设备
    async fn action_list(&self) -> Result<String, DispatchRemoteTaskError> {
        let devices = self.dispatcher.list_online_devices().await;
        if devices.is_empty() {
            return Ok("当前没有在线的已配对设备。".to_string());
        }

        let total = devices.len();
        // 预估每条 ~96 字节（id + name + address + 时间戳）
        let mut out = String::with_capacity(64 + total * 96);
        out.push_str(&format!("在线已配对设备（共 {total} 个）：\n\n"));

        for (i, dev) in devices.iter().enumerate() {
            out.push_str(&format!(
                "{}. {} ({})\n   地址: {}  |  最近在线: {}\n",
                i + 1,
                dev.name,
                dev.id,
                dev.address,
                format_last_seen(dev.last_seen)
            ));
            if i + 1 < total {
                out.push('\n');
            }
        }
        out.push_str("\n提示：使用 dispatch action 派发任务时，device_id 取上方括号内的 id。");
        Ok(out)
    }

    /// 向指定设备派发任务
    async fn action_dispatch(&self, args: DispatchRemoteTaskArgs) -> Result<String, DispatchRemoteTaskError> {
        let device_id = args
            .device_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| DispatchRemoteTaskError("dispatch 操作需要 device_id 参数".to_string()))?;

        let task = args
            .task
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| DispatchRemoteTaskError("dispatch 操作需要 task 参数".to_string()))?;

        // 先校验目标在线，给出更友好的错误（避免直接派发到离线设备后等超时）
        let online = self.dispatcher.list_online_devices().await;
        let target_online = online.iter().any(|d| d.id == device_id);
        if !target_online {
            let names: Vec<&str> = online.iter().map(|d| d.name.as_str()).collect();
            let hint = if names.is_empty() {
                "当前没有在线的已配对设备。".to_string()
            } else {
                format!("当前在线设备：{}", names.join("、"))
            };
            return Err(DispatchRemoteTaskError(format!(
                "设备 {device_id} 不在线或未配对。{hint}"
            )));
        }

        // 派发任务（阻塞至远端 AI 返回结果或超时）
        let result = self
            .dispatcher
            .dispatch_remote_task(device_id, task)
            .await
            .map_err(|e| DispatchRemoteTaskError(format!("派发失败：{e}")))?;

        // 拼装回执：包含原任务摘要 + 远端结果，便于 LLM 在多设备协作中保持上下文
        let task_preview = truncate_preview(task, 80);
        let mut out = String::with_capacity(128 + result.len());
        out.push_str(&format!("已向设备 {device_id} 派发任务「{task_preview}」。\n\n"));
        out.push_str("远端处理结果：\n");
        out.push_str(&result);
        Ok(out)
    }
}

// =========================================================
// 辅助函数
// =========================================================

/// 把 Unix 秒时间戳格式化为可读时间（本地时区）。失败时回退为 "未知"。
#[inline]
fn format_last_seen(ts: u64) -> String {
    if ts == 0 {
        return "未知".to_string();
    }
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(ts as i64, 0) {
        chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        _ => "未知".to_string(),
    }
}

/// 截取任务预览，避免回执过长。中文字符按 char 计数（不会切坏 UTF-8 边界）。
#[inline]
fn truncate_preview(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use effisuite_core::{Device, DeviceStatus, Result};
    use std::sync::Mutex;

    /// Mock dispatcher：可配置在线设备列表与派发结果
    ///
    /// `dispatch_result` 用 `std::result::Result<String, String>` 存储（error 为字符串），
    /// 调用时转成 `effisuite_core::Result<String>`（`CoreError` 未实现 `Clone`，
    /// 无法直接 clone `Result<String, CoreError>`）。
    struct MockDispatcher {
        online: Mutex<Vec<Device>>,
        dispatch_result: Mutex<std::result::Result<String, String>>,
    }

    impl MockDispatcher {
        fn new(
            online: Vec<Device>,
            dispatch_result: std::result::Result<String, String>,
        ) -> Self {
            Self {
                online: Mutex::new(online),
                dispatch_result: Mutex::new(dispatch_result),
            }
        }
    }

    #[async_trait]
    impl RemoteTaskDispatcher for MockDispatcher {
        async fn list_online_devices(&self) -> Vec<Device> {
            self.online.lock().unwrap().clone()
        }

        async fn dispatch_remote_task(&self, _device_id: &str, _task: &str) -> Result<String> {
            match self.dispatch_result.lock().unwrap().clone() {
                Ok(s) => Ok(s),
                Err(e) => Err(effisuite_core::CoreError::P2p(e)),
            }
        }
    }

    fn make_device(id: &str, name: &str) -> Device {
        Device {
            id: id.to_string(),
            name: name.to_string(),
            address: "192.168.1.10:47823".to_string(),
            last_seen: 1700000000,
            status: DeviceStatus::Paired,
        }
    }

    fn make_tool(
        online: Vec<Device>,
        dispatch_result: std::result::Result<String, String>,
    ) -> DispatchRemoteTaskTool {
        let dispatcher: Arc<dyn RemoteTaskDispatcher> =
            Arc::new(MockDispatcher::new(online, dispatch_result));
        DispatchRemoteTaskTool::new(dispatcher)
    }

    #[tokio::test]
    async fn list_returns_empty_message_when_no_devices() {
        let tool = make_tool(Vec::new(), Ok("".to_string()));
        let out = tool
            .call(DispatchRemoteTaskArgs {
                action: DispatchAction::List,
                device_id: None,
                task: None,
            })
            .await
            .unwrap();
        assert!(out.contains("没有在线"));
    }

    #[tokio::test]
    async fn list_formats_devices_with_id_and_name() {
        let tool = make_tool(
            vec![make_device("dev-aaa", "电脑"), make_device("dev-bbb", "手机")],
            Ok("".to_string()),
        );
        let out = tool
            .call(DispatchRemoteTaskArgs {
                action: DispatchAction::List,
                device_id: None,
                task: None,
            })
            .await
            .unwrap();
        assert!(out.contains("共 2 个"));
        assert!(out.contains("电脑"));
        assert!(out.contains("dev-aaa"));
        assert!(out.contains("手机"));
        assert!(out.contains("dev-bbb"));
    }

    #[tokio::test]
    async fn dispatch_rejects_missing_device_id() {
        let tool = make_tool(vec![make_device("dev-aaa", "电脑")], Ok("ok".to_string()));
        let err = tool
            .call(DispatchRemoteTaskArgs {
                action: DispatchAction::Dispatch,
                device_id: None,
                task: Some("做点事".to_string()),
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("device_id"));
    }

    #[tokio::test]
    async fn dispatch_rejects_missing_task() {
        let tool = make_tool(vec![make_device("dev-aaa", "电脑")], Ok("ok".to_string()));
        let err = tool
            .call(DispatchRemoteTaskArgs {
                action: DispatchAction::Dispatch,
                device_id: Some("dev-aaa".to_string()),
                task: None,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("task"));
    }

    #[tokio::test]
    async fn dispatch_rejects_offline_device() {
        let tool = make_tool(vec![make_device("dev-aaa", "电脑")], Ok("ok".to_string()));
        let err = tool
            .call(DispatchRemoteTaskArgs {
                action: DispatchAction::Dispatch,
                device_id: Some("dev-offline".to_string()),
                task: Some("做点事".to_string()),
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("不在线"));
    }

    #[tokio::test]
    async fn dispatch_success_returns_remote_result() {
        let tool = make_tool(
            vec![make_device("dev-aaa", "电脑")],
            Ok("任务已完成：检索到 3 条用户信息".to_string()),
        );
        let out = tool
            .call(DispatchRemoteTaskArgs {
                action: DispatchAction::Dispatch,
                device_id: Some("dev-aaa".to_string()),
                task: Some("检索用户信息".to_string()),
            })
            .await
            .unwrap();
        assert!(out.contains("已向设备 dev-aaa 派发任务"));
        assert!(out.contains("检索用户信息"));
        assert!(out.contains("远端处理结果"));
        assert!(out.contains("3 条用户信息"));
    }

    #[tokio::test]
    async fn dispatch_propagates_dispatcher_error() {
        let tool = make_tool(
            vec![make_device("dev-aaa", "电脑")],
            Err("远端超时".to_string()),
        );
        let err = tool
            .call(DispatchRemoteTaskArgs {
                action: DispatchAction::Dispatch,
                device_id: Some("dev-aaa".to_string()),
                task: Some("做点事".to_string()),
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("派发失败"));
        assert!(err.to_string().contains("远端超时"));
    }

    #[tokio::test]
    async fn dispatch_trims_whitespace_in_params() {
        let tool = make_tool(
            vec![make_device("dev-aaa", "电脑")],
            Ok("ok".to_string()),
        );
        let out = tool
            .call(DispatchRemoteTaskArgs {
                action: DispatchAction::Dispatch,
                device_id: Some("  dev-aaa  ".to_string()),
                task: Some("  做点事  ".to_string()),
            })
            .await
            .unwrap();
        assert!(out.contains("已向设备 dev-aaa 派发任务"));
    }

    #[test]
    fn truncate_preview_keeps_short_string_intact() {
        assert_eq!(truncate_preview("短任务", 80), "短任务");
    }

    #[test]
    fn truncate_preview_adds_ellipsis_for_long_string() {
        let long = "a".repeat(100);
        let out = truncate_preview(&long, 10);
        assert_eq!(out.chars().count(), 11);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn format_last_seen_handles_zero() {
        assert_eq!(format_last_seen(0), "未知");
    }

    #[test]
    fn format_last_seen_formats_valid_timestamp() {
        let out = format_last_seen(1700000000);
        assert!(!out.is_empty());
        assert_ne!(out, "未知");
    }
}

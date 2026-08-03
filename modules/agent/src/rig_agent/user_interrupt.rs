//! 用户中断注入：AI 生成期间用户仍可发送消息并排队，在「下一个 completion」之前
//! 把排队消息插入模型输入。
//!
//! 背景：`send_message_stream` 发起一次多轮 agent run 后，模型可能多次调用 LLM
//! （一次 completion = 一次 LLM 调用，中间穿插工具执行）。旧行为下用户只能等
//! 整个 run 结束再发新消息。本模块让用户可以在生成期间继续发送：
//!
//! - [`PendingUserMessages`]：线程安全队列（key = conversation_id）。
//!   Tauri 命令层 `queue_user_message` 先把消息持久化到会话存储，再 push 到队列；
//!   hook / 续接循环负责消费。
//! - [`InjectPendingUserHook`]：rig `AgentHook`，监听 `CompletionCall` 事件
//!   （每次 LLM 调用**之前**触发），把排队消息追加到本轮 history 末尾，
//!   通过 [`RequestPatch::history`] 只改"本轮发送给 provider 的消息"，
//!   不污染 rig 内部 transcript，也不影响 RAG 检索文本（检索仍基于原始 history）。
//!
//! 消费路径（两者互补，消息不会重复处理）：
//! 1. **中途注入**：run 仍在进行（如工具调用后还有下一次 completion）时，hook 在
//!    下一个 completion 前 drain 队列并注入 → 模型下一轮直接看到用户的新指示。
//! 2. **结束后续接**：若 run 已结束而队列仍有未注入消息（例如第一轮纯文本无工具
//!    调用，不会再有下一次 completion），`send_message_stream` 的续接循环会把它们
//!    并入新的一轮对话（消息已持久化到 store，重建完整历史即可）。
//!
//! 只在 `turn >= 2` 注入：第一轮 completion 在用户可能排队之前就已开始发送，
//! 排队消息只应影响"下一个 completion"（通常是从第二轮起的续接轮）。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use rig_core::agent::{
    AgentBuilder, AgentHook, Flow, HookContext, RequestPatch, StepEvent, StepEventKind,
};
use rig_core::completion::CompletionModel;
use rig_core::message::Message;
use tokio::sync::RwLock;

/// 按会话缓存的待注入用户消息队列（线程安全）。
///
/// 语义：`queue_user_message` 命令**先**持久化到会话存储、**再** push 到这里，
/// 因此队列中的消息一定已在 store 中；消费方（hook 注入 / 续接轮）可安全地从
/// store 重建完整历史而不丢失消息。
#[derive(Clone, Default)]
pub struct PendingUserMessages {
    inner: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
}

impl PendingUserMessages {
    /// 创建空队列。
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一条待注入消息（内容为完整用户输入，含引用前缀）。
    pub fn push(&self, conversation_id: &str, content: String) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(conversation_id.to_string())
            .or_default()
            .push_back(content);
    }

    /// 是否有待注入消息。
    pub fn has_pending(&self, conversation_id: &str) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(conversation_id)
            .is_some_and(|q| !q.is_empty())
    }

    /// 待注入消息条数。
    pub fn pending_count(&self, conversation_id: &str) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(conversation_id)
            .map_or(0, VecDeque::len)
    }

    /// 取出并清空该会话的全部待注入消息（按入队顺序返回）。
    pub fn drain_all(&self, conversation_id: &str) -> Vec<String> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(conversation_id)
            .map_or_else(Vec::new, |q| q.into_iter().collect())
    }

    /// 清空该会话的待注入队列（消息已在 store 中，不会丢失）。
    pub fn clear(&self, conversation_id: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(conversation_id);
    }
}

/// 在每次 LLM completion 调用前，把该会话排队中的用户消息注入模型输入。
///
/// 只在 `turn >= 2` 时注入（见模块文档）。通过 `RequestPatch::history` 把排队
/// 消息追加到本轮 history 末尾，使模型在下一个 completion 时看到用户的新指示，
/// 但不修改 rig 持久化 transcript。
pub struct InjectPendingUserHook {
    pending: Arc<PendingUserMessages>,
    current_conversation_id: Arc<RwLock<Option<String>>>,
}

impl InjectPendingUserHook {
    /// 构造注入 hook。
    pub fn new(
        pending: Arc<PendingUserMessages>,
        current_conversation_id: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            pending,
            current_conversation_id,
        }
    }
}

impl<M: CompletionModel> AgentHook<M> for InjectPendingUserHook {
    async fn on_event(&self, _ctx: &HookContext, event: StepEvent<'_, M>) -> Flow {
        let StepEvent::CompletionCall { history, turn, .. } = event else {
            return Flow::cont();
        };
        // 第一轮不注入：本轮请求在用户可能排队之前就已开始。
        if turn <= 1 {
            return Flow::cont();
        }
        // 读取当前会话 id，取走该会话排队中的消息（无当前会话则不注入）。
        let Some(conv_id) = self.current_conversation_id.read().await.clone() else {
            return Flow::cont();
        };
        let queued = self.pending.drain_all(&conv_id);
        if queued.is_empty() {
            return Flow::cont();
        }
        // 把排队消息追加到本轮 history 末尾（紧邻 prompt 之前 = 下一个 completion 之前）。
        let mut history = history.to_vec();
        history.extend(queued.into_iter().map(Message::user));
        Flow::patch_request(RequestPatch::new().history(history))
    }

    fn observes(&self, kind: StepEventKind) -> bool {
        matches!(kind, StepEventKind::CompletionCall)
    }
}

/// 在 `AgentBuilder` 上按需附加用户中断注入 hook。
///
/// `pending` 为 `Some` 时注册 hook（生成中排队功能启用）；`None` 时原样返回 builder。
/// 泛型于 ToolState（`NoToolConfig` / `WithBuilderTools`），两种状态下均可附加。
pub fn attach_user_inject_hook<M: CompletionModel, TS>(
    builder: AgentBuilder<M, TS>,
    pending: Option<Arc<PendingUserMessages>>,
    current_conversation_id: Arc<RwLock<Option<String>>>,
) -> AgentBuilder<M, TS> {
    match pending {
        Some(p) => builder.add_hook(InjectPendingUserHook::new(p, current_conversation_id)),
        None => builder,
    }
}

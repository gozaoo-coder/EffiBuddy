//! sub_agent 工具 + SubAgentManager：让主 agent 召唤子 agent 多轮对话
//!
//! 全流程：
//! 1. 主 agent 调用 `sub_agent`（可指定模型/指令/工具白名单/会话 id）
//! 2. 管理器按 session_id 创建或复用子 agent 会话（独立消息历史，不落盘）
//! 3. 子 agent 以流式执行任务（可调用工具），全过程通过事件回调实时推送：
//!    started → token / tool_call / tool_result / attachment → done | error
//! 4. 最终回复文本返回给主 agent，同时前端展示完整的子 agent 过程卡片
//!
//! 约束：
//! - 嵌套深度上限（默认 2 层），用 AtomicUsize 计数（进入 +1，退出 -1）
//! - 会话数上限（默认 16，超出淘汰最久未用）；单会话消息上限（默认 40）
//! - 会话按 session_id 复用实现多轮；close=true 可显式关闭
//! - 子 agent 默认排除 set_title / display_image / image_gen（避免污染主会话），
//!   显式传 tools 列表可覆盖

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use effisuite_core::{
    AgentConfig, ClawHubClient, CompressionStore, ConversationStore, MemoryIndex, Message,
    PinnedMemoryStore, PluginStore, Role, SkillIndex, SkillStore,
};
use futures::StreamExt;
use rig_core::tool::Tool;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use super::image_gen::ImageGenConfig;
use super::model_manager::ModelManagerHandle;
use crate::agent::{AgentStreamItem, ChatAgent};
use crate::agent_pool::{AgentPoolStore, PoolEntry, PoolKind, PoolStatus};
use crate::rig_agent::RigAgent;

/// 嵌套深度上限：主 agent=0，子 agent=1，孙 agent=2，再深拒绝
const DEFAULT_MAX_DEPTH: usize = 2;
/// 同时存活会话数上限（超出淘汰最久未用的）
const DEFAULT_MAX_SESSIONS: usize = 16;
/// 单会话消息数上限（超出丢弃最早的消息）
const DEFAULT_MAX_MESSAGES: usize = 40;

/// 子 agent 的共享句柄集合（Tauri 层一次性构造注入）
pub struct SubAgentKit {
    pub memory: Option<Arc<MemoryIndex>>,
    pub pinned_memory: Option<Arc<PinnedMemoryStore>>,
    pub current_conversation_id: Arc<RwLock<Option<String>>>,
    pub working_dir: Arc<RwLock<Option<PathBuf>>>,
    pub image_gen_config: Arc<RwLock<Option<ImageGenConfig>>>,
    pub attachments_dir: PathBuf,
    pub store: Arc<ConversationStore>,
    pub skill_index: Option<Arc<SkillIndex>>,
    pub skill_store: Option<SkillStore>,
    pub clawhub_client: Option<ClawHubClient>,
    pub skills_dir: Option<PathBuf>,
    pub plugin_store: Option<PluginStore>,
    pub compression_store: Option<CompressionStore>,
    /// 模型管理句柄（子 agent 也可管理模型列表）
    /// `Arc<RwLock<Arc<AgentConfig>>>` 快照模式：读 clone Arc（廉价）
    pub model_config: Arc<RwLock<Arc<AgentConfig>>>,
    /// 模型管理句柄（子 agent 也可管理模型列表）
    pub model_manager: Option<Arc<ModelManagerHandle>>,
    /// 运行时 agent 公共会话交流池存储：子 agent 创建时自动加入交流池并按
    /// 长任务周期上报状态（开始→进行中，完成→已完成）。None 时不参与交流池。
    pub agent_pool: Option<AgentPoolStore>,
}

/// 子 agent 事件：经 Tauri 层转发为前端 `sub-agent-event`
#[derive(Debug, Clone, Serialize)]
pub struct SubAgentEvent {
    /// 主会话 conversation_id（前端据此过滤）
    /// 主会话 conversation_id（前端据此过滤）
    pub conversation_id: String,
    /// 子 agent 会话 id
    pub session_id: String,
    /// 显示名
    pub name: String,
    /// 模型名
    pub model: String,
    /// 嵌套深度（1=主 agent 直接召唤，2=子 agent 再召唤）
    pub depth: usize,
    /// 事件类型
    pub kind: SubAgentEventKind,
    /// 文本增量 / 工具结果 / 错误信息 / 附件 JSON
    pub content: String,
    /// 工具名（tool_call / tool_result 时有效）
    pub tool_name: String,
    /// 工具参数 JSON（tool_call 时有效）
    pub arguments: String,
    /// 是否为错误结果
    pub is_error: bool,
}

/// 子 agent 事件类型
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubAgentEventKind {
    /// 会话开始（content=任务 prompt）
    Started,
    /// 回复文本增量
    Token,
    /// 工具调用开始
    ToolCall,
    /// 工具执行结果
    ToolResult,
    /// 生成图片附件（content=ImageGenOutput JSON，前端可 read_attachment 渲染）
    Attachment,
    /// 会话完成（content=最终回复全文）
    Done,
    /// 出错中断
    Error,
}

/// 工具参数
#[derive(Deserialize)]
pub struct SubAgentArgs {
    /// 交给子 agent 的任务
    pub prompt: String,
    /// 会话 id：留空自动生成；复用同一 id 可多轮继续；close=true 时执行后关闭
    #[serde(default)]
    pub session_id: Option<String>,
    /// 显示名（前端卡片与日志展示）
    #[serde(default)]
    pub name: Option<String>,
    /// 子 agent 使用的模型 id（manage_model list 可查看）；缺省与主 agent 相同
    #[serde(default)]
    pub model_id: Option<String>,
    /// 任务指令（系统提示词追加内容）
    #[serde(default)]
    pub instructions: Option<String>,
    /// 工具白名单：缺省=默认工具集（排除 set_title/display_image/image_gen）；
    /// 空数组=无工具；数组=仅列出的工具
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    /// 执行完毕后关闭会话（释放内存）
    #[serde(default)]
    pub close: Option<bool>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("sub_agent error: {0}")]
pub struct SubAgentError(String);

/// 子 agent 会话（内存态，不落盘）
struct SubAgentSession {
    name: String,
    model_name: String,
    agent: Arc<RigAgent>,
    messages: Vec<Message>,
    depth: usize,
    last_active: u64,
}

/// 事件上下文：标识子 agent 会话（emit 的元数据来源）
struct EventCtx<'a> {
    session_id: &'a str,
    name: &'a str,
    model: &'a str,
    depth: usize,
}

/// 子 agent 管理器：会话注册表 + 嵌套深度计数 + 事件推送
pub struct SubAgentManager {
    kit: SubAgentKit,
    sessions: RwLock<HashMap<String, SubAgentSession>>,
    depth: AtomicUsize,
    max_depth: usize,
    max_sessions: usize,
    max_messages: usize,
    /// 事件回调（Tauri 层：app_handle.emit("sub-agent-event", ev)）
    emitter: Box<dyn Fn(&SubAgentEvent) + Send + Sync>,
}

/// 当前 Unix 毫秒时间戳
/// 当前 Unix 毫秒时间戳
fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

/// 截断文本到指定字符数（子 agent 交流池上报用，避免研究报告过长撑爆 pool.json）
fn truncate_for_pool(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

impl SubAgentManager {
    pub fn new(kit: SubAgentKit, emitter: Box<dyn Fn(&SubAgentEvent) + Send + Sync>) -> Self {
        Self {
            kit,
            sessions: RwLock::new(HashMap::new()),
            depth: AtomicUsize::new(0),
            max_depth: DEFAULT_MAX_DEPTH,
            max_sessions: DEFAULT_MAX_SESSIONS,
            max_messages: DEFAULT_MAX_MESSAGES,
            emitter,
        }
    }

    /// 当前嵌套深度
    pub fn current_depth(&self) -> usize {
        self.depth.load(Ordering::SeqCst)
    }

    /// 运行一次子 agent 调用（深度守卫：进入 +1，退出 -1）
    pub async fn run(self: &Arc<Self>, args: &SubAgentArgs) -> Result<String, SubAgentError> {
        if args.prompt.trim().is_empty() {
            return Err(SubAgentError("prompt 不能为空".into()));
        }
        let current = self.depth.load(Ordering::SeqCst);
        if current >= self.max_depth {
            return Err(SubAgentError(format!(
                "子 agent 嵌套深度已达上限 {}({current})，禁止继续嵌套。请在主 agent 层直接处理或合并任务。",
                self.max_depth
            )));
        }
        self.depth.fetch_add(1, Ordering::SeqCst);
        let result = self.run_inner(args).await;
        self.depth.fetch_sub(1, Ordering::SeqCst);
        result
    }

    /// 实际执行：获取/创建会话 → 流式运行 → 回写历史 → 事件推送
    async fn run_inner(self: &Arc<Self>, args: &SubAgentArgs) -> Result<String, SubAgentError> {
        let session_id = match args.session_id.as_deref().map(str::trim) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => format!("sa_{}", &uuid::Uuid::new_v4().to_string()[..8]),
        };
        let close = args.close.unwrap_or(false);

        // 1. 获取或创建会话（短暂持有写锁）
        let (agent, name, model_name, depth, mut messages) = {
            let mut sessions = self.sessions.write().await;
            if let Some(s) = sessions.get_mut(&session_id) {
                s.last_active = now_ms();
                (
                    Arc::clone(&s.agent),
                    s.name.clone(),
                    s.model_name.clone(),
                    s.depth,
                    s.messages.clone(),
                )
            } else {
                if sessions.len() >= self.max_sessions {
                    let oldest = sessions
                        .iter()
                        .min_by_key(|(_, s)| s.last_active)
                        .map(|(k, _)| k.clone());
                    if let Some(k) = oldest {
                        sessions.remove(&k);
                    }
                }
                let depth = self.depth.load(Ordering::SeqCst);
                let name = args
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("子 agent · {}", &session_id[3..8]));
                  let (agent, model_name) = self.build_sub_agent(args, &name, &session_id, depth).await?;
                let session = SubAgentSession {
                    name: name.clone(),
                    model_name: model_name.clone(),
                    agent: Arc::clone(&agent),
                    messages: Vec::new(),
                    depth,
                    last_active: now_ms(),
                };
                sessions.insert(session_id.clone(), session);
                (agent, name, model_name, depth, Vec::new())
            }
        };

        // 2.0. 子 agent 自动登记交流池：长任务开始 → 进行中（若有交流池）
        //     agent_id = sa:<session_id>，conversation_id = 主会话 id，kind = SubAgent。
        //     其他 agent 可 pool_lookup 到本子任务、pool_at @ 询问状态。
        if let Some(pool) = &self.kit.agent_pool {
            let conv_id = self
                .kit
                .current_conversation_id
                .read()
                .await
                .clone()
                .unwrap_or_default();
            pool.register(PoolEntry {
                agent_id: PoolEntry::sub_agent_id(&session_id),
                conversation_id: conv_id,
                name: name.clone(),
                kind: PoolKind::SubAgent,
                task: args.prompt.clone(),
                research_report: String::new(),
                todo_summary: String::new(),
                status: PoolStatus::InProgress,
                last_report: format!("子 agent「{name}」已开始执行任务"),
                created_at: 0,
                updated_at: 0,
                inbox: Vec::new(),
            })
            .await
            .ok();
        }
        // 2. 追加用户消息并推送 started
        messages.push(Message::new(
            effisuite_core::gen_message_id(),
            Role::User,
            args.prompt.clone(),
            now_ms(),
        ));
        let ctx = EventCtx {
            session_id: &session_id,
            name: &name,
            model: &model_name,
            depth,
        };
        self.emit(SubAgentEventKind::Started, &ctx, args.prompt.clone(), "", "", false)
            .await;

        // 3. 流式运行（锁外，避免嵌套 sub_agent 写锁死锁）
        let mut stream = agent.chat_stream(&messages);
        let mut full = String::with_capacity(256);
        // 跟踪 call_id → 工具名（ToolResult 事件需要展示工具名）
        let mut tool_names: HashMap<String, String> = HashMap::new();
        while let Some(item) = stream.next().await {
            match item {
                Ok(AgentStreamItem::Text { content }) => {
                    full.push_str(&content);
                    self.emit(SubAgentEventKind::Token, &ctx, content, "", "", false)
                        .await;
                }
                Ok(AgentStreamItem::ToolCallStart {
                    call_id,
                    tool_name,
                    arguments,
                    ..
                }) => {
                    tool_names.insert(call_id, tool_name.clone());
                    self.emit(
                        SubAgentEventKind::ToolCall,
                        &ctx,
                        "",
                        &tool_name,
                        &arguments.to_string(),
                        false,
                    )
                    .await;
                }
                Ok(AgentStreamItem::ToolResult {
                    call_id,
                    output,
                    is_error,
                    ..
                }) => {
                    let tool_name = tool_names.get(&call_id).cloned().unwrap_or_default();
                    // image_gen 工具结果：解析出附件信息，额外推送 attachment 事件
                    if output.trim_start().starts_with('{') {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&output) {
                            if v.get("path").is_some() && v.get("id").is_some() {
                                self.emit(
                                    SubAgentEventKind::Attachment,
                                    &ctx,
                                    output.clone(),
                                    "",
                                    "",
                                    false,
                                )
                                .await;
                            }
                        }
                    }
                    self.emit(
                        SubAgentEventKind::ToolResult,
                        &ctx,
                        output,
                        &tool_name,
                        "",
                        is_error,
                    )
                    .await;
                }
                Ok(AgentStreamItem::Reasoning { .. })
                | Ok(AgentStreamItem::Usage { .. }) => {
                    // 子 agent 的推理与 token 统计暂不推送
                }
                Err(e) => {
                    let err_text = e.to_string();
                    self.emit(
                        SubAgentEventKind::Error,
                        &ctx,
                        err_text.clone(),
                        "",
                        "",
                        true,
                    )
                    .await;
                    // 交流池：出错也上报为已完成（last_report 记录错误摘要，避免卡在"进行中"）
                    if let Some(pool) = &self.kit.agent_pool {
                        pool.update(
                            &PoolEntry::sub_agent_id(&session_id),
                            args.prompt.clone(),
                            String::new(),
                            String::new(),
                            PoolStatus::Completed,
                            format!(
                                "子 agent「{name}」执行出错：{}",
                                truncate_for_pool(&err_text, 300)
                            ),
                        )
                        .await
                        .ok();
                    }
                    return Err(SubAgentError(format!(
                        "子 agent「{name}」执行失败: {err_text}"
                    )));
                }
            }
        }

        // 4. 回写会话历史（截断到上限）
        // 先释放 stream 对 messages 的借用
        drop(stream);
        messages.push(Message::new(
            effisuite_core::gen_message_id(),
            Role::Assistant,
            full.clone(),
            now_ms(),
        ));
        if messages.len() > self.max_messages {
            messages.drain(0..messages.len() - self.max_messages);
        }
        {
            let mut sessions = self.sessions.write().await;
            if close {
                sessions.remove(&session_id);
            } else if let Some(s) = sessions.get_mut(&session_id) {
                s.messages = messages;
                s.last_active = now_ms();
            }
        }

        // 4.5. 子 agent 自动上报交流池：完成 → 已完成（最终文本摘要作为 last_report）
        if let Some(pool) = &self.kit.agent_pool {
            let summary = truncate_for_pool(&full, 500);
            let report = if summary.is_empty() {
                format!("子 agent「{name}」已执行完成")
            } else {
                format!("子 agent「{name}」已完成：{summary}")
            };
            pool.update(
                &PoolEntry::sub_agent_id(&session_id),
                args.prompt.clone(),
                String::new(),
                String::new(),
                PoolStatus::Completed,
                report,
            )
            .await
            .ok();
        }

        // 5. 推送完成事件并返回最终文本
        self.emit(SubAgentEventKind::Done, &ctx, full.clone(), "", "", false)
            .await;
        Ok(format!(
            "[子 agent「{name}」完成：会话 {session_id}，模型 {model_name}，嵌套深度 {depth}]\n{full}"
        ))
    }

    /// 构造子 agent 的 RigAgent：解析模型配置 + 组装 preamble + 应用工具白名单 +
    /// 注入运行时 agent 公共会话交流池（agent_pool + 子 agent 身份）。
    async fn build_sub_agent(
        self: &Arc<Self>,
        args: &SubAgentArgs,
        name: &str,
        session_id: &str,
        depth: usize,
    ) -> Result<(Arc<RigAgent>, String), SubAgentError> {
        let (api_key, base_url, model_name, model_tools) = {
            let config = self.kit.model_config.read().await;
            resolve_model_config(&config, args.model_id.as_deref()).map_err(SubAgentError)?
        };

        let preamble = build_sub_agent_preamble(name, args.instructions.as_deref(), depth);
        // 工具白名单：显式空数组 = 无工具
        let tools_disabled = matches!(args.tools.as_deref(), Some(t) if t.is_empty());
        let allowlist = args.tools.clone();
        let enable_tools = !tools_disabled && model_tools;

        let mut agent = RigAgent::from_key(
            &api_key,
            &base_url,
            &model_name,
            &preamble,
            enable_tools,
            self.kit.memory.clone(),
            self.kit.pinned_memory.clone(),
            Arc::clone(&self.kit.current_conversation_id),
            Arc::clone(&self.kit.working_dir),
            Arc::clone(&self.kit.image_gen_config),
            self.kit.attachments_dir.clone(),
            Arc::clone(&self.kit.store),
            self.kit.skill_index.clone(),
            self.kit.skill_store.clone(),
            self.kit.clawhub_client.clone(),
            self.kit.skills_dir.clone(),
            self.kit.plugin_store.clone(),
            self.kit.compression_store.clone(),
            self.kit.model_manager.clone(),
            Some(Arc::clone(self)),
        )
        .map_err(|e| SubAgentError(format!("子 agent 构造失败: {e}")))?
        .with_tool_allowlist(allowlist)
        .with_excluded_tools(
            crate::rig_agent::SUB_AGENT_DEFAULT_EXCLUDED
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );

        // 注入运行时 agent 公共会话交流池：子 agent 拥有 pool_* 工具与
        // `[Agent 交流池]` 上下文段，agent_id 按 `sa:<session_id>` 推导，显示名用子 agent 名。
        agent = agent
            .with_agent_pool(self.kit.agent_pool.clone())
            .with_pool_sub_agent_identity(Some(session_id.to_string()), Some(name.to_string()));

        Ok((Arc::new(agent), model_name))
    }

    /// 推送事件：conversation_id 取自共享句柄（与主会话一致，前端据此过滤）
    async fn emit(
        &self,
        kind: SubAgentEventKind,
        ctx: &EventCtx<'_>,
        content: impl Into<String>,
        tool_name: impl Into<String>,
        arguments: impl Into<String>,
        is_error: bool,
    ) {
        let conversation_id = self
            .kit
            .current_conversation_id
            .read()
            .await
            .clone()
            .unwrap_or_default();
        let ev = SubAgentEvent {
            conversation_id,
            session_id: ctx.session_id.to_string(),
            name: ctx.name.to_string(),
            model: ctx.model.to_string(),
            depth: ctx.depth,
            kind,
            content: content.into(),
            tool_name: tool_name.into(),
            arguments: arguments.into(),
            is_error,
        };
        (self.emitter)(&ev);
    }
}

/// 解析子 agent 使用的模型配置：model_id > 当前激活对话模型 > 运行时内联配置
///
/// 返回 (api_key, base_url, model_name, enable_tools)
fn resolve_model_config(
    config: &AgentConfig,
    model_id: Option<&str>,
) -> Result<(String, String, String, bool), String> {
    if let Some(id) = model_id {
        let m = config
            .models
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| format!("模型 {id} 不存在（manage_model list 可查看可用 id）"))?;
        if m.kind != effisuite_core::ModelKind::Chat {
            return Err(format!(
                "模型 {id} 不是对话模型（kind={:?}），子 agent 只能使用 kind=chat 的模型",
                m.kind
            ));
        }
        if m.api_key.trim().is_empty() {
            return Err(format!("模型 {id} 未配置 api_key"));
        }
        return Ok((
            m.api_key.clone(),
            m.base_url.clone(),
            m.model_name.clone(),
            m.enable_tools,
        ));
    }
    if let Some(active_id) = config.active_model_id.as_ref() {
        if let Some(m) = config.models.iter().find(|m| &m.id == active_id) {
            if !m.api_key.trim().is_empty() {
                return Ok((
                    m.api_key.clone(),
                    m.base_url.clone(),
                    m.model_name.clone(),
                    m.enable_tools,
                ));
            }
        }
    }
    if config.api_key.trim().is_empty() {
        return Err("未配置任何可用的对话模型（api_key 为空）".to_string());
    }
    Ok((
        config.api_key.clone(),
        config.base_url.clone(),
        config.model_name.clone(),
        config.enable_tools,
    ))
}

/// 组装子 agent 系统提示词：角色定位 + 任务要求
fn build_sub_agent_preamble(name: &str, instructions: Option<&str>, depth: usize) -> String {
    let mut p = format!(
        "你是被主 agent 召唤的子 agent「{name}」（嵌套层级 {depth}）。你的任务由主 agent 下达。\n\
         执行准则：\n\
         1. 直接执行任务，不要寒暄，不要复述任务。\n\
         2. 需要读取文件、搜索代码、执行命令时，主动调用对应工具。\n\
         3. 若任务超出能力或信息不足，明确说明缺什么。\n\
         4. 完成时用简洁的最终答复总结结果（主 agent 会直接读取你的最终回复）。"
    );
    if let Some(i) = instructions.map(str::trim).filter(|s| !s.is_empty()) {
        p.push_str("\n\n【任务要求】\n");
        p.push_str(i);
    }
    p
}

/// 子 agent 工具
pub struct SubAgentTool {
    manager: Arc<SubAgentManager>,
}

impl SubAgentTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

impl Tool for SubAgentTool {
    const NAME: &'static str = "sub_agent";

    type Error = SubAgentError;
    type Args = SubAgentArgs;
    type Output = String;

    fn description(&self) -> String {
        "召唤一个子 agent 执行任务并返回其最终回复。子 agent 拥有独立的会话历史\
         （可多轮继续）、可调用工具、可使用其他模型（model_id），全过程会在前端\
         以卡片形式展示。\n\
         - 复杂任务可拆给子 agent 并行/独立完成（如代码审查、独立调研、文案起草）\n\
         - 复用同一 session_id 可让子 agent 记住上下文继续对话\n\
         - 嵌套深度上限 2 层，禁止无限递归\n\
         - 单轮简单询问用 call_model 更轻量"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "交给子 agent 的任务"
                },
                "session_id": {
                    "type": "string",
                    "description": "会话 id：留空自动生成；复用同一 id 可多轮继续"
                },
                "name": {
                    "type": "string",
                    "description": "显示名，如 '代码审查员'"
                },
                "model_id": {
                    "type": "string",
                    "description": "子 agent 使用的模型 id（manage_model list 可查看）；缺省与主 agent 相同"
                },
                "instructions": {
                    "type": "string",
                    "description": "任务指令（系统提示词追加内容），如 '只审查安全问题'"
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "工具白名单：缺省=默认工具集；空数组=无工具"
                },
                "close": {
                    "type": "boolean",
                    "description": "执行完毕后关闭会话，默认 false",
                    "default": false
                }
            },
            "required": ["prompt"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.manager.run(&args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preamble_includes_name_instructions_and_depth() {
        let p = build_sub_agent_preamble("审查员", Some("只关注安全问题"), 1);
        assert!(p.contains("审查员"));
        assert!(p.contains("只关注安全问题"));
        assert!(p.contains("嵌套层级 1"));
    }

    #[test]
    fn preamble_skips_empty_instructions() {
        let p = build_sub_agent_preamble("A", None, 2);
        assert!(!p.contains("【任务要求】"));
        assert!(p.contains("嵌套层级 2"));
    }

    #[tokio::test]
    async fn depth_guard_rejects_deep_nesting() {
        // 无 api_key 配置时 build 会失败，但深度守卫应先于 build 触发
        let kit = SubAgentKit {
            memory: None,
            pinned_memory: None,
            current_conversation_id: Arc::new(RwLock::new(None)),
            working_dir: Arc::new(RwLock::new(None)),
            image_gen_config: Arc::new(RwLock::new(None)),
            attachments_dir: PathBuf::from("."),
            store: Arc::new(
                ConversationStore::new(std::env::temp_dir().join(format!(
                    "effisuite-subagent-test-{}",
                    uuid::Uuid::new_v4()
                )))
                .unwrap(),
            ),
            skill_index: None,
            skill_store: None,
            clawhub_client: None,
            skills_dir: None,
            plugin_store: None,
            compression_store: None,
            model_config: Arc::new(RwLock::new(Arc::new(AgentConfig::default()))),
            model_manager: None,
            agent_pool: None,
        };
        let manager = Arc::new(SubAgentManager::new(kit, Box::new(|_| {})));
        // 直接推高深度计数模拟嵌套
        manager.depth.store(manager.max_depth, Ordering::SeqCst);
        let r = manager
            .run(&SubAgentArgs {
                prompt: "hi".to_string(),
                session_id: None,
                name: None,
                model_id: None,
                instructions: None,
                tools: None,
                close: None,
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("嵌套深度"));
    }

    #[tokio::test]
    async fn empty_prompt_rejected() {
        let kit = SubAgentKit {
            memory: None,
            pinned_memory: None,
            current_conversation_id: Arc::new(RwLock::new(None)),
            working_dir: Arc::new(RwLock::new(None)),
            image_gen_config: Arc::new(RwLock::new(None)),
            attachments_dir: PathBuf::from("."),
            store: Arc::new(
                ConversationStore::new(std::env::temp_dir().join(format!(
                    "effisuite-subagent-test-{}",
                    uuid::Uuid::new_v4()
                )))
                .unwrap(),
            ),
            skill_index: None,
            skill_store: None,
            clawhub_client: None,
            skills_dir: None,
            plugin_store: None,
            compression_store: None,
            model_config: Arc::new(RwLock::new(Arc::new(AgentConfig::default()))),
            model_manager: None,
            agent_pool: None,
        };
        let manager = Arc::new(SubAgentManager::new(kit, Box::new(|_| {})));
        let r = manager
            .run(&SubAgentArgs {
                prompt: "   ".to_string(),
                session_id: None,
                name: None,
                model_id: None,
                instructions: None,
                tools: None,
                close: None,
            })
            .await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn missing_model_errors() {
        let mut config = AgentConfig::default();
        config.api_key = "sk-test".to_string();
        config.base_url = "https://api.openai.com/v1".to_string();
        config.model_name = "gpt-4o-mini".to_string();
        let kit = SubAgentKit {
            memory: None,
            pinned_memory: None,
            current_conversation_id: Arc::new(RwLock::new(None)),
            working_dir: Arc::new(RwLock::new(None)),
            image_gen_config: Arc::new(RwLock::new(None)),
            attachments_dir: PathBuf::from("."),
            store: Arc::new(
                ConversationStore::new(std::env::temp_dir().join(format!(
                    "effisuite-subagent-test-{}",
                    uuid::Uuid::new_v4()
                )))
                .unwrap(),
            ),
            skill_index: None,
            skill_store: None,
            clawhub_client: None,
            skills_dir: None,
            plugin_store: None,
            compression_store: None,
            model_config: Arc::new(RwLock::new(Arc::new(config))),
            model_manager: None,
            agent_pool: None,
        };
        let manager = Arc::new(SubAgentManager::new(kit, Box::new(|_| {})));
        let r = manager
            .run(&SubAgentArgs {
                prompt: "hi".to_string(),
                session_id: None,
                name: None,
                model_id: Some("missing-id".to_string()),
                instructions: None,
                tools: None,
                close: None,
            })
            .await;
        // 应报"模型不存在"（而非 api_key 错误）
        let err = r.unwrap_err().to_string();
        assert!(err.contains("missing-id"), "err: {err}");
    }
}

// 与 Rust 侧 serde 输出对应的类型定义。
// Role: #[serde(rename_all = "lowercase")] -> "system" | "user" | "assistant"
// DeviceStatus: #[serde(rename_all = "snake_case")] -> "discovered" | "paired" | "offline" | "pairing"
// BusEvent: #[serde(tag = "kind", rename_all = "snake_case")]，每个事件携带 kind 标签。

export type Role = 'system' | 'user' | 'assistant'

export type DeviceStatus = 'discovered' | 'paired' | 'offline' | 'pairing'

// ModelKind: #[serde(rename_all = "snake_case")]
export type ModelKind = 'chat' | 'image_gen' | 'video_gen' | 'audio_transcribe'

// AttachmentKind: #[serde(rename_all = "snake_case")]
export type AttachmentKind = 'image' | 'file' | 'audio'

export interface Attachment {
  id: string
  kind: AttachmentKind
  /** 相对 attachments 目录的文件名（如 gen_xxx.png） */
  path: string
  name: string
  mime_type: string
  size: number
}

export interface Message {
  id: string
  content: string
  timestamp: number
  role: Role
  /** 消息附件（图片/文件），旧消息无此字段时为空数组 */
  attachments?: Attachment[]
  /** 助手消息的思考过程（reasoning）全文；新版持久化，旧消息无此字段 */
  reasoning?: string | null
  /** 助手消息的工具调用记录；新版持久化，旧消息无此字段 */
  toolCalls?: ToolCallRecord[]
  /** 助手消息的 token 用量统计；新版持久化，旧消息无此字段 */
  usage?: MessageUsage | null
  /** 助手消息的子 agent 过程记录；新版持久化，旧消息无此字段 */
  subAgents?: SubAgentRecord[]
  }

// 引用块 chip：composer 顶部展示被引用消息的摘要
// send() 时把所有 chips 的内容拼接到用户输入前面，作为上下文交给后端
export interface QuoteChip {
  /** 被引用消息的 id（用于点击 chip 跳转高亮） */
  messageId: string
  /** 摘要文本（前 40 字符 + …），用于 chip 显示 */
  snippet: string
  /** 被引用消息的完整内容，发送时拼接用 */
  content: string
  /** 被引用消息的 role，用于拼接 "[引用消息] 用户(id:xxx): ..." */
  role: Role
}

export interface Device {
  id: string
  name: string
  address: string
  last_seen: number
  status: DeviceStatus
}

// PairRole: #[serde(rename_all = "snake_case")] -> "mirror" | "host" | "replica"
export type PairRole = 'mirror' | 'host' | 'replica'

// 与后端 effisuite_p2p::pairing::PairingRequest 对齐
export interface PairingRequest {
  device_id: string
  name: string
  address: string
  /** 对端 Ed25519 公钥 hex（广播阶段为空，配对握手时交换） */
  pubkey_hex: string
  timestamp: number
}

// 与后端 commands::p2p::P2pStatus 对齐
export interface P2pStatus {
  started: boolean
  self_device_id: string
}

// 后端把整个 BusEvent 作为事件 payload emit 出来，前端按事件名订阅。
export interface AgentMessagePayload {
  kind: 'agent_message'
  conversation_id: string
  content: string
  done: boolean
}

export interface DeviceFoundPayload {
  kind: 'device_found'
  device: Device
}

export interface DeviceStatusChangedPayload {
  kind: 'device_status_changed'
  device_id: string
  status: DeviceStatus
}

export interface PairingRequestPayload {
  kind: 'pairing_request'
  device: Device
}

// =========================================================
// 会话 & 配置（与 core::{Conversation, AgentConfig} 对齐）
// =========================================================

/** 自动归类结果（与后端 AutoClassifyResult 对齐） */
export interface AutoClassifyResult {
  /** LLM 生成的标题（已截断到 25 字符） */
  title: string
  /** 匹配到的已有文件夹名；无匹配时为 null */
  folder: string | null
}

/** 批量删除会话结果：成功数 + 失败 id 列表 */
export interface BatchDeleteResult {
  /** 成功删除的会话数 */
  success: number
  /** 删除失败的会话 id（IO 错误等） */
  failed: string[]
}

export interface Conversation {
  id: string
  messages: Message[]
  device_id: string | null
  created_at: number
  title?: string | null
  pinned?: boolean
  pinned_at?: number | null
  updated_at?: number
  /** 会话级工作区路径，覆盖技能级 working_dir */
  working_dir?: string | null
}

export interface ConversationMeta {
  id: string
  title?: string | null
  pinned: boolean
  pinned_at?: number | null
  created_at: number
  updated_at: number
  message_count: number
}

// =========================================================
// 运行时 agent 公共会话交流池（与 agent::agent_pool::* 对齐）
// =========================================================

// PoolStatus: #[serde(rename_all = "snake_case")]
export type PoolStatus = 'in_progress' | 'waiting' | 'completed'

// PoolKind: #[serde(rename_all = "snake_case")]
export type PoolKind = 'main' | 'sub_agent'

// AtStatus: #[serde(rename_all = "snake_case")]
export type AtStatus = 'pending' | 'answered'

/** @ 消息（目标 agent 收件箱条目） */
export interface AtMessage {
  at_id: string
  from: string
  from_name: string
  question: string
  status: AtStatus
  reply?: string | null
  created_at: number
  answered_at?: number | null
}

/** 交流池条目：一个长任务的完整登记信息（含收件箱 @ 消息） */
export interface PoolEntry {
  agent_id: string
  conversation_id: string
  name: string
  kind: PoolKind
  task: string
  research_report: string
  todo_summary: string
  status: PoolStatus
  last_report: string
  created_at: number
  updated_at: number
  inbox: AtMessage[]
}

/** 会话在交流池中的运行状态（由该会话的全部条目聚合而来） */
export type ConversationPoolStatus = 'idle' | 'in_progress' | 'waiting' | 'completed'
export type BackendKind = 'mock' | 'openai'

// =========================================================
// 搜索 & 文件选择（与后端 search_conversations / pick_file 等对齐）
// =========================================================

export interface SearchHit {
  conversation_id: string
  conversation_title: string
  message_id: string
  snippet: string
  score: number
  timestamp: number
  pinned: boolean
  updated_at: number
}

export interface PickedFile {
  path: string
  name: string
  size: number
}

// =========================================================
// 附件（已上移到文件顶部与 Message 一起定义）
// =========================================================

export type ThemeMode = 'system' | 'light' | 'dark'

export interface CompressionSettings {
  /** 自动压缩阈值（百分比 1-100）：上下文使用达到该比例时自动触发压缩 */
  threshold_percent: number
  /** 是否启用自动压缩（达到阈值时在回复完成后自动压缩历史） */
  auto_compress: boolean
  /** 是否压缩工具调用 / 工具返回 */
  compress_tool_calls: boolean
  /** 是否逐句对话压缩 */
  compress_sentences: boolean
}

export interface AgentConfig {
  backend: BackendKind
  api_key: string
  base_url: string
  model_name: string
  preamble: string
  provider_id: string
  enable_tools: boolean
  theme: ThemeMode
  models: AvailableModel[]
  active_model_id: string | null
  /** 当前激活的图像生成模型 id（独立于 active_model_id） */
  active_image_gen_model_id?: string | null
  /** 对话命名模型 id（auto_classify 用）；None 时回退到 active_model_id */
  title_model_id?: string | null
  /** 会话历史压缩模型 id（compress_messages 用）；None 时回退到 active_model_id */
  compression_model_id?: string | null
  /** 语音实时转文字模型 id；None 时回退到 asr_config 原生配置 */
  asr_stream_model_id?: string | null
  /** 音频转文字模型 id（文件转写）；None 时回退到 asr_config 原生配置 */
  asr_transcribe_model_id?: string | null
  /** 会话历史压缩机制设置 */
  compression_settings?: CompressionSettings
}

export interface AvailableModel {
  id: string
  label: string
  provider_id: string
  base_url: string
  model_name: string
  api_key: string
  preamble: string
  enable_tools: boolean
  /** 模型能力类型：chat（对话）/ image_gen（图像生成）/ video_gen（视频生成，预留）/ audio_transcribe（音频转文字） */
  kind?: ModelKind
  /** 图像生成专用：默认尺寸（如 1024x1024），仅 kind=image_gen 时有效 */
  image_size?: string | null
  /** 图像生成专用：默认质量（如 standard/hd），仅 kind=image_gen 时有效 */
  image_quality?: string | null
  /** 视频生成专用：默认分辨率（如 720p），仅 kind=video_gen 时有效 */
  video_resolution?: string | null
  /** 视频生成专用：默认宽高比（如 16:9），仅 kind=video_gen 时有效 */
  video_ratio?: string | null
  /** 音频转文字专用：默认源语言（如 zh/en/auto），仅 kind=audio_transcribe 时有效 */
  audio_language?: string | null
  /** 模型上下文窗口大小（tokens），null 表示未设置 */
  context_window_tokens?: number | null
  /** 视频生成专用：默认时长（秒，2..=15；null 用模型默认），仅 kind=video_gen 时有效 */
  video_duration?: number | null
  /** 计费单价（元/百万 tokens），null 表示未配置（聊天中不显示消费金额） */
  pricing?: ModelPricing | null
  created_at: number
}

/** 模型计费单价（元/百万 tokens），由用户在模型配置面板填写，不硬编码 */
export interface ModelPricing {
  /** 缓存命中输入单价（元/百万 tokens） */
  cache_hit_per_m: number
  /** 缓存未命中输入单价（元/百万 tokens） */
  cache_miss_per_m: number
  /** 输出单价（元/百万 tokens） */
  output_per_m: number
}

export interface ProviderPreset {
  id: string
  name: string
  default_base_url: string
  default_model: string
  env_var: string
  docs_url: string
  openai_compat: boolean
}

/** 远程模型条目（OpenAI 兼容 /v1/models 响应） */
export interface RemoteModelInfo {
  id: string
  object: string
  owned_by: string
  /** 模型创建时间（Unix 秒），部分 provider 不返回 */
  created: number | null
}

/** 单次/累计 token 使用统计（agent-usage 事件 payload） */
export interface AgentUsagePayload {
  conversation_id: string
  // 本次单次值
  input_tokens: number
  output_tokens: number
  total_tokens: number
  reasoning_tokens: number
  // 本轮累计值
  cumulative_input: number
  cumulative_output: number
  cumulative_total: number
  cumulative_reasoning: number
}

/** 单条助手消息的 token 用量统计（持久化到消息，历史回看用） */
export interface MessageUsage {
  input_tokens: number
  output_tokens: number
  total_tokens: number
  reasoning_tokens: number
  cache_hit_tokens: number
  cache_miss_tokens: number
  /** 处理轮数：该消息所有 completion 次数（含工具调用轮） */
  rounds: number
}
/**
 * 回答结束时的计费统计（agent-billing 事件 payload）
 *
 * 用户发送一次"询问"后，模型可能因工具调用进行多次 Completions；
 * 全部结束（"回答结束"）时后端 emit 一次，前端据此在气泡底部
 * 显示本次询问的最终消费价格，悬浮可查看分项明细。
 */
export interface AgentBillingPayload {
  conversation_id: string
  /** 模型名（agent 实际使用的模型） */
  model_name: string
  /** 处理轮数：本次询问所有 completion 次数（含工具调用轮） */
  rounds: number
  /** 缓存命中输入 token 总数 */
  cache_hit_tokens: number
  /** 缓存未命中输入 token 总数 */
  cache_miss_tokens: number
  /** 输出 token 总数 */
  output_tokens: number
  /** 总 token 数（缓存命中 + 未命中 + 输出） */
  total_tokens: number
  /** 是否已配置计费单价；false 时各 cost 字段为 0，只显示 token */
  priced: boolean
  /** 缓存计费（元） */
  cache_hit_cost: number
  /** 未缓存计费（元） */
  cache_miss_cost: number
  /** 输出计费（元） */
  output_cost: number
  /** 合计消费（元） */
  total_cost: number
}

// =========================================================
// 消息压缩（与 tauriFront/src-tauri/src/lib.rs 中
// CompressTokenPayload / CompressStatusPayload / CompressDonePayload /
// CompressErrorPayload 对齐，与 core::compression::CompressionAction 对齐）
// =========================================================

/** 压缩决策类型（与后端 CompressionAction 枚举的 serde tag 一致） */
export type CompressionMethod = 'keep' | 'hide' | 'replace'

/** 单条压缩决策（后端用 #[serde(tag = "method", rename_all = "lowercase")]） */
export interface CompressionAction {
  method: CompressionMethod
  reason: string
  message_ids: string[]
  /** 仅 method=replace 时存在 */
  new_content?: string
}

/** 压缩状态（一个会话的完整压缩快照） */
export interface CompressionState {
  actions: CompressionAction[]
  updated_at: number
}

/** 压缩阶段标识（agent-compress-status 事件的 stage 字段） */
export type CompressionStage =
  | 'loading_conv'
  | 'building_prompt'
  | 'streaming'
  | 'parsing'
  | 'persisting'
  | 'done'
  | 'error'

/** agent-compress-status 事件 payload */
export interface CompressStatusPayload {
  conversation_id: string
  stage: CompressionStage
  message: string
}

/** agent-compress-token 事件 payload（流式文本增量） */
export interface CompressTokenPayload {
  conversation_id: string
  token: string
}

/** agent-compress-done 事件 payload（完成时携带解析结果与耗时） */
export interface CompressDonePayload {
  conversation_id: string
  actions: CompressionAction[]
  raw_text: string
  elapsed_ms: number
}

/** agent-compress-error 事件 payload（失败时携带错误与已接收部分文本） */
export interface CompressErrorPayload {
  conversation_id: string
  error: string
  partial: string
}

// =========================================================
// 流式事件 payload（与 tauriFront/src-tauri/src/lib.rs 中
// StreamTokenPayload / StreamErrorPayload 对齐）
// =========================================================

export interface StreamTokenPayload {
  conversation_id: string
  content: string
  done: boolean
}

export interface StreamErrorPayload {
  conversation_id: string
  error: string
}

// 推理增量 payload（agent-reasoning 事件）
export interface AgentReasoningPayload {
  conversation_id: string
  content: string
}

// 工具调用开始 payload（agent-tool-call 事件）
export interface AgentToolCallPayload {
  conversation_id: string
  call_id: string
  tool_name: string
  // JSON 字符串形式的参数
  arguments: string
}

// 工具执行结果 payload（agent-tool-result 事件）
export interface AgentToolResultPayload {
  conversation_id: string
  call_id: string
  output: string
  is_error: boolean
}

// 图片附件生成 payload（agent-attachment 事件）
// image_gen 工具成功生成图片时实时 emit，前端收到后立即渲染图片
export interface AgentAttachmentPayload {
  conversation_id: string
  attachment: Attachment
}

// 会话标题更新 payload（conversation-title-updated 事件）
// set_title 工具成功更新标题后实时 emit，前端立即刷新 SideNav 列表
export interface ConversationTitlePayload {
  conversation_id: string
  title: string
}

// 单次工具调用记录（前端聚合 ToolCallStart + ToolResult 后的结构）
export interface ToolCallRecord {
  call_id: string
  tool_name: string
  // 原始 JSON 字符串参数
  arguments: string
  // 执行结果（未到达时为 null）
  result: string | null
  is_error: boolean
  // 是否正在执行中
  pending: boolean
}

// 子 agent 事件 payload（sub-agent-event 事件）
// 后端 SubAgentManager 在子 agent 执行全流程中实时推送：
// started → token / tool_call / tool_result / attachment → done | error
export interface SubAgentEventPayload {
  conversation_id: string
  session_id: string
  name: string
  model: string
  /** 嵌套深度：1 = 主 agent 直接召唤，2 = 子 agent 再召唤 */
  depth: number
  kind: 'started' | 'token' | 'tool_call' | 'tool_result' | 'attachment' | 'done' | 'error'
  /** token 增量 / 工具结果 / 错误信息 / 附件 JSON（ImageGenOutput） */
  content: string
  /** 工具名（tool_call / tool_result 时有效） */
  tool_name: string
  /** 工具参数 JSON（tool_call 时有效） */
  arguments: string
  is_error: boolean
}

// 子 agent 卡片记录（前端按 session_id 聚合事件后的结构）
export interface SubAgentRecord {
  session_id: string
  name: string
  model: string
  depth: number
  status: 'running' | 'done' | 'error'
  /** 主 agent 交给子 agent 的任务 */
  task: string
  /** 子 agent 回复全文（流式累积） */
  text: string
  /** 子 agent 内部工具调用记录 */
  toolCalls: ToolCallRecord[]
  /** 子 agent 生成的图片附件（path + name） */
  images: { path: string; name: string }[]
  /** 错误信息（status=error 时） */
  error: string
  /** 完成时间 */
  finishedAt: number | null
}

// 后台命令会话（shell_session_* 工具 + 前端底栏便签）
// 会话事件（shell-session-event）payload：后端 ShellSessionManager 实时推送
// started → command / output → exited | error
export interface ShellSessionEventPayload {
  /** 当前对话 conversation_id（前端据此过滤） */
  conversation_id: string
  /** 会话短 ID（如 a1b2） */
  session_id: string
  kind: 'started' | 'command' | 'output' | 'exited' | 'error'
  /** 输出行 / 命令文本 / 错误信息 / 退出码 */
  content: string
  is_error: boolean
}

// 命令会话列表条目（list_shell_sessions 命令返回）
export interface ShellSessionInfo {
  id: string
  name: string
  shell: string
  cwd: string
  running: boolean
  last_command: string
  last_active: number
}

// 前端底栏便签聚合后的会话记录
export interface ShellSessionRecord {
  id: string
  name: string
  shell: string
  cwd: string
  running: boolean
  last_command: string
  /** 会话日志行（含命令标记） */
  lines: { kind: 'cmd' | 'out' | 'err' | 'info'; text: string }[]
  /** 最近活跃时间戳 */
  last_active: number
}


// =========================================================
// 技能 & 定时任务（与 core::{Skill, ScheduledTask} 对齐）
// =========================================================

export interface Skill {
  id: string
  name: string
  description: string
  preamble: string
  tools: string[]
  /** 技能级工作区路径，apply_skill 时注入会话（会话级未设置时） */
  working_dir?: string | null
  created_at: number
  builtin: boolean
  /** 技能来源：null = 本地创建；"clawhub" = 从 ClawHub 安装 */
  source?: string | null
  /** ClawHub slug（仅 source="clawhub" 时存在） */
  source_slug?: string | null
  /** ClawHub owner handle */
  source_owner?: string | null
  /** ClawHub 版本字符串 */
  source_version?: string | null
}

export interface ScheduledTask {
  id: string
  name: string
  skill_id: string
  cron: string
  enabled: boolean
  created_at: number
  last_run?: number | null
}

// 后端 scheduler.rs 在任务触发时 emit "scheduled-task-result" 事件的 payload
export interface ScheduledTaskResult {
  task_id: string
  task_name: string
  conversation_id: string
  content: string
  success: boolean
}

// =========================================================
// 永久记忆（与 core::{PinnedMemory, PinnedMemorySource} 对齐）
// =========================================================

// PinnedMemorySource: #[serde(rename_all = "snake_case")]
export type PinnedMemorySource = 'manual' | 'user_request' | 'assistant'

export interface PinnedMemory {
  id: string
  content: string
  // 可选分类标签，如 "preference" / "fact" / "instruction"
  category?: string | null
  created_at: number
  source: PinnedMemorySource
  // 来源会话 id（若通过对话触发），用于审计回溯
  source_conversation_id?: string | null
}

// =========================================================
// 上下文注入预览（与 agent::ContextPreview 对齐）
// =========================================================
//
// 由后端 `get_context_preview` 命令返回，结构化展示当前 agent 对指定会话
// 将注入到 LLM 的完整 prompt 拼装结果，便于在"上下文管理"面板可视化展示。
//
// 字段对应 rig_agent.rs 中 ContextPreview 结构（snake_case 序列化）。
export interface ContextPreview {
  /** 当前激活 agent 的系统提示词（preamble） */
  preamble: string
  /** `[永久记忆]` 段格式化字符串（含头部说明），空表示无永久记忆 */
  pinned_section: string
  /** `[相关历史记忆]` 段格式化字符串（含头部说明），空表示无 RAG 命中 */
  memory_section: string
  /** `[当前对话最近]` 段格式化字符串（含头部说明），空表示无历史 */
  history_section: string
  /** 当前用户问题文本（最后一条 user 消息） */
  current_question: string
  /** 拼装后的完整 prompt（与实际发给 LLM 的内容一致） */
  full_prompt: string
  /** 永久记忆条目数 */
  pinned_count: number
  /** RAG 命中条目数 */
  memory_hits_count: number
  /** 当前对话历史保留的消息条数（已应用窗口截断） */
  history_keep_count: number
  /** 当前对话总消息条数（包含当前问题） */
  history_total_count: number
  /** 自动注入的相关历史记忆条数上限 */
  memory_inject_limit: number
  /** 启用记忆增强时当前对话保留的最近消息条数 */
  recent_history_limit: number
  /** 单条历史消息截断字符数 */
  history_truncate_chars: number
  /** 是否启用了 RAG 跨会话记忆增强 */
  memory_enabled: boolean
}

// =========================================================
// ClawHub（与 core::clawhub 与 core::InstalledPlugin 对齐）
// =========================================================
//
// 用于前端浏览 / 搜索 / 安装 ClawHub 技能与插件。
// 字段对应后端 clawhub.rs 中的响应结构（snake_case 序列化）。

/** ClawHub 技能最新版本信息 */
export interface SkillLatestVersion {
  version: string
  created_at?: number
  changelog?: string
}

/** ClawHub 所有者信息 */
export interface ClawHubOwner {
  handle?: string | null
  display_name?: string | null
  image?: string | null
}

/** ClawHub 安全审核信息 */
export interface ClawHubModeration {
  is_suspicious?: boolean
  is_malware_blocked?: boolean
  verdict?: string | null
  reason_codes?: string[]
  summary?: string | null
}

/** `GET /api/v1/skills` 列表项 */
export interface SkillListItem {
  slug: string
  display_name: string
  summary?: string | null
  topics?: string[]
  /** 版本 tag 映射，结构松散，保留为原始 JSON */
  tags?: unknown
  /** 统计信息（downloads/stars 等） */
  stats?: unknown
  created_at?: number
  updated_at?: number
  latest_version?: SkillLatestVersion | null
}

/** `GET /api/v1/skills` 响应 */
export interface SkillListResponse {
  items: SkillListItem[]
  next_cursor?: string | null
}

/** 技能详情（比 ListItem 多出 moderation 等字段） */
export interface SkillDetail {
  slug: string
  display_name: string
  summary?: string | null
  tags?: unknown
  stats?: unknown
  created_at?: number
  updated_at?: number
}

/** `GET /api/v1/skills/{slug}` 响应 */
export interface SkillResponse {
  skill: SkillDetail
  latest_version?: SkillLatestVersion | null
  owner?: ClawHubOwner | null
  moderation?: ClawHubModeration | null
}

/** `GET /api/v1/search` 结果项 */
export interface ClawHubSearchResult {
  score?: number
  slug?: string | null
  display_name?: string | null
  summary?: string | null
  version?: string | null
  updated_at?: number | null
  owner_handle?: string | null
  owner?: ClawHubOwner | null
}

/** `GET /api/v1/search` 响应 */
export interface SearchResponse {
  results: ClawHubSearchResult[]
}

/** `GET /api/v1/plugins` 列表项 */
export interface PackageCatalogItem {
  name: string
  display_name: string
  /** `skill` | `code-plugin` | `bundle-plugin` */
  family?: string
  /** `official` | `community` | `private` */
  channel?: string
  is_official?: boolean
  summary?: string | null
  owner_handle?: string | null
  created_at?: number
  updated_at?: number
  latest_version?: string | null
}

/** `GET /api/v1/plugins` 响应 */
export interface PackageListResponse {
  items: PackageCatalogItem[]
  next_cursor?: string | null
}

/** `GET /api/v1/plugins/search` 结果项 */
export interface PackageSearchResult {
  score?: number
  name?: string | null
  display_name?: string | null
  family?: string | null
  summary?: string | null
  owner_handle?: string | null
  updated_at?: number | null
}

/** `GET /api/v1/plugins/search` 响应 */
export interface PackageSearchResponse {
  results: PackageSearchResult[]
}

/** `GET /api/v1/packages/{name}` 包详情 */
export interface PackageDetail {
  name: string
  display_name: string
  family?: string
  channel?: string
  is_official?: boolean
  summary?: string | null
  owner_handle?: string | null
  created_at?: number
  updated_at?: number
  latest_version?: string | null
}

/** `GET /api/v1/packages/{name}` 响应 */
export interface PackageResponse {
  package?: PackageDetail | null
  owner?: ClawHubOwner | null
}

/** 本地已安装插件（与 core::InstalledPlugin 对齐） */
export interface InstalledPlugin {
  /** 主键：`<owner>/<name>` 形式 */
  id: string
  name: string
  display_name: string
  summary?: string
  /** `code-plugin` | `bundle-plugin` */
  family?: string
  /** `official` | `community` | `private` */
  channel?: string
  owner_handle?: string
  version?: string
  install_path?: string | null
  installed_at: number
}

// =========================================================
// 多页签系统（Tab System）
// 主内容栏从单一 ChatWindow 重构为多页签，允许同时打开多个活跃页签。
// chat 页签 id = conversation_id（新对话用 `__new_chat__` 哨兵，会话建立后迁移为真实 id）
// asr-* 页签 id = 业务 uuid
// =========================================================

export type TabKind = 'chat' | 'asr-stream' | 'asr-upload' | 'asr-history'

export interface TabItem {
  /** 唯一 id（chat 用 conversation_id，asr-* 用 uuid） */
  id: string
  kind: TabKind
  title: string
  icon?: string
  /** chat 类型默认 true；asr-stream 录音中不可关 */
  closable: boolean
  status?: 'idle' | 'loading' | 'recording' | 'active' | 'error'
  /** 仅 chat 类型：关联的会话 id */
  conversationId?: string
  /**
   * 页签实例稳定 key（openTab 时自动生成，永不变更）。
   * 用途：作为 Vue <component :key> 的取值。
   * 为什么不直接用 id：chat 页签在新建会话建立后会由 updateTab 把 id 从
   * `__new_chat__` 迁移为真实 conversation_id；若用 id 作 :key 会触发组件
   * 重新挂载，导致流式传输中的 ChatWindow 实例被销毁、Tauri 事件监听丢失。
   * instanceKey 与 id 解耦，迁移 id 时组件实例保持不变，流式持续可用。
   */
  instanceKey: string
}

// =========================================================
// ASR 语音转写（与 core::asr::types + commands::asr 对齐）
// 后端 serde 均为 snake_case（未启用 rename_all = "camelCase"）：
//   - AsrStatus / AsrSource / AsrProvider: #[serde(rename_all = "snake_case")]
//   - AsrRecord / AsrConfig / AsrSummaryHit / AsrSessionInfo / AsrFinishResult: 字段名直传
// BusEvent: #[serde(tag = "kind", rename_all = "snake_case")]
// =========================================================

/** ASR 服务商（与 AsrProvider 枚举对齐，snake_case 变体） */
export type AsrProvider = 'volc_engine' | 'qwen'

/** ASR 转写状态机 */
export type AsrStatus =
  | 'pending'
  | 'transcribing'
  | 'transcribed'
  | 'summarizing'
  | 'completed'
  | 'failed'

/** ASR 音频来源 */
export type AsrSource = 'streaming' | 'upload'

/** 一条 ASR 转写记录（list_records 不含 transcript，get_record 含） */
export interface AsrRecord {
  id: string
  audio_path: string
  /** 完整转写文本；list_records 返回时可能为空字符串 */
  transcript: string
  title: string
  language: string
  summary: string | null
  error_message: string | null
  tags: string[]
  /** ISO 8601 字符串 */
  created_at: string
  updated_at: string
  duration_ms: number
  sample_rate: number
  provider: AsrProvider
  status: AsrStatus
  source: AsrSource
}

/** 摘要 RAG 命中条目（asr_search_summaries 返回） */
export interface AsrSummaryHit {
  record_id: string
  title: string
  summary_snippet: string
  created_at: string
  /** BM25 相关性得分，越大越相关 */
  score: number
}

/** 活跃流式会话信息（asr_list_sessions 返回） */
export interface AsrSessionInfo {
  session_id: string
  record_id: string | null
  language: string
  state: string
}

/** ASR 配置（与 core::config::AsrConfig 对齐，snake_case 字段） */
export interface AsrConfig {
  provider: AsrProvider
  volc_app_id: string
  volc_access_token: string
  volc_cluster: string
  qwen_api_key: string
  qwen_base_url: string
  qwen_audio_model: string
  default_language: string
  enable_auto_summary: boolean
  summary_model: string | null
}

/** finish_streaming / transcribe_file 的返回结果 */
export interface AsrFinishResult {
  transcript: string
  record_id: string | null
  summary: string | null
}

/** asr_update_record 可选字段补丁 */
export interface AsrRecordPatch {
  title?: string
  tags?: string[]
  summary?: string
}

// ============= ASR 事件 payload（BusEvent 子类型，kind 标签已固定） =============

export interface AsrStreamChunkPayload {
  kind: 'asr_stream_chunk'
  session_id: string
  text: string
  is_final: boolean
}

export interface AsrSessionStatusPayload {
  kind: 'asr_session_status'
  session_id: string
  /** "started" | "transcribing" | "completed" | "failed" | "cancelled" */
  status: string
  error?: string | null
}

export interface AsrUploadProgressPayload {
  kind: 'asr_upload_progress'
  record_id: string
  /** 0–1 */
  progress: number
  status: string
}

export interface AsrRecordUpdatedPayload {
  kind: 'asr_record_updated'
  record_id: string
}

// =========================================================
// 服务模型角色（与后端 ServiceModelRole 枚举对齐，snake_case 变体）
// =========================================================

/**
 * 服务模型角色：对应 AgentConfig 中的服务角色字段。
 *
 * 前端"服务模型"面板的每个角色槽位用此类型标识，调用 set_service_model_role 命令配置默认模型。
 */
export type ServiceModelRole =
  | 'chat'
  | 'image_gen'
  | 'title'
  | 'compression'
  | 'asr_stream'
  | 'asr_transcribe'

/**
 * 服务角色元数据：用于 UI 渲染（标题、描述、对应的 AgentConfig 字段、允许的 ModelKind）。
 */
export interface ServiceRoleMeta {
  role: ServiceModelRole
  /** 所属场景分组：聊天 / 语音 / 生图 */
  group: 'chat' | 'voice' | 'image'
  /** 中文标题 */
  label: string
  /** 简短描述 */
  desc: string
  /** 该角色允许的 ModelKind 列表（用于过滤可选模型） */
  allowedKinds: ModelKind[]
  /** 对应 AgentConfig 中的字段名 */
  configField:
    | 'active_model_id'
    | 'active_image_gen_model_id'
    | 'title_model_id'
    | 'compression_model_id'
    | 'asr_stream_model_id'
    | 'asr_transcribe_model_id'
      | 'active_image_gen_model_id'
      | 'title_model_id'
      | 'compression_model_id'
      | 'asr_stream_model_id'
      | 'asr_transcribe_model_id'
}

// =========================================================
// 每会话 todoTree（与 agent::todo_store / tools::todo_write 对齐）
// =========================================================

export type TodoPriority = 'high' | 'medium' | 'low'
export type TodoStatus = 'pending' | 'in_progress' | 'completed'

/** 单个待办项（扁平列表，parent_id 指针表达树形层级） */
export interface TodoItem {
  id: string
  content: string
  priority: TodoPriority
  status: TodoStatus
  /** 仅在 completed 时可选填入的完成总结 */
  summary?: string | null
  /** 父任务 id；null/缺省表示根任务，Some(id) 表示该 id 任务的子任务 */
  parent_id?: string | null
}

/** 树形节点（前端由 TodoItem 列表还原） */
export interface TodoNode {
  id: string
  content: string
  priority: TodoPriority
  status: TodoStatus
  summary?: string | null
  children: TodoNode[]
}

// 与 Rust 侧 serde 输出对应的类型定义。
// Role: #[serde(rename_all = "lowercase")] -> "system" | "user" | "assistant"
// DeviceStatus: #[serde(rename_all = "snake_case")] -> "discovered" | "paired" | "offline" | "pairing"
// BusEvent: #[serde(tag = "kind", rename_all = "snake_case")]，每个事件携带 kind 标签。

export type Role = 'system' | 'user' | 'assistant'

export type DeviceStatus = 'discovered' | 'paired' | 'offline' | 'pairing'

// ModelKind: #[serde(rename_all = "snake_case")]
export type ModelKind = 'chat' | 'image_gen' | 'video_gen'

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
}

export interface Device {
  id: string
  name: string
  address: string
  last_seen: number
  status: DeviceStatus
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
  /** 模型能力类型：chat（对话）/ image_gen（图像生成）/ video_gen（视频生成，预留） */
  kind?: ModelKind
  /** 图像生成专用：默认尺寸（如 1024x1024），仅 kind=image_gen 时有效 */
  image_size?: string | null
  /** 图像生成专用：默认质量（如 standard/hd），仅 kind=image_gen 时有效 */
  image_quality?: string | null
  created_at: number
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

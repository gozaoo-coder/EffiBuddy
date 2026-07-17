// 与 Rust 侧 serde 输出对应的类型定义。
// Role: #[serde(rename_all = "lowercase")] -> "system" | "user" | "assistant"
// DeviceStatus: #[serde(rename_all = "snake_case")] -> "discovered" | "paired" | "offline" | "pairing"
// BusEvent: #[serde(tag = "kind", rename_all = "snake_case")]，每个事件携带 kind 标签。

export type Role = 'system' | 'user' | 'assistant'

export type DeviceStatus = 'discovered' | 'paired' | 'offline' | 'pairing'

export interface Message {
  id: string
  content: string
  timestamp: number
  role: Role
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
// 附件（输入区工具 Sheet 选中后附加到输入）
// =========================================================

export type AttachmentKind = 'image' | 'file'

export interface Attachment {
  kind: AttachmentKind
  path: string
  name: string
  size: number
}

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

// =========================================================
// 技能 & 定时任务（与 core::{Skill, ScheduledTask} 对齐）
// =========================================================

export interface Skill {
  id: string
  name: string
  description: string
  preamble: string
  tools: string[]
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

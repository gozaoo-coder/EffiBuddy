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
  created_at: number
  message_count: number
}

export type BackendKind = 'mock' | 'openai'

export interface AgentConfig {
  backend: BackendKind
  api_key: string
  base_url: string
  model_name: string
  preamble: string
  enable_tools: boolean
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

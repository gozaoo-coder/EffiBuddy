/**
 * useSubAgentStore —— 子 agent 会话全局状态单例
 *
 * 职责：
 * - 全局订阅 `sub-agent-event`，按 session_id 聚合为 SubAgentRecord（跨页签 / 跨组件共享）
 * - 子 agent 启动时自动在**后台**打开独立会话页签（kind='sub-agent'），使每个子 agent
 *   成为"单独的新会话"窗口；可在交流池会话卡片中点击该窗口进入查看（替代 main-content）
 * - 子 agent 事件到达时同步刷新交流池（后端子 agent 自动登记到 pool 但不发
 *   agent-pool-updated 事件，前端据此在生命周期事件时补一次拉取，保持状态实时）
 * - 暴露 records / getRecord / activeCount，供 SubAgentWindow 与交流池渲染
 *
 * 设计：
 * - 单例模式：module-level `records` 被所有调用方共享，多组件实例看到同一份状态
 * - 事件监听在首次调用时安装，全局生命周期不卸载
 * - 记录用 reactive 持有，Vue 响应式自动驱动 UI 更新
 *
 * # 与 ChatWindow 内嵌卡片的关系
 * 主会话 ChatWindow 仍会在气泡内聚合子代理记录（用于历史持久化到 Message.subAgents），
 * 本 store 是独立的全局视图层：即使主会话页签被关闭 / 子代理页签被关闭，
 * 只要事件仍在推送，记录都会在此累积，可随时从交流池重新打开查看。
 */
import { reactive, computed, type ComputedRef } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { SubAgentEventPayload, SubAgentRecord } from '../types'
import { useTabs } from './useTabs'
import { useAgentPool } from './useAgentPool'

// ── 全局单例状态 ────────────────────────────────────────────────────────

/** 全部子代理会话记录：key = session_id */
const records = reactive<Record<string, SubAgentRecord>>({})

// ── 事件监听安装（幂等，仅首次安装） ────────────────────────────────────

let installed = false
let unlisten: UnlistenFn | null = null

/** 安装全局事件监听（幂等） */
async function install(): Promise<void> {
  if (installed) return
  installed = true
  unlisten = await listen<SubAgentEventPayload>('sub-agent-event', (e) => {
    handleEvent(e.payload)
  })
}

// ── 事件处理 ────────────────────────────────────────────────────────────

/** 交流池刷新函数缓存（避免每次事件重复调用 useAgentPool()） */
let poolRefresh: (() => Promise<void>) | null = null
function refreshPool(): void {
  if (!poolRefresh) poolRefresh = useAgentPool().refresh
  void poolRefresh()
}

function handleEvent(p: SubAgentEventPayload): void {
  let rec = records[p.session_id]
  if (!rec) {
    rec = {
      session_id: p.session_id,
      name: p.name,
      model: p.model,
      depth: p.depth,
      status: 'running',
      task: '',
      text: '',
      toolCalls: [],
      images: [],
      error: '',
      finishedAt: null,
    }
    records[p.session_id] = rec
  }

  let poolChanged = false
  switch (p.kind) {
    case 'started':
      rec.task = p.content
      rec.status = 'running'
      openSubAgentTab(p)
      poolChanged = true
      break
    case 'token':
      rec.text += p.content
      break
    case 'tool_call':
      rec.toolCalls.push({
        call_id: p.session_id + '_' + rec.toolCalls.length,
        tool_name: p.tool_name,
        arguments: p.arguments,
        result: null,
        is_error: false,
        pending: true,
      })
      break
    case 'tool_result': {
      const tc = rec.toolCalls.find((t) => t.tool_name === p.tool_name && t.pending)
      if (tc) {
        tc.result = p.content
        tc.is_error = p.is_error
        tc.pending = false
      }
      break
    }
    case 'attachment':
      try {
        const parsed = JSON.parse(p.content)
        if (parsed.path && parsed.name) {
          rec.images.push({ path: parsed.path, name: parsed.name })
        }
      } catch {
        /* 忽略解析失败 */
      }
      break
    case 'done':
      rec.status = 'done'
      rec.text = p.content || rec.text
      rec.finishedAt = Date.now()
      poolChanged = true
      break
    case 'error':
      rec.status = 'error'
      rec.error = p.content
      rec.finishedAt = Date.now()
      poolChanged = true
      break
  }

  // 生命周期事件（started / done / error）对应后端 pool 登记/上报变化，
  // 后端未发 agent-pool-updated，这里补一次拉取保持交流池实时。
  if (poolChanged) refreshPool()
}
/** 子 agent 启动 → 后台打开独立会话页签（不抢当前会话焦点） */
function openSubAgentTab(p: SubAgentEventPayload): void {
  const { openTab } = useTabs()
  openTab(
    {
      id: `sa:${p.session_id}`,
      kind: 'sub-agent',
      title: p.name || '子 agent',
      closable: true,
      conversationId: p.conversation_id,
      subAgentSessionId: p.session_id,
      instanceKey: '',
    },
    false, // 后台打开
  )
}

// ── 派生状态 ────────────────────────────────────────────────────────────

/** 运行中的子代理会话数（角标 / 交流池统计用） */
const activeCount = computed(() => {
  let n = 0
  for (const k in records) {
    if (records[k].status === 'running') n++
  }
  return n
})

/** 按 session_id 取记录（未开始返回 undefined） */
function getRecord(sessionId: string): SubAgentRecord | undefined {
  return records[sessionId]
}

// ── composable 入口 ─────────────────────────────────────────────────────

export interface UseSubAgentStore {
  /** 全部子代理记录（key = session_id，reactive） */
  records: Record<string, SubAgentRecord>
  /** 按 session_id 取记录（未开始返回 undefined） */
  getRecord: (sessionId: string) => SubAgentRecord | undefined
  /** 运行中的子代理会话数 */
  activeCount: ComputedRef<number>
}

/**
 * 子代理会话全局 store（单例）。
 * 首次调用安装全局事件监听；后续调用共享同一份状态。
 */
export function useSubAgentStore(): UseSubAgentStore {
  void install()
  return {
    records,
    getRecord,
    activeCount,
  }
}

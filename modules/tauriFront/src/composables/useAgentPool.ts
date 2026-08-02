/**
 * useAgentPool — 运行时 agent 公共会话交流池的状态管理与 Tauri 命令封装。
 *
 * # 职责
 * - 持有交流池全局状态：全部条目列表（含已完成）
 * - 封装交流池 Tauri 命令（list_pool / clear_pool），统一错误处理
 * - 监听 `agent-pool-updated` 事件，实时刷新状态
 * - 提供派生状态：按状态分组、活跃条目数、待处理 @ 消息数
 *
 * # 设计
 * - 单例模式：模块级 `poolEntries` 被 `useAgentPool()` 共享，多组件实例看到同一份状态
 * - 事件监听在首次 `useAgentPool()` 调用时安装，组件卸载不卸载（全局生命周期）
 * - 状态用 `ref` 持有，Vue 响应式自动驱动 UI 更新
 * - 所有 async 方法返回 `Promise<void>`，错误经 toast 反馈，异常不向上抛
 *
 * # 与 HistoryRail 的关系
 * HistoryRail 内部也调用 `list_pool` 来展示会话级 badge；本 composable
 * 把同一份状态提升为全局单例，供 AgentPoolRail 与 HistoryRail 共享，
 * 避免重复请求与状态不一致。后续 HistoryRail 可迁移至本 composable。
 */
import { ref, computed, type Ref, type ComputedRef } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { PoolEntry, PoolStatus, AtStatus } from '../types'

// ── 全局单例状态 ────────────────────────────────────────────────────────

/** 交流池全部条目（含已完成；按最近更新倒序） */
const poolEntries = ref<PoolEntry[]>([])

/** 是否正在加载（防止重复并发请求） */
const loading = ref(false)

/** 上次刷新时间戳（用于显示刷新指示） */
const lastLoadedAt = ref(0)

// ── 事件监听安装（幂等，仅首次安装） ────────────────────────────────────

let listenersInstalled = false
let unlistenPoolUpdated: UnlistenFn | null = null

/** 安装全局事件监听（幂等） */
async function installListeners(): Promise<void> {
  if (listenersInstalled) return
  listenersInstalled = true

  // 交流池更新（pool_report / pool_at / pool_reply 后触发）→ 刷新列表
  unlistenPoolUpdated = await listen<{ conversation_id: string }>(
    'agent-pool-updated',
    () => {
      void refresh()
    },
  )
}

// ── Tauri 命令封装 ──────────────────────────────────────────────────────

/** 拉取交流池全部条目（按最近更新倒序） */
async function refresh(): Promise<void> {
  // 防止并发请求（事件可能在短时间内多次触发）
  if (loading.value) return
  loading.value = true
  try {
    poolEntries.value = await invoke<PoolEntry[]>('list_pool')
    lastLoadedAt.value = Date.now()
  } catch (e) {
    console.warn('list_pool failed', e)
    poolEntries.value = []
  } finally {
    loading.value = false
  }
}

/** 清空交流池全部条目（调试 / 管理用；不影响会话本身） */
async function clearPool(): Promise<boolean> {
  loading.value = true
  try {
    await invoke('clear_pool')
    poolEntries.value = []
    return true
  } catch (e) {
    console.warn('clear_pool failed', e)
    return false
  } finally {
    loading.value = false
  }
}

// ── 派生状态 ────────────────────────────────────────────────────────────

/** 按状态分组条目（用于左侧栏分组展示） */
const entriesByStatus = computed<{
  inProgress: PoolEntry[]
  waiting: PoolEntry[]
  completed: PoolEntry[]
}>(() => {
  const inProgress: PoolEntry[] = []
  const waiting: PoolEntry[] = []
  const completed: PoolEntry[] = []
  for (const e of poolEntries.value) {
    if (e.status === 'in_progress') inProgress.push(e)
    else if (e.status === 'waiting') waiting.push(e)
    else completed.push(e)
  }
  return { inProgress, waiting, completed }
})

/** 活跃条目数（进行中 + 等待中；用于角标） */
const activeCount = computed(
  () => entriesByStatus.value.inProgress.length + entriesByStatus.value.waiting.length,
)

/** 全部待处理 @ 消息数（跨所有条目的 inbox 汇总） */
const pendingAtCount = computed(() => {
  let n = 0
  for (const e of poolEntries.value) {
    for (const m of e.inbox) {
      if (m.status === 'pending') n++
    }
  }
  return n
})

/** 按会话聚合的条目映射（conversation_id → 该会话下的所有条目） */
const entriesByConversation = computed<Map<string, PoolEntry[]>>(() => {
  const m = new Map<string, PoolEntry[]>()
  for (const e of poolEntries.value) {
    const arr = m.get(e.conversation_id)
    if (arr) arr.push(e)
    else m.set(e.conversation_id, [e])
  }
  return m
})

// ── 公开 @ 消息流（跨全部条目聚合，按时间排序） ──────────────────────────

/** 公开 @ 消息条目：携带发送方 / 接收方 / 内容 / 回复 */
export interface PublicAtItem {
  at_id: string
  from: string
  from_name: string
  to_agent_id: string
  to_name: string
  question: string
  status: AtStatus
  reply?: string | null
  created_at: number
  answered_at?: number | null
}

/**
 * 全部 @ 消息的公开时间线（类似群聊 @ 消息流）。
 *
 * 从所有条目的 inbox 中聚合，按 created_at 降序（最新在前）。
 * 交流池是 agent 之间的公共会话空间：@ 消息在此公开可见，
 * 任何 agent 都能看到谁 @ 了谁、问了什么、是否已回复。
 */
const publicAtFeed = computed<PublicAtItem[]>(() => {
  const items: PublicAtItem[] = []
  for (const e of poolEntries.value) {
    for (const m of e.inbox) {
      items.push({
        at_id: m.at_id,
        from: m.from,
        from_name: m.from_name,
        to_agent_id: e.agent_id,
        to_name: e.name,
        question: m.question,
        status: m.status,
        reply: m.reply,
        created_at: m.created_at,
        answered_at: m.answered_at,
      })
    }
  }
  items.sort((a, b) => b.created_at - a.created_at)
  return items
})

// ── 工具函数 ────────────────────────────────────────────────────────────

/** 中文状态标签 */
function statusLabel(s: PoolStatus): string {
  if (s === 'in_progress') return '进行中'
  if (s === 'waiting') return '等待中'
  return '已完成'
}

/** 中文条目类型标签 */
function kindLabel(k: 'main' | 'sub_agent'): string {
  return k === 'main' ? '主' : '子'
}

/** 格式化相对时间 */
function formatRelativeTime(ts: number): string {
  const diff = Date.now() - ts
  if (diff < 60000) return '刚刚'
  if (diff < 3600000) return `${Math.floor(diff / 60000)}分钟前`
  if (diff < 86400000) return `${Math.floor(diff / 3600000)}小时前`
  if (diff < 2592000000) return `${Math.floor(diff / 86400000)}天前`
  try {
    return new Date(ts).toLocaleDateString()
  } catch {
    return ''
  }
}

// ── composable 入口 ─────────────────────────────────────────────────────

export interface UseAgentPool {
  // 响应式状态
  entries: Ref<PoolEntry[]>
  loading: Ref<boolean>
  lastLoadedAt: Ref<number>
  // 派生
  byStatus: ComputedRef<{
    inProgress: PoolEntry[]
    waiting: PoolEntry[]
    completed: PoolEntry[]
  }>
  byConversation: ComputedRef<Map<string, PoolEntry[]>>
  activeCount: ComputedRef<number>
  pendingAtCount: ComputedRef<number>
  publicAtFeed: ComputedRef<PublicAtItem[]>
  // 方法
  refresh: () => Promise<void>
  clearPool: () => Promise<boolean>
  // 工具
  statusLabel: (s: PoolStatus) => string
  kindLabel: (k: 'main' | 'sub_agent') => string
  formatRelativeTime: (ts: number) => string
}

/**
 * 交流池状态管理 composable（单例）。
 *
 * 首次调用安装全局事件监听并刷新状态；后续调用共享同一份状态。
 */
export function useAgentPool(): UseAgentPool {
  // 安装事件监听（异步，不阻塞返回）
  installListeners()

  return {
    entries: poolEntries,
    loading,
    lastLoadedAt,
    byStatus: entriesByStatus,
    byConversation: entriesByConversation,
    activeCount,
    pendingAtCount,
    publicAtFeed,
    refresh,
    clearPool,
    statusLabel,
    kindLabel,
    formatRelativeTime,
  }
}

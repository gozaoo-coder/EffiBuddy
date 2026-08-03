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
import { useTabs } from './useTabs'

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


// ── 会话视图（将「正在运行的会话」加入交流池） ──────────────────────────

/** 窗口状态：交流池状态 + 空闲（有打开窗口但未在跑） */
export type PoolWindowStatus = PoolStatus | 'idle'

/** 会话状态：进行中 / 等待中 / 已完成 / 空闲（有窗口打开但未在跑） */
export type PoolSessionStatus = PoolStatus | 'idle'

/** 交流池会话内的一个窗口（主 agent / 子 agent） */
export interface PoolWindow {
  /** 窗口类型：主 = 会话主 agent；子 = 子 agent */
  kind: 'main' | 'sub_agent'
  /** 窗口唯一标识：主 = conversation_id；子 = session_id */
  windowId: string
  /** 打开该窗口对应的页签 id：主 = conversation_id；子 = `sa:<session_id>` */
  tabId: string
  /** 所属会话 id（子 agent 为父会话） */
  conversationId: string
  /** 子 agent 专属：session_id */
  sessionId?: string
  /** 显示名 */
  name: string
  /** 任务描述 */
  task: string
  /** 状态（idle = 窗口已打开但无池条目/未在生成） */
  status: PoolWindowStatus
  /** 最近上报 */
  lastReport: string
  updatedAt: number
  /** 待处理 @ 消息数 */
  atCount: number
  /** 是否已在 UI 中打开（有页签） */
  opened: boolean
}

/** 交流池会话：按 conversation_id 聚合的全部活跃窗口 */
export interface PoolSession {
  conversationId: string
  /** 会话标题（优先主 agent 池条目名，回退到页签标题 / id 后缀） */
  title: string
  /** 窗口列表（主 agent 在前，子 agent 在后） */
  windows: PoolWindow[]
  status: PoolSessionStatus
  /** 活跃窗口数（已打开或池状态活跃的窗口） */
  activeCount: number
  /** 该会话全部交流池条目（含已完成） */
  entries: PoolEntry[]
  /** 待处理 @ 消息总数 */
  pendingAtCount: number
  updatedAt: number
}

/** 页签句柄（会话聚合用） */
const { tabs: openTabs, openTab, activate: activateTab } = useTabs()

/** 把交流池条目转为窗口模型 */
function entryToWindow(e: PoolEntry): PoolWindow {
  const isSub = e.kind === 'sub_agent'
  const sessionId = isSub ? e.agent_id.replace(/^sa:/, '') : undefined
  return {
    kind: e.kind,
    windowId: isSub ? (sessionId ?? e.agent_id) : e.conversation_id,
    tabId: isSub ? `sa:${sessionId}` : e.conversation_id,
    conversationId: e.conversation_id,
    sessionId,
    name: e.name,
    task: e.task,
    status: e.status,
    lastReport: e.last_report,
    updatedAt: e.updated_at,
    atCount: e.inbox.filter((m) => m.status === 'pending').length,
    opened: false,
  }
}

/** 合并 / 更新会话中的窗口（池条目与页签互补：条目补状态，页签补 opened） */
function upsertWindow(s: PoolSession, win: PoolWindow): void {
  const idx = s.windows.findIndex(
    (w) => w.kind === win.kind && w.windowId === win.windowId,
  )
  if (idx === -1) {
    s.windows.push(win)
    return
  }
  const ex = s.windows[idx]
  // 池条目存在：状态/任务/上报以条目为准；页签打开状态保留
  if (win.task) ex.task = win.task
  if (win.lastReport) ex.lastReport = win.lastReport
  if (win.status !== 'idle') ex.status = win.status
  ex.updatedAt = Math.max(ex.updatedAt, win.updatedAt)
  ex.atCount = Math.max(ex.atCount, win.atCount)
  ex.name = win.name || ex.name
  ex.opened = ex.opened || win.opened
  // 子 agent 补充会话信息
  if (win.sessionId) ex.sessionId = win.sessionId
  if (win.conversationId) ex.conversationId = win.conversationId
}

/** 汇总会话状态 */
function aggregateSessionStatus(s: PoolSession): PoolSessionStatus {
  let inProgress = false
  let waiting = false
  let anyActiveOrOpen = false
  for (const w of s.windows) {
    if (w.status === 'in_progress') inProgress = true
    if (w.status === 'waiting') waiting = true
    if (w.status !== 'completed' || w.opened) anyActiveOrOpen = true
  }
  if (inProgress) return 'in_progress'
  if (waiting) return 'waiting'
  if (anyActiveOrOpen) return 'idle'
  return 'completed'
}

/**
 * 会话聚合视图：把「正在运行的会话」（打开的页签窗口）+「交流池登记条目」
 * 合并为按 conversation_id 分组的会话列表。每个会话展示活跃窗口数，
 * 展开后列出主 agent（第一个）+ 子 agent 窗口，点击可在 main-content 打开。
 */
const sessions = computed<PoolSession[]>(() => {
  const map = new Map<string, PoolSession>()
  const ensure = (convId: string): PoolSession => {
    let s = map.get(convId)
    if (!s) {
      s = {
        conversationId: convId,
        title: '',
        windows: [],
        status: 'idle',
        activeCount: 0,
        entries: [],
        pendingAtCount: 0,
        updatedAt: 0,
      }
      map.set(convId, s)
    }
    return s
  }

  // 1. 交流池条目 → 会话窗口（含已完成，保留历史登记）
  for (const e of poolEntries.value) {
    const s = ensure(e.conversation_id)
    s.entries.push(e)
    upsertWindow(s, entryToWindow(e))
  }

  // 2. 打开的页签 → 会话窗口（正在运行的会话：chat = 主，sub-agent = 子）
  const now = Date.now()
  for (const t of openTabs.value) {
    if (t.kind === 'chat' && t.conversationId) {
      const s = ensure(t.conversationId)
      upsertWindow(s, {
        kind: 'main',
        windowId: t.conversationId,
        tabId: t.conversationId,
        conversationId: t.conversationId,
        name: t.title || '对话',
        task: '',
        // 正在生成 → 进行中；否则有窗口但空闲
        status: t.status === 'loading' ? 'in_progress' : 'idle',
        lastReport: '',
        updatedAt: now,
        atCount: 0,
        opened: true,
      })
      if (!s.title) s.title = t.title || ''
    } else if (t.kind === 'sub-agent' && t.conversationId && t.subAgentSessionId) {
      const s = ensure(t.conversationId)
      upsertWindow(s, {
        kind: 'sub_agent',
        windowId: t.subAgentSessionId,
        tabId: t.id,
        conversationId: t.conversationId,
        sessionId: t.subAgentSessionId,
        name: t.title || '子 agent',
        task: '',
        status: 'in_progress',
        lastReport: '',
        updatedAt: now,
        atCount: 0,
        opened: true,
      })
    }
  }

  // 3. 汇总每个会话
  const out: PoolSession[] = []
  for (const s of map.values()) {
    // 主 agent 窗口在前，子 agent 在后
    s.windows.sort((a, b) =>
      a.kind === 'main' && b.kind !== 'main' ? -1 : a.kind !== 'main' && b.kind === 'main' ? 1 : 0,
    )
    // 标题：主 agent 池条目名优先
    const mainEntry = s.entries.find((e) => e.kind === 'main')
    if (mainEntry && mainEntry.name) s.title = mainEntry.name
    if (!s.title)
      s.title =
        s.conversationId.length > 12 ? s.conversationId.slice(0, 12) + '…' : s.conversationId
    s.status = aggregateSessionStatus(s)
    s.activeCount = s.windows.filter((w) => w.opened || w.status !== 'completed').length
    s.pendingAtCount = s.entries.reduce(
      (n, e) => n + e.inbox.filter((m) => m.status === 'pending').length,
      0,
    )
    s.updatedAt = Math.max(
      0,
      ...s.windows.map((w) => w.updatedAt),
      ...s.entries.map((e) => e.updated_at),
    )
    out.push(s)
  }

  // 排序：活跃在前，按最近更新倒序
  const prio: Record<PoolSessionStatus, number> = {
    in_progress: 0,
    waiting: 1,
    idle: 2,
    completed: 3,
  }
  out.sort((a, b) => prio[a.status] - prio[b.status] || b.updatedAt - a.updatedAt)
  return out
})

/** 活跃会话数（进行中 / 等待中 / 空闲，不含已完成） */
const activeSessionCount = computed(
  () => sessions.value.filter((s) => s.status !== 'completed').length,
)

/**
 * 在 main-content 打开指定窗口：已打开则激活页签，未打开则新建页签。
 * 主 agent → chat 页签；子 agent → sub-agent 页签（实时会话视图）。
 */
function openWindow(w: PoolWindow): void {
  if (openTabs.value.some((t) => t.id === w.tabId)) {
    activateTab(w.tabId)
    return
  }
  if (w.kind === 'main') {
    openTab({
      id: w.conversationId,
      kind: 'chat',
      title: w.name || '对话',
      closable: true,
      conversationId: w.conversationId,
      instanceKey: '',
    })
  } else {
    openTab({
      id: w.tabId,
      kind: 'sub-agent',
      title: w.name || '子 agent',
      closable: true,
      conversationId: w.conversationId,
      subAgentSessionId: w.sessionId,
      instanceKey: '',
    })
  }
}

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

/** 中文状态标签（含 idle=空闲/活跃窗口） */
function statusLabel(s: PoolStatus | 'idle'): string {
  if (s === 'in_progress') return '进行中'
  if (s === 'waiting') return '等待中'
  if (s === 'idle') return '活跃'
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
  /** 会话聚合视图（正在运行的会话 + 池登记条目） */
  sessions: ComputedRef<PoolSession[]>
  /** 活跃会话数（不含已完成） */
  activeSessionCount: ComputedRef<number>
  // 方法
  refresh: () => Promise<void>
  clearPool: () => Promise<boolean>
  /** 在 main-content 打开指定窗口（主 agent / 子 agent） */
  openWindow: (w: PoolWindow) => void
  // 工具
  statusLabel: (s: PoolStatus | 'idle') => string
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
    sessions,
    activeSessionCount,
    refresh,
    clearPool,
    openWindow,
    statusLabel,
    kindLabel,
    formatRelativeTime,
  }
}

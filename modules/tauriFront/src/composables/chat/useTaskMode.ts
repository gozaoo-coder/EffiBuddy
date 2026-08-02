/**
 * 长程任务模式（多任务组）
 *
 * 核心思路：一次「长程任务」= agent 调用一次 `todo_write` 后直到下一轮用户输入
 * 之间的全部 assistant 消息。这些消息被聚合进一个 TaskGroup，在聊天列表中渲染为
 * 一个 TaskBubble（内含 todoTree + 实时工作输出）。
 *
 * 多组并存：会话中可以先后创建多个长程任务，每个独立成组、独立渲染。
 * 早期非任务消息（问候 / 普通问答）始终保持为普通气泡，不被吸纳。
 *
 * 边界判定：
 * - 组起始 = assistant 消息的 toolCalls 中含 `todo_write`
 * - 组结束 = 下一条 user 消息（或下一个 todo_write 开新组）
 * - user 消息永不入组
 */
import { ref, computed, watch, reactive } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { TodoItem, TodoNode, Message } from '../../types'
import type { useChatCore } from './useChatCore'

/** 一个长程任务组：包含本轮 todo_write 之后的所有 assistant 消息 */
export interface TaskGroup {
  /** 唯一 id */
  id: string
  /** 属于本组的 assistant 消息 id（有序数组，保证渲染顺序） */
  messageIds: string[]
  /** 本组首条消息 id（在渲染列表中定位 TaskBubble 插入位置） */
  startMessageId: string
  /** 是否为当前活跃组（活跃组展示最新 todoTree；历史组仅展示工作输出） */
  active: boolean
}

/** 渲染列表项：消息 或 任务气泡占位 */
export type RenderItem =
  | { kind: 'message'; message: Message }
  | { kind: 'task'; group: TaskGroup }

export function useTaskMode(core: ReturnType<typeof useChatCore>) {
  const todoItems = ref<TodoItem[]>([])
  /** 全部任务组（按创建顺序；渲染时按 startMessageId 在消息流中的位置插入） */
  const taskGroups = ref<TaskGroup[]>([])
  /** 当前正在构建的任务组（streaming 期间新增 assistant 消息追加到此组） */
  const currentGroup = ref<TaskGroup | null>(null)
  /** 当前这一轮用户输入是否属于长程任务（agent 调用 todo_write 则置 true） */
  const currentTurnIsTask = ref(false)

  // ── 各组折叠/展开状态（groupId → state），reactive 保证模板响应 ─────────
  interface GroupUiState {
    collapsed: boolean
    outputExpanded: boolean
  }
  const groupUi = reactive<Record<string, GroupUiState>>({})

  function ensureGroupUi(id: string): GroupUiState {
    if (!groupUi[id]) {
      groupUi[id] = { collapsed: false, outputExpanded: false }
    }
    return groupUi[id]
  }

  function isGroupCollapsed(g: TaskGroup): boolean {
    return ensureGroupUi(g.id).collapsed
  }

  function toggleGroupCollapsed(g: TaskGroup): void {
    ensureGroupUi(g.id).collapsed = !ensureGroupUi(g.id).collapsed
  }

  function isGroupOutputExpanded(g: TaskGroup): boolean {
    return ensureGroupUi(g.id).outputExpanded
  }

  function toggleGroupOutputExpanded(g: TaskGroup): void {
    ensureGroupUi(g.id).outputExpanded = !ensureGroupUi(g.id).outputExpanded
  }

  // ── todoTree 加载与构建 ──────────────────────────────────────────────

  /** 加载当前会话的任务清单 */
  async function loadTodoTree() {
    const id = core.activeId.value
    if (!id || id.startsWith('__')) {
      todoItems.value = []
      return
    }
    try {
      todoItems.value = await invoke<TodoItem[]>('get_todo_tree', { conversationId: id })
    } catch {
      todoItems.value = []
    }
  }

  /** 把扁平 TodoItem 还原为树 */
  function buildTodoTree(items: TodoItem[]): TodoNode[] {
    const nodes: TodoNode[] = items.map((t) => ({
      id: t.id,
      content: t.content,
      priority: t.priority,
      status: t.status,
      summary: t.summary,
      children: [],
    }))
    const index = new Map<string, TodoNode>()
    nodes.forEach((n) => index.set(n.id, n))
    const roots: TodoNode[] = []
    for (const t of items) {
      const node = index.get(t.id)!
      const pid = t.parent_id
      if (pid && index.has(pid) && pid !== t.id) {
        index.get(pid)!.children.push(node)
      } else {
        roots.push(node)
      }
    }
    return roots
  }

  const todoTree = computed(() => buildTodoTree(todoItems.value))

  /** 任务进度摘要 */
  const taskSummary = computed(() => {
    let total = 0
    let done = 0
    let progress = 0
    for (const t of todoItems.value) {
      total++
      if (t.status === 'completed') done++
      else if (t.status === 'in_progress') progress++
    }
    if (total === 0) return '正在建立任务清单…'
    return `${done}/${total} 完成 · ${progress} 进行中`
  })

  // ── 渲染列表：消息与任务气泡交错 ──────────────────────────────────────

  /** 所有任务组包含的消息 id 集合（用于快速判断消息是否属于某组） */
  const allTaskMessageIds = computed<Set<string>>(() => {
    const s = new Set<string>()
    for (const g of taskGroups.value) {
      for (const id of g.messageIds) s.add(id)
    }
    return s
  })

  /** 起始消息 id → 任务组 映射 */
  const groupByStartMsg = computed<Map<string, TaskGroup>>(() => {
    const m = new Map<string, TaskGroup>()
    for (const g of taskGroups.value) {
      m.set(g.startMessageId, g)
    }
    return m
  })

  /** 是否存在任务组（替代原 taskMode 布尔） */
  const hasTaskGroups = computed(() => taskGroups.value.length > 0)

  /** 渲染列表：在消息流中按起始位置插入 TaskBubble 占位，组内消息被隐藏 */
  const renderList = computed<RenderItem[]>(() => {
    const items: RenderItem[] = []
    const taskIds = allTaskMessageIds.value
    const startMap = groupByStartMsg.value

    for (const m of core.messages.value) {
      if (taskIds.has(m.id)) {
        // 组内消息：仅在起始位置插入 TaskBubble，其余跳过（由 TaskBubble 内部渲染）
        const group = startMap.get(m.id)
        if (group) {
          items.push({ kind: 'task', group })
        }
      } else {
        items.push({ kind: 'message', message: m })
      }
    }
    return items
  })

  /** 获取某组的消息对象列表（按 messageIds 顺序） */
  function groupMessages(group: TaskGroup): Message[] {
    const msgs = core.messages.value
    const byId = new Map<string, Message>()
    for (const m of msgs) byId.set(m.id, m)
    const out: Message[] = []
    for (const id of group.messageIds) {
      const m = byId.get(id)
      if (m) out.push(m)
    }
    return out
  }

  // ── 流式期间新消息追踪 ────────────────────────────────────────────────
  // 监听 messages 长度变化，将新增 assistant 消息追加到当前组。
  // flush:'sync' 确保在 DOM 更新前完成，避免消息先以普通气泡闪现再消失。
  let lastProcessedLen = 0

  watch(
    () => core.messages.value.length,
    (newLen) => {
      if (newLen <= lastProcessedLen) {
        lastProcessedLen = newLen
        return
      }
      const g = currentGroup.value
      if (!g) {
        lastProcessedLen = newLen
        return
      }
      for (let i = lastProcessedLen; i < newLen; i++) {
        const m = core.messages.value[i]
        if (m.role === 'assistant' && !g.messageIds.includes(m.id)) {
          g.messageIds.push(m.id)
        }
      }
      lastProcessedLen = newLen
    },
    { flush: 'sync' },
  )

  // ── 任务清单清空 → 标记所有组为非活跃（保留历史展示） ──────────────────
  watch(todoItems, (items) => {
    if (items.length === 0) {
      for (const g of taskGroups.value) g.active = false
      currentGroup.value = null
      currentTurnIsTask.value = false
    }
  })

  // ── 会话加载成功后：从消息历史重建任务组 ──────────────────────────────

  /** 从消息历史重建任务组：扫描 todo_write 工具调用确定组边界 */
  function syncFromTodo() {
    // 重置追踪索引，避免新会话的 messages 与旧索引错位
    lastProcessedLen = core.messages.value.length

    const groups: TaskGroup[] = []
    let cur: TaskGroup | null = null

    for (const m of core.messages.value) {
      // user 消息封存当前组（下一个 todo_write 开新组）
      if (m.role === 'user') {
        cur = null
        continue
      }
      if (m.role !== 'assistant') continue

      // 检查该消息是否包含 todo_write 工具调用
      const hasTodoWrite = m.toolCalls?.some((tc) => tc.tool_name === 'todo_write') ?? false

      if (hasTodoWrite) {
        // 新组起始
        cur = {
          id: newGroupId(),
          messageIds: [m.id],
          startMessageId: m.id,
          active: false,
        }
        groups.push(cur)
      } else if (cur) {
        // 追加到当前组
        cur.messageIds.push(m.id)
      }
      // else: 普通消息（无组），正常渲染
    }

    // 最后一个组标记为活跃（展示最新 todoTree），仅当 todoItems 非空
    if (groups.length > 0 && todoItems.value.length > 0) {
      groups[groups.length - 1].active = true
    }

    taskGroups.value = groups
    currentGroup.value = null
    currentTurnIsTask.value = false
  }

  // ── 新一轮用户输入：封存当前组（新消息不再合并进旧组） ──────────────────
  function beginNewTurn() {
    currentTurnIsTask.value = false
    // currentGroup 保持引用但不接收新消息（currentTurnIsTask=false 时 watch 不追加）
    // 实际上 watch 只检查 currentGroup 非空，这里通过置 null 来封存
    currentGroup.value = null
  }

  // ── 流式期间收到 todo_write 调用：创建新任务组 ────────────────────────
  function markTaskTurn(currentBubbleId: string | null) {
    // 创建新组
    const g: TaskGroup = {
      id: newGroupId(),
      messageIds: [],
      startMessageId: '',
      active: true,
    }

    // 把上一个活跃组降级为历史
    for (const prev of taskGroups.value) {
      prev.active = false
    }

    // 如果当前有流式气泡，把它作为组起始
    if (currentBubbleId) {
      g.messageIds.push(currentBubbleId)
      g.startMessageId = currentBubbleId
    }

    taskGroups.value.push(g)
    currentGroup.value = g
    currentTurnIsTask.value = true

    // 加载任务清单
    void loadTodoTree()
  }

  // ── 会话切换/清空：重置全部状态 ───────────────────────────────────────
  function resetAll() {
    taskGroups.value = []
    currentGroup.value = null
    currentTurnIsTask.value = false
    todoItems.value = []
    lastProcessedLen = 0
    // 清理 UI 状态
    for (const key of Object.keys(groupUi)) {
      delete groupUi[key]
    }
  }

  // ── 工具 ──────────────────────────────────────────────────────────────

  function newGroupId(): string {
    if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
      return crypto.randomUUID()
    }
    return `${Date.now()}-${Math.random().toString(16).slice(2)}`
  }

  return {
    // 状态
    todoItems,
    taskGroups,
    currentGroup,
    currentTurnIsTask,
    groupUi,
    // 派生
    hasTaskGroups,
    todoTree,
    taskSummary,
    renderList,
    // UI 状态
    isGroupCollapsed,
    toggleGroupCollapsed,
    isGroupOutputExpanded,
    toggleGroupOutputExpanded,
    // 数据访问
    groupMessages,
    // 方法
    loadTodoTree,
    syncFromTodo,
    beginNewTurn,
    markTaskTurn,
    resetAll,
  }
}

/**
 * useTabs —— 多页签状态管理（全局单例）
 *
 * 设计要点：
 * 1. 状态在 module-level 声明，所有调用 useTabs() 的组件共享同一份 tabs / activeTabId，
 *    天然实现跨组件通信（TabBar / TabContent / App.vue / HistoryRail 触发处均一致）。
 * 2. 不做磁盘持久化：运行时内存即可，刷新即清空（与"活跃页签"语义一致）。
 * 3. id 去重：openTab 按 id 去重，已存在则仅激活。chat 页签 id = conversation_id；
 *    新对话用 `__new_chat__` 哨兵（保证全局只有一个空对话页签），会话建立后由
 *    App.vue 调用 updateTab 将 id 迁移为真实 conversation_id，迁移时自动同步 activeTabId。
 * 4. updateTab 支持变更 id（partial.id）：若目标 id 已被其它页签占用，则合并——
 *    移除当前页签并激活已存在者，避免重复；否则原地改 id 并同步 activeTabId。
 */
import { ref, computed, type Ref, type ComputedRef } from 'vue'
import type { TabItem } from '../types'

// 新对话页签的全局哨兵 id：保证同时最多一个"未发送"的空对话页签
export const NEW_CHAT_TAB_ID = '__new_chat__'

// 生成页签实例稳定 key（crypto.randomUUID 优先；降级到时间戳+计数器）
let instanceKeyCounter = 0
function genInstanceKey(): string {
  instanceKeyCounter += 1
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID()
  }
  return `tab_${Date.now().toString(36)}_${instanceKeyCounter.toString(36)}`
}

// ============= module-level 单例状态 =============
const tabs = ref<TabItem[]>([])
const activeTabId = ref<string | null>(null)

// 当前激活页签（computed 缓存，跨调用共享同一引用）
const activeTab = computed<TabItem | null>(
  () => tabs.value.find((t) => t.id === activeTabId.value) ?? null,
)

// ============= 方法 =============

/**
 * 打开页签：同 id 已存在则仅激活，否则新增并激活。
 * instanceKey 自动生成（调用方无需提供），用于 Vue <component :key> 稳定标识。
 */
function openTab(tab: TabItem): void {
  const existing = tabs.value.find((t) => t.id === tab.id)
  if (existing) {
    activeTabId.value = tab.id
    return
  }
  // 确保实例 key 存在且唯一（调用方一般不传，此处兜底）
  if (!tab.instanceKey) {
    tab.instanceKey = genInstanceKey()
  }
  tabs.value.push(tab)
  activeTabId.value = tab.id
}

/**
 * 关闭页签：移除并自动激活相邻页签；若为最后一个，activeTabId 置 null（显示空状态）。
 * 返回被关闭页签的 kind / conversationId，供调用方做后续清理（如停止录音）。
 */
function closeTab(id: string): TabItem | null {
  const idx = tabs.value.findIndex((t) => t.id === id)
  if (idx === -1) return null
  const removed = tabs.value[idx]
  tabs.value.splice(idx, 1)
  if (activeTabId.value === id) {
    if (tabs.value.length === 0) {
      activeTabId.value = null
    } else {
      // 优先激活右侧邻居，越界则回退左侧
      const nextIdx = Math.min(idx, tabs.value.length - 1)
      activeTabId.value = tabs.value[nextIdx].id
    }
  }
  return removed
}

/**
 * 激活指定页签（若存在）。
 */
function activate(id: string): void {
  if (tabs.value.some((t) => t.id === id)) {
    activeTabId.value = id
  }
}

/**
 * 更新页签字段。若 partial.id 变更：
 *   - 目标 id 已被其它页签占用 → 合并：移除当前页签、激活已存在者、把 partial（除 id）合并过去
 *   - 否则原地改 id，并同步 activeTabId（若当前页签是激活的）
 */
function updateTab(id: string, partial: Partial<TabItem>): void {
  const idx = tabs.value.findIndex((t) => t.id === id)
  if (idx === -1) return
  const target = tabs.value[idx]

  const newId = partial.id
  if (newId !== undefined && newId !== id) {
    const collisionIdx = tabs.value.findIndex((t) => t.id === newId)
    if (collisionIdx !== -1 && collisionIdx !== idx) {
      // 合并：移除当前页签，激活已存在者
      tabs.value.splice(idx, 1)
      const existing = tabs.value.find((t) => t.id === newId)
      if (existing) {
        const { id: _omit, ...rest } = partial
        void _omit
        Object.assign(existing, rest)
      }
      activeTabId.value = newId
      return
    }
    // 原地改 id
    const updated: TabItem = { ...target, ...partial }
    tabs.value.splice(idx, 1, updated)
    if (activeTabId.value === id) {
      activeTabId.value = newId
    }
    return
  }

  // 普通字段更新
  tabs.value.splice(idx, 1, { ...target, ...partial })
}

/**
 * 重命名页签标题。
 */
function renameTab(id: string, title: string): void {
  updateTab(id, { title })
}

/**
 * 获取当前激活页签的 computed 引用（跨调用共享同一缓存）。
 */
function getActive(): ComputedRef<TabItem | null> {
  return activeTab
}

/**
 * 按 conversationId 查找 chat 页签（用于 HistoryRail 重复打开去重）。
 */
function findChatByConversationId(conversationId: string): TabItem | undefined {
  return tabs.value.find((t) => t.kind === 'chat' && t.conversationId === conversationId)
}

export interface UseTabsReturn {
  tabs: Ref<TabItem[]>
  activeTabId: Ref<string | null>
  openTab: typeof openTab
  closeTab: typeof closeTab
  activate: typeof activate
  updateTab: typeof updateTab
  renameTab: typeof renameTab
  getActive: typeof getActive
  findChatByConversationId: typeof findChatByConversationId
}

export function useTabs(): UseTabsReturn {
  return {
    tabs,
    activeTabId,
    openTab,
    closeTab,
    activate,
    updateTab,
    renameTab,
    getActive,
    findChatByConversationId,
  }
}

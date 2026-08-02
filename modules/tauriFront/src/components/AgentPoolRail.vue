<script setup lang="ts">
/**
 * AgentPoolRail 交流池二级栏目（左2栏）
 *
 * 当用户在左1栏（IconRail）点击"交流池"图标时，左2栏从 HistoryRail 切换为本栏。
 * 本栏展示运行时 agent 公共会话交流池的全部条目：
 *
 * - 顶部：标题 + 刷新按钮 + 清空按钮 + 活跃/待@ 计数
 * - 视图切换：「条目」/「@ 消息」
 *   · 条目视图：按状态分组展示条目（进行中 / 等待中 / 已完成）
 *   · @ 消息视图：全部 @ 消息的公开时间线（类似群聊消息流，谁 @ 了谁、问了什么、是否已回复）
 * - 单条目渲染 → AgentPoolItem 组件（展开查看研究报告 / todoTree / @ 消息）
 *
 * 数据来源：useAgentPool composable（单例状态 + agent-pool-updated 事件实时刷新）
 *
 * 设计原则：
 * - 简约：与 HistoryRail 同宽（248px），复用 design tokens
 * - 一致：hover/active 态与 HistoryItem 视觉对齐
 * - 实时：监听 agent-pool-updated 事件，条目变更自动刷新
 * - 模块化：单条目渲染抽到 AgentPoolItem，本栏只负责容器与列表
 */
import { ref, computed, onMounted, nextTick } from 'vue'
import { animate } from 'animejs'
import { Icon, Dialog, useToast } from './basic'
import AgentPoolItem from './AgentPoolItem.vue'
import { useAgentPool } from '../composables/useAgentPool'
import type { PoolEntry, PoolStatus } from '../types'

const { toast } = useToast()
const {
  entries,
  loading,
  byStatus,
  activeCount,
  pendingAtCount,
  publicAtFeed,
  refresh,
  clearPool,
  formatRelativeTime,
} = useAgentPool()

// ---------- 视图切换 ----------
type ViewMode = 'entries' | 'atFeed'
const viewMode = ref<ViewMode>('entries')
const viewContentRef = ref<HTMLElement | null>(null)
let viewSwitching = false

async function switchView(mode: ViewMode) {
  if (viewMode.value === mode || viewSwitching) return
  viewSwitching = true
  const el = viewContentRef.value
  if (el) {
    // 淡出 → 切换 → 淡入
    await new Promise<void>((resolve) => {
      animate(el, {
        opacity: [1, 0],
        duration: 120,
        ease: 'inOut(2)',
        onComplete: () => resolve(),
      })
    })
    viewMode.value = mode
    await nextTick()
    animate(el, {
      opacity: [0, 1],
      duration: 160,
      ease: 'out(3)',
      onComplete: () => {
        viewSwitching = false
      },
    })
  } else {
    viewMode.value = mode
    viewSwitching = false
  }
}

// ---------- 状态筛选（条目视图） ----------

type FilterKey = 'all' | PoolStatus
const filter = ref<FilterKey>('all')

const filterOptions: { key: FilterKey; label: string; count: () => number }[] = [
  { key: 'all', label: '全部', count: () => entries.value.length },
  { key: 'in_progress', label: '进行中', count: () => byStatus.value.inProgress.length },
  { key: 'waiting', label: '等待中', count: () => byStatus.value.waiting.length },
  { key: 'completed', label: '已完成', count: () => byStatus.value.completed.length },
]

/** 按当前筛选过滤后的条目（按状态优先级排序：进行中 > 等待中 > 已完成） */
const filteredEntries = computed<PoolEntry[]>(() => {
  if (filter.value === 'all') {
    return [...entries.value].sort((a, b) => {
      const pa = statusPriority(a.status)
      const pb = statusPriority(b.status)
      if (pa !== pb) return pa - pb
      return b.updated_at - a.updated_at
    })
  }
  return entries.value.filter((e) => e.status === filter.value)
})

function statusPriority(s: PoolStatus): number {
  if (s === 'in_progress') return 0
  if (s === 'waiting') return 1
  return 2
}

/** 分组展示（仅 'all' 模式启用分组；其它筛选模式直接平铺） */
const groups = computed<{ label: string; status: PoolStatus | null; items: PoolEntry[] }[]>(() => {
  if (filter.value !== 'all') {
    return [{ label: '', status: null, items: filteredEntries.value }]
  }
  const list: { label: string; status: PoolStatus | null; items: PoolEntry[] }[] = [
    { label: '进行中', status: 'in_progress', items: byStatus.value.inProgress },
    { label: '等待中', status: 'waiting', items: byStatus.value.waiting },
    { label: '已完成', status: 'completed', items: byStatus.value.completed },
  ]
  return list.filter((g) => g.items.length > 0)
})

// ---------- 已完成分组折叠 ----------
const completedCollapsed = ref(false)

// ---------- 清空确认对话框 ----------
const clearDialogVisible = ref(false)

async function confirmClear() {
  const ok = await clearPool()
  clearDialogVisible.value = false
  if (ok) {
    toast({ content: '交流池已清空', type: 'success' })
  } else {
    toast({ content: '清空失败', type: 'error' })
  }
}

// ---------- 刷新 ----------
async function onRefresh() {
  await refresh()
  toast({ content: '交流池已刷新', type: 'success' })
}

// ---------- 初始加载 ----------
onMounted(() => {
  // 进入视图时拉取一次最新数据（composable 已自动监听事件）
  void refresh()
})
</script>

<template>
  <aside class="ap-rail">
    <!-- 顶部：标题 + 操作 -->
    <header class="ap-rail-head">
      <div class="ap-rail-title-row">
        <span class="ap-rail-title">
          <Icon name="merge" :size="16" />
          Agent 交流池
        </span>
        <div class="ap-rail-actions">
          <button
            type="button"
            class="ap-rail-action"
            :class="{ 'is-loading': loading }"
            :disabled="loading"
            title="刷新"
            aria-label="刷新交流池"
            @click="onRefresh"
          >
            <Icon :name="loading ? 'loader' : 'refresh'" :size="14" />
          </button>
          <button
            type="button"
            class="ap-rail-action ap-rail-action--danger"
            title="清空交流池"
            aria-label="清空交流池"
            :disabled="entries.length === 0"
            @click="clearDialogVisible = true"
          >
            <Icon name="delete" :size="14" />
          </button>
        </div>
      </div>
      <!-- 计数摘要 -->
      <div class="ap-rail-stats">
        <span class="ap-rail-stat">
          <span class="ap-rail-stat-dot status-in_progress" />
          活跃 {{ activeCount }}
        </span>
        <span class="ap-rail-stat">
          <Icon name="message" :size="11" />
          待 @ {{ pendingAtCount }}
        </span>
        <span class="ap-rail-stat ap-rail-stat--total">
          共 {{ entries.length }} 条
        </span>
      </div>
    </header>

    <!-- 视图切换：条目 / @ 消息 -->
    <div class="ap-rail-view-toggle">
      <button
        type="button"
        class="ap-rail-view-btn"
        :class="{ active: viewMode === 'entries' }"
        @click="switchView('entries')"
      >
        <Icon name="list" :size="13" />
        条目
        <span class="ap-rail-view-count">{{ entries.length }}</span>
      </button>
      <button
        type="button"
        class="ap-rail-view-btn"
        :class="{ active: viewMode === 'atFeed' }"
        @click="switchView('atFeed')"
      >
        <Icon name="at" :size="13" />
        @ 消息
        <span class="ap-rail-view-count" :class="{ 'has-pending': pendingAtCount > 0 }">
          {{ publicAtFeed.length }}
        </span>
      </button>
    </div>

    <!-- 视图内容区（淡入淡出过渡） -->
    <div ref="viewContentRef" class="ap-rail-view-content">
      <!-- ═════════ 条目视图 ═════════ -->
      <template v-if="viewMode === 'entries'">
        <!-- 状态筛选 chips -->
        <div class="ap-rail-filters">
          <button
            v-for="opt in filterOptions"
            :key="opt.key"
            type="button"
            class="ap-rail-chip"
            :class="{ active: filter === opt.key }"
            @click="filter = opt.key"
          >
            {{ opt.label }}
            <span class="ap-rail-chip-count">{{ opt.count() }}</span>
          </button>
        </div>

        <!-- 列表主体 -->
        <div class="ap-rail-body">
          <!-- 空状态 -->
          <div v-if="filteredEntries.length === 0 && !loading" class="ap-rail-empty">
            <Icon name="merge" :size="28" />
            <p class="ap-rail-empty-title">
              {{ entries.length === 0 ? '交流池为空' : '该状态下无条目' }}
            </p>
            <p class="ap-rail-empty-hint">
              {{ entries.length === 0
                ? 'Agent 执行长任务时会在此登记状态，其他 agent 可通过 @ 机制协作'
                : '切换其他筛选条件查看' }}
            </p>
          </div>

          <!-- 加载中骨架 -->
          <div v-else-if="loading && entries.length === 0" class="ap-rail-loading">
            <Icon name="loader" :size="20" />
            <span>加载中...</span>
          </div>

          <!-- 分组列表 -->
          <template v-else>
            <template v-for="g in groups" :key="g.label || 'flat'">
              <!-- 分组标题（仅 all 模式 + 有 label 时显示） -->
              <div
                v-if="g.label && filter === 'all'"
                class="ap-rail-group-head"
                :class="{ collapsible: g.status === 'completed' }"
                @click="g.status === 'completed' && (completedCollapsed = !completedCollapsed)"
              >
                <span
                  v-if="g.status === 'completed'"
                  class="ap-rail-group-arrow"
                  :class="{ collapsed: completedCollapsed }"
                >
                  <Icon name="chevron-down" :size="12" />
                </span>
                <span class="ap-rail-group-label">{{ g.label }}</span>
                <span class="ap-rail-group-count">{{ g.items.length }}</span>
              </div>

              <!-- 分组条目（已完成分组在折叠态下隐藏） -->
              <div
                v-if="!(g.status === 'completed' && completedCollapsed)"
                class="ap-rail-group-items"
              >
                <AgentPoolItem
                  v-for="e in g.items"
                  :key="e.agent_id"
                  :entry="e"
                />
              </div>
            </template>
          </template>
        </div>
      </template>

      <!-- ═════════ @ 消息公开流视图 ═════════ -->
      <template v-else>
        <div class="ap-rail-body ap-at-feed-body">
          <!-- 空状态 -->
          <div v-if="publicAtFeed.length === 0 && !loading" class="ap-rail-empty">
            <Icon name="at" :size="28" />
            <p class="ap-rail-empty-title">暂无 @ 消息</p>
            <p class="ap-rail-empty-hint">
              Agent 之间通过 @ 机制协作时，消息会在此公开同步
            </p>
          </div>

          <!-- 加载中 -->
          <div v-else-if="loading && publicAtFeed.length === 0" class="ap-rail-loading">
            <Icon name="loader" :size="20" />
            <span>加载中...</span>
          </div>

          <!-- @ 消息列表 -->
          <template v-else>
            <div
              v-for="item in publicAtFeed"
              :key="item.at_id"
              class="ap-at-item"
              :class="`at-${item.status}`"
            >
              <!-- 消息头：from → to + 时间 -->
              <div class="ap-at-item-head">
                <div class="ap-at-item-route">
                  <span class="ap-at-item-from">{{ item.from_name || item.from }}</span>
                  <Icon name="arrow-right" :size="11" class="ap-at-item-arrow" />
                  <span class="ap-at-item-to">{{ item.to_name || item.to_agent_id }}</span>
                </div>
                <span class="ap-at-item-time">{{ formatRelativeTime(item.created_at) }}</span>
              </div>
              <!-- 提问内容 -->
              <div class="ap-at-item-question">{{ item.question }}</div>
              <!-- 回复 -->
              <div v-if="item.reply" class="ap-at-item-reply">
                <Icon name="enter" :size="11" />
                <span>{{ item.reply }}</span>
              </div>
              <!-- 待回复标记 -->
              <div v-else-if="item.status === 'pending'" class="ap-at-item-pending">
                <span class="ap-at-item-pending-dot" />
                待回复
              </div>
            </div>
          </template>
        </div>
      </template>
    </div>

    <!-- 底部提示 -->
    <footer class="ap-rail-foot">
      <p class="ap-rail-hint">
        <Icon name="info" :size="12" />
        左侧图标栏切换回聊天
      </p>
    </footer>

    <!-- 清空确认对话框（复用基础组件） -->
    <Dialog
      v-model:visible="clearDialogVisible"
      title="清空交流池"
      danger
      confirm-text="清空"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmClear"
    >
      <div class="ap-clear-body">
        确定清空全部 {{ entries.length }} 条交流池条目？此操作不可撤销，但不会影响会话本身。
      </div>
    </Dialog>
  </aside>
</template>

<style scoped>
.ap-rail {
  display: flex;
  flex-direction: column;
  width: 248px;
  flex-shrink: 0;
  background: var(--bg-2);
  border-right: 1px solid var(--border);
  overflow: hidden;
  user-select: none;
}

/* 顶部 */
.ap-rail-head {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 12px 12px 8px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}

.ap-rail-title-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
}

.ap-rail-title {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
  letter-spacing: 0.2px;
}

.ap-rail-actions {
  display: inline-flex;
  align-items: center;
  gap: 2px;
}

.ap-rail-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.ap-rail-action:hover {
  background: var(--card);
  color: var(--text);
}

.ap-rail-action:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

.ap-rail-action.is-loading {
  color: var(--primary);
}

.ap-rail-action.is-loading :deep(svg) {
  animation: ap-rail-spin 0.8s linear infinite;
}

.ap-rail-action--danger:hover {
  color: var(--danger);
}

@keyframes ap-rail-spin {
  to { transform: rotate(360deg); }
}

/* 计数摘要 */
.ap-rail-stats {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 11px;
  color: var(--muted);
}

.ap-rail-stat {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.ap-rail-stat-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
}

.ap-rail-stat-dot.status-in_progress {
  background: var(--primary, #4a7eff);
  animation: ap-rail-pulse 1.2s ease-in-out infinite;
}

@keyframes ap-rail-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

.ap-rail-stat--total {
  margin-left: auto;
}

/* 视图切换 */
.ap-rail-view-toggle {
  display: flex;
  gap: 2px;
  padding: 6px 12px;
  flex-shrink: 0;
  border-bottom: 1px solid var(--border);
}

.ap-rail-view-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex: 1;
  justify-content: center;
  padding: 5px 8px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--muted);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.ap-rail-view-btn:hover {
  background: var(--card);
  color: var(--text);
}

.ap-rail-view-btn.active {
  background: var(--card);
  color: var(--primary);
}

.ap-rail-view-count {
  font-size: 10px;
  font-weight: 400;
  opacity: 0.8;
  padding: 0 4px;
  border-radius: var(--radius-full);
  background: var(--bg-2);
}

.ap-rail-view-count.has-pending {
  background: rgba(240, 92, 92, 0.9);
  color: #fff;
  opacity: 1;
  animation: ap-at-badge-pop 300ms var(--ease-decelerated);
}

@keyframes ap-at-badge-pop {
  0% { transform: scale(0); }
  60% { transform: scale(1.15); }
  100% { transform: scale(1); }
}

/* 视图内容区 */
.ap-rail-view-content {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
}

/* 筛选 chips */
.ap-rail-filters {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 8px 12px;
  flex-shrink: 0;
  overflow-x: auto;
  scrollbar-width: none;
}

.ap-rail-filters::-webkit-scrollbar {
  display: none;
}

.ap-rail-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 4px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--muted);
  font-size: 11px;
  font-weight: 500;
  cursor: pointer;
  white-space: nowrap;
  flex-shrink: 0;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard),
    border-color var(--duration-fast) var(--ease-standard);
}

.ap-rail-chip:hover {
  background: var(--card);
  color: var(--text);
}

.ap-rail-chip.active {
  background: rgba(74, 126, 255, 0.14);
  color: var(--primary);
  border-color: rgba(74, 126, 255, 0.3);
}

.ap-rail-chip-count {
  font-size: 10px;
  font-weight: 400;
  opacity: 0.8;
}

/* 列表主体 */
.ap-rail-body {
  flex: 1;
  overflow-y: auto;
  min-height: 0;
  padding: 4px 8px 12px;
}

.ap-rail-body::-webkit-scrollbar {
  width: 6px;
}

.ap-rail-body::-webkit-scrollbar-thumb {
  background: var(--border);
  border-radius: 3px;
}

.ap-rail-body::-webkit-scrollbar-thumb:hover {
  background: var(--muted);
}

/* 分组标题 */
.ap-rail-group-head {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 10px 8px 6px;
  font-size: 12px;
  font-weight: 600;
  color: var(--muted);
}

.ap-rail-group-head.collapsible {
  cursor: pointer;
}

.ap-rail-group-head.collapsible:hover {
  color: var(--text);
}

.ap-rail-group-arrow {
  display: inline-flex;
  color: var(--muted);
  transition: transform var(--duration-fast) var(--ease-standard);
}

.ap-rail-group-arrow.collapsed {
  transform: rotate(-90deg);
}

.ap-rail-group-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--muted);
}

.ap-rail-group-count {
  font-size: 11px;
  font-weight: 400;
  color: var(--muted);
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  padding: 0 6px;
  margin-left: 2px;
}

.ap-rail-group-items {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

/* 空状态 */
.ap-rail-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 40px 16px 24px;
  text-align: center;
  color: var(--muted);
}

.ap-rail-empty :deep(svg) {
  opacity: 0.5;
}

.ap-rail-empty-title {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}

.ap-rail-empty-hint {
  margin: 0;
  font-size: 11px;
  color: var(--muted);
  line-height: 1.5;
}

/* 加载中 */
.ap-rail-loading {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 32px 16px;
  color: var(--muted);
  font-size: 12px;
}

.ap-rail-loading :deep(svg) {
  animation: ap-rail-spin 0.8s linear infinite;
  color: var(--primary);
}

/* ═══ @ 消息公开流 ═══ */
.ap-at-feed-body {
  padding: 6px 8px 12px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.ap-at-item {
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 8px 10px;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.ap-at-item:hover {
  border-color: color-mix(in srgb, var(--primary) 20%, var(--border));
}

.ap-at-item.at-pending {
  border-left: 2px solid var(--warn, #f0c04a);
}

.ap-at-item.at-answered {
  border-left: 2px solid var(--success, #3ecf8e);
}

.ap-at-item-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  margin-bottom: 4px;
}

.ap-at-item-route {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  font-weight: 600;
  min-width: 0;
}

.ap-at-item-from {
  color: var(--primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 80px;
}

.ap-at-item-arrow {
  color: var(--muted);
  flex-shrink: 0;
}

.ap-at-item-to {
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 80px;
}

.ap-at-item-time {
  font-size: 10px;
  color: var(--muted);
  flex-shrink: 0;
}

.ap-at-item-question {
  font-size: 12px;
  color: var(--text);
  line-height: 1.5;
  word-break: break-word;
}

.ap-at-item-reply {
  display: flex;
  align-items: flex-start;
  gap: 4px;
  margin-top: 6px;
  padding-top: 6px;
  border-top: 1px dashed var(--border);
  font-size: 12px;
  color: var(--success);
  line-height: 1.5;
  word-break: break-word;
}

.ap-at-item-reply :deep(svg) {
  flex-shrink: 0;
  margin-top: 2px;
}

.ap-at-item-pending {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin-top: 5px;
  font-size: 11px;
  color: var(--warn);
  font-style: italic;
}

.ap-at-item-pending-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--warn, #f0c04a);
  animation: ap-at-pending-pulse 1.5s ease-in-out infinite;
}

@keyframes ap-at-pending-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

/* 底部 */
.ap-rail-foot {
  padding: 8px 12px;
  border-top: 1px solid var(--border);
  flex-shrink: 0;
}

.ap-rail-hint {
  display: flex;
  align-items: center;
  gap: 4px;
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1.5;
}

/* 清空对话框正文 */
.ap-clear-body {
  font-size: 14px;
  line-height: 1.6;
  color: var(--text);
  padding: 4px 0;
}
</style>

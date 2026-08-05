<script setup lang="ts">
/**
 * AgentPoolRail 交流池二级栏目（左2栏）
 *
 * 当用户在左1栏（IconRail）点击"交流池"图标时，左2栏从 HistoryRail 切换为本栏。
 * 本栏以「会话」为维度展示运行时 agent 公共会话交流池：
 *
 * - 顶部：标题 + 刷新按钮 + 清空按钮 + 统计（活跃会话 / 待@ / 总会话）
 * - 视图切换：「会话」/「@ 消息」
 *   · 会话视图：按会话展示（正在运行的会话 + 交流池登记的长任务）。每个会话
 *     头部展示标题 + 状态 + 活跃窗口数；展开后列出窗口（第一个是主 agent，
 *     其余是子 agent），点击窗口即在 main-content 打开该会话窗口。
 *   · @ 消息视图：全部 @ 消息的公开时间线（群聊消息流，谁 @ 了谁、是否已回复）
 * - 单会话渲染 → AgentPoolItem（展开查看窗口列表 / 研究报告 / todoTree / @ 消息）
 *
 * 数据来源：useAgentPool composable（单例状态 + agent-pool-updated 事件实时刷新；
 * 会话聚合合并了打开的页签窗口 + 交流池条目，将"正在运行的会话"纳入交流池）。
 *
 * 设计原则：
 * - 简约：与 HistoryRail 同宽（248px），复用 design tokens
 * - 一致：hover/active 态与 HistoryItem 视觉对齐
 * - 实时：监听 agent-pool-updated 事件，条目变更自动刷新
 * - 模块化：单会话渲染抽到 AgentPoolItem，窗口行抽到 AgentPoolWindowItem
 */
import { ref, computed, onMounted, nextTick } from 'vue'
import { animate } from 'animejs'
import { Icon, Dialog, useToast } from './basic'
import AgentPoolItem from './AgentPoolItem.vue'
import { useAgentPool } from '../composables/useAgentPool'
import type { PoolWindow } from '../composables/useAgentPool'

const { toast } = useToast()
const {
  sessions,
  activeSessionCount,
  pendingAtCount,
  publicAtFeed,
  refresh,
  clearPool,
  openWindow,
  formatRelativeTime,
} = useAgentPool()

// ---------- 视图切换 ----------
type ViewMode = 'sessions' | 'atFeed'
const viewMode = ref<ViewMode>('sessions')
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

// ---------- 状态筛选（会话视图） ----------

type FilterKey = 'all' | 'active' | 'completed'
const filter = ref<FilterKey>('all')

const filterOptions: { key: FilterKey; label: string; count: () => number }[] = [
  { key: 'all', label: '全部', count: () => sessions.value.length },
  { key: 'active', label: '活跃', count: () => activeSessionCount.value },
  { key: 'completed', label: '已完成', count: () => sessions.value.length - activeSessionCount.value },
]

/** 按当前筛选过滤后的会话 */
const filteredSessions = computed(() => {
  if (filter.value === 'all') return sessions.value
  if (filter.value === 'active') return sessions.value.filter((s) => s.status !== 'completed')
  return sessions.value.filter((s) => s.status === 'completed')
})

/** 是否有正在运行的会话（空状态提示用） */
const hasRunning = computed(() => sessions.value.some((s) => s.status !== 'completed'))

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

// ---------- 打开窗口（替代 main-content 内容） ----------
function onOpenWindow(w: PoolWindow) {
  openWindow(w)
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
            title="刷新"
            aria-label="刷新交流池"
            @click="onRefresh"
          >
            <Icon name="refresh" :size="14" />
          </button>
          <button
            type="button"
            class="ap-rail-action ap-rail-action--danger"
            title="清空交流池"
            aria-label="清空交流池"
            :disabled="sessions.length === 0"
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
          活跃 {{ activeSessionCount }}
        </span>
        <span class="ap-rail-stat">
          <Icon name="message" :size="11" />
          待 @ {{ pendingAtCount }}
        </span>
        <span class="ap-rail-stat ap-rail-stat--total">
          共 {{ sessions.length }} 会话
        </span>
      </div>
    </header>

    <!-- 视图切换：会话 / @ 消息 -->
    <div class="ap-rail-view-toggle">
      <button
        type="button"
        class="ap-rail-view-btn"
        :class="{ active: viewMode === 'sessions' }"
        @click="switchView('sessions')"
      >
        <Icon name="chat" :size="13" />
        会话
        <span class="ap-rail-view-count">{{ sessions.length }}</span>
      </button>
      <button
        type="button"
        class="ap-rail-view-btn"
        :class="{ active: viewMode === 'atFeed' }"
        @click="switchView('atFeed')"
      >
        <Icon name="message" :size="13" />
        @ 消息
        <span class="ap-rail-view-count" :class="{ 'has-pending': pendingAtCount > 0 }">
          {{ publicAtFeed.length }}
        </span>
      </button>
    </div>

    <!-- 视图内容区（淡入淡出过渡） -->
    <div ref="viewContentRef" class="ap-rail-view-content">
      <!-- ═════════ 会话视图 ═════════ -->
      <template v-if="viewMode === 'sessions'">
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
          <div v-if="filteredSessions.length === 0" class="ap-rail-empty">
            <Icon name="merge" :size="28" />
            <p class="ap-rail-empty-title">
              {{ sessions.length === 0 ? '暂无活跃会话' : '该状态下无会话' }}
            </p>
            <p class="ap-rail-empty-hint">
              {{ sessions.length === 0
                ? '打开的会话窗口与 Agent 登记的长任务会在此按会话聚合展示'
                : '切换其他筛选条件查看' }}
            </p>
          </div>

          <!-- 会话列表 -->
          <template v-else>
            <!-- 活跃会话分组（进行中 / 等待中 / 活跃） -->
            <div
              v-if="filter !== 'completed' && sessions.some((s) => s.status !== 'completed')"
              class="ap-rail-group-head"
            >
              <span class="ap-rail-group-label">活跃会话</span>
              <span class="ap-rail-group-count">{{ activeSessionCount }}</span>
            </div>
            <div
              v-if="filter !== 'completed'"
              class="ap-rail-group-items"
            >
              <AgentPoolItem
                v-for="s in filteredSessions.filter((x) => x.status !== 'completed')"
                :key="s.conversationId"
                :session="s"
                @open-window="onOpenWindow"
              />
            </div>

            <!-- 已完成分组 -->
            <template v-if="filteredSessions.some((s) => s.status === 'completed')">
              <div class="ap-rail-group-head">
                <span class="ap-rail-group-label">已完成</span>
                <span class="ap-rail-group-count">
                  {{ filteredSessions.filter((x) => x.status === 'completed').length }}
                </span>
              </div>
              <div class="ap-rail-group-items">
                <AgentPoolItem
                  v-for="s in filteredSessions.filter((x) => x.status === 'completed')"
                  :key="s.conversationId"
                  :session="s"
                  @open-window="onOpenWindow"
                />
              </div>
            </template>
          </template>

          <!-- 运行提示（非空但无正在运行的会话） -->
          <div
            v-if="filter === 'all' && sessions.length > 0 && !hasRunning"
            class="ap-rail-running-hint"
          >
            <Icon name="info" :size="12" />
            当前无正在运行的会话窗口
          </div>
        </div>
      </template>

      <!-- ═════════ @ 消息公开流视图 ═════════ -->
      <template v-else>
        <div class="ap-rail-body ap-at-feed-body">
          <!-- 空状态 -->
          <div v-if="publicAtFeed.length === 0" class="ap-rail-empty">
            <Icon name="message" :size="28" />
            <p class="ap-rail-empty-title">暂无 @ 消息</p>
            <p class="ap-rail-empty-hint">
              Agent 之间通过 @ 机制协作时，消息会在此公开同步
            </p>
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
        展开会话查看窗口，点击窗口在主区打开
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
        确定清空全部 {{ sessions.length }} 条交流池会话记录？此操作不可撤销，但不会影响会话本身。
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
  background: var(--bg-rail-2);
  border-right: 1px solid var(--border-strong);
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
  border-radius: var(--radius-md);
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

.ap-rail-action--danger:hover {
  color: var(--danger);
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
  border-radius: var(--radius-md);
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

/* 运行提示 */
.ap-rail-running-hint {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 10px 12px;
  font-size: 11px;
  color: var(--muted);
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
  border-radius: var(--radius-md);
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

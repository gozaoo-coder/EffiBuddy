<script setup lang="ts">
/**
 * AgentPoolItem —— 交流池会话卡片（会话维度）
 *
 * 从 AgentPoolRail.vue 抽取的独立组件，展示一个"会话"及其全部活跃窗口：
 * - 折叠态头部：会话标题 + 状态徽标 + 活跃窗口数 + @ 角标 + 折叠箭头 + 相对时间
 * - 展开态：
 *   · 活跃窗口列表（主 agent 第一个，子 agent 其后）→ 点击在 main-content 打开
 *   · 主 agent 的研究报告 / todoTree 摘要
 *   · 该会话全部 @ 消息（含提问 / 回复）
 *
 * 动画管线设计（全流程）：
 * - 展开/折叠：anime.js 驱动 maxHeight + opacity 过渡，避免 v-if 立即卸载 DOM
 *   · 展开：从 maxHeight:0,opacity:0 → scrollHeight,opacity:1（260ms out(3)）
 *   · 折叠：从 scrollHeight,opacity:1 → 0,0（200ms inOut(2)）
 *   · 中断处理：anime.js 自动覆盖正在进行的动画，无缝接续
 * - 状态徽标 pulse：进行中状态 dot 1.2s 无限脉冲
 */
import { ref, computed, nextTick } from 'vue'
import { animate } from 'animejs'
import { Icon } from './basic'
import AgentPoolWindowItem from './AgentPoolWindowItem.vue'
import type { PoolSession, PoolWindow } from '../composables/useAgentPool'
import { useAgentPool } from '../composables/useAgentPool'

const props = defineProps<{
  /** 会话模型（由 useAgentPool().sessions 提供） */
  session: PoolSession
}>()

const emit = defineEmits<{
  /** 点击窗口行：请求在 main-content 打开该窗口 */
  (e: 'open-window', w: PoolWindow): void
}>()

const { statusLabel, formatRelativeTime } = useAgentPool()

// 折叠状态：默认折叠（用户主动点击展开查看窗口列表）
const collapsed = ref(true)
const bodyRef = ref<HTMLElement | null>(null)
let animating = false

function toggle() {
  if (animating) return
  animating = true
  if (collapsed.value) {
    collapsed.value = false
    nextTick(() => {
      const el = bodyRef.value
      if (!el) {
        animating = false
        return
      }
      const h = el.scrollHeight
      animate(el, {
        maxHeight: ['0px', `${h}px`],
        opacity: [0, 1],
        duration: 260,
        ease: 'out(3)',
        onComplete: () => {
          el.style.maxHeight = ''
          animating = false
        },
      })
    })
  } else {
    const el = bodyRef.value
    if (!el) {
      collapsed.value = true
      animating = false
      return
    }
    animate(el, {
      maxHeight: [`${el.scrollHeight}px`, '0px'],
      opacity: [1, 0],
      duration: 200,
      ease: 'inOut(2)',
      onComplete: () => {
        collapsed.value = true
        el.style.maxHeight = ''
        animating = false
      },
    })
  }
}

/** 主 agent 条目（研究报告 / todoTree 来源） */
const mainEntry = computed(() =>
  props.session.entries.find((e) => e.kind === 'main'),
)

/** 该会话全部 @ 消息（跨条目聚合，按时间倒序） */
const inbox = computed(() => {
  const list: { from_name: string; question: string; reply?: string | null; status: string; created_at: number }[] = []
  for (const e of props.session.entries) {
    for (const m of e.inbox) {
      list.push({
        from_name: m.from_name,
        question: m.question,
        reply: m.reply,
        status: m.status,
        created_at: m.created_at,
      })
    }
  }
  list.sort((a, b) => b.created_at - a.created_at)
  return list
})

/** 截断长文本 */
function truncate(s: string, max: number): string {
  const t = s.trim()
  if (t.length <= max) return t
  return t.slice(0, max) + '…'
}
</script>

<template>
  <div class="ap-item" :class="`st-${session.status}`">
    <!-- 头部 -->
    <div class="ap-item-head" @click="toggle">
      <span class="ap-item-glyph">
        <Icon name="merge" :size="14" />
      </span>

      <div class="ap-item-info">
        <div class="ap-item-title-row">
          <span class="ap-item-name">{{ session.title }}</span>
          <span class="ap-item-status" :class="`st-${session.status}`">
            <span class="ap-item-status-dot" />
            {{ statusLabel(session.status) }}
          </span>
        </div>
        <div class="ap-item-meta">
          <span class="ap-item-wins" :title="`${session.activeCount} 个活跃窗口`">
            <Icon name="chat" :size="10" />
            {{ session.activeCount }} 窗口
          </span>
          <template v-if="session.pendingAtCount > 0">
            <span class="ap-item-at-count">
              <Icon name="message" :size="10" />
              {{ session.pendingAtCount }} @
            </span>
          </template>
          <span class="ap-item-time">{{ formatRelativeTime(session.updatedAt) }}</span>
        </div>
      </div>

      <!-- @ 收件箱角标（仅 pending > 0 时显示） -->
      <span
        v-if="session.pendingAtCount > 0"
        class="ap-item-at-badge"
        :title="`${session.pendingAtCount} 条待处理 @ 消息`"
      >
        <Icon name="message" :size="11" />
        {{ session.pendingAtCount > 99 ? '99+' : session.pendingAtCount }}
      </span>

      <!-- 折叠箭头 -->
      <span class="ap-item-arrow" :class="{ expanded: !collapsed }">
        <Icon name="chevron-down" :size="14" />
      </span>
    </div>

    <!-- 展开内容（v-show + anime.js 动画） -->
    <div v-show="!collapsed" ref="bodyRef" class="ap-item-body">
      <div class="ap-item-body-inner">
        <!-- 活跃窗口列表：第一个是主 agent，其余是子 agent -->
        <div class="ap-item-section">
          <div class="ap-item-section-label">
            <Icon name="chat" :size="12" />
            活跃窗口（{{ session.windows.length }}）
            <span class="ap-item-section-hint">点击进入查看</span>
          </div>
          <div class="ap-item-windows">
            <AgentPoolWindowItem
              v-for="w in session.windows"
              :key="`${w.kind}-${w.windowId}`"
              :window="w"
              @open="(win) => emit('open-window', win)"
            />
          </div>
        </div>

        <!-- 主 agent 研究报告 -->
        <div
          v-if="mainEntry && mainEntry.research_report.trim()"
          class="ap-item-section"
        >
          <div class="ap-item-section-label">
            <Icon name="book" :size="12" />
            研究报告
          </div>
          <div class="ap-item-section-text">{{ mainEntry.research_report }}</div>
        </div>

        <!-- 主 agent todoTree 摘要 -->
        <div
          v-if="mainEntry && mainEntry.todo_summary.trim()"
          class="ap-item-section"
        >
          <div class="ap-item-section-label">
            <Icon name="check" :size="12" />
            todoTree
          </div>
          <pre class="ap-item-section-pre">{{ mainEntry.todo_summary }}</pre>
        </div>

        <!-- @ 消息 -->
        <div v-if="inbox.length > 0" class="ap-item-section">
          <div class="ap-item-section-label">
            <Icon name="message" :size="12" />
            @ 消息（{{ inbox.length }}）
          </div>
          <div class="ap-item-inbox">
            <div
              v-for="(m, i) in inbox"
              :key="i"
              class="ap-item-at-msg"
              :class="`at-${m.status}`"
            >
              <div class="ap-item-at-q">
                <span class="ap-item-at-from">{{ m.from_name || '未知' }}</span>
                <span class="ap-item-at-time">{{ formatRelativeTime(m.created_at) }}</span>
              </div>
              <div class="ap-item-at-text">{{ m.question }}</div>
              <div v-if="m.reply" class="ap-item-at-reply">
                <Icon name="enter" :size="11" />
                {{ m.reply }}
              </div>
              <div v-else-if="m.status === 'pending'" class="ap-item-at-pending">
                待回复
              </div>
            </div>
          </div>
        </div>

        <!-- 元信息 -->
        <div class="ap-item-meta-detail">
          <span>conversation: {{ truncate(session.conversationId, 12) }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.ap-item {
  display: flex;
  flex-direction: column;
  padding: 9px 10px;
  border-radius: var(--radius-sm);
  background: transparent;
  border: 1px solid transparent;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    border-color var(--duration-fast) var(--ease-standard);
}

.ap-item:hover {
  background: var(--card);
}

.ap-item.st-in_progress {
  border-left: 2px solid var(--primary, #4a7eff);
}

.ap-item.st-waiting {
  border-left: 2px solid var(--warn, #f0c04a);
}

.ap-item.st-idle {
  border-left: 2px solid var(--muted);
}

.ap-item.st-completed {
  border-left: 2px solid var(--success, #3ecf8e);
  opacity: 0.85;
}

/* 头部 */
.ap-item-head {
  display: flex;
  align-items: flex-start;
  gap: 8px;
}

.ap-item-glyph {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border-radius: var(--radius-xs);
  background: var(--card-2);
  color: var(--muted);
  flex-shrink: 0;
  margin-top: 1px;
}

.ap-item-glyph .app-icon {
  color: var(--primary);
}

.ap-item-info {
  flex: 1;
  min-width: 0;
}

.ap-item-title-row {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.ap-item-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 130px;
}

/* 状态徽标 */
.ap-item-status {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  color: var(--muted);
  white-space: nowrap;
}

.ap-item-status-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--border);
  flex-shrink: 0;
}

.ap-item.st-in_progress .ap-item-status-dot {
  background: var(--primary, #4a7eff);
  animation: ap-item-pulse 1.2s ease-in-out infinite;
}

.ap-item.st-waiting .ap-item-status-dot {
  background: var(--warn, #f0c04a);
}

.ap-item.st-idle .ap-item-status-dot {
  background: var(--muted);
}

.ap-item.st-completed .ap-item-status-dot {
  background: var(--success, #3ecf8e);
}

@keyframes ap-item-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

/* 元信息行 */
.ap-item-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 11px;
  color: var(--muted);
  margin-top: 3px;
  overflow: hidden;
  white-space: nowrap;
}

.ap-item-wins {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 0 5px;
  border-radius: var(--radius-full);
  background: var(--card);
  border: 1px solid var(--border);
  font-size: 10px;
  flex-shrink: 0;
}

.ap-item-at-count {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  color: var(--danger);
  flex-shrink: 0;
}

.ap-item-time {
  flex-shrink: 0;
  margin-left: auto;
}

/* @ 收件箱角标 */
.ap-item-at-badge {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  min-width: 18px;
  height: 18px;
  padding: 0 5px;
  border-radius: var(--radius-full);
  background: rgba(240, 92, 92, 0.92);
  color: #fff;
  font-size: 10px;
  font-weight: 600;
  line-height: 1;
  flex-shrink: 0;
  margin-top: 2px;
  animation: ap-at-pop 300ms var(--ease-decelerated);
  pointer-events: none;
}

@keyframes ap-at-pop {
  0% { transform: scale(0); }
  60% { transform: scale(1.15); }
  100% { transform: scale(1); }
}

/* 折叠箭头 */
.ap-item-arrow {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  color: var(--muted);
  flex-shrink: 0;
  margin-top: 2px;
  transition: transform var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.ap-item-arrow.expanded {
  transform: rotate(180deg);
  color: var(--primary);
}

/* 展开内容 */
.ap-item-body {
  overflow: hidden;
  max-height: 0;
  opacity: 0;
}

.ap-item-body-inner {
  padding: 8px 0 4px 32px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.ap-item-section {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.ap-item-section-label {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  font-weight: 600;
  color: var(--muted);
}

.ap-item-section-hint {
  font-weight: 400;
  opacity: 0.7;
}

.ap-item-windows {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.ap-item-section-text {
  font-size: 12px;
  color: var(--text);
  line-height: 1.5;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-xs);
  padding: 6px 8px;
  white-space: pre-wrap;
  word-break: break-word;
}

.ap-item-section-pre {
  font-family: 'SF Mono', 'JetBrains Mono', 'Consolas', monospace;
  font-size: 11px;
  color: var(--text);
  line-height: 1.5;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-xs);
  padding: 6px 8px;
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
}

/* @ 消息列表 */
.ap-item-inbox {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.ap-item-at-msg {
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-xs);
  padding: 6px 8px;
}

.ap-item-at-msg.at-pending {
  border-left: 2px solid var(--warn, #f0c04a);
}

.ap-item-at-msg.at-answered {
  border-left: 2px solid var(--success, #3ecf8e);
}

.ap-item-at-q {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
}

.ap-item-at-from {
  font-size: 11px;
  font-weight: 600;
  color: var(--primary);
}

.ap-item-at-time {
  font-size: 10px;
  color: var(--muted);
}

.ap-item-at-text {
  font-size: 12px;
  color: var(--text);
  line-height: 1.5;
  margin-top: 2px;
  word-break: break-word;
}

.ap-item-at-reply {
  display: flex;
  align-items: flex-start;
  gap: 4px;
  margin-top: 4px;
  padding-top: 4px;
  border-top: 1px dashed var(--border);
  font-size: 12px;
  color: var(--success);
  line-height: 1.5;
  word-break: break-word;
}

.ap-item-at-reply :deep(svg) {
  flex-shrink: 0;
  margin-top: 2px;
}

.ap-item-at-pending {
  margin-top: 4px;
  padding-top: 4px;
  border-top: 1px dashed var(--border);
  font-size: 11px;
  color: var(--warn);
  font-style: italic;
}

/* 元信息 */
.ap-item-meta-detail {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
  font-size: 10px;
  color: var(--muted);
  font-family: 'SF Mono', 'JetBrains Mono', 'Consolas', monospace;
  padding-top: 4px;
  border-top: 1px dashed var(--border);
}
</style>

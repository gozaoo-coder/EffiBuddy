<script setup lang="ts">
/**
 * AgentPoolItem —— 交流池单条目卡片
 *
 * 从 AgentPoolRail.vue 抽取的独立组件，职责：
 * - 渲染条目头部：名称 + 类型徽标（主/子）+ 状态徽标 + @ 收件箱角标 + 折叠箭头
 * - 折叠态：任务摘要 + 最近上报（若与任务不同）+ 相对时间
 * - 展开态：研究报告 + todoTree 摘要 + 收件箱 @ 消息列表（含提问 / 回复）
 *
 * 动画管线设计（全流程）：
 * - 展开/折叠：anime.js 驱动 maxHeight + opacity 过渡，避免 v-if 立即卸载 DOM
 *   · 展开：从 maxHeight:0,opacity:0 → scrollHeight,opacity:1（260ms out(3)）
 *   · 折叠：从 scrollHeight,opacity:1 → 0,0（200ms inOut(2)）
 *   · 中断处理：anime.js 自动覆盖正在进行的动画，无缝接续
 * - 状态徽标 pulse：进行中状态 dot 1.2s 无限脉冲
 * - hover 背景：CSS background-color 过渡（150ms）
 * - @ 角标 pop：首次出现时 scale(0→1.15→1) 一次性动画
 */
import { ref, nextTick } from 'vue'
import { animate } from 'animejs'
import { Icon } from './basic'
import type { PoolEntry } from '../types'

const props = defineProps<{
  /** 交流池条目 */
  entry: PoolEntry
}>()

// 折叠状态：默认折叠（用户主动点击展开查看详情）
const collapsed = ref(true)
const bodyRef = ref<HTMLElement | null>(null)
let animating = false

function toggle() {
  if (animating) return
  animating = true
  if (collapsed.value) {
    // 展开：先把 v-if 显示出来，下一帧测量 scrollHeight 并动画
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
    // 折叠：先把当前高度动画到 0，再 v-show 隐藏
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

/** 待处理 @ 消息数（pending 状态） */
function pendingAtCount(entry: PoolEntry): number {
  let n = 0
  for (const m of entry.inbox) {
    if (m.status === 'pending') n++
  }
  return n
}

/** 中文状态标签 */
function statusLabel(s: PoolEntry['status']): string {
  if (s === 'in_progress') return '进行中'
  if (s === 'waiting') return '等待中'
  return '已完成'
}

/** 中文条目类型标签 */
function kindLabel(k: PoolEntry['kind']): string {
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

/** 截断长文本 */
function truncate(s: string, max: number): string {
  const t = s.trim()
  if (t.length <= max) return t
  return t.slice(0, max) + '…'
}
</script>

<template>
  <div
    class="ap-item"
    :class="[`status-${entry.status}`, `kind-${entry.kind}`]"
    @click="toggle"
  >
    <!-- 头部 -->
    <div class="ap-item-head">
      <span class="ap-item-glyph" :class="`kind-${entry.kind}`">
        <Icon :name="entry.kind === 'main' ? 'robot' : 'sparkles'" :size="14" />
      </span>

      <div class="ap-item-info">
        <div class="ap-item-title-row">
          <span class="ap-item-name">{{ entry.name || '未命名 agent' }}</span>
          <span class="ap-item-kind" :class="`kind-${entry.kind}`">{{ kindLabel(entry.kind) }}</span>
          <span class="ap-item-status" :class="`status-${entry.status}`">
            <span class="ap-item-status-dot" />
            {{ statusLabel(entry.status) }}
          </span>
        </div>
        <div class="ap-item-task">{{ truncate(entry.task || '（无任务描述）', 60) }}</div>
        <div class="ap-item-meta">
          {{ formatRelativeTime(entry.updated_at) }}
          <template v-if="entry.last_report && entry.last_report !== entry.task">
            · {{ truncate(entry.last_report, 30) }}
          </template>
        </div>
      </div>

      <!-- @ 收件箱角标（仅 pending > 0 时显示） -->
      <span
        v-if="pendingAtCount(entry) > 0"
        class="ap-item-at-badge"
        :title="`${pendingAtCount(entry)} 条待处理 @ 消息`"
      >
        <Icon name="message" :size="11" />
        {{ pendingAtCount(entry) > 99 ? '99+' : pendingAtCount(entry) }}
      </span>

      <!-- 折叠箭头 -->
      <span class="ap-item-arrow" :class="{ expanded: !collapsed }">
        <Icon name="chevron-down" :size="14" />
      </span>
    </div>

    <!-- 展开内容（v-if + anime.js 动画） -->
    <div v-show="!collapsed" ref="bodyRef" class="ap-item-body">
      <div class="ap-item-body-inner">
        <!-- 研究报告 -->
        <div v-if="entry.research_report.trim()" class="ap-item-section">
          <div class="ap-item-section-label">
            <Icon name="book" :size="12" />
            研究报告
          </div>
          <div class="ap-item-section-text">{{ entry.research_report }}</div>
        </div>

        <!-- todoTree 摘要 -->
        <div v-if="entry.todo_summary.trim()" class="ap-item-section">
          <div class="ap-item-section-label">
            <Icon name="check" :size="12" />
            todoTree
          </div>
          <pre class="ap-item-section-pre">{{ entry.todo_summary }}</pre>
        </div>

        <!-- 收件箱 @ 消息 -->
        <div v-if="entry.inbox.length > 0" class="ap-item-section">
          <div class="ap-item-section-label">
            <Icon name="message" :size="12" />
            @ 消息（{{ entry.inbox.length }}）
          </div>
          <div class="ap-item-inbox">
            <div
              v-for="m in entry.inbox"
              :key="m.at_id"
              class="ap-item-at-msg"
              :class="`at-${m.status}`"
            >
              <div class="ap-item-at-q">
                <span class="ap-item-at-from">{{ m.from_name || m.from }}</span>
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
          <span>agent_id: {{ entry.agent_id }}</span>
          <span>conversation: {{ truncate(entry.conversation_id, 12) }}</span>
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

.ap-item.status-in_progress {
  border-left: 2px solid var(--primary, #4a7eff);
}

.ap-item.status-waiting {
  border-left: 2px solid var(--warn, #f0c04a);
}

.ap-item.status-completed {
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

.ap-item-glyph.kind-main {
  color: var(--primary);
  background: rgba(74, 126, 255, 0.12);
}

.ap-item-glyph.kind-sub_agent {
  color: var(--warn);
  background: rgba(240, 192, 74, 0.12);
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
  max-width: 140px;
}

.ap-item-kind {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 18px;
  height: 16px;
  padding: 0 4px;
  border-radius: var(--radius-xs);
  font-size: 10px;
  font-weight: 600;
  line-height: 1;
}

.ap-item-kind.kind-main {
  color: var(--primary);
  background: rgba(74, 126, 255, 0.14);
}

.ap-item-kind.kind-sub_agent {
  color: var(--warn);
  background: rgba(240, 192, 74, 0.14);
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

.ap-item-status.status-in_progress .ap-item-status-dot {
  background: var(--primary, #4a7eff);
  animation: ap-item-pulse 1.2s ease-in-out infinite;
}

.ap-item-status.status-waiting .ap-item-status-dot {
  background: var(--warn, #f0c04a);
}

.ap-item-status.status-completed .ap-item-status-dot {
  background: var(--success, #3ecf8e);
}

@keyframes ap-item-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

.ap-item-task {
  font-size: 12px;
  color: var(--text);
  margin-top: 3px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ap-item-meta {
  font-size: 11px;
  color: var(--muted);
  margin-top: 2px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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

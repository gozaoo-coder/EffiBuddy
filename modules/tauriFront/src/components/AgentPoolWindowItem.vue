<script setup lang="ts">
/**
 * AgentPoolWindowItem —— 交流池会话卡片内的单个窗口行（主 agent / 子 agent）
 *
 * 会话展开后展示的"活跃窗口"列表项：点击即在 main-content 打开该窗口
 * （主 agent → chat 页签；子 agent → sub-agent 页签，替代 main-content 内容）。
 */
import { Icon } from './basic'
import type { PoolWindow, PoolWindowStatus } from '../composables/useAgentPool'
import { useAgentPool } from '../composables/useAgentPool'

const props = defineProps<{
  /** 窗口模型 */
  window: PoolWindow
  /** 是否主 agent 窗口（控制展示样式，主窗口默认展开可打开） */
}>()

const { statusLabel, formatRelativeTime } = useAgentPool()

/** 中文类型标签 */
function kindLabel(k: PoolWindow['kind']): string {
  return k === 'main' ? '主' : '子'
}

/** 任务摘要（截断） */
function taskSummary(task: string): string {
  const t = task.trim()
  if (!t) return props.window.opened ? '窗口已打开，点击切换查看' : '（无任务描述）'
  return t.length > 40 ? t.slice(0, 40) + '…' : t
}

/** 状态样式（含 idle） */
function statusClass(s: PoolWindowStatus): string {
  return s === 'idle' ? 'idle' : `st-${s}`
}
</script>

<template>
  <div
    class="ap-win"
    :class="[`kind-${props.window.kind}`, statusClass(props.window.status)]"
    role="button"
    tabindex="0"
    :title="`打开「${props.window.name}」窗口`"
    @click="$emit('open', props.window)"
    @keyup.enter="$emit('open', props.window)"
  >
    <span class="ap-win-glyph" :class="`kind-${props.window.kind}`">
      <Icon :name="props.window.kind === 'main' ? 'robot' : 'sparkles'" :size="13" />
    </span>

    <div class="ap-win-info">
      <div class="ap-win-title-row">
        <span class="ap-win-name">{{ props.window.name || '未命名' }}</span>
        <span class="ap-win-kind" :class="`kind-${props.window.kind}`">
          {{ kindLabel(props.window.kind) }}
        </span>
      </div>
      <div class="ap-win-task">{{ taskSummary(props.window.task) }}</div>
      <div class="ap-win-meta">
        <span class="ap-win-status">
          <span class="ap-win-status-dot" />
          {{ statusLabel(props.window.status) }}
        </span>
        <template v-if="props.window.atCount > 0">
          <span class="ap-win-at">
            <Icon name="message" :size="10" /> {{ props.window.atCount }} @
          </span>
        </template>
        <span v-if="!props.window.opened" class="ap-win-closed">未打开</span>
        <span class="ap-win-time">{{ formatRelativeTime(props.window.updatedAt) }}</span>
      </div>
    </div>

    <span class="ap-win-open">
      <Icon name="arrow-right" :size="12" />
    </span>
  </div>
</template>

<style scoped>
.ap-win {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border-radius: var(--radius-sm);
  background: var(--bg-2);
  border: 1px solid var(--border);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    border-color var(--duration-fast) var(--ease-standard);
}

.ap-win:hover {
  background: var(--card);
  border-color: color-mix(in srgb, var(--primary) 30%, var(--border));
}

.ap-win:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--primary) 50%, transparent);
  outline-offset: -1px;
}

.ap-win.kind-main {
  border-left: 2px solid var(--primary, #4a7eff);
}

.ap-win.kind-sub_agent {
  border-left: 2px solid var(--warn, #f0c04a);
}

/* 图标 */
.ap-win-glyph {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border-radius: var(--radius-sm);
  background: var(--card-2);
  color: var(--muted);
  flex-shrink: 0;
}

.ap-win-glyph.kind-main {
  color: var(--primary);
  background: rgba(74, 126, 255, 0.12);
}

.ap-win-glyph.kind-sub_agent {
  color: var(--warn);
  background: rgba(240, 192, 74, 0.12);
}

.ap-win-info {
  flex: 1;
  min-width: 0;
}

.ap-win-title-row {
  display: flex;
  align-items: center;
  gap: 5px;
}

.ap-win-name {
  font-size: 12px;
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ap-win-kind {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 14px;
  padding: 0 4px;
  border-radius: var(--radius-sm);
  font-size: 9px;
  font-weight: 600;
  line-height: 1;
  flex-shrink: 0;
}

.ap-win-kind.kind-main {
  color: var(--primary);
  background: rgba(74, 126, 255, 0.14);
}

.ap-win-kind.kind-sub_agent {
  color: var(--warn);
  background: rgba(240, 192, 74, 0.14);
}

.ap-win-task {
  font-size: 11px;
  color: var(--muted);
  margin-top: 2px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ap-win-meta {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 10px;
  color: var(--muted);
  margin-top: 2px;
  overflow: hidden;
  white-space: nowrap;
}

.ap-win-status {
  display: inline-flex;
  align-items: center;
  gap: 3px;
}

.ap-win-status-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--border);
  flex-shrink: 0;
}

.ap-win.st-in_progress .ap-win-status-dot {
  background: var(--primary, #4a7eff);
  animation: ap-win-pulse 1.2s ease-in-out infinite;
}

.ap-win.st-waiting .ap-win-status-dot {
  background: var(--warn, #f0c04a);
}

.ap-win.st-completed .ap-win-status-dot {
  background: var(--success, #3ecf8e);
}

.ap-win.idle .ap-win-status-dot {
  background: var(--muted);
}

@keyframes ap-win-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

.ap-win-at {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  color: var(--danger);
  flex-shrink: 0;
}

.ap-win-closed {
  color: var(--warn);
  flex-shrink: 0;
}

.ap-win-time {
  flex-shrink: 0;
  margin-left: auto;
}

/* 打开箭头 */
.ap-win-open {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  color: var(--muted);
  flex-shrink: 0;
  transition: transform var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.ap-win:hover .ap-win-open {
  color: var(--primary);
  transform: translateX(1px);
}
</style>

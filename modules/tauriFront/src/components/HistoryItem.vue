<script setup lang="ts">
/**
 * HistoryItem —— 历史记录单条会话项
 *
 * 从 HistoryRail.vue 抽取的独立组件，职责：
 * - 渲染会话标题、元信息、置顶标记
 * - 多选模式下显示 checkbox（点击切换选中）
 * - 桌面 hover 显示「自动归类」+「更多」操作按钮
 * - 触屏环境始终显示操作按钮（无 hover）
 * - 长按触发上下文菜单
 *
 * 动画管线设计（全流程）：
 * - 操作按钮 hover 进入：CSS opacity+translateX 过渡（150ms ease-out）
 * - 操作按钮 hover 离开：CSS 反向过渡（120ms）—— CSS transition 天然处理中断
 * - 多选模式切换：checkbox 用 Vue Transition + anime.js scale(0→1) 进入/离开
 * - 选中态背景：CSS background-color 过渡
 * - 自动归类中：loader 图标 CSS rotate 无限旋转
 */
import { ref, computed, onUnmounted } from 'vue'
import { Icon } from './basic'
import { useAnimeTransition } from '../composables/useAnimeTransition'
import type { ConversationMeta } from '../types'

const props = defineProps<{
  conv: ConversationMeta
  active: boolean
  displayTitle: string
  /** 多选模式 */
  selectionMode: boolean
  /** 是否已选中 */
  selected: boolean
  /** 自动归类进行中 */
  classifying: boolean
  /** 是否显示置顶标记 */
  showPin: boolean
  /** 交流池运行状态（进行中/等待中；null = 无活跃长任务，不展示） */
  poolStatus?: 'in_progress' | 'waiting' | 'completed' | null
  /** 交流池登记的任务描述（badge hover 提示） */
  poolTask?: string
}>()
const emit = defineEmits<{
  (e: 'click'): void
  (e: 'contextmenu', ev: MouseEvent): void
  (e: 'pointerdown', ev: PointerEvent): void
  (e: 'pointerup'): void
  (e: 'pointerleave'): void
  (e: 'pointercancel'): void
  (e: 'toggle-select'): void
  (e: 'more', ev: MouseEvent): void
  (e: 'auto-classify'): void
}>()

// checkbox 进入/离开动画：scale + fade
const { onEnter: onCheckEnter, onLeave: onCheckLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    transform: ['scale(0.5)', 'scale(1)'],
    duration: 180,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    transform: ['scale(1)', 'scale(0.5)'],
    duration: 140,
    ease: 'inOut(2)',
  },
})

function onItemClick() {
  if (props.selectionMode) {
    emit('toggle-select')
  } else {
    emit('click')
  }
}

// ---------- hover 悬浮提示卡（Teleport 到 body，避免被导航栏 overflow 裁切） ----------
const tooltip = ref<{ x: number; y: number } | null>(null)
let tooltipTimer: number | null = null

function onItemMouseEnter(e: MouseEvent) {
  const target = e.currentTarget as HTMLElement
  if (!target) return
  if (tooltipTimer) window.clearTimeout(tooltipTimer)
  // 短暂延迟避免扫过列表时频繁闪现
  tooltipTimer = window.setTimeout(() => {
    const rect = target.getBoundingClientRect()
    tooltip.value = {
      x: rect.right + 10,
      y: Math.max(8, Math.min(rect.top - 4, window.innerHeight - 120)),
    }
  }, 350)
}

function onItemMouseLeave() {
  if (tooltipTimer) {
    window.clearTimeout(tooltipTimer)
    tooltipTimer = null
  }
  tooltip.value = null
}

onUnmounted(() => {
  if (tooltipTimer) window.clearTimeout(tooltipTimer)
})

// ---------- 状态 icon：蓝色完成(当前会话) / 灰色已阅 / 绿色环状旋转(进行中) ----------
const statusIcon = computed<{ name: string; cls: string }>(() => {
  if (props.poolStatus === 'in_progress' || props.classifying) {
    return { name: 'loader', cls: 'st-running' }
  }
  if (props.poolStatus === 'waiting') {
    return { name: 'clock', cls: 'st-waiting' }
  }
  if (props.active) {
    return { name: 'check', cls: 'st-done' }
  }
  return { name: 'chat', cls: 'st-read' }
})

function onMoreClick(ev: MouseEvent) {
  ev.stopPropagation()
  emit('more', ev)
}

function onAutoClassifyClick(ev: MouseEvent) {
  ev.stopPropagation()
  if (props.classifying) return
  emit('auto-classify')
}

/** 格式化相对时间（从 HistoryRail 提取的纯函数，避免组件间重复） */
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
</script>

<template>
  <div
    class="hr-item"
    :class="{
      active: active && !selectionMode,
      selected: selected,
      'select-mode': selectionMode,
      classifying: classifying,
    }"
    @click="onItemClick"
    @mouseenter="onItemMouseEnter"
    @mouseleave="onItemMouseLeave"
    @pointerdown="emit('pointerdown', $event)"
    @pointerup="emit('pointerup')"
    @pointerleave="emit('pointerleave')"
    @pointercancel="emit('pointercancel')"
    @contextmenu="emit('contextmenu', $event)"
  >
    <!-- 多选 checkbox（仅多选模式） -->
    <Transition :css="false" @enter="onCheckEnter" @leave="onCheckLeave">
      <span v-if="selectionMode" class="hr-item-check" @click.stop="emit('toggle-select')">
        <span class="hr-item-check-box" :class="{ checked: selected }">
          <Icon v-if="selected" name="check" :size="12" />
        </span>
      </span>
    </Transition>

    <!-- 状态 icon：蓝色完成 / 灰色已阅 / 绿色环状旋转（进行中） -->
    <span class="hr-item-status-icon" :class="statusIcon.cls">
      <Icon :name="statusIcon.name" :size="14" />
    </span>

    <!-- 置顶标记：固定项用 pin -->
    <span v-if="showPin" class="hr-item-pin"><Icon name="pin" :size="12" /></span>

      <div class="hr-item-main">
        <div class="hr-item-title">{{ displayTitle }}</div>
        <div class="hr-item-meta">
          {{ formatRelativeTime(conv.updated_at) }}
          <span
            v-if="poolStatus"
            class="hr-item-status"
            :class="`status-${poolStatus}`"
            :title="poolTask ? `交流池任务：${poolTask}` : '交流池运行状态'"
          >
            <span class="hr-item-status-dot" />
            {{ poolStatus === 'in_progress' ? '进行中' : poolStatus === 'waiting' ? '等待中' : '已完成' }}
          </span>
        </div>
      </div>

    <!-- 操作按钮（多选模式下隐藏） -->
    <div v-if="!selectionMode" class="hr-item-actions">
      <button
        type="button"
        class="hr-item-action"
        :class="{ 'is-loading': classifying }"
        :disabled="classifying"
        :title="classifying ? '归类中...' : '自动归类'"
        :aria-label="classifying ? '归类中' : '自动归类'"
        @click="onAutoClassifyClick"
      >
        <Icon :name="classifying ? 'loader' : 'sparkles'" :size="14" />
      </button>
      <button
        type="button"
        class="hr-item-action"
        title="更多操作"
        aria-label="更多操作"
        @click="onMoreClick"
      >
        <Icon name="more" :size="14" />
      </button>
    </div>

    <!-- hover 悬浮提示卡：完整标题 + 项目路径 + 元信息（Teleport 到 body 避免裁切） -->
    <Teleport to="body">
      <div
        v-if="tooltip"
        class="hr-item-tooltip"
        :style="{ left: tooltip.x + 'px', top: tooltip.y + 'px' }"
      >
        <div class="hr-item-tooltip-title">{{ conv.title?.trim() || '新对话' }}</div>
        <div v-if="conv.working_dir" class="hr-item-tooltip-path">
          <Icon name="folder" :size="12" />
          <span>{{ conv.working_dir }}</span>
        </div>
        <div class="hr-item-tooltip-meta">
          {{ conv.message_count }} 条消息 · {{ formatRelativeTime(conv.updated_at) }}
          <template v-if="poolStatus === 'in_progress'"> · 进行中</template>
          <template v-else-if="poolStatus === 'waiting'"> · 等待中</template>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.hr-item {
  display: flex;
  align-items: flex-start;
  gap: 7px;
  /* 轻量化布局：行高适中，上下留白有呼吸感 */
  padding: 7px 10px;
  margin: 1px 0;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard);
  position: relative;
}

.hr-item:hover {
  background: var(--card);
}

/* 非多选模式下的 active 高亮 */
.hr-item.active {
  background: rgba(74, 126, 255, 0.12);
}

.hr-item.active .hr-item-title {
  color: var(--primary);
  font-size: 11px;
}

/* 多选模式下的选中态 */
.hr-item.selected {
  background: rgba(74, 126, 255, 0.08);
}

.hr-item.selected .hr-item-title {
  color: var(--primary);
}

/* 多选模式下取消 hover 高亮（避免与选中态冲突） */
.hr-item.select-mode:hover {
  background: var(--card);
}

.hr-item.select-mode.selected:hover {
  background: rgba(74, 126, 255, 0.12);
}

/* checkbox */
.hr-item-check {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  margin-top: 1px;
  flex-shrink: 0;
  width: 16px;
  height: 16px;
  cursor: pointer;
}

.hr-item-check-box {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border: 1.5px solid var(--border);
  border-radius: 3px;
  background: transparent;
  color: #fff;
  transition: background var(--duration-fast) var(--ease-standard),
    border-color var(--duration-fast) var(--ease-standard);
}

.hr-item-check-box.checked {
  background: var(--primary);
  border-color: var(--primary);
}

/* 置顶标记 */
.hr-item-pin {
  display: inline-flex;
  margin-top: 2px;
  color: var(--warn);
  flex-shrink: 0;
}

/* 状态 icon：蓝色完成 / 灰色已阅 / 绿色环状旋转 */
.hr-item-status-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  margin-top: 1px;
  flex-shrink: 0;
  color: var(--muted);
}

/* 蓝色完成（当前会话） */
.hr-item-status-icon.st-done {
  color: #4a7eff;
}

/* 灰色已阅（默认） */
.hr-item-status-icon.st-read {
  color: var(--muted);
  opacity: 0.75;
}

/* 等待中 */
.hr-item-status-icon.st-waiting {
  color: var(--warn);
}

/* 正在进行：绿色环状箭头圆形旋转 */
.hr-item-status-icon.st-running {
  color: var(--success);
}

.hr-item-status-icon.st-running :deep(svg) {
  animation: hr-item-status-spin 1s linear infinite;
}

@keyframes hr-item-status-spin {
  to {
    transform: rotate(360deg);
  }
}

/* hover 悬浮提示卡：浅灰背景 / 轻圆角 / 无明显边框 */
.hr-item-tooltip {
  position: fixed;
  z-index: 600;
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-width: 320px;
  padding: 10px 12px;
  background: #f5f5f5;
  color: #1d1d1f;
  border-radius: 8px;
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.18);
  pointer-events: none;
  animation: hr-item-tooltip-in 0.16s var(--ease-standard) both;
}

@keyframes hr-item-tooltip-in {
  from {
    opacity: 0;
    transform: translateX(-4px);
  }
  to {
    opacity: 1;
    transform: none;
  }
}

.hr-item-tooltip-title {
  font-size: 13px;
  font-weight: 600;
  line-height: 1.4;
  word-break: break-all;
}

.hr-item-tooltip-path {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  color: #6e6e73;
  min-width: 0;
}

.hr-item-tooltip-path span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hr-item-tooltip-meta {
  font-size: 11px;
  color: #8e8e93;
}

/* 主体：标题 + 元信息横向排布，两端对齐 */
.hr-item-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: row;
  justify-content: space-between;
  align-items: baseline;
  gap: 8px;
}

.hr-item-title {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  font-weight: 500;
  line-height: 1.4;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hr-item-meta {
  font-size: 11px;
  color: var(--muted);
  flex-shrink: 0;
  white-space: nowrap;
}

/* 操作按钮显示时隐藏 meta，标题撑满整行（操作按钮占据原 meta 区域） */
.hr-item:hover .hr-item-meta {
  display: none;
}

/* 交流池运行状态 badge */
.hr-item-status {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin-left: 6px;
  font-size: 10px;
  color: var(--muted);
  white-space: nowrap;
}

.hr-item-status-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--border);
  flex-shrink: 0;
}

.hr-item-status.status-in_progress .hr-item-status-dot {
  background: var(--primary, #4a7eff);
  animation: hr-item-pulse 1.2s ease-in-out infinite;
}

.hr-item-status.status-waiting .hr-item-status-dot {
  background: var(--warn, #f5a623);
}

.hr-item-status.status-completed .hr-item-status-dot {
  background: var(--success, #34c759);
}

@keyframes hr-item-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.4;
  }
}

/* 操作按钮容器 */
.hr-item-actions {
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
  opacity: 0;
  transform: translateX(4px);
  transition: opacity 0.15s var(--ease-standard),
    transform 0.15s var(--ease-standard);
  align-self: center;
}

.hr-item:hover .hr-item-actions {
  opacity: 1;
  transform: translateX(0);
}

/* 触屏环境：无 hover，始终显示 */
@media (hover: none) {
  .hr-item-actions {
    opacity: 0.6;
    transform: none;
  }

  .hr-item:hover .hr-item-actions {
    opacity: 1;
  }
}

.hr-item-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  padding: 0;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);

  /* 触控目标 ≥ 24px */
}

.hr-item-action:hover {
  background: rgba(0, 0, 0, 0.06);
  color: var(--text);
}

.hr-item-action:disabled {
  cursor: progress;
  opacity: 0.6;
}

.hr-item-action.is-loading {
  color: var(--primary);
}

/* loader 旋转动画 */
.hr-item-action.is-loading :deep(svg) {
  animation: hr-item-spin 0.8s linear infinite;
}

@keyframes hr-item-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>

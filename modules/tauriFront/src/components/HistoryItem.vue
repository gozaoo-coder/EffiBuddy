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

    <!-- 置顶标记 -->
    <span v-if="showPin" class="hr-item-pin"><Icon name="pin" :size="13" /></span>

    <!-- 主体内容 -->
    <div class="hr-item-main">
      <div class="hr-item-title">{{ displayTitle }}</div>
      <div class="hr-item-meta">
        {{ conv.message_count }} 条 · {{ formatRelativeTime(conv.updated_at) }}
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
  </div>
</template>

<style scoped>
.hr-item {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 9px 10px;
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

/* 主体 */
.hr-item-main {
  flex: 1;
  min-width: 0;
}

.hr-item-title {
  font-size: 13px;
  font-weight: 500;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hr-item-meta {
  font-size: 11px;
  color: var(--muted);
  margin-top: 2px;
}

/* 操作按钮组 */
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

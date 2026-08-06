<script setup lang="ts">
/**
 * WidgetCard —— 基础卡片组件
 *
 * 支持：
 * - 尺寸枚举（tiny / small / medium / large / xlarge / wide / tall / full）
 * - 原生指针拖拽（带边界限制、轻扫判定、回弹动画）
 * - 点击 / 关闭 / 自定义交互
 * - 进入/离开动画
 *
 * 动画全流程管线：
 * 1. 开始：卡片 mounted 后执行 enter 动画（opacity 0→1 + scale 0.95→1）
 * 2. 中断：拖拽开始 → pause enter 动画
 * 3. 打断：新 pointerdown 中断旧拖拽 → reset position
 * 4. 返回：拖拽未触发轻扫 → animate 回原位
 * 5. 离开：卡片 unmount 前执行 leave 动画（opacity 1→0 + scale 0.95→1.05）
 * 6. DOM：leave 动画完成后才移除 DOM 元素
 */
import { ref, onMounted, onBeforeUnmount, computed } from 'vue'
import { animate } from 'animejs'
import { useWidgetDrag } from '../../composables/useWidgetDrag'
import { WidgetSize } from '../../types/widget'
import type { DragConfig } from '../../types/widget'

const props = withDefaults(
  defineProps<{
    /** 卡片唯一 id */
    cardId: string
    /** 卡片标题 */
    title?: string
    /** 卡片尺寸 */
    size?: WidgetSize
    /** 自定义 class */
    class?: string
    /** 是否可关闭 */
    closable?: boolean
    /** 拖拽配置 */
    dragConfig?: Partial<DragConfig>
    /** 是否在堆叠模式 */
    stacked?: boolean
    /** 堆叠中的 z-index */
    stackZIndex?: number
    /** 是否为堆叠顶层（可拖拽） */
    isStackTop?: boolean
  }>(),
  {
    title: '',
    size: WidgetSize.MEDIUM,
    class: '',
    closable: false,
    stacked: false,
    stackZIndex: 0,
    isStackTop: true,
  },
)

const emit = defineEmits<{
  (e: 'close', id: string): void
  (e: 'click', id: string): void
  (e: 'drag-start', id: string): void
  (e: 'drag-end', id: string, x: number, y: number, flicked: boolean): void
  (e: 'flick', id: string, direction: 'left' | 'right' | 'up' | 'down'): void
}>()

const cardRef = ref<HTMLElement | null>(null)
const isEntered = ref(false)

// 拖拽（仅非堆叠模式或堆叠顶层启用）
const dragEnabled = computed(() => !props.stacked || props.isStackTop)

const { status, offsetX, offsetY, bind, dispose: disposeDrag } = useWidgetDrag(cardRef, {
  config: { ...props.dragConfig, enabled: dragEnabled.value },
  onDragStart: () => { emit('drag-start', props.cardId) },
  onDragEnd: (x, y, flicked) => { emit('drag-end', props.cardId, x, y, flicked) },
  onFlick: (dir) => { emit('flick', props.cardId, dir) },
})

// 尺寸映射
const sizeClass = computed(() => `widget-size-${props.size}`)

// 进入动画
onMounted(() => {
  if (!cardRef.value || props.stacked) {
    isEntered.value = true
    return
  }
  animate(cardRef.value, {
    opacity: [0, 1],
    scale: [0.95, 1],
    duration: 280,
    ease: 'out(3)',
    onComplete: () => {
      isEntered.value = true
      if (cardRef.value) cardRef.value.style.opacity = ''
    },
  })
})

// 离开动画
onBeforeUnmount(() => {
  disposeDrag()
})

function handleClose(e: MouseEvent) {
  e.stopPropagation()
  emit('close', props.cardId)
}

function handleClick() {
  emit('click', props.cardId)
}
</script>

<template>
  <div
    ref="cardRef"
    :class="[
      'widget-card',
      sizeClass,
      props.class,
      {
        'widget-card--stacked': stacked,
        'widget-card--dragging': status === 'dragging',
        'widget-card--returning': status === 'returning',
        'widget-card--dismissing': status === 'dismissing',
        'widget-card--stack-top': isStackTop,
      },
    ]"
    :data-card-id="cardId"
    :style="{
      zIndex: stackZIndex || undefined,
      '--drag-x': `${offsetX}px`,
      '--drag-y': `${offsetY}px`,
    }"
    v-bind="dragEnabled ? bind : {}"
    @click="handleClick"
  >
    <!-- 卡片头部 -->
    <div class="widget-card__header" v-if="title || closable">
      <span class="widget-card__title" v-if="title">{{ title }}</span>
      <button
        v-if="closable"
        class="widget-card__close"
        @click="handleClose"
        aria-label="关闭"
      >
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
          <path d="M3 3L11 11M11 3L3 11" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
        </svg>
      </button>
    </div>

    <!-- 卡片内容插槽 -->
    <div class="widget-card__body">
      <slot />
    </div>

    <!-- 拖拽状态指示器（堆叠顶层可见） -->
    <div v-if="isStackTop && stacked" class="widget-card__drag-hint">
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
        <circle cx="8" cy="4" r="1.5" fill="currentColor" opacity="0.4"/>
        <circle cx="8" cy="8" r="1.5" fill="currentColor" opacity="0.4"/>
        <circle cx="8" cy="12" r="1.5" fill="currentColor" opacity="0.4"/>
      </svg>
    </div>
  </div>
</template>

<style scoped>
.widget-card {
  --card-radius: var(--radius-lg, 7px);
  --card-bg: var(--card, rgb(40, 40, 45));
  --card-border: var(--border, rgba(255, 255, 255, 0.09));
  --card-shadow: 0 2px 12px rgba(0, 0, 0, 0.25);

  position: relative;
  display: flex;
  flex-direction: column;
  background: var(--card-bg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
  overflow: hidden;
  cursor: default;
  will-change: transform, opacity;
  transition: box-shadow 0.2s ease;
}

.widget-card:hover {
  --card-border: var(--border-strong, rgba(255, 255, 255, 0.16));
}

/* 拖拽中 */
.widget-card--dragging {
  cursor: grabbing;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.35);
  transition: none;
}

.widget-card--returning {
  pointer-events: none;
}

.widget-card--dismissing {
  pointer-events: none;
}

/* 堆叠模式 */
.widget-card--stacked {
  position: absolute;
  inset: 0;
  margin: 0;
}

.widget-card--stack-top {
  cursor: grab;
}

.widget-card--stack-top:active {
  cursor: grabbing;
}

/* ============= 尺寸映射 ============= */
.widget-size-tiny {
  min-width: 120px;
  min-height: 120px;
}

.widget-size-small {
  min-width: 120px;
  min-height: 200px;
}

.widget-size-medium {
  min-width: 200px;
  min-height: 200px;
}

.widget-size-large {
  min-width: 200px;
  min-height: 280px;
}

.widget-size-xlarge {
  min-width: 280px;
  min-height: 280px;
}

.widget-size-wide {
  min-width: 280px;
  min-height: 120px;
}

.widget-size-tall {
  min-width: 120px;
  min-height: 280px;
}

.widget-size-full {
  min-width: 100%;
  min-height: 100%;
}

/* ============= 头部 ============= */
.widget-card__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 10px;
  border-bottom: 1px solid var(--card-border);
  flex-shrink: 0;
}

.widget-card__title {
  font-size: var(--fs-sm, 12px);
  font-weight: 500;
  color: var(--text, #e9ebf1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.widget-card__close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: none;
  border-radius: var(--radius-sm, 3px);
  background: transparent;
  color: var(--muted, #9ba2b2);
  cursor: pointer;
  flex-shrink: 0;
}

.widget-card__close:hover {
  background: var(--hover, rgba(255, 255, 255, 0.1));
  color: var(--text, #e9ebf1);
}

/* ============= 内容体 ============= */
.widget-card__body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 10px;
}

/* ============= 拖拽指示器 ============= */
.widget-card__drag-hint {
  position: absolute;
  top: 4px;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  align-items: center;
  justify-content: center;
  pointer-events: none;
  opacity: 0;
  transition: opacity 0.2s ease;
}

.widget-card--stack-top .widget-card__drag-hint {
  opacity: 0.6;
}
</style>
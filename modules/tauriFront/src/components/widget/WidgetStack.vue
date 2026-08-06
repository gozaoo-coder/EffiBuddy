<script setup lang="ts">
/**
 * WidgetStack —— 卡片堆叠容器组件（vue-bits Stack 交互模型）
 *
 * 核心交互（参照 vue-bits Stack）：
 * - 循环堆叠：拖拽 / 点击将顶层卡片「送回底部」（sendToBack），下一张成为顶层
 * - 拖拽：顶层卡片可拖，实时位移 + 3D 倾斜（rotateX/rotateY 跟随），
 *   超过灵敏度阈值 → 沿拖拽方向飞出 → 送回底部；否则 spring 回弹
 * - 点击：按 interactionMode 送回底部（vue-bits）/ 提到顶层（原版）
 * - autoplay 自动轮播 + pauseOnHover 悬停暂停
 * - spring 弹簧动画（stiffness / damping 可调）
 * - 尺寸枚举：卡片支持 WidgetSize 大小映射
 *
 * 动画全流程管线：
 * 1. 开始（push）：新卡片加入，全部卡片 spring 排列到新层
 * 2. 中断（pointerdown）：拖拽暂停排列动画
 * 3. 拖拽中（pointermove）：实时更新 transform（层变换 + 位移 + 3D 倾斜）
 * 4. 返回（拖拽未超阈值）：spring 回弹到层位置
 * 5. 循环（拖拽超阈值）：飞出 → sendToBack → 全卡 spring 重排
 * 6. 离开（dismiss）：轻扫飞出移除 / 循环送回底部
 */
import { ref, computed, onBeforeUnmount } from 'vue'
import { useWidgetStack } from '../../composables/useWidgetStack'
import { STACK_PRESETS, WIDGET_SIZE_PX, WidgetSize } from '../../types/widget'
import type { StackEvent, StackLayer, StackInteractionOptions } from '../../types/widget'
import WidgetCard from './WidgetCard.vue'

export interface StackItem {
  id: string
  title?: string
  /** 卡片尺寸（堆叠模式下的物理大小） */
  size?: WidgetSize
  [key: string]: unknown
}

const props = withDefaults(
  defineProps<{
    /** 预设堆叠样式（'bits' 为 vue-bits 风格：按层索引旋转/缩放） */
    preset?: keyof typeof STACK_PRESETS | string
    /** 自定义层配置 */
    layers?: StackLayer[]
    /** 最大可见层数 */
    maxVisible?: number
    /** 堆叠容器宽度 */
    width?: string
    /** 堆叠容器高度 */
    height?: string
    /** 根据顶层卡片尺寸自动调整容器大小（WIDGET_SIZE_PX 映射） */
    autoSize?: boolean
    /** 排列动画时长（ms，非 spring 模式下生效） */
    duration?: number
    /** 拖拽灵敏度阈值（px）：拖拽偏移超过该值则送回底部 */
    sensitivity?: number
    /** 每张卡片随机旋转角度（±deg） */
    randomRotation?: boolean
    /** 点击卡片时将其送回底部（vue-bits 行为） */
    sendToBackOnClick?: boolean
    /** 交互模式：send-to-back 送回底部（vue-bits）/ bring-to-front 提到顶层（原版） */
    interactionMode?: 'send-to-back' | 'bring-to-front'
    /** 自动轮播：定时将顶层卡片送回底部 */
    autoplay?: boolean
    /** 自动轮播间隔（ms） */
    autoplayDelay?: number
    /** 悬停时暂停自动轮播 */
    pauseOnHover?: boolean
    /** 弹簧动画刚度（越大越硬） */
    stiffness?: number
    /** 弹簧动画阻尼（越大衰减越快） */
    damping?: number
    /** 3D 倾斜强度（deg，拖拽时卡片跟随旋转） */
    tiltAmount?: number
    /** 拖拽弹性系数（0=无弹性，0.6=强弹性，越界时生效） */
    dragElastic?: number
    /** 是否只允许顶层卡片拖拽 */
    topOnlyDraggable?: boolean
    /** 是否循环堆叠（送回底部后不移除卡片） */
    loop?: boolean
  }>(),
  {
    preset: 'bits',
    width: '320px',
    height: '420px',
    autoSize: false,
    duration: 300,
  },
)

const emit = defineEmits<{
  (e: 'card-selected', id: string): void
  (e: 'card-dismissed', id: string): void
  (e: 'card-send-back', id: string): void
  (e: 'card-click', id: string): void
  (e: 'card-drag-start', id: string): void
  (e: 'card-drag-end', id: string): void
  (e: 'stack-event', event: StackEvent): void
}>()

const stackRef = ref<HTMLElement | null>(null)

// autoSize：容器尺寸跟随顶层卡片尺寸（WIDGET_SIZE_PX 映射，带过渡动画）
const containerStyle = computed(() => {
  if (!props.autoSize) return { width: props.width, height: props.height }
  const topItem = stack.items.value[stack.items.value.length - 1]
  const size = topItem?.size ?? WidgetSize.MEDIUM
  const px = WIDGET_SIZE_PX[size]
  return { width: `${px.width}px`, height: `${px.height}px` }
})

// 解析 width/height 为 px（层偏移 % → px 转换需要）
function parsePx(v: string): number {
  const n = parseFloat(v)
  return Number.isNaN(n) ? 320 : n
}
const cardW = computed(() => parsePx(props.width))
const cardH = computed(() => parsePx(props.height))

// 组装 vue-bits 交互配置（响应式透传）
const interaction = computed<StackInteractionOptions>(() => ({
  sensitivity: props.sensitivity,
  randomRotation: props.randomRotation,
  sendToBackOnClick: props.sendToBackOnClick,
  interactionMode: props.interactionMode,
  autoplay: props.autoplay,
  autoplayDelay: props.autoplayDelay,
  pauseOnHover: props.pauseOnHover,
  stiffness: props.stiffness,
  damping: props.damping,
  tiltAmount: props.tiltAmount,
  dragElastic: props.dragElastic,
  topOnlyDraggable: props.topOnlyDraggable,
  loop: props.loop,
}))

const stack = useWidgetStack<StackItem>(stackRef, {
  preset: props.preset,
  layers: props.layers,
  maxVisible: props.maxVisible,
  duration: props.duration,
  cardWidth: cardW.value,
  cardHeight: cardH.value,
  interaction: interaction,
  onEvent: (evt) => {
    emit('stack-event', evt)
    if (evt.type === 'card-selected') emit('card-selected', evt.cardId)
    if (evt.type === 'card-dismissed') emit('card-dismissed', evt.cardId)
    if (evt.type === 'card-send-back') emit('card-send-back', evt.cardId)
    if (evt.type === 'card-drag-start') emit('card-drag-start', evt.cardId)
    if (evt.type === 'card-drag-end') emit('card-drag-end', evt.cardId)
  },
})

const { stacked, push, remove, bringToFront, sendToBack, dismissTop, clear, getIndex, getCardStyle } = stack

const isTop = (id: string) => getIndex(id) === stack.items.value.length - 1

// ===================== 拖拽（vue-bits 模型，统一由 Stack 管理） =====================
let dragPointerId = 0
let dragStartX = 0
let dragStartY = 0
let dragLastX = 0
let dragLastY = 0
let dragLastTime = 0
let dragActiveId = ''
let dragEndDistance = 0
let dragVelocityX = 0
let dragVelocityY = 0

function onPointerDown(e: PointerEvent, id: string) {
  if (e.button !== 0) return
  if (stack.isDragging.value) return
  const el = e.currentTarget as HTMLElement
  if (!el) return

  dragActiveId = id
  dragPointerId = e.pointerId
  dragStartX = e.clientX
  dragStartY = e.clientY
  dragLastX = dragStartX
  dragLastY = dragStartY
  dragLastTime = performance.now()
  dragEndDistance = 0
  dragVelocityX = 0
  dragVelocityY = 0

  el.setPointerCapture?.(e.pointerId)
  el.style.touchAction = 'none'
  el.style.userSelect = 'none'

  stack.dragStart(id)
  document.addEventListener('pointermove', onPointerMove)
  document.addEventListener('pointerup', onPointerUp)
  document.addEventListener('pointercancel', onPointerCancel)
}

function onPointerMove(e: PointerEvent) {
  if (!dragActiveId || e.pointerId !== dragPointerId) return
  const now = performance.now()
  const dt = now - dragLastTime
  if (dt > 0) {
    dragVelocityX = (e.clientX - dragLastX) / dt
    dragVelocityY = (e.clientY - dragLastY) / dt
  }
  dragLastX = e.clientX
  dragLastY = e.clientY
  dragLastTime = now
  stack.dragMove(dragActiveId, e.clientX - dragStartX, e.clientY - dragStartY)
}

function onPointerUp(e: PointerEvent) {
  if (!dragActiveId || e.pointerId !== dragPointerId) return
  dragEndDistance = Math.hypot(e.clientX - dragStartX, e.clientY - dragStartY)
  cleanupDragListeners()
  const id = dragActiveId
  dragActiveId = ''
  stack.dragEnd(id, dragVelocityX, dragVelocityY)
}

function onPointerCancel() {
  if (!dragActiveId) return
  cleanupDragListeners()
  const id = dragActiveId
  dragActiveId = ''
  stack.cancelDrag(id)
}

function cleanupDragListeners() {
  document.removeEventListener('pointermove', onPointerMove)
  document.removeEventListener('pointerup', onPointerUp)
  document.removeEventListener('pointercancel', onPointerCancel)
  const el = stackRef.value?.querySelector(`[data-stack-id="${dragActiveId}"]`) as HTMLElement | null
  if (el) {
    el.style.touchAction = ''
    el.style.userSelect = ''
  }
}

// ===================== 点击（堆叠交互） =====================
function onCardClick(id: string) {
  if (stack.isDragging.value) return
  // 拖拽结束后紧跟的 click 忽略（位移大于 8px 视为拖拽而非点击）
  if (dragEndDistance > 8) {
    dragEndDistance = 0
    return
  }
  const idx = getIndex(id)
  const topIdx = stack.items.value.length - 1
  const mode = interaction.value.interactionMode ?? 'send-to-back'

  if (mode === 'send-to-back') {
    // vue-bits 模型：点击卡片送回底部（循环堆叠）
    const shouldSendBack =
      interaction.value.sendToBackOnClick ? idx > 0 : idx > 0 && idx < topIdx
    if (shouldSendBack) sendToBack(id)
  } else {
    // 原版模型：点击底层卡片提到顶层
    if (idx < topIdx) bringToFront(id)
  }
  emit('card-click', id)
}

// 顶层卡片轻扫移除（外部调用，loop=true 循环送回底部）
function onCardFlick(id: string, direction: 'left' | 'right' | 'up' | 'down') {
  if (isTop(id)) dismissTop(direction)
}

// 悬停暂停自动轮播
function onMouseEnter() {
  if (interaction.value.pauseOnHover) stack.stopAutoplay()
}
function onMouseLeave() {
  if (interaction.value.pauseOnHover) stack.startAutoplay()
}

onBeforeUnmount(() => {
  cleanupDragListeners()
  stack.stopAutoplay()
})

defineExpose({
  push,
  remove,
  bringToFront,
  sendToBack,
  dismissTop,
  clear,
  getIndex,
  items: stack.items,
  startAutoplay: stack.startAutoplay,
  stopAutoplay: stack.stopAutoplay,
})
</script>

<template>
  <div
    ref="stackRef"
    class="widget-stack"
    :style="containerStyle"
    @mouseenter="onMouseEnter"
    @mouseleave="onMouseLeave"
  >
    <template v-for="s in stacked" :key="s.item.id">
      <WidgetCard
        :card-id="s.item.id"
        :title="s.item.title || ''"
        :size="s.item.size ?? WidgetSize.MEDIUM"
        :stack-z-index="s.zIndex"
        :stacked="true"
        :is-stack-top="isTop(s.item.id)"
        class="widget-stack__card"
        :data-stack-id="s.item.id"
        :style="getCardStyle(s.item.id)"
        @pointerdown="(e: PointerEvent) => isTop(s.item.id) && onPointerDown(e, s.item.id)"
        @click="onCardClick(s.item.id)"
        @flick="onCardFlick"
      >
        <slot name="card" :item="s.item" :index="getIndex(s.item.id)" :is-top="isTop(s.item.id)">
          <!-- 默认内容：显示 item 的 JSON 预览 -->
          <div class="widget-stack__default-content">
            <pre>{{ JSON.stringify(s.item, null, 2) }}</pre>
          </div>
        </slot>
      </WidgetCard>
    </template>

    <!-- 空态 -->
    <div v-if="stacked.length === 0" class="widget-stack__empty">
      <slot name="empty">
        <span class="widget-stack__empty-text">无卡片</span>
      </slot>
    </div>
  </div>
</template>

<style scoped>
.widget-stack {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  perspective: 600px;
  transition: width 0.4s cubic-bezier(0.34, 1.3, 0.64, 1), height 0.4s cubic-bezier(0.34, 1.3, 0.64, 1);
}

.widget-stack__card {
  position: absolute;
  width: 90%;
  height: 90%;
  max-width: 100%;
  max-height: 100%;
  will-change: transform, opacity;
  transition: none;
  transform-origin: 50% 100%;
}

/* 顶层卡片拖拽手感 */
.widget-stack__card:last-of-type {
  cursor: grab;
}

.widget-stack__empty {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
}

.widget-stack__empty-text {
  color: var(--muted, #9ba2b2);
  font-size: var(--fs-sm, 12px);
}

.widget-stack__default-content {
  font-size: 11px;
  color: var(--muted, #9ba2b2);
  overflow: auto;
  height: 100%;
}

.widget-stack__default-content pre {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-all;
}
</style>

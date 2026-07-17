<script setup lang="ts">
import { ref, computed, watch, onUnmounted } from 'vue'
import { useAnimeTransition } from '../../composables/useAnimeTransition'
import Icon from '../Icon.vue'

export type PopupPlacement = 'top' | 'bottom' | 'left' | 'right'
export type PopupAlign = 'start' | 'center' | 'end'
export type PopupTrigger = 'hover' | 'click' | 'focus'

export interface PopupButton {
  text: string
  onClick?: () => void
}

const props = withDefaults(
  defineProps<{
    /** 是否显示（v-model） */
    visible?: boolean
    /** 触发方式 */
    trigger?: PopupTrigger
    /** 弹出位置，默认 top */
    placement?: PopupPlacement
    /** 对齐方式，默认 center */
    align?: PopupAlign
    /** 标题（可选） */
    title?: string
    /** 主文本 */
    message: string
    /** 是否显示关闭按钮 */
    showClose?: boolean
    /** 操作按钮（可选） */
    button?: PopupButton
    /** 图标（可选，传 emoji 或字符） */
    icon?: string
    /** 定时关闭 ms，0 表示不自动关闭，默认 0（带 button 时强制 0） */
    duration?: number
    /** 最大宽度 px，默认 400 */
    maxWidth?: number
  }>(),
  {
    visible: undefined,
    trigger: 'hover',
    placement: 'top',
    align: 'center',
    showClose: false,
    duration: 0,
    maxWidth: 400,
  },
)

const emit = defineEmits<{
  (e: 'update:visible', v: boolean): void
  (e: 'close'): void
  (e: 'button-click'): void
}>()

// 内部 visible 状态：支持 v-model 与非受控两种模式
const innerVisible = ref(props.visible ?? false)
watch(
  () => props.visible,
  (v) => {
    if (v !== undefined) innerVisible.value = v
  },
)

function setVisible(v: boolean) {
  innerVisible.value = v
  emit('update:visible', v)
  if (!v) emit('close')
}

// trigger 元素引用 + popup 浮层引用
const triggerEl = ref<HTMLElement | null>(null)
const popupEl = ref<HTMLElement | null>(null)
// 浮层定位样式
const popupStyle = ref<Record<string, string>>({})
const arrowStyle = ref<Record<string, string>>({})

// 触发事件
function onTriggerEnter() {
  if (props.trigger === 'hover') setVisible(true)
}
function onTriggerLeave() {
  if (props.trigger === 'hover' && !props.button && !props.showClose) {
    // 带操作/关闭按钮时不自动 hover 关闭
    scheduleHide()
  }
}
function onTriggerClick(e: MouseEvent) {
  e.stopPropagation()
  if (props.trigger === 'click') {
    setVisible(!innerVisible.value)
  }
}
function onTriggerFocus() {
  if (props.trigger === 'focus') setVisible(true)
}
function onTriggerBlur() {
  if (props.trigger === 'focus' && !props.button && !props.showClose) {
    setVisible(false)
  }
}

// 浮层 hover 时取消隐藏
let hideTimer: number | null = null
function scheduleHide() {
  if (hideTimer) window.clearTimeout(hideTimer)
  hideTimer = window.setTimeout(() => setVisible(false), 100)
}
function cancelHide() {
  if (hideTimer) {
    window.clearTimeout(hideTimer)
    hideTimer = null
  }
}

function onPopupEnter() {
  cancelHide()
}
function onPopupLeave() {
  if (props.trigger === 'hover' && !props.button && !props.showClose) {
    scheduleHide()
  }
}

// 计算定位（同步：调用方需确保 popupEl 已挂载）
// 移除原 async + await nextTick()，因为 beforeEnter / scroll 回调触发时元素已在 DOM 中。
function updatePosition() {
  if (!innerVisible.value || !triggerEl.value || !popupEl.value) return
  const trig = triggerEl.value.getBoundingClientRect()
  const pop = popupEl.value.getBoundingClientRect()
  const margin = 8
  const arrowSize = 8
  const vw = window.innerWidth
  const vh = window.innerHeight

  const style: Record<string, string> = {}
  const arrow: Record<string, string> = {}

  let top = 0
  let left = 0

  if (props.placement === 'top' || props.placement === 'bottom') {
    // 水平对齐
    if (props.align === 'start') {
      left = trig.left
    } else if (props.align === 'end') {
      left = trig.right - pop.width
    } else {
      left = trig.left + trig.width / 2 - pop.width / 2
    }
    // 垂直位置
    if (props.placement === 'top') {
      top = trig.top - pop.height - margin
    } else {
      top = trig.bottom + margin
    }
    // 箭头水平位置（跟随 trigger 中心）
    const arrowLeft = trig.left + trig.width / 2 - left
    arrow.left = `${Math.max(arrowSize + 4, Math.min(pop.width - arrowSize - 4, arrowLeft))}px`
  } else {
    // left / right
    if (props.align === 'start') {
      top = trig.top
    } else if (props.align === 'end') {
      top = trig.bottom - pop.height
    } else {
      top = trig.top + trig.height / 2 - pop.height / 2
    }
    if (props.placement === 'left') {
      left = trig.left - pop.width - margin
    } else {
      left = trig.right + margin
    }
    const arrowTop = trig.top + trig.height / 2 - top
    arrow.top = `${Math.max(arrowSize + 4, Math.min(pop.height - arrowSize - 4, arrowTop))}px`
  }

  // 边界适配：距屏幕边缘最小 6px
  if (left < 6) left = 6
  if (left + pop.width > vw - 6) left = vw - pop.width - 6
  if (top < 6) top = 6
  if (top + pop.height > vh - 6) top = vh - pop.height - 6

  style.top = `${top + window.scrollY}px`
  style.left = `${left + window.scrollX}px`
  popupStyle.value = style
  arrowStyle.value = arrow
}

// anime.js v4 Transition：替换原 <Transition name="popup"> 的 CSS 过渡
// beforeEnter 在动画开始前同步计算定位，避免动画中的 scale 影响 getBoundingClientRect
// enter/leave 用 transform + opacity，动画完成后由 useAnimeTransition 清理内联 transform
const { onBeforeEnter, onEnter, onBeforeLeave, onLeave } = useAnimeTransition({
  beforeEnter: (el) => {
    const htmlEl = el as HTMLElement
    // 预置透明，避免插入到 animate 首帧之间的可见闪烁
    htmlEl.style.opacity = '0'
    // 在动画开始前确定定位（此时 transform 尚未被 animate 写入）
    updatePosition()
  },
  enter: {
    opacity: [0, 1],
    transform: ['scale(.92)', 'scale(1)'],
    duration: 180,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    transform: ['scale(1)', 'scale(.92)'],
    duration: 150,
    ease: 'out(3)',
  },
})

// 窗口滚动/缩放时重新定位
function onScroll() {
  if (innerVisible.value) updatePosition()
}
if (typeof window !== 'undefined') {
  window.addEventListener('scroll', onScroll, true)
  window.addEventListener('resize', onScroll)
}
onUnmounted(() => {
  if (typeof window !== 'undefined') {
    window.removeEventListener('scroll', onScroll, true)
    window.removeEventListener('resize', onScroll)
  }
})

// 定时关闭
let durationTimer: number | null = null
watch(innerVisible, (v) => {
  if (durationTimer) {
    window.clearTimeout(durationTimer)
    durationTimer = null
  }
  const d = props.duration ?? 0
  // 带 button 时不自动关闭
  if (v && d > 0 && !props.button) {
    durationTimer = window.setTimeout(() => setVisible(false), d)
  }
})

// 点击外部关闭（click 模式或带 close）
function onDocumentClick(e: MouseEvent) {
  if (!innerVisible.value) return
  const target = e.target as Node
  if (triggerEl.value?.contains(target)) return
  if (popupEl.value?.contains(target)) return
  // 带操作按钮的常驻 popup 不因外部点击关闭
  if (props.button) return
  setVisible(false)
}
if (typeof document !== 'undefined') {
  document.addEventListener('click', onDocumentClick)
}
onUnmounted(() => {
  if (typeof document !== 'undefined') {
    document.removeEventListener('click', onDocumentClick)
  }
})

function onButtonClick() {
  emit('button-click')
  props.button?.onClick?.()
  setVisible(false)
}

function onClose() {
  setVisible(false)
}

const hasHeader = computed(() => props.title || props.icon || props.showClose)
</script>

<template>
  <span
    ref="triggerEl"
    class="popup-trigger"
    @mouseenter="onTriggerEnter"
    @mouseleave="onTriggerLeave"
    @click="onTriggerClick"
    @focus="onTriggerFocus"
    @blur="onTriggerBlur"
  >
    <slot />
  </span>

  <Teleport to="body">
    <Transition :css="false" @before-enter="onBeforeEnter" @enter="onEnter" @before-leave="onBeforeLeave" @leave="onLeave">
      <div
        v-if="innerVisible"
        ref="popupEl"
        class="popup"
        :class="`popup--${placement}`"
        :style="{ ...popupStyle, maxWidth: `${maxWidth}px` }"
        @mouseenter="onPopupEnter"
        @mouseleave="onPopupLeave"
      >
        <!-- 箭头 -->
        <span class="popup-arrow" :class="`popup-arrow--${placement}`" :style="arrowStyle"></span>

        <!-- 图标 -->
        <div v-if="icon" class="popup-icon"><Icon :name="icon" :size="20" :fallback="icon" /></div>

        <div class="popup-body">
          <!-- 头部：title + close -->
          <div v-if="hasHeader" class="popup-head">
            <span v-if="title" class="popup-title">{{ title }}</span>
            <button
              v-if="showClose"
              class="popup-close"
              aria-label="关闭"
              @click="onClose"
            ><Icon name="close" :size="18" /></button>
          </div>
          <!-- 消息 -->
          <div class="popup-message">{{ message }}</div>
          <!-- 操作按钮 -->
          <div v-if="button" class="popup-actions">
            <button class="popup-btn" @click="onButtonClick">{{ button.text }}</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

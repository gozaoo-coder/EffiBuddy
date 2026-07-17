<script setup lang="ts">
/**
 * BindSheet 半模态面板组件
 * 从底部/侧边滑入的面板
 * 参考 HarmonyOS NEXT 设计规范
 *
 * 特性：
 * - 支持 bottom / right / top / left 四个方向滑入
 * - 顶部标题栏 + 关闭按钮
 * - 内容区可滚动
 * - 半模态：side=bottom 时居中显示，两侧留空，最大宽度 480px
 * - ESC 关闭、点击遮罩关闭（可配置）
 * - body 滚动锁
 * - 进出动画使用 anime.js v4（通过 useAnimeTransition），按 side 选择滑动方向
 */
import { ref, watch, onUnmounted, computed } from 'vue'
import { animate } from 'animejs'
import { useAnimeTransition } from '../../composables/useAnimeTransition'

const props = withDefaults(
  defineProps<{
    /** 是否显示（v-model） */
    visible?: boolean
    /** 标题 */
    title?: string
    /** 滑入方向，默认 bottom */
    side?: 'bottom' | 'right' | 'top' | 'left'
    /** side=right/left 时的宽度，默认 480px */
    width?: string
    /** side=bottom/top 时的高度，默认 50vh */
    height?: string
    /** 显示关闭按钮，默认 true */
    showClose?: boolean
    /** 点击遮罩关闭，默认 true */
    closeOnClickOverlay?: boolean
    /** ESC 关闭，默认 true */
    closeOnEsc?: boolean
    /** 面板圆角，默认 var(--radius-lg) */
    radius?: string
  }>(),
  {
    visible: undefined,
    title: '',
    side: 'bottom',
    width: '480px',
    height: '50vh',
    showClose: true,
    closeOnClickOverlay: true,
    closeOnEsc: true,
    radius: 'var(--radius-lg)',
  },
)

const emit = defineEmits<{
  (e: 'update:visible', v: boolean): void
  (e: 'close'): void
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

// 是否水平方向（left/right）
const isHorizontal = computed(() => props.side === 'left' || props.side === 'right')

// 面板尺寸样式
const panelSizeStyle = computed(() => {
  if (isHorizontal.value) {
    return { width: props.width }
  }
  return { height: props.height }
})

// body 滚动锁
function lockBodyScroll() {
  if (typeof document !== 'undefined') {
    document.body.style.overflow = 'hidden'
  }
}
function unlockBodyScroll() {
  if (typeof document !== 'undefined') {
    document.body.style.overflow = ''
  }
}

watch(innerVisible, (v) => {
  if (v) lockBodyScroll()
  else unlockBodyScroll()
})

onUnmounted(() => {
  unlockBodyScroll()
})

// ESC 键监听
function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape' && innerVisible.value && props.closeOnEsc) {
    e.stopPropagation()
    onClose()
  }
}
if (typeof document !== 'undefined') {
  document.addEventListener('keydown', onKeydown)
}
onUnmounted(() => {
  if (typeof document !== 'undefined') {
    document.removeEventListener('keydown', onKeydown)
  }
})

// 遮罩点击
function onOverlayClick() {
  if (props.closeOnClickOverlay) {
    onClose()
  }
}

// 阻止面板内部点击冒泡到遮罩
function onPanelClick(e: MouseEvent) {
  e.stopPropagation()
}

// 关闭
function onClose() {
  setVisible(false)
}

// 根据 side 计算面板滑入/滑出的 transform
// - bottom: translateY(100%) → 0
// - top:    translateY(-100%) → 0
// - left:   translateX(-100%) → 0
// - right:  translateX(100%) → 0
function panelAxis() {
  return props.side === 'left' || props.side === 'right' ? 'translateX' : 'translateY'
}
function panelFromTransform() {
  const axis = panelAxis()
  const sign = props.side === 'top' || props.side === 'left' ? '-' : ''
  return `${axis}(${sign}100%)`
}
function panelToTransform() {
  return `${panelAxis()}(0px)`
}

// 进出动画：遮罩 fade + 面板按 side 方向滑动
// 遮罩与面板分别动画化，面板动画完成后清理内联 transform/opacity 避免影响布局
const { onEnter, onLeave } = useAnimeTransition({
  enter: (el, done) => {
    const overlay = el.querySelector('.bindsheet-overlay')
    const panel = el.querySelector('.bindsheet-panel') as HTMLElement | null
    if (overlay) {
      animate(overlay, {
        opacity: [0, 1],
        duration: 250,
        ease: 'outQuad',
      })
    }
    if (panel) {
      animate(panel, {
        transform: [panelFromTransform(), panelToTransform()],
        duration: 300,
        ease: 'out(3)',
        onComplete: () => {
          panel.style.transform = ''
          if (overlay) (overlay as HTMLElement).style.opacity = ''
          done()
        },
      })
    } else {
      done()
    }
  },
  leave: (el, done) => {
    const overlay = el.querySelector('.bindsheet-overlay')
    const panel = el.querySelector('.bindsheet-panel') as HTMLElement | null
    if (overlay) {
      animate(overlay, {
        opacity: [1, 0],
        duration: 220,
        ease: 'inOut(2)',
      })
    }
    if (panel) {
      animate(panel, {
        transform: [panelToTransform(), panelFromTransform()],
        duration: 240,
        ease: 'inOut(2)',
        onComplete: () => {
          done()
        },
      })
    } else {
      done()
    }
  },
})
</script>

<template>
  <Teleport to="body">
    <Transition :css="false" @enter="onEnter" @leave="onLeave" appear>
      <div
        v-if="innerVisible"
        class="bindsheet-root"
        :class="[`bindsheet-root--${side}`]"
      >
        <!-- 遮罩 -->
        <div class="bindsheet-overlay" @click="onOverlayClick"></div>

        <!-- 面板 -->
        <div
          class="bindsheet-panel"
          :class="[`bindsheet-panel--${side}`]"
          :style="{
            ...panelSizeStyle,
            '--bindsheet-radius': radius,
          }"
          @click="onPanelClick"
        >
          <!-- 顶部标题栏 -->
          <div v-if="title || showClose" class="bindsheet-header">
            <span v-if="title" class="bindsheet-title">{{ title }}</span>
            <span v-else class="bindsheet-title-placeholder"></span>
            <button
              v-if="showClose"
              type="button"
              class="bindsheet-close"
              aria-label="关闭"
              @click="onClose"
            >×</button>
          </div>

          <!-- 内容区：可滚动 -->
          <div class="bindsheet-content">
            <slot />
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

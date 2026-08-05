<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick, type Ref } from 'vue'
import { animate } from 'animejs'

// ── Types ──────────────────────────────────────────────
type HeightLevel = 'Large' | 'Medium' | 'Low' | 'Free'
type LayoutMode = 'bottom' | 'center' | 'popup'
type Platform = 'auto' | 'phone' | 'tablet' | 'desktop'

interface PanelPage {
  title: string
  subtitle: string
}

// ── Props ──────────────────────────────────────────────
const props = withDefaults(
  defineProps<{
    modelValue: boolean
    modal?: boolean
    defaultHeight?: HeightLevel
    title?: string
    subtitle?: string
    showClose?: boolean
    showHandle?: boolean
    platform?: Platform
    customHeight?: number | string
  }>(),
  {
    modelValue: false,
    modal: true,
    defaultHeight: 'Medium',
    title: '',
    subtitle: '',
    showClose: true,
    showHandle: true,
    platform: 'auto',
  },
)

const emit = defineEmits<{
  'update:modelValue': [value: boolean]
  close: []
}>()

// ── Template refs ──────────────────────────────────────
const panelRef: Ref<HTMLElement | null> = ref(null)
const contentRef: Ref<HTMLElement | null> = ref(null)
const overlayRef: Ref<HTMLElement | null> = ref(null)

// ── Window state ───────────────────────────────────────
const windowWidth = ref(window.innerWidth)
const windowHeight = ref(window.innerHeight)

function onResize() {
  windowWidth.value = window.innerWidth
  windowHeight.value = window.innerHeight
}

onMounted(() => window.addEventListener('resize', onResize))
onUnmounted(() => window.removeEventListener('resize', onResize))

// ── Device / layout detection ──────────────────────────
const layoutMode = computed<LayoutMode>(() => {
  if (props.platform === 'phone') return 'bottom'
  if (props.platform === 'tablet') return 'center'
  if (props.platform === 'desktop') return 'popup'
  // auto
  const w = windowWidth.value
  if (w < 600) return 'bottom'
  if (w < 1024) return 'center'
  return 'popup'
})

const isLandscape = computed(() => windowWidth.value > windowHeight.value && layoutMode.value === 'bottom')

// ── Height / snap logic ────────────────────────────────
const currentSnapLevel = ref<HeightLevel>(
  props.defaultHeight === 'Free' ? 'Medium' : props.defaultHeight,
)

const snapHeights = computed(() => {
  const h = windowHeight.value
  return {
    Large: Math.max(h - 8, 200),
    Medium: Math.round(h * 0.6),
    Low: Math.round(h * 0.3),
  }
})

const freeHeight = ref<number>(0)

watch(
  () => props.customHeight,
  (v) => {
    if (v !== undefined) {
      const n = typeof v === 'number' ? v : parseInt(String(v), 10)
      if (!isNaN(n)) freeHeight.value = n
    }
  },
  { immediate: true },
)

const currentHeightPx = computed(() => {
  if (props.defaultHeight === 'Free') {
    if (freeHeight.value > 0) return freeHeight.value
    // auto: fit content up to 90% window
    if (contentRef.value) {
      const contentH = contentRef.value.scrollHeight
      const headerH = 48 // header height
      const handleH = layoutMode.value === 'bottom' && props.showHandle ? 20 : 0
      const total = contentH + headerH + handleH + 32 // 32px padding
      const maxH = Math.round(windowHeight.value * 0.9)
      return Math.min(total, maxH)
    }
    return snapHeights.value.Medium
  }
  return snapHeights.value[currentSnapLevel.value as keyof typeof snapHeights.value]
})

// Determine which levels are available (ordered small → large)
const availableLevels = computed<HeightLevel[]>(() => {
  const levels: HeightLevel[] = ['Low', 'Medium', 'Large']
  if (props.defaultHeight === 'Free') return ['Low', 'Medium', 'Large']
  return levels
})

const currentLevelIndex = computed(() => {
  return availableLevels.value.indexOf(currentSnapLevel.value as HeightLevel)
})

// ── Visibility & animation ─────────────────────────────
const visible = ref(props.modelValue)
const animating = ref(false)

watch(
  () => props.modelValue,
  async (v) => {
    if (v) {
      visible.value = true
      currentSnapLevel.value =
        props.defaultHeight === 'Free' ? 'Medium' : (props.defaultHeight as HeightLevel)
      await nextTick()
      await animateEnter()
    } else {
      await animateExit()
    }
  },
)

async function animateEnter(): Promise<void> {
  animating.value = true
  if (overlayRef.value) {
    animate(overlayRef.value, {
      opacity: [0, 1],
      duration: 280,
      easing: 'easeOutQuad',
    })
  }
  if (panelRef.value) {
    if (layoutMode.value === 'bottom') {
      const h = currentHeightPx.value
      panelRef.value.style.height = `${h}px`
      await animate(panelRef.value, {
        translateY: [h, 0],
        duration: 320,
        easing: 'easeOutCubic',
      })
    } else if (layoutMode.value === 'center') {
      await animate(panelRef.value, {
        opacity: [0, 1],
        scale: [0.92, 1],
        duration: 300,
        easing: 'easeOutCubic',
      })
    } else {
      await animate(panelRef.value, {
        opacity: [0, 1],
        translateX: [40, 0],
        duration: 280,
        easing: 'easeOutCubic',
      })
    }
  }
  animating.value = false
}

async function animateExit(): Promise<void> {
  animating.value = true
  if (overlayRef.value) {
    animate(overlayRef.value, {
      opacity: [1, 0],
      duration: 240,
      easing: 'easeInQuad',
    })
  }
  if (panelRef.value) {
    if (layoutMode.value === 'bottom') {
      await animate(panelRef.value, {
        translateY: [0, currentHeightPx.value],
        duration: 260,
        easing: 'easeInCubic',
      })
    } else if (layoutMode.value === 'center') {
      await animate(panelRef.value, {
        opacity: [1, 0],
        scale: [1, 0.92],
        duration: 240,
        easing: 'easeInCubic',
      })
    } else {
      await animate(panelRef.value, {
        opacity: [1, 0],
        translateX: [0, 40],
        duration: 240,
        easing: 'easeInCubic',
      })
    }
  }
  visible.value = false
  animating.value = false
}

// ── Close ──────────────────────────────────────────────
function close() {
  emit('update:modelValue', false)
  emit('close')
}

function onOverlayClick() {
  if (props.modal) {
    close()
  }
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape' && visible.value && props.modal) {
    close()
  }
}

onMounted(() => window.addEventListener('keydown', onKeydown))
onUnmounted(() => window.removeEventListener('keydown', onKeydown))

// ── Drag handling (bottom sheet) ───────────────────────
const isDragging = ref(false)
const dragStartY = ref(0)
const dragStartHeight = ref(0)
const dragDeltaY = ref(0)
const dragVelocity = ref(0)
const lastDragY = ref(0)
const lastDragTime = ref(0)
const dragTarget = ref<'handle' | 'content' | null>(null)

function onPointerDown(e: PointerEvent) {
  if (layoutMode.value !== 'bottom' || animating.value) return

  const target = e.target as HTMLElement
  const isHandleArea = target.closest('.bindsheet-handle') !== null
  const isHeaderArea = target.closest('.bindsheet-header') !== null

  if (isHandleArea || isHeaderArea) {
    dragTarget.value = 'handle'
  } else if (target.closest('.bindsheet-content') !== null) {
    dragTarget.value = 'content'
  } else {
    return
  }

  isDragging.value = true
  dragStartY.value = e.clientY
  dragStartHeight.value = currentHeightPx.value
  dragDeltaY.value = 0
  lastDragY.value = e.clientY
  lastDragTime.value = Date.now()
  dragVelocity.value = 0

  if (panelRef.value) {
    panelRef.value.style.transition = 'none'
  }

  e.preventDefault()
}

function onPointerMove(e: PointerEvent) {
  if (!isDragging.value) return

  const now = Date.now()
  const dy = dragStartY.value - e.clientY // positive = moving up (expanding)
  dragDeltaY.value = dy

  // Calculate velocity (px/ms)
  const timeDelta = now - lastDragTime.value
  if (timeDelta > 0) {
    const yDelta = e.clientY - lastDragY.value
    dragVelocity.value = yDelta / timeDelta // positive = moving down (shrinking)
  }
  lastDragY.value = e.clientY
  lastDragTime.value = now

  if (dragTarget.value === 'handle') {
    // Direct height manipulation: drag up = taller, drag down = shorter
    let newHeight = dragStartHeight.value + dy
    // Clamp between Low and Large
    newHeight = Math.max(snapHeights.value.Low, Math.min(snapHeights.value.Large, newHeight))
    if (panelRef.value) {
      panelRef.value.style.height = `${newHeight}px`
    }
  } else if (dragTarget.value === 'content') {
    // Only intervene at scroll boundaries
    const el = contentRef.value
    if (!el) return
    const atTop = el.scrollTop <= 1

    if (atTop && dy < 0) {
      // Dragging down at top → shrink panel
      let newHeight = dragStartHeight.value + dy
      newHeight = Math.max(snapHeights.value.Low, Math.min(snapHeights.value.Large, newHeight))
      if (panelRef.value) {
        panelRef.value.style.height = `${newHeight}px`
      }
      e.preventDefault()
    } else if (atTop && dy > 0 && currentLevelIndex.value < availableLevels.value.length - 1) {
      // Dragging up at top → expand panel if available
      let newHeight = dragStartHeight.value + dy
      newHeight = Math.max(snapHeights.value.Low, Math.min(snapHeights.value.Large, newHeight))
      if (panelRef.value) {
        panelRef.value.style.height = `${newHeight}px`
      }
      e.preventDefault()
    }
    // Otherwise let content scroll naturally
  }
}

function onPointerUp(_e: PointerEvent) {
  if (!isDragging.value) return
  isDragging.value = false

  // Calculate final height
  const finalHeight = panelRef.value
    ? parseInt(panelRef.value.style.height || String(currentHeightPx.value), 10)
    : currentHeightPx.value

  // Determine snap target based on velocity and distance
  let targetLevel: HeightLevel

  // Absolute velocity threshold: > 0.5 px/ms is a "fast" swipe
  const fastSwipe = Math.abs(dragVelocity.value) > 0.5
  const significantDistance = Math.abs(dragDeltaY.value) > 80

  if (fastSwipe) {
    if (dragVelocity.value > 0) {
      // Fast swipe down → shrink
      const idx = currentLevelIndex.value
      if (dragVelocity.value > 1.2) {
        // Very fast → close or go to Low
        targetLevel = 'Low'
        // If already at Low and fast enough, close
        if (currentSnapLevel.value === 'Low') {
          close()
          return
        }
      } else if (idx > 0) {
        targetLevel = availableLevels.value[idx - 1]
      } else {
        // Already at smallest → close
        close()
        return
      }
    } else {
      // Fast swipe up → expand
      const idx = currentLevelIndex.value
      if (dragVelocity.value < -1.2 && idx < availableLevels.value.length - 1) {
        // Very fast up → go to Large
        targetLevel = 'Large'
      } else if (idx < availableLevels.value.length - 1) {
        targetLevel = availableLevels.value[idx + 1]
      } else {
        targetLevel = availableLevels.value[idx]
      }
    }
  } else if (significantDistance) {
    // Snap to nearest
    targetLevel = findNearestLevel(finalHeight)
  } else {
    // No significant movement → snap back to current
    targetLevel = currentSnapLevel.value as HeightLevel
  }

  // Handle close threshold: if dragged below Low, close
  if (finalHeight < snapHeights.value.Low * 0.6) {
    close()
    // Reset style
    if (panelRef.value) {
      panelRef.value.style.transition = ''
      panelRef.value.style.height = ''
    }
    return
  }

  currentSnapLevel.value = targetLevel
  snapToLevel(targetLevel)

  dragTarget.value = null
}

function findNearestLevel(height: number): HeightLevel {
  const levels = availableLevels.value
  const heights = levels.map((l) => snapHeights.value[l as keyof typeof snapHeights.value])
  let best = levels[0]
  let bestDist = Math.abs(height - heights[0])
  for (let i = 1; i < levels.length; i++) {
    const dist = Math.abs(height - heights[i])
    if (dist < bestDist) {
      bestDist = dist
      best = levels[i]
    }
  }
  return best
}

function snapToLevel(level: HeightLevel) {
  if (!panelRef.value) return
  const targetH = snapHeights.value[level as keyof typeof snapHeights.value]
  panelRef.value.style.transition = 'height 0.3s ease-out'
  panelRef.value.style.height = `${targetH}px`
  // Clear inline transition after it completes
  setTimeout(() => {
    if (panelRef.value && !isDragging.value) {
      panelRef.value.style.transition = ''
    }
  }, 320)
}

// ── Content scroll tracking ────────────────────────────
const contentScrollTop = ref(0)

function onContentScroll() {
  if (contentRef.value) {
    contentScrollTop.value = contentRef.value.scrollTop
  }
}

// ── Panel stack (multi-panel navigation) ───────────────
const panelStack = ref<PanelPage[]>([])

const activeTitle = computed(() => {
  if (panelStack.value.length > 0) {
    return panelStack.value[panelStack.value.length - 1].title
  }
  return props.title
})

const activeSubtitle = computed(() => {
  if (panelStack.value.length > 0) {
    return panelStack.value[panelStack.value.length - 1].subtitle
  }
  return props.subtitle
})

const hasStack = computed(() => panelStack.value.length > 0)

function pushPanel(title: string, subtitle = '') {
  panelStack.value.push({ title, subtitle })
}

function popPanel() {
  if (panelStack.value.length > 0) {
    panelStack.value.pop()
  }
}

// ── Programmatic height control ────────────────────────
function setHeight(level: HeightLevel) {
  if (props.defaultHeight === 'Free') {
    if (level === 'Free') return
    currentSnapLevel.value = level
    if (panelRef.value) {
      panelRef.value.style.transition = 'height 0.3s ease-out'
      panelRef.value.style.height = `${snapHeights.value[level]}px`
      setTimeout(() => {
        if (panelRef.value) panelRef.value.style.transition = ''
      }, 320)
    }
    return
  }
  const levels = ['Low', 'Medium', 'Large'] as HeightLevel[]
  if (levels.includes(level)) {
    currentSnapLevel.value = level
    snapToLevel(level)
  }
}

// ── Exposed API ────────────────────────────────────────
defineExpose({
  pushPanel,
  popPanel,
  setHeight,
  close,
  get layoutMode() {
    return layoutMode.value
  },
  get currentSnapLevel() {
    return currentSnapLevel.value
  },
  get panelStack() {
    return panelStack.value
  },
})

// ── Body scroll lock ───────────────────────────────────
function lockBodyScroll() {
  document.body.style.overflow = 'hidden'
}

function unlockBodyScroll() {
  document.body.style.overflow = ''
}

watch(visible, (v) => {
  if (v) lockBodyScroll()
  else unlockBodyScroll()
})

onUnmounted(() => unlockBodyScroll())
</script>

<template>
  <!-- Overlay -->
  <div
    v-if="visible && modal"
    ref="overlayRef"
    class="bindsheet-overlay"
    @click="onOverlayClick"
  />

  <!-- Panel -->
  <div
    v-if="visible"
    ref="panelRef"
    class="bindsheet-panel"
    :class="[
      `bindsheet-panel--${layoutMode}`,
      {
        'bindsheet-panel--landscape': isLandscape,
        'bindsheet-panel--dragging': isDragging,
        'bindsheet-panel--nonmodal': !modal,
      },
    ]"
    :style="{
      height: layoutMode === 'bottom' ? `${currentHeightPx}px` : undefined,
    }"
  >
    <!-- Drag handle (bottom sheet only) -->
    <div
      v-if="showHandle && layoutMode === 'bottom'"
      class="bindsheet-handle"
      @pointerdown="onPointerDown"
    >
      <div class="bindsheet-handle-bar" />
    </div>

    <!-- Header -->
    <div
      class="bindsheet-header"
      @pointerdown="onPointerDown"
    >
      <div class="bindsheet-header-left">
        <!-- Back button for panel stack -->
        <button
          v-if="hasStack"
          class="bindsheet-back-btn"
          @click.stop="popPanel"
        >
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <path
              d="M10 3L5 8l5 5"
              stroke="currentColor"
              stroke-width="1.5"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
        </button>

        <!-- Title area -->
        <div class="bindsheet-title-area">
          <slot name="title">
            <span v-if="activeTitle" class="bindsheet-title">{{ activeTitle }}</span>
            <span v-if="activeSubtitle" class="bindsheet-subtitle">{{ activeSubtitle }}</span>
          </slot>
        </div>
      </div>

      <!-- Right side: actions + close -->
      <div class="bindsheet-header-right">
        <slot name="actions" />
        <button
          v-if="showClose"
          class="bindsheet-close-btn"
          @click.stop="close"
          aria-label="关闭"
        >
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <path
              d="M4 4l8 8M12 4l-8 8"
              stroke="currentColor"
              stroke-width="1.5"
              stroke-linecap="round"
            />
          </svg>
        </button>
      </div>
    </div>

    <!-- Content -->
    <div
      ref="contentRef"
      class="bindsheet-content"
      @scroll="onContentScroll"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
    >
      <slot />
    </div>
  </div>
</template>

<style scoped>
/* ── Overlay ────────────────────────────────────────── */
.bindsheet-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  z-index: 2000;
  opacity: 0;
}

/* ── Panel base ─────────────────────────────────────── */
.bindsheet-panel {
  position: fixed;
  z-index: 2001;
  background: var(--bg-2, #12141c);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  box-shadow: 0 -4px 24px rgba(0, 0, 0, 0.4), 0 -1px 4px rgba(0, 0, 0, 0.15);
}

/* ── Bottom sheet (phone) ───────────────────────────── */
.bindsheet-panel--bottom {
  left: 0;
  right: 0;
  bottom: 0;
  border-radius: 7px 7px 0 0;
  will-change: height;
}

.bindsheet-panel--bottom.bindsheet-panel--landscape {
  left: 50%;
  right: auto;
  bottom: 0;
  transform: translateX(-50%);
  width: 480px;
  max-width: 100vw;
  border-radius: 7px 7px 0 0;
}

.bindsheet-panel--bottom.bindsheet-panel--nonmodal {
  box-shadow: 0 -2px 12px rgba(0, 0, 0, 0.15);
}

/* ── Center panel (tablet) ──────────────────────────── */
.bindsheet-panel--center {
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  width: 480px;
  max-width: calc(100vw - 32px);
  min-height: 320px;
  max-height: 90vh;
  border-radius: 7px;
}

/* ── Popup (desktop) ────────────────────────────────── */
.bindsheet-panel--popup {
  top: 50%;
  right: 24px;
  transform: translateY(-50%);
  width: 400px;
  max-width: calc(100vw - 48px);
  min-height: 200px;
  max-height: 90vh;
  border-radius: 7px;
}

/* ── Drag handle ────────────────────────────────────── */
.bindsheet-handle {
  display: flex;
  justify-content: center;
  align-items: center;
  padding: 8px 0 4px;
  cursor: grab;
  flex-shrink: 0;
  touch-action: none;
  user-select: none;
}

.bindsheet-handle:active {
  cursor: grabbing;
}

.bindsheet-handle-bar {
  width: 36px;
  height: 4px;
  background: var(--border, #2a2f3d);
  border-radius: 2px;
  opacity: 0.8;
}

/* ── Header ─────────────────────────────────────────── */
.bindsheet-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 16px;
  min-height: 48px;
  flex-shrink: 0;
  border-bottom: 1px solid var(--border, #2a2f3d);
  cursor: default;
  touch-action: none;
  user-select: none;
  gap: 12px;
}

.bindsheet-header-left {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  flex: 1;
}

.bindsheet-title-area {
  display: flex;
  flex-direction: column;
  min-width: 0;
  gap: 2px;
}

.bindsheet-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--text, #e4e6eb);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  line-height: 1.3;
}

.bindsheet-subtitle {
  font-size: 12px;
  color: var(--muted, #8b909c);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  line-height: 1.3;
}

.bindsheet-header-right {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

/* ── Buttons ────────────────────────────────────────── */
.bindsheet-back-btn,
.bindsheet-close-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  padding: 0;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--text, #e4e6eb);
  cursor: pointer;
  flex-shrink: 0;
  transition: background 0.15s;
}

.bindsheet-back-btn:hover,
.bindsheet-close-btn:hover {
  background: var(--card, #1a1d27);
}

.bindsheet-close-btn {
  color: var(--muted, #8b909c);
}

.bindsheet-close-btn:hover {
  color: var(--text, #e4e6eb);
}

/* ── Content ────────────────────────────────────────── */
.bindsheet-content {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 16px;
  overscroll-behavior: contain;
  -webkit-overflow-scrolling: touch;
}

.bindsheet-panel--dragging .bindsheet-content {
  overflow-y: hidden;
}

/* Scrollbar */
.bindsheet-content::-webkit-scrollbar {
  width: 6px;
}

.bindsheet-content::-webkit-scrollbar-thumb {
  background: var(--border, #2a2f3d);
  border-radius: 3px;
}

.bindsheet-content::-webkit-scrollbar-thumb:hover {
  background: #3a4050;
}

/* ── Responsive: phone landscape bottom sheet ───────── */
@media (max-width: 599px) and (orientation: landscape) {
  .bindsheet-panel--bottom {
    left: 50%;
    right: auto;
    bottom: 0;
    transform: translateX(-50%);
    width: 480px;
    max-width: 100vw;
    border-radius: 7px 7px 0 0;
  }
}

/* ── Responsive: tablet range ───────────────────────── */
@media (min-width: 600px) and (max-width: 1023px) {
  .bindsheet-panel--center {
    width: min(480px, calc(100vw - 32px));
  }
}

/* ── Responsive: narrow tablet (portrait foldable) ──── */
@media (min-width: 600px) and (max-width: 767px) {
  .bindsheet-panel--center {
    width: min(420px, calc(100vw - 32px));
    min-height: 280px;
  }
}
</style>

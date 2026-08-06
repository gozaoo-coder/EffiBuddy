/**
 * useWidgetStack —— 卡片堆叠 composable（vue-bits Stack 交互模型）
 *
 * 核心交互（参照 vue-bits Stack）：
 * - 循环堆叠：拖拽 / 点击将顶层卡片「送回底部」（sendToBack），下一张成为顶层
 * - 拖拽：实时位移 + 3D 倾斜（rotateX/rotateY 跟随），超灵敏度阈值 → 飞出 → 送回底部
 * - 点击：按 interactionMode 送回底部 / 提到顶层
 * - autoplay：定时轮播（悬停可暂停）
 * - spring 弹簧动画（animejs spring easing，stiffness/damping 可调）
 *
 * 动画全流程管线：
 * 1. 开始（push）：新卡片加入，全部卡片重新排列到新层
 * 2. 中断（dragStart）：拖拽暂停当前排列动画
 * 3. 拖拽中（dragMove）：实时更新 transform（层变换 + 位移 + 3D 倾斜）
 * 4. 返回（dragEnd 未超阈值）：spring 回弹到层位置
 * 5. 循环（dragEnd 超阈值）：飞出 → sendToBack → 全卡 spring 重排
 * 6. 离开（dismiss）：轻扫飞出移除（loop=false）/ 循环送回底部（loop=true）
 * 7. 父子关系：Stack 管理所有子卡片的 z-index、transform
 */
import { ref, computed, watch, onScopeDispose, toValue, type MaybeRefOrGetter, type Ref } from 'vue'
import { animate } from 'animejs'
import {
  STACK_PRESETS,
  DEFAULT_STACK_INTERACTION,
  type StackLayer,
  type StackEvent,
  type StackInteractionOptions,
} from '../types/widget'

export interface UseWidgetStackOptions<T extends { id: string }> {
  /** 堆叠预设名（'bits' 为 vue-bits 风格：按层索引旋转/缩放） */
  preset?: keyof typeof STACK_PRESETS | string
  /** 自定义层配置（覆盖预设） */
  layers?: StackLayer[]
  /** 最大可见层数 */
  maxVisible?: number
  /** 排列动画时长（ms，非 spring 模式下生效） */
  duration?: number
  /** 排列动画缓动 */
  ease?: string
  /** 事件回调 */
  onEvent?: (event: StackEvent) => void
  /** 卡片容器尺寸（px，用于层偏移 % → px 转换） */
  cardWidth?: number
  /** 卡片容器高度（px） */
  cardHeight?: number
  /** vue-bits 风格交互配置（支持响应式） */
  interaction?: MaybeRefOrGetter<StackInteractionOptions>
}

/** 单张卡片的拖拽运行时状态（非响应式，直接操作 DOM，保证拖拽零开销） */
interface DragState {
  /** 拖拽位移 x（px） */
  x: number
  /** 拖拽位移 y（px） */
  y: number
  /** 3D 倾斜 x（deg） */
  rotateX: number
  /** 3D 倾斜 y（deg） */
  rotateY: number
  /** 是否处于拖拽中 */
  active: boolean
}

export interface UseWidgetStackReturn<T extends { id: string }> {
  /** 当前堆叠中的卡片列表（底部 → 顶部） */
  items: Ref<T[]>
  /** 顶层卡片索引 */
  topIndex: Ref<number>
  /** 按堆叠顺序排列的卡片（含层变换数据，偏移已转为 px） */
  stacked: Ref<Array<{ item: T; layer: StackLayer; zIndex: number; offsetXPx: number; offsetYPx: number }>>
  /** 添加卡片到堆叠顶部 */
  push: (item: T) => void
  /** 移除指定卡片 */
  remove: (id: string) => void
  /** 将指定卡片提到顶层 */
  bringToFront: (id: string) => void
  /** 将指定卡片送回底部（vue-bits 循环堆叠核心） */
  sendToBack: (id: string) => void
  /** 拖拽移除顶层卡片（loop=false 移除 / loop=true 循环送回底部） */
  dismissTop: (direction?: 'left' | 'right' | 'up' | 'down') => void
  /** 重新排列全部卡片 */
  reorder: (items: T[]) => void
  /** 清空堆叠 */
  clear: () => void
  /** 获取卡片在堆叠中的索引 */
  getIndex: (id: string) => number
  /** 拖拽开始（绑定到顶层卡片 pointerdown） */
  dragStart: (id: string) => void
  /** 拖拽移动（实时位移 + 3D 倾斜） */
  dragMove: (id: string, dx: number, dy: number) => void
  /** 拖拽结束（阈值判定：飞出送回底部 / spring 回位） */
  dragEnd: (id: string, velocityX?: number, velocityY?: number) => void
  /** 取消拖拽（spring 回位） */
  cancelDrag: (id: string) => void
  /** 是否正在拖拽 */
  isDragging: Ref<boolean>
  /** 当前拖拽的卡片 id */
  activeDragId: Ref<string | null>
  /** 开始自动轮播 */
  startAutoplay: () => void
  /** 停止自动轮播 */
  stopAutoplay: () => void
  /** 获取卡片当前完整 transform（层 + 拖拽），供组件渲染 */
  getCardStyle: (id: string) => { transform: string; opacity: number; zIndex: number }
}

export function useWidgetStack<T extends { id: string }>(
  rootRef: Ref<HTMLElement | null>,
  options: UseWidgetStackOptions<T> = {},
): UseWidgetStackReturn<T> {
  const preset = options.preset || 'fan'
  const isBitsPreset = preset === 'bits'
  const baseLayers = options.layers || STACK_PRESETS[preset] || STACK_PRESETS.fan
  const maxVisible = options.maxVisible || baseLayers.length
  const animDuration = options.duration ?? 300
  const animEase = options.ease ?? 'out(3)'
  const cardWidth = options.cardWidth ?? 320
  const cardHeight = options.cardHeight ?? 420

  // vue-bits 交互配置（支持响应式更新）
  const interaction = ref<Required<StackInteractionOptions>>({
    ...DEFAULT_STACK_INTERACTION,
    ...(toValue(options.interaction) ?? {}),
  })
  watch(
    () => toValue(options.interaction),
    (v) => {
      interaction.value = { ...DEFAULT_STACK_INTERACTION, ...(v ?? {}) }
      if (interaction.value.autoplay) startAutoplay()
      else stopAutoplay()
    },
    { deep: true },
  )
  const stiffness = () => interaction.value.stiffness
  const damping = () => interaction.value.damping
  const sensitivity = () => interaction.value.sensitivity
  const tiltScale = () => interaction.value.tiltAmount / 100
  const springEase = () => `spring(1, ${stiffness()}, ${damping()})`

  const items = ref<T[]>([]) as Ref<T[]>
  const topIndex = ref(0)
  const isDragging = ref(false)
  const activeDragId = ref<string | null>(null)
  let currentAnim: ReturnType<typeof animate> | null = null
  let autoplayTimer: ReturnType<typeof setInterval> | null = null

  /** 拖拽运行时状态（按卡片 id） */
  const dragStates = new Map<string, DragState>()
  /** 每张卡片的随机旋转角 */
  const randomRotations = new Map<string, number>()

  function ensureRandomRotation(id: string): number {
    let rot = randomRotations.get(id)
    if (rot === undefined) {
      rot = interaction.value.randomRotation ? Math.random() * 10 - 5 : 0
      randomRotations.set(id, rot)
    }
    return rot
  }

  /** 计算每张卡片的层变换（偏移转为 px） */
  const stacked = computed(() => {
    const visible = items.value.slice(-maxVisible)
    const total = visible.length
    return visible.map((item, i) => {
      const layerIndex = total - 1 - i // 最新卡片在顶层
      let layer: StackLayer
      if (isBitsPreset) {
        // vue-bits 风格：旋转/缩放随层索引动态计算
        const randomRot = ensureRandomRotation(item.id)
        layer = {
          offsetX: 0,
          offsetY: 0,
          scale: 1 + i * 0.06 - total * 0.06,
          rotate: (total - i - 1) * 4 + randomRot,
          opacity: 1,
          zOffset: total - i - 1,
        }
      } else {
        layer = baseLayers[Math.min(layerIndex, baseLayers.length - 1)] || baseLayers[baseLayers.length - 1]
      }
      return {
        item,
        layer,
        zIndex: layer.zOffset + i * 10,
        offsetXPx: (layer.offsetX / 100) * cardWidth,
        offsetYPx: (layer.offsetY / 100) * cardHeight,
      }
    })
  })

  function getLayerOf(id: string) {
    return stacked.value.find((s) => s.item.id === id)
  }

  function getCardEl(id: string): HTMLElement | null {
    if (!rootRef.value) return null
    return rootRef.value.querySelector(`[data-stack-id="${id}"]`) as HTMLElement | null
  }

  function buildTransform(
    offsetXPx: number,
    offsetYPx: number,
    rotate: number,
    scale: number,
    drag?: DragState,
  ): string {
    const x = offsetXPx + (drag?.x ?? 0)
    const y = offsetYPx + (drag?.y ?? 0)
    const rotX = drag?.rotateX ?? 0
    const rotY = drag?.rotateY ?? 0
    return `translate(${x}px, ${y}px) rotate(${rotate}deg) rotateX(${rotX}deg) rotateY(${rotY}deg) scale(${scale})`
  }

  function getCardStyle(id: string): { transform: string; opacity: number; zIndex: number } {
    const layer = getLayerOf(id)
    const drag = dragStates.get(id)
    if (!layer) return { transform: 'translate(0px, 0px)', opacity: 1, zIndex: 0 }
    return {
      transform: buildTransform(layer.offsetXPx, layer.offsetYPx, layer.layer.rotate, layer.layer.scale, drag),
      opacity: layer.layer.opacity,
      zIndex: layer.zIndex,
    }
  }

  function applyCardTransform(id: string) {
    const el = getCardEl(id)
    if (!el) return
    const style = getCardStyle(id)
    el.style.transform = style.transform
    el.style.opacity = String(style.opacity)
    el.style.zIndex = String(style.zIndex)
  }

  /** spring 动画卡片到目标层（或直接设置） */
  function animateCardToLayer(id: string, immediate = false): Promise<void> {
    const el = getCardEl(id)
    const layer = getLayerOf(id)
    if (!el || !layer) return Promise.resolve()

    const target = buildTransform(layer.offsetXPx, layer.offsetYPx, layer.layer.rotate, layer.layer.scale)

    if (immediate) {
      el.style.transform = target
      el.style.opacity = String(layer.layer.opacity)
      el.style.zIndex = String(layer.zIndex)
      return Promise.resolve()
    }

    return new Promise((resolve) => {
      currentAnim?.pause()
      currentAnim = animate(el, {
        transform: target,
        opacity: layer.layer.opacity,
        zIndex: layer.zIndex,
        ease: springEase(),
        duration: animDuration,
        onComplete: () => {
          currentAnim = null
          resolve()
        },
      })
    })
  }

  async function arrangeAll(immediate = false) {
    const promises = stacked.value.map((s) => animateCardToLayer(s.item.id, immediate))
    await Promise.all(promises)
  }

  function emitEvent(type: StackEvent['type'], cardId: string, index?: number) {
    options.onEvent?.({ type, cardId, index })
  }

  function push(item: T) {
    items.value = [...items.value, item]
    topIndex.value = items.value.length - 1
    void arrangeAll()
    emitEvent('card-selected', item.id, items.value.length - 1)
  }

  function remove(id: string) {
    const idx = items.value.findIndex((i) => i.id === id)
    if (idx === -1) return
    dragStates.delete(id)
    items.value = items.value.filter((i) => i.id !== id)
    topIndex.value = Math.max(0, items.value.length - 1)
    void arrangeAll()
    emitEvent('card-dismissed', id, idx)
  }

  function bringToFront(id: string) {
    const idx = items.value.findIndex((i) => i.id === id)
    if (idx === -1 || idx === items.value.length - 1) return
    const item = items.value[idx]
    items.value = [...items.value.filter((i) => i.id !== id), item]
    topIndex.value = items.value.length - 1
    void arrangeAll()
    emitEvent('card-selected', id, items.value.length - 1)
  }

  /** vue-bits 循环堆叠核心：将卡片送回底部，其余卡片上移一层 */
  function sendToBack(id: string) {
    const idx = items.value.findIndex((i) => i.id === id)
    if (idx === -1 || idx === 0) return
    const item = items.value[idx]
    items.value = [item, ...items.value.filter((i) => i.id !== id)]
    topIndex.value = items.value.length - 1
    // 送回底部的卡片立即落到底层位置（无回放动画），其余卡片 spring 上移
    const moved = getLayerOf(id)
    if (moved) {
      void animateCardToLayer(id, true)
    }
    void arrangeAll()
    emitEvent('card-send-back', id, 0)
  }

  // ===================== 拖拽交互（vue-bits 模型） =====================

  function getDragState(id: string): DragState {
    let state = dragStates.get(id)
    if (!state) {
      state = { x: 0, y: 0, rotateX: 0, rotateY: 0, active: false }
      dragStates.set(id, state)
    }
    return state
  }

  function dragStart(id: string) {
    if (interaction.value.topOnlyDraggable && getIndex(id) !== items.value.length - 1) return
    currentAnim?.pause()
    currentAnim = null
    const state = getDragState(id)
    state.x = 0
    state.y = 0
    state.rotateX = 0
    state.rotateY = 0
    state.active = true
    isDragging.value = true
    activeDragId.value = id
    emitEvent('card-drag-start', id)
  }

  function dragMove(id: string, dx: number, dy: number) {
    const state = dragStates.get(id)
    const el = getCardEl(id)
    const layer = getLayerOf(id)
    if (!state || !el || !layer) return

    state.x = dx
    state.y = dy
    // vue-bits 3D 倾斜：位移 ±100px ↔ 旋转 ±tiltAmount deg
    state.rotateX = -dy * tiltScale()
    state.rotateY = dx * tiltScale()

    el.style.transform = buildTransform(
      layer.offsetXPx,
      layer.offsetYPx,
      layer.layer.rotate,
      layer.layer.scale,
      state,
    )
    el.style.opacity = String(layer.layer.opacity)
  }

  function dragEnd(id: string, velocityX = 0, velocityY = 0) {
    const state = dragStates.get(id)
    if (!state) return
    state.active = false
    isDragging.value = false
    activeDragId.value = null

    const distance = Math.hypot(state.x, state.y)
    const hasFlick = Math.hypot(velocityX, velocityY) > 0.5 && distance > sensitivity() * 0.6

    if (distance > sensitivity() || hasFlick) {
      // 超出阈值 → 沿拖拽方向飞出 → 送回底部（循环）
      flyOut(id, state.x, state.y, () => {
        dragStates.delete(id)
        sendToBack(id)
      })
    } else {
      // 未超阈值 → spring 回弹到层位置
      springBack(id)
    }
    emitEvent('card-drag-end', id)
  }

  function cancelDrag(id: string) {
    const state = dragStates.get(id)
    if (!state) return
    state.active = false
    isDragging.value = false
    activeDragId.value = null
    springBack(id)
  }

  function springBack(id: string) {
    const el = getCardEl(id)
    const layer = getLayerOf(id)
    if (!el || !layer) return
    const target = buildTransform(layer.offsetXPx, layer.offsetYPx, layer.layer.rotate, layer.layer.scale)
    currentAnim?.pause()
    currentAnim = animate(el, {
      transform: target,
      ease: springEase(),
      duration: animDuration,
      onComplete: () => {
        currentAnim = null
        const s = dragStates.get(id)
        if (s) {
          s.x = 0
          s.y = 0
          s.rotateX = 0
          s.rotateY = 0
        }
        el.style.transform = target
        emitEvent('card-returned', id)
      },
    })
  }

  function flyOut(id: string, x: number, y: number, onDone: () => void) {
    const el = getCardEl(id)
    const layer = getLayerOf(id)
    if (!el || !layer) {
      onDone()
      return
    }
    const len = Math.hypot(x, y) || 1
    const flyDist = 600
    const flyX = layer.offsetXPx + x + (x / len) * flyDist
    const flyY = layer.offsetYPx + y + (y / len) * flyDist

    currentAnim?.pause()
    currentAnim = animate(el, {
      transform: `translate(${flyX}px, ${flyY}px) rotate(${layer.layer.rotate}deg) rotateX(${-y * tiltScale()}deg) rotateY(${x * tiltScale()}deg) scale(0.8)`,
      opacity: 0,
      duration: 320,
      ease: 'in(2)',
      onComplete: () => {
        currentAnim = null
        el.style.opacity = ''
        onDone()
      },
    })
  }

  /** 轻扫移除：loop=true 循环送回底部 / loop=false 真正移除 */
  function dismissTop(direction: 'left' | 'right' | 'up' | 'down' = 'left') {
    if (items.value.length === 0) return
    const top = items.value[items.value.length - 1]
    const el = getCardEl(top.id)
    const layer = getLayerOf(top.id)
    if (!el || !layer) return

    const signMap = { left: -1, right: 1, up: -1, down: 1 }
    const axis = direction === 'left' || direction === 'right' ? 'x' : 'y'
    const sign = signMap[direction]
    const distance = 500

    currentAnim?.pause()
    const target =
      axis === 'x'
        ? `translate(${layer.offsetXPx + sign * distance}px, ${layer.offsetYPx}px) rotate(${layer.layer.rotate + sign * 15}deg) scale(0.8)`
        : `translate(${layer.offsetXPx}px, ${layer.offsetYPx + sign * distance}px) rotate(${layer.layer.rotate}deg) scale(0.8)`

    currentAnim = animate(el, {
      transform: target,
      opacity: 0,
      duration: 350,
      ease: 'in(2)',
      onComplete: () => {
        currentAnim = null
        el.style.opacity = ''
        if (interaction.value.loop) {
          // 循环堆叠：直接送回底部
          dragStates.delete(top.id)
          sendToBack(top.id)
        } else {
          remove(top.id)
        }
      },
    })
  }

  // ===================== 自动轮播 =====================

  function startAutoplay() {
    stopAutoplay()
    if (!interaction.value.autoplay || items.value.length < 2) return
    autoplayTimer = setInterval(() => {
      if (isDragging.value || items.value.length < 2) return
      const top = items.value[items.value.length - 1]
      sendToBack(top.id)
      emitEvent('autoplay', top.id)
    }, interaction.value.autoplayDelay)
  }

  function stopAutoplay() {
    if (autoplayTimer) {
      clearInterval(autoplayTimer)
      autoplayTimer = null
    }
  }

  // 交互配置 / 卡片数量变化时同步轮播
  watch(
    () => [interaction.value.autoplay, interaction.value.autoplayDelay, items.value.length],
    () => {
      if (interaction.value.autoplay) startAutoplay()
      else stopAutoplay()
    },
    { immediate: true },
  )

  onScopeDispose(() => {
    stopAutoplay()
    currentAnim?.pause()
    currentAnim = null
  })

  function reorder(newItems: T[]) {
    items.value = newItems
    topIndex.value = Math.max(0, newItems.length - 1)
    void arrangeAll()
    emitEvent('stack-reordered', '')
  }

  function clear() {
    stopAutoplay()
    items.value = []
    topIndex.value = 0
    dragStates.clear()
  }

  function getIndex(id: string): number {
    return items.value.findIndex((i) => i.id === id)
  }

  return {
    items,
    topIndex,
    stacked,
    push,
    remove,
    bringToFront,
    sendToBack,
    dismissTop,
    reorder,
    clear,
    getIndex,
    dragStart,
    dragMove,
    dragEnd,
    cancelDrag,
    isDragging,
    activeDragId,
    startAutoplay,
    stopAutoplay,
    getCardStyle,
  }
}

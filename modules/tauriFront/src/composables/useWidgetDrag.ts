/**
 * useWidgetDrag —— 原生指针事件驱动的卡片拖拽 composable
 *
 * 动画全流程管线：
 * 1. 开始（pointerdown）：记录起始位置，置 DragStatus='idle'
 * 2. 阈值判定：移动超过 threshold px 后进入 dragging 态
 * 3. 拖拽中（pointermove）：update transform，实时更新位置（越界时按 elastic 弹性跟随）
 * 4. 打断：新 pointerdown 中断旧拖拽，reset 到起始位置
 * 5. 轻扫判定（pointerup）：计算速度/距离，若超阈值则 dismissing
 * 6. 返回（pointerup 未触发轻扫）：spring 回原始位置，DragStatus='returning'
 * 7. DOM 清理：动画结束后清除内联 transform
 */
import { ref, type Ref } from 'vue'
import { animate } from 'animejs'
import { DEFAULT_DRAG_CONFIG, type DragConfig, type DragStatus } from '../types/widget'

export interface UseWidgetDragOptions {
  /** 拖拽配置 */
  config?: Partial<DragConfig>
  /** 拖拽开始回调（返回 false 可阻止拖拽） */
  onDragStart?: (e: PointerEvent) => boolean | void
  /** 拖拽中回调 */
  onDragMove?: (x: number, y: number, e: PointerEvent) => void
  /** 拖拽结束回调 */
  onDragEnd?: (x: number, y: number, flicked: boolean) => void
  /** 轻扫回调 */
  onFlick?: (direction: 'left' | 'right' | 'up' | 'down') => void
  /** 返回回调 */
  onReturn?: () => void
}

export interface UseWidgetDragReturn {
  /** 当前拖拽状态 */
  status: Ref<DragStatus>
  /** 当前偏移量 x */
  offsetX: Ref<number>
  /** 当前偏移量 y */
  offsetY: Ref<number>
  /** 绑定到卡片的 pointer 事件处理 */
  bind: {
    onPointerDown: (e: PointerEvent) => void
  }
  /** 重置到原始位置（带动画） */
  reset: (animated?: boolean) => void
  /** 销毁清理 */
  dispose: () => void
}

export function useWidgetDrag(
  elRef: Ref<HTMLElement | null>,
  options: UseWidgetDragOptions = {},
): UseWidgetDragReturn {
  const mergedConfig: DragConfig = {
    ...DEFAULT_DRAG_CONFIG,
    ...options.config,
  }

  const status = ref<DragStatus>('idle')
  const offsetX = ref(0)
  const offsetY = ref(0)

  // 内部状态
  let startX = 0
  let startY = 0
  let lastX = 0
  let lastY = 0
  let lastTime = 0
  let velocityX = 0
  let velocityY = 0
  let isDragging = false
  let currentAnim: ReturnType<typeof animate> | null = null
  let parentRect: DOMRect | null = null

  function getBounds(): { minX: number; maxX: number; minY: number; maxY: number } {
    if (mergedConfig.bound === 'none' || !elRef.value) {
      return { minX: -Infinity, maxX: Infinity, minY: -Infinity, maxY: Infinity }
    }
    const el = elRef.value
    const elRect = el.getBoundingClientRect()
    if (mergedConfig.bound === 'viewport') {
      return {
        minX: -elRect.left,
        maxX: window.innerWidth - elRect.right,
        minY: -elRect.top,
        maxY: window.innerHeight - elRect.bottom,
      }
    }
    // parent
    if (!parentRect) {
      parentRect = el.parentElement?.getBoundingClientRect() ?? null
    }
    if (parentRect) {
      return {
        minX: -(elRect.left - parentRect.left),
        maxX: parentRect.width - (elRect.left - parentRect.left) - elRect.width,
        minY: -(elRect.top - parentRect.top),
        maxY: parentRect.height - (elRect.top - parentRect.top) - elRect.height,
      }
    }
    return { minX: -Infinity, maxX: Infinity, minY: -Infinity, maxY: Infinity }
  }

  function clamp(v: number, min: number, max: number): number {
    if (v < min) {
      // 越界：超出部分按弹性系数跟随（elastic=0 硬边界 / 0.6 强弹性）
      const elastic = mergedConfig.elastic
      return elastic > 0 ? min + (v - min) * (1 - elastic) : min
    }
    if (v > max) {
      const elastic = mergedConfig.elastic
      return elastic > 0 ? max + (v - max) * (1 - elastic) : max
    }
    return v
  }

  function onPointerDown(e: PointerEvent) {
    if (!mergedConfig.enabled || e.button !== mergedConfig.button) return
    if (options.onDragStart?.(e) === false) return

    // 打断当前动画
    currentAnim?.pause()
    currentAnim = null

    const el = elRef.value
    if (!el) return

    el.setPointerCapture(e.pointerId)
    el.style.touchAction = 'none'
    el.style.userSelect = 'none'

    startX = e.clientX
    startY = e.clientY
    lastX = startX
    lastY = startY
    lastTime = performance.now()
    velocityX = 0
    velocityY = 0
    isDragging = false
    status.value = 'idle'
    parentRect = null

    // 预计算边界
    getBounds()

    document.addEventListener('pointermove', onPointerMove)
    document.addEventListener('pointerup', onPointerUp)
    document.addEventListener('pointercancel', onPointerCancel)
  }

  function onPointerMove(e: PointerEvent) {
    const dx = e.clientX - startX
    const dy = e.clientY - startY

    // 速度计算
    const now = performance.now()
    const dt = now - lastTime
    if (dt > 0) {
      velocityX = (e.clientX - lastX) / dt
      velocityY = (e.clientY - lastY) / dt
    }
    lastX = e.clientX
    lastY = e.clientY
    lastTime = now

    // 阈值判定
    if (!isDragging) {
      if (Math.abs(dx) < mergedConfig.threshold && Math.abs(dy) < mergedConfig.threshold) return
      isDragging = true
      status.value = 'dragging'
    }

    const bounds = getBounds()
    const newX = clamp(dx, bounds.minX, bounds.maxX)
    const newY = clamp(dy, bounds.minY, bounds.maxY)

    offsetX.value = newX
    offsetY.value = newY
    applyTransform(newX, newY)

    options.onDragMove?.(newX, newY, e)
  }

  function onPointerUp(e: PointerEvent) {
    cleanup()
    if (!isDragging) return

    // 轻扫判定
    const speed = Math.sqrt(velocityX * velocityX + velocityY * velocityY)
    const distance = Math.sqrt(offsetX.value * offsetX.value + offsetY.value * offsetY.value)
    const flicked =
      speed > mergedConfig.flickVelocity && distance > mergedConfig.flickDistance

    if (flicked) {
      status.value = 'dismissing'
      // 沿速度方向飞出
      const angle = Math.atan2(velocityY, velocityX)
      const flyDistance = 500
      const flyX = offsetX.value + Math.cos(angle) * flyDistance
      const flyY = offsetY.value + Math.sin(angle) * flyDistance
      animateOut(flyX, flyY, () => {
        const dir = getDirection()
        options.onFlick?.(dir)
        options.onDragEnd?.(offsetX.value, offsetY.value, true)
      })
    } else {
      // 返回原位
      returnToOrigin()
    }
  }

  function getDirection(): 'left' | 'right' | 'up' | 'down' {
    if (Math.abs(velocityX) > Math.abs(velocityY)) {
      return velocityX > 0 ? 'right' : 'left'
    }
    return velocityY > 0 ? 'down' : 'up'
  }

  function onPointerCancel() {
    cleanup()
    if (isDragging) {
      returnToOrigin()
    }
  }

  function cleanup() {
    document.removeEventListener('pointermove', onPointerMove)
    document.removeEventListener('pointerup', onPointerUp)
    document.removeEventListener('pointercancel', onPointerCancel)
    const el = elRef.value
    if (el) {
      el.style.touchAction = ''
      el.style.userSelect = ''
    }
  }

  function applyTransform(x: number, y: number) {
    const el = elRef.value
    if (!el) return
    el.style.transform = `translate(${x}px, ${y}px)`
  }

  function returnToOrigin() {
    const el = elRef.value
    if (!el) return
    status.value = 'returning'
    currentAnim = animate(el, {
      translateX: [offsetX.value + 'px', 0],
      translateY: [offsetY.value + 'px', 0],
      ease: 'spring(1, 260, 20)',
      onComplete: () => {
        offsetX.value = 0
        offsetY.value = 0
        status.value = 'idle'
        currentAnim = null
        el.style.transform = ''
        options.onReturn?.()
        options.onDragEnd?.(0, 0, false)
      },
    })
  }

  function animateOut(x: number, y: number, onDone: () => void) {
    const el = elRef.value
    if (!el) {
      onDone()
      return
    }
    currentAnim = animate(el, {
      translateX: x + 'px',
      translateY: y + 'px',
      opacity: 0,
      scale: 0.6,
      duration: 400,
      ease: 'in(2)',
      onComplete: () => {
        currentAnim = null
        el.style.transform = ''
        el.style.opacity = ''
        el.style.scale = ''
        onDone()
      },
    })
  }

  function reset(animated = true) {
    const el = elRef.value
    currentAnim?.pause()
    offsetX.value = 0
    offsetY.value = 0
    status.value = 'idle'
    if (animated && el) {
      currentAnim = animate(el, {
        translateX: '0px',
        translateY: '0px',
        ease: 'spring(1, 260, 20)',
        onComplete: () => {
          currentAnim = null
          el.style.transform = ''
        },
      })
    } else if (el) {
      el.style.transform = ''
    }
  }

  function dispose() {
    cleanup()
    currentAnim?.pause()
    currentAnim = null
  }

  return {
    status,
    offsetX,
    offsetY,
    bind: {
      onPointerDown,
    },
    reset,
    dispose,
  }
}
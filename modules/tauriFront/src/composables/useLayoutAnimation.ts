/**
 * useLayoutAnimation — animejs v4 Layout 模块封装
 *
 * 提供三个核心能力：
 * 1. CSS display property animation — flex/grid 等布局切换动画
 * 2. Enter layout animation — 新元素进入布局时的入场动画
 * 3. Exit layout animation — 元素离开布局时的退场动画
 *
 * 基于 animejs v4 createLayout / AutoLayout，自动与 Vue 响应式系统协作。
 * 延迟初始化：在 onMounted 后首次调用 update() 时才创建 Layout 实例，
 * 确保 DOM 容器已就绪。
 */
import { createLayout, utils, stagger } from 'animejs'
import type { AutoLayout } from 'animejs'
import { onMounted, onUnmounted } from 'vue'

export interface LayoutAnimationOptions {
  /** 容器选择器或元素引用 */
  container: string | HTMLElement
  /** 动画时长 (ms)，默认 300 */
  duration?: number
  /** 缓动函数，默认 'outQuad' */
  ease?: string
  /** 入场动画起始状态 */
  enterFrom?: Record<string, number | string>
  /** 入场动画时长 (ms)，默认继承 duration */
  enterDuration?: number
  /** 入场缓动，默认继承 ease */
  enterEase?: string
  /** 退场动画目标状态 */
  leaveTo?: Record<string, number | string>
  /** 退场动画时长 (ms)，默认继承 duration */
  leaveDuration?: number
  /** 退场缓动，默认继承 ease */
  leaveEase?: string
  /** 交错延迟 (ms)，通过 stagger() 控制 */
  staggerDelay?: number
}

export interface LayoutInstance {
  /** 原始 animejs AutoLayout 实例（延迟初始化，首次 update 前为 null） */
  layout: AutoLayout | null
  /** 更新布局：在回调中修改 DOM，animejs 自动处理动画 */
  update: (fn: (ctx: { root: HTMLElement }) => void) => Promise<void>
  /** 离开中的元素引用 */
  leaving: Array<Element>
  /** 销毁实例 */
  destroy: () => void
}

/**
 * 创建一个 animejs Layout 实例，绑定到指定容器。
 * 延迟初始化：在 onMounted 后首次调用 update() 时才创建实例。
 *
 * @example 基础用法 — CSS display 切换动画
 * ```ts
 * const layout = useLayoutAnimation({
 *   container: '.layout-container',
 *   leaveTo: { transform: 'scale(0)', opacity: 0 },
 * })
 *
 * function switchLayout() {
 *   layout.update(({ root }) => {
 *     root.classList.remove('flex-row')
 *     root.classList.add('grid-2')
 *   })
 * }
 * ```
 *
 * @example 入场动画
 * ```ts
 * const layout = useLayoutAnimation({
 *   container: '.msg-list',
 *   enterFrom: { transform: 'translateY(20px) scale(0.95)', opacity: 0 },
 * })
 *
 * layout.update(({ root }) => {
 *   const el = document.createElement('div')
 *   el.className = 'msg-bubble'
 *   el.textContent = 'Hello'
 *   root.appendChild(el)
 * })
 * ```
 *
 * @example 退场动画 + DOM 清理
 * ```ts
 * const layout = useLayoutAnimation({
 *   container: '.msg-list',
 *   leaveTo: { transform: 'translateY(-20px) scale(0.9)', opacity: 0 },
 * })
 *
 * function removeFirst() {
 *   layout.update(({ root }) => {
 *     const first = root.querySelector('.item:not(.is-hidden)')
 *     if (first) first.classList.add('is-hidden')
 *   }).then(() => {
 *     layout.leaving.forEach(el => (el as HTMLElement).remove())
 *   })
 * }
 * ```
 */
export function useLayoutAnimation(options: LayoutAnimationOptions): LayoutInstance {
  const {
    container,
    duration = 300,
    ease = 'outQuad',
    enterFrom,
    enterDuration,
    enterEase,
    leaveTo,
    leaveDuration,
    leaveEase,
    staggerDelay,
  } = options

  let layout: AutoLayout | null = null
  let mounted = false

  function ensureLayout(): AutoLayout {
    if (!layout) {
      // 构建 animejs createLayout 的配置
      const layoutConfig: Record<string, any> = { duration, ease }

      if (enterFrom) {
        layoutConfig.enterFrom = {
          ...enterFrom,
          duration: enterDuration ?? duration,
          ease: enterEase ?? ease,
        }
      }

      if (leaveTo) {
        const leaveConfig: Record<string, any> = {
          ...leaveTo,
          duration: leaveDuration ?? duration,
          ease: leaveEase ?? ease,
        }
        if (staggerDelay !== undefined) {
          leaveConfig.delay = stagger(staggerDelay)
        }
        layoutConfig.leaveTo = leaveConfig
      }

      layout = createLayout(container, layoutConfig as any)
    }
    return layout
  }

  const instance: LayoutInstance = {
    get layout() {
      return layout
    },
    get leaving() {
      return (layout?.leaving as unknown as Array<Element>) ?? []
    },
    update(fn: (ctx: { root: HTMLElement }) => void): Promise<void> {
      // 确保 DOM 已挂载 + 容器存在
      const containerExists =
        typeof container === 'string'
          ? !!document.querySelector(container)
          : document.contains(container)

      if (!mounted && !containerExists) {
        // 尚未挂载且容器不存在，延迟到下一帧再试
        return new Promise((resolve) => {
          requestAnimationFrame(() => {
            instance.update(fn).then(resolve)
          })
        })
      }

      const l = ensureLayout()
      const timeline = l.update((self: AutoLayout) => {
        fn({ root: self.root as HTMLElement })
      })

      if (timeline && typeof (timeline as any).then === 'function') {
        return (timeline as any).then(() => {})
      }
      return Promise.resolve()
    },
    destroy() {
      if (layout) {
        try {
          layout.revert()
        } catch {
          // 忽略清理错误
        }
        layout = null
      }
    },
  }

  onMounted(() => {
    mounted = true
    // 预初始化 Layout（让 animejs 开始观察 DOM）
    ensureLayout()
  })

  onUnmounted(() => {
    instance.destroy()
  })

  return instance
}

/**
 * 便捷方法：创建一个带入场动画的列表布局
 * 适用于消息列表、会话列表等动态添加元素的场景
 */
export function useListLayout(options: {
  container: string | HTMLElement
  enterFrom?: Record<string, number | string>
  duration?: number
  ease?: string
}): LayoutInstance {
  return useLayoutAnimation({
    container: options.container,
    duration: options.duration ?? 300,
    ease: options.ease ?? 'outQuad',
    enterFrom: options.enterFrom ?? {
      transform: 'translateY(16px) scale(0.95)',
      opacity: 0,
      duration: 350,
      ease: 'out(3)',
    },
  })
}

/**
 * 便捷方法：创建一个带退场动画的列表布局
 */
export function useRemovableListLayout(options: {
  container: string | HTMLElement
  leaveTo?: Record<string, number | string>
  duration?: number
  ease?: string
  staggerDelay?: number
}): LayoutInstance {
  return useLayoutAnimation({
    container: options.container,
    duration: options.duration ?? 280,
    ease: options.ease ?? 'outQuad',
    leaveTo: options.leaveTo ?? {
      transform: 'translateY(-16px) scale(0.9)',
      opacity: 0,
      duration: 350,
      ease: 'out(3)',
    },
    staggerDelay: options.staggerDelay ?? 60,
  })
}

/**
 * 便捷方法：CSS 布局切换动画（flex ↔ grid 等）
 */
export function useLayoutSwitcher(options: {
  container: string | HTMLElement
  duration?: number
  ease?: string
}): LayoutInstance {
  return useLayoutAnimation({
    container: options.container,
    duration: options.duration ?? 350,
    ease: options.ease ?? 'outQuad',
    leaveTo: {
      transform: 'scale(0.85)',
      opacity: 0,
      duration: 280,
      ease: 'out(2)',
    },
    enterFrom: {
      transform: 'scale(0.85)',
      opacity: 0,
      duration: 350,
      ease: 'out(3)',
    },
  })
}

export { stagger, utils }
/**
 * useLayout —— anime.js v4 Layout 模块的 Vue 适配 composable
 *
 * 核心能力：
 * 1. CSS display 属性动画（flex-row ↔ grid ↔ flex-col ↔ none …）
 * 2. Enter 动画（元素进入布局时）
 * 3. Exit 动画（元素离开布局时）
 *
 * Vue DOM 渲染问题处理：
 * - Vue 的 v-if 会立即移除元素，而 anime.js Layout 的 leaveTo 需要元素
 *   在动画期间仍留在 DOM 中。因此本 composable 提供 `update` 方法，
 *   在回调中变更 class/数据后再让 anime.js 接管过渡。
 * - 对于列表项的移除：使用 is-hidden 临时隐藏（display:none），动画
 *   完成后再从数据源移除，避免 Vue 直接卸载导致动画丢失。
 */
import { onBeforeUnmount, onMounted, ref, shallowRef, type Ref } from 'vue'
import { createLayout, type AutoLayout } from 'animejs'

/** 状态参数：CSS 属性 + 可选的 delay/duration/ease */
export type LayoutStateParams = Record<string, number | string> & {
  delay?: number
  duration?: number
  ease?: string
}

export interface UseLayoutOptions {
  /** 子元素选择器（决定哪些元素参与布局动画） */
  children?: string | string[]
  /** 整体动画时长 ms */
  duration?: number
  /** 整体缓动 */
  ease?: string
  /** 整体延迟 */
  delay?: number
  /** 进入布局时的起始状态 */
  enterFrom?: LayoutStateParams
  /** 离开布局时的目标状态 */
  leaveTo?: LayoutStateParams
  /** 位置交换中间态（默认 opacity:0，保持可见用 opacity:1） */
  swapAt?: LayoutStateParams
  /** 额外跟踪的 CSS 属性 */
  properties?: string[]
  /** 动画开始回调 */
  onBegin?: () => void
  /** 动画完成回调 */
  onComplete?: () => void
}

export interface UseLayoutReturn {
  /** Layout 实例（挂载前为 null） */
  layout: Ref<AutoLayout | null>
  /** 是否已就绪 */
  ready: Ref<boolean>
  /**
   * 在回调中变更布局（class/display/data-* 等），变更后自动执行动画。
   * 这是 anime.js Layout 的核心用法，参考：
   *   layout.update(({ root }) => root.classList.toggle('grid'))
   */
  update: (
    mutator: (layout: AutoLayout) => void,
    params?: {
      duration?: number
      delay?: number
      ease?: string
      onComplete?: () => void
    },
  ) => Promise<void>
  /** 仅记录当前快照（用于 record + animate 两步式调用） */
  record: () => void
  /** 在 record 后执行动画 */
  animate: (params?: { duration?: number; delay?: number; ease?: string }) => Promise<void>
  /** 手动销毁 */
  dispose: () => void
}

/**
 * 创建一个绑定到指定根元素的 anime.js Layout 实例。
 *
 * @param rootRef 根元素 ref（须在 onMounted 后才有值）
 * @param options 布局动画配置
 *
 * @example
 * const rootEl = ref<HTMLElement | null>(null)
 * const { update, layout } = useLayout(rootEl, {
 *   children: '.item',
 *   duration: 280,
 *   ease: 'outQuad',
 *   enterFrom: { opacity: 0, transform: 'translateY(20px) scale(.8)' },
 *   leaveTo: { opacity: 0, transform: 'scale(0)' },
 * })
 * // 切换 display 模式
 * async function toggleGrid() {
 *   await update(({ root }) => root.classList.toggle('grid-1'))
 * }
 */
export function useLayout(
  rootRef: Ref<HTMLElement | null>,
  options: UseLayoutOptions = {},
): UseLayoutReturn {
  const layout = shallowRef<AutoLayout | null>(null)
  const ready = ref(false)

  function buildParams(): Record<string, unknown> {
    const p: Record<string, unknown> = {}
    if (options.children !== undefined) p.children = options.children
    if (options.duration !== undefined) p.duration = options.duration
    if (options.ease !== undefined) p.ease = options.ease
    if (options.delay !== undefined) p.delay = options.delay
    if (options.enterFrom !== undefined) p.enterFrom = options.enterFrom
    if (options.leaveTo !== undefined) p.leaveTo = options.leaveTo
    if (options.swapAt !== undefined) p.swapAt = options.swapAt
    if (options.properties !== undefined) p.properties = options.properties
    if (options.onBegin !== undefined) p.onBegin = options.onBegin
    if (options.onComplete !== undefined) p.onComplete = options.onComplete
    return p
  }

  function init() {
    const root = rootRef.value
    if (!root || layout.value) return
    layout.value = createLayout(root, buildParams())
    ready.value = true
  }

  function dispose() {
    if (layout.value) {
      try {
        layout.value.revert()
      } catch {
        // ignore
      }
      layout.value = null
      ready.value = false
    }
  }

  onMounted(() => {
    // 等待 DOM 挂载后再创建 Layout，确保子元素已渲染
    init()
  })

  onBeforeUnmount(() => {
    dispose()
  })

  function toPromise(tl: { finished?: Promise<unknown> } | unknown): Promise<void> {
    if (tl && typeof tl === 'object' && 'finished' in tl && tl.finished) {
      return Promise.resolve(tl.finished).then(() => undefined)
    }
    return Promise.resolve()
  }

  function update(
    mutator: (layout: AutoLayout) => void,
    params?: { duration?: number; delay?: number; ease?: string; onComplete?: () => void },
  ): Promise<void> {
    if (!layout.value) return Promise.resolve()
    const animParams: Record<string, unknown> = {}
    if (params?.duration !== undefined) animParams.duration = params.duration
    if (params?.delay !== undefined) animParams.delay = params.delay
    if (params?.ease !== undefined) animParams.ease = params.ease
    if (params?.onComplete !== undefined) animParams.onComplete = params.onComplete
    const tl = layout.value.update(mutator as (layout: AutoLayout) => void, animParams)
    return toPromise(tl)
  }

  function record() {
    layout.value?.record()
  }

  function animate(params?: { duration?: number; delay?: number; ease?: string }): Promise<void> {
    if (!layout.value) return Promise.resolve()
    const animParams: Record<string, unknown> = {}
    if (params?.duration !== undefined) animParams.duration = params.duration
    if (params?.delay !== undefined) animParams.delay = params.delay
    if (params?.ease !== undefined) animParams.ease = params.ease
    const tl = layout.value.animate(animParams)
    return toPromise(tl)
  }

  return { layout, ready, update, record, animate, dispose }
}

/**
 * 预设的显示模式 class 列表，用于 CSS display 属性动画。
 * 与 main.css 中定义的 .layout-* 工具类一一对应。
 */
export const DISPLAY_MODES = [
  'layout-flex-row',
  'layout-grid-1',
  'layout-flex-col',
  'layout-none',
  'layout-grid-2',
  'layout-flex-row-reverse',
] as const

export type DisplayMode = (typeof DISPLAY_MODES)[number]

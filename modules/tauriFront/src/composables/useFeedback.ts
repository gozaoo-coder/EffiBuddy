import { reactive, readonly } from 'vue'

// 动画时长（ms），与 ToastHost/SnackbarHost 中 useLayout 的 duration 保持一致，
// 留 40ms 缓冲以确保动画完成后才从数据源移除元素。
const LEAVE_ANIM_DURATION = 320

// ============= Toast（即时反馈）=============
export type ToastType = 'info' | 'success' | 'warn' | 'error'
export type ToastPosition = 'top' | 'bottom'

export interface ToastOptions {
  /** 文本内容 */
  content: string
  /** 类型，默认 info */
  type?: ToastType
  /** 位置，默认 top */
  position?: ToastPosition
  /** 自动消失时长 ms，默认 3000；传 0 表示不自动消失 */
  duration?: number
}

interface ToastItem extends Required<Omit<ToastOptions, 'duration'>> {
  id: number
  duration: number
  timer: number | null
  /** 是否正在执行离开动画，true 时元素会被加上 is-hidden（display:none），动画完成后才从数据源移除 */
  hiding: boolean
}

const toastState = reactive<{ items: ToastItem[] }>({ items: [] })
let toastSeq = 0

function removeToast(id: number) {
  const idx = toastState.items.findIndex((t) => t.id === id)
  if (idx >= 0) {
    toastState.items.splice(idx, 1)
  }
}

function dismissToast(id: number) {
  const item = toastState.items.find((t) => t.id === id)
  if (!item || item.hiding) return
  if (item.timer) {
    window.clearTimeout(item.timer)
    item.timer = null
  }
  // 标记为正在离开，触发 anime.js Layout 的 leaveTo 动画
  item.hiding = true
  // 动画完成后再真正从数据源移除，避免 Vue v-for 立即卸载元素导致动画丢失
  window.setTimeout(() => removeToast(id), LEAVE_ANIM_DURATION)
}

function showToast(opts: ToastOptions): number {
  const id = ++toastSeq
  const duration = opts.duration ?? 3000
  const item: ToastItem = {
    id,
    content: opts.content,
    type: opts.type ?? 'info',
    position: opts.position ?? 'top',
    duration,
    timer: null,
    hiding: false,
  }
  if (duration > 0) {
    item.timer = window.setTimeout(() => dismissToast(id), duration)
  }
  // 同位置最多保留 3 条可见 toast，老的先动画移除（排除已在隐藏流程中的）
  const samePos = toastState.items.filter((t) => t.position === item.position && !t.hiding)
  while (samePos.length >= 3) {
    const old = samePos.shift()!
    dismissToast(old.id)
  }
  toastState.items.push(item)
  return id
}

export function useToast() {
  return {
    toast: showToast,
    dismiss: dismissToast,
    state: readonly(toastState),
  }
}

// ============= Snackbar（即时操作）=============
export interface SnackbarAction {
  /** 操作按钮文本，如"撤销"/"重试" */
  text: string
  /** 点击操作回调 */
  onClick?: () => void
}

export interface SnackbarOptions {
  /** 主文本 */
  content: string
  /** 可选操作按钮 */
  action?: SnackbarAction
  /** 模式：定时（默认 5s）/常驻；定时模式下滚动页面会自动关闭 */
  mode?: 'timed' | 'persistent'
  /** timed 模式下的自动消失时长 ms，默认 5000，范围 5000-10000 */
  duration?: number
}

interface SnackbarItem {
  id: number
  content: string
  action: SnackbarAction | null
  mode: 'timed' | 'persistent'
  duration: number
  timer: number | null
  /** 是否正在执行离开动画 */
  hiding: boolean
}

const snackbarState = reactive<{ items: SnackbarItem[] }>({ items: [] })
let snackbarSeq = 0

function removeSnackbar(id: number) {
  const idx = snackbarState.items.findIndex((s) => s.id === id)
  if (idx >= 0) {
    snackbarState.items.splice(idx, 1)
  }
}

function dismissSnackbar(id: number) {
  const item = snackbarState.items.find((s) => s.id === id)
  if (!item || item.hiding) return
  if (item.timer) {
    window.clearTimeout(item.timer)
    item.timer = null
  }
  item.hiding = true
  window.setTimeout(() => removeSnackbar(id), LEAVE_ANIM_DURATION)
}

function showSnackbar(opts: SnackbarOptions): number {
  const id = ++snackbarSeq
  const mode = opts.mode ?? 'timed'
  // timed 模式 duration 钳制到 5-10s
  let duration = opts.duration ?? 5000
  if (mode === 'timed') {
    duration = Math.max(5000, Math.min(10000, duration))
  } else {
    duration = 0 // persistent 不自动消失
  }
  const item: SnackbarItem = {
    id,
    content: opts.content,
    action: opts.action ?? null,
    mode,
    duration,
    timer: null,
    hiding: false,
  }
  if (duration > 0) {
    item.timer = window.setTimeout(() => dismissSnackbar(id), duration)
  }
  // 同一时间最多显示 1 条可见 snackbar，旧的动画移除（已在隐藏流程中的保留至动画结束）
  snackbarState.items.forEach((s) => {
    if (!s.hiding) dismissSnackbar(s.id)
  })
  snackbarState.items.push(item)
  return id
}

// 全局滚动监听：timed 模式下滚动自动关闭
if (typeof window !== 'undefined') {
  let scrollTimer: number | null = null
  window.addEventListener(
    'scroll',
    () => {
      if (scrollTimer) window.clearTimeout(scrollTimer)
      scrollTimer = window.setTimeout(() => {
        const timed = snackbarState.items.filter((s) => s.mode === 'timed' && !s.hiding)
        timed.forEach((s) => dismissSnackbar(s.id))
      }, 50)
    },
    true,
  )
}

export function useSnackbar() {
  return {
    snackbar: showSnackbar,
    dismiss: dismissSnackbar,
    state: readonly(snackbarState),
  }
}

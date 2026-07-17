import { reactive, readonly } from 'vue'

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
}

const toastState = reactive<{ items: ToastItem[] }>({ items: [] })
let toastSeq = 0

function dismissToast(id: number) {
  const idx = toastState.items.findIndex((t) => t.id === id)
  if (idx >= 0) {
    const item = toastState.items[idx]
    if (item.timer) window.clearTimeout(item.timer)
    toastState.items.splice(idx, 1)
  }
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
  }
  if (duration > 0) {
    item.timer = window.setTimeout(() => dismissToast(id), duration)
  }
  // 同位置最多保留 3 条，老的先移除
  const samePos = toastState.items.filter((t) => t.position === item.position)
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
}

const snackbarState = reactive<{ items: SnackbarItem[] }>({ items: [] })
let snackbarSeq = 0

function dismissSnackbar(id: number) {
  const idx = snackbarState.items.findIndex((s) => s.id === id)
  if (idx >= 0) {
    const item = snackbarState.items[idx]
    if (item.timer) window.clearTimeout(item.timer)
    snackbarState.items.splice(idx, 1)
  }
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
  }
  if (duration > 0) {
    item.timer = window.setTimeout(() => dismissSnackbar(id), duration)
  }
  // 同一时间最多显示 1 条 snackbar，新的替换旧的
  while (snackbarState.items.length >= 1) {
    const old = snackbarState.items.shift()!
    if (old.timer) window.clearTimeout(old.timer)
  }
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
        const timed = snackbarState.items.filter((s) => s.mode === 'timed')
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

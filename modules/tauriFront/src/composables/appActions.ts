/**
 * appActions —— 全局动作中枢（模块级单例）
 *
 * 用途：把「打开面板 / 打开页签」这类 App 级动作注册为键值函数，
 * 供深层组件（如 ChatHome 空态引导卡片、插件页面）解耦调用，
 * 避免把 App.vue 的函数一路 prop 传递。
 *
 * 约定：App.vue onMounted 时注册，组件销毁不注销（全局单例生命周期与应用一致）。
 */
import { ref } from 'vue'

const actions = ref<Record<string, () => void>>({})

function register(key: string, fn: () => void): void {
  actions.value[key] = fn
}

function get(key: string): (() => void) | undefined {
  return actions.value[key]
}

function run(key: string): void {
  actions.value[key]?.()
}

export interface AppActionsReturn {
  actions: typeof actions
  register: typeof register
  get: typeof get
  run: typeof run
}

export function useAppActions(): AppActionsReturn {
  return { actions, register, get, run }
}

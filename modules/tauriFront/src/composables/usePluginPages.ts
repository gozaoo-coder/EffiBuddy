/**
 * usePluginPages —— 插件页面组件注册表（模块级单例）
 *
 * 职责：把「插件页面 id」解析为 Vue 组件。
 * 插件 manifest 只声明页面元数据（id / 标题 / 图标），真正的渲染组件
 * 由前端按页面 id 在注册表中查找。当前版本：
 * - 内置页面（entry='builtin'）直接走本注册表
 * - 未来插件包内页面（entry='file'）可在此扩展为动态 import / 远程加载
 *
 * 所有已注册页面 id 在 buildPluginPageRegistry() 中集中登记，
 * 新增插件页面只需在下方 import 并加入映射即可。
 */
import { type Component, markRaw } from 'vue'
import UserTodoPage from '../components/plugin-pages/UserTodoPage.vue'

/** 页面 id → 组件映射（markRaw 避免 Vue 对组件做响应式代理，提升性能） */
const registry: Record<string, Component> = {
  // EffiSuite 内置示例插件页面：我的待办
  'effisuite/user-todo': markRaw(UserTodoPage),
}

/** 注册一个页面组件（供未来动态注册使用） */
function registerPage(pageId: string, component: Component): void {
  registry[pageId] = markRaw(component)
}

/** 按页面 id 解析组件；未注册返回 null */
function resolvePageComponent(pageId: string): Component | null {
  return registry[pageId] ?? null
}

export interface UsePluginPagesReturn {
  registry: typeof registry
  registerPage: typeof registerPage
  resolvePageComponent: typeof resolvePageComponent
}

export function usePluginPages(): UsePluginPagesReturn {
  return { registry, registerPage, resolvePageComponent }
}

/**
 * usePluginContributions —— 插件贡献注册表（模块级单例）
 *
 * 职责：从后端 `list_plugin_contributions` 拉取全部已安装插件的声明式贡献
 * （左栏按钮 / 页面 / 命令），供左栏一、页签系统、插件面板消费。
 *
 * 设计要点：
 * - 模块级状态：所有调用 usePluginContributions() 的组件共享同一份数据
 * - 懒加载 + 缓存：首次调用触发加载；plugins-changed 事件到达时自动刷新
 * - 后端命令缺失（旧版本 / 降级环境）时静默降级为空贡献，不阻断启动
 * - 贡献均为声明式数据（不执行插件代码），安全性由后端 manifest 校验保证
 */
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type {
  PluginContributionSet,
  PluginPageContribution,
  PluginRailContribution,
} from '../types'

// ============= module-level 单例状态 =============
const pluginSets = ref<PluginContributionSet[]>([])
const loading = ref(false)
const loaded = ref(false)
let unlisten: (() => void) | null = null
let installPromise: Promise<void> | null = null

/** 已注册插件列表 */
const plugins = computed(() => pluginSets.value)

/** 全部左栏按钮（合并所有插件） */
const railButtons = computed<PluginRailContribution[]>(() =>
  pluginSets.value.flatMap((p) => p.rail),
)

/** 全部页面（合并所有插件） */
const pages = computed<PluginPageContribution[]>(() =>
  pluginSets.value.flatMap((p) => p.pages),
)

/** 按页面 id 查找页面定义 */
function findPage(pageId: string): PluginPageContribution | undefined {
  return pages.value.find((p) => p.id === pageId)
}

/** 按插件 id 查找贡献集合 */
function findSet(pluginId: string): PluginContributionSet | undefined {
  return pluginSets.value.find((p) => p.pluginId === pluginId)
}

/** 从后端刷新贡献（幂等：并发调用共享同一 Promise） */
function refresh(): Promise<void> {
  if (installPromise) return installPromise
  loading.value = true
  installPromise = (async () => {
    try {
      const agg = await invoke<{ plugins: PluginContributionSet[] }>(
        'list_plugin_contributions',
      )
      pluginSets.value = agg.plugins ?? []
      loaded.value = true
    } catch (e) {
      // 后端未实现 / 出错时降级为空贡献
      console.warn('list_plugin_contributions failed', e)
      pluginSets.value = []
      loaded.value = true
    } finally {
      loading.value = false
      installPromise = null
    }
  })()
  return installPromise
}

/** 一次性安装：加载 + 订阅 plugins-changed 事件 */
function install(): Promise<void> {
  if (unlisten) return Promise.resolve()
  void refresh()
  return listen<void>('plugins-changed', () => {
    void refresh()
  }).then((fn) => {
    unlisten = fn
  })
}

export interface UsePluginContributionsReturn {
  plugins: typeof plugins
  railButtons: typeof railButtons
  pages: typeof pages
  loading: typeof loading
  loaded: typeof loaded
  refresh: typeof refresh
  install: typeof install
  findPage: typeof findPage
  findSet: typeof findSet
}

export function usePluginContributions(): UsePluginContributionsReturn {
  return {
    plugins,
    railButtons,
    pages,
    loading,
    loaded,
    refresh,
    install,
    findPage,
    findSet,
  }
}

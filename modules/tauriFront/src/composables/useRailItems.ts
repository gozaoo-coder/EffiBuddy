/**
 * useRailItems —— 左栏一（IconRail）项注册表（模块级单例）
 *
 * 职责：
 * 1. 定义内置 rail 项（聊天 / 交流池 / 模型配置 / 自动化 / 技能 / 插件 / ASR / P2P / 更多）
 * 2. 合并插件贡献的 rail 按钮（来自 usePluginContributions）
 * 3. 维护每项的「显隐」与「图标」用户偏好（localStorage 持久化）
 *
 * 设计要点（单一原子文件原则）：
 * - IconRail.vue 只做渲染与事件转发，项的数据与偏好状态全部收敛到这里
 * - 「更多」菜单里的「修改侧栏icon」入口 → IconRailSettings.vue 读写本注册表
 * - 插件项 key 形如 `plugin:<pluginId>:<itemId>`，卸载插件后自动消失
 */
import { ref, computed } from 'vue'
import type { PluginRailContribution } from '../types'

export type RailItemKind = 'view' | 'asr' | 'p2p' | 'more' | 'plugin'

export interface RailItemDef {
  /** 全局唯一 key（内置用 view 名 / 功能名；插件用 plugin: 前缀） */
  key: string
  /** 显示标签 */
  label: string
  /** 语义图标名（iconMap 命中；未命中显示首字符） */
  icon: string
  /** 所在分组：main（主区）/ bottom（底部） */
  section: 'main' | 'bottom'
  kind: RailItemKind
  /** kind=view 时的视图值（对应 RailView） */
  value?: string
  /** 插件归属插件 id */
  pluginId?: string
  /** 插件项：点击后打开的页面 id */
  pageId?: string
  /** 插件项：点击后触发的命令 id */
  command?: string
  /** 是否内置项 */
  builtin: boolean
  /** 是否固定显示（如「更多」），不可被隐藏 */
  fixed?: boolean
}

const STORAGE_KEY = 'effisuite:rail-preferences'

interface RailPrefs {
  hidden: string[]
  icons: Record<string, string>
}

function loadPrefs(): RailPrefs {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return { hidden: [], icons: {} }
    const p = JSON.parse(raw) as Partial<RailPrefs>
    return {
      hidden: Array.isArray(p.hidden) ? p.hidden : [],
      icons: p.icons && typeof p.icons === 'object' ? p.icons : {},
    }
  } catch {
    return { hidden: [], icons: {} }
  }
}

function persistPrefs(): void {
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({
        hidden: [...hiddenKeys.value],
        icons: iconOverrides.value,
      }),
    )
  } catch {
    /* ignore */
  }
}

// ============= 内置 rail 项定义 =============
export const BUILTIN_RAIL_ITEMS: RailItemDef[] = [
  { key: 'chat', label: '聊天', icon: 'chat', section: 'main', kind: 'view', value: 'chat', builtin: true },
  { key: 'pool', label: 'Agent 交流池', icon: 'merge', section: 'main', kind: 'view', value: 'pool', builtin: true },
  { key: 'model-config', label: '模型配置', icon: 'robot', section: 'main', kind: 'view', value: 'model-config', builtin: true },
  { key: 'automation', label: '自动化', icon: 'alarm', section: 'main', kind: 'view', value: 'automation', builtin: true },
  { key: 'skills', label: '技能', icon: 'bolt', section: 'main', kind: 'view', value: 'skills', builtin: true },
  { key: 'plugins', label: '插件', icon: 'puzzle', section: 'main', kind: 'view', value: 'plugins', builtin: true },
  { key: 'asr', label: '语音转写', icon: 'mic', section: 'main', kind: 'asr', builtin: true },
  { key: 'p2p', label: 'P2P 设备', icon: 'device', section: 'bottom', kind: 'p2p', builtin: true },
  { key: 'more', label: '更多', icon: 'more-horizontal', section: 'bottom', kind: 'more', builtin: true, fixed: true },
]

// ============= module-level 单例状态 =============
const prefs = loadPrefs()
const hiddenKeys = ref<Set<string>>(new Set(prefs.hidden))
const iconOverrides = ref<Record<string, string>>(prefs.icons)
const pluginItems = ref<RailItemDef[]>([])

/** 全部项 = 内置 + 插件 */
const allItems = computed<RailItemDef[]>(() => [...BUILTIN_RAIL_ITEMS, ...pluginItems.value])

/** 主区可见项 */
const mainItems = computed<RailItemDef[]>(() =>
  allItems.value.filter((i) => i.section === 'main' && isVisible(i.key)),
)
/** 底部可见项 */
const bottomItems = computed<RailItemDef[]>(() =>
  allItems.value.filter((i) => i.section === 'bottom' && isVisible(i.key)),
)
/** 设置面板可编辑项（固定项如「更多」不可隐藏但可改图标） */
const editableItems = computed<RailItemDef[]>(() => allItems.value)

/** 该项是否显示（固定项恒显示；其余看 hidden 集合） */
function isVisible(key: string): boolean {
  const item = allItems.value.find((i) => i.key === key)
  if (item?.fixed) return true
  return !hiddenKeys.value.has(key)
}

/** 设置显隐（固定项忽略） */
function setVisible(key: string, visible: boolean): void {
  const item = allItems.value.find((i) => i.key === key)
  if (item?.fixed) return
  const next = new Set(hiddenKeys.value)
  if (visible) next.delete(key)
  else next.add(key)
  hiddenKeys.value = next
  persistPrefs()
}

/** 该项当前生效图标（用户覆盖优先，回退内置/插件图标） */
function iconFor(key: string): string {
  const item = allItems.value.find((i) => i.key === key)
  return iconOverrides.value[key] || item?.icon || ''
}

/** 设置该项图标（空串表示恢复默认） */
function setIcon(key: string, icon: string): void {
  const next = { ...iconOverrides.value }
  if (!icon) delete next[key]
  else next[key] = icon
  iconOverrides.value = next
  persistPrefs()
}

/** 重置某用户的显隐/图标偏好 */
function resetItemPrefs(): void {
  hiddenKeys.value = new Set()
  iconOverrides.value = {}
  persistPrefs()
}

/**
 * 合并插件 rail 贡献为 RailItemDef 列表（每次插件贡献刷新后调用）。
 * 保留用户对已有项的显隐/图标偏好（以 key 匹配）。
 */
function setPluginContributions(contributions: PluginRailContribution[]): void {
  pluginItems.value = contributions.map((c) => ({
    key: `plugin:${c.id}`,
    label: c.label,
    icon: c.icon,
    section: c.section === 'bottom' ? 'bottom' : 'main',
    kind: 'plugin',
    pluginId: c.id,
    pageId: c.action?.type === 'open-page' ? c.action.pageId : undefined,
    command: c.action?.type === 'command' ? c.action.command : undefined,
    builtin: false,
  }))
}

export interface UseRailItemsReturn {
  allItems: typeof allItems
  mainItems: typeof mainItems
  bottomItems: typeof bottomItems
  editableItems: typeof editableItems
  isVisible: typeof isVisible
  setVisible: typeof setVisible
  iconFor: typeof iconFor
  setIcon: typeof setIcon
  resetItemPrefs: typeof resetItemPrefs
  setPluginContributions: typeof setPluginContributions
}

export function useRailItems(): UseRailItemsReturn {
  return {
    allItems,
    mainItems,
    bottomItems,
    editableItems,
    isVisible,
    setVisible,
    iconFor,
    setIcon,
    resetItemPrefs,
    setPluginContributions,
  }
}

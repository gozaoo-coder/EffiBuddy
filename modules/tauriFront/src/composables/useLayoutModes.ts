/**
 * useLayoutModes —— 左栏模态全局状态（模块级单例）
 *
 * 职责：
 * - 左栏一（IconRail）：`icon`（纯图标窄栏）| `icon-text`（图标+文字宽栏）
 * - 左栏二（HistoryRail）：`expanded`（展开完整列表）| `collapsed`（收起为窄条）
 *
 * 持久化：localStorage（键 `effisuite:layout-modes`），启动时同步、变更时落盘。
 * 顶栏 TitleBar 的模态切换按钮、IconRail 的「修改侧栏icon」设置都读写这里，
 * 保证全局唯一数据源、跨组件实时响应。
 */
import { ref, watch } from 'vue'

export type Rail1Mode = 'icon' | 'icon-text'
export type Rail2Mode = 'expanded' | 'collapsed'

export interface LayoutModes {
  rail1: Rail1Mode
  rail2: Rail2Mode
}

const STORAGE_KEY = 'effisuite:layout-modes'

/** 默认模态：左栏一纯图标、左栏二展开 */
const DEFAULT_MODES: LayoutModes = { rail1: 'icon', rail2: 'expanded' }

function loadModes(): LayoutModes {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return { ...DEFAULT_MODES }
    const parsed = JSON.parse(raw) as Partial<LayoutModes>
    return {
      rail1: parsed.rail1 === 'icon-text' ? 'icon-text' : 'icon',
      rail2: parsed.rail2 === 'collapsed' ? 'collapsed' : 'expanded',
    }
  } catch {
    return { ...DEFAULT_MODES }
  }
}

// ============= module-level 单例状态 =============
const modes = ref<LayoutModes>(loadModes())

// 变更时持久化
watch(
  modes,
  (m) => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(m))
    } catch {
      /* localStorage 不可用时忽略 */
    }
  },
  { deep: true },
)

/** 切换左栏一模态（icon ↔ icon-text） */
function toggleRail1Mode(): void {
  modes.value.rail1 = modes.value.rail1 === 'icon' ? 'icon-text' : 'icon'
}

/** 切换左栏二模态（expanded ↔ collapsed） */
function toggleRail2Mode(): void {
  modes.value.rail2 = modes.value.rail2 === 'expanded' ? 'collapsed' : 'expanded'
}

export interface UseLayoutModesReturn {
  modes: typeof modes
  toggleRail1Mode: typeof toggleRail1Mode
  toggleRail2Mode: typeof toggleRail2Mode
}

export function useLayoutModes(): UseLayoutModesReturn {
  return { modes, toggleRail1Mode, toggleRail2Mode }
}

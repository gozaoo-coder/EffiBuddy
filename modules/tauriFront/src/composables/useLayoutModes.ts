/**
 * useLayoutModes —— 左栏模态全局状态（模块级单例）
 *
 * 职责：
 * - 左栏一（IconRail）：`icon`（纯图标窄栏）| `icon-text`（图标+文字宽栏）| `hidden`（隐藏）
 * - 左栏二（HistoryRail）：`expanded`（展开完整列表）| `hidden`（隐藏）
 *
 * 持久化：localStorage（键 `effisuite:layout-modes`），启动时同步、变更时落盘。
 * 顶栏 TitleBar 的模态切换按钮、IconRail 的「修改侧栏icon」设置都读写这里，
 * 保证全局唯一数据源、跨组件实时响应。
 */
import { ref, watch } from 'vue'

export type Rail1Mode = 'icon' | 'icon-text' | 'hidden'
export type Rail2Mode = 'expanded' | 'hidden'

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
      rail1:
        parsed.rail1 === 'icon-text' ? 'icon-text' : parsed.rail1 === 'hidden' ? 'hidden' : 'icon',
      rail2: parsed.rail2 === 'hidden' ? 'hidden' : 'expanded',
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

/** 左栏一循环顺序：icon → icon-text → hidden → icon */
const RAIL1_CYCLE: Rail1Mode[] = ['icon', 'icon-text', 'hidden']
/** 左栏二循环顺序：expanded → hidden → expanded */
const RAIL2_CYCLE: Rail2Mode[] = ['expanded', 'hidden']

/** 切换左栏一模态（icon → icon-text → hidden 循环） */
function toggleRail1Mode(): void {
  const i = RAIL1_CYCLE.indexOf(modes.value.rail1)
  modes.value.rail1 = RAIL1_CYCLE[(i + 1) % RAIL1_CYCLE.length]
}

/** 切换左栏二模态（expanded ↔ hidden 循环） */
function toggleRail2Mode(): void {
  const i = RAIL2_CYCLE.indexOf(modes.value.rail2)
  modes.value.rail2 = RAIL2_CYCLE[(i + 1) % RAIL2_CYCLE.length]
}

export interface UseLayoutModesReturn {
  modes: typeof modes
  toggleRail1Mode: typeof toggleRail1Mode
  toggleRail2Mode: typeof toggleRail2Mode
}

export function useLayoutModes(): UseLayoutModesReturn {
  return { modes, toggleRail1Mode, toggleRail2Mode }
}

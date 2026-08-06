/**
 * useLayoutModes —— 布局模态全局状态（模块级单例）
 *
 * 2026-08 重构：左栏一（IconRail）已重定义为功能菜单（FeatureMenu，由 topbar 左侧
 * 第一个按钮点击弹出），不再作为常驻布局栏占用空间，故 rail1 模态（icon / icon-text /
 * hidden）整体移除；仅保留左栏二（HistoryRail / SecondRailHost）：
 * - `expanded`（展开完整列表）| `hidden`（隐藏）
 *
 * 持久化：localStorage（键 `effisuite:layout-modes`），启动时同步、变更时落盘。
 * 兼容读取旧数据（忽略已废弃的 rail1 字段）。顶栏 TitleBar 的左栏二切换按钮读写这里，
 * 保证全局唯一数据源、跨组件实时响应。
 */
import { ref, watch } from 'vue'

export type Rail2Mode = 'expanded' | 'hidden'

export interface LayoutModes {
  rail2: Rail2Mode
}

const STORAGE_KEY = 'effisuite:layout-modes'

/** 默认模态：左栏二展开 */
const DEFAULT_MODES: LayoutModes = { rail2: 'expanded' }

function loadModes(): LayoutModes {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return { ...DEFAULT_MODES }
    const parsed = JSON.parse(raw) as Partial<LayoutModes>
    return {
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

/** 左栏二循环顺序：expanded → hidden → expanded */
const RAIL2_CYCLE: Rail2Mode[] = ['expanded', 'hidden']

/** 切换左栏二模态（expanded ↔ hidden 循环） */
function toggleRail2Mode(): void {
  const i = RAIL2_CYCLE.indexOf(modes.value.rail2)
  modes.value.rail2 = RAIL2_CYCLE[(i + 1) % RAIL2_CYCLE.length]
}

export interface UseLayoutModesReturn {
  modes: typeof modes
  toggleRail2Mode: typeof toggleRail2Mode
}

export function useLayoutModes(): UseLayoutModesReturn {
  return { modes, toggleRail2Mode }
}

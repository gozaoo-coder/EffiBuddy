/**
 * Widget 桌面小组件类型定义
 *
 * 全尺寸枚举 + 堆叠 / 拖拽 / 布局系统
 * 堆叠交互模型参照 vue-bits Stack（循环堆叠：拖拽/点击将卡片送回底部）
 */

/** 卡片尺寸枚举（网格列x行，4px 基线对齐） */
export enum WidgetSize {
  /** 1×1 极小（图标+单行文字） */
  TINY = 'tiny',
  /** 1×2 小（短信息卡片） */
  SMALL = 'small',
  /** 2×2 中（默认大小） */
  MEDIUM = 'medium',
  /** 2×3 大（带图表或列表） */
  LARGE = 'large',
  /** 3×3 超大（仪表盘卡片） */
  XLARGE = 'xlarge',
  /** 3×1 宽横幅（广告/提示条） */
  WIDE = 'wide',
  /** 1×3 高条（纵向状态列表） */
  TALL = 'tall',
  /** 4×4 全尺寸（占据整个视图） */
  FULL = 'full',
}

/** 尺寸映射表：每档对应的 grid-column / grid-row span */
export const WIDGET_SIZE_MAP: Record<WidgetSize, { cols: number; rows: number }> = {
  [WidgetSize.TINY]: { cols: 1, rows: 1 },
  [WidgetSize.SMALL]: { cols: 1, rows: 2 },
  [WidgetSize.MEDIUM]: { cols: 2, rows: 2 },
  [WidgetSize.LARGE]: { cols: 2, rows: 3 },
  [WidgetSize.XLARGE]: { cols: 3, rows: 3 },
  [WidgetSize.WIDE]: { cols: 3, rows: 1 },
  [WidgetSize.TALL]: { cols: 1, rows: 3 },
  [WidgetSize.FULL]: { cols: 4, rows: 4 },
}

/** 尺寸映射表：每档对应的物理像素尺寸（堆叠模式/独立卡片使用） */
export const WIDGET_SIZE_PX: Record<WidgetSize, { width: number; height: number }> = {
  [WidgetSize.TINY]: { width: 120, height: 120 },
  [WidgetSize.SMALL]: { width: 120, height: 200 },
  [WidgetSize.MEDIUM]: { width: 220, height: 220 },
  [WidgetSize.LARGE]: { width: 220, height: 300 },
  [WidgetSize.XLARGE]: { width: 300, height: 300 },
  [WidgetSize.WIDE]: { width: 300, height: 130 },
  [WidgetSize.TALL]: { width: 130, height: 300 },
  [WidgetSize.FULL]: { width: 400, height: 400 },
}

/** 拖拽状态 */
export type DragStatus = 'idle' | 'dragging' | 'dismissing' | 'returning'

/** 卡片位置 */
export interface WidgetPosition {
  /** 网格列（1-indexed） */
  col: number
  /** 网格行（1-indexed） */
  row: number
}

/** 单个小组件项 */
export interface WidgetItem {
  /** 唯一 id */
  id: string
  /** 标题 */
  title: string
  /** 尺寸 */
  size: WidgetSize
  /** 在桌面网格中的位置 */
  position: WidgetPosition
  /** 内容组件名（动态渲染用） */
  component: string
  /** 传入内容的 props */
  props?: Record<string, unknown>
  /** 是否可关闭 */
  closable?: boolean
  /** 自定义样式 class */
  class?: string
}

/** 堆叠层配置 */
export interface StackLayer {
  /** 偏移量百分比（相对于父容器宽高） */
  offsetX: number
  offsetY: number
  /** 缩放比 */
  scale: number
  /** 旋转角度（deg） */
  rotate: number
  /** 透明度 */
  opacity: number
  /** z-index 偏移 */
  zOffset: number
}

/** 堆叠预设 */
export const STACK_PRESETS: Record<string, StackLayer[]> = {
  /** 经典扇形堆叠：每层右偏 + 下偏 + 微缩放 */
  fan: [
    { offsetX: 0, offsetY: 0, scale: 1, rotate: 0, opacity: 1, zOffset: 4 },
    { offsetX: 6, offsetY: 4, scale: 0.97, rotate: 0.5, opacity: 0.92, zOffset: 3 },
    { offsetX: 12, offsetY: 8, scale: 0.94, rotate: 1, opacity: 0.84, zOffset: 2 },
    { offsetX: 18, offsetY: 12, scale: 0.91, rotate: 1.5, opacity: 0.76, zOffset: 1 },
  ],
  /** 瀑布堆叠：仅向下偏移 */
  cascade: [
    { offsetX: 0, offsetY: 0, scale: 1, rotate: 0, opacity: 1, zOffset: 4 },
    { offsetX: 0, offsetY: 8, scale: 0.96, rotate: 0, opacity: 0.88, zOffset: 3 },
    { offsetX: 0, offsetY: 16, scale: 0.92, rotate: 0, opacity: 0.76, zOffset: 2 },
    { offsetX: 0, offsetY: 24, scale: 0.88, rotate: 0, opacity: 0.64, zOffset: 1 },
  ],
  /** 水平偏移：仅向右 */
  horizontal: [
    { offsetX: 0, offsetY: 0, scale: 1, rotate: 0, opacity: 1, zOffset: 4 },
    { offsetX: 8, offsetY: 0, scale: 0.96, rotate: 0, opacity: 0.88, zOffset: 3 },
    { offsetX: 16, offsetY: 0, scale: 0.92, rotate: 0, opacity: 0.76, zOffset: 2 },
    { offsetX: 24, offsetY: 0, scale: 0.88, rotate: 0, opacity: 0.64, zOffset: 1 },
  ],
}

/** 拖拽配置（所有字段均有默认值，详见 DEFAULT_DRAG_CONFIG） */
export interface DragConfig {
  /** 是否启用拖拽 */
  enabled: boolean
  /** 触发拖拽的鼠标按键（0=左键，2=右键） */
  button: number
  /** 拖拽开始前需要按住的像素阈值 */
  threshold: number
  /** 轻扫判定速度（px/ms） */
  flickVelocity: number
  /** 轻扫判定距离 */
  flickDistance: number
  /** 拖拽区域限制（'parent' | 'viewport' | 'none'） */
  bound: 'parent' | 'viewport' | 'none'
  /** 是否启用吸附对齐 */
  snap: boolean
  /** 吸附网格大小（px） */
  snapSize: number
  /** 弹性系数（0=硬边界，0.6=强弹性，越界时超出部分按 (1-elastic) 跟随） */
  elastic: number
}

/** 默认拖拽配置 */
export const DEFAULT_DRAG_CONFIG: DragConfig = {
  enabled: true,
  button: 0,
  threshold: 4,
  flickVelocity: 0.5,
  flickDistance: 100,
  bound: 'parent',
  snap: false,
  snapSize: 20,
  elastic: 0.6,
}

/** 堆叠事件 */
export type StackEventType =
  | 'card-selected'
  | 'card-dismissed'
  | 'stack-reordered'
  | 'card-returned'
  | 'card-send-back'
  | 'card-drag-start'
  | 'card-drag-end'
  | 'autoplay'

export interface StackEvent {
  type: StackEventType
  cardId: string
  index?: number
}

/**
 * 堆叠交互配置（参照 vue-bits Stack）
 *
 * 核心交互：循环堆叠 —— 拖拽 / 点击将顶层卡片「送回底部」，下一张卡片成为顶层
 */
export interface StackInteractionOptions {
  /** 拖拽灵敏度阈值（px）：拖拽偏移超过该值则送回底部，否则回弹复位 */
  sensitivity?: number
  /** 每张卡片随机旋转角度（±deg） */
  randomRotation?: boolean
  /** 点击卡片时将其送回底部（vue-bits 行为） */
  sendToBackOnClick?: boolean
  /** 交互模式：send-to-back 送回底部（vue-bits）/ bring-to-front 提到顶层（原版） */
  interactionMode?: 'send-to-back' | 'bring-to-front'
  /** 自动轮播：定时将顶层卡片送回底部 */
  autoplay?: boolean
  /** 自动轮播间隔（ms） */
  autoplayDelay?: number
  /** 悬停时暂停自动轮播 */
  pauseOnHover?: boolean
  /** 弹簧动画刚度（越大越硬，默认 260） */
  stiffness?: number
  /** 弹簧动画阻尼（越大衰减越快，默认 20） */
  damping?: number
  /** 拖拽弹性系数（0=无弹性，0.6=强弹性，拖出边界时生效） */
  dragElastic?: number
  /** 3D 倾斜强度（deg，拖拽时卡片跟随旋转，默认 60） */
  tiltAmount?: number
  /** 是否只允许顶层卡片拖拽 */
  topOnlyDraggable?: boolean
  /** 是否循环堆叠（送回底部后不移除卡片） */
  loop?: boolean
}

/** 默认堆叠交互配置 */
export const DEFAULT_STACK_INTERACTION: Required<StackInteractionOptions> = {
  sensitivity: 200,
  randomRotation: false,
  sendToBackOnClick: false,
  interactionMode: 'send-to-back',
  autoplay: false,
  autoplayDelay: 3000,
  pauseOnHover: false,
  stiffness: 260,
  damping: 20,
  dragElastic: 0.6,
  tiltAmount: 60,
  topOnlyDraggable: true,
  loop: true,
}
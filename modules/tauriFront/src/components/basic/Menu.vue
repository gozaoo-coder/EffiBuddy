<script setup lang="ts">
/**
 * Menu 菜单组件
 * 临时性弹出窗口，展示可执行操作
 * 现代玻璃质感设计风格
 *
 * 支持类型：
 * - 普通菜单：纯文本项
 * - 带图标菜单：每项有 icon
 * - 带标题菜单：顶部有标题栏
 * - 多级菜单：项有 children，支持原地展开（inline）和层叠（overlay）两种模式
 * - 上下文菜单：右键触发（搭配 useContextMenu）
 * - 指向型菜单：带箭头指向触发元素
 */
import { ref, computed, watch, nextTick, onUnmounted } from 'vue'
import { useAnimeTransition } from '../../composables/useAnimeTransition'
import Icon from '../Icon.vue'

/** 菜单项配置 */
export interface MenuItemOption {
  /** 唯一标识 */
  key: string
  /** 显示文本 */
  label: string
  /** 图标（emoji 或字符） */
  icon?: string
  /** 禁用 */
  disabled?: boolean
  /** 选中状态，显示 ✓ */
  selected?: boolean
  /** 项之间显示分隔线 */
  divided?: boolean
  /** 数字/文本角标（如待办计数，显示在项右侧） */
  badge?: number | string
  /** 危险项，红色文字 */
  danger?: boolean
  /** 子菜单 */
  children?: MenuItemOption[]
}

/** 弹出位置 */
export type MenuPlacement =
  | 'top-start'
  | 'top'
  | 'top-end'
  | 'bottom-start'
  | 'bottom'
  | 'bottom-end'
  | 'left-start'
  | 'left'
  | 'left-end'
  | 'right-start'
  | 'right'
  | 'right-end'

/** 子菜单展开方式 */
export type MenuSubMenuMode = 'inline' | 'overlay'

const props = withDefaults(
  defineProps<{
    /** 是否显示（v-model） */
    visible?: boolean
    /** 菜单项列表 */
    items: MenuItemOption[]
    /** 菜单标题 */
    title?: string
    /** 弹出位置，默认 bottom-start */
    placement?: MenuPlacement
    /** 子菜单展开方式：inline 原地展开 / overlay 层叠展开 */
    subMenuMode?: 'inline' | 'overlay'
    /** 是否显示指向箭头 */
    arrow?: boolean
    /** 菜单最小宽度 px，默认 160 */
    minWidth?: number
    /** 触发元素引用，用于定位；若不提供则使用 position 坐标定位 */
    triggerRef?: HTMLElement | null
    /** 上下文菜单坐标（来自 contextmenu 事件）；优先于 triggerRef */
    position?: { x: number; y: number } | null
    /** 定位偏移量（px）：在自动定位结果上叠加（如贴齐侧栏边缘） */
    positionOffset?: { x?: number; y?: number }
    /** 未选中项隐藏 ✓ 占位：仅选中项渲染 menu-item-check */
    hideCheckWhenUnselected?: boolean
  }>(),
  {
    visible: undefined,
    title: '',
    placement: 'bottom-start',
    subMenuMode: 'overlay',
    arrow: false,
    minWidth: 160,
    triggerRef: null,
    position: null,
    positionOffset: undefined,
    hideCheckWhenUnselected: true,
  },
)

const emit = defineEmits<{
  (e: 'update:visible', v: boolean): void
  (e: 'select', item: MenuItemOption): void
}>()

// 内部 visible 状态：支持 v-model 与非受控两种模式
const innerVisible = ref(props.visible ?? false)
watch(
  () => props.visible,
  (v) => {
    if (v !== undefined) innerVisible.value = v
  },
)

function setVisible(v: boolean) {
  innerVisible.value = v
  emit('update:visible', v)
}

// 浮层元素引用
const panelEl = ref<HTMLElement | null>(null)
// 浮层定位样式
const panelStyle = ref<Record<string, string>>({})
// 箭头定位样式
const arrowStyle = ref<Record<string, string>>({})
// 实际生效的 placement（边界翻转后可能与传入不同）
const realPlacement = ref<MenuPlacement>(props.placement)

// inline 模式下展开的子菜单 key 集合
const expandedKeys = ref<Set<string>>(new Set())
// overlay 模式下 hover 中的项 key（用于高亮 + 子菜单面板定位）
const hoverKey = ref<string | null>(null)
// overlay 模式下子菜单面板定位样式
const subPanelStyle = ref<Record<string, string>>({})

// 键盘导航：当前焦点索引
const focusIndex = ref(-1)
// 菜单项列表（含 inline 展开的子项）
const flatItems = computed(() => {
  const list: { item: MenuItemOption; depth: number; parentKey?: string }[] = []
  for (const item of props.items) {
    list.push({ item, depth: 0 })
    if (props.subMenuMode === 'inline' && expandedKeys.value.has(item.key) && item.children) {
      for (const child of item.children) {
        list.push({ item: child, depth: 1, parentKey: item.key })
      }
    }
  }
  return list
})

// 主面板与子菜单面板共用进入/离开动画：淡入 + scale(.92)→1 + 向上位移
const { onEnter: onMenuEnter, onLeave: onMenuLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    transform: ['translateY(4px) scale(.92)', 'translateY(0) scale(1)'],
    duration: 200,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    transform: ['translateY(0) scale(1)', 'translateY(-2px) scale(.96)'],
    duration: 160,
    ease: 'in(2)',
  },
})

// 解析 placement 为 side + align
function parsePlacement(p: MenuPlacement): { side: 'top' | 'bottom' | 'left' | 'right'; align: 'start' | 'center' | 'end' } {
  const [side, align] = p.split('-') as ['top' | 'bottom' | 'left' | 'right', 'start' | 'center' | 'end' | undefined]
  return { side, align: (align ?? 'center') as 'start' | 'center' | 'end' }
}

// 计算浮层定位（基于 trigger 的 getBoundingClientRect 或 position 坐标）
async function updatePosition() {
  if (!innerVisible.value) return
  await nextTick()
  if (!panelEl.value) return

  // 锚点矩形：trigger 元素或 position 坐标（视为 1x1 的点）
  let anchor: DOMRect
  if (props.position) {
    const { x, y } = props.position
    anchor = {
      x, y, left: x, top: y, right: x, bottom: y,
      width: 1, height: 1,
      toJSON: () => ({}),
    } as DOMRect
  } else if (props.triggerRef) {
    anchor = props.triggerRef.getBoundingClientRect()
  } else {
    anchor = {
      x: window.innerWidth / 2, y: window.innerHeight / 2,
      left: window.innerWidth / 2, top: window.innerHeight / 2,
      right: window.innerWidth / 2, bottom: window.innerHeight / 2,
      width: 1, height: 1,
      toJSON: () => ({}),
    } as DOMRect
  }

  const panel = panelEl.value.getBoundingClientRect()
  const margin = 6
  const arrowSize = 8
  const vw = window.innerWidth
  const vh = window.innerHeight

  // 解析初始 placement
  let { side, align } = parsePlacement(props.placement)
  // 边界自动翻转：上下/左右
  if (side === 'bottom' && anchor.bottom + margin + panel.height > vh - 8) {
    if (anchor.top - margin - panel.height >= 8) side = 'top'
  } else if (side === 'top' && anchor.top - margin - panel.height < 8) {
    if (anchor.bottom + margin + panel.height <= vh - 8) side = 'bottom'
  } else if (side === 'right' && anchor.right + margin + panel.width > vw - 8) {
    if (anchor.left - margin - panel.width >= 8) side = 'left'
  } else if (side === 'left' && anchor.left - margin - panel.width < 8) {
    if (anchor.right + margin + panel.width <= vw - 8) side = 'right'
  }

  let top = 0
  let left = 0
  const arrow: Record<string, string> = {}

  if (side === 'top' || side === 'bottom') {
    if (align === 'start') {
      left = anchor.left
    } else if (align === 'end') {
      left = anchor.right - panel.width
    } else {
      left = anchor.left + anchor.width / 2 - panel.width / 2
    }
    if (side === 'top') {
      top = anchor.top - panel.height - margin
    } else {
      top = anchor.bottom + margin
    }
    const arrowLeft = anchor.left + anchor.width / 2 - left
    arrow.left = `${Math.max(arrowSize + 4, Math.min(panel.width - arrowSize - 4, arrowLeft))}px`
  } else {
    if (align === 'start') {
      top = anchor.top
    } else if (align === 'end') {
      top = anchor.bottom - panel.height
    } else {
      top = anchor.top + anchor.height / 2 - panel.height / 2
    }
    if (side === 'left') {
      left = anchor.left - panel.width - margin
    } else {
      left = anchor.right + margin
    }
    const arrowTop = anchor.top + anchor.height / 2 - top
    arrow.top = `${Math.max(arrowSize + 4, Math.min(panel.height - arrowSize - 4, arrowTop))}px`
  }

  // 边界适配：距屏幕边缘最小 8px
  if (left < 8) left = 8
  if (left + panel.width > vw - 8) left = vw - panel.width - 8
  if (top < 8) top = 8
  if (top + panel.height > vh - 8) top = vh - panel.height - 8

  realPlacement.value = `${side}-${align === 'center' ? '' : align}`.replace('-$', '') as MenuPlacement
  if (align === 'center') realPlacement.value = side as MenuPlacement

  const offsetX = props.positionOffset?.x ?? 0
  const offsetY = props.positionOffset?.y ?? 0
  panelStyle.value = {
    top: `${top + window.scrollY + offsetY}px`,
    left: `${left + window.scrollX + offsetX}px`,
    minWidth: `${props.minWidth}px`,
  }
  arrowStyle.value = arrow
}

// 监听 visible 变化重新计算定位
watch(innerVisible, (v) => {
  if (v) {
    expandedKeys.value = new Set()
    hoverKey.value = null
    focusIndex.value = -1
    nextTick(() => updatePosition())
  }
})

// 监听 trigger/position 变化重新计算
watch(
  () => [props.triggerRef, props.position],
  () => {
    if (innerVisible.value) updatePosition()
  },
  { deep: true },
)

// 窗口滚动/缩放时重新定位
function onScroll() {
  if (innerVisible.value) updatePosition()
}
if (typeof window !== 'undefined') {
  window.addEventListener('scroll', onScroll, true)
  window.addEventListener('resize', onScroll)
}
onUnmounted(() => {
  if (typeof window !== 'undefined') {
    window.removeEventListener('scroll', onScroll, true)
    window.removeEventListener('resize', onScroll)
  }
})

// 项点击处理
function onItemClick(item: MenuItemOption) {
  if (item.disabled) return
  if (item.children && item.children.length > 0) {
    if (props.subMenuMode === 'inline') {
      const next = new Set(expandedKeys.value)
      if (next.has(item.key)) next.delete(item.key)
      else next.add(item.key)
      expandedKeys.value = next
      nextTick(() => updatePosition())
      return
    }
    return
  }
  emit('select', item)
  setVisible(false)
}

// 子菜单隐藏延迟定时器（避免移入子菜单项时闪烁消失）
const HIDE_DELAY = 180
let hideTimer: ReturnType<typeof setTimeout> | null = null

function clearHideTimer() {
  if (hideTimer !== null) {
    clearTimeout(hideTimer)
    hideTimer = null
  }
}

function scheduleHideSubMenu() {
  clearHideTimer()
  hideTimer = setTimeout(() => {
    hoverKey.value = null
    hideTimer = null
  }, HIDE_DELAY)
}

// overlay 模式 hover 处理
function onItemEnter(item: MenuItemOption, ev: MouseEvent) {
  if (props.subMenuMode !== 'overlay') return
  clearHideTimer()

  // 有子菜单的项：立即切换显示
  if (item.children && item.children.length > 0) {
    hoverKey.value = item.key
    const target = ev.currentTarget as HTMLElement
    const rect = target.getBoundingClientRect()
    const vw = window.innerWidth
    let left = rect.right + 4
    if (left + 200 > vw - 8) {
      left = rect.left - 200 - 4
      if (left < 8) left = 8
    }
    subPanelStyle.value = {
      top: `${rect.top + window.scrollY}px`,
      left: `${left + window.scrollX}px`,
      minWidth: `${props.minWidth}px`,
    }
    return
  }

  // 无子菜单的项：不修改 hoverKey，让子面板保持打开
  // 子面板关闭由 onSubPanelLeave 的延迟定时器处理
}

function onItemLeave() {
  // overlay 模式不立即清空，由子面板的 leave 处理
}

function onSubPanelEnter() {
  // 鼠标进入子面板 → 取消任何待执行的隐藏定时器
  clearHideTimer()
}

function onSubPanelLeave() {
  // 鼠标离开子面板 → 延迟后隐藏（给用户返回父菜单的缓冲时间）
  scheduleHideSubMenu()
}

// 点击外部关闭
function onDocumentClick(e: MouseEvent) {
  if (!innerVisible.value) return
  const target = e.target as Node
  if (panelEl.value?.contains(target)) return
  if (props.triggerRef?.contains(target)) return
  setVisible(false)
}
if (typeof document !== 'undefined') {
  document.addEventListener('click', onDocumentClick)
}
onUnmounted(() => {
  if (typeof document !== 'undefined') {
    document.removeEventListener('click', onDocumentClick)
  }
})

// ESC 关闭
function onKeydown(e: KeyboardEvent) {
  if (!innerVisible.value) return

  switch (e.key) {
    case 'Escape':
      e.stopPropagation()
      if (hoverKey.value) {
        hoverKey.value = null
      } else {
        setVisible(false)
      }
      break
    case 'ArrowDown':
      e.preventDefault()
      e.stopPropagation()
      if (flatItems.value.length > 0) {
        focusIndex.value = (focusIndex.value + 1) % flatItems.value.length
        scrollToFocused()
      }
      break
    case 'ArrowUp':
      e.preventDefault()
      e.stopPropagation()
      if (flatItems.value.length > 0) {
        focusIndex.value = (focusIndex.value - 1 + flatItems.value.length) % flatItems.value.length
        scrollToFocused()
      }
      break
    case 'Enter':
    case ' ':
      e.preventDefault()
      e.stopPropagation()
      if (focusIndex.value >= 0 && focusIndex.value < flatItems.value.length) {
        const entry = flatItems.value[focusIndex.value]
        if (entry && !entry.item.disabled) {
          onItemClick(entry.item)
        }
      }
      break
  }
}
if (typeof document !== 'undefined') {
  document.addEventListener('keydown', onKeydown)
}
onUnmounted(() => {
  clearHideTimer()
  if (typeof document !== 'undefined') {
    document.removeEventListener('keydown', onKeydown)
  }
})

function scrollToFocused() {
  nextTick(() => {
    if (!panelEl.value) return
    const items = panelEl.value.querySelectorAll('.menu-item')
    const target = items[focusIndex.value] as HTMLElement | undefined
    target?.scrollIntoView({ block: 'nearest' })
  })
}

// 阻止浮层点击冒泡到 document 防止误关闭
function onPanelClick(e: MouseEvent) {
  e.stopPropagation()
}

// 阻止右键默认菜单（浮层内）
function onPanelContextmenu(e: MouseEvent) {
  e.preventDefault()
  e.stopPropagation()
}

const hasTitle = computed(() => !!props.title)

defineExpose({ updatePosition })
</script>

<script lang="ts">
/** useContextMenu：右键菜单 composable
 * 返回 { open, menuRef, onContextMenu, position }
 * 用法：
 *   const { open, menuRef, onContextMenu, position } = useContextMenu()
 *   <div @contextmenu="onContextMenu">...</div>
 *   <Menu ref="menuRef" v-model:visible="open" :items="items" :position="position" />
 */
import { ref as refInner, type Ref } from 'vue'

export function useContextMenu(): {
  open: Ref<boolean>
  menuRef: Ref<{ updatePosition: () => void } | null>
  onContextMenu: (e: MouseEvent) => void
  position: Ref<{ x: number; y: number } | null>
} {
  const open = refInner(false)
  const position = refInner<{ x: number; y: number } | null>(null)
  const menuRef = refInner<{ updatePosition: () => void } | null>(null)

  function onContextMenu(e: MouseEvent) {
    e.preventDefault()
    position.value = { x: e.clientX, y: e.clientY }
    open.value = true
  }

  return { open, menuRef, onContextMenu, position }
}
</script>

<template>
  <Teleport to="body">
    <Transition :css="false" @enter="onMenuEnter" @leave="onMenuLeave" appear>
      <div
        v-if="innerVisible"
        ref="panelEl"
        class="menu"
        :class="{
          'menu--with-title': hasTitle,
          'menu--with-arrow': arrow,
        }"
        :style="panelStyle"
        @click="onPanelClick"
        @contextmenu="onPanelContextmenu"
      >
        <!-- 指向箭头 -->
        <span
          v-if="arrow"
          class="menu-arrow"
          :class="`menu-arrow--${realPlacement.split('-')[0]}`"
          :style="arrowStyle"
        ></span>

        <!-- 标题栏 -->
        <div v-if="hasTitle" class="menu-header">
          <span class="menu-header-label">{{ title }}</span>
        </div>

        <!-- 菜单项列表 -->
        <div class="menu-list">
          <template v-for="(entry, idx) in flatItems" :key="entry.item.key">
            <div
              v-if="entry.item.divided && idx > 0"
              class="menu-divider"
            ></div>
            <button
              type="button"
              class="menu-item"
              :class="{
                'is-disabled': entry.item.disabled,
                'is-selected': entry.item.selected,
                'is-danger': entry.item.danger,
                'is-hover': hoverKey === entry.item.key && !entry.parentKey,
                'is-focused': focusIndex === idx,
                'is-child': entry.depth > 0,
                'has-children': entry.item.children && entry.item.children.length > 0 && !entry.parentKey,
                'is-expanded': expandedKeys.has(entry.item.key),
              }"
              :disabled="entry.item.disabled"
              :style="entry.depth > 0 ? { paddingLeft: '44px' } : undefined"
              @click="onItemClick(entry.item)"
              @mouseenter="onItemEnter(entry.item, $event)"
              @mouseleave="onItemLeave"
            >
              <!-- 选中标记 -->
              <span class="menu-item-indicator">
                <span v-if="entry.item.selected" class="menu-item-dot"></span>
              </span>
              <!-- 图标 -->
              <span v-if="entry.item.icon" class="menu-item-icon"><Icon :name="entry.item.icon" :size="16" /></span>
              <!-- 文本 -->
              <span class="menu-item-label">{{ entry.item.label }}</span>
              <!-- 快捷键提示（预留） -->
              <!-- 数字/文本角标 -->
              <span
                v-if="entry.item.badge !== undefined && entry.item.badge !== '' && entry.item.badge !== 0"
                class="menu-item-badge"
              >{{ entry.item.badge }}</span>
              <!-- 子菜单指示箭头 -->
              <span
                v-if="entry.item.children && entry.item.children.length > 0 && !entry.parentKey"
                class="menu-item-arrow"
                :class="{ 'is-expanded': expandedKeys.has(entry.item.key) }"
              >
                <Icon name="chevron-right" :size="12" />
              </span>
            </button>
          </template>
        </div>

        <!-- overlay 模式：层叠子菜单面板 -->
        <Teleport to="body">
          <Transition :css="false" @enter="onMenuEnter" @leave="onMenuLeave" appear>
            <div
              v-if="subMenuMode === 'overlay' && hoverKey"
              class="menu menu--sub"
              :style="subPanelStyle"
              @mouseenter="onSubPanelEnter"
              @mouseleave="onSubPanelLeave"
              @click="onPanelClick"
            >
              <div class="menu-list">
                <template v-for="(child, cidx) in items.find((i) => i.key === hoverKey)?.children ?? []" :key="child.key">
                  <div
                    v-if="child.divided && cidx > 0"
                    class="menu-divider"
                  ></div>
                  <button
                    type="button"
                    class="menu-item"
                    :class="{
                      'is-disabled': child.disabled,
                      'is-selected': child.selected,
                      'is-danger': child.danger,
                    }"
                    :disabled="child.disabled"
                    @click="onItemClick(child)"
                    @mouseenter="onItemEnter(child, $event)"
                  >
                    <span class="menu-item-indicator">
                      <span v-if="child.selected" class="menu-item-dot"></span>
                    </span>
                    <span v-if="child.icon" class="menu-item-icon"><Icon :name="child.icon" :size="16" /></span>
                    <span class="menu-item-label">{{ child.label }}</span>
                    <span
                      v-if="child.badge !== undefined && child.badge !== '' && child.badge !== 0"
                      class="menu-item-badge"
                    >{{ child.badge }}</span>
                  </button>
                </template>
              </div>
            </div>
          </Transition>
        </Teleport>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
/* ============================================================
 * Menu 菜单组件样式
 * 现代玻璃质感设计风格
 * 参考 macOS / HarmonyOS NEXT 设计语言
 * ============================================================ */

/* ---------- 浮层面板 ---------- */
.menu {
  position: absolute;
  z-index: var(--z-menu);
  min-width: 160px;
  max-width: 320px;
  padding: 6px;
  background: color-mix(in srgb, var(--card-2) 85%, transparent);
  backdrop-filter: blur(24px) saturate(1.2);
  -webkit-backdrop-filter: blur(24px) saturate(1.2);
  border: 1px solid color-mix(in srgb, var(--border-strong) 80%, transparent);
  border-radius: 10px;
  box-shadow:
    0 0 0 0.5px color-mix(in srgb, var(--border-strong) 30%, transparent),
    0 8px 32px rgba(0, 0, 0, 0.2),
    0 2px 8px rgba(0, 0, 0, 0.08);
  overflow: visible;
  display: flex;
  flex-direction: column;
  transform-origin: var(--menu-origin, top center);
}

[data-theme='light'] .menu {
  background: color-mix(in srgb, var(--card-2) 92%, transparent);
  backdrop-filter: blur(20px) saturate(1.1);
  -webkit-backdrop-filter: blur(20px) saturate(1.1);
  border-color: color-mix(in srgb, var(--border-strong) 70%, transparent);
  box-shadow:
    0 0 0 0.5px color-mix(in srgb, var(--border-strong) 20%, transparent),
    0 8px 32px rgba(0, 0, 0, 0.08),
    0 2px 8px rgba(0, 0, 0, 0.04);
}

.menu--sub {
  z-index: calc(var(--z-menu) + 1);
}

.menu--with-title {
  padding-top: 0;
}

/* ---------- 标题栏 ---------- */
.menu-header {
  display: flex;
  align-items: center;
  padding: 8px 10px 4px;
  margin-bottom: 2px;
  border-bottom: 1px solid var(--border);
}

.menu-header-label {
  font-size: 11px;
  font-weight: 600;
  color: var(--muted);
  text-transform: uppercase;
  letter-spacing: 0.6px;
  user-select: none;
}

/* ---------- 列表容器 ---------- */
.menu-list {
  display: flex;
  flex-direction: column;
  gap: 1px;
  max-height: 320px;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 2px 0;
}

.menu-list::-webkit-scrollbar {
  width: 4px;
}

.menu-list::-webkit-scrollbar-track {
  background: transparent;
}

.menu-list::-webkit-scrollbar-thumb {
  background: color-mix(in srgb, var(--muted) 25%, transparent);
  border-radius: 2px;
}

.menu-list::-webkit-scrollbar-thumb:hover {
  background: color-mix(in srgb, var(--muted) 40%, transparent);
}

/* ---------- 分隔线 ---------- */
.menu-divider {
  height: 1px;
  margin: 3px 8px;
  background: linear-gradient(
    to right,
    transparent,
    color-mix(in srgb, var(--border-strong) 60%, transparent) 20%,
    color-mix(in srgb, var(--border-strong) 60%, transparent) 80%,
    transparent
  );
  flex-shrink: 0;
}

/* ---------- 菜单项 ---------- */
.menu-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 8px;
  height: 32px;
  padding: 0 10px;
  border: none;
  background: transparent;
  color: var(--text);
  font-family: inherit;
  font-size: 13px;
  font-weight: 450;
  line-height: 1;
  text-align: left;
  white-space: nowrap;
  cursor: pointer;
  outline: none;
  border-radius: 6px;
  transition:
    background 0.12s ease,
    transform 0.1s ease;
  user-select: none;
}

/* 子项缩进 */
.menu-item.is-child {
  font-size: 12.5px;
}

/* hover 状态 */
.menu-item:not(.is-disabled):hover,
.menu-item.is-hover {
  background: color-mix(in srgb, var(--text) 8%, transparent);
}

[data-theme='light'] .menu-item:not(.is-disabled):hover,
[data-theme='light'] .menu-item.is-hover {
  background: color-mix(in srgb, var(--text) 6%, transparent);
}

/* 键盘焦点状态 */
.menu-item.is-focused:not(.is-disabled) {
  background: color-mix(in srgb, var(--text) 8%, transparent);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--primary) 40%, transparent);
}

/* 选中状态：左侧圆点指示器 */
.menu-item.is-selected {
  color: var(--primary);
}

/* 危险项 */
.menu-item.is-danger {
  color: var(--danger);
}

.menu-item.is-danger:not(.is-disabled):hover,
.menu-item.is-danger.is-hover {
  background: color-mix(in srgb, var(--danger) 10%, transparent);
}

/* 禁用 */
.menu-item.is-disabled {
  cursor: not-allowed;
  opacity: 0.4;
}

/* active 按下效果 */
.menu-item:not(.is-disabled):active {
  transform: scale(0.97);
  background: color-mix(in srgb, var(--text) 12%, transparent);
}

/* ---------- 选中指示器 ---------- */
.menu-item-indicator {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  flex-shrink: 0;
}

.menu-item-dot {
  display: block;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--primary);
  animation: menu-dot-enter 0.2s cubic-bezier(0.3, 0, 0, 1);
}

@keyframes menu-dot-enter {
    from {
      transform: scale(0.5);
      opacity: 0;
    }
    to {
      transform: scale(1);
      opacity: 1;
    }
  }

.menu-item.is-danger .menu-item-dot {
  background: var(--danger);
}

/* ---------- 图标 ---------- */
.menu-item-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  flex-shrink: 0;
  color: var(--muted);
  transition: color 0.12s ease;
}

.menu-item:not(.is-disabled):hover .menu-item-icon,
.menu-item.is-hover .menu-item-icon {
  color: var(--text);
}

.menu-item.is-selected .menu-item-icon {
  color: var(--primary);
}

.menu-item.is-danger .menu-item-icon {
  color: var(--danger);
}

/* ---------- 文本 ---------- */
.menu-item-label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* ---------- 角标 ---------- */
.menu-item-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 18px;
  height: 18px;
  padding: 0 5px;
  border-radius: 9px;
  background: color-mix(in srgb, var(--primary) 90%, white);
  color: #fff;
  font-size: 10px;
  font-weight: 600;
  line-height: 1;
  flex-shrink: 0;
  letter-spacing: 0.2px;
}

.menu-item.is-danger .menu-item-badge {
  background: var(--danger);
}

/* ---------- 子菜单指示箭头 ---------- */
.menu-item-arrow {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  color: var(--muted);
  flex-shrink: 0;
  transition: transform 0.15s ease, color 0.15s ease;
}

.menu-item-arrow.is-expanded {
  transform: rotate(90deg);
  color: var(--primary);
}

.menu-item:not(.is-disabled):hover .menu-item-arrow {
  color: var(--text);
}

/* ---------- 指向箭头（指向触发元素） ---------- */
.menu-arrow {
  position: absolute;
  width: 10px;
  height: 10px;
  background: color-mix(in srgb, var(--card-2) 90%, transparent);
  border: 1px solid color-mix(in srgb, var(--border-strong) 60%, transparent);
  z-index: -1;
  backdrop-filter: blur(24px);
  -webkit-backdrop-filter: blur(24px);
}

.menu-arrow--top {
  bottom: -6px;
  transform: rotate(225deg);
  border-top: none;
  border-left: none;
  clip-path: polygon(0 0, 100% 100%, 0 100%);
}

.menu-arrow--bottom {
  top: -6px;
  transform: rotate(45deg);
  border-bottom: none;
  border-right: none;
  clip-path: polygon(0 0, 100% 0, 100% 100%);
}

.menu-arrow--left {
  right: -6px;
  transform: rotate(135deg);
  border-top: none;
  border-right: none;
  clip-path: polygon(0 0, 100% 0, 100% 100%);
}

.menu-arrow--right {
  left: -6px;
  transform: rotate(315deg);
  border-bottom: none;
  border-left: none;
  clip-path: polygon(0 0, 0 100%, 100% 100%);
}
</style>
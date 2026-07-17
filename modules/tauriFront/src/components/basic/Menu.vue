<script setup lang="ts">
/**
 * Menu 菜单组件
 * 临时性弹出窗口，展示可执行操作
 * 参考 HarmonyOS NEXT 设计规范
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

// 主面板与子菜单面板共用进入/离开动画：淡入 + scale(.95)→1（保持原 CSS 行为）
const { onEnter: onMenuEnter, onLeave: onMenuLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    transform: ['scale(.95)', 'scale(1)'],
    duration: 180,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    transform: ['scale(1)', 'scale(.95)'],
    duration: 150,
    ease: 'inOut(2)',
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
      x,
      y,
      left: x,
      top: y,
      right: x,
      bottom: y,
      width: 1,
      height: 1,
      toJSON: () => ({}),
    } as DOMRect
  } else if (props.triggerRef) {
    anchor = props.triggerRef.getBoundingClientRect()
  } else {
    // 既没有 trigger 也没有 position，居中显示
    anchor = {
      x: window.innerWidth / 2,
      y: window.innerHeight / 2,
      left: window.innerWidth / 2,
      top: window.innerHeight / 2,
      right: window.innerWidth / 2,
      bottom: window.innerHeight / 2,
      width: 1,
      height: 1,
      toJSON: () => ({}),
    } as DOMRect
  }

  const panel = panelEl.value.getBoundingClientRect()
  const margin = 4
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
    // 水平对齐
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
    // 箭头水平位置（跟随锚点中心）
    const arrowLeft = anchor.left + anchor.width / 2 - left
    arrow.left = `${Math.max(arrowSize + 4, Math.min(panel.width - arrowSize - 4, arrowLeft))}px`
  } else {
    // left / right
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

  panelStyle.value = {
    top: `${top + window.scrollY}px`,
    left: `${left + window.scrollX}px`,
    minWidth: `${props.minWidth}px`,
  }
  arrowStyle.value = arrow
}

// 监听 visible 变化重新计算定位
watch(innerVisible, (v) => {
  if (v) {
    // 重置内部状态
    expandedKeys.value = new Set()
    hoverKey.value = null
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
  // 有子菜单时根据模式处理
  if (item.children && item.children.length > 0) {
    if (props.subMenuMode === 'inline') {
      // inline 模式：切换展开
      const next = new Set(expandedKeys.value)
      if (next.has(item.key)) next.delete(item.key)
      else next.add(item.key)
      expandedKeys.value = next
      nextTick(() => updatePosition())
      return
    }
    // overlay 模式：点击有子菜单的项不直接关闭，等用户选子项
    return
  }
  emit('select', item)
  setVisible(false)
}

// overlay 模式 hover 处理
function onItemEnter(item: MenuItemOption, ev: MouseEvent) {
  if (props.subMenuMode !== 'overlay') return
  if (!item.children || item.children.length === 0) {
    hoverKey.value = null
    return
  }
  hoverKey.value = item.key
  // 计算子菜单面板位置（基于当前项元素的位置）
  const target = ev.currentTarget as HTMLElement
  const rect = target.getBoundingClientRect()
  const vw = window.innerWidth
  // 默认放在右侧
  let left = rect.right + 4
  // 右侧空间不足则放左侧
  if (left + 200 > vw - 8) {
    left = rect.left - 200 - 4
    if (left < 8) left = 8
  }
  subPanelStyle.value = {
    top: `${rect.top + window.scrollY}px`,
    left: `${left + window.scrollX}px`,
    minWidth: `${props.minWidth}px`,
  }
}

function onItemLeave() {
  // overlay 模式不立即清空，由子面板的 leave 处理
}

function onSubPanelLeave() {
  hoverKey.value = null
}

// 点击外部关闭
function onDocumentClick(e: MouseEvent) {
  if (!innerVisible.value) return
  const target = e.target as Node
  if (panelEl.value?.contains(target)) return
  // 检查 trigger
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
  if (e.key === 'Escape' && innerVisible.value) {
    e.stopPropagation()
    setVisible(false)
  }
}
if (typeof document !== 'undefined') {
  document.addEventListener('keydown', onKeydown)
}
onUnmounted(() => {
  if (typeof document !== 'undefined') {
    document.removeEventListener('keydown', onKeydown)
  }
})

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
        <div v-if="hasTitle" class="menu-title">{{ title }}</div>

        <!-- 菜单项列表 -->
        <div class="menu-list">
          <template v-for="item in items" :key="item.key">
            <button
              type="button"
              class="menu-item"
              :class="{
                'is-disabled': item.disabled,
                'is-selected': item.selected,
                'is-danger': item.danger,
                'is-divided': item.divided,
                'is-hover': hoverKey === item.key,
                'has-children': item.children && item.children.length > 0,
                'is-expanded': expandedKeys.has(item.key),
              }"
              :disabled="item.disabled"
              @click="onItemClick(item)"
              @mouseenter="onItemEnter(item, $event)"
              @mouseleave="onItemLeave"
            >
              <!-- 选中状态占位（左侧 ✓） -->
              <span class="menu-item-check"><Icon v-if="item.selected" name="check-builtin" :size="16" /></span>
              <!-- 图标 -->
              <span v-if="item.icon" class="menu-item-icon"><Icon :name="item.icon" :size="18" /></span>
              <!-- 文本 -->
              <span class="menu-item-label">{{ item.label }}</span>
              <!-- 子菜单指示箭头 -->
              <span
                v-if="item.children && item.children.length > 0"
                class="menu-item-arrow"
                :class="{ 'is-expanded': expandedKeys.has(item.key) }"
              ><Icon name="chevron-right" :size="12" /></span>
            </button>

            <!-- inline 模式：展开子项在原位置下方 -->
            <template v-if="subMenuMode === 'inline' && expandedKeys.has(item.key) && item.children">
              <button
                v-for="child in item.children"
                :key="child.key"
                type="button"
                class="menu-item menu-item--child"
                :class="{
                  'is-disabled': child.disabled,
                  'is-selected': child.selected,
                  'is-danger': child.danger,
                  'is-divided': child.divided,
                }"
                :disabled="child.disabled"
                @click="onItemClick(child)"
              >
                <span class="menu-item-check"><Icon v-if="child.selected" name="check-builtin" :size="16" /></span>
                <span v-if="child.icon" class="menu-item-icon"><Icon :name="child.icon" :size="18" /></span>
                <span class="menu-item-label">{{ child.label }}</span>
              </button>
            </template>
          </template>
        </div>

        <!-- overlay 模式：层叠子菜单面板 -->
        <Teleport to="body">
          <Transition :css="false" @enter="onMenuEnter" @leave="onMenuLeave" appear>
            <div
              v-if="subMenuMode === 'overlay' && hoverKey"
              class="menu menu--sub"
              :style="subPanelStyle"
              @mouseleave="onSubPanelLeave"
              @click="onPanelClick"
            >
              <div class="menu-list">
                <button
                  v-for="child in items.find((i) => i.key === hoverKey)?.children ?? []"
                  :key="child.key"
                  type="button"
                  class="menu-item"
                  :class="{
                    'is-disabled': child.disabled,
                    'is-selected': child.selected,
                    'is-danger': child.danger,
                    'is-divided': child.divided,
                  }"
                  :disabled="child.disabled"
                  @click="onItemClick(child)"
                >
                  <span class="menu-item-check"><Icon v-if="child.selected" name="check-builtin" :size="16" /></span>
                  <span v-if="child.icon" class="menu-item-icon"><Icon :name="child.icon" :size="18" /></span>
                  <span class="menu-item-label">{{ child.label }}</span>
                </button>
              </div>
            </div>
          </Transition>
        </Teleport>
      </div>
    </Transition>
  </Teleport>
</template>

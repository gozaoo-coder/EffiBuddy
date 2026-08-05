<script setup lang="ts">
/**
 * TabBar —— 多页签栏
 *
 * 动画全流程管线设计（开始 / 中断 / 打断 / 返回 / DOM 进入退出 / 父子关系）：
 *
 * 1. Tab 进入（@enter）：
 *    - 开始：新页签 push 进 tabs → TransitionGroup 触发 @enter(el, done)
 *    - 动画：width 0→natural + opacity 0→1 + translateX(-8px)→0，弹性 ease out(3)
 *    - 中断：若同一元素仍有未完成的 enter 动画（极端竞态），先 pause 旧动画再起新的
 *    - DOM：元素由 Vue 插入后立即动画；overflow:hidden 防止宽度收缩时内容溢出；
 *           onComplete 清除全部内联样式（width/opacity/transform/overflow）→ done()
 *
 * 2. Tab 退出（@leave）：
 *    - 开始：页签从 tabs 移除 → TransitionGroup 触发 @leave(el, done)
 *    - 中断/打断：若该元素正处于 enter 动画中（刚进入即被关闭），先 pause enter 并清除
 *                其残留内联 transform，保证 leave 从自然态起步
 *    - 动画：width natural→0 + opacity 1→0，ease inOut(2)
 *    - DOM：元素在动画期间保留在 DOM（Vue 不立即卸载），onComplete → done() 后才真正移除
 *
 * 3. 活跃切换（底部高亮条 FLIP）：
 *    - 开始：activeTabId 变化 → watch 触发 flipIndicator
 *    - FLIP：记录 oldRect → nextTick 更新逻辑 left/width（响应式 style）→ nextTick 记录 newRect
 *           → 计算 dx/scale → 施加反向 transform（translateX(dx) scaleX(scale)）→ 强制 reflow
 *           → animate transform → 0 / scale → 1，ease out(3)
 *    - 中断：快速连续切换时，pause 上一次未完成的 indicator 动画再起新的（indicatorAnim.pause()）
 *    - 父子关系：指示器是 tab-list 的直接子元素，与 TransitionGroup(tab-track) 同级；
 *               通过 getBoundingClientRect 相对 listRef 计算位置，兼容横向滚动
 *    - 返回：onComplete 清除 transform/transformOrigin，回归纯 left/width 定位
 *
 * 4. 首次激活：跳过 FLIP，直接定位（isFirstIndicator 标志），避免从 0 滑入的突兀感
 *
 * 5. Hover 关闭按钮：纯 CSS transition（opacity/transform），无 animejs 开销
 * 6. 录音红点脉冲 / 加载 spinner：CSS keyframe，infinite
 *
 * 交互：
 *  - 左键：激活
 *  - 中键（auxclick button=1）：关闭
 *  - 右键：上下文菜单（关闭 / 关闭其他 / 关闭右侧 / 重命名）
 *  - 横向滚轮：垂直滚轮转水平滚动；激活时自动 scrollIntoView
 */
import { ref, watch, nextTick, onMounted, onBeforeUnmount, computed } from 'vue'
import { animate } from 'animejs'
import Icon from './Icon.vue'
import { Menu, Dialog, useToast, type MenuItemOption } from './basic'
import { useTabs } from '../composables/useTabs'
import type { TabItem } from '../types'

defineOptions({ name: 'TabBar' })

const { tabs, activeTabId, activate, closeTab, renameTab } = useTabs()
const { toast } = useToast()

// ============= DOM refs =============
const listRef = ref<HTMLElement | null>(null)
const indicatorRef = ref<HTMLElement | null>(null)
const tabRefs = new Map<string, HTMLElement>()

function setTabRef(id: string, el: Element | null) {
  if (el instanceof HTMLElement) tabRefs.set(id, el)
  else tabRefs.delete(id)
}

// ============= 动画实例追踪（中断用） =============
type Anim = ReturnType<typeof animate> | null
const enterAnims = new WeakMap<HTMLElement, NonNullable<Anim>>()
const leaveAnims = new WeakMap<HTMLElement, NonNullable<Anim>>()
let indicatorAnim: NonNullable<Anim> | null = null

// ============= 指示器位置 =============
const indicatorStyle = ref<Record<string, string>>({ opacity: '0' })
let isFirstIndicator = true

function getActiveTabEl(): HTMLElement | null {
  const id = activeTabId.value
  if (!id) return null
  return tabRefs.get(id) ?? null
}

function computeIndicatorPos(): { left: number; width: number } | null {
  const activeEl = getActiveTabEl()
  const listEl = listRef.value
  if (!activeEl || !listEl) return null
  const listRect = listEl.getBoundingClientRect()
  const tabRect = activeEl.getBoundingClientRect()
  return {
    left: tabRect.left - listRect.left + listEl.scrollLeft,
    width: tabRect.width,
  }
}

function applyIndicatorPos(pos: { left: number; width: number } | null) {
  if (!pos) {
    indicatorStyle.value = { opacity: '0' }
    return
  }
  indicatorStyle.value = {
    left: `${pos.left}px`,
    width: `${pos.width}px`,
    opacity: '1',
  }
}

function repositionIndicator() {
  applyIndicatorPos(computeIndicatorPos())
}

async function flipIndicator() {
  const indEl = indicatorRef.value
  if (!indEl) return
  const oldRect = indEl.getBoundingClientRect()
  await nextTick()
  applyIndicatorPos(computeIndicatorPos())
  await nextTick()
  const pos = computeIndicatorPos()
  if (!pos) return
  const newRect = indEl.getBoundingClientRect()
  const dx = oldRect.left - newRect.left
  const scale = newRect.width > 0 ? oldRect.width / newRect.width : 1
  if (Math.abs(dx) < 0.5 && Math.abs(scale - 1) < 0.01) return
  indEl.style.transformOrigin = 'left center'
  indEl.style.transform = `translateX(${dx}px) scaleX(${scale})`
  // 强制 reflow，确保反向 transform 先生效再动画
  void indEl.offsetWidth
  indicatorAnim?.pause()
  indicatorAnim = animate(indEl, {
    translateX: 0,
    scaleX: 1,
    duration: 320,
    ease: 'out(3)',
    onComplete: () => {
      indEl.style.transform = ''
      indEl.style.transformOrigin = ''
      indicatorAnim = null
    },
  })
}

watch(activeTabId, () => {
  if (isFirstIndicator) {
    void nextTick().then(() => {
      repositionIndicator()
      isFirstIndicator = false
    })
    return
  }
  void flipIndicator()
})

// tabs 增删后重定位指示器（无 FLIP，避免与 enter/leave 宽度动画打架）
watch(
  () => tabs.value.length,
  () => {
    void nextTick().then(() => repositionIndicator())
  },
)

// ============= TransitionGroup 钩子 =============
function onTabEnter(el: Element, done: () => void) {
  const htmlEl = el as HTMLElement
  enterAnims.get(htmlEl)?.pause()
  // 测量自然宽度（在施加 width:0 之前）
  const naturalWidth = htmlEl.offsetWidth
  htmlEl.style.overflow = 'hidden'
  const anim = animate(htmlEl, {
    width: [0, naturalWidth],
    opacity: [0, 1],
    translateX: ['-8px', 0],
    duration: 300,
    ease: 'out(3)',
    onComplete: () => {
      htmlEl.style.width = ''
      htmlEl.style.opacity = ''
      htmlEl.style.transform = ''
      htmlEl.style.overflow = ''
      enterAnims.delete(htmlEl)
      repositionIndicator()
      done()
    },
  })
  enterAnims.set(htmlEl, anim)
}

function onTabLeave(el: Element, done: () => void) {
  const htmlEl = el as HTMLElement
  // 打断：若 enter 未完成，先暂停并清除其残留 transform，保证 leave 从自然态起步
  const prevEnter = enterAnims.get(htmlEl)
  if (prevEnter) {
    prevEnter.pause()
    enterAnims.delete(htmlEl)
  }
  htmlEl.style.transform = ''
  htmlEl.style.opacity = ''
  const currentWidth = htmlEl.offsetWidth
  htmlEl.style.overflow = 'hidden'
  leaveAnims.get(htmlEl)?.pause()
  const anim = animate(htmlEl, {
    width: [currentWidth, 0],
    opacity: [1, 0],
    duration: 240,
    ease: 'inOut(2)',
    onComplete: () => {
      leaveAnims.delete(htmlEl)
      done()
    },
  })
  leaveAnims.set(htmlEl, anim)
}

// ============= 交互 =============
  function tabIcon(tab: TabItem): string {
    if (tab.icon) return tab.icon
    switch (tab.kind) {
      case 'chat':
        return 'chat'
      case 'sub-agent':
        return 'sparkles'
      case 'plugin':
        return 'puzzle'
      case 'asr-stream':
        return 'mic'
      case 'asr-upload':
        return 'attachment'
      case 'asr-history':
        return 'clock'
    }
  }

function onTabClick(tab: TabItem) {
  if (tab.id !== activeTabId.value) activate(tab.id)
}

function onClose(tab: TabItem) {
  if (!tab.closable) {
    toast({ content: '该页签当前不可关闭', type: 'warn' })
    return
  }
  closeTab(tab.id)
}

function onAuxClick(e: MouseEvent, tab: TabItem) {
  // 中键关闭
  if (e.button === 1) {
    e.preventDefault()
    onClose(tab)
  }
}

// ============= 右键上下文菜单 =============
const menuVisible = ref(false)
const menuPosition = ref<{ x: number; y: number } | null>(null)
const menuTarget = ref<TabItem | null>(null)

const menuItems = computed<MenuItemOption[]>(() => {
  const t = menuTarget.value
  if (!t) return []
  const items: MenuItemOption[] = [
    { key: 'close', label: '关闭', icon: 'close', disabled: !t.closable },
  ]
  // 关闭其他：仅当 tabs > 1 时有意义
  if (tabs.value.length > 1) {
    items.push({ key: 'close-others', label: '关闭其他', icon: 'close', divided: true })
  }
  // 关闭右侧：仅当目标右侧还有可关闭页签
  const idx = tabs.value.findIndex((x) => x.id === t.id)
  const hasRight = tabs.value.slice(idx + 1).some((x) => x.closable)
  if (hasRight) {
    items.push({ key: 'close-right', label: '关闭右侧', icon: 'arrow-right' })
  }
  // 重命名（所有类型都可改标题）
  items.push({ key: 'rename', label: '重命名', icon: 'edit', divided: true })
  return items
})

function onContextMenu(e: MouseEvent, tab: TabItem) {
  e.preventDefault()
  menuPosition.value = { x: e.clientX, y: e.clientY }
  menuTarget.value = tab
  menuVisible.value = true
}

function onMenuSelect(item: MenuItemOption) {
  const t = menuTarget.value
  menuTarget.value = null
  if (!t) return
  switch (item.key) {
    case 'close':
      onClose(t)
      break
    case 'close-others':
      closeOthers(t)
      break
    case 'close-right':
      closeRight(t)
      break
    case 'rename':
      startRename(t)
      break
  }
}

function closeOthers(keep: TabItem) {
  const toClose = tabs.value.filter((t) => t.id !== keep.id && t.closable)
  toClose.forEach((t) => closeTab(t.id))
  activate(keep.id)
}

function closeRight(from: TabItem) {
  const idx = tabs.value.findIndex((t) => t.id === from.id)
  const toClose = tabs.value.slice(idx + 1).filter((t) => t.closable)
  // 倒序关闭，避免索引漂移
  for (let i = toClose.length - 1; i >= 0; i--) {
    closeTab(toClose[i].id)
  }
  activate(from.id)
}

// ============= 重命名对话框 =============
const renameVisible = ref(false)
const renameTarget = ref<TabItem | null>(null)
const renameInput = ref('')

function startRename(tab: TabItem) {
  renameTarget.value = tab
  renameInput.value = tab.title
  renameVisible.value = true
}

function confirmRename() {
  const t = renameTarget.value
  if (!t) return
  const title = renameInput.value.trim()
  if (!title) {
    toast({ content: '标题不能为空', type: 'warn' })
    return
  }
  renameTab(t.id, title)
  toast({ content: '已重命名', type: 'success' })
  renameTarget.value = null
}

// ============= 横向滚动 =============
function onWheel(e: WheelEvent) {
  const list = listRef.value
  if (!list) return
  // 垂直滚轮转水平滚动，避免页面整体滚动
  if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
    list.scrollLeft += e.deltaY
    e.preventDefault()
  }
}

function scrollActiveIntoView() {
  void nextTick().then(() => {
    const el = getActiveTabEl()
    el?.scrollIntoView({ behavior: 'smooth', inline: 'nearest', block: 'nearest' })
  })
}

watch(activeTabId, () => scrollActiveIntoView())

// ============= 窗口尺寸变化 =============
let resizeObserver: ResizeObserver | null = null
onMounted(() => {
  const list = listRef.value
  if (list && typeof ResizeObserver !== 'undefined') {
    resizeObserver = new ResizeObserver(() => repositionIndicator())
    resizeObserver.observe(list)
  }
  window.addEventListener('resize', repositionIndicator)
  // 首次定位
  void nextTick().then(() => repositionIndicator())
})

onBeforeUnmount(() => {
  resizeObserver?.disconnect()
  resizeObserver = null
  window.removeEventListener('resize', repositionIndicator)
  indicatorAnim?.pause()
})
</script>

<template>
  <div class="tab-bar">
    <div
      ref="listRef"
      class="tab-list"
      @wheel="onWheel"
    >
      <TransitionGroup
        :css="false"
        name="tab"
        tag="div"
        class="tab-track"
        @enter="onTabEnter"
        @leave="onTabLeave"
      >
        <button
          v-for="tab in tabs"
          :key="tab.id"
          :ref="(el) => setTabRef(tab.id, el as Element | null)"
          type="button"
          class="tab-item"
          :class="{
            active: tab.id === activeTabId,
            recording: tab.status === 'recording',
            loading: tab.status === 'loading',
            error: tab.status === 'error',
            'not-closable': !tab.closable,
          }"
          :title="tab.title"
          @click="onTabClick(tab)"
          @auxclick="onAuxClick($event, tab)"
          @contextmenu="onContextMenu($event, tab)"
        >
          <span class="tab-icon">
            <Icon
              v-if="tab.status === 'loading'"
              name="loader"
              :size="14"
              class="tab-spinner"
            />
            <Icon v-else :name="tabIcon(tab)" :size="14" />
          </span>
          <span v-if="tab.status === 'recording'" class="tab-rec-dot" />
          <span class="tab-title">{{ tab.title }}</span>
          <span
            v-if="tab.closable"
            class="tab-close"
            role="button"
            tabindex="-1"
            :aria-label="`关闭 ${tab.title}`"
            @click.stop="onClose(tab)"
          >
            <Icon name="close" :size="12" />
          </span>
        </button>
      </TransitionGroup>
      <!-- 底部高亮指示器：FLIP 平滑移动 -->
      <div ref="indicatorRef" class="tab-indicator" :style="indicatorStyle" />
    </div>

    <!-- 右键上下文菜单 -->
    <Menu
      v-model:visible="menuVisible"
      :items="menuItems"
      :position="menuPosition"
      @select="onMenuSelect"
    />

    <!-- 重命名对话框 -->
    <Dialog
      v-model:visible="renameVisible"
      title="重命名页签"
      confirm-text="保存"
      cancel-text="取消"
      @confirm="confirmRename"
    >
      <input
        v-model="renameInput"
        type="text"
        class="tab-rename-input"
        placeholder="输入新标题"
        @keyup.enter="confirmRename"
      />
    </Dialog>
  </div>
</template>

<style scoped>
/* 页签栏已移入标题栏中间区域（titlebar-center）：
   撑满标题栏高度、去除底部分割线（标题栏自带边框），
   背景透明融入标题栏；min/max-width 约束防止超出中间区域 */
.tab-bar {
  display: flex;
  align-items: stretch;
  height: 100%;
  flex-shrink: 0;
  min-width: 0;
  max-width: 100%;
  background: transparent;
  user-select: none;
  position: relative;
}

.tab-list {
  position: relative;
  display: flex;
  flex: 1;
  min-width: 0;
  overflow-x: auto;
  overflow-y: hidden;
  scrollbar-width: none;
}

.tab-list::-webkit-scrollbar {
  display: none;
}

.tab-track {
  display: inline-flex;
  align-items: center;
  height: 100%;
}

/* ---------- 单个 tab：紧凑高度 + margin-top/bottom，胶囊式背景区分 ---------- */
.tab-item {
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 200px;
  min-width: 0;
  padding: 0 10px;
  height: 30px;
  margin: 7px 3px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--muted);
  font-family: inherit;
  font-size: var(--fs-base);
  line-height: 1;
  white-space: nowrap;
  text-decoration: none;
  cursor: pointer;
  outline: none;
  flex-shrink: 0;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.tab-item:hover {
  background: var(--card-2);
  color: var(--text);
}

/* 激活页签：用 background-color 明确区分激活状态（取消底部 underline 设计） */
.tab-item.active {
  background: color-mix(in srgb, var(--primary) 16%, transparent);
  color: var(--text);
  font-weight: 500;
}

.tab-item.error {
  color: var(--danger);
}

/* 图标 */
.tab-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  color: inherit;
}

.tab-spinner {
  animation: tab-spin 0.7s linear infinite;
}

@keyframes tab-spin {
  to {
    transform: rotate(360deg);
  }
}

/* 录音红点脉冲 */
.tab-rec-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--danger);
  flex-shrink: 0;
  box-shadow: 0 0 0 0 rgba(255, 92, 92, 0.6);
  animation: tab-rec-pulse 1.4s var(--ease-standard) infinite;
}

@keyframes tab-rec-pulse {
  0% { box-shadow: 0 0 0 0 rgba(255, 92, 92, 0.55); }
  70% { box-shadow: 0 0 0 5px rgba(255, 92, 92, 0); }
  100% { box-shadow: 0 0 0 0 rgba(255, 92, 92, 0); }
}

/* 标题 */
.tab-title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}

/* 关闭按钮：hover tab 时淡入 */
.tab-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  color: var(--muted);
  opacity: 0;
  transform: scale(0.8);
  flex-shrink: 0;
  transition: opacity var(--duration-fast) var(--ease-standard),
    transform var(--duration-fast) var(--ease-standard),
    background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.tab-item:hover .tab-close,
.tab-item.active .tab-close {
  opacity: 1;
  transform: scale(1);
}

.tab-close:hover {
  background: var(--card-2);
  color: var(--danger);
}

/* 不可关闭的 tab 不留关闭按钮空间 */
.tab-item.not-closable {
  padding-right: 12px;
}

/* ---------- 底部高亮指示器：已取消 underline 设计，改用激活态背景色区分 ---------- */
.tab-indicator {
  display: none;
}

/* ---------- 重命名输入框 ---------- */
.tab-rename-input {
  width: 100%;
  height: var(--h-control-md);
  padding: 0 12px;
  font-family: inherit;
  font-size: var(--fs-base);
  color: var(--text);
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  outline: none;
  transition: border-color var(--duration-fast) var(--ease-standard);
  margin: 4px 0;
}

.tab-rename-input:focus {
  border-color: var(--primary);
}
</style>
